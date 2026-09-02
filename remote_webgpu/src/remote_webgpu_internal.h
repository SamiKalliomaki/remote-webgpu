#ifndef REMOTE_WEBGPU_INTERNAL_H
#define REMOTE_WEBGPU_INTERNAL_H

#include <webgpu/remote.h>
#include <webgpu/webgpu.h>

#include "remote_webgpu.pb-c.h"

/*
 * Object model of the remote WebGPU implementation.
 *
 * Every WGPU* handle points at a struct that starts with RemoteObject.
 * Objects carry only the state the webgpu.h API forces the server to
 * answer locally (sizes, formats, map state, ...); everything else lives
 * on the client, addressed by server-assigned uint32 ids.
 */

typedef struct {
    unsigned refcount;
} RemoteObject;

typedef struct RemoteInstance {
    RemoteObject obj;
    /* Future ids that have completed; wgpuInstanceWaitAny() consults this
     * (the transport is app-driven, so it can never block). */
    uint64_t *completed_futures;
    size_t completed_count, completed_capacity;
} RemoteInstance;

struct RemoteHandle;
struct RemoteDevice;

/* A command sent to the client that expects an asynchronous reply. */
typedef enum RwRequestType {
    RW_REQUEST_MAP,         /* MapBuffer          -> MapBufferData */
    RW_REQUEST_POP_ERROR,   /* PopErrorScope      -> ErrorScopeResult */
    RW_REQUEST_WORK_DONE,   /* OnSubmittedWorkDone-> WorkDone */
    RW_REQUEST_COMPILATION, /* GetCompilationInfo -> CompilationInfoResult */
    RW_REQUEST_TEXTURE_LOAD,/* LoadTextureFromUrl -> TextureLoaded */
} RwRequestType;

typedef struct RwRequest {
    struct RwRequest *next;
    uint64_t request_id;
    uint64_t future_id;
    RwRequestType type;
    /* RW_REQUEST_MAP: the buffer being mapped; RW_REQUEST_TEXTURE_LOAD:
     * the texture being filled (ref held either way). */
    struct RemoteHandle *handle;
    WGPUMapMode map_mode;
    uint64_t map_offset;
    /* RW_REQUEST_MAP: how many bytes the client owes us for this mapping
     * (WGPU_WHOLE_MAP_SIZE already resolved against the buffer size).  A
     * reply that carries fewer than this is rejected: the application would
     * otherwise be handed a range shorter than the one it asked for. */
    uint64_t map_size;
    union {
        WGPUBufferMapCallbackInfo map;
        WGPUPopErrorScopeCallbackInfo pop_error;
        WGPUQueueWorkDoneCallbackInfo work_done;
        WGPUCompilationInfoCallbackInfo compilation;
        WGPURemoteTextureLoadCallbackInfo texture_load;
    } cb;
} RwRequest;

/* One wgpuRemoteSurfaceOnNextVsync() wait: fires (and completes its future)
 * once the client has acknowledged `present_seq` presents. */
typedef struct RwVsyncWait {
    struct RwVsyncWait *next;
    uint64_t present_seq;
    uint64_t future_id;
    WGPURemoteVsyncCallbackInfo callback;
} RwVsyncWait;

typedef struct RemoteAdapter {
    RemoteObject obj;
    RemoteInstance *instance;
    /* How protocol messages reach the remote GPU; owned by the app. */
    WGPURemoteSendCallback send;
    void *send_userdata;
    /* Handshake state: ready once a valid ClientHello has been received,
     * failed on a protocol error (the adapter is then unusable). */
    int ready;
    int failed;
    /* Next object id to hand out to the client (0 is reserved for "none"). */
    uint32_t next_id;
    /* Next future id to hand out (0 is reserved for "none"). */
    uint64_t next_future_id;
    /* Next request id for commands that expect a reply. */
    uint64_t next_request_id;
    /* Outstanding requests, completed by replies fed into
     * wgpuRemoteAdapterReceiveData(). */
    RwRequest *requests;
    /* Latest canvas size reported by the client (ClientHello or a
     * canvas-resize event), in device pixels.  0 until reported. */
    uint32_t canvas_width;
    uint32_t canvas_height;
    /* Adapter info reported by the client in its ClientHello (strdup()ed). */
    char *vendor;
    char *architecture;
    char *device;
    char *description;
    int is_fallback;
    /* Capabilities reported by the client in its ClientHello. */
    WGPULimits limits;
    WGPUFeatureName *features;
    size_t feature_count;
    WGPUWGSLLanguageFeatureName *wgsl_features;
    size_t wgsl_feature_count;
    /* Receives client events (resize, user-defined); zeroed until the app
     * registers one via wgpuRemoteAdapterSetEventCallback(). */
    WGPURemoteEventCallbackInfo event_callback;
    /* Pending vsync waits, oldest first.  Each is tied to the last present
     * sent before it was registered; every PresentDone from the client bumps
     * presents_done and fires the waits whose present has been acknowledged. */
    RwVsyncWait *vsync_waits;
    uint64_t presents_sent;
    uint64_t presents_done;
    /* The device the client reported lost (owning the callbacks below);
     * see remote_webgpu.c. */
    struct RemoteDevice *lost_device;
} RemoteAdapter;

typedef struct RemoteDevice {
    RemoteObject obj;
    RemoteAdapter *adapter;
    /* From the WGPUDeviceDescriptor handed to wgpuAdapterRequestDevice. */
    WGPUDeviceLostCallbackInfo lost_callback;
    WGPUUncapturedErrorCallbackInfo uncaptured_callback;
    uint64_t lost_future_id; /* 0 until wgpuDeviceGetLostFuture() */
    int lost;                /* DeviceLost received (or destroy sent) */
} RemoteDevice;

typedef struct RemoteQueue {
    RemoteObject obj;
    RemoteDevice *device;
} RemoteQueue;

typedef struct RemoteSurface {
    RemoteObject obj;
    RemoteInstance *instance;
    /* Device from the last wgpuSurfaceConfigure() (ref held); NULL before. */
    RemoteDevice *device;
    /* Last wgpuSurfaceConfigure(); zeroed until the first call. */
    WGPUSurfaceConfiguration config;
} RemoteSurface;

/*
 * Every other WebGPU object (buffer, texture, pipeline, encoder, ...) is a
 * RemoteHandle: a client-side object identified by `id`, plus whatever
 * state webgpu.h getters must answer without a round trip.
 */
typedef struct RemoteHandle {
    RemoteObject obj;
    RemoteDevice *device; /* ref held; routes to the adapter's socket */
    uint32_t id;
    /* Buffers: descriptor state and the CPU shadow of the mapped range. */
    uint64_t size;
    WGPUBufferUsage usage;
    WGPUBufferMapState map_state;
    uint8_t *mapped;
    size_t mapped_len;
    uint64_t mapped_offset;
    int mapped_write; /* flush to the client on unmap */
    /* Textures: descriptor state for the local getters. */
    uint32_t width, height, depth_or_array_layers;
    uint32_t mip_level_count, sample_count;
    WGPUTextureDimension dimension;
    WGPUTextureFormat format;
    WGPUTextureUsage texture_usage;
    /* Query sets. */
    WGPUQueryType query_type;
    uint32_t query_count;
} RemoteHandle;

/* --- shared plumbing (remote_webgpu.c) ----------------------------- */

/* Serialize one envelope and hand it to the app's send callback. */
void rw_send_envelope(RemoteAdapter *adapter, const RemoteWebgpu__Envelope *envelope);

/* Allocate a RemoteHandle with a fresh id, holding a ref on `device`. */
RemoteHandle *rw_handle_create(RemoteDevice *device);
void rw_handle_addref(void *handle);
/* Drop a ref; at zero notifies the client (DestroyObject) and frees. */
void rw_handle_release(void *handle);

/* The adapter behind a device. */
RemoteAdapter *rw_device_adapter(RemoteDevice *device);

/* Queue an outstanding request; returns it (or NULL on OOM).  The request
 * is freed by the reply handler in remote_webgpu.c. */
RwRequest *rw_request_create(RemoteAdapter *adapter, RwRequestType type);

/* Mark a future id completed (for wgpuInstanceWaitAny). */
void rw_future_complete(RemoteInstance *instance, uint64_t future_id);

/* A fresh future id, pre-registered on the adapter's instance. */
uint64_t rw_next_future_id(RemoteAdapter *adapter);

/* malloc()ed C string from a string view; "" for NULL.  Caller frees. */
char *rw_dup_stringview(WGPUStringView s);

/* Warn once per call site about an argument the protocol cannot express. */
void rw_warn_unsupported(const char *what);

/* Print "unimplemented: <name>" and abort.  Used by every generated stub. */
_Noreturn void remote_wgpu_unimplemented(const char *name);

#endif /* REMOTE_WEBGPU_INTERNAL_H */
