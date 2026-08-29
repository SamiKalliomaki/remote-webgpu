/*
 * remote-webgpu-client
 *
 * The client side of the remote WebGPU protocol (see
 * ../../proto/remote_webgpu.proto).  The *server* is a native application
 * written against webgpu.h; the *client* is the machine that actually owns a
 * GPU.  This library connects to the server's websocket, obtains a local
 * WebGPU adapter/device via navigator.gpu, and introduces them to the server
 * (the ServerHello/ClientHello handshake).
 *
 * Everything past the handshake -- executing the server's device methods on
 * the local GPU, presenting frames to the canvas and answering the
 * round-trips -- lives in `src/executor.ts`.
 */

import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  EnvelopeSchema,
  ProtocolVersion,
  type Envelope,
} from "./gen/remote_webgpu_pb.js";
import { CommandExecutor, type ReplyKind } from "./executor.js";
import {
  FEATURE_NAME_VALUES,
  WGSL_LANGUAGE_FEATURE_VALUES,
} from "./gen/enums.js";

export interface RemoteGpuClientOptions {
  /**
   * Canvas the remote application's frames will eventually be presented to.
   * Optional while presentation is unimplemented.
   */
  canvas?: HTMLCanvasElement;
  /**
   * Use this adapter instead of calling navigator.gpu.requestAdapter().
   * Pass null to connect without a GPU (the server just gets empty adapter
   * info); mainly useful in tests.
   */
  adapter?: GPUAdapter | null;
  /** Called with human-readable progress/status messages. */
  onStatus?: (message: string) => void;
  /** Called once per presented frame; useful for FPS counters. */
  onFrame?: (stats: FrameStats) => void;
  /** Called when the connection closes. */
  onClose?: (reason: string) => void;
}

/** Protocol traffic attributed to one presented frame. */
export interface FrameStats {
  /** Protobuf messages received from the server since the previous frame
   * (the Present command itself included). */
  messages: number;
  /** Total encoded size of those messages, in bytes. */
  bytes: number;
}

export class RemoteGpuClient {
  /** Protocol messages received since the last presented frame. */
  private frameMessages = 0;
  /** Bytes received since the last presented frame. */
  private frameBytes = 0;

  private readonly executor: CommandExecutor | null;
  /** Commands run strictly in order; this chain serializes the async ones. */
  private queue: Promise<void> = Promise.resolve();

  private constructor(
    private readonly ws: WebSocket,
    private readonly options: RemoteGpuClientOptions,
    /** The local adapter backing this connection (null when unavailable). */
    readonly adapter: GPUAdapter | null,
    /** The local device the server's commands will run on. */
    readonly device: GPUDevice | null,
  ) {
    this.executor = device
      ? new CommandExecutor(device, options.canvas ?? null, (kind) => this.send(kind),
                            () => {
                              options.onFrame?.({
                                messages: this.frameMessages,
                                bytes: this.frameBytes,
                              });
                              this.frameMessages = 0;
                              this.frameBytes = 0;
                            })
      : null;
    this.watchCanvasSize();
    ws.onmessage = (event) => {
      const data = new Uint8Array(event.data as ArrayBuffer);
      this.frameMessages += 1;
      this.frameBytes += data.byteLength;
      this.handleEnvelope(fromBinary(EnvelopeSchema, data));
    };
    ws.onclose = (event) => {
      options.onClose?.(event.reason || `connection closed (code ${event.code})`);
    };
    ws.onerror = () => {
      options.onClose?.("websocket error");
    };
  }

  /**
   * Connect to a remote WebGPU server and perform the opening handshake.
   * Resolves once the server's hello has been answered.
   */
  static async connect(
    url: string,
    options: RemoteGpuClientOptions = {},
  ): Promise<RemoteGpuClient> {
    const status = options.onStatus ?? (() => {});

    /* Bring up the local GPU first: there is no point connecting without
     * knowing what we can offer. */
    let adapter: GPUAdapter | null;
    if (options.adapter !== undefined) {
      adapter = options.adapter;
    } else if (typeof navigator !== "undefined" && navigator.gpu) {
      status("requesting local WebGPU adapter...");
      adapter = await navigator.gpu.requestAdapter();
    } else {
      adapter = null;
    }
    if (!adapter) status("no local WebGPU adapter available");
    const device = adapter ? await adapter.requestDevice() : null;

    status(`connecting to ${url}...`);
    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    await new Promise<void>((resolve, reject) => {
      ws.onopen = () => resolve();
      ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
      ws.onclose = () => reject(new Error(`connection to ${url} closed during setup`));
    });

    /* Handshake: the server speaks first. */
    status("waiting for ServerHello...");
    const first = await RemoteGpuClient.nextEnvelope(ws);
    if (first.kind.case !== "serverHello") {
      ws.close();
      throw new Error(`expected ServerHello, got ${first.kind.case ?? "nothing"}`);
    }
    if (first.kind.value.protocolVersion !== ProtocolVersion.CURRENT) {
      ws.close();
      throw new Error(
        `protocol version mismatch: server speaks ${first.kind.value.protocolVersion}, ` +
          `we speak ${ProtocolVersion.CURRENT}`,
      );
    }

    const info = adapter?.info;
    const size = options.canvas
      ? RemoteGpuClient.canvasSize(options.canvas)
      : { width: 0, height: 0 };
    const wgsl = typeof navigator !== "undefined" && navigator.gpu
      ? navigator.gpu.wgslLanguageFeatures : undefined;
    const hello = create(EnvelopeSchema, {
      kind: {
        case: "clientHello",
        value: {
          protocolVersion: ProtocolVersion.CURRENT,
          adapter: {
            vendor: info?.vendor ?? "",
            architecture: info?.architecture ?? "",
            device: info?.device ?? "",
            description: info?.description ?? "",
            isFallback: info?.isFallbackAdapter ?? false,
          },
          canvasWidth: size.width,
          canvasHeight: size.height,
          limits: adapter ? RemoteGpuClient.limitsMessage(adapter.limits) : undefined,
          features: adapter
            ? [...adapter.features]
                .map((f) => FEATURE_NAME_VALUES[f])
                .filter((v): v is number => v !== undefined)
            : [],
          wgslFeatures: wgsl
            ? [...wgsl]
                .map((f) => WGSL_LANGUAGE_FEATURE_VALUES[f])
                .filter((v): v is number => v !== undefined)
            : [],
        },
      },
    });
    ws.send(toBinary(EnvelopeSchema, hello));
    status("handshake complete");

    return new RemoteGpuClient(ws, options, adapter, device);
  }

  close(): void {
    this.ws.close();
  }

  /**
   * Send a user-defined event to the server application.  The name and
   * payload encoding are a contract between the page and the server (the
   * examples send "mousemove" with two little-endian float32s); the
   * library forwards them verbatim.
   */
  sendEvent(name: string, payload: Uint8Array = new Uint8Array()): void {
    this.send({ case: "event", value: { kind: { case: "user", value: { name, payload } } } });
  }

  /** The adapter's GPUSupportedLimits as a wire Limits message. */
  private static limitsMessage(l: GPUSupportedLimits) {
    return {
      maxTextureDimension1d: l.maxTextureDimension1D,
      maxTextureDimension2d: l.maxTextureDimension2D,
      maxTextureDimension3d: l.maxTextureDimension3D,
      maxTextureArrayLayers: l.maxTextureArrayLayers,
      maxBindGroups: l.maxBindGroups,
      maxBindGroupsPlusVertexBuffers: l.maxBindGroupsPlusVertexBuffers,
      maxBindingsPerBindGroup: l.maxBindingsPerBindGroup,
      maxDynamicUniformBuffersPerPipelineLayout:
        l.maxDynamicUniformBuffersPerPipelineLayout,
      maxDynamicStorageBuffersPerPipelineLayout:
        l.maxDynamicStorageBuffersPerPipelineLayout,
      maxSampledTexturesPerShaderStage: l.maxSampledTexturesPerShaderStage,
      maxSamplersPerShaderStage: l.maxSamplersPerShaderStage,
      maxStorageBuffersPerShaderStage: l.maxStorageBuffersPerShaderStage,
      maxStorageTexturesPerShaderStage: l.maxStorageTexturesPerShaderStage,
      maxUniformBuffersPerShaderStage: l.maxUniformBuffersPerShaderStage,
      maxUniformBufferBindingSize: BigInt(l.maxUniformBufferBindingSize),
      maxStorageBufferBindingSize: BigInt(l.maxStorageBufferBindingSize),
      minUniformBufferOffsetAlignment: l.minUniformBufferOffsetAlignment,
      minStorageBufferOffsetAlignment: l.minStorageBufferOffsetAlignment,
      maxVertexBuffers: l.maxVertexBuffers,
      maxBufferSize: BigInt(l.maxBufferSize),
      maxVertexAttributes: l.maxVertexAttributes,
      maxVertexBufferArrayStride: l.maxVertexBufferArrayStride,
      maxInterStageShaderVariables: l.maxInterStageShaderVariables,
      maxColorAttachments: l.maxColorAttachments,
      maxColorAttachmentBytesPerSample: l.maxColorAttachmentBytesPerSample,
      maxComputeWorkgroupStorageSize: l.maxComputeWorkgroupStorageSize,
      maxComputeInvocationsPerWorkgroup: l.maxComputeInvocationsPerWorkgroup,
      maxComputeWorkgroupSizeX: l.maxComputeWorkgroupSizeX,
      maxComputeWorkgroupSizeY: l.maxComputeWorkgroupSizeY,
      maxComputeWorkgroupSizeZ: l.maxComputeWorkgroupSizeZ,
      maxComputeWorkgroupsPerDimension: l.maxComputeWorkgroupsPerDimension,
    };
  }

  /** Canvas size in device pixels (what the server should render at). */
  private static canvasSize(canvas: HTMLCanvasElement): { width: number; height: number } {
    const scale = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
    /* Fall back to the backing-store size outside a layout (e.g. tests). */
    const w = canvas.clientWidth ? canvas.clientWidth * scale : canvas.width;
    const h = canvas.clientHeight ? canvas.clientHeight * scale : canvas.height;
    return { width: Math.max(1, Math.round(w)), height: Math.max(1, Math.round(h)) };
  }

  /**
   * Tell the server whenever the canvas element changes size (a built-in
   * event).  The server application decides how to react; reconfiguring
   * the surface is what actually resizes the canvas backing store, so the
   * canvas keeps its old pixel size until it does.
   */
  private watchCanvasSize(): void {
    const canvas = this.options.canvas;
    if (!canvas || typeof ResizeObserver === "undefined")
      return;
    let last = RemoteGpuClient.canvasSize(canvas);
    new ResizeObserver(() => {
      const size = RemoteGpuClient.canvasSize(canvas);
      if (size.width === last.width && size.height === last.height)
        return;
      last = size;
      this.send({ case: "event", value: { kind: { case: "canvasResize", value: size } } });
    }).observe(canvas);
  }

  /** Wait for a single envelope; used only during the handshake. */
  private static nextEnvelope(ws: WebSocket): Promise<Envelope> {
    return new Promise((resolve, reject) => {
      ws.onmessage = (event) => {
        ws.onmessage = null;
        resolve(fromBinary(EnvelopeSchema, new Uint8Array(event.data as ArrayBuffer)));
      };
      ws.onclose = () => reject(new Error("connection closed during handshake"));
    });
  }

  private send(kind: ReplyKind): void {
    this.ws.send(toBinary(EnvelopeSchema, create(EnvelopeSchema, { kind })));
  }

  /** Dispatch a message from the server, post-handshake. */
  private handleEnvelope(envelope: Envelope): void {
    if (envelope.kind.case === "error") {
      this.options.onStatus?.(`server error: ${envelope.kind.value.message}`);
      return;
    }
    /* Queue the command: order is part of the protocol, and some commands
     * (present, mapBufferRead) are asynchronous. */
    this.queue = this.queue.then(async () => {
      if (!this.executor)
        throw new Error("received a GPU command but no local device exists");
      await this.executor.execute(envelope.kind);
    }).catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      this.options.onStatus?.(`command failed: ${message}`);
      this.send({ case: "error", value: { message } });
      this.ws.close();
    });
  }
}
