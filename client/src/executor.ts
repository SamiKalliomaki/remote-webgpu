/*
 * Executes the server's command stream against the local WebGPU device.
 * One CommandExecutor per connection; objects live in an id -> object map,
 * mirroring the RemoteHandle ids assigned by the C library.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import type { Envelope, EnvelopeSchema } from "./gen/remote_webgpu_pb.js";

/** Reply payload: the `kind` oneof of an outgoing Envelope. */
export type ReplyKind = NonNullable<MessageInitShape<typeof EnvelopeSchema>["kind"]>;

/* Enum translation: the wire carries the numeric webgpu.h values. */

const TEXTURE_FORMATS: Record<number, GPUTextureFormat> = {
  0x16: "rgba8unorm",
  0x1b: "bgra8unorm",
};

const VERTEX_FORMATS: Record<number, GPUVertexFormat> = {
  0x1d: "float32x2",
  0x1e: "float32x3",
  0x1f: "float32x4",
};

const TOPOLOGIES: Record<number, GPUPrimitiveTopology> = {
  1: "point-list", 2: "line-list", 3: "line-strip",
  4: "triangle-list", 5: "triangle-strip",
};

const FRONT_FACES: Record<number, GPUFrontFace> = { 1: "ccw", 2: "cw" };
const CULL_MODES: Record<number, GPUCullMode> = { 1: "none", 2: "front", 3: "back" };
const STEP_MODES: Record<number, GPUVertexStepMode> = { 1: "vertex", 2: "instance" };
const LOAD_OPS: Record<number, GPULoadOp> = { 1: "load", 2: "clear" };
const STORE_OPS: Record<number, GPUStoreOp> = { 1: "store", 2: "discard" };
const BUFFER_BINDING_TYPES: Record<number, GPUBufferBindingType> = {
  2: "uniform", 3: "storage", 4: "read-only-storage",
};

function lookup<T>(table: Record<number, T>, value: number, what: string): T {
  const mapped = table[value];
  if (mapped === undefined)
    throw new Error(`unsupported ${what} value ${value}`);
  return mapped;
}

export class CommandExecutor {
  private readonly objects = new Map<number, unknown>();
  private context: GPUCanvasContext | null = null;
  /**
   * Commands of the frame currently being received, buffered from
   * surfaceGetCurrentTexture onwards.  The browser hands the canvas's
   * current texture to the compositor at the next rendering update after
   * getCurrentTexture() -- drawn into or not -- so acquiring it and
   * submitting must happen in the same task.  Executing command-by-command
   * as messages trickle in lets a vsync land between the two, which showed
   * up as intermittent black frames.  Instead, everything from the acquire
   * to the submit is deferred and replayed synchronously inside the
   * requestAnimationFrame callback that answers Present.
   */
  private frame: Envelope["kind"][] | null = null;

  constructor(
    private readonly device: GPUDevice,
    private readonly canvas: HTMLCanvasElement | null,
    /** Serialize and send a reply envelope back to the server. */
    private readonly reply: (kind: ReplyKind) => void,
    /** Called once per presented frame. */
    private readonly onPresent: () => void = () => {},
  ) {}

  private get<T>(id: number, what: string): T {
    const object = this.objects.get(id);
    if (object === undefined)
      throw new Error(`unknown ${what} id ${id}`);
    return object as T;
  }

  /** Handle one post-handshake command from the server. */
  async execute(kind: Envelope["kind"]): Promise<void> {
    switch (kind.case) {
      case "surfaceGetCurrentTexture":
        /* Start of a frame: buffer everything up to the present/readback. */
        this.frame = [kind];
        return;

      case "present": {
        const commands = this.frame;
        this.frame = null;
        /* Draw inside the animation-frame callback: acquire + submit happen
         * in one task, right before the compositor takes the frame, and
         * PresentDone paces the server to the display's refresh rate. */
        await new Promise<void>((resolve, reject) => {
          const draw = () => {
            try {
              if (commands)
                for (const command of commands) this.executeSync(command);
              resolve();
            } catch (error) {
              reject(error);
            }
          };
          if (typeof requestAnimationFrame === "function")
            requestAnimationFrame(draw);
          else
            setTimeout(draw, 16); /* non-browser environments */
        });
        this.reply({ case: "presentDone", value: {} });
        this.onPresent();
        return;
      }

      case "mapBufferRead": {
        /* Readback can arrive mid-frame (screenshots): flush the buffered
         * commands now so the copy is submitted before mapping. */
        if (this.frame) {
          for (const command of this.frame) this.executeSync(command);
          this.frame = null;
        }
        const m = kind.value;
        const buffer = this.get<GPUBuffer>(m.bufferId, "buffer");
        const READ = typeof GPUMapMode !== "undefined" ? GPUMapMode.READ : 1;
        await buffer.mapAsync(READ, Number(m.offset), Number(m.size));
        const data = new Uint8Array(
          buffer.getMappedRange(Number(m.offset), Number(m.size))).slice();
        buffer.unmap();
        this.reply({ case: "mapBufferData", value: { bufferId: m.bufferId, data } });
        return;
      }

      default:
        if (this.frame) {
          this.frame.push(kind);
          return;
        }
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

      case "createBuffer": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createBuffer({
          label: m.label, size: Number(m.size), usage: m.usage,
        }));
        break;
      }

      case "writeBuffer": {
        const m = kind.value;
        // Copy: the protobuf payload view may not be ArrayBuffer-aligned.
        this.device.queue.writeBuffer(
          this.get<GPUBuffer>(m.bufferId, "buffer"), Number(m.offset),
          m.data.slice().buffer);
        break;
      }

      case "createBindGroupLayout": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createBindGroupLayout({
          label: m.label,
          entries: m.entries.map((e) => ({
            binding: e.binding,
            visibility: e.visibility,
            buffer: {
              type: lookup(BUFFER_BINDING_TYPES, e.bufferBindingType, "buffer binding type"),
              minBindingSize: Number(e.minBindingSize),
            },
          })),
        }));
        break;
      }

      case "createBindGroup": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createBindGroup({
          label: m.label,
          layout: this.get<GPUBindGroupLayout>(m.layoutId, "bind group layout"),
          entries: m.entries.map((e) => ({
            binding: e.binding,
            resource: {
              buffer: this.get<GPUBuffer>(e.bufferId, "buffer"),
              offset: Number(e.offset),
              size: Number(e.size),
            },
          })),
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
            entryPoint: m.vertexEntryPoint,
            buffers: m.vertexBuffers.map((b) => ({
              arrayStride: Number(b.arrayStride),
              stepMode: lookup(STEP_MODES, b.stepMode, "vertex step mode"),
              attributes: b.attributes.map((a) => ({
                format: lookup(VERTEX_FORMATS, a.format, "vertex format"),
                offset: Number(a.offset),
                shaderLocation: a.shaderLocation,
              })),
            })),
          },
          primitive: {
            topology: lookup(TOPOLOGIES, m.topology, "topology"),
            frontFace: lookup(FRONT_FACES, m.frontFace, "front face"),
            cullMode: lookup(CULL_MODES, m.cullMode, "cull mode"),
          },
          multisample: { count: m.multisampleCount || 1 },
        };
        if (m.fragmentModuleId) {
          descriptor.fragment = {
            module: this.get<GPUShaderModule>(m.fragmentModuleId, "shader module"),
            entryPoint: m.fragmentEntryPoint,
            targets: m.targets.map((t) => ({
              format: lookup(TEXTURE_FORMATS, t.format, "texture format"),
              writeMask: t.writeMask,
            })),
          };
        }
        this.objects.set(m.id, this.device.createRenderPipeline(descriptor));
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
        this.context.configure({
          device: this.device,
          format: lookup(TEXTURE_FORMATS, m.format, "texture format"),
          usage: m.usage,
          alphaMode: "opaque",
        });
        break;
      }

      case "surfaceGetCurrentTexture": {
        if (!this.context)
          throw new Error("surfaceGetCurrentTexture before configureSurface");
        this.objects.set(kind.value.textureId, this.context.getCurrentTexture());
        break;
      }

      case "createTextureView": {
        const m = kind.value;
        this.objects.set(m.id, this.get<GPUTexture>(m.textureId, "texture").createView());
        break;
      }

      case "createCommandEncoder": {
        const m = kind.value;
        this.objects.set(m.id, this.device.createCommandEncoder({ label: m.label }));
        break;
      }

      case "beginRenderPass": {
        const m = kind.value;
        const encoder = this.get<GPUCommandEncoder>(m.encoderId, "command encoder");
        this.objects.set(m.id, encoder.beginRenderPass({
          label: m.label,
          colorAttachments: m.colorAttachments.map((a) => ({
            view: this.get<GPUTextureView>(a.viewId, "texture view"),
            loadOp: lookup(LOAD_OPS, a.loadOp, "load op"),
            storeOp: lookup(STORE_OPS, a.storeOp, "store op"),
            clearValue: { r: a.clearR, g: a.clearG, b: a.clearB, a: a.clearA },
          })),
        }));
        break;
      }

      case "setPipeline":
        this.get<GPURenderPassEncoder>(kind.value.passId, "render pass")
          .setPipeline(this.get<GPURenderPipeline>(kind.value.pipelineId, "pipeline"));
        break;

      case "setBindGroup":
        this.get<GPURenderPassEncoder>(kind.value.passId, "render pass")
          .setBindGroup(kind.value.index,
                        this.get<GPUBindGroup>(kind.value.bindGroupId, "bind group"));
        break;

      case "setVertexBuffer": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .setVertexBuffer(m.slot, this.get<GPUBuffer>(m.bufferId, "buffer"),
                           Number(m.offset), Number(m.size));
        break;
      }

      case "draw": {
        const m = kind.value;
        this.get<GPURenderPassEncoder>(m.passId, "render pass")
          .draw(m.vertexCount, m.instanceCount, m.firstVertex, m.firstInstance);
        break;
      }

      case "endRenderPass":
        this.get<GPURenderPassEncoder>(kind.value.passId, "render pass").end();
        break;

      case "finishEncoder": {
        const m = kind.value;
        this.objects.set(m.commandBufferId,
          this.get<GPUCommandEncoder>(m.encoderId, "command encoder").finish());
        break;
      }

      case "submit": {
        this.device.queue.submit(kind.value.commandBufferIds.map((id) =>
          this.get<GPUCommandBuffer>(id, "command buffer")));
        break;
      }

      case "copyTextureToBuffer": {
        const m = kind.value;
        this.get<GPUCommandEncoder>(m.encoderId, "command encoder").copyTextureToBuffer(
          { texture: this.get<GPUTexture>(m.textureId, "texture") },
          {
            buffer: this.get<GPUBuffer>(m.bufferId, "buffer"),
            bytesPerRow: m.bytesPerRow,
            rowsPerImage: m.rowsPerImage,
          },
          { width: m.width, height: m.height });
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
