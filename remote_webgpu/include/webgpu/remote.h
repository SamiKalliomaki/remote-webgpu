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
 * ClientHello and any subsequent CanvasResize notifications.  0x0 until the
 * client reports a size.
 */
WGPU_EXPORT void wgpuRemoteAdapterGetCanvasSize(WGPUAdapter adapter,
                                                uint32_t *width, uint32_t *height);

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
