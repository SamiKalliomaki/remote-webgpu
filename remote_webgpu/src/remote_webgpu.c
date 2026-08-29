/*
 * Hand-written core of the remote WebGPU implementation: object lifetime,
 * the socket-backed adapter/device bring-up, and the dispatch of client
 * replies (buffer maps, error scopes, work-done, compilation info, device
 * loss).  The webgpu.h methods that translate into protocol commands live
 * in remote_methods.c; anything left over is a generated stub in stubs.c
 * that aborts with a clear message.
 */

#include "remote_webgpu_internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <webgpu/remote.h>
#include <webgpu/wgpu.h>

#include "remote_webgpu.pb-c.h"

static void *alloc_object(size_t size);
static int release_object(RemoteObject *obj);

_Noreturn void remote_wgpu_unimplemented(const char *name)
{
    fprintf(stderr, "remote_webgpu: unimplemented: %s\n", name);
    abort();
}

void rw_warn_unsupported(const char *what)
{
    fprintf(stderr, "remote_webgpu: %s is not supported; ignored\n", what);
}

static WGPUStringView sv(const char *s)
{
    WGPUStringView v = { s, s ? strlen(s) : 0 };
    return v;
}

static char *dup_or_empty(const char *s)
{
    return strdup(s ? s : "");
}

char *rw_dup_stringview(WGPUStringView s)
{
    char *out = malloc(s.length + 1);
    if (!out)
        return NULL;
    memcpy(out, s.data ? s.data : "", s.length);
    out[s.length] = '\0';
    return out;
}

/* ------------------------------------------------------------------ */
/* protobuf envelopes over the websocket                              */
/* ------------------------------------------------------------------ */

void rw_send_envelope(RemoteAdapter *adapter, const RemoteWebgpu__Envelope *envelope)
{
    size_t len = remote_webgpu__envelope__get_packed_size(envelope);
    uint8_t *buf = malloc(len ? len : 1);
    if (!buf)
        return;
    remote_webgpu__envelope__pack(envelope, buf);
    adapter->send(buf, len, adapter->send_userdata);
    free(buf);
}

RemoteAdapter *rw_device_adapter(RemoteDevice *device)
{
    return device->adapter;
}

RemoteHandle *rw_handle_create(RemoteDevice *device)
{
    RemoteHandle *handle = alloc_object(sizeof *handle);
    if (!handle)
        return NULL;
    handle->device = device;
    wgpuDeviceAddRef((WGPUDevice)device);
    handle->id = device->adapter->next_id++;
    handle->map_state = WGPUBufferMapState_Unmapped;
    return handle;
}

void rw_handle_addref(void *handle)
{
    ((RemoteHandle *)handle)->obj.refcount++;
}

void rw_handle_release(void *handle)
{
    RemoteHandle *self = handle;
    if (!release_object(&self->obj))
        return;

    RemoteWebgpu__DestroyObject destroy = REMOTE_WEBGPU__DESTROY_OBJECT__INIT;
    destroy.id = self->id;
    RemoteWebgpu__Envelope envelope = REMOTE_WEBGPU__ENVELOPE__INIT;
    envelope.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_DESTROY_OBJECT;
    envelope.destroy_object = &destroy;
    rw_send_envelope(self->device->adapter, &envelope);

    free(self->mapped);
    wgpuDeviceRelease((WGPUDevice)self->device);
    free(self);
}

/* ------------------------------------------------------------------ */
/* futures and outstanding requests                                   */
/* ------------------------------------------------------------------ */

void rw_future_complete(RemoteInstance *instance, uint64_t future_id)
{
    if (!future_id)
        return;
    if (instance->completed_count == instance->completed_capacity) {
        size_t capacity = instance->completed_capacity ? instance->completed_capacity * 2 : 16;
        uint64_t *grown = realloc(instance->completed_futures,
                                  capacity * sizeof *grown);
        if (!grown)
            return;
        instance->completed_futures = grown;
        instance->completed_capacity = capacity;
    }
    instance->completed_futures[instance->completed_count++] = future_id;
}

static int future_is_complete(RemoteInstance *instance, uint64_t future_id)
{
    for (size_t i = 0; i < instance->completed_count; ++i)
        if (instance->completed_futures[i] == future_id)
            return 1;
    return 0;
}

uint64_t rw_next_future_id(RemoteAdapter *adapter)
{
    return adapter->next_future_id++;
}

RwRequest *rw_request_create(RemoteAdapter *adapter, RwRequestType type)
{
    RwRequest *request = calloc(1, sizeof *request);
    if (!request)
        return NULL;
    request->request_id = adapter->next_request_id++;
    request->future_id = rw_next_future_id(adapter);
    request->type = type;
    request->next = adapter->requests;
    adapter->requests = request;
    return request;
}

/* Detach the request with `request_id` of `type`; NULL if unknown. */
static RwRequest *request_take(RemoteAdapter *adapter, uint64_t request_id,
                               RwRequestType type)
{
    for (RwRequest **link = &adapter->requests; *link; link = &(*link)->next) {
        RwRequest *request = *link;
        if (request->request_id == request_id && request->type == type) {
            *link = request->next;
            return request;
        }
    }
    return NULL;
}

/* Complete the request's future and free it. */
static void request_finish(RemoteAdapter *adapter, RwRequest *request)
{
    rw_future_complete(adapter->instance, request->future_id);
    free(request);
}

/* ------------------------------------------------------------------ */
/* client -> server message handlers                                  */
/* ------------------------------------------------------------------ */

static void limits_from_message(WGPULimits *out, const RemoteWebgpu__Limits *in)
{
    memset(out, 0, sizeof *out);
    if (!in)
        return;
    out->maxTextureDimension1D = in->max_texture_dimension_1d;
    out->maxTextureDimension2D = in->max_texture_dimension_2d;
    out->maxTextureDimension3D = in->max_texture_dimension_3d;
    out->maxTextureArrayLayers = in->max_texture_array_layers;
    out->maxBindGroups = in->max_bind_groups;
    out->maxBindGroupsPlusVertexBuffers = in->max_bind_groups_plus_vertex_buffers;
    out->maxBindingsPerBindGroup = in->max_bindings_per_bind_group;
    out->maxDynamicUniformBuffersPerPipelineLayout =
        in->max_dynamic_uniform_buffers_per_pipeline_layout;
    out->maxDynamicStorageBuffersPerPipelineLayout =
        in->max_dynamic_storage_buffers_per_pipeline_layout;
    out->maxSampledTexturesPerShaderStage = in->max_sampled_textures_per_shader_stage;
    out->maxSamplersPerShaderStage = in->max_samplers_per_shader_stage;
    out->maxStorageBuffersPerShaderStage = in->max_storage_buffers_per_shader_stage;
    out->maxStorageTexturesPerShaderStage = in->max_storage_textures_per_shader_stage;
    out->maxUniformBuffersPerShaderStage = in->max_uniform_buffers_per_shader_stage;
    out->maxUniformBufferBindingSize = in->max_uniform_buffer_binding_size;
    out->maxStorageBufferBindingSize = in->max_storage_buffer_binding_size;
    out->minUniformBufferOffsetAlignment = in->min_uniform_buffer_offset_alignment;
    out->minStorageBufferOffsetAlignment = in->min_storage_buffer_offset_alignment;
    out->maxVertexBuffers = in->max_vertex_buffers;
    out->maxBufferSize = in->max_buffer_size;
    out->maxVertexAttributes = in->max_vertex_attributes;
    out->maxVertexBufferArrayStride = in->max_vertex_buffer_array_stride;
    out->maxInterStageShaderVariables = in->max_inter_stage_shader_variables;
    out->maxColorAttachments = in->max_color_attachments;
    out->maxColorAttachmentBytesPerSample = in->max_color_attachment_bytes_per_sample;
    out->maxComputeWorkgroupStorageSize = in->max_compute_workgroup_storage_size;
    out->maxComputeInvocationsPerWorkgroup = in->max_compute_invocations_per_workgroup;
    out->maxComputeWorkgroupSizeX = in->max_compute_workgroup_size_x;
    out->maxComputeWorkgroupSizeY = in->max_compute_workgroup_size_y;
    out->maxComputeWorkgroupSizeZ = in->max_compute_workgroup_size_z;
    out->maxComputeWorkgroupsPerDimension = in->max_compute_workgroups_per_dimension;
    out->maxImmediateSize = in->max_immediate_size;
}

static void handle_client_hello(RemoteAdapter *adapter,
                                const RemoteWebgpu__ClientHello *hello)
{
    if (hello->protocol_version
        != REMOTE_WEBGPU__PROTOCOL_VERSION__PROTOCOL_VERSION_CURRENT) {
        fprintf(stderr, "remote_webgpu: protocol version mismatch (client %u)\n",
                hello->protocol_version);
        adapter->failed = 1;
        return;
    }

    const RemoteWebgpu__AdapterInfo *info = hello->adapter;
    adapter->vendor = dup_or_empty(info ? info->vendor : NULL);
    adapter->architecture = dup_or_empty(info ? info->architecture : NULL);
    adapter->device = dup_or_empty(info ? info->device : NULL);
    adapter->description = dup_or_empty(info ? info->description : NULL);
    adapter->is_fallback = info ? info->is_fallback : 0;
    adapter->canvas_width = hello->canvas_width;
    adapter->canvas_height = hello->canvas_height;

    limits_from_message(&adapter->limits, hello->limits);
    adapter->feature_count = hello->n_features;
    adapter->features = calloc(hello->n_features ? hello->n_features : 1,
                               sizeof *adapter->features);
    for (size_t i = 0; adapter->features && i < hello->n_features; ++i)
        adapter->features[i] = (WGPUFeatureName)hello->features[i];
    adapter->wgsl_feature_count = hello->n_wgsl_features;
    adapter->wgsl_features = calloc(hello->n_wgsl_features ? hello->n_wgsl_features : 1,
                                    sizeof *adapter->wgsl_features);
    for (size_t i = 0; adapter->wgsl_features && i < hello->n_wgsl_features; ++i)
        adapter->wgsl_features[i] = (WGPUWGSLLanguageFeatureName)hello->wgsl_features[i];

    adapter->ready = 1;
    fprintf(stderr,
            "remote_webgpu: client adapter: vendor=\"%s\" architecture=\"%s\""
            " device=\"%s\" description=\"%s\"%s\n",
            adapter->vendor, adapter->architecture, adapter->device,
            adapter->description, adapter->is_fallback ? " (fallback)" : "");
}

static void handle_map_buffer_data(RemoteAdapter *adapter,
                                   const RemoteWebgpu__MapBufferData *data)
{
    RwRequest *request = request_take(adapter, data->request_id, RW_REQUEST_MAP);
    if (!request) {
        fprintf(stderr, "remote_webgpu: unsolicited MapBufferData (request %llu)\n",
                (unsigned long long)data->request_id);
        return;
    }

    RemoteHandle *buffer = request->handle;
    WGPUBufferMapCallbackInfo callback = request->cb.map;

    WGPUMapAsyncStatus status = WGPUMapAsyncStatus_Error;
    WGPUStringView message = { NULL, 0 };
    if (data->failed) {
        message = sv(data->message ? data->message : "map failed");
        buffer->map_state = WGPUBufferMapState_Unmapped;
    } else {
        free(buffer->mapped);
        buffer->mapped = malloc(data->data.len ? data->data.len : 1);
        if (buffer->mapped) {
            memcpy(buffer->mapped, data->data.data, data->data.len);
            buffer->mapped_len = data->data.len;
            buffer->mapped_offset = request->map_offset;
            buffer->mapped_write = (request->map_mode & WGPUMapMode_Write) != 0;
            buffer->map_state = WGPUBufferMapState_Mapped;
            status = WGPUMapAsyncStatus_Success;
        } else {
            message = sv("out of memory");
            buffer->map_state = WGPUBufferMapState_Unmapped;
        }
    }

    request_finish(adapter, request);
    if (callback.callback)
        callback.callback(status, message, callback.userdata1, callback.userdata2);
    wgpuBufferRelease((WGPUBuffer)buffer);
}

static void handle_error_scope_result(RemoteAdapter *adapter,
                                      const RemoteWebgpu__ErrorScopeResult *result)
{
    RwRequest *request = request_take(adapter, result->request_id,
                                      RW_REQUEST_POP_ERROR);
    if (!request) {
        fprintf(stderr, "remote_webgpu: unsolicited ErrorScopeResult\n");
        return;
    }
    WGPUPopErrorScopeCallbackInfo callback = request->cb.pop_error;
    request_finish(adapter, request);
    if (callback.callback)
        callback.callback(WGPUPopErrorScopeStatus_Success,
                          (WGPUErrorType)result->error_type,
                          sv(result->message ? result->message : ""),
                          callback.userdata1, callback.userdata2);
}

static void handle_work_done(RemoteAdapter *adapter,
                             const RemoteWebgpu__WorkDone *done)
{
    RwRequest *request = request_take(adapter, done->request_id,
                                      RW_REQUEST_WORK_DONE);
    if (!request) {
        fprintf(stderr, "remote_webgpu: unsolicited WorkDone\n");
        return;
    }
    WGPUQueueWorkDoneCallbackInfo callback = request->cb.work_done;
    request_finish(adapter, request);
    if (callback.callback)
        callback.callback(WGPUQueueWorkDoneStatus_Success, sv(NULL),
                          callback.userdata1, callback.userdata2);
}

static void handle_compilation_info(RemoteAdapter *adapter,
                                    const RemoteWebgpu__CompilationInfoResult *result)
{
    RwRequest *request = request_take(adapter, result->request_id,
                                      RW_REQUEST_COMPILATION);
    if (!request) {
        fprintf(stderr, "remote_webgpu: unsolicited CompilationInfoResult\n");
        return;
    }
    WGPUCompilationInfoCallbackInfo callback = request->cb.compilation;
    request_finish(adapter, request);
    if (!callback.callback)
        return;

    size_t n = result->n_messages;
    WGPUCompilationMessage *messages = calloc(n ? n : 1, sizeof *messages);
    for (size_t i = 0; messages && i < n; ++i) {
        const RemoteWebgpu__CompilationMessage *in = result->messages[i];
        messages[i].message = sv(in->text ? in->text : "");
        messages[i].type = (WGPUCompilationMessageType)in->type;
        messages[i].lineNum = in->line_num;
        messages[i].linePos = in->line_pos;
        messages[i].offset = in->offset;
        messages[i].length = in->length;
    }
    WGPUCompilationInfo info = { NULL, messages ? n : 0, messages };
    callback.callback(WGPUCompilationInfoRequestStatus_Success, &info,
                      callback.userdata1, callback.userdata2);
    free(messages);
}

static void handle_texture_loaded(RemoteAdapter *adapter,
                                  const RemoteWebgpu__TextureLoaded *loaded)
{
    RwRequest *request = request_take(adapter, loaded->request_id,
                                      RW_REQUEST_TEXTURE_LOAD);
    if (!request) {
        fprintf(stderr, "remote_webgpu: unsolicited TextureLoaded\n");
        return;
    }
    RemoteHandle *texture = request->handle;
    WGPURemoteTextureLoadCallbackInfo callback = request->cb.texture_load;
    request_finish(adapter, request);

    if (loaded->failed) {
        const char *why = loaded->message ? loaded->message : "image load failed";
        fprintf(stderr, "remote_webgpu: texture load failed: %s\n", why);
        wgpuTextureRelease((WGPUTexture)texture);
        if (callback.callback)
            callback.callback(WGPUStatus_Error, NULL, sv(why),
                              callback.userdata1, callback.userdata2);
        return;
    }

    texture->width = loaded->width;
    texture->height = loaded->height;
    /* The callback owns the reference held since the request was made. */
    if (callback.callback)
        callback.callback(WGPUStatus_Success, (WGPUTexture)texture, sv(NULL),
                          callback.userdata1, callback.userdata2);
    else
        wgpuTextureRelease((WGPUTexture)texture);
}

static void handle_device_lost(RemoteAdapter *adapter,
                               const RemoteWebgpu__DeviceLost *lost)
{
    RemoteDevice *device = adapter->lost_device;
    if (!device || device->lost)
        return;
    device->lost = 1;
    if (device->lost_future_id)
        rw_future_complete(adapter->instance, device->lost_future_id);
    if (device->lost_callback.callback) {
        WGPUDevice handle = (WGPUDevice)device;
        device->lost_callback.callback(&handle,
                                       (WGPUDeviceLostReason)(lost->reason
                                                              ? lost->reason
                                                              : WGPUDeviceLostReason_Unknown),
                                       sv(lost->message ? lost->message : ""),
                                       device->lost_callback.userdata1,
                                       device->lost_callback.userdata2);
    }
}

static void handle_uncaptured_error(RemoteAdapter *adapter,
                                    const RemoteWebgpu__UncapturedError *error)
{
    RemoteDevice *device = adapter->lost_device;
    fprintf(stderr, "remote_webgpu: uncaptured error (type %u): %s\n",
            error->type, error->message ? error->message : "");
    if (device && device->uncaptured_callback.callback) {
        WGPUDevice handle = (WGPUDevice)device;
        device->uncaptured_callback.callback(&handle, (WGPUErrorType)error->type,
                                             sv(error->message ? error->message : ""),
                                             device->uncaptured_callback.userdata1,
                                             device->uncaptured_callback.userdata2);
    }
}

static void handle_event(RemoteAdapter *adapter, const RemoteWebgpu__Event *event)
{
    WGPURemoteEvent out;
    memset(&out, 0, sizeof out);

    switch (event->kind_case) {
    case REMOTE_WEBGPU__EVENT__KIND_CANVAS_RESIZE:
        /* Absorbed into the adapter state either way, so the size is
         * queryable even without a registered callback. */
        adapter->canvas_width = event->canvas_resize->width;
        adapter->canvas_height = event->canvas_resize->height;
        out.type = WGPURemoteEventType_CanvasResize;
        out.width = event->canvas_resize->width;
        out.height = event->canvas_resize->height;
        break;

    case REMOTE_WEBGPU__EVENT__KIND_USER:
        out.type = WGPURemoteEventType_User;
        out.name = event->user->name ? event->user->name : "";
        out.payload = event->user->payload.data;
        out.payload_size = event->user->payload.len;
        break;

    default:
        fprintf(stderr, "remote_webgpu: unknown event kind %d from client\n",
                (int)event->kind_case);
        return;
    }

    WGPURemoteEventCallbackInfo callback = adapter->event_callback;
    if (callback.callback)
        callback.callback(&out, callback.userdata1, callback.userdata2);
}

void wgpuRemoteAdapterSetEventCallback(WGPUAdapter adapter,
                                       WGPURemoteEventCallbackInfo callbackInfo)
{
    ((RemoteAdapter *)adapter)->event_callback = callbackInfo;
}

void wgpuRemoteAdapterReceiveData(WGPUAdapter adapter, void const *data, size_t size)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    RemoteWebgpu__Envelope *envelope =
        remote_webgpu__envelope__unpack(NULL, size, data);
    if (!envelope) {
        fprintf(stderr, "remote_webgpu: cannot parse Envelope from client\n");
        self->failed = 1;
        return;
    }

    switch (envelope->kind_case) {
    case REMOTE_WEBGPU__ENVELOPE__KIND_CLIENT_HELLO:
        handle_client_hello(self, envelope->client_hello);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_EVENT:
        handle_event(self, envelope->event);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_PRESENT_DONE:
        if (self->vsync_pending) {
            WGPURemoteVsyncCallbackInfo callback = self->vsync_callback;
            self->vsync_pending = 0;
            memset(&self->vsync_callback, 0, sizeof self->vsync_callback);
            if (callback.callback)
                callback.callback(callback.userdata1, callback.userdata2);
        } else {
            fprintf(stderr, "remote_webgpu: unsolicited PresentDone\n");
        }
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_MAP_BUFFER_DATA:
        handle_map_buffer_data(self, envelope->map_buffer_data);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_ERROR_SCOPE_RESULT:
        handle_error_scope_result(self, envelope->error_scope_result);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_WORK_DONE:
        handle_work_done(self, envelope->work_done);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_COMPILATION_INFO_RESULT:
        handle_compilation_info(self, envelope->compilation_info_result);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_TEXTURE_LOADED:
        handle_texture_loaded(self, envelope->texture_loaded);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_DEVICE_LOST:
        handle_device_lost(self, envelope->device_lost);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_UNCAPTURED_ERROR:
        handle_uncaptured_error(self, envelope->uncaptured_error);
        break;

    case REMOTE_WEBGPU__ENVELOPE__KIND_ERROR:
        fprintf(stderr, "remote_webgpu: client error: %s\n",
                envelope->error->message);
        self->failed = 1;
        break;

    default:
        fprintf(stderr, "remote_webgpu: unexpected message kind %d from client\n",
                (int)envelope->kind_case);
        break;
    }

    remote_webgpu__envelope__free_unpacked(envelope, NULL);
}

WGPUBool wgpuRemoteAdapterIsReady(WGPUAdapter adapter)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    return self->ready && !self->failed;
}

static void *alloc_object(size_t size)
{
    RemoteObject *obj = calloc(1, size);
    if (obj)
        obj->refcount = 1;
    return obj;
}

/* Returns 1 when the refcount hit zero and the object must be freed. */
static int release_object(RemoteObject *obj)
{
    return --obj->refcount == 0;
}

/* ------------------------------------------------------------------ */
/* instance                                                           */
/* ------------------------------------------------------------------ */

WGPUInstance wgpuCreateInstance(WGPU_NULLABLE WGPUInstanceDescriptor const *descriptor)
{
    (void)descriptor;
    return (WGPUInstance)alloc_object(sizeof(RemoteInstance));
}

void wgpuInstanceAddRef(WGPUInstance instance)
{
    ((RemoteInstance *)instance)->obj.refcount++;
}

void wgpuInstanceRelease(WGPUInstance instance)
{
    RemoteInstance *self = (RemoteInstance *)instance;
    if (release_object(&self->obj)) {
        free(self->completed_futures);
        free(self);
    }
}

void wgpuInstanceProcessEvents(WGPUInstance instance)
{
    (void)instance;
    /* Completions fire from inside wgpuRemoteAdapterReceiveData(); the
     * application drives progress by feeding received messages in. */
}

WGPUWaitStatus wgpuInstanceWaitAny(WGPUInstance instance, size_t futureCount,
                                   WGPUFutureWaitInfo *futures, uint64_t timeoutNS)
{
    /* The transport is app-driven: this call can only observe futures that
     * already completed inside wgpuRemoteAdapterReceiveData(); it cannot
     * block for new messages. */
    (void)timeoutNS;
    RemoteInstance *self = (RemoteInstance *)instance;
    int any = 0;
    for (size_t i = 0; i < futureCount; ++i) {
        if (future_is_complete(self, futures[i].future.id)) {
            futures[i].completed = 1;
            any = 1;
        }
    }
    return any ? WGPUWaitStatus_Success : WGPUWaitStatus_TimedOut;
}

void wgpuGetInstanceFeatures(WGPUSupportedInstanceFeatures *features)
{
    features->featureCount = 0;
    features->features = NULL;
}

WGPUStatus wgpuGetInstanceLimits(WGPUInstanceLimits *limits)
{
    limits->timedWaitAnyMaxCount = 0;
    return WGPUStatus_Success;
}

WGPUBool wgpuHasInstanceFeature(WGPUInstanceFeatureName feature)
{
    (void)feature;
    return 0;
}

void wgpuSupportedInstanceFeaturesFreeMembers(WGPUSupportedInstanceFeatures f)
{
    (void)f;
}

WGPUFuture wgpuInstanceRequestAdapter(WGPUInstance instance,
                                      WGPU_NULLABLE WGPURequestAdapterOptions const *options,
                                      WGPURequestAdapterCallbackInfo callbackInfo)
{
    /* Remote adapters exist per connection and are created with
     * wgpuRemoteInstanceCreateAdapter(); there is nothing to discover. */
    (void)options;
    WGPUFuture future = { 0 };
    if (callbackInfo.callback)
        callbackInfo.callback(WGPURequestAdapterStatus_Unavailable, NULL,
                              sv("use wgpuRemoteInstanceCreateAdapter()"),
                              callbackInfo.userdata1, callbackInfo.userdata2);
    (void)instance;
    return future;
}

/* ------------------------------------------------------------------ */
/* adapter                                                            */
/* ------------------------------------------------------------------ */

WGPUAdapter wgpuRemoteInstanceCreateAdapter(WGPUInstance instance,
                                            WGPURemoteSendCallback send,
                                            void *userdata)
{
    if (!instance || !send)
        return NULL;

    RemoteAdapter *adapter = alloc_object(sizeof *adapter);
    if (!adapter)
        return NULL;

    adapter->instance = (RemoteInstance *)instance;
    wgpuInstanceAddRef(instance);
    adapter->send = send;
    adapter->send_userdata = userdata;
    adapter->next_id = 1;
    adapter->next_future_id = 1;
    adapter->next_request_id = 1;

    /* Speak first; the adapter becomes ready when the client's hello is
     * fed back in via wgpuRemoteAdapterReceiveData(). */
    RemoteWebgpu__ServerHello server_hello = REMOTE_WEBGPU__SERVER_HELLO__INIT;
    server_hello.protocol_version = REMOTE_WEBGPU__PROTOCOL_VERSION__PROTOCOL_VERSION_CURRENT;
    RemoteWebgpu__Envelope envelope = REMOTE_WEBGPU__ENVELOPE__INIT;
    envelope.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_SERVER_HELLO;
    envelope.server_hello = &server_hello;
    rw_send_envelope(adapter, &envelope);

    return (WGPUAdapter)adapter;
}

void wgpuRemoteAdapterGetCanvasSize(WGPUAdapter adapter, uint32_t *width,
                                    uint32_t *height)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    *width = self->canvas_width;
    *height = self->canvas_height;
}

void wgpuAdapterAddRef(WGPUAdapter adapter)
{
    ((RemoteAdapter *)adapter)->obj.refcount++;
}

void wgpuAdapterRelease(WGPUAdapter adapter)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    if (release_object(&self->obj)) {
        while (self->requests) {
            RwRequest *request = self->requests;
            self->requests = request->next;
            free(request);
        }
        free(self->features);
        free(self->wgsl_features);
        free(self->vendor);
        free(self->architecture);
        free(self->device);
        free(self->description);
        wgpuInstanceRelease((WGPUInstance)self->instance);
        free(self);
    }
}

WGPUStatus wgpuAdapterGetInfo(WGPUAdapter adapter, WGPUAdapterInfo *info)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    memset(info, 0, sizeof *info);
    info->vendor = sv(self->vendor);
    info->architecture = sv(self->architecture);
    info->device = sv(self->device);
    info->description = sv(self->description);
    info->backendType = WGPUBackendType_WebGPU;
    info->adapterType = self->is_fallback ? WGPUAdapterType_CPU
                                          : WGPUAdapterType_Unknown;
    return WGPUStatus_Success;
}

void wgpuAdapterInfoFreeMembers(WGPUAdapterInfo adapterInfo)
{
    (void)adapterInfo; /* the strings are owned by the adapter */
}

WGPUStatus wgpuAdapterGetLimits(WGPUAdapter adapter, WGPULimits *limits)
{
    WGPUChainedStruct *chain = limits->nextInChain;
    *limits = ((RemoteAdapter *)adapter)->limits;
    limits->nextInChain = chain;
    return WGPUStatus_Success;
}

static void copy_features(WGPUSupportedFeatures *out, const WGPUFeatureName *in,
                          size_t count)
{
    WGPUFeatureName *features = calloc(count ? count : 1, sizeof *features);
    if (features)
        memcpy(features, in, count * sizeof *features);
    out->featureCount = features ? count : 0;
    out->features = features;
}

void wgpuAdapterGetFeatures(WGPUAdapter adapter, WGPUSupportedFeatures *features)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    copy_features(features, self->features, self->feature_count);
}

WGPUBool wgpuAdapterHasFeature(WGPUAdapter adapter, WGPUFeatureName feature)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    for (size_t i = 0; i < self->feature_count; ++i)
        if (self->features[i] == feature)
            return 1;
    return 0;
}

void wgpuSupportedFeaturesFreeMembers(WGPUSupportedFeatures features)
{
    free((void *)features.features);
}

void wgpuInstanceGetWGSLLanguageFeatures(WGPUInstance instance,
                                         WGPUSupportedWGSLLanguageFeatures *features)
{
    /* WGSL features are a property of the connected client; without an
     * adapter there is nothing to report.  The remote extension exposes
     * them through the instance's first adapter... but instances do not
     * track adapters, so report none here; use the adapter's list. */
    (void)instance;
    features->featureCount = 0;
    features->features = NULL;
}

WGPUBool wgpuInstanceHasWGSLLanguageFeature(WGPUInstance instance,
                                            WGPUWGSLLanguageFeatureName feature)
{
    (void)instance; (void)feature;
    return 0;
}

void wgpuSupportedWGSLLanguageFeaturesFreeMembers(WGPUSupportedWGSLLanguageFeatures f)
{
    free((void *)f.features);
}

/* ------------------------------------------------------------------ */
/* device / queue                                                     */
/* ------------------------------------------------------------------ */

WGPUFuture wgpuAdapterRequestDevice(WGPUAdapter adapter,
                                    WGPU_NULLABLE WGPUDeviceDescriptor const *descriptor,
                                    WGPURequestDeviceCallbackInfo callbackInfo)
{
    RemoteAdapter *self = (RemoteAdapter *)adapter;
    WGPUFuture future = { rw_next_future_id(self) };
    RemoteDevice *device = alloc_object(sizeof *device);
    if (device) {
        device->adapter = self;
        wgpuAdapterAddRef(adapter);
        if (descriptor) {
            device->lost_callback = descriptor->deviceLostCallbackInfo;
            device->uncaptured_callback = descriptor->uncapturedErrorCallbackInfo;
        }
        /* Device-lost / uncaptured-error reports route here. */
        self->lost_device = device;

        /* Forward the requirements; the client re-creates its device with
         * them.  A failure to satisfy them surfaces as a client Error. */
        RemoteWebgpu__RequestDevice msg = REMOTE_WEBGPU__REQUEST_DEVICE__INIT;
        RemoteWebgpu__Limits limits = REMOTE_WEBGPU__LIMITS__INIT;
        char *label = descriptor ? rw_dup_stringview(descriptor->label) : NULL;
        char *queue_label = descriptor
            ? rw_dup_stringview(descriptor->defaultQueue.label) : NULL;
        uint32_t *features = NULL;
        msg.label = label ? label : "";
        msg.default_queue_label = queue_label ? queue_label : "";
        if (descriptor && descriptor->requiredFeatureCount) {
            features = calloc(descriptor->requiredFeatureCount, sizeof *features);
            if (features) {
                for (size_t i = 0; i < descriptor->requiredFeatureCount; ++i)
                    features[i] = (uint32_t)descriptor->requiredFeatures[i];
                msg.n_required_features = descriptor->requiredFeatureCount;
                msg.required_features = features;
            }
        }
        if (descriptor && descriptor->requiredLimits) {
            const WGPULimits *in = descriptor->requiredLimits;
            limits.max_texture_dimension_1d = in->maxTextureDimension1D;
            limits.max_texture_dimension_2d = in->maxTextureDimension2D;
            limits.max_texture_dimension_3d = in->maxTextureDimension3D;
            limits.max_texture_array_layers = in->maxTextureArrayLayers;
            limits.max_bind_groups = in->maxBindGroups;
            limits.max_bind_groups_plus_vertex_buffers = in->maxBindGroupsPlusVertexBuffers;
            limits.max_bindings_per_bind_group = in->maxBindingsPerBindGroup;
            limits.max_dynamic_uniform_buffers_per_pipeline_layout =
                in->maxDynamicUniformBuffersPerPipelineLayout;
            limits.max_dynamic_storage_buffers_per_pipeline_layout =
                in->maxDynamicStorageBuffersPerPipelineLayout;
            limits.max_sampled_textures_per_shader_stage =
                in->maxSampledTexturesPerShaderStage;
            limits.max_samplers_per_shader_stage = in->maxSamplersPerShaderStage;
            limits.max_storage_buffers_per_shader_stage =
                in->maxStorageBuffersPerShaderStage;
            limits.max_storage_textures_per_shader_stage =
                in->maxStorageTexturesPerShaderStage;
            limits.max_uniform_buffers_per_shader_stage =
                in->maxUniformBuffersPerShaderStage;
            limits.max_uniform_buffer_binding_size = in->maxUniformBufferBindingSize;
            limits.max_storage_buffer_binding_size = in->maxStorageBufferBindingSize;
            limits.min_uniform_buffer_offset_alignment =
                in->minUniformBufferOffsetAlignment;
            limits.min_storage_buffer_offset_alignment =
                in->minStorageBufferOffsetAlignment;
            limits.max_vertex_buffers = in->maxVertexBuffers;
            limits.max_buffer_size = in->maxBufferSize;
            limits.max_vertex_attributes = in->maxVertexAttributes;
            limits.max_vertex_buffer_array_stride = in->maxVertexBufferArrayStride;
            limits.max_inter_stage_shader_variables = in->maxInterStageShaderVariables;
            limits.max_color_attachments = in->maxColorAttachments;
            limits.max_color_attachment_bytes_per_sample =
                in->maxColorAttachmentBytesPerSample;
            limits.max_compute_workgroup_storage_size =
                in->maxComputeWorkgroupStorageSize;
            limits.max_compute_invocations_per_workgroup =
                in->maxComputeInvocationsPerWorkgroup;
            limits.max_compute_workgroup_size_x = in->maxComputeWorkgroupSizeX;
            limits.max_compute_workgroup_size_y = in->maxComputeWorkgroupSizeY;
            limits.max_compute_workgroup_size_z = in->maxComputeWorkgroupSizeZ;
            limits.max_compute_workgroups_per_dimension =
                in->maxComputeWorkgroupsPerDimension;
            limits.max_immediate_size = in->maxImmediateSize;
            msg.required_limits = &limits;
        }
        RemoteWebgpu__Envelope envelope = REMOTE_WEBGPU__ENVELOPE__INIT;
        envelope.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_REQUEST_DEVICE;
        envelope.request_device = &msg;
        rw_send_envelope(self, &envelope);
        free(features);
        free(label);
        free(queue_label);
    }

    /* Resolved synchronously, like wgpu-native does. */
    rw_future_complete(self->instance, future.id);
    if (callbackInfo.callback) {
        if (device)
            callbackInfo.callback(WGPURequestDeviceStatus_Success, (WGPUDevice)device,
                                  sv(NULL), callbackInfo.userdata1, callbackInfo.userdata2);
        else
            callbackInfo.callback(WGPURequestDeviceStatus_Error, NULL,
                                  sv("out of memory"),
                                  callbackInfo.userdata1, callbackInfo.userdata2);
    }
    return future;
}

void wgpuDeviceAddRef(WGPUDevice device)
{
    ((RemoteDevice *)device)->obj.refcount++;
}

void wgpuDeviceRelease(WGPUDevice device)
{
    RemoteDevice *self = (RemoteDevice *)device;
    if (release_object(&self->obj)) {
        if (self->adapter->lost_device == self)
            self->adapter->lost_device = NULL;
        wgpuAdapterRelease((WGPUAdapter)self->adapter);
        free(self);
    }
}

WGPUStatus wgpuDeviceGetAdapterInfo(WGPUDevice device, WGPUAdapterInfo *adapterInfo)
{
    return wgpuAdapterGetInfo((WGPUAdapter)((RemoteDevice *)device)->adapter,
                              adapterInfo);
}

WGPUStatus wgpuDeviceGetLimits(WGPUDevice device, WGPULimits *limits)
{
    return wgpuAdapterGetLimits((WGPUAdapter)((RemoteDevice *)device)->adapter,
                                limits);
}

void wgpuDeviceGetFeatures(WGPUDevice device, WGPUSupportedFeatures *features)
{
    wgpuAdapterGetFeatures((WGPUAdapter)((RemoteDevice *)device)->adapter, features);
}

WGPUBool wgpuDeviceHasFeature(WGPUDevice device, WGPUFeatureName feature)
{
    return wgpuAdapterHasFeature((WGPUAdapter)((RemoteDevice *)device)->adapter,
                                 feature);
}

WGPUFuture wgpuDeviceGetLostFuture(WGPUDevice device)
{
    RemoteDevice *self = (RemoteDevice *)device;
    if (!self->lost_future_id) {
        self->lost_future_id = rw_next_future_id(self->adapter);
        if (self->lost)
            rw_future_complete(self->adapter->instance, self->lost_future_id);
    }
    WGPUFuture future = { self->lost_future_id };
    return future;
}

WGPUQueue wgpuDeviceGetQueue(WGPUDevice device)
{
    RemoteQueue *queue = alloc_object(sizeof *queue);
    if (!queue)
        return NULL;
    queue->device = (RemoteDevice *)device;
    wgpuDeviceAddRef(device);
    return (WGPUQueue)queue;
}

void wgpuQueueAddRef(WGPUQueue queue)
{
    ((RemoteQueue *)queue)->obj.refcount++;
}

void wgpuQueueRelease(WGPUQueue queue)
{
    RemoteQueue *self = (RemoteQueue *)queue;
    if (release_object(&self->obj)) {
        wgpuDeviceRelease((WGPUDevice)self->device);
        free(self);
    }
}

/* wgpu-native extension (see webgpu/wgpu.h shim). */
WGPUBool wgpuDevicePoll(WGPUDevice device, WGPUBool wait,
                        WGPUSubmissionIndex const *wrappedSubmissionIndex)
{
    (void)device; (void)wait; (void)wrappedSubmissionIndex;
    /* Progress happens when the app feeds received messages in via
     * wgpuRemoteAdapterReceiveData(); nothing to do here. */
    return 1;
}

/* ------------------------------------------------------------------ */
/* surface                                                            */
/* ------------------------------------------------------------------ */

WGPUSurface wgpuInstanceCreateSurface(WGPUInstance instance,
                                      WGPUSurfaceDescriptor const *descriptor)
{
    (void)descriptor; /* the client's canvas is the only surface */

    RemoteSurface *surface = alloc_object(sizeof *surface);
    if (!surface)
        return NULL;
    surface->instance = (RemoteInstance *)instance;
    wgpuInstanceAddRef(instance);
    return (WGPUSurface)surface;
}

void wgpuSurfaceAddRef(WGPUSurface surface)
{
    ((RemoteSurface *)surface)->obj.refcount++;
}

void wgpuSurfaceRelease(WGPUSurface surface)
{
    RemoteSurface *self = (RemoteSurface *)surface;
    if (release_object(&self->obj)) {
        if (self->device)
            wgpuDeviceRelease((WGPUDevice)self->device);
        wgpuInstanceRelease((WGPUInstance)self->instance);
        free(self);
    }
}

static const WGPUTextureFormat kSurfaceFormats[] = {
    WGPUTextureFormat_BGRA8Unorm,
    WGPUTextureFormat_RGBA8Unorm,
    WGPUTextureFormat_RGBA16Float,
};
static const WGPUPresentMode kPresentModes[] = { WGPUPresentMode_Fifo };
static const WGPUCompositeAlphaMode kAlphaModes[] = {
    WGPUCompositeAlphaMode_Opaque,
    WGPUCompositeAlphaMode_Premultiplied,
};

WGPUStatus wgpuSurfaceGetCapabilities(WGPUSurface surface, WGPUAdapter adapter,
                                      WGPUSurfaceCapabilities *capabilities)
{
    (void)surface; (void)adapter;

    /* A browser canvas context accepts these; CopySrc/CopyDst/binding
     * usages are allowed so the application can read frames back
     * (screenshots) or sample them. */
    memset(capabilities, 0, sizeof *capabilities);
    capabilities->usages = WGPUTextureUsage_RenderAttachment
        | WGPUTextureUsage_CopySrc | WGPUTextureUsage_CopyDst
        | WGPUTextureUsage_TextureBinding | WGPUTextureUsage_StorageBinding;
    capabilities->formatCount = sizeof kSurfaceFormats / sizeof kSurfaceFormats[0];
    capabilities->formats = kSurfaceFormats;
    capabilities->presentModeCount = 1;
    capabilities->presentModes = kPresentModes;
    capabilities->alphaModeCount = sizeof kAlphaModes / sizeof kAlphaModes[0];
    capabilities->alphaModes = kAlphaModes;
    return WGPUStatus_Success;
}

void wgpuSurfaceCapabilitiesFreeMembers(WGPUSurfaceCapabilities surfaceCapabilities)
{
    (void)surfaceCapabilities; /* the arrays above are static */
}

void wgpuSurfaceConfigure(WGPUSurface surface, WGPUSurfaceConfiguration const *config)
{
    RemoteSurface *self = (RemoteSurface *)surface;
    self->config = *config;

    if (self->device != (RemoteDevice *)config->device) {
        if (self->device)
            wgpuDeviceRelease((WGPUDevice)self->device);
        self->device = (RemoteDevice *)config->device;
        wgpuDeviceAddRef(config->device);
    }

    RemoteWebgpu__ConfigureSurface configure = REMOTE_WEBGPU__CONFIGURE_SURFACE__INIT;
    configure.width = config->width;
    configure.height = config->height;
    configure.format = (uint32_t)config->format;
    configure.usage = (uint32_t)config->usage;
    configure.alpha_mode = (uint32_t)config->alphaMode;
    uint32_t *view_formats = NULL;
    if (config->viewFormatCount) {
        view_formats = calloc(config->viewFormatCount, sizeof *view_formats);
        if (view_formats) {
            for (size_t i = 0; i < config->viewFormatCount; ++i)
                view_formats[i] = (uint32_t)config->viewFormats[i];
            configure.n_view_formats = config->viewFormatCount;
            configure.view_formats = view_formats;
        }
    }
    RemoteWebgpu__Envelope envelope = REMOTE_WEBGPU__ENVELOPE__INIT;
    envelope.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_CONFIGURE_SURFACE;
    envelope.configure_surface = &configure;
    rw_send_envelope(self->device->adapter, &envelope);
    free(view_formats);
}

void wgpuSurfaceUnconfigure(WGPUSurface surface)
{
    RemoteSurface *self = (RemoteSurface *)surface;
    if (!self->device)
        return;
    RemoteWebgpu__SurfaceUnconfigure msg = REMOTE_WEBGPU__SURFACE_UNCONFIGURE__INIT;
    RemoteWebgpu__Envelope envelope = REMOTE_WEBGPU__ENVELOPE__INIT;
    envelope.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_SURFACE_UNCONFIGURE;
    envelope.surface_unconfigure = &msg;
    rw_send_envelope(self->device->adapter, &envelope);
}
