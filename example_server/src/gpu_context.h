#ifndef GPU_CONTEXT_H
#define GPU_CONTEXT_H

#include <webgpu/webgpu.h>

/*
 * Everything the renderer needs in order to draw, and nothing about how it
 * was obtained.  gpu_setup.c is the only place that knows about the socket
 * handover and the adapter/device request dance; the main loop just
 * consumes this struct.  There is no OS window: the "surface" is the
 * connected client's canvas.
 */
typedef struct {
    WGPUInstance instance;
    WGPUSurface surface;
    WGPUAdapter adapter;
    WGPUDevice device;
    WGPUQueue queue;
    WGPUTextureFormat surface_format;
    WGPUTextureUsage surface_usage;
    WGPUPresentMode present_mode;
    WGPUCompositeAlphaMode alpha_mode;
    uint32_t width;
    uint32_t height;
} GpuContext;

#endif /* GPU_CONTEXT_H */
