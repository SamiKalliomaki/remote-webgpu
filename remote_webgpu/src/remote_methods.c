/*
 * webgpu.h entry points that forward to the remote GPU.
 *
 * Every function here translates its arguments into one protocol message
 * (see ../../proto/remote_webgpu.proto) and sends it; object handles are
 * RemoteHandle (a client-side id plus a device ref).  Only two calls wait
 * for a reply: wgpuSurfacePresent (which paces the render loop) and
 * wgpuBufferMapAsync (which pulls the buffer contents back for readback).
 */

#include "remote_webgpu_internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <webgpu/wgpu.h>

/* Send one envelope holding `submsg` under `kind`, via `device`. */
#define SEND(device, KIND_ENUM, field, submsg)                                   \
    do {                                                                         \
        RemoteWebgpu__Envelope envelope_ = REMOTE_WEBGPU__ENVELOPE__INIT;        \
        envelope_.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_##KIND_ENUM;         \
        envelope_.field = (submsg);                                              \
        rw_send_envelope(rw_device_fd(device), &envelope_);                      \
    } while (0)

static uint32_t handle_id(const void *handle)
{
    return handle ? ((const RemoteHandle *)handle)->id : 0;
}

/* ------------------------------------------------------------------ */
/* shader modules                                                     */
/* ------------------------------------------------------------------ */

WGPUShaderModule wgpuDeviceCreateShaderModule(WGPUDevice device,
                                              WGPUShaderModuleDescriptor const *descriptor)
{
    const WGPUShaderSourceWGSL *wgsl = NULL;
    for (const WGPUChainedStruct *chain = descriptor->nextInChain; chain;
         chain = chain->next) {
        if (chain->sType == WGPUSType_ShaderSourceWGSL) {
            wgsl = (const WGPUShaderSourceWGSL *)chain;
            break;
        }
    }
    if (!wgsl) {
        fprintf(stderr, "remote_webgpu: only WGSL shader modules are supported\n");
        return NULL;
    }

    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    RemoteWebgpu__CreateShaderModule msg = REMOTE_WEBGPU__CREATE_SHADER_MODULE__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.wgsl = rw_dup_stringview(wgsl->code);
    SEND((RemoteDevice *)device, CREATE_SHADER_MODULE, create_shader_module, &msg);
    free(msg.label);
    free(msg.wgsl);
    return (WGPUShaderModule)handle;
}

void wgpuShaderModuleAddRef(WGPUShaderModule shaderModule) { rw_handle_addref(shaderModule); }
void wgpuShaderModuleRelease(WGPUShaderModule shaderModule) { rw_handle_release(shaderModule); }

/* ------------------------------------------------------------------ */
/* buffers                                                            */
/* ------------------------------------------------------------------ */

WGPU_NULLABLE WGPUBuffer wgpuDeviceCreateBuffer(WGPUDevice device,
                                                WGPUBufferDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    RemoteWebgpu__CreateBuffer msg = REMOTE_WEBGPU__CREATE_BUFFER__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.size = descriptor->size;
    msg.usage = (uint32_t)descriptor->usage;
    SEND((RemoteDevice *)device, CREATE_BUFFER, create_buffer, &msg);
    free(msg.label);
    return (WGPUBuffer)handle;
}

void wgpuBufferDestroy(WGPUBuffer buffer)
{
    /* The client-side GPUBuffer is destroyed when the last ref goes away
     * (DestroyObject); an early destroy is not forwarded yet. */
    (void)buffer;
}

WGPUFuture wgpuBufferMapAsync(WGPUBuffer buffer, WGPUMapMode mode, size_t offset,
                              size_t size, WGPUBufferMapCallbackInfo callbackInfo)
{
    (void)mode; /* only Read is supported; the message implies it */
    RemoteHandle *self = (RemoteHandle *)buffer;
    WGPUFuture future = { 0 };

    RemoteWebgpu__MapBufferRead msg = REMOTE_WEBGPU__MAP_BUFFER_READ__INIT;
    msg.buffer_id = self->id;
    msg.offset = offset;
    msg.size = size;
    SEND(self->device, MAP_BUFFER_READ, map_buffer_read, &msg);

    WGPUMapAsyncStatus status = WGPUMapAsyncStatus_Error;
    WGPUStringView message = { NULL, 0 };
    RemoteWebgpu__Envelope *reply =
        rw_recv_expect(self->device->adapter,
                       REMOTE_WEBGPU__ENVELOPE__KIND_MAP_BUFFER_DATA);
    if (reply) {
        const ProtobufCBinaryData *data = &reply->map_buffer_data->data;
        free(self->mapped);
        self->mapped = malloc(data->len ? data->len : 1);
        if (self->mapped) {
            memcpy(self->mapped, data->data, data->len);
            self->mapped_len = data->len;
            status = WGPUMapAsyncStatus_Success;
        }
        remote_webgpu__envelope__free_unpacked(reply, NULL);
    } else {
        message.data = "client did not return buffer contents";
        message.length = strlen(message.data);
    }

    if (callbackInfo.callback)
        callbackInfo.callback(status, message, callbackInfo.userdata1,
                              callbackInfo.userdata2);
    return future;
}

void const *wgpuBufferGetConstMappedRange(WGPUBuffer buffer, size_t offset, size_t size)
{
    RemoteHandle *self = (RemoteHandle *)buffer;
    if (!self->mapped || offset + size > self->mapped_len)
        return NULL;
    return self->mapped + offset;
}

void wgpuBufferUnmap(WGPUBuffer buffer)
{
    RemoteHandle *self = (RemoteHandle *)buffer;
    free(self->mapped);
    self->mapped = NULL;
    self->mapped_len = 0;
}

void wgpuBufferAddRef(WGPUBuffer buffer) { rw_handle_addref(buffer); }
void wgpuBufferRelease(WGPUBuffer buffer) { rw_handle_release(buffer); }

/* ------------------------------------------------------------------ */
/* bind groups and layouts                                            */
/* ------------------------------------------------------------------ */

WGPUBindGroupLayout wgpuDeviceCreateBindGroupLayout(WGPUDevice device,
                                                    WGPUBindGroupLayoutDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    size_t n = descriptor->entryCount;
    RemoteWebgpu__BindGroupLayoutEntry *entries = calloc(n ? n : 1, sizeof *entries);
    RemoteWebgpu__BindGroupLayoutEntry **entry_ptrs = calloc(n ? n : 1, sizeof *entry_ptrs);
    for (size_t i = 0; i < n; ++i) {
        const WGPUBindGroupLayoutEntry *in = &descriptor->entries[i];
        remote_webgpu__bind_group_layout_entry__init(&entries[i]);
        entries[i].binding = in->binding;
        entries[i].visibility = (uint32_t)in->visibility;
        entries[i].buffer_binding_type = (uint32_t)in->buffer.type;
        entries[i].min_binding_size = in->buffer.minBindingSize;
        entry_ptrs[i] = &entries[i];
        if (in->buffer.type == WGPUBufferBindingType_BindingNotUsed)
            fprintf(stderr, "remote_webgpu: non-buffer bind group layout entries "
                            "are not supported yet (binding %u)\n", in->binding);
    }

    RemoteWebgpu__CreateBindGroupLayout msg = REMOTE_WEBGPU__CREATE_BIND_GROUP_LAYOUT__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.n_entries = n;
    msg.entries = entry_ptrs;
    SEND((RemoteDevice *)device, CREATE_BIND_GROUP_LAYOUT, create_bind_group_layout, &msg);
    free(msg.label);
    free(entry_ptrs);
    free(entries);
    return (WGPUBindGroupLayout)handle;
}

void wgpuBindGroupLayoutAddRef(WGPUBindGroupLayout l) { rw_handle_addref(l); }
void wgpuBindGroupLayoutRelease(WGPUBindGroupLayout l) { rw_handle_release(l); }

WGPUBindGroup wgpuDeviceCreateBindGroup(WGPUDevice device,
                                        WGPUBindGroupDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    size_t n = descriptor->entryCount;
    RemoteWebgpu__BindGroupEntry *entries = calloc(n ? n : 1, sizeof *entries);
    RemoteWebgpu__BindGroupEntry **entry_ptrs = calloc(n ? n : 1, sizeof *entry_ptrs);
    for (size_t i = 0; i < n; ++i) {
        const WGPUBindGroupEntry *in = &descriptor->entries[i];
        remote_webgpu__bind_group_entry__init(&entries[i]);
        entries[i].binding = in->binding;
        entries[i].buffer_id = handle_id(in->buffer);
        entries[i].offset = in->offset;
        entries[i].size = in->size;
        entry_ptrs[i] = &entries[i];
    }

    RemoteWebgpu__CreateBindGroup msg = REMOTE_WEBGPU__CREATE_BIND_GROUP__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.layout_id = handle_id(descriptor->layout);
    msg.n_entries = n;
    msg.entries = entry_ptrs;
    SEND((RemoteDevice *)device, CREATE_BIND_GROUP, create_bind_group, &msg);
    free(msg.label);
    free(entry_ptrs);
    free(entries);
    return (WGPUBindGroup)handle;
}

void wgpuBindGroupAddRef(WGPUBindGroup g) { rw_handle_addref(g); }
void wgpuBindGroupRelease(WGPUBindGroup g) { rw_handle_release(g); }

/* ------------------------------------------------------------------ */
/* pipelines                                                          */
/* ------------------------------------------------------------------ */

WGPUPipelineLayout wgpuDeviceCreatePipelineLayout(WGPUDevice device,
                                                  WGPUPipelineLayoutDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    size_t n = descriptor->bindGroupLayoutCount;
    uint32_t *ids = calloc(n ? n : 1, sizeof *ids);
    for (size_t i = 0; i < n; ++i)
        ids[i] = handle_id(descriptor->bindGroupLayouts[i]);

    RemoteWebgpu__CreatePipelineLayout msg = REMOTE_WEBGPU__CREATE_PIPELINE_LAYOUT__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.n_bind_group_layout_ids = n;
    msg.bind_group_layout_ids = ids;
    SEND((RemoteDevice *)device, CREATE_PIPELINE_LAYOUT, create_pipeline_layout, &msg);
    free(msg.label);
    free(ids);
    return (WGPUPipelineLayout)handle;
}

void wgpuPipelineLayoutAddRef(WGPUPipelineLayout l) { rw_handle_addref(l); }
void wgpuPipelineLayoutRelease(WGPUPipelineLayout l) { rw_handle_release(l); }

WGPURenderPipeline wgpuDeviceCreateRenderPipeline(WGPUDevice device,
                                                  WGPURenderPipelineDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    /* vertex buffer layouts */
    size_t nbuf = descriptor->vertex.bufferCount;
    RemoteWebgpu__VertexBufferLayout *layouts = calloc(nbuf ? nbuf : 1, sizeof *layouts);
    RemoteWebgpu__VertexBufferLayout **layout_ptrs = calloc(nbuf ? nbuf : 1, sizeof *layout_ptrs);
    RemoteWebgpu__VertexAttribute **attr_bases = calloc(nbuf ? nbuf : 1, sizeof *attr_bases);
    for (size_t i = 0; i < nbuf; ++i) {
        const WGPUVertexBufferLayout *in = &descriptor->vertex.buffers[i];
        remote_webgpu__vertex_buffer_layout__init(&layouts[i]);
        layouts[i].array_stride = in->arrayStride;
        layouts[i].step_mode = (uint32_t)in->stepMode;

        size_t nattr = in->attributeCount;
        RemoteWebgpu__VertexAttribute *attrs = calloc(nattr ? nattr : 1, sizeof *attrs);
        RemoteWebgpu__VertexAttribute **attr_ptrs = calloc(nattr ? nattr : 1, sizeof *attr_ptrs);
        for (size_t j = 0; j < nattr; ++j) {
            remote_webgpu__vertex_attribute__init(&attrs[j]);
            attrs[j].format = (uint32_t)in->attributes[j].format;
            attrs[j].offset = in->attributes[j].offset;
            attrs[j].shader_location = in->attributes[j].shaderLocation;
            attr_ptrs[j] = &attrs[j];
        }
        layouts[i].n_attributes = nattr;
        layouts[i].attributes = attr_ptrs;
        attr_bases[i] = attrs;
        layout_ptrs[i] = &layouts[i];
    }

    /* fragment targets */
    size_t ntgt = descriptor->fragment ? descriptor->fragment->targetCount : 0;
    RemoteWebgpu__ColorTargetState *targets = calloc(ntgt ? ntgt : 1, sizeof *targets);
    RemoteWebgpu__ColorTargetState **target_ptrs = calloc(ntgt ? ntgt : 1, sizeof *target_ptrs);
    for (size_t i = 0; i < ntgt; ++i) {
        const WGPUColorTargetState *in = &descriptor->fragment->targets[i];
        remote_webgpu__color_target_state__init(&targets[i]);
        targets[i].format = (uint32_t)in->format;
        targets[i].write_mask = (uint32_t)in->writeMask;
        target_ptrs[i] = &targets[i];
        if (in->blend)
            fprintf(stderr, "remote_webgpu: blend state is not supported yet\n");
    }

    RemoteWebgpu__CreateRenderPipeline msg = REMOTE_WEBGPU__CREATE_RENDER_PIPELINE__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.layout_id = handle_id(descriptor->layout);
    msg.vertex_module_id = handle_id(descriptor->vertex.module);
    msg.vertex_entry_point = rw_dup_stringview(descriptor->vertex.entryPoint);
    msg.n_vertex_buffers = nbuf;
    msg.vertex_buffers = layout_ptrs;
    msg.topology = (uint32_t)descriptor->primitive.topology;
    msg.front_face = (uint32_t)descriptor->primitive.frontFace;
    msg.cull_mode = (uint32_t)descriptor->primitive.cullMode;
    msg.multisample_count = descriptor->multisample.count;
    if (descriptor->fragment) {
        msg.fragment_module_id = handle_id(descriptor->fragment->module);
        msg.fragment_entry_point = rw_dup_stringview(descriptor->fragment->entryPoint);
    }
    msg.n_targets = ntgt;
    msg.targets = target_ptrs;
    SEND((RemoteDevice *)device, CREATE_RENDER_PIPELINE, create_render_pipeline, &msg);

    free(msg.label);
    free(msg.vertex_entry_point);
    if (descriptor->fragment)
        free(msg.fragment_entry_point);
    for (size_t i = 0; i < nbuf; ++i) {
        free(attr_bases[i]);
        free(layouts[i].attributes);
    }
    free(attr_bases);
    free(layout_ptrs);
    free(layouts);
    free(target_ptrs);
    free(targets);
    return (WGPURenderPipeline)handle;
}

void wgpuRenderPipelineAddRef(WGPURenderPipeline p) { rw_handle_addref(p); }
void wgpuRenderPipelineRelease(WGPURenderPipeline p) { rw_handle_release(p); }

/* ------------------------------------------------------------------ */
/* surface frames and textures                                        */
/* ------------------------------------------------------------------ */

void wgpuSurfaceGetCurrentTexture(WGPUSurface surface, WGPUSurfaceTexture *surfaceTexture)
{
    RemoteSurface *self = (RemoteSurface *)surface;
    memset(surfaceTexture, 0, sizeof *surfaceTexture);
    if (!self->device) {
        fprintf(stderr, "remote_webgpu: surface used before configuration\n");
        surfaceTexture->status = WGPUSurfaceGetCurrentTextureStatus_Error;
        return;
    }

    RemoteHandle *texture = rw_handle_create(self->device);
    if (!texture) {
        surfaceTexture->status = WGPUSurfaceGetCurrentTextureStatus_Error;
        return;
    }
    texture->width = self->config.width;
    texture->height = self->config.height;

    RemoteWebgpu__SurfaceGetCurrentTexture msg =
        REMOTE_WEBGPU__SURFACE_GET_CURRENT_TEXTURE__INIT;
    msg.texture_id = texture->id;
    SEND(self->device, SURFACE_GET_CURRENT_TEXTURE, surface_get_current_texture, &msg);

    surfaceTexture->texture = (WGPUTexture)texture;
    surfaceTexture->status = WGPUSurfaceGetCurrentTextureStatus_SuccessOptimal;
}

WGPUStatus wgpuSurfacePresent(WGPUSurface surface)
{
    RemoteSurface *self = (RemoteSurface *)surface;
    if (!self->device)
        return WGPUStatus_Error;

    RemoteWebgpu__Present msg = REMOTE_WEBGPU__PRESENT__INIT;
    SEND(self->device, PRESENT, present, &msg);

    /* The reply arrives once the frame is on the client's screen; this is
     * what paces the render loop to the client's refresh rate. */
    RemoteWebgpu__Envelope *reply =
        rw_recv_expect(self->device->adapter,
                       REMOTE_WEBGPU__ENVELOPE__KIND_PRESENT_DONE);
    if (!reply)
        return WGPUStatus_Error;
    remote_webgpu__envelope__free_unpacked(reply, NULL);
    return WGPUStatus_Success;
}

WGPUTextureView wgpuTextureCreateView(WGPUTexture texture,
                                      WGPU_NULLABLE WGPUTextureViewDescriptor const *descriptor)
{
    if (descriptor)
        fprintf(stderr, "remote_webgpu: non-default texture views are not supported yet\n");

    RemoteHandle *self = (RemoteHandle *)texture;
    RemoteHandle *view = rw_handle_create(self->device);
    if (!view)
        return NULL;

    RemoteWebgpu__CreateTextureView msg = REMOTE_WEBGPU__CREATE_TEXTURE_VIEW__INIT;
    msg.id = view->id;
    msg.texture_id = self->id;
    SEND(self->device, CREATE_TEXTURE_VIEW, create_texture_view, &msg);
    return (WGPUTextureView)view;
}

uint32_t wgpuTextureGetWidth(WGPUTexture texture) { return ((RemoteHandle *)texture)->width; }
uint32_t wgpuTextureGetHeight(WGPUTexture texture) { return ((RemoteHandle *)texture)->height; }

void wgpuTextureAddRef(WGPUTexture t) { rw_handle_addref(t); }
void wgpuTextureRelease(WGPUTexture t) { rw_handle_release(t); }
void wgpuTextureViewAddRef(WGPUTextureView v) { rw_handle_addref(v); }
void wgpuTextureViewRelease(WGPUTextureView v) { rw_handle_release(v); }

/* ------------------------------------------------------------------ */
/* command recording                                                  */
/* ------------------------------------------------------------------ */

WGPUCommandEncoder wgpuDeviceCreateCommandEncoder(WGPUDevice device,
                                                  WGPU_NULLABLE WGPUCommandEncoderDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    RemoteWebgpu__CreateCommandEncoder msg = REMOTE_WEBGPU__CREATE_COMMAND_ENCODER__INIT;
    msg.id = handle->id;
    msg.label = descriptor ? rw_dup_stringview(descriptor->label) : strdup("");
    SEND((RemoteDevice *)device, CREATE_COMMAND_ENCODER, create_command_encoder, &msg);
    free(msg.label);
    return (WGPUCommandEncoder)handle;
}

WGPURenderPassEncoder wgpuCommandEncoderBeginRenderPass(WGPUCommandEncoder commandEncoder,
                                                        WGPURenderPassDescriptor const *descriptor)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteHandle *pass = rw_handle_create(encoder->device);
    if (!pass)
        return NULL;

    size_t n = descriptor->colorAttachmentCount;
    RemoteWebgpu__ColorAttachment *atts = calloc(n ? n : 1, sizeof *atts);
    RemoteWebgpu__ColorAttachment **att_ptrs = calloc(n ? n : 1, sizeof *att_ptrs);
    for (size_t i = 0; i < n; ++i) {
        const WGPURenderPassColorAttachment *in = &descriptor->colorAttachments[i];
        remote_webgpu__color_attachment__init(&atts[i]);
        atts[i].view_id = handle_id(in->view);
        atts[i].load_op = (uint32_t)in->loadOp;
        atts[i].store_op = (uint32_t)in->storeOp;
        atts[i].clear_r = in->clearValue.r;
        atts[i].clear_g = in->clearValue.g;
        atts[i].clear_b = in->clearValue.b;
        atts[i].clear_a = in->clearValue.a;
        att_ptrs[i] = &atts[i];
    }
    if (descriptor->depthStencilAttachment)
        fprintf(stderr, "remote_webgpu: depth/stencil attachments are not supported yet\n");

    RemoteWebgpu__BeginRenderPass msg = REMOTE_WEBGPU__BEGIN_RENDER_PASS__INIT;
    msg.id = pass->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.encoder_id = encoder->id;
    msg.n_color_attachments = n;
    msg.color_attachments = att_ptrs;
    SEND(encoder->device, BEGIN_RENDER_PASS, begin_render_pass, &msg);
    free(msg.label);
    free(att_ptrs);
    free(atts);
    return (WGPURenderPassEncoder)pass;
}

void wgpuRenderPassEncoderSetPipeline(WGPURenderPassEncoder renderPassEncoder,
                                      WGPURenderPipeline pipeline)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetPipeline msg = REMOTE_WEBGPU__SET_PIPELINE__INIT;
    msg.pass_id = pass->id;
    msg.pipeline_id = handle_id(pipeline);
    SEND(pass->device, SET_PIPELINE, set_pipeline, &msg);
}

void wgpuRenderPassEncoderSetBindGroup(WGPURenderPassEncoder renderPassEncoder,
                                       uint32_t groupIndex, WGPU_NULLABLE WGPUBindGroup group,
                                       size_t dynamicOffsetCount, uint32_t const *dynamicOffsets)
{
    (void)dynamicOffsets;
    if (dynamicOffsetCount)
        fprintf(stderr, "remote_webgpu: dynamic offsets are not supported yet\n");

    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetBindGroup msg = REMOTE_WEBGPU__SET_BIND_GROUP__INIT;
    msg.pass_id = pass->id;
    msg.index = groupIndex;
    msg.bind_group_id = handle_id(group);
    SEND(pass->device, SET_BIND_GROUP, set_bind_group, &msg);
}

void wgpuRenderPassEncoderSetVertexBuffer(WGPURenderPassEncoder renderPassEncoder,
                                          uint32_t slot, WGPU_NULLABLE WGPUBuffer buffer,
                                          uint64_t offset, uint64_t size)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetVertexBuffer msg = REMOTE_WEBGPU__SET_VERTEX_BUFFER__INIT;
    msg.pass_id = pass->id;
    msg.slot = slot;
    msg.buffer_id = handle_id(buffer);
    msg.offset = offset;
    msg.size = size;
    SEND(pass->device, SET_VERTEX_BUFFER, set_vertex_buffer, &msg);
}

void wgpuRenderPassEncoderDraw(WGPURenderPassEncoder renderPassEncoder, uint32_t vertexCount,
                               uint32_t instanceCount, uint32_t firstVertex,
                               uint32_t firstInstance)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__Draw msg = REMOTE_WEBGPU__DRAW__INIT;
    msg.pass_id = pass->id;
    msg.vertex_count = vertexCount;
    msg.instance_count = instanceCount;
    msg.first_vertex = firstVertex;
    msg.first_instance = firstInstance;
    SEND(pass->device, DRAW, draw, &msg);
}

void wgpuRenderPassEncoderEnd(WGPURenderPassEncoder renderPassEncoder)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__EndRenderPass msg = REMOTE_WEBGPU__END_RENDER_PASS__INIT;
    msg.pass_id = pass->id;
    SEND(pass->device, END_RENDER_PASS, end_render_pass, &msg);
}

void wgpuRenderPassEncoderAddRef(WGPURenderPassEncoder p) { rw_handle_addref(p); }
void wgpuRenderPassEncoderRelease(WGPURenderPassEncoder p) { rw_handle_release(p); }

void wgpuCommandEncoderCopyTextureToBuffer(WGPUCommandEncoder commandEncoder,
                                           WGPUTexelCopyTextureInfo const *source,
                                           WGPUTexelCopyBufferInfo const *destination,
                                           WGPUExtent3D const *copySize)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__CopyTextureToBuffer msg = REMOTE_WEBGPU__COPY_TEXTURE_TO_BUFFER__INIT;
    msg.encoder_id = encoder->id;
    msg.texture_id = handle_id(source->texture);
    msg.buffer_id = handle_id(destination->buffer);
    msg.bytes_per_row = destination->layout.bytesPerRow;
    msg.rows_per_image = destination->layout.rowsPerImage;
    msg.width = copySize->width;
    msg.height = copySize->height;
    SEND(encoder->device, COPY_TEXTURE_TO_BUFFER, copy_texture_to_buffer, &msg);
}

WGPUCommandBuffer wgpuCommandEncoderFinish(WGPUCommandEncoder commandEncoder,
                                           WGPU_NULLABLE WGPUCommandBufferDescriptor const *descriptor)
{
    (void)descriptor;
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteHandle *commands = rw_handle_create(encoder->device);
    if (!commands)
        return NULL;

    RemoteWebgpu__FinishEncoder msg = REMOTE_WEBGPU__FINISH_ENCODER__INIT;
    msg.encoder_id = encoder->id;
    msg.command_buffer_id = commands->id;
    SEND(encoder->device, FINISH_ENCODER, finish_encoder, &msg);
    return (WGPUCommandBuffer)commands;
}

void wgpuCommandEncoderAddRef(WGPUCommandEncoder e) { rw_handle_addref(e); }
void wgpuCommandEncoderRelease(WGPUCommandEncoder e) { rw_handle_release(e); }
void wgpuCommandBufferAddRef(WGPUCommandBuffer b) { rw_handle_addref(b); }
void wgpuCommandBufferRelease(WGPUCommandBuffer b) { rw_handle_release(b); }

/* ------------------------------------------------------------------ */
/* queue                                                              */
/* ------------------------------------------------------------------ */

void wgpuQueueWriteBuffer(WGPUQueue queue, WGPUBuffer buffer, uint64_t bufferOffset,
                          void const *data, size_t size)
{
    RemoteQueue *self = (RemoteQueue *)queue;
    RemoteWebgpu__WriteBuffer msg = REMOTE_WEBGPU__WRITE_BUFFER__INIT;
    msg.buffer_id = handle_id(buffer);
    msg.offset = bufferOffset;
    msg.data.data = (uint8_t *)data; /* packed (copied) inside rw_send_envelope */
    msg.data.len = size;
    SEND(self->device, WRITE_BUFFER, write_buffer, &msg);
}

void wgpuQueueSubmit(WGPUQueue queue, size_t commandCount, WGPUCommandBuffer const *commands)
{
    RemoteQueue *self = (RemoteQueue *)queue;
    uint32_t *ids = calloc(commandCount ? commandCount : 1, sizeof *ids);
    for (size_t i = 0; i < commandCount; ++i)
        ids[i] = handle_id(commands[i]);

    RemoteWebgpu__Submit msg = REMOTE_WEBGPU__SUBMIT__INIT;
    msg.n_command_buffer_ids = commandCount;
    msg.command_buffer_ids = ids;
    SEND(self->device, SUBMIT, submit, &msg);
    free(ids);
}
