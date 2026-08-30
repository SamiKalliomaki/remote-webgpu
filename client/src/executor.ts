/*
 * Executes the server's command stream against the local WebGPU device.
 * One CommandExecutor per connection; objects live in an id -> object map,
 * mirroring the RemoteHandle ids assigned by the C library.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import type {
  Envelope,
  EnvelopeSchema,
  TexelCopyBuffer,
  TexelCopyTexture,
  Extent3D,
  PassTimestampWrites,
  ConstantEntry,
  BlendComponent,
  StencilFaceState,
  Limits,
} from "./gen/remote_webgpu_pb.js";
import {
  ADDRESS_MODES,
  ALPHA_MODES,
  BLEND_FACTORS,
  BLEND_OPERATIONS,
  BUFFER_BINDING_TYPES,
  COMPARE_FUNCTIONS,
  CULL_MODES,
  ERROR_FILTERS,
  FEATURE_NAMES,
  FILTER_MODES,
  FRONT_FACES,
  INDEX_FORMATS,
  LOAD_OPS,
  MIPMAP_FILTER_MODES,
  QUERY_TYPES,
  SAMPLER_BINDING_TYPES,
  STENCIL_OPERATIONS,
  STEP_MODES,
  STORAGE_TEXTURE_ACCESSES,
  STORE_OPS,
  TEXTURE_ASPECTS,
  TEXTURE_DIMENSIONS,
  TEXTURE_FORMATS,
  TEXTURE_SAMPLE_TYPES,
  TEXTURE_VIEW_DIMENSIONS,
  TOPOLOGIES,
  VERTEX_FORMATS,
} from "./gen/enums.js";

/** Reply payload: the `kind` oneof of an outgoing Envelope. */
export type ReplyKind = NonNullable<MessageInitShape<typeof EnvelopeSchema>["kind"]>;

/* Sentinels from webgpu.h: "no value" markers that map to `undefined`. */
const U32_UNDEFINED = 0xffffffff;
const U64_WHOLE = 0xffffffffffffffffn;

function lookup<T>(table: Record<number, T>, value: number, what: string): T {
  const mapped = table[value];
  if (mapped === undefined)
    throw new Error(`unsupported ${what} value ${value}`);
  return mapped;
}

/** Like lookup(), but webgpu.h's 0 ("undefined") maps to `undefined`. */
function lookupOpt<T>(
  table: Record<number, T>, value: number, what: string,
): T | undefined {
  return value === 0 ? undefined : lookup(table, value, what);
}

function constants(entries: ConstantEntry[]): Record<string, number> | undefined {
  if (!entries.length)
    return undefined;
  const out: Record<string, number> = {};
  for (const e of entries) out[e.key] = e.value;
  return out;
}

function blendComponent(c: BlendComponent | undefined): GPUBlendComponent {
  return {
    operation: lookupOpt(BLEND_OPERATIONS, c?.operation ?? 0, "blend operation"),
    srcFactor: lookupOpt(BLEND_FACTORS, c?.srcFactor ?? 0, "blend factor"),
    dstFactor: lookupOpt(BLEND_FACTORS, c?.dstFactor ?? 0, "blend factor"),
  };
}

function stencilFace(s: StencilFaceState | undefined): GPUStencilFaceState {
  return {
    compare: lookupOpt(COMPARE_FUNCTIONS, s?.compare ?? 0, "compare function"),
    failOp: lookupOpt(STENCIL_OPERATIONS, s?.failOp ?? 0, "stencil operation"),
    depthFailOp: lookupOpt(STENCIL_OPERATIONS, s?.depthFailOp ?? 0, "stencil operation"),
    passOp: lookupOpt(STENCIL_OPERATIONS, s?.passOp ?? 0, "stencil operation"),
  };
}

function extent(e: Extent3D | undefined): GPUExtent3DDict {
  return {
    width: e?.width ?? 0,
    height: Math.max(1, e?.height ?? 1),
    depthOrArrayLayers: Math.max(1, e?.depthOrArrayLayers ?? 1),
  };
}

/** WGPUDeviceLostReason values. */
const LOST_REASONS: Record<GPUDeviceLostReason, number> = {
  unknown: 1,
  destroyed: 2,
};

/** Any of the encoder-like objects a pass_id may name (structural: the
 * branded GPU* types intersect to `never`). */
interface PassLike {
  setPipeline(pipeline: GPURenderPipeline & GPUComputePipeline): void;
  setBindGroup(index: number, group: GPUBindGroup | null,
               dynamicOffsets?: Iterable<number>): void;
  setVertexBuffer(slot: number, buffer: GPUBuffer | null,
                  offset?: number, size?: number): void;
  setIndexBuffer(buffer: GPUBuffer, format: GPUIndexFormat,
                 offset?: number, size?: number): void;
  draw(vertexCount: number, instanceCount?: number, firstVertex?: number,
       firstInstance?: number): void;
  drawIndexed(indexCount: number, instanceCount?: number, firstIndex?: number,
              baseVertex?: number, firstInstance?: number): void;
  drawIndirect(buffer: GPUBuffer, offset: number): void;
  drawIndexedIndirect(buffer: GPUBuffer, offset: number): void;
}

export class CommandExecutor {
  private readonly objects = new Map<number, unknown>();
  private context: GPUCanvasContext | null = null;
  private warnedWriteTimestamp = false;
  /**
   * Offscreen texture standing in for the canvas's current texture.  The
   * browser hands the canvas's real texture to the compositor at the next
   * rendering update after getCurrentTexture() -- drawn into or not -- so
   * the server's commands must not touch it directly (a vsync landing
   * between acquire and submit showed up as intermittent black frames).
   * Instead the server renders into this texture, at whatever pace its
   * messages arrive, and Present copies it into the canvas texture inside a
   * single requestAnimationFrame task.  Created by ConfigureSurface;
   * re-used from frame to frame (like a real swapchain, its previous
   * contents are unspecified as far as the server is concerned).
   */
  private surfaceTexture: GPUTexture | null = null;

  constructor(
    private device: GPUDevice,
    private readonly canvas: HTMLCanvasElement | null,
    /** Serialize and send a reply envelope back to the server. */
    private readonly reply: (kind: ReplyKind) => void,
    /** Called once per presented frame. */
    private readonly onPresent: () => void = () => {},
  ) {
    this.watchDevice(device);
  }

  /** Forward device-lost and uncaptured errors to the server. */
  private watchDevice(device: GPUDevice): void {
    device.lost.then((info) => {
      this.reply({
        case: "deviceLost",
        value: { reason: LOST_REASONS[info.reason] ?? 1, message: info.message },
      });
    }).catch(() => {});
    device.addEventListener?.("uncapturederror", (event) => {
      const error = (event as GPUUncapturedErrorEvent).error;
      this.reply({
        case: "uncapturedError",
        value: { type: this.errorType(error), message: error.message },
      });
    });
  }

  /** WGPUErrorType value for a GPUError. */
  private errorType(error: GPUError): number {
    if (typeof GPUValidationError !== "undefined" && error instanceof GPUValidationError)
      return 2;
    if (typeof GPUOutOfMemoryError !== "undefined" && error instanceof GPUOutOfMemoryError)
      return 3;
    if (typeof GPUInternalError !== "undefined" && error instanceof GPUInternalError)
      return 4;
    return 5;
  }

  private get<T>(id: number, what: string): T {
    const object = this.objects.get(id);
    if (object === undefined)
      throw new Error(`unknown ${what} id ${id}`);
    return object as T;
  }

  private getOpt<T>(id: number, what: string): T | null {
    return id === 0 ? null : this.get<T>(id, what);
  }

  private pass(id: number): PassLike {
    return this.get<PassLike>(id, "pass encoder");
  }

  private buffer(id: number): GPUBuffer {
    return this.get<GPUBuffer>(id, "buffer");
  }

  private encoder(id: number): GPUCommandEncoder {
    return this.get<GPUCommandEncoder>(id, "command encoder");
  }

  private copyTexture(t: TexelCopyTexture | undefined): GPUTexelCopyTextureInfo {
    if (!t)
      throw new Error("missing texel copy texture");
    return {
      texture: this.get<GPUTexture>(t.textureId, "texture"),
      mipLevel: t.mipLevel,
      origin: { x: t.originX, y: t.originY, z: t.originZ },
      aspect: lookupOpt(TEXTURE_ASPECTS, t.aspect, "texture aspect"),
    };
  }

  private copyBuffer(b: TexelCopyBuffer | undefined): GPUTexelCopyBufferInfo {
    if (!b)
      throw new Error("missing texel copy buffer");
    return {
      buffer: this.buffer(b.bufferId),
      offset: Number(b.offset),
      bytesPerRow: b.bytesPerRow === U32_UNDEFINED ? undefined : b.bytesPerRow,
      rowsPerImage: b.rowsPerImage === U32_UNDEFINED ? undefined : b.rowsPerImage,
    };
  }

  private timestampWrites(
    w: PassTimestampWrites | undefined,
  ): GPUComputePassTimestampWrites | undefined {
    if (!w)
      return undefined;
    return {
      querySet: this.get<GPUQuerySet>(w.querySetId, "query set"),
      beginningOfPassWriteIndex: w.beginningOfPassWriteIndex === U32_UNDEFINED
        ? undefined : w.beginningOfPassWriteIndex,
      endOfPassWriteIndex: w.endOfPassWriteIndex === U32_UNDEFINED
        ? undefined : w.endOfPassWriteIndex,
    };
  }

  /** Handle one post-handshake command from the server. */
  async execute(kind: Envelope["kind"]): Promise<void> {
    switch (kind.case) {
      case "present": {
        if (this.surfaceTexture && this.context) {
          const encoder = this.device.createCommandEncoder(
            { label: "present copy" });
          encoder.copyTextureToTexture(
            { texture: this.surfaceTexture },
            { texture: this.context.getCurrentTexture() },
            [this.surfaceTexture.width, this.surfaceTexture.height]);
          this.device.queue.submit([encoder.finish()]);
        }

        const presentDone = () => {
          this.reply({ case: "presentDone", value: {} });
          this.onPresent();
        };
        if (typeof requestAnimationFrame === "function")
          requestAnimationFrame(presentDone);
        else
          setTimeout(presentDone, 16); /* non-browser environments */
        return;
      }

      case "mapBuffer": {
        const m = kind.value;
        const buffer = this.buffer(m.bufferId);
        try {
          const size = m.size === U64_WHOLE ? undefined
            : Number(m.size);
          await buffer.mapAsync(m.mode, Number(m.offset), size);
          /* Send the contents back for read maps and write maps alike (a
           * write mapping exposes the buffer's current data too).  Write
           * maps stay mapped client-side until UnmapBuffer. */
          const data = new Uint8Array(
            buffer.getMappedRange(Number(m.offset), size)).slice();
          const WRITE = typeof GPUMapMode !== "undefined" ? GPUMapMode.WRITE : 2;
          if (!(m.mode & WRITE))
            buffer.unmap();
          this.reply({
            case: "mapBufferData",
            value: { requestId: m.requestId, data },
          });
        } catch (error) {
          this.reply({
            case: "mapBufferData",
            value: {
              requestId: m.requestId,
              failed: true,
              message: error instanceof Error ? error.message : String(error),
            },
          });
        }
        return;
      }

      case "loadTextureFromUrl": {
        /* Runs in the background: the server cannot reference the texture
         * id before our TextureLoaded reply reaches it, so later commands
         * need not wait for the fetch. */
        const m = kind.value;
        void (async () => {
          try {
            if (typeof fetch !== "function" || typeof createImageBitmap !== "function")
              throw new Error("image loading needs fetch/createImageBitmap");
            const response = await fetch(m.url);
            if (!response.ok)
              throw new Error(`HTTP ${response.status} for ${m.url}`);
            const bitmap = await createImageBitmap(await response.blob(), {
              colorSpaceConversion: "none",
              premultiplyAlpha: "none",
            });
            try {
              const texture = this.device.createTexture({
                label: m.label,
                format: "rgba8unorm",
                size: { width: bitmap.width, height: bitmap.height },
                usage: m.usage,
              });
              this.device.queue.copyExternalImageToTexture(
                { source: bitmap },
                { texture },
                { width: bitmap.width, height: bitmap.height });
              this.objects.set(m.textureId, texture);
              this.reply({
                case: "textureLoaded",
                value: {
                  requestId: m.requestId,
                  width: bitmap.width,
                  height: bitmap.height,
                },
              });
            } finally {
              bitmap.close();
            }
          } catch (error) {
            this.reply({
              case: "textureLoaded",
              value: {
                requestId: m.requestId,
                failed: true,
                message: error instanceof Error ? error.message : String(error),
              },
            });
          }
        })();
        return;
      }

      case "requestDevice": {
        const m = kind.value;
        const features = m.requiredFeatures.map((f) =>
          lookup(FEATURE_NAMES, f, "feature name") as GPUFeatureName);
        const limits = m.requiredLimits
          ? requiredLimits(m.requiredLimits)
          : undefined;
        if (features.length || limits) {
          /* Browsers expire an adapter after one requestDevice; get a
           * fresh one to carry the requirements. */
          if (typeof navigator === "undefined" || !navigator.gpu)
            throw new Error("no navigator.gpu to re-request a device from");
          const adapter = await navigator.gpu.requestAdapter();
          if (!adapter)
            throw new Error("cannot re-request an adapter for requestDevice");
          this.device = await adapter.requestDevice({
            label: m.label,
            requiredFeatures: features,
            requiredLimits: limits,
            defaultQueue: { label: m.defaultQueueLabel },
          });
          this.watchDevice(this.device);
          /* Reconfiguring binds the new device (and re-creates the
           * offscreen surface texture on it). */
          this.context = null;
          this.surfaceTexture?.destroy();
          this.surfaceTexture = null;
        }
        return;
      }

      default:
        this.executeSync(kind);
    }
  }

  /** Execute one synchronous command immediately. */
  private executeSync(kind: Envelope["kind"]): void {
    switch (kind.case) {
      case "createShaderModule": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createShaderModule({
          label: m.label, code: m.wgsl,
        }));
        break;
      }

      case "getCompilationInfo": {
        const m = kind.value;
        const module = this.get<GPUShaderModule>(m.moduleId, "shader module");
        module.getCompilationInfo().then((info) => {
          this.reply({
            case: "compilationInfoResult",
            value: {
              requestId: m.requestId,
              messages: info.messages.map((msg) => ({
                type: msg.type === "error" ? 1 : msg.type === "warning" ? 2 : 3,
                text: msg.message,
                lineNum: BigInt(msg.lineNum),
                linePos: BigInt(msg.linePos),
                offset: BigInt(msg.offset),
                length: BigInt(msg.length),
              })),
            },
          });
        });
        break;
      }

      case "createBuffer": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createBuffer({
          label: m.label, size: Number(m.size), usage: m.usage,
          mappedAtCreation: m.mappedAtCreation,
        }));
        break;
      }

      case "writeBuffer": {
        const m = kind.value;
        // Copy: the protobuf payload view may not be ArrayBuffer-aligned.
        this.device.queue.writeBuffer(
          this.buffer(m.bufferId), Number(m.offset), m.data.slice().buffer);
        break;
      }

      case "unmapBuffer": {
        const m = kind.value;
        const buffer = this.buffer(m.bufferId);
        if (m.write && m.data.length)
          new Uint8Array(buffer.getMappedRange(Number(m.offset), m.data.length))
            .set(m.data);
        buffer.unmap();
        break;
      }

      case "createTexture": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createTexture({
          label: m.label,
          usage: m.usage,
          dimension: lookupOpt(TEXTURE_DIMENSIONS, m.dimension, "texture dimension"),
          size: {
            width: m.width,
            height: Math.max(1, m.height),
            depthOrArrayLayers: Math.max(1, m.depthOrArrayLayers),
          },
          format: lookup(TEXTURE_FORMATS, m.format, "texture format"),
          mipLevelCount: m.mipLevelCount || 1,
          sampleCount: m.sampleCount || 1,
          viewFormats: m.viewFormats.map((f) =>
            lookup(TEXTURE_FORMATS, f, "texture format")),
        }));
        break;
      }

      case "createTextureView": {
        const m = kind.value;
        this.objects.set(m.id, this.get<GPUTexture>(m.textureId, "texture").createView({
          label: m.label,
          format: lookupOpt(TEXTURE_FORMATS, m.format, "texture format"),
          dimension: lookupOpt(TEXTURE_VIEW_DIMENSIONS, m.dimension, "view dimension"),
          aspect: lookupOpt(TEXTURE_ASPECTS, m.aspect, "texture aspect"),
          baseMipLevel: m.baseMipLevel,
          mipLevelCount: m.mipLevelCount === U32_UNDEFINED ? undefined : m.mipLevelCount,
          baseArrayLayer: m.baseArrayLayer,
          arrayLayerCount: m.arrayLayerCount === U32_UNDEFINED ? undefined : m.arrayLayerCount,
          ...(m.usage ? { usage: m.usage } : {}),
        }));
        break;
      }

      case "createSampler": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createSampler({
          label: m.label,
          addressModeU: lookupOpt(ADDRESS_MODES, m.addressModeU, "address mode"),
          addressModeV: lookupOpt(ADDRESS_MODES, m.addressModeV, "address mode"),
          addressModeW: lookupOpt(ADDRESS_MODES, m.addressModeW, "address mode"),
          magFilter: lookupOpt(FILTER_MODES, m.magFilter, "filter mode"),
          minFilter: lookupOpt(FILTER_MODES, m.minFilter, "filter mode"),
          mipmapFilter: lookupOpt(MIPMAP_FILTER_MODES, m.mipmapFilter, "mipmap filter"),
          lodMinClamp: m.lodMinClamp,
          lodMaxClamp: m.lodMaxClamp,
          compare: lookupOpt(COMPARE_FUNCTIONS, m.compare, "compare function"),
          maxAnisotropy: Math.max(1, m.maxAnisotropy),
        }));
        break;
      }

      case "createQuerySet": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createQuerySet({
          label: m.label,
          type: lookup(QUERY_TYPES, m.type, "query type"),
          count: m.count,
        }));
        break;
      }

      case "createBindGroupLayout": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createBindGroupLayout({
          label: m.label,
          entries: m.entries.map((e) => {
            const entry: GPUBindGroupLayoutEntry = {
              binding: e.binding,
              visibility: e.visibility,
            };
            if (e.buffer) {
              entry.buffer = {
                type: lookupOpt(BUFFER_BINDING_TYPES, e.buffer.type, "buffer binding type"),
                hasDynamicOffset: e.buffer.hasDynamicOffset,
                minBindingSize: Number(e.buffer.minBindingSize),
              };
            } else if (e.sampler) {
              entry.sampler = {
                type: lookupOpt(SAMPLER_BINDING_TYPES, e.sampler.type, "sampler binding type"),
              };
            } else if (e.texture) {
              entry.texture = {
                sampleType: lookupOpt(TEXTURE_SAMPLE_TYPES, e.texture.sampleType,
                                      "texture sample type"),
                viewDimension: lookupOpt(TEXTURE_VIEW_DIMENSIONS, e.texture.viewDimension,
                                         "view dimension"),
                multisampled: e.texture.multisampled,
              };
            } else if (e.storageTexture) {
              entry.storageTexture = {
                access: lookupOpt(STORAGE_TEXTURE_ACCESSES, e.storageTexture.access,
                                  "storage texture access"),
                format: lookup(TEXTURE_FORMATS, e.storageTexture.format, "texture format"),
                viewDimension: lookupOpt(TEXTURE_VIEW_DIMENSIONS,
                                         e.storageTexture.viewDimension, "view dimension"),
              };
            }
            return entry;
          }),
        }));
        break;
      }

      case "createBindGroup": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createBindGroup({
          label: m.label,
          layout: this.get<GPUBindGroupLayout>(m.layoutId, "bind group layout"),
          entries: m.entries.map((e) => {
            let resource: GPUBindingResource;
            if (e.samplerId) {
              resource = this.get<GPUSampler>(e.samplerId, "sampler");
            } else if (e.textureViewId) {
              resource = this.get<GPUTextureView>(e.textureViewId, "texture view");
            } else {
              resource = {
                buffer: this.buffer(e.bufferId),
                offset: Number(e.offset),
                size: e.size === U64_WHOLE ? undefined : Number(e.size),
              };
            }
            return { binding: e.binding, resource };
          }),
        }));
        break;
      }

      case "createPipelineLayout": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createPipelineLayout({
          label: m.label,
          bindGroupLayouts: m.bindGroupLayoutIds.map((id) =>
            this.get<GPUBindGroupLayout>(id, "bind group layout")),
        }));
        break;
      }

      case "createRenderPipeline": {
        const m = kind.value;
        const descriptor: GPURenderPipelineDescriptor = {
          label: m.label,
          layout: m.layoutId
            ? this.get<GPUPipelineLayout>(m.layoutId, "pipeline layout")
            : "auto",
          vertex: {
            module: this.get<GPUShaderModule>(m.vertexModuleId, "shader module"),
            entryPoint: m.vertexEntryPoint || undefined,
            constants: constants(m.vertexConstants),
            buffers: m.vertexBuffers.map((b) => ({
              arrayStride: Number(b.arrayStride),
              stepMode: lookupOpt(STEP_MODES, b.stepMode, "vertex step mode"),
              attributes: b.attributes.map((a) => ({
                format: lookup(VERTEX_FORMATS, a.format, "vertex format"),
                offset: Number(a.offset),
                shaderLocation: a.shaderLocation,
              })),
            })),
          },
          primitive: {
            topology: lookupOpt(TOPOLOGIES, m.topology, "topology"),
            stripIndexFormat: lookupOpt(INDEX_FORMATS, m.stripIndexFormat, "index format"),
            frontFace: lookupOpt(FRONT_FACES, m.frontFace, "front face"),
            cullMode: lookupOpt(CULL_MODES, m.cullMode, "cull mode"),
            unclippedDepth: m.unclippedDepth,
          },
          multisample: {
            count: m.multisampleCount || 1,
            mask: m.multisampleMask,
            alphaToCoverageEnabled: m.alphaToCoverageEnabled,
          },
        };
        if (m.depthStencil) {
          const d = m.depthStencil;
          descriptor.depthStencil = {
            format: lookup(TEXTURE_FORMATS, d.format, "texture format"),
            /* WGPUOptionalBool: 0 = false, 1 = true, 2 = undefined. */
            depthWriteEnabled: d.depthWriteEnabled === 2
              ? undefined : d.depthWriteEnabled === 1,
            depthCompare: lookupOpt(COMPARE_FUNCTIONS, d.depthCompare, "compare function"),
            stencilFront: stencilFace(d.stencilFront),
            stencilBack: stencilFace(d.stencilBack),
            stencilReadMask: d.stencilReadMask,
            stencilWriteMask: d.stencilWriteMask,
            depthBias: d.depthBias,
            depthBiasSlopeScale: d.depthBiasSlopeScale,
            depthBiasClamp: d.depthBiasClamp,
          };
        }
        if (m.fragmentModuleId) {
          descriptor.fragment = {
            module: this.get<GPUShaderModule>(m.fragmentModuleId, "shader module"),
            entryPoint: m.fragmentEntryPoint || undefined,
            constants: constants(m.fragmentConstants),
            targets: m.targets.map((t) => ({
              format: lookup(TEXTURE_FORMATS, t.format, "texture format"),
              writeMask: t.writeMask,
              blend: t.blend
                ? { color: blendComponent(t.blend.color), alpha: blendComponent(t.blend.alpha) }
                : undefined,
            })),
          };
        }
        this.objects.set(m.id, this.device.createRenderPipeline(descriptor));
        break;
      }

      case "createComputePipeline": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createComputePipeline({
          label: m.label,
          layout: m.layoutId
            ? this.get<GPUPipelineLayout>(m.layoutId, "pipeline layout")
            : "auto",
          compute: {
            module: this.get<GPUShaderModule>(m.moduleId, "shader module"),
            entryPoint: m.entryPoint || undefined,
            constants: constants(m.constants),
          },
        }));
        break;
      }

      case "pipelineGetBindGroupLayout": {
        const m = kind.value;
        const pipeline = this.get<GPURenderPipeline | GPUComputePipeline>(
          m.pipelineId, "pipeline");
        this.objects.set(m.id, pipeline.getBindGroupLayout(m.groupIndex));
        break;
      }

      case "configureSurface": {
        const m = kind.value;
        if (!this.canvas)
          throw new Error("server configured a surface but no canvas was provided");
        this.canvas.width = m.width;
        this.canvas.height = m.height;
        if (!this.context) {
          this.context = this.canvas.getContext("webgpu");
          if (!this.context)
            throw new Error("cannot create a webgpu canvas context");
        }
        const format = lookup(TEXTURE_FORMATS, m.format, "texture format");
        /* The canvas texture only ever receives the present-time copy. */
        this.context.configure({
          device: this.device,
          format,
          usage: GPUTextureUsage.COPY_DST,
          alphaMode: lookupOpt(ALPHA_MODES, m.alphaMode, "alpha mode") ?? "opaque",
        });
        this.surfaceTexture?.destroy();
        this.surfaceTexture = this.device.createTexture({
          label: "remote surface",
          size: [m.width, m.height],
          format,
          usage: m.usage | GPUTextureUsage.COPY_SRC,
          viewFormats: m.viewFormats.map((f) =>
            lookup(TEXTURE_FORMATS, f, "texture format")),
        });
        break;
      }

      case "surfaceUnconfigure": {
        this.context?.unconfigure();
        this.surfaceTexture?.destroy();
        this.surfaceTexture = null;
        break;
      }

      case "surfaceGetCurrentTexture": {
        if (!this.surfaceTexture)
          throw new Error("surfaceGetCurrentTexture before configureSurface");
        this.objects.set(kind.value.textureId, this.surfaceTexture);
        break;
      }

      case "createCommandEncoder": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createCommandEncoder({ label: m.label }));
        break;
      }

      case "beginRenderPass": {
        const m = kind.value;
        const descriptor: GPURenderPassDescriptor = {
          label: m.label,
          colorAttachments: m.colorAttachments.map((a) => ({
            view: this.get<GPUTextureView>(a.viewId, "texture view"),
            resolveTarget: this.getOpt<GPUTextureView>(
              a.resolveTargetId, "texture view") ?? undefined,
            depthSlice: a.depthSlice === U32_UNDEFINED ? undefined : a.depthSlice,
            loadOp: lookup(LOAD_OPS, a.loadOp, "load op"),
            storeOp: lookup(STORE_OPS, a.storeOp, "store op"),
            clearValue: { r: a.clearR, g: a.clearG, b: a.clearB, a: a.clearA },
          })),
          occlusionQuerySet: this.getOpt<GPUQuerySet>(
            m.occlusionQuerySetId, "query set") ?? undefined,
          timestampWrites: this.timestampWrites(m.timestampWrites),
          maxDrawCount: m.maxDrawCount ? Number(m.maxDrawCount) : undefined,
        };
        if (m.depthStencilAttachment) {
          const d = m.depthStencilAttachment;
          descriptor.depthStencilAttachment = {
            view: this.get<GPUTextureView>(d.viewId, "texture view"),
            depthLoadOp: lookupOpt(LOAD_OPS, d.depthLoadOp, "load op"),
            depthStoreOp: lookupOpt(STORE_OPS, d.depthStoreOp, "store op"),
            depthClearValue: d.depthClearValue,
            depthReadOnly: d.depthReadOnly,
            stencilLoadOp: lookupOpt(LOAD_OPS, d.stencilLoadOp, "load op"),
            stencilStoreOp: lookupOpt(STORE_OPS, d.stencilStoreOp, "store op"),
            stencilClearValue: d.stencilClearValue,
            stencilReadOnly: d.stencilReadOnly,
          };
        }
        this.objects.set(m.id, this.encoder(m.encoderId).beginRenderPass(descriptor));
        break;
      }

      case "beginComputePass": {
        const m = kind.value;
        this.objects.set(m.id, this.encoder(m.encoderId).beginComputePass({
          label: m.label,
          timestampWrites: this.timestampWrites(m.timestampWrites),
        }));
        break;
      }

      case "createRenderBundleEncoder": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createRenderBundleEncoder({
          label: m.label,
          colorFormats: m.colorFormats.map((f) =>
            lookupOpt(TEXTURE_FORMATS, f, "texture format") ?? null),
          depthStencilFormat: lookupOpt(TEXTURE_FORMATS, m.depthStencilFormat,
                                        "texture format"),
          sampleCount: m.sampleCount || 1,
          depthReadOnly: m.depthReadOnly,
          stencilReadOnly: m.stencilReadOnly,
        }));
        break;
      }

      case "finishRenderBundle": {
        const m = kind.value;
        this.objects.set(m.bundleId,
          this.get<GPURenderBundleEncoder>(m.encoderId, "render bundle encoder")
            .finish({ label: m.label }));
        break;
      }

      case "executeBundles": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass").executeBundles(
          m.bundleIds.map((id) => this.get<GPURenderBundle>(id, "render bundle")));
        break;
      }

      case "setPipeline": {
        const pipeline = this.get<GPURenderPipeline & GPUComputePipeline>(
          kind.value.pipelineId, "pipeline");
        this.pass(kind.value.passId).setPipeline(pipeline);
        break;
      }

      case "setBindGroup": {
        const m = kind.value;
        this.pass(m.passId).setBindGroup(
          m.index, this.getOpt<GPUBindGroup>(m.bindGroupId, "bind group"),
          m.dynamicOffsets);
        break;
      }

      case "setVertexBuffer": {
        const m = kind.value;
        this.pass(m.passId).setVertexBuffer(
          m.slot, this.getOpt<GPUBuffer>(m.bufferId, "buffer"), Number(m.offset),
          m.size === U64_WHOLE ? undefined : Number(m.size));
        break;
      }

      case "setIndexBuffer": {
        const m = kind.value;
        this.pass(m.passId).setIndexBuffer(
          this.buffer(m.bufferId), lookup(INDEX_FORMATS, m.format, "index format"),
          Number(m.offset), m.size === U64_WHOLE ? undefined : Number(m.size));
        break;
      }

      case "draw": {
        const m = kind.value;
        this.pass(m.passId).draw(
          m.vertexCount, m.instanceCount, m.firstVertex, m.firstInstance);
        break;
      }

      case "drawIndexed": {
        const m = kind.value;
        this.pass(m.passId).drawIndexed(
          m.indexCount, m.instanceCount, m.firstIndex, m.baseVertex, m.firstInstance);
        break;
      }

      case "drawIndirect": {
        const m = kind.value;
        this.pass(m.passId).drawIndirect(this.buffer(m.bufferId), Number(m.offset));
        break;
      }

      case "drawIndexedIndirect": {
        const m = kind.value;
        this.pass(m.passId).drawIndexedIndirect(this.buffer(m.bufferId), Number(m.offset));
        break;
      }

      case "dispatchWorkgroups": {
        const m = kind.value;
        this.get<GPUComputePassEncoder>(m.passId, "compute pass")
          .dispatchWorkgroups(m.x, m.y, m.z);
        break;
      }

      case "dispatchWorkgroupsIndirect": {
        const m = kind.value;
        this.get<GPUComputePassEncoder>(m.passId, "compute pass")
          .dispatchWorkgroupsIndirect(this.buffer(m.bufferId), Number(m.offset));
        break;
      }

      case "setViewport": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .setViewport(m.x, m.y, m.width, m.height, m.minDepth, m.maxDepth);
        break;
      }

      case "setScissorRect": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .setScissorRect(m.x, m.y, m.width, m.height);
        break;
      }

      case "setBlendConstant": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .setBlendConstant({ r: m.r, g: m.g, b: m.b, a: m.a });
        break;
      }

      case "setStencilReference": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .setStencilReference(m.reference);
        break;
      }

      case "beginOcclusionQuery": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .beginOcclusionQuery(m.queryIndex);
        break;
      }

      case "endOcclusionQuery":
        this.get<GPURenderPassEncoder>(kind.value.passId, "render pass")
          .endOcclusionQuery();
        break;

      case "endPass":
        this.get<GPURenderPassEncoder | GPUComputePassEncoder>(
          kind.value.passId, "pass").end();
        break;

      case "debugMarker": {
        const m = kind.value;
        const target = this.get<GPUCommandEncoder>(m.objectId, "encoder");
        if (m.op === 1)
          target.pushDebugGroup(m.label);
        else if (m.op === 2)
          target.popDebugGroup();
        else
          target.insertDebugMarker(m.label);
        break;
      }

      case "finishEncoder": {
        const m = kind.value;
        this.objects.set(m.commandBufferId,
          this.encoder(m.encoderId).finish({ label: m.label }));
        break;
      }

      case "submit": {
        this.device.queue.submit(kind.value.commandBufferIds.map((id) =>
          this.get<GPUCommandBuffer>(id, "command buffer")));
        break;
      }

      case "onSubmittedWorkDone": {
        const m = kind.value;
        this.device.queue.onSubmittedWorkDone().then(() => {
          this.reply({ case: "workDone", value: { requestId: m.requestId } });
        });
        break;
      }

      case "copyBufferToBuffer": {
        const m = kind.value;
        this.encoder(m.encoderId).copyBufferToBuffer(
          this.buffer(m.sourceId), Number(m.sourceOffset),
          this.buffer(m.destinationId), Number(m.destinationOffset),
          Number(m.size));
        break;
      }

      case "copyBufferToTexture": {
        const m = kind.value;
        this.encoder(m.encoderId).copyBufferToTexture(
          this.copyBuffer(m.source), this.copyTexture(m.destination),
          extent(m.size));
        break;
      }

      case "copyTextureToBuffer": {
        const m = kind.value;
        this.encoder(m.encoderId).copyTextureToBuffer(
          this.copyTexture(m.source), this.copyBuffer(m.destination),
          extent(m.size));
        break;
      }

      case "copyTextureToTexture": {
        const m = kind.value;
        this.encoder(m.encoderId).copyTextureToTexture(
          this.copyTexture(m.source), this.copyTexture(m.destination),
          extent(m.size));
        break;
      }

      case "clearBuffer": {
        const m = kind.value;
        this.encoder(m.encoderId).clearBuffer(
          this.buffer(m.bufferId), Number(m.offset),
          m.size === U64_WHOLE ? undefined : Number(m.size));
        break;
      }

      case "resolveQuerySet": {
        const m = kind.value;
        this.encoder(m.encoderId).resolveQuerySet(
          this.get<GPUQuerySet>(m.querySetId, "query set"),
          m.firstQuery, m.queryCount,
          this.buffer(m.destinationId), Number(m.destinationOffset));
        break;
      }

      case "writeTimestamp": {
        /* Not part of the JS WebGPU API; honor it only where the
         * implementation has the (non-standard) method. */
        const m = kind.value;
        const encoder = this.encoder(m.encoderId) as GPUCommandEncoder & {
          writeTimestamp?: (querySet: GPUQuerySet, index: number) => void;
        };
        if (encoder.writeTimestamp) {
          encoder.writeTimestamp(this.get<GPUQuerySet>(m.querySetId, "query set"),
                                 m.queryIndex);
        } else if (!this.warnedWriteTimestamp) {
          this.warnedWriteTimestamp = true;
          console.warn("remote-webgpu: writeTimestamp is not supported here; ignored");
        }
        break;
      }

      case "writeTexture": {
        const m = kind.value;
        this.device.queue.writeTexture(
          this.copyTexture(m.destination),
          m.data.slice().buffer,
          {
            offset: Number(m.layoutOffset),
            bytesPerRow: m.bytesPerRow === U32_UNDEFINED ? undefined : m.bytesPerRow,
            rowsPerImage: m.rowsPerImage === U32_UNDEFINED ? undefined : m.rowsPerImage,
          },
          extent(m.size));
        break;
      }

      case "pushErrorScope":
        this.device.pushErrorScope(
          lookup(ERROR_FILTERS, kind.value.filter, "error filter"));
        break;

      case "popErrorScope": {
        const m = kind.value;
        this.device.popErrorScope().then((error) => {
          this.reply({
            case: "errorScopeResult",
            value: {
              requestId: m.requestId,
              errorType: error ? this.errorType(error) : 1 /* NoError */,
              message: error?.message ?? "",
            },
          });
        }).catch((error) => {
          this.reply({
            case: "errorScopeResult",
            value: {
              requestId: m.requestId,
              errorType: 5 /* Unknown */,
              message: error instanceof Error ? error.message : String(error),
            },
          });
        });
        break;
      }

      case "destroyDevice":
        this.device.destroy(); /* device.lost reports DeviceLost */
        break;

      case "destroyResource": {
        const object = this.objects.get(kind.value.id) as { destroy?: () => void };
        object?.destroy?.();
        break;
      }

      case "setObjectLabel": {
        const object = this.objects.get(kind.value.id) as { label?: string };
        if (object)
          object.label = kind.value.label;
        break;
      }

      case "destroyObject": {
        const object = this.objects.get(kind.value.id);
        this.objects.delete(kind.value.id);
        if (typeof GPUBuffer !== "undefined" && object instanceof GPUBuffer)
          object.destroy();
        break;
      }

      default:
        throw new Error(`unhandled message kind "${kind.case ?? "unknown"}"`);
    }
  }
}

/** Build a GPUDeviceDescriptor.requiredLimits from a Limits message;
 * zero-valued fields are "not required". */
function requiredLimits(limits: Limits): Record<string, number> | undefined {
  const out: Record<string, number> = {};
  const set = (name: string, value: number | bigint) => {
    if (value)
      out[name] = Number(value);
  };
  set("maxTextureDimension1D", limits.maxTextureDimension1d);
  set("maxTextureDimension2D", limits.maxTextureDimension2d);
  set("maxTextureDimension3D", limits.maxTextureDimension3d);
  set("maxTextureArrayLayers", limits.maxTextureArrayLayers);
  set("maxBindGroups", limits.maxBindGroups);
  set("maxBindGroupsPlusVertexBuffers", limits.maxBindGroupsPlusVertexBuffers);
  set("maxBindingsPerBindGroup", limits.maxBindingsPerBindGroup);
  set("maxDynamicUniformBuffersPerPipelineLayout",
      limits.maxDynamicUniformBuffersPerPipelineLayout);
  set("maxDynamicStorageBuffersPerPipelineLayout",
      limits.maxDynamicStorageBuffersPerPipelineLayout);
  set("maxSampledTexturesPerShaderStage", limits.maxSampledTexturesPerShaderStage);
  set("maxSamplersPerShaderStage", limits.maxSamplersPerShaderStage);
  set("maxStorageBuffersPerShaderStage", limits.maxStorageBuffersPerShaderStage);
  set("maxStorageTexturesPerShaderStage", limits.maxStorageTexturesPerShaderStage);
  set("maxUniformBuffersPerShaderStage", limits.maxUniformBuffersPerShaderStage);
  set("maxUniformBufferBindingSize", limits.maxUniformBufferBindingSize);
  set("maxStorageBufferBindingSize", limits.maxStorageBufferBindingSize);
  set("minUniformBufferOffsetAlignment", limits.minUniformBufferOffsetAlignment);
  set("minStorageBufferOffsetAlignment", limits.minStorageBufferOffsetAlignment);
  set("maxVertexBuffers", limits.maxVertexBuffers);
  set("maxBufferSize", limits.maxBufferSize);
  set("maxVertexAttributes", limits.maxVertexAttributes);
  set("maxVertexBufferArrayStride", limits.maxVertexBufferArrayStride);
  set("maxInterStageShaderVariables", limits.maxInterStageShaderVariables);
  set("maxColorAttachments", limits.maxColorAttachments);
  set("maxColorAttachmentBytesPerSample", limits.maxColorAttachmentBytesPerSample);
  set("maxComputeWorkgroupStorageSize", limits.maxComputeWorkgroupStorageSize);
  set("maxComputeInvocationsPerWorkgroup", limits.maxComputeInvocationsPerWorkgroup);
  set("maxComputeWorkgroupSizeX", limits.maxComputeWorkgroupSizeX);
  set("maxComputeWorkgroupSizeY", limits.maxComputeWorkgroupSizeY);
  set("maxComputeWorkgroupSizeZ", limits.maxComputeWorkgroupSizeZ);
  set("maxComputeWorkgroupsPerDimension", limits.maxComputeWorkgroupsPerDimension);
  return Object.keys(out).length ? out : undefined;
}
