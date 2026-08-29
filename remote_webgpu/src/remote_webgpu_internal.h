#ifndef REMOTE_WEBGPU_INTERNAL_H
#define REMOTE_WEBGPU_INTERNAL_H

#include <webgpu/remote.h>
#include <webgpu/webgpu.h>

#include "remote_webgpu.pb-c.h"

/*
 * Object model of the remote WebGPU implementation.
 *
 * Every WGPU* handle points at a struct that starts with RemoteObject.  For
 * now the objects carry almost no state: the interesting part is the socket
 * on the instance, over which every method will eventually be forwarded to
 * the remote GPU process.
 */

typedef struct {
    unsigned refcount;
} RemoteObject;

typedef struct RemoteInstance {
    RemoteObject obj;
} RemoteInstance;

struct RemoteHandle;

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
    /* Latest canvas size reported by the client (ClientHello/CanvasResize),
     * in device pixels.  0 until the client reports one. */
    uint32_t canvas_width;
    uint32_t canvas_height;
    /* Adapter info reported by the client in its ClientHello (strdup()ed). */
    char *vendor;
    char *architecture;
    char *device;
    char *description;
    int is_fallback;
    /* Pending vsync wait (one at a time); fires on PresentDone. */
    WGPURemoteVsyncCallbackInfo vsync_callback;
    int vsync_pending;
    /* Pending buffer map (one at a time); completes on MapBufferData. */
    struct RemoteHandle *map_handle;
    WGPUBufferMapCallbackInfo map_callback;
} RemoteAdapter;

typedef struct RemoteDevice {
    RemoteObject obj;
    RemoteAdapter *adapter;
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
 * RemoteHandle: a client-side object identified by `id`.  The server keeps
 * no state beyond what the webgpu.h API forces it to answer locally.
 */
typedef struct RemoteHandle {
    RemoteObject obj;
    RemoteDevice *device; /* ref held; routes to the adapter's socket */
    uint32_t id;
    /* Buffers only: CPU copy of the contents while mapped for reading. */
    uint8_t *mapped;
    size_t mapped_len;
    /* Textures only: size, to answer wgpuTextureGetWidth/Height locally. */
    uint32_t width, height;
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

/* malloc()ed C string from a string view; "" for NULL.  Caller frees. */
char *rw_dup_stringview(WGPUStringView s);

/* Print "unimplemented: <name>" and abort.  Used by every generated stub. */
_Noreturn void remote_wgpu_unimplemented(const char *name);

#endif /* REMOTE_WEBGPU_INTERNAL_H */
