#ifndef WEBGPU_REMOTE_H
#define WEBGPU_REMOTE_H

/*
 * Extensions to the standard webgpu.h API for the remote implementation.
 *
 * The library is transport-agnostic: the application supplies a callback
 * through which the library sends protocol messages, and pushes received
 * messages back in with wgpuRemoteAdapterReceiveData().  How the bytes
 * travel (websocket, pipe, ...) is entirely the application's business.
 *
 * Because the library never blocks on the transport, asynchronous results
 * (adapter readiness, buffer maps, vsync) complete from inside
 * wgpuRemoteAdapterReceiveData(); the application drives progress by
 * feeding it data.
 */

#include <stddef.h>

#include <webgpu/webgpu.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Sends one complete protocol message (a serialized Envelope) to the remote
 * GPU.  Must deliver messages reliably and in order.
 */
typedef void (*WGPURemoteSendCallback)(void const *data, size_t size,
                                       void *userdata);

/*
 * Create a WebGPU adapter that talks to a remote GPU through `send`.
 *
 * The server hello is sent immediately, but the adapter is not usable until
 * the client's reply has been fed in via wgpuRemoteAdapterReceiveData() and
 * wgpuRemoteAdapterIsReady() returns true.  Returns NULL on failure.
 */
WGPU_EXPORT WGPUAdapter wgpuRemoteInstanceCreateAdapter(WGPUInstance instance,
                                                        WGPURemoteSendCallback send,
                                                        void *userdata);

/*
 * Feed one complete protocol message (a serialized Envelope) received from
 * the remote GPU into the library.  Completion callbacks (vsync, buffer
 * maps) fire from inside this call.
 */
WGPU_EXPORT void wgpuRemoteAdapterReceiveData(WGPUAdapter adapter,
                                              void const *data, size_t size);

/* True once the client's hello has been received and validated. */
WGPU_EXPORT WGPUBool wgpuRemoteAdapterIsReady(WGPUAdapter adapter);

/*
 * Latest size of the client's canvas in device pixels, as reported in its
 * ClientHello and any subsequent canvas-resize events.  0x0 until the
 * client reports a size.
 */
WGPU_EXPORT void wgpuRemoteAdapterGetCanvasSize(WGPUAdapter adapter,
                                                uint32_t *width, uint32_t *height);

/* ------------------------------------------------------------------ */
/* events                                                             */
/* ------------------------------------------------------------------ */

/*
 * Events are notifications from the client: built-in ones (the canvas
 * changed size) and user-defined ones sent by the client application (a
 * name plus an opaque payload whose encoding is a contract between the two
 * applications).  Neither expects a reply.
 */
typedef enum WGPURemoteEventType {
    /* The client's canvas changed physical size.  The canvas keeps its old
     * pixel size until the application reconfigures the surface, which is
     * what actually resizes the canvas backing store. */
    WGPURemoteEventType_CanvasResize = 1,
    /* A user-defined event from the client application. */
    WGPURemoteEventType_User = 2,
} WGPURemoteEventType;

typedef struct WGPURemoteEvent {
    WGPURemoteEventType type;
    /* CanvasResize: the new canvas size in device pixels. */
    uint32_t width;
    uint32_t height;
    /* User: NUL-terminated name and opaque payload.  Both point into the
     * received message and are only valid for the duration of the
     * callback; copy them to keep them. */
    const char *name;
    const void *payload;
    size_t payload_size;
} WGPURemoteEvent;

typedef void (*WGPURemoteEventCallback)(const WGPURemoteEvent *event,
                                        void *userdata1, void *userdata2);

typedef struct WGPURemoteEventCallbackInfo {
    WGPURemoteEventCallback callback;
    void *userdata1;
    void *userdata2;
} WGPURemoteEventCallbackInfo;

/*
 * Register the callback that receives client events.  One callback per
 * adapter; registering again replaces it, and a zeroed info unregisters.
 * The callback fires from inside wgpuRemoteAdapterReceiveData().  Events
 * arriving with no callback registered are still absorbed into the state
 * wgpuRemoteAdapterGetCanvasSize() reports (resizes) or dropped (user
 * events).
 */
WGPU_EXPORT void wgpuRemoteAdapterSetEventCallback(WGPUAdapter adapter,
                                                   WGPURemoteEventCallbackInfo callbackInfo);

typedef void (*WGPURemoteVsyncCallback)(void *userdata1, void *userdata2);

typedef struct WGPURemoteVsyncCallbackInfo {
    WGPURemoteVsyncCallback callback;
    void *userdata1;
    void *userdata2;
} WGPURemoteVsyncCallbackInfo;

/*
 * Complete a future when the frame most recently handed to
 * wgpuSurfacePresent() is on the client's screen (the client acknowledges
 * its next vsync).  wgpuSurfacePresent() itself only queues the present and
 * returns immediately; wait on this future to pace a render loop to the
 * client's refresh rate.  The callback fires from inside
 * wgpuRemoteAdapterReceiveData().
 */
WGPU_EXPORT WGPUFuture wgpuRemoteSurfaceOnNextVsync(WGPUSurface surface,
                                                    WGPURemoteVsyncCallbackInfo callbackInfo);

#ifdef __cplusplus
}
#endif

#endif /* WEBGPU_REMOTE_H */
