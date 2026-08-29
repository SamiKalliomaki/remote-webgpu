#ifndef WEBGPU_REMOTE_H
#define WEBGPU_REMOTE_H

/*
 * Extensions to the standard webgpu.h API for the remote (socket-backed)
 * implementation.  This is how an application hands the library the
 * connection to the remote GPU: instead of wgpuInstanceRequestAdapter, it
 * creates an adapter directly from an already-connected socket.
 */

#include <webgpu/webgpu.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Create a WebGPU adapter that talks to a remote GPU over `socket_fd`.
 *
 * `socket_fd` must be a connected stream socket (e.g. an accepted websocket
 * connection whose HTTP handshake is already complete).  The adapter takes
 * ownership of the fd and closes it when the adapter is destroyed.
 *
 * Returns NULL on failure; the fd is not consumed in that case.
 */
WGPU_EXPORT WGPUAdapter wgpuRemoteInstanceCreateAdapter(WGPUInstance instance,
                                                        int socket_fd);

/*
 * Latest size of the client's canvas in device pixels, as reported in its
 * ClientHello and any subsequent CanvasResize notifications.  (Resize
 * notifications are absorbed whenever the library reads from the socket,
 * e.g. while waiting for a present acknowledgement.)  0x0 until the client
 * reports a size.
 */
WGPU_EXPORT void wgpuRemoteAdapterGetCanvasSize(WGPUAdapter adapter,
                                                uint32_t *width, uint32_t *height);

#ifdef __cplusplus
}
#endif

#endif /* WEBGPU_REMOTE_H */
