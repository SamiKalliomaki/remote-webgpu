#ifndef GPU_SETUP_H
#define GPU_SETUP_H

#include "gpu_context.h"

/*
 * Bring up a WebGPU instance, surface, adapter and device.  The adapter is
 * created from `gpu_socket_fd`, an already-connected (websocket) socket to
 * the remote GPU; the adapter takes ownership of the fd.  The surface is
 * left configured at the size the client reported for its canvas, falling
 * back to fallback_width x fallback_height if it reported none.
 * Returns 0 on success, non-zero on failure (nothing is left allocated).
 */
int gpu_setup(int gpu_socket_fd, uint32_t fallback_width, uint32_t fallback_height,
              GpuContext *out);

/* Reconfigure the surface after a resize.  Safe to call every frame. */
void gpu_configure_surface(GpuContext *ctx, uint32_t width, uint32_t height);

/*
 * Current size of the client's canvas in device pixels (the size the next
 * frame should be rendered at); ctx->width/height when the client has not
 * reported one.  Resize notifications are picked up while the library
 * waits for present acknowledgements, so this changes between frames.
 */
void gpu_remote_size(const GpuContext *ctx, uint32_t *width, uint32_t *height);

/* Tear down everything gpu_setup() produced, in reverse order. */
void gpu_teardown(GpuContext *ctx);

#endif /* GPU_SETUP_H */
