/*
 * webgpu.h entry points that forward to the remote GPU.
 *
 * Every function here translates its arguments into one protocol message
 * (see ../../proto/remote_webgpu.proto) and sends it; object handles are
 * RemoteHandle (a client-side id plus a device ref).  Calls that need an
 * answer (buffer maps, error scopes, work-done, compilation info) queue an
 * RwRequest; the reply handlers in remote_webgpu.c complete them from
 * inside wgpuRemoteAdapterReceiveData().
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
        rw_send_envelope(rw_device_adapter(device), &envelope_);                 \
    } while (0)

static uint32_t handle_id(const void *handle)
{
    return handle ? ((const RemoteHandle *)handle)->id : 0;
}

/* Forward a wgpu*SetLabel() for any object with a client-side id. */
static void set_label(void *handle, WGPUStringView label)
{
    RemoteHandle *self = handle;
    RemoteWebgpu__SetObjectLabel msg = REMOTE_WEBGPU__SET_OBJECT_LABEL__INIT;
    msg.id = self->id;
    msg.label = rw_dup_stringview(label);
    SEND(self->device, SET_OBJECT_LABEL, set_object_label, &msg);
    free(msg.label);
}

/* Forward a wgpu*Destroy() (eager resource destruction). */
static void destroy_resource(void *handle)
{
    RemoteHandle *self = handle;
    RemoteWebgpu__DestroyResource msg = REMOTE_WEBGPU__DESTROY_RESOURCE__INIT;
    msg.id = self->id;
    SEND(self->device, DESTROY_RESOURCE, destroy_resource, &msg);
}

/* Debug groups / markers on any encoder or pass. */
static void debug_marker(void *handle, uint32_t op, WGPUStringView label)
{
    RemoteHandle *self = handle;
    RemoteWebgpu__DebugMarker msg = REMOTE_WEBGPU__DEBUG_MARKER__INIT;
    msg.object_id = self->id;
    msg.op = op;
    msg.label = rw_dup_stringview(label);
    SEND(self->device, DEBUG_MARKER, debug_marker, &msg);
    free(msg.label);
}

static void texel_copy_texture(RemoteWebgpu__TexelCopyTexture *out,
                               const WGPUTexelCopyTextureInfo *in)
{
    remote_webgpu__texel_copy_texture__init(out);
    out->texture_id = handle_id(in->texture);
    out->mip_level = in->mipLevel;
    out->origin_x = in->origin.x;
    out->origin_y = in->origin.y;
    out->origin_z = in->origin.z;
    out->aspect = (uint32_t)in->aspect;
}

static void texel_copy_buffer(RemoteWebgpu__TexelCopyBuffer *out,
                              const WGPUTexelCopyBufferInfo *in)
{
    remote_webgpu__texel_copy_buffer__init(out);
    out->buffer_id = handle_id(in->buffer);
    out->offset = in->layout.offset;
    out->bytes_per_row = in->layout.bytesPerRow;
    out->rows_per_image = in->layout.rowsPerImage;
}

static void extent3d(RemoteWebgpu__Extent3D *out, const WGPUExtent3D *in)
{
    remote_webgpu__extent3_d__init(out);
    out->width = in->width;
    out->height = in->height;
    out->depth_or_array_layers = in->depthOrArrayLayers;
}

/* malloc()ed array of ConstantEntry pointers (NULL when count is 0);
 * frees with free_constants(). */
static RemoteWebgpu__ConstantEntry **build_constants(size_t count,
                                                     const WGPUConstantEntry *in)
{
    if (!count)
        return NULL;
    RemoteWebgpu__ConstantEntry *entries = calloc(count, sizeof *entries);
    RemoteWebgpu__ConstantEntry **ptrs = calloc(count, sizeof *ptrs);
    if (!entries || !ptrs) {
        free(entries);
        free(ptrs);
        return NULL;
    }
    for (size_t i = 0; i < count; ++i) {
        remote_webgpu__constant_entry__init(&entries[i]);
        entries[i].key = rw_dup_stringview(in[i].key);
        entries[i].value = in[i].value;
        ptrs[i] = &entries[i];
    }
    return ptrs;
}

static void free_constants(RemoteWebgpu__ConstantEntry **ptrs, size_t count)
{
    if (!ptrs)
        return;
    for (size_t i = 0; i < count; ++i)
        free(ptrs[i]->key);
    free(ptrs[0]); /* the entries array */
    free(ptrs);
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

WGPUFuture wgpuShaderModuleGetCompilationInfo(WGPUShaderModule shaderModule,
                                              WGPUCompilationInfoCallbackInfo callbackInfo)
{
    RemoteHandle *self = (RemoteHandle *)shaderModule;
    RemoteAdapter *adapter = rw_device_adapter(self->device);
    WGPUFuture future = { 0 };

    RwRequest *request = rw_request_create(adapter, RW_REQUEST_COMPILATION);
    if (!request)
        return future;
    request->cb.compilation = callbackInfo;
    future.id = request->future_id;

    RemoteWebgpu__GetCompilationInfo msg = REMOTE_WEBGPU__GET_COMPILATION_INFO__INIT;
    msg.request_id = request->request_id;
    msg.module_id = self->id;
    SEND(self->device, GET_COMPILATION_INFO, get_compilation_info, &msg);
    return future;
}

void wgpuShaderModuleSetLabel(WGPUShaderModule m, WGPUStringView label) { set_label(m, label); }
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
    handle->size = descriptor->size;
    handle->usage = descriptor->usage;

    if (descriptor->mappedAtCreation) {
        /* The spec maps the whole (zero-filled) buffer. */
        handle->mapped = calloc(1, descriptor->size ? descriptor->size : 1);
        if (!handle->mapped) {
            rw_handle_release(handle);
            return NULL;
        }
        handle->mapped_len = descriptor->size;
        handle->mapped_offset = 0;
        handle->mapped_write = 1;
        handle->map_state = WGPUBufferMapState_Mapped;
    }

    RemoteWebgpu__CreateBuffer msg = REMOTE_WEBGPU__CREATE_BUFFER__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.size = descriptor->size;
    msg.usage = (uint32_t)descriptor->usage;
    msg.mapped_at_creation = descriptor->mappedAtCreation != 0;
    SEND((RemoteDevice *)device, CREATE_BUFFER, create_buffer, &msg);
    free(msg.label);
    return (WGPUBuffer)handle;
}

void wgpuBufferDestroy(WGPUBuffer buffer)
{
    destroy_resource(buffer);
}

WGPUFuture wgpuBufferMapAsync(WGPUBuffer buffer, WGPUMapMode mode, size_t offset,
                              size_t size, WGPUBufferMapCallbackInfo callbackInfo)
{
    RemoteHandle *self = (RemoteHandle *)buffer;
    RemoteAdapter *adapter = rw_device_adapter(self->device);
    WGPUFuture future = { 0 };

    if (self->map_state != WGPUBufferMapState_Unmapped) {
        if (callbackInfo.callback) {
            WGPUStringView message = { "buffer is already mapped or pending", 35 };
            callbackInfo.callback(WGPUMapAsyncStatus_Error, message,
                                  callbackInfo.userdata1, callbackInfo.userdata2);
        }
        return future;
    }

    RwRequest *request = rw_request_create(adapter, RW_REQUEST_MAP);
    if (!request)
        return future;
    request->cb.map = callbackInfo;
    request->handle = self;
    request->map_mode = mode;
    request->map_offset = offset;
    wgpuBufferAddRef(buffer); /* released when the reply arrives */
    future.id = request->future_id;
    self->map_state = WGPUBufferMapState_Pending;

    RemoteWebgpu__MapBuffer msg = REMOTE_WEBGPU__MAP_BUFFER__INIT;
    msg.request_id = request->request_id;
    msg.buffer_id = self->id;
    msg.mode = (uint32_t)mode;
    msg.offset = offset;
    msg.size = size == WGPU_WHOLE_MAP_SIZE ? UINT64_MAX : (uint64_t)size;
    SEND(self->device, MAP_BUFFER, map_buffer, &msg);
    return future;
}

static void *mapped_range(RemoteHandle *self, size_t offset, size_t size)
{
    if (self->map_state != WGPUBufferMapState_Mapped || !self->mapped)
        return NULL;
    if (offset < self->mapped_offset)
        return NULL;
    size_t start = offset - (size_t)self->mapped_offset;
    if (start > self->mapped_len)
        return NULL;
    if (size == WGPU_WHOLE_MAP_SIZE)
        size = self->mapped_len - start;
    if (start + size > self->mapped_len)
        return NULL;
    return self->mapped + start;
}

void const *wgpuBufferGetConstMappedRange(WGPUBuffer buffer, size_t offset, size_t size)
{
    return mapped_range((RemoteHandle *)buffer, offset, size);
}

void *wgpuBufferGetMappedRange(WGPUBuffer buffer, size_t offset, size_t size)
{
    RemoteHandle *self = (RemoteHandle *)buffer;
    if (!self->mapped_write)
        return NULL; /* read-only mapping */
    return mapped_range(self, offset, size);
}

WGPUStatus wgpuBufferReadMappedRange(WGPUBuffer buffer, size_t offset, void *data,
                                     size_t size)
{
    const void *range = mapped_range((RemoteHandle *)buffer, offset, size);
    if (!range)
        return WGPUStatus_Error;
    memcpy(data, range, size);
    return WGPUStatus_Success;
}

WGPUStatus wgpuBufferWriteMappedRange(WGPUBuffer buffer, size_t offset,
                                      void const *data, size_t size)
{
    void *range = wgpuBufferGetMappedRange(buffer, offset, size);
    if (!range)
        return WGPUStatus_Error;
    memcpy(range, data, size);
    return WGPUStatus_Success;
}

void wgpuBufferUnmap(WGPUBuffer buffer)
{
    RemoteHandle *self = (RemoteHandle *)buffer;
    if (self->map_state != WGPUBufferMapState_Mapped)
        return;

    RemoteWebgpu__UnmapBuffer msg = REMOTE_WEBGPU__UNMAP_BUFFER__INIT;
    msg.buffer_id = self->id;
    if (self->mapped_write) {
        msg.write = 1;
        msg.offset = self->mapped_offset;
        msg.data.data = self->mapped;
        msg.data.len = self->mapped_len;
    }
    SEND(self->device, UNMAP_BUFFER, unmap_buffer, &msg);

    free(self->mapped);
    self->mapped = NULL;
    self->mapped_len = 0;
    self->mapped_offset = 0;
    self->mapped_write = 0;
    self->map_state = WGPUBufferMapState_Unmapped;
}

uint64_t wgpuBufferGetSize(WGPUBuffer buffer)
{
    return ((RemoteHandle *)buffer)->size;
}

WGPUBufferUsage wgpuBufferGetUsage(WGPUBuffer buffer)
{
    return ((RemoteHandle *)buffer)->usage;
}

WGPUBufferMapState wgpuBufferGetMapState(WGPUBuffer buffer)
{
    return ((RemoteHandle *)buffer)->map_state;
}

void wgpuBufferSetLabel(WGPUBuffer b, WGPUStringView label) { set_label(b, label); }
void wgpuBufferAddRef(WGPUBuffer buffer) { rw_handle_addref(buffer); }
void wgpuBufferRelease(WGPUBuffer buffer) { rw_handle_release(buffer); }

/* ------------------------------------------------------------------ */
/* textures and views                                                 */
/* ------------------------------------------------------------------ */

WGPUTexture wgpuDeviceCreateTexture(WGPUDevice device,
                                    WGPUTextureDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;
    handle->width = descriptor->size.width;
    handle->height = descriptor->size.height;
    handle->depth_or_array_layers = descriptor->size.depthOrArrayLayers;
    handle->mip_level_count = descriptor->mipLevelCount ? descriptor->mipLevelCount : 1;
    handle->sample_count = descriptor->sampleCount ? descriptor->sampleCount : 1;
    handle->dimension = descriptor->dimension ? descriptor->dimension
                                              : WGPUTextureDimension_2D;
    handle->format = descriptor->format;
    handle->texture_usage = descriptor->usage;

    RemoteWebgpu__CreateTexture msg = REMOTE_WEBGPU__CREATE_TEXTURE__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.usage = (uint32_t)descriptor->usage;
    msg.dimension = (uint32_t)descriptor->dimension;
    msg.width = descriptor->size.width;
    msg.height = descriptor->size.height;
    msg.depth_or_array_layers = descriptor->size.depthOrArrayLayers;
    msg.format = (uint32_t)descriptor->format;
    msg.mip_level_count = descriptor->mipLevelCount;
    msg.sample_count = descriptor->sampleCount;
    uint32_t *view_formats = NULL;
    if (descriptor->viewFormatCount) {
        view_formats = calloc(descriptor->viewFormatCount, sizeof *view_formats);
        if (view_formats) {
            for (size_t i = 0; i < descriptor->viewFormatCount; ++i)
                view_formats[i] = (uint32_t)descriptor->viewFormats[i];
            msg.n_view_formats = descriptor->viewFormatCount;
            msg.view_formats = view_formats;
        }
    }
    SEND((RemoteDevice *)device, CREATE_TEXTURE, create_texture, &msg);
    free(view_formats);
    free(msg.label);
    return (WGPUTexture)handle;
}

WGPUTextureView wgpuTextureCreateView(WGPUTexture texture,
                                      WGPU_NULLABLE WGPUTextureViewDescriptor const *descriptor)
{
    RemoteHandle *self = (RemoteHandle *)texture;
    RemoteHandle *view = rw_handle_create(self->device);
    if (!view)
        return NULL;

    RemoteWebgpu__CreateTextureView msg = REMOTE_WEBGPU__CREATE_TEXTURE_VIEW__INIT;
    msg.id = view->id;
    msg.texture_id = self->id;
    msg.mip_level_count = WGPU_MIP_LEVEL_COUNT_UNDEFINED;
    msg.array_layer_count = WGPU_ARRAY_LAYER_COUNT_UNDEFINED;
    if (descriptor) {
        msg.label = rw_dup_stringview(descriptor->label);
        msg.format = (uint32_t)descriptor->format;
        msg.dimension = (uint32_t)descriptor->dimension;
        msg.base_mip_level = descriptor->baseMipLevel;
        msg.mip_level_count = descriptor->mipLevelCount;
        msg.base_array_layer = descriptor->baseArrayLayer;
        msg.array_layer_count = descriptor->arrayLayerCount;
        msg.aspect = (uint32_t)descriptor->aspect;
        msg.usage = (uint32_t)descriptor->usage;
    }
    SEND(self->device, CREATE_TEXTURE_VIEW, create_texture_view, &msg);
    if (descriptor)
        free(msg.label);
    return (WGPUTextureView)view;
}

WGPUFuture wgpuRemoteDeviceLoadTextureFromURL(WGPUDevice device,
                                              WGPUStringView url,
                                              WGPUTextureUsage usage,
                                              WGPURemoteTextureLoadCallbackInfo callbackInfo)
{
    RemoteDevice *self = (RemoteDevice *)device;
    RemoteAdapter *adapter = self->adapter;
    WGPUFuture future = { 0 };

    if (usage == WGPUTextureUsage_None)
        usage = WGPUTextureUsage_TextureBinding;
    /* copyExternalImageToTexture requires these client-side; mirror them
     * here so wgpuTextureGetUsage() agrees with the real texture. */
    usage |= WGPUTextureUsage_CopyDst | WGPUTextureUsage_RenderAttachment;

    RemoteHandle *texture = rw_handle_create(self);
    if (!texture)
        goto fail;
    /* Dimensions arrive with the client's reply; the rest is known now. */
    texture->depth_or_array_layers = 1;
    texture->mip_level_count = 1;
    texture->sample_count = 1;
    texture->dimension = WGPUTextureDimension_2D;
    texture->format = WGPUTextureFormat_RGBA8Unorm;
    texture->texture_usage = usage;

    RwRequest *request = rw_request_create(adapter, RW_REQUEST_TEXTURE_LOAD);
    if (!request) {
        wgpuTextureRelease((WGPUTexture)texture);
        goto fail;
    }
    request->cb.texture_load = callbackInfo;
    request->handle = texture;
    future.id = request->future_id;

    RemoteWebgpu__LoadTextureFromUrl msg = REMOTE_WEBGPU__LOAD_TEXTURE_FROM_URL__INIT;
    msg.request_id = request->request_id;
    msg.texture_id = texture->id;
    msg.url = rw_dup_stringview(url);
    msg.usage = (uint32_t)usage;
    msg.label = msg.url; /* the URL is the natural label */
    SEND(self, LOAD_TEXTURE_FROM_URL, load_texture_from_url, &msg);
    free(msg.url);
    return future;

fail:
    if (callbackInfo.callback) {
        WGPUStringView message = { "out of memory", 13 };
        callbackInfo.callback(WGPUStatus_Error, NULL, message,
                              callbackInfo.userdata1, callbackInfo.userdata2);
    }
    return future;
}

void wgpuTextureDestroy(WGPUTexture texture) { destroy_resource(texture); }

uint32_t wgpuTextureGetWidth(WGPUTexture texture) { return ((RemoteHandle *)texture)->width; }
uint32_t wgpuTextureGetHeight(WGPUTexture texture) { return ((RemoteHandle *)texture)->height; }

uint32_t wgpuTextureGetDepthOrArrayLayers(WGPUTexture texture)
{
    return ((RemoteHandle *)texture)->depth_or_array_layers;
}

uint32_t wgpuTextureGetMipLevelCount(WGPUTexture texture)
{
    return ((RemoteHandle *)texture)->mip_level_count;
}

uint32_t wgpuTextureGetSampleCount(WGPUTexture texture)
{
    return ((RemoteHandle *)texture)->sample_count;
}

WGPUTextureDimension wgpuTextureGetDimension(WGPUTexture texture)
{
    return ((RemoteHandle *)texture)->dimension;
}

WGPUTextureFormat wgpuTextureGetFormat(WGPUTexture texture)
{
    return ((RemoteHandle *)texture)->format;
}

WGPUTextureUsage wgpuTextureGetUsage(WGPUTexture texture)
{
    return ((RemoteHandle *)texture)->texture_usage;
}

WGPUTextureViewDimension wgpuTextureGetTextureBindingViewDimension(WGPUTexture texture)
{
    RemoteHandle *self = (RemoteHandle *)texture;
    switch (self->dimension) {
    case WGPUTextureDimension_1D: return WGPUTextureViewDimension_1D;
    case WGPUTextureDimension_3D: return WGPUTextureViewDimension_3D;
    default:
        return self->depth_or_array_layers > 1 ? WGPUTextureViewDimension_2DArray
                                               : WGPUTextureViewDimension_2D;
    }
}

void wgpuTextureSetLabel(WGPUTexture t, WGPUStringView label) { set_label(t, label); }
void wgpuTextureViewSetLabel(WGPUTextureView v, WGPUStringView label) { set_label(v, label); }
void wgpuTextureAddRef(WGPUTexture t) { rw_handle_addref(t); }
void wgpuTextureRelease(WGPUTexture t) { rw_handle_release(t); }
void wgpuTextureViewAddRef(WGPUTextureView v) { rw_handle_addref(v); }
void wgpuTextureViewRelease(WGPUTextureView v) { rw_handle_release(v); }

/* ------------------------------------------------------------------ */
/* samplers                                                           */
/* ------------------------------------------------------------------ */

WGPUSampler wgpuDeviceCreateSampler(WGPUDevice device,
                                    WGPU_NULLABLE WGPUSamplerDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    RemoteWebgpu__CreateSampler msg = REMOTE_WEBGPU__CREATE_SAMPLER__INIT;
    msg.id = handle->id;
    msg.lod_max_clamp = 32.0f;
    msg.max_anisotropy = 1;
    if (descriptor) {
        msg.label = rw_dup_stringview(descriptor->label);
        msg.address_mode_u = (uint32_t)descriptor->addressModeU;
        msg.address_mode_v = (uint32_t)descriptor->addressModeV;
        msg.address_mode_w = (uint32_t)descriptor->addressModeW;
        msg.mag_filter = (uint32_t)descriptor->magFilter;
        msg.min_filter = (uint32_t)descriptor->minFilter;
        msg.mipmap_filter = (uint32_t)descriptor->mipmapFilter;
        msg.lod_min_clamp = descriptor->lodMinClamp;
        msg.lod_max_clamp = descriptor->lodMaxClamp;
        msg.compare = (uint32_t)descriptor->compare;
        msg.max_anisotropy = descriptor->maxAnisotropy;
    }
    SEND((RemoteDevice *)device, CREATE_SAMPLER, create_sampler, &msg);
    if (descriptor)
        free(msg.label);
    return (WGPUSampler)handle;
}

void wgpuSamplerSetLabel(WGPUSampler s, WGPUStringView label) { set_label(s, label); }
void wgpuSamplerAddRef(WGPUSampler s) { rw_handle_addref(s); }
void wgpuSamplerRelease(WGPUSampler s) { rw_handle_release(s); }

/* ------------------------------------------------------------------ */
/* query sets                                                         */
/* ------------------------------------------------------------------ */

WGPUQuerySet wgpuDeviceCreateQuerySet(WGPUDevice device,
                                      WGPUQuerySetDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;
    handle->query_type = descriptor->type;
    handle->query_count = descriptor->count;

    RemoteWebgpu__CreateQuerySet msg = REMOTE_WEBGPU__CREATE_QUERY_SET__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.type = (uint32_t)descriptor->type;
    msg.count = descriptor->count;
    SEND((RemoteDevice *)device, CREATE_QUERY_SET, create_query_set, &msg);
    free(msg.label);
    return (WGPUQuerySet)handle;
}

void wgpuQuerySetDestroy(WGPUQuerySet querySet) { destroy_resource(querySet); }

WGPUQueryType wgpuQuerySetGetType(WGPUQuerySet querySet)
{
    return ((RemoteHandle *)querySet)->query_type;
}

uint32_t wgpuQuerySetGetCount(WGPUQuerySet querySet)
{
    return ((RemoteHandle *)querySet)->query_count;
}

void wgpuQuerySetSetLabel(WGPUQuerySet q, WGPUStringView label) { set_label(q, label); }
void wgpuQuerySetAddRef(WGPUQuerySet q) { rw_handle_addref(q); }
void wgpuQuerySetRelease(WGPUQuerySet q) { rw_handle_release(q); }

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
    RemoteWebgpu__BufferBindingLayout *buffers = calloc(n ? n : 1, sizeof *buffers);
    RemoteWebgpu__SamplerBindingLayout *samplers = calloc(n ? n : 1, sizeof *samplers);
    RemoteWebgpu__TextureBindingLayout *textures = calloc(n ? n : 1, sizeof *textures);
    RemoteWebgpu__StorageTextureBindingLayout *storages =
        calloc(n ? n : 1, sizeof *storages);
    for (size_t i = 0; i < n; ++i) {
        const WGPUBindGroupLayoutEntry *in = &descriptor->entries[i];
        remote_webgpu__bind_group_layout_entry__init(&entries[i]);
        entries[i].binding = in->binding;
        entries[i].visibility = (uint32_t)in->visibility;
        entries[i].binding_array_size = in->bindingArraySize;
        if (in->buffer.type != WGPUBufferBindingType_BindingNotUsed) {
            remote_webgpu__buffer_binding_layout__init(&buffers[i]);
            buffers[i].type = (uint32_t)in->buffer.type;
            buffers[i].has_dynamic_offset = in->buffer.hasDynamicOffset != 0;
            buffers[i].min_binding_size = in->buffer.minBindingSize;
            entries[i].buffer = &buffers[i];
        } else if (in->sampler.type != WGPUSamplerBindingType_BindingNotUsed) {
            remote_webgpu__sampler_binding_layout__init(&samplers[i]);
            samplers[i].type = (uint32_t)in->sampler.type;
            entries[i].sampler = &samplers[i];
        } else if (in->texture.sampleType != WGPUTextureSampleType_BindingNotUsed) {
            remote_webgpu__texture_binding_layout__init(&textures[i]);
            textures[i].sample_type = (uint32_t)in->texture.sampleType;
            textures[i].view_dimension = (uint32_t)in->texture.viewDimension;
            textures[i].multisampled = in->texture.multisampled != 0;
            entries[i].texture = &textures[i];
        } else if (in->storageTexture.access != WGPUStorageTextureAccess_BindingNotUsed) {
            remote_webgpu__storage_texture_binding_layout__init(&storages[i]);
            storages[i].access = (uint32_t)in->storageTexture.access;
            storages[i].format = (uint32_t)in->storageTexture.format;
            storages[i].view_dimension = (uint32_t)in->storageTexture.viewDimension;
            entries[i].storage_texture = &storages[i];
        } else {
            fprintf(stderr, "remote_webgpu: bind group layout entry %u uses no "
                            "binding kind\n", in->binding);
        }
        entry_ptrs[i] = &entries[i];
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
    free(buffers);
    free(samplers);
    free(textures);
    free(storages);
    return (WGPUBindGroupLayout)handle;
}

void wgpuBindGroupLayoutSetLabel(WGPUBindGroupLayout l, WGPUStringView label)
{
    set_label(l, label);
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
        if (in->buffer) {
            entries[i].buffer_id = handle_id(in->buffer);
            entries[i].offset = in->offset;
            entries[i].size = in->size;
        } else if (in->sampler) {
            entries[i].sampler_id = handle_id(in->sampler);
        } else if (in->textureView) {
            entries[i].texture_view_id = handle_id(in->textureView);
        }
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

void wgpuBindGroupSetLabel(WGPUBindGroup g, WGPUStringView label) { set_label(g, label); }
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

void wgpuPipelineLayoutSetLabel(WGPUPipelineLayout l, WGPUStringView label)
{
    set_label(l, label);
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

    /* fragment targets, including blend state */
    size_t ntgt = descriptor->fragment ? descriptor->fragment->targetCount : 0;
    RemoteWebgpu__ColorTargetState *targets = calloc(ntgt ? ntgt : 1, sizeof *targets);
    RemoteWebgpu__ColorTargetState **target_ptrs = calloc(ntgt ? ntgt : 1, sizeof *target_ptrs);
    RemoteWebgpu__BlendState *blends = calloc(ntgt ? ntgt : 1, sizeof *blends);
    RemoteWebgpu__BlendComponent *blend_comps = calloc(ntgt ? ntgt * 2 : 1,
                                                       sizeof *blend_comps);
    for (size_t i = 0; i < ntgt; ++i) {
        const WGPUColorTargetState *in = &descriptor->fragment->targets[i];
        remote_webgpu__color_target_state__init(&targets[i]);
        targets[i].format = (uint32_t)in->format;
        targets[i].write_mask = (uint32_t)in->writeMask;
        if (in->blend) {
            remote_webgpu__blend_state__init(&blends[i]);
            remote_webgpu__blend_component__init(&blend_comps[i * 2]);
            blend_comps[i * 2].operation = (uint32_t)in->blend->color.operation;
            blend_comps[i * 2].src_factor = (uint32_t)in->blend->color.srcFactor;
            blend_comps[i * 2].dst_factor = (uint32_t)in->blend->color.dstFactor;
            remote_webgpu__blend_component__init(&blend_comps[i * 2 + 1]);
            blend_comps[i * 2 + 1].operation = (uint32_t)in->blend->alpha.operation;
            blend_comps[i * 2 + 1].src_factor = (uint32_t)in->blend->alpha.srcFactor;
            blend_comps[i * 2 + 1].dst_factor = (uint32_t)in->blend->alpha.dstFactor;
            blends[i].color = &blend_comps[i * 2];
            blends[i].alpha = &blend_comps[i * 2 + 1];
            targets[i].blend = &blends[i];
        }
        target_ptrs[i] = &targets[i];
    }

    /* depth/stencil state */
    RemoteWebgpu__DepthStencilState depth_stencil = REMOTE_WEBGPU__DEPTH_STENCIL_STATE__INIT;
    RemoteWebgpu__StencilFaceState stencil_front = REMOTE_WEBGPU__STENCIL_FACE_STATE__INIT;
    RemoteWebgpu__StencilFaceState stencil_back = REMOTE_WEBGPU__STENCIL_FACE_STATE__INIT;
    if (descriptor->depthStencil) {
        const WGPUDepthStencilState *in = descriptor->depthStencil;
        depth_stencil.format = (uint32_t)in->format;
        depth_stencil.depth_write_enabled = (uint32_t)in->depthWriteEnabled;
        depth_stencil.depth_compare = (uint32_t)in->depthCompare;
        stencil_front.compare = (uint32_t)in->stencilFront.compare;
        stencil_front.fail_op = (uint32_t)in->stencilFront.failOp;
        stencil_front.depth_fail_op = (uint32_t)in->stencilFront.depthFailOp;
        stencil_front.pass_op = (uint32_t)in->stencilFront.passOp;
        stencil_back.compare = (uint32_t)in->stencilBack.compare;
        stencil_back.fail_op = (uint32_t)in->stencilBack.failOp;
        stencil_back.depth_fail_op = (uint32_t)in->stencilBack.depthFailOp;
        stencil_back.pass_op = (uint32_t)in->stencilBack.passOp;
        depth_stencil.stencil_front = &stencil_front;
        depth_stencil.stencil_back = &stencil_back;
        depth_stencil.stencil_read_mask = in->stencilReadMask;
        depth_stencil.stencil_write_mask = in->stencilWriteMask;
        depth_stencil.depth_bias = in->depthBias;
        depth_stencil.depth_bias_slope_scale = in->depthBiasSlopeScale;
        depth_stencil.depth_bias_clamp = in->depthBiasClamp;
    }

    /* pipeline-overridable constants */
    RemoteWebgpu__ConstantEntry **vertex_constants =
        build_constants(descriptor->vertex.constantCount, descriptor->vertex.constants);
    size_t fragment_constant_count =
        descriptor->fragment ? descriptor->fragment->constantCount : 0;
    RemoteWebgpu__ConstantEntry **fragment_constants = descriptor->fragment
        ? build_constants(fragment_constant_count, descriptor->fragment->constants)
        : NULL;

    RemoteWebgpu__CreateRenderPipeline msg = REMOTE_WEBGPU__CREATE_RENDER_PIPELINE__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.layout_id = handle_id(descriptor->layout);
    msg.vertex_module_id = handle_id(descriptor->vertex.module);
    msg.vertex_entry_point = rw_dup_stringview(descriptor->vertex.entryPoint);
    msg.n_vertex_buffers = nbuf;
    msg.vertex_buffers = layout_ptrs;
    if (vertex_constants) {
        msg.n_vertex_constants = descriptor->vertex.constantCount;
        msg.vertex_constants = vertex_constants;
    }
    msg.topology = (uint32_t)descriptor->primitive.topology;
    msg.strip_index_format = (uint32_t)descriptor->primitive.stripIndexFormat;
    msg.front_face = (uint32_t)descriptor->primitive.frontFace;
    msg.cull_mode = (uint32_t)descriptor->primitive.cullMode;
    msg.unclipped_depth = descriptor->primitive.unclippedDepth != 0;
    msg.multisample_count = descriptor->multisample.count;
    msg.multisample_mask = descriptor->multisample.mask;
    msg.alpha_to_coverage_enabled = descriptor->multisample.alphaToCoverageEnabled != 0;
    if (descriptor->depthStencil)
        msg.depth_stencil = &depth_stencil;
    if (descriptor->fragment) {
        msg.fragment_module_id = handle_id(descriptor->fragment->module);
        msg.fragment_entry_point = rw_dup_stringview(descriptor->fragment->entryPoint);
        if (fragment_constants) {
            msg.n_fragment_constants = fragment_constant_count;
            msg.fragment_constants = fragment_constants;
        }
    }
    msg.n_targets = ntgt;
    msg.targets = target_ptrs;
    SEND((RemoteDevice *)device, CREATE_RENDER_PIPELINE, create_render_pipeline, &msg);

    free(msg.label);
    free(msg.vertex_entry_point);
    if (descriptor->fragment)
        free(msg.fragment_entry_point);
    free_constants(vertex_constants, descriptor->vertex.constantCount);
    free_constants(fragment_constants, fragment_constant_count);
    for (size_t i = 0; i < nbuf; ++i) {
        free(attr_bases[i]);
        free(layouts[i].attributes);
    }
    free(attr_bases);
    free(layout_ptrs);
    free(layouts);
    free(target_ptrs);
    free(targets);
    free(blends);
    free(blend_comps);
    return (WGPURenderPipeline)handle;
}

WGPUFuture wgpuDeviceCreateRenderPipelineAsync(WGPUDevice device,
                                               WGPURenderPipelineDescriptor const *descriptor,
                                               WGPUCreateRenderPipelineAsyncCallbackInfo callbackInfo)
{
    RemoteAdapter *adapter = rw_device_adapter((RemoteDevice *)device);
    WGPUFuture future = { rw_next_future_id(adapter) };
    WGPURenderPipeline pipeline = wgpuDeviceCreateRenderPipeline(device, descriptor);
    /* Creation errors surface through error scopes; resolve immediately. */
    rw_future_complete(adapter->instance, future.id);
    if (callbackInfo.callback) {
        WGPUStringView none = { NULL, 0 };
        if (pipeline)
            callbackInfo.callback(WGPUCreatePipelineAsyncStatus_Success, pipeline,
                                  none, callbackInfo.userdata1, callbackInfo.userdata2);
        else
            callbackInfo.callback(WGPUCreatePipelineAsyncStatus_InternalError, NULL,
                                  none, callbackInfo.userdata1, callbackInfo.userdata2);
    }
    return future;
}

/* Bind `out_id` to a pipeline's implicit bind group layout. */
static WGPUBindGroupLayout pipeline_get_bind_group_layout(void *pipeline,
                                                          uint32_t groupIndex)
{
    RemoteHandle *self = pipeline;
    RemoteHandle *layout = rw_handle_create(self->device);
    if (!layout)
        return NULL;

    RemoteWebgpu__PipelineGetBindGroupLayout msg =
        REMOTE_WEBGPU__PIPELINE_GET_BIND_GROUP_LAYOUT__INIT;
    msg.pipeline_id = self->id;
    msg.group_index = groupIndex;
    msg.id = layout->id;
    SEND(self->device, PIPELINE_GET_BIND_GROUP_LAYOUT, pipeline_get_bind_group_layout,
         &msg);
    return (WGPUBindGroupLayout)layout;
}

WGPUBindGroupLayout wgpuRenderPipelineGetBindGroupLayout(WGPURenderPipeline pipeline,
                                                         uint32_t groupIndex)
{
    return pipeline_get_bind_group_layout(pipeline, groupIndex);
}

void wgpuRenderPipelineSetLabel(WGPURenderPipeline p, WGPUStringView label)
{
    set_label(p, label);
}
void wgpuRenderPipelineAddRef(WGPURenderPipeline p) { rw_handle_addref(p); }
void wgpuRenderPipelineRelease(WGPURenderPipeline p) { rw_handle_release(p); }

WGPUComputePipeline wgpuDeviceCreateComputePipeline(WGPUDevice device,
                                                    WGPUComputePipelineDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    RemoteWebgpu__ConstantEntry **constants =
        build_constants(descriptor->compute.constantCount, descriptor->compute.constants);

    RemoteWebgpu__CreateComputePipeline msg = REMOTE_WEBGPU__CREATE_COMPUTE_PIPELINE__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.layout_id = handle_id(descriptor->layout);
    msg.module_id = handle_id(descriptor->compute.module);
    msg.entry_point = rw_dup_stringview(descriptor->compute.entryPoint);
    if (constants) {
        msg.n_constants = descriptor->compute.constantCount;
        msg.constants = constants;
    }
    SEND((RemoteDevice *)device, CREATE_COMPUTE_PIPELINE, create_compute_pipeline, &msg);
    free(msg.label);
    free(msg.entry_point);
    free_constants(constants, descriptor->compute.constantCount);
    return (WGPUComputePipeline)handle;
}

WGPUFuture wgpuDeviceCreateComputePipelineAsync(WGPUDevice device,
                                                WGPUComputePipelineDescriptor const *descriptor,
                                                WGPUCreateComputePipelineAsyncCallbackInfo callbackInfo)
{
    RemoteAdapter *adapter = rw_device_adapter((RemoteDevice *)device);
    WGPUFuture future = { rw_next_future_id(adapter) };
    WGPUComputePipeline pipeline = wgpuDeviceCreateComputePipeline(device, descriptor);
    rw_future_complete(adapter->instance, future.id);
    if (callbackInfo.callback) {
        WGPUStringView none = { NULL, 0 };
        if (pipeline)
            callbackInfo.callback(WGPUCreatePipelineAsyncStatus_Success, pipeline,
                                  none, callbackInfo.userdata1, callbackInfo.userdata2);
        else
            callbackInfo.callback(WGPUCreatePipelineAsyncStatus_InternalError, NULL,
                                  none, callbackInfo.userdata1, callbackInfo.userdata2);
    }
    return future;
}

WGPUBindGroupLayout wgpuComputePipelineGetBindGroupLayout(WGPUComputePipeline pipeline,
                                                          uint32_t groupIndex)
{
    return pipeline_get_bind_group_layout(pipeline, groupIndex);
}

void wgpuComputePipelineSetLabel(WGPUComputePipeline p, WGPUStringView label)
{
    set_label(p, label);
}
void wgpuComputePipelineAddRef(WGPUComputePipeline p) { rw_handle_addref(p); }
void wgpuComputePipelineRelease(WGPUComputePipeline p) { rw_handle_release(p); }

/* ------------------------------------------------------------------ */
/* device error scopes / destruction                                  */
/* ------------------------------------------------------------------ */

void wgpuDevicePushErrorScope(WGPUDevice device, WGPUErrorFilter filter)
{
    RemoteWebgpu__PushErrorScope msg = REMOTE_WEBGPU__PUSH_ERROR_SCOPE__INIT;
    msg.filter = (uint32_t)filter;
    SEND((RemoteDevice *)device, PUSH_ERROR_SCOPE, push_error_scope, &msg);
}

WGPUFuture wgpuDevicePopErrorScope(WGPUDevice device,
                                   WGPUPopErrorScopeCallbackInfo callbackInfo)
{
    RemoteDevice *self = (RemoteDevice *)device;
    RemoteAdapter *adapter = self->adapter;
    WGPUFuture future = { 0 };

    RwRequest *request = rw_request_create(adapter, RW_REQUEST_POP_ERROR);
    if (!request)
        return future;
    request->cb.pop_error = callbackInfo;
    future.id = request->future_id;

    RemoteWebgpu__PopErrorScope msg = REMOTE_WEBGPU__POP_ERROR_SCOPE__INIT;
    msg.request_id = request->request_id;
    SEND(self, POP_ERROR_SCOPE, pop_error_scope, &msg);
    return future;
}

void wgpuDeviceDestroy(WGPUDevice device)
{
    RemoteDevice *self = (RemoteDevice *)device;
    RemoteWebgpu__DestroyDevice msg = REMOTE_WEBGPU__DESTROY_DEVICE__INIT;
    SEND(self, DESTROY_DEVICE, destroy_device, &msg);
    /* The client answers with DeviceLost(destroyed), which fires the lost
     * callback and completes the lost future. */
}

void wgpuDeviceSetLabel(WGPUDevice device, WGPUStringView label)
{
    (void)device; (void)label; /* the device has no client-side id */
}

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
    texture->depth_or_array_layers = 1;
    texture->mip_level_count = 1;
    texture->sample_count = 1;
    texture->dimension = WGPUTextureDimension_2D;
    texture->format = self->config.format;
    texture->texture_usage = self->config.usage;

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

    /* Fire and forget: the client acknowledges with PresentDone once the
     * frame is on screen, which completes the future handed out by
     * wgpuRemoteSurfaceOnNextVsync(). */
    RemoteWebgpu__Present msg = REMOTE_WEBGPU__PRESENT__INIT;
    SEND(self->device, PRESENT, present, &msg);
    return WGPUStatus_Success;
}

WGPUFuture wgpuRemoteSurfaceOnNextVsync(WGPUSurface surface,
                                        WGPURemoteVsyncCallbackInfo callbackInfo)
{
    RemoteSurface *self = (RemoteSurface *)surface;
    WGPUFuture future = { 0 };
    if (!self->device)
        return future;

    RemoteAdapter *adapter = rw_device_adapter(self->device);
    future.id = rw_next_future_id(adapter);
    if (adapter->vsync_pending)
        fprintf(stderr, "remote_webgpu: replacing an unfired vsync callback\n");
    adapter->vsync_callback = callbackInfo;
    adapter->vsync_pending = 1;
    return future;
}

void wgpuSurfaceSetLabel(WGPUSurface surface, WGPUStringView label)
{
    (void)surface; (void)label; /* the surface has no client-side id */
}

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

static void timestamp_writes_msg(RemoteWebgpu__PassTimestampWrites *out,
                                 const WGPUPassTimestampWrites *in)
{
    remote_webgpu__pass_timestamp_writes__init(out);
    out->query_set_id = handle_id(in->querySet);
    out->beginning_of_pass_write_index = in->beginningOfPassWriteIndex;
    out->end_of_pass_write_index = in->endOfPassWriteIndex;
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
        atts[i].resolve_target_id = handle_id(in->resolveTarget);
        atts[i].depth_slice = in->depthSlice;
        atts[i].load_op = (uint32_t)in->loadOp;
        atts[i].store_op = (uint32_t)in->storeOp;
        atts[i].clear_r = in->clearValue.r;
        atts[i].clear_g = in->clearValue.g;
        atts[i].clear_b = in->clearValue.b;
        atts[i].clear_a = in->clearValue.a;
        att_ptrs[i] = &atts[i];
    }

    RemoteWebgpu__DepthStencilAttachment depth_stencil =
        REMOTE_WEBGPU__DEPTH_STENCIL_ATTACHMENT__INIT;
    if (descriptor->depthStencilAttachment) {
        const WGPURenderPassDepthStencilAttachment *in =
            descriptor->depthStencilAttachment;
        depth_stencil.view_id = handle_id(in->view);
        depth_stencil.depth_load_op = (uint32_t)in->depthLoadOp;
        depth_stencil.depth_store_op = (uint32_t)in->depthStoreOp;
        depth_stencil.depth_clear_value = in->depthClearValue;
        depth_stencil.depth_read_only = in->depthReadOnly != 0;
        depth_stencil.stencil_load_op = (uint32_t)in->stencilLoadOp;
        depth_stencil.stencil_store_op = (uint32_t)in->stencilStoreOp;
        depth_stencil.stencil_clear_value = in->stencilClearValue;
        depth_stencil.stencil_read_only = in->stencilReadOnly != 0;
    }

    RemoteWebgpu__PassTimestampWrites timestamps;
    RemoteWebgpu__BeginRenderPass msg = REMOTE_WEBGPU__BEGIN_RENDER_PASS__INIT;
    msg.id = pass->id;
    msg.label = rw_dup_stringview(descriptor->label);
    msg.encoder_id = encoder->id;
    msg.n_color_attachments = n;
    msg.color_attachments = att_ptrs;
    if (descriptor->depthStencilAttachment)
        msg.depth_stencil_attachment = &depth_stencil;
    msg.occlusion_query_set_id = handle_id(descriptor->occlusionQuerySet);
    if (descriptor->timestampWrites) {
        timestamp_writes_msg(&timestamps, descriptor->timestampWrites);
        msg.timestamp_writes = &timestamps;
    }
    for (const WGPUChainedStruct *chain = descriptor->nextInChain; chain;
         chain = chain->next) {
        if (chain->sType == WGPUSType_RenderPassMaxDrawCount)
            msg.max_draw_count =
                ((const WGPURenderPassMaxDrawCount *)chain)->maxDrawCount;
    }
    SEND(encoder->device, BEGIN_RENDER_PASS, begin_render_pass, &msg);
    free(msg.label);
    free(att_ptrs);
    free(atts);
    return (WGPURenderPassEncoder)pass;
}

WGPUComputePassEncoder wgpuCommandEncoderBeginComputePass(WGPUCommandEncoder commandEncoder,
                                                          WGPU_NULLABLE WGPUComputePassDescriptor const *descriptor)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteHandle *pass = rw_handle_create(encoder->device);
    if (!pass)
        return NULL;

    RemoteWebgpu__PassTimestampWrites timestamps;
    RemoteWebgpu__BeginComputePass msg = REMOTE_WEBGPU__BEGIN_COMPUTE_PASS__INIT;
    msg.id = pass->id;
    msg.label = descriptor ? rw_dup_stringview(descriptor->label) : strdup("");
    msg.encoder_id = encoder->id;
    if (descriptor && descriptor->timestampWrites) {
        timestamp_writes_msg(&timestamps, descriptor->timestampWrites);
        msg.timestamp_writes = &timestamps;
    }
    SEND(encoder->device, BEGIN_COMPUTE_PASS, begin_compute_pass, &msg);
    free(msg.label);
    return (WGPUComputePassEncoder)pass;
}

/* --- commands shared by render passes, compute passes and bundles -- */

static void pass_set_pipeline(void *pass_handle, void *pipeline)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__SetPipeline msg = REMOTE_WEBGPU__SET_PIPELINE__INIT;
    msg.pass_id = pass->id;
    msg.pipeline_id = handle_id(pipeline);
    SEND(pass->device, SET_PIPELINE, set_pipeline, &msg);
}

static void pass_set_bind_group(void *pass_handle, uint32_t groupIndex, void *group,
                                size_t dynamicOffsetCount, uint32_t const *dynamicOffsets)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__SetBindGroup msg = REMOTE_WEBGPU__SET_BIND_GROUP__INIT;
    msg.pass_id = pass->id;
    msg.index = groupIndex;
    msg.bind_group_id = handle_id(group);
    msg.n_dynamic_offsets = dynamicOffsetCount;
    msg.dynamic_offsets = (uint32_t *)dynamicOffsets;
    SEND(pass->device, SET_BIND_GROUP, set_bind_group, &msg);
}

static void pass_set_vertex_buffer(void *pass_handle, uint32_t slot, void *buffer,
                                   uint64_t offset, uint64_t size)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__SetVertexBuffer msg = REMOTE_WEBGPU__SET_VERTEX_BUFFER__INIT;
    msg.pass_id = pass->id;
    msg.slot = slot;
    msg.buffer_id = handle_id(buffer);
    msg.offset = offset;
    msg.size = size;
    SEND(pass->device, SET_VERTEX_BUFFER, set_vertex_buffer, &msg);
}

static void pass_set_index_buffer(void *pass_handle, void *buffer,
                                  WGPUIndexFormat format, uint64_t offset, uint64_t size)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__SetIndexBuffer msg = REMOTE_WEBGPU__SET_INDEX_BUFFER__INIT;
    msg.pass_id = pass->id;
    msg.buffer_id = handle_id(buffer);
    msg.format = (uint32_t)format;
    msg.offset = offset;
    msg.size = size;
    SEND(pass->device, SET_INDEX_BUFFER, set_index_buffer, &msg);
}

static void pass_draw(void *pass_handle, uint32_t vertexCount, uint32_t instanceCount,
                      uint32_t firstVertex, uint32_t firstInstance)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__Draw msg = REMOTE_WEBGPU__DRAW__INIT;
    msg.pass_id = pass->id;
    msg.vertex_count = vertexCount;
    msg.instance_count = instanceCount;
    msg.first_vertex = firstVertex;
    msg.first_instance = firstInstance;
    SEND(pass->device, DRAW, draw, &msg);
}

static void pass_draw_indexed(void *pass_handle, uint32_t indexCount,
                              uint32_t instanceCount, uint32_t firstIndex,
                              int32_t baseVertex, uint32_t firstInstance)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__DrawIndexed msg = REMOTE_WEBGPU__DRAW_INDEXED__INIT;
    msg.pass_id = pass->id;
    msg.index_count = indexCount;
    msg.instance_count = instanceCount;
    msg.first_index = firstIndex;
    msg.base_vertex = baseVertex;
    msg.first_instance = firstInstance;
    SEND(pass->device, DRAW_INDEXED, draw_indexed, &msg);
}

static void pass_draw_indirect(void *pass_handle, void *buffer, uint64_t offset)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__DrawIndirect msg = REMOTE_WEBGPU__DRAW_INDIRECT__INIT;
    msg.pass_id = pass->id;
    msg.buffer_id = handle_id(buffer);
    msg.offset = offset;
    SEND(pass->device, DRAW_INDIRECT, draw_indirect, &msg);
}

static void pass_draw_indexed_indirect(void *pass_handle, void *buffer, uint64_t offset)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__DrawIndexedIndirect msg = REMOTE_WEBGPU__DRAW_INDEXED_INDIRECT__INIT;
    msg.pass_id = pass->id;
    msg.buffer_id = handle_id(buffer);
    msg.offset = offset;
    SEND(pass->device, DRAW_INDEXED_INDIRECT, draw_indexed_indirect, &msg);
}

static void pass_end(void *pass_handle)
{
    RemoteHandle *pass = pass_handle;
    RemoteWebgpu__EndPass msg = REMOTE_WEBGPU__END_PASS__INIT;
    msg.pass_id = pass->id;
    SEND(pass->device, END_PASS, end_pass, &msg);
}

/* --- render pass encoder ------------------------------------------ */

void wgpuRenderPassEncoderSetPipeline(WGPURenderPassEncoder renderPassEncoder,
                                      WGPURenderPipeline pipeline)
{
    pass_set_pipeline(renderPassEncoder, pipeline);
}

void wgpuRenderPassEncoderSetBindGroup(WGPURenderPassEncoder renderPassEncoder,
                                       uint32_t groupIndex, WGPU_NULLABLE WGPUBindGroup group,
                                       size_t dynamicOffsetCount, uint32_t const *dynamicOffsets)
{
    pass_set_bind_group(renderPassEncoder, groupIndex, group,
                        dynamicOffsetCount, dynamicOffsets);
}

void wgpuRenderPassEncoderSetVertexBuffer(WGPURenderPassEncoder renderPassEncoder,
                                          uint32_t slot, WGPU_NULLABLE WGPUBuffer buffer,
                                          uint64_t offset, uint64_t size)
{
    pass_set_vertex_buffer(renderPassEncoder, slot, buffer, offset, size);
}

void wgpuRenderPassEncoderSetIndexBuffer(WGPURenderPassEncoder renderPassEncoder,
                                         WGPUBuffer buffer, WGPUIndexFormat format,
                                         uint64_t offset, uint64_t size)
{
    pass_set_index_buffer(renderPassEncoder, buffer, format, offset, size);
}

void wgpuRenderPassEncoderDraw(WGPURenderPassEncoder renderPassEncoder, uint32_t vertexCount,
                               uint32_t instanceCount, uint32_t firstVertex,
                               uint32_t firstInstance)
{
    pass_draw(renderPassEncoder, vertexCount, instanceCount, firstVertex, firstInstance);
}

void wgpuRenderPassEncoderDrawIndexed(WGPURenderPassEncoder renderPassEncoder,
                                      uint32_t indexCount, uint32_t instanceCount,
                                      uint32_t firstIndex, int32_t baseVertex,
                                      uint32_t firstInstance)
{
    pass_draw_indexed(renderPassEncoder, indexCount, instanceCount, firstIndex,
                      baseVertex, firstInstance);
}

void wgpuRenderPassEncoderDrawIndirect(WGPURenderPassEncoder renderPassEncoder,
                                       WGPUBuffer indirectBuffer, uint64_t indirectOffset)
{
    pass_draw_indirect(renderPassEncoder, indirectBuffer, indirectOffset);
}

void wgpuRenderPassEncoderDrawIndexedIndirect(WGPURenderPassEncoder renderPassEncoder,
                                              WGPUBuffer indirectBuffer,
                                              uint64_t indirectOffset)
{
    pass_draw_indexed_indirect(renderPassEncoder, indirectBuffer, indirectOffset);
}

void wgpuRenderPassEncoderSetViewport(WGPURenderPassEncoder renderPassEncoder, float x,
                                      float y, float width, float height, float minDepth,
                                      float maxDepth)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetViewport msg = REMOTE_WEBGPU__SET_VIEWPORT__INIT;
    msg.pass_id = pass->id;
    msg.x = x;
    msg.y = y;
    msg.width = width;
    msg.height = height;
    msg.min_depth = minDepth;
    msg.max_depth = maxDepth;
    SEND(pass->device, SET_VIEWPORT, set_viewport, &msg);
}

void wgpuRenderPassEncoderSetScissorRect(WGPURenderPassEncoder renderPassEncoder,
                                         uint32_t x, uint32_t y, uint32_t width,
                                         uint32_t height)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetScissorRect msg = REMOTE_WEBGPU__SET_SCISSOR_RECT__INIT;
    msg.pass_id = pass->id;
    msg.x = x;
    msg.y = y;
    msg.width = width;
    msg.height = height;
    SEND(pass->device, SET_SCISSOR_RECT, set_scissor_rect, &msg);
}

void wgpuRenderPassEncoderSetBlendConstant(WGPURenderPassEncoder renderPassEncoder,
                                           WGPUColor const *color)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetBlendConstant msg = REMOTE_WEBGPU__SET_BLEND_CONSTANT__INIT;
    msg.pass_id = pass->id;
    msg.r = color->r;
    msg.g = color->g;
    msg.b = color->b;
    msg.a = color->a;
    SEND(pass->device, SET_BLEND_CONSTANT, set_blend_constant, &msg);
}

void wgpuRenderPassEncoderSetStencilReference(WGPURenderPassEncoder renderPassEncoder,
                                              uint32_t reference)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__SetStencilReference msg = REMOTE_WEBGPU__SET_STENCIL_REFERENCE__INIT;
    msg.pass_id = pass->id;
    msg.reference = reference;
    SEND(pass->device, SET_STENCIL_REFERENCE, set_stencil_reference, &msg);
}

void wgpuRenderPassEncoderBeginOcclusionQuery(WGPURenderPassEncoder renderPassEncoder,
                                              uint32_t queryIndex)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__BeginOcclusionQuery msg = REMOTE_WEBGPU__BEGIN_OCCLUSION_QUERY__INIT;
    msg.pass_id = pass->id;
    msg.query_index = queryIndex;
    SEND(pass->device, BEGIN_OCCLUSION_QUERY, begin_occlusion_query, &msg);
}

void wgpuRenderPassEncoderEndOcclusionQuery(WGPURenderPassEncoder renderPassEncoder)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    RemoteWebgpu__EndOcclusionQuery msg = REMOTE_WEBGPU__END_OCCLUSION_QUERY__INIT;
    msg.pass_id = pass->id;
    SEND(pass->device, END_OCCLUSION_QUERY, end_occlusion_query, &msg);
}

void wgpuRenderPassEncoderExecuteBundles(WGPURenderPassEncoder renderPassEncoder,
                                         size_t bundleCount,
                                         WGPURenderBundle const *bundles)
{
    RemoteHandle *pass = (RemoteHandle *)renderPassEncoder;
    uint32_t *ids = calloc(bundleCount ? bundleCount : 1, sizeof *ids);
    for (size_t i = 0; i < bundleCount; ++i)
        ids[i] = handle_id(bundles[i]);

    RemoteWebgpu__ExecuteBundles msg = REMOTE_WEBGPU__EXECUTE_BUNDLES__INIT;
    msg.pass_id = pass->id;
    msg.n_bundle_ids = bundleCount;
    msg.bundle_ids = ids;
    SEND(pass->device, EXECUTE_BUNDLES, execute_bundles, &msg);
    free(ids);
}

void wgpuRenderPassEncoderEnd(WGPURenderPassEncoder renderPassEncoder)
{
    pass_end(renderPassEncoder);
}

void wgpuRenderPassEncoderPushDebugGroup(WGPURenderPassEncoder e, WGPUStringView label)
{
    debug_marker(e, 1, label);
}
void wgpuRenderPassEncoderPopDebugGroup(WGPURenderPassEncoder e)
{
    WGPUStringView none = { NULL, 0 };
    debug_marker(e, 2, none);
}
void wgpuRenderPassEncoderInsertDebugMarker(WGPURenderPassEncoder e, WGPUStringView label)
{
    debug_marker(e, 3, label);
}

void wgpuRenderPassEncoderSetImmediates(WGPURenderPassEncoder renderPassEncoder,
                                        uint32_t offset, void const *data, size_t size)
{
    (void)renderPassEncoder; (void)offset; (void)data; (void)size;
    rw_warn_unsupported("wgpuRenderPassEncoderSetImmediates (no JS equivalent)");
}

void wgpuRenderPassEncoderSetLabel(WGPURenderPassEncoder p, WGPUStringView label)
{
    set_label(p, label);
}
void wgpuRenderPassEncoderAddRef(WGPURenderPassEncoder p) { rw_handle_addref(p); }
void wgpuRenderPassEncoderRelease(WGPURenderPassEncoder p) { rw_handle_release(p); }

/* --- compute pass encoder ----------------------------------------- */

void wgpuComputePassEncoderSetPipeline(WGPUComputePassEncoder computePassEncoder,
                                       WGPUComputePipeline pipeline)
{
    pass_set_pipeline(computePassEncoder, pipeline);
}

void wgpuComputePassEncoderSetBindGroup(WGPUComputePassEncoder computePassEncoder,
                                        uint32_t groupIndex, WGPU_NULLABLE WGPUBindGroup group,
                                        size_t dynamicOffsetCount,
                                        uint32_t const *dynamicOffsets)
{
    pass_set_bind_group(computePassEncoder, groupIndex, group,
                        dynamicOffsetCount, dynamicOffsets);
}

void wgpuComputePassEncoderDispatchWorkgroups(WGPUComputePassEncoder computePassEncoder,
                                              uint32_t workgroupCountX,
                                              uint32_t workgroupCountY,
                                              uint32_t workgroupCountZ)
{
    RemoteHandle *pass = (RemoteHandle *)computePassEncoder;
    RemoteWebgpu__DispatchWorkgroups msg = REMOTE_WEBGPU__DISPATCH_WORKGROUPS__INIT;
    msg.pass_id = pass->id;
    msg.x = workgroupCountX;
    msg.y = workgroupCountY;
    msg.z = workgroupCountZ;
    SEND(pass->device, DISPATCH_WORKGROUPS, dispatch_workgroups, &msg);
}

void wgpuComputePassEncoderDispatchWorkgroupsIndirect(WGPUComputePassEncoder computePassEncoder,
                                                      WGPUBuffer indirectBuffer,
                                                      uint64_t indirectOffset)
{
    RemoteHandle *pass = (RemoteHandle *)computePassEncoder;
    RemoteWebgpu__DispatchWorkgroupsIndirect msg =
        REMOTE_WEBGPU__DISPATCH_WORKGROUPS_INDIRECT__INIT;
    msg.pass_id = pass->id;
    msg.buffer_id = handle_id(indirectBuffer);
    msg.offset = indirectOffset;
    SEND(pass->device, DISPATCH_WORKGROUPS_INDIRECT, dispatch_workgroups_indirect, &msg);
}

void wgpuComputePassEncoderEnd(WGPUComputePassEncoder computePassEncoder)
{
    pass_end(computePassEncoder);
}

void wgpuComputePassEncoderPushDebugGroup(WGPUComputePassEncoder e, WGPUStringView label)
{
    debug_marker(e, 1, label);
}
void wgpuComputePassEncoderPopDebugGroup(WGPUComputePassEncoder e)
{
    WGPUStringView none = { NULL, 0 };
    debug_marker(e, 2, none);
}
void wgpuComputePassEncoderInsertDebugMarker(WGPUComputePassEncoder e, WGPUStringView label)
{
    debug_marker(e, 3, label);
}

void wgpuComputePassEncoderSetImmediates(WGPUComputePassEncoder computePassEncoder,
                                         uint32_t offset, void const *data, size_t size)
{
    (void)computePassEncoder; (void)offset; (void)data; (void)size;
    rw_warn_unsupported("wgpuComputePassEncoderSetImmediates (no JS equivalent)");
}

void wgpuComputePassEncoderSetLabel(WGPUComputePassEncoder p, WGPUStringView label)
{
    set_label(p, label);
}
void wgpuComputePassEncoderAddRef(WGPUComputePassEncoder p) { rw_handle_addref(p); }
void wgpuComputePassEncoderRelease(WGPUComputePassEncoder p) { rw_handle_release(p); }

/* --- render bundles ------------------------------------------------ */

WGPURenderBundleEncoder wgpuDeviceCreateRenderBundleEncoder(WGPUDevice device,
                                                            WGPURenderBundleEncoderDescriptor const *descriptor)
{
    RemoteHandle *handle = rw_handle_create((RemoteDevice *)device);
    if (!handle)
        return NULL;

    RemoteWebgpu__CreateRenderBundleEncoder msg =
        REMOTE_WEBGPU__CREATE_RENDER_BUNDLE_ENCODER__INIT;
    msg.id = handle->id;
    msg.label = rw_dup_stringview(descriptor->label);
    uint32_t *formats = NULL;
    if (descriptor->colorFormatCount) {
        formats = calloc(descriptor->colorFormatCount, sizeof *formats);
        if (formats) {
            for (size_t i = 0; i < descriptor->colorFormatCount; ++i)
                formats[i] = (uint32_t)descriptor->colorFormats[i];
            msg.n_color_formats = descriptor->colorFormatCount;
            msg.color_formats = formats;
        }
    }
    msg.depth_stencil_format = (uint32_t)descriptor->depthStencilFormat;
    msg.sample_count = descriptor->sampleCount;
    msg.depth_read_only = descriptor->depthReadOnly != 0;
    msg.stencil_read_only = descriptor->stencilReadOnly != 0;
    SEND((RemoteDevice *)device, CREATE_RENDER_BUNDLE_ENCODER,
         create_render_bundle_encoder, &msg);
    free(formats);
    free(msg.label);
    return (WGPURenderBundleEncoder)handle;
}

WGPURenderBundle wgpuRenderBundleEncoderFinish(WGPURenderBundleEncoder renderBundleEncoder,
                                               WGPU_NULLABLE WGPURenderBundleDescriptor const *descriptor)
{
    RemoteHandle *encoder = (RemoteHandle *)renderBundleEncoder;
    RemoteHandle *bundle = rw_handle_create(encoder->device);
    if (!bundle)
        return NULL;

    RemoteWebgpu__FinishRenderBundle msg = REMOTE_WEBGPU__FINISH_RENDER_BUNDLE__INIT;
    msg.encoder_id = encoder->id;
    msg.bundle_id = bundle->id;
    msg.label = descriptor ? rw_dup_stringview(descriptor->label) : strdup("");
    SEND(encoder->device, FINISH_RENDER_BUNDLE, finish_render_bundle, &msg);
    free(msg.label);
    return (WGPURenderBundle)bundle;
}

void wgpuRenderBundleEncoderSetPipeline(WGPURenderBundleEncoder e, WGPURenderPipeline pipeline)
{
    pass_set_pipeline(e, pipeline);
}

void wgpuRenderBundleEncoderSetBindGroup(WGPURenderBundleEncoder e, uint32_t groupIndex,
                                         WGPU_NULLABLE WGPUBindGroup group,
                                         size_t dynamicOffsetCount,
                                         uint32_t const *dynamicOffsets)
{
    pass_set_bind_group(e, groupIndex, group, dynamicOffsetCount, dynamicOffsets);
}

void wgpuRenderBundleEncoderSetVertexBuffer(WGPURenderBundleEncoder e, uint32_t slot,
                                            WGPU_NULLABLE WGPUBuffer buffer,
                                            uint64_t offset, uint64_t size)
{
    pass_set_vertex_buffer(e, slot, buffer, offset, size);
}

void wgpuRenderBundleEncoderSetIndexBuffer(WGPURenderBundleEncoder e, WGPUBuffer buffer,
                                           WGPUIndexFormat format, uint64_t offset,
                                           uint64_t size)
{
    pass_set_index_buffer(e, buffer, format, offset, size);
}

void wgpuRenderBundleEncoderDraw(WGPURenderBundleEncoder e, uint32_t vertexCount,
                                 uint32_t instanceCount, uint32_t firstVertex,
                                 uint32_t firstInstance)
{
    pass_draw(e, vertexCount, instanceCount, firstVertex, firstInstance);
}

void wgpuRenderBundleEncoderDrawIndexed(WGPURenderBundleEncoder e, uint32_t indexCount,
                                        uint32_t instanceCount, uint32_t firstIndex,
                                        int32_t baseVertex, uint32_t firstInstance)
{
    pass_draw_indexed(e, indexCount, instanceCount, firstIndex, baseVertex, firstInstance);
}

void wgpuRenderBundleEncoderDrawIndirect(WGPURenderBundleEncoder e,
                                         WGPUBuffer indirectBuffer, uint64_t indirectOffset)
{
    pass_draw_indirect(e, indirectBuffer, indirectOffset);
}

void wgpuRenderBundleEncoderDrawIndexedIndirect(WGPURenderBundleEncoder e,
                                                WGPUBuffer indirectBuffer,
                                                uint64_t indirectOffset)
{
    pass_draw_indexed_indirect(e, indirectBuffer, indirectOffset);
}

void wgpuRenderBundleEncoderPushDebugGroup(WGPURenderBundleEncoder e, WGPUStringView label)
{
    debug_marker(e, 1, label);
}
void wgpuRenderBundleEncoderPopDebugGroup(WGPURenderBundleEncoder e)
{
    WGPUStringView none = { NULL, 0 };
    debug_marker(e, 2, none);
}
void wgpuRenderBundleEncoderInsertDebugMarker(WGPURenderBundleEncoder e,
                                              WGPUStringView label)
{
    debug_marker(e, 3, label);
}

void wgpuRenderBundleEncoderSetImmediates(WGPURenderBundleEncoder renderBundleEncoder,
                                          uint32_t offset, void const *data, size_t size)
{
    (void)renderBundleEncoder; (void)offset; (void)data; (void)size;
    rw_warn_unsupported("wgpuRenderBundleEncoderSetImmediates (no JS equivalent)");
}

void wgpuRenderBundleEncoderSetLabel(WGPURenderBundleEncoder e, WGPUStringView label)
{
    set_label(e, label);
}
void wgpuRenderBundleEncoderAddRef(WGPURenderBundleEncoder e) { rw_handle_addref(e); }
void wgpuRenderBundleEncoderRelease(WGPURenderBundleEncoder e) { rw_handle_release(e); }

void wgpuRenderBundleSetLabel(WGPURenderBundle b, WGPUStringView label)
{
    set_label(b, label);
}
void wgpuRenderBundleAddRef(WGPURenderBundle b) { rw_handle_addref(b); }
void wgpuRenderBundleRelease(WGPURenderBundle b) { rw_handle_release(b); }

/* --- copies and queries on the command encoder --------------------- */

void wgpuCommandEncoderCopyBufferToBuffer(WGPUCommandEncoder commandEncoder,
                                          WGPUBuffer source, uint64_t sourceOffset,
                                          WGPUBuffer destination, uint64_t destinationOffset,
                                          uint64_t size)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__CopyBufferToBuffer msg = REMOTE_WEBGPU__COPY_BUFFER_TO_BUFFER__INIT;
    msg.encoder_id = encoder->id;
    msg.source_id = handle_id(source);
    msg.source_offset = sourceOffset;
    msg.destination_id = handle_id(destination);
    msg.destination_offset = destinationOffset;
    msg.size = size;
    SEND(encoder->device, COPY_BUFFER_TO_BUFFER, copy_buffer_to_buffer, &msg);
}

void wgpuCommandEncoderCopyBufferToTexture(WGPUCommandEncoder commandEncoder,
                                           WGPUTexelCopyBufferInfo const *source,
                                           WGPUTexelCopyTextureInfo const *destination,
                                           WGPUExtent3D const *copySize)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__TexelCopyBuffer src;
    RemoteWebgpu__TexelCopyTexture dst;
    RemoteWebgpu__Extent3D size;
    texel_copy_buffer(&src, source);
    texel_copy_texture(&dst, destination);
    extent3d(&size, copySize);

    RemoteWebgpu__CopyBufferToTexture msg = REMOTE_WEBGPU__COPY_BUFFER_TO_TEXTURE__INIT;
    msg.encoder_id = encoder->id;
    msg.source = &src;
    msg.destination = &dst;
    msg.size = &size;
    SEND(encoder->device, COPY_BUFFER_TO_TEXTURE, copy_buffer_to_texture, &msg);
}

void wgpuCommandEncoderCopyTextureToBuffer(WGPUCommandEncoder commandEncoder,
                                           WGPUTexelCopyTextureInfo const *source,
                                           WGPUTexelCopyBufferInfo const *destination,
                                           WGPUExtent3D const *copySize)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__TexelCopyTexture src;
    RemoteWebgpu__TexelCopyBuffer dst;
    RemoteWebgpu__Extent3D size;
    texel_copy_texture(&src, source);
    texel_copy_buffer(&dst, destination);
    extent3d(&size, copySize);

    RemoteWebgpu__CopyTextureToBuffer msg = REMOTE_WEBGPU__COPY_TEXTURE_TO_BUFFER__INIT;
    msg.encoder_id = encoder->id;
    msg.source = &src;
    msg.destination = &dst;
    msg.size = &size;
    SEND(encoder->device, COPY_TEXTURE_TO_BUFFER, copy_texture_to_buffer, &msg);
}

void wgpuCommandEncoderCopyTextureToTexture(WGPUCommandEncoder commandEncoder,
                                            WGPUTexelCopyTextureInfo const *source,
                                            WGPUTexelCopyTextureInfo const *destination,
                                            WGPUExtent3D const *copySize)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__TexelCopyTexture src;
    RemoteWebgpu__TexelCopyTexture dst;
    RemoteWebgpu__Extent3D size;
    texel_copy_texture(&src, source);
    texel_copy_texture(&dst, destination);
    extent3d(&size, copySize);

    RemoteWebgpu__CopyTextureToTexture msg = REMOTE_WEBGPU__COPY_TEXTURE_TO_TEXTURE__INIT;
    msg.encoder_id = encoder->id;
    msg.source = &src;
    msg.destination = &dst;
    msg.size = &size;
    SEND(encoder->device, COPY_TEXTURE_TO_TEXTURE, copy_texture_to_texture, &msg);
}

void wgpuCommandEncoderClearBuffer(WGPUCommandEncoder commandEncoder, WGPUBuffer buffer,
                                   uint64_t offset, uint64_t size)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__ClearBuffer msg = REMOTE_WEBGPU__CLEAR_BUFFER__INIT;
    msg.encoder_id = encoder->id;
    msg.buffer_id = handle_id(buffer);
    msg.offset = offset;
    msg.size = size;
    SEND(encoder->device, CLEAR_BUFFER, clear_buffer, &msg);
}

void wgpuCommandEncoderResolveQuerySet(WGPUCommandEncoder commandEncoder,
                                       WGPUQuerySet querySet, uint32_t firstQuery,
                                       uint32_t queryCount, WGPUBuffer destination,
                                       uint64_t destinationOffset)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__ResolveQuerySet msg = REMOTE_WEBGPU__RESOLVE_QUERY_SET__INIT;
    msg.encoder_id = encoder->id;
    msg.query_set_id = handle_id(querySet);
    msg.first_query = firstQuery;
    msg.query_count = queryCount;
    msg.destination_id = handle_id(destination);
    msg.destination_offset = destinationOffset;
    SEND(encoder->device, RESOLVE_QUERY_SET, resolve_query_set, &msg);
}

void wgpuCommandEncoderWriteTimestamp(WGPUCommandEncoder commandEncoder,
                                      WGPUQuerySet querySet, uint32_t queryIndex)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteWebgpu__WriteTimestamp msg = REMOTE_WEBGPU__WRITE_TIMESTAMP__INIT;
    msg.encoder_id = encoder->id;
    msg.query_set_id = handle_id(querySet);
    msg.query_index = queryIndex;
    SEND(encoder->device, WRITE_TIMESTAMP, write_timestamp, &msg);
}

void wgpuCommandEncoderPushDebugGroup(WGPUCommandEncoder e, WGPUStringView label)
{
    debug_marker(e, 1, label);
}
void wgpuCommandEncoderPopDebugGroup(WGPUCommandEncoder e)
{
    WGPUStringView none = { NULL, 0 };
    debug_marker(e, 2, none);
}
void wgpuCommandEncoderInsertDebugMarker(WGPUCommandEncoder e, WGPUStringView label)
{
    debug_marker(e, 3, label);
}

WGPUCommandBuffer wgpuCommandEncoderFinish(WGPUCommandEncoder commandEncoder,
                                           WGPU_NULLABLE WGPUCommandBufferDescriptor const *descriptor)
{
    RemoteHandle *encoder = (RemoteHandle *)commandEncoder;
    RemoteHandle *commands = rw_handle_create(encoder->device);
    if (!commands)
        return NULL;

    RemoteWebgpu__FinishEncoder msg = REMOTE_WEBGPU__FINISH_ENCODER__INIT;
    msg.encoder_id = encoder->id;
    msg.command_buffer_id = commands->id;
    msg.label = descriptor ? rw_dup_stringview(descriptor->label) : strdup("");
    SEND(encoder->device, FINISH_ENCODER, finish_encoder, &msg);
    free(msg.label);
    return (WGPUCommandBuffer)commands;
}

void wgpuCommandEncoderSetLabel(WGPUCommandEncoder e, WGPUStringView label)
{
    set_label(e, label);
}
void wgpuCommandEncoderAddRef(WGPUCommandEncoder e) { rw_handle_addref(e); }
void wgpuCommandEncoderRelease(WGPUCommandEncoder e) { rw_handle_release(e); }

void wgpuCommandBufferSetLabel(WGPUCommandBuffer b, WGPUStringView label)
{
    set_label(b, label);
}
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

void wgpuQueueWriteTexture(WGPUQueue queue, WGPUTexelCopyTextureInfo const *destination,
                           void const *data, size_t dataSize,
                           WGPUTexelCopyBufferLayout const *dataLayout,
                           WGPUExtent3D const *writeSize)
{
    RemoteQueue *self = (RemoteQueue *)queue;
    RemoteWebgpu__TexelCopyTexture dst;
    RemoteWebgpu__Extent3D size;
    texel_copy_texture(&dst, destination);
    extent3d(&size, writeSize);

    RemoteWebgpu__WriteTexture msg = REMOTE_WEBGPU__WRITE_TEXTURE__INIT;
    msg.destination = &dst;
    msg.data.data = (uint8_t *)data;
    msg.data.len = dataSize;
    msg.layout_offset = dataLayout->offset;
    msg.bytes_per_row = dataLayout->bytesPerRow;
    msg.rows_per_image = dataLayout->rowsPerImage;
    msg.size = &size;
    SEND(self->device, WRITE_TEXTURE, write_texture, &msg);
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

WGPUFuture wgpuQueueOnSubmittedWorkDone(WGPUQueue queue,
                                        WGPUQueueWorkDoneCallbackInfo callbackInfo)
{
    RemoteQueue *self = (RemoteQueue *)queue;
    RemoteAdapter *adapter = rw_device_adapter(self->device);
    WGPUFuture future = { 0 };

    RwRequest *request = rw_request_create(adapter, RW_REQUEST_WORK_DONE);
    if (!request)
        return future;
    request->cb.work_done = callbackInfo;
    future.id = request->future_id;

    RemoteWebgpu__OnSubmittedWorkDone msg = REMOTE_WEBGPU__ON_SUBMITTED_WORK_DONE__INIT;
    msg.request_id = request->request_id;
    SEND(self->device, ON_SUBMITTED_WORK_DONE, on_submitted_work_done, &msg);
    return future;
}

void wgpuQueueSetLabel(WGPUQueue queue, WGPUStringView label)
{
    (void)queue; (void)label; /* the queue has no client-side id */
}
