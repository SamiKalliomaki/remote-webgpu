#include "gpu_setup.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <webgpu/remote.h>

/* ------------------------------------------------------------------ */
/* helpers                                                            */
/* ------------------------------------------------------------------ */

static WGPUStringView sv(const char *s)
{
    WGPUStringView v;
    v.data = s;
    v.length = s ? strlen(s) : 0;
    return v;
}

static void print_message(const char *prefix, WGPUStringView msg)
{
    if (msg.data && msg.length)
        fprintf(stderr, "%s: %.*s\n", prefix, (int)msg.length, msg.data);
    else
        fprintf(stderr, "%s\n", prefix);
}

static void on_device_lost(WGPUDevice const *device, WGPUDeviceLostReason reason,
                           WGPUStringView message, void *ud1, void *ud2)
{
    (void)device; (void)reason; (void)ud1; (void)ud2;
    print_message("wgpu device lost", message);
}

static void on_uncaptured_error(WGPUDevice const *device, WGPUErrorType type,
                                WGPUStringView message, void *ud1, void *ud2)
{
    (void)device; (void)ud1; (void)ud2;
    fprintf(stderr, "wgpu error (type %d)\n", (int)type);
    print_message("  ", message);
}

/* ------------------------------------------------------------------ */
/* device request                                                     */
/*                                                                    */
/* The remote implementation resolves this synchronously from within   */
/* the request call, so a plain stack slot catches the result.         */
/* ------------------------------------------------------------------ */

typedef struct {
    WGPUDevice device;
    int done;
} DeviceRequest;

static void on_device(WGPURequestDeviceStatus status, WGPUDevice device,
                      WGPUStringView message, void *ud1, void *ud2)
{
    DeviceRequest *req = (DeviceRequest *)ud1;
    (void)ud2;
    if (status == WGPURequestDeviceStatus_Success)
        req->device = device;
    else
        print_message("wgpuAdapterRequestDevice failed", message);
    req->done = 1;
}

/* GpuContext.pump: void* adapter shim over connection_pump(). */
static int pump_connection(void *userdata)
{
    return connection_pump(userdata);
}

/* ------------------------------------------------------------------ */
/* public API                                                         */
/* ------------------------------------------------------------------ */

void gpu_configure_surface(GpuContext *ctx, uint32_t width, uint32_t height)
{
    if (width == 0 || height == 0)
        return;

    WGPUSurfaceConfiguration config;
    memset(&config, 0, sizeof config);
    config.device = ctx->device;
    config.format = ctx->surface_format;
    config.usage = ctx->surface_usage;
    config.width = width;
    config.height = height;
    config.alphaMode = ctx->alpha_mode;
    config.presentMode = ctx->present_mode;

    wgpuSurfaceConfigure(ctx->surface, &config);
    ctx->width = width;
    ctx->height = height;
}

int gpu_setup(Connection *conn, uint32_t fallback_width, uint32_t fallback_height,
              GpuContext *out)
{
    memset(out, 0, sizeof *out);
    out->pump = pump_connection;
    out->pump_userdata = conn;

    out->instance = wgpuCreateInstance(NULL);
    if (!out->instance) {
        fprintf(stderr, "failed to create WebGPU instance\n");
        goto fail;
    }

    /* The surface is the client's canvas; there is no native window, so a
     * plain descriptor with no platform source chained in. */
    WGPUSurfaceDescriptor surface_desc;
    memset(&surface_desc, 0, sizeof surface_desc);
    surface_desc.label = sv("remote canvas");
    out->surface = wgpuInstanceCreateSurface(out->instance, &surface_desc);
    if (!out->surface) {
        fprintf(stderr, "failed to create WebGPU surface\n");
        goto fail;
    }

    /* The adapter is not discovered locally: it sends to the remote GPU
     * through the connection.  It becomes ready once the client's hello
     * has been pumped in. */
    out->adapter = wgpuRemoteInstanceCreateAdapter(out->instance, connection_send, conn);
    if (!out->adapter) {
        fprintf(stderr, "failed to create remote WebGPU adapter\n");
        goto fail;
    }
    conn->adapter = out->adapter;
    while (!wgpuRemoteAdapterIsReady(out->adapter)) {
        if (connection_pump(conn) != 0) {
            fprintf(stderr, "client disconnected during handshake\n");
            goto fail;
        }
    }

    WGPUDeviceDescriptor device_desc;
    memset(&device_desc, 0, sizeof device_desc);
    device_desc.label = sv("main device");
    device_desc.deviceLostCallbackInfo.mode = WGPUCallbackMode_AllowProcessEvents;
    device_desc.deviceLostCallbackInfo.callback = on_device_lost;
    device_desc.uncapturedErrorCallbackInfo.callback = on_uncaptured_error;

    DeviceRequest device_req = {0};
    WGPURequestDeviceCallbackInfo device_cb;
    memset(&device_cb, 0, sizeof device_cb);
    device_cb.mode = WGPUCallbackMode_AllowProcessEvents;
    device_cb.callback = on_device;
    device_cb.userdata1 = &device_req;
    wgpuAdapterRequestDevice(out->adapter, &device_desc, device_cb);
    while (!device_req.done)
        wgpuInstanceProcessEvents(out->instance);
    out->device = device_req.device;
    if (!out->device)
        goto fail;

    out->queue = wgpuDeviceGetQueue(out->device);

    WGPUSurfaceCapabilities caps;
    memset(&caps, 0, sizeof caps);
    if (wgpuSurfaceGetCapabilities(out->surface, out->adapter, &caps) != WGPUStatus_Success
        || caps.formatCount == 0) {
        fprintf(stderr, "surface reports no supported formats\n");
        goto fail;
    }
    out->surface_format = caps.formats[0];
    /* CopySrc is optional but lets the caller grab a screenshot of a frame. */
    out->surface_usage = WGPUTextureUsage_RenderAttachment
                       | (caps.usages & WGPUTextureUsage_CopySrc);
    out->present_mode = caps.presentModeCount ? caps.presentModes[0] : WGPUPresentMode_Fifo;
    out->alpha_mode = caps.alphaModeCount ? caps.alphaModes[0] : WGPUCompositeAlphaMode_Auto;
    wgpuSurfaceCapabilitiesFreeMembers(caps);

    /* First configuration: the size the client reported for its canvas. */
    uint32_t width, height;
    wgpuRemoteAdapterGetCanvasSize(out->adapter, &width, &height);
    if (width == 0 || height == 0) {
        width = fallback_width;
        height = fallback_height;
    }
    gpu_configure_surface(out, width, height);

    return 0;

fail:
    gpu_teardown(out);
    return 1;
}

void gpu_teardown(GpuContext *ctx)
{
    if (ctx->queue)    wgpuQueueRelease(ctx->queue);
    if (ctx->device)   wgpuDeviceRelease(ctx->device);
    if (ctx->adapter)  wgpuAdapterRelease(ctx->adapter);
    if (ctx->surface)  wgpuSurfaceRelease(ctx->surface);
    if (ctx->instance) wgpuInstanceRelease(ctx->instance);
    memset(ctx, 0, sizeof *ctx);
}
