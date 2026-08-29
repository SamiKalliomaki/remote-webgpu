/*
 * Hand-written part of the remote WebGPU implementation.
 *
 * Only the object-lifetime plumbing and the socket-backed adapter/device
 * bring-up exist so far.  Everything else lives in stubs.c (generated) and
 * aborts with a clear message when called, so missing pieces surface
 * immediately as they are needed.
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
    RemoteHandle *handle = adapter->map_handle;
    WGPUBufferMapCallbackInfo callback = adapter->map_callback;
    adapter->map_handle = NULL;
    memset(&adapter->map_callback, 0, sizeof adapter->map_callback);

    if (!handle) {
        fprintf(stderr, "remote_webgpu: unsolicited MapBufferData\n");
        return;
    }

    WGPUMapAsyncStatus status = WGPUMapAsyncStatus_Error;
    WGPUStringView message = { NULL, 0 };
    free(handle->mapped);
    handle->mapped = malloc(data->data.len ? data->data.len : 1);
    if (handle->mapped) {
        memcpy(handle->mapped, data->data.data, data->data.len);
        handle->mapped_len = data->data.len;
        status = WGPUMapAsyncStatus_Success;
    } else {
        message.data = "out of memory";
        message.length = strlen(message.data);
    }

    if (callback.callback)
        callback.callback(status, message, callback.userdata1, callback.userdata2);
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
    if (release_object(&self->obj))
        free(self);
}

void wgpuInstanceProcessEvents(WGPUInstance instance)
{
    (void)instance;
    /* TODO: pump completed replies from the socket and fire callbacks. */
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

/* ------------------------------------------------------------------ */
/* device / queue                                                     */
/* ------------------------------------------------------------------ */

WGPUFuture wgpuAdapterRequestDevice(WGPUAdapter adapter,
                                    WGPU_NULLABLE WGPUDeviceDescriptor const *descriptor,
                                    WGPURequestDeviceCallbackInfo callbackInfo)
{
    (void)descriptor; /* TODO: forward the descriptor to the remote GPU. */

    WGPUFuture future = { 0 };
    RemoteDevice *device = alloc_object(sizeof *device);
    if (device) {
        device->adapter = (RemoteAdapter *)adapter;
        wgpuAdapterAddRef(adapter);
    }

    /* Resolved synchronously, like wgpu-native does. */
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
        wgpuAdapterRelease((WGPUAdapter)self->adapter);
        free(self);
    }
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
    /* TODO: wait for the remote queue; nothing is ever in flight yet. */
    return 1;
}

/* ------------------------------------------------------------------ */
/* surface                                                            */
/* ------------------------------------------------------------------ */

WGPUSurface wgpuInstanceCreateSurface(WGPUInstance instance,
                                      WGPUSurfaceDescriptor const *descriptor)
{
    (void)descriptor; /* TODO: presentation to a local window is undecided. */

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

static const WGPUTextureFormat kSurfaceFormats[] = { WGPUTextureFormat_BGRA8Unorm };
static const WGPUPresentMode kPresentModes[] = { WGPUPresentMode_Fifo };
static const WGPUCompositeAlphaMode kAlphaModes[] = { WGPUCompositeAlphaMode_Opaque };

WGPUStatus wgpuSurfaceGetCapabilities(WGPUSurface surface, WGPUAdapter adapter,
                                      WGPUSurfaceCapabilities *capabilities)
{
    (void)surface; (void)adapter;

    /* TODO: query the remote GPU; these are hard-coded placeholders.
     * CopySrc is advertised so the application can take screenshots via
     * the readback path. */
    memset(capabilities, 0, sizeof *capabilities);
    capabilities->usages = WGPUTextureUsage_RenderAttachment | WGPUTextureUsage_CopySrc;
    capabilities->formatCount = 1;
    capabilities->formats = kSurfaceFormats;
    capabilities->presentModeCount = 1;
    capabilities->presentModes = kPresentModes;
    capabilities->alphaModeCount = 1;
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
    RemoteWebgpu__Envelope envelope = REMOTE_WEBGPU__ENVELOPE__INIT;
    envelope.kind_case = REMOTE_WEBGPU__ENVELOPE__KIND_CONFIGURE_SURFACE;
    envelope.configure_surface = &configure;
    rw_send_envelope(self->device->adapter, &envelope);
}
