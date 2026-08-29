#ifndef RENDER_H
#define RENDER_H

#include "gpu_context.h"

typedef struct {
    /* Stop after this many frames; 0 means run until the window is closed. */
    unsigned max_frames;
    /* If non-NULL, write the last rendered frame here as a binary PPM. */
    const char *screenshot_path;
} RenderOptions;

/*
 * Draw a spinning red triangle until the window is closed.  The device in
 * `ctx` is already created and owned by the caller; the loop only borrows it
 * (it reconfigures the surface on resize, but never creates or destroys it).
 * Returns 0 on success.
 */
int render_run(GpuContext *ctx, const RenderOptions *options);

#endif /* RENDER_H */
