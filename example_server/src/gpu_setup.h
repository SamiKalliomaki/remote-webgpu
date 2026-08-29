#ifndef GPU_SETUP_H
#define GPU_SETUP_H

#include "connection.h"
#include "gpu_context.h"

/*
 * Bring up a WebGPU instance, surface, adapter and device on top of an
 * accepted Connection.  The adapter sends through connection_send() and the
 * handshake is driven by pumping the connection until the client's hello
 * arrives.  The surface is left configured at the size the client reported
 * for its canvas, falling back to fallback_width x fallback_height if it
 * reported none.  The connection remains owned by the caller.
 * Returns 0 on success, non-zero on failure (nothing is left allocated).
 */
int gpu_setup(Connection *conn, uint32_t fallback_width, uint32_t fallback_height,
              GpuContext *out);

/* Reconfigure the surface after a resize.  Safe to call every frame. */
void gpu_configure_surface(GpuContext *ctx, uint32_t width, uint32_t height);

/* Tear down everything gpu_setup() produced, in reverse order. */
void gpu_teardown(GpuContext *ctx);

#endif /* GPU_SETUP_H */
