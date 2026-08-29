#include "render.h"
#include "gpu_setup.h"

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include <webgpu/remote.h>
#include <webgpu/wgpu.h>

static double now_seconds(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

#define COPY_ALIGN 256u

static WGPUStringView sv(const char *s)
{
    WGPUStringView v;
    v.data = s;
    v.length = s ? strlen(s) : 0;
    return v;
}

/* ------------------------------------------------------------------ */
/* shader                                                             */
/* ------------------------------------------------------------------ */

static const char *kShaderSource =
    "struct Uniforms {\n"
    "    angle : f32,\n"
    "    aspect : f32,\n"
    "    _pad : vec2<f32>,\n"
    "};\n"
    "@group(0) @binding(0) var<uniform> u : Uniforms;\n"
    "\n"
    "@vertex\n"
    "fn vs_main(@location(0) pos : vec2<f32>) -> @builtin(position) vec4<f32> {\n"
    "    let s = sin(u.angle);\n"
    "    let c = cos(u.angle);\n"
    "    let rotated = vec2<f32>(pos.x * c - pos.y * s, pos.x * s + pos.y * c);\n"
    "    return vec4<f32>(rotated.x / u.aspect, rotated.y, 0.0, 1.0);\n"
    "}\n"
    "\n"
    "@fragment\n"
    "fn fs_main() -> @location(0) vec4<f32> {\n"
    "    return vec4<f32>(0.75, 0.01, 0.01, 1.0);\n"
    "}\n";

/* An equilateral triangle centred on the origin. */
static const float kVertices[] = {
     0.0f,      0.75f,
    -0.6495f,  -0.375f,
     0.6495f,  -0.375f,
};

typedef struct {
    float angle;
    float aspect;
    float pad[2];
} Uniforms;

/* ------------------------------------------------------------------ */
/* GPU resources owned by the loop (not by the device)                */
/* ------------------------------------------------------------------ */

typedef struct {
    WGPUShaderModule shader;
    WGPUBuffer vertices;
    WGPUBuffer uniforms;
    WGPUBindGroupLayout bind_group_layout;
    WGPUBindGroup bind_group;
    WGPUPipelineLayout pipeline_layout;
    WGPURenderPipeline pipeline;
} Resources;

static void resources_destroy(Resources *r)
{
    if (r->pipeline)           wgpuRenderPipelineRelease(r->pipeline);
    if (r->pipeline_layout)    wgpuPipelineLayoutRelease(r->pipeline_layout);
    if (r->bind_group)         wgpuBindGroupRelease(r->bind_group);
    if (r->bind_group_layout)  wgpuBindGroupLayoutRelease(r->bind_group_layout);
    if (r->uniforms)         { wgpuBufferDestroy(r->uniforms); wgpuBufferRelease(r->uniforms); }
    if (r->vertices)         { wgpuBufferDestroy(r->vertices); wgpuBufferRelease(r->vertices); }
    if (r->shader)             wgpuShaderModuleRelease(r->shader);
    memset(r, 0, sizeof *r);
}

static int resources_create(const GpuContext *ctx, Resources *r)
{
    memset(r, 0, sizeof *r);

    WGPUShaderSourceWGSL wgsl;
    memset(&wgsl, 0, sizeof wgsl);
    wgsl.chain.sType = WGPUSType_ShaderSourceWGSL;
    wgsl.code = sv(kShaderSource);

    WGPUShaderModuleDescriptor shader_desc;
    memset(&shader_desc, 0, sizeof shader_desc);
    shader_desc.nextInChain = &wgsl.chain;
    shader_desc.label = sv("triangle");
    r->shader = wgpuDeviceCreateShaderModule(ctx->device, &shader_desc);
    if (!r->shader)
        goto fail;

    WGPUBufferDescriptor vb_desc;
    memset(&vb_desc, 0, sizeof vb_desc);
    vb_desc.label = sv("triangle vertices");
    vb_desc.usage = WGPUBufferUsage_Vertex | WGPUBufferUsage_CopyDst;
    vb_desc.size = sizeof kVertices;
    r->vertices = wgpuDeviceCreateBuffer(ctx->device, &vb_desc);
    if (!r->vertices)
        goto fail;
    wgpuQueueWriteBuffer(ctx->queue, r->vertices, 0, kVertices, sizeof kVertices);

    WGPUBufferDescriptor ub_desc;
    memset(&ub_desc, 0, sizeof ub_desc);
    ub_desc.label = sv("uniforms");
    ub_desc.usage = WGPUBufferUsage_Uniform | WGPUBufferUsage_CopyDst;
    ub_desc.size = sizeof(Uniforms);
    r->uniforms = wgpuDeviceCreateBuffer(ctx->device, &ub_desc);
    if (!r->uniforms)
        goto fail;

    WGPUBindGroupLayoutEntry bgl_entry;
    memset(&bgl_entry, 0, sizeof bgl_entry);
    bgl_entry.binding = 0;
    bgl_entry.visibility = WGPUShaderStage_Vertex;
    bgl_entry.buffer.type = WGPUBufferBindingType_Uniform;
    bgl_entry.buffer.minBindingSize = sizeof(Uniforms);

    WGPUBindGroupLayoutDescriptor bgl_desc;
    memset(&bgl_desc, 0, sizeof bgl_desc);
    bgl_desc.label = sv("uniform layout");
    bgl_desc.entryCount = 1;
    bgl_desc.entries = &bgl_entry;
    r->bind_group_layout = wgpuDeviceCreateBindGroupLayout(ctx->device, &bgl_desc);
    if (!r->bind_group_layout)
        goto fail;

    WGPUBindGroupEntry bg_entry;
    memset(&bg_entry, 0, sizeof bg_entry);
    bg_entry.binding = 0;
    bg_entry.buffer = r->uniforms;
    bg_entry.size = sizeof(Uniforms);

    WGPUBindGroupDescriptor bg_desc;
    memset(&bg_desc, 0, sizeof bg_desc);
    bg_desc.label = sv("uniform bind group");
    bg_desc.layout = r->bind_group_layout;
    bg_desc.entryCount = 1;
    bg_desc.entries = &bg_entry;
    r->bind_group = wgpuDeviceCreateBindGroup(ctx->device, &bg_desc);
    if (!r->bind_group)
        goto fail;

    WGPUPipelineLayoutDescriptor pl_desc;
    memset(&pl_desc, 0, sizeof pl_desc);
    pl_desc.label = sv("pipeline layout");
    pl_desc.bindGroupLayoutCount = 1;
    pl_desc.bindGroupLayouts = &r->bind_group_layout;
    r->pipeline_layout = wgpuDeviceCreatePipelineLayout(ctx->device, &pl_desc);
    if (!r->pipeline_layout)
        goto fail;

    WGPUVertexAttribute attribute;
    memset(&attribute, 0, sizeof attribute);
    attribute.format = WGPUVertexFormat_Float32x2;
    attribute.offset = 0;
    attribute.shaderLocation = 0;

    WGPUVertexBufferLayout vb_layout;
    memset(&vb_layout, 0, sizeof vb_layout);
    vb_layout.stepMode = WGPUVertexStepMode_Vertex;
    vb_layout.arrayStride = 2 * sizeof(float);
    vb_layout.attributeCount = 1;
    vb_layout.attributes = &attribute;

    WGPUColorTargetState target;
    memset(&target, 0, sizeof target);
    target.format = ctx->surface_format;
    target.writeMask = WGPUColorWriteMask_All;

    WGPUFragmentState fragment;
    memset(&fragment, 0, sizeof fragment);
    fragment.module = r->shader;
    fragment.entryPoint = sv("fs_main");
    fragment.targetCount = 1;
    fragment.targets = &target;

    WGPURenderPipelineDescriptor pipe_desc;
    memset(&pipe_desc, 0, sizeof pipe_desc);
    pipe_desc.label = sv("triangle pipeline");
    pipe_desc.layout = r->pipeline_layout;
    pipe_desc.vertex.module = r->shader;
    pipe_desc.vertex.entryPoint = sv("vs_main");
    pipe_desc.vertex.bufferCount = 1;
    pipe_desc.vertex.buffers = &vb_layout;
    pipe_desc.primitive.topology = WGPUPrimitiveTopology_TriangleList;
    pipe_desc.primitive.frontFace = WGPUFrontFace_CCW;
    pipe_desc.primitive.cullMode = WGPUCullMode_None;
    pipe_desc.multisample.count = 1;
    pipe_desc.multisample.mask = 0xFFFFFFFFu;
    pipe_desc.fragment = &fragment;
    r->pipeline = wgpuDeviceCreateRenderPipeline(ctx->device, &pipe_desc);
    if (!r->pipeline)
        goto fail;

    return 0;

fail:
    fprintf(stderr, "failed to create render resources\n");
    resources_destroy(r);
    return 1;
}

/* ------------------------------------------------------------------ */
/* optional screenshot of the frame that was just drawn               */
/* ------------------------------------------------------------------ */

typedef struct {
    WGPUMapAsyncStatus status;
    int done;
} MapRequest;

typedef struct {
    int done;
} VsyncWait;

static void on_vsync(void *ud1, void *ud2)
{
    (void)ud2;
    ((VsyncWait *)ud1)->done = 1;
}

static void on_buffer_mapped(WGPUMapAsyncStatus status, WGPUStringView message,
                             void *ud1, void *ud2)
{
    MapRequest *req = (MapRequest *)ud1;
    (void)ud2;
    if (status != WGPUMapAsyncStatus_Success && message.data)
        fprintf(stderr, "buffer map failed: %.*s\n", (int)message.length, message.data);
    req->status = status;
    req->done = 1;
}

/* Colour channel order of the surface format, so the PPM comes out right. */
static int format_is_bgra(WGPUTextureFormat format)
{
    return format == WGPUTextureFormat_BGRA8Unorm
        || format == WGPUTextureFormat_BGRA8UnormSrgb;
}

static int write_ppm(const char *path, const unsigned char *rows, uint32_t width,
                     uint32_t height, uint32_t stride, int bgra)
{
    FILE *f = fopen(path, "wb");
    if (!f) {
        fprintf(stderr, "cannot open %s for writing\n", path);
        return 1;
    }
    fprintf(f, "P6\n%u %u\n255\n", width, height);
    for (uint32_t y = 0; y < height; ++y) {
        const unsigned char *row = rows + (size_t)y * stride;
        for (uint32_t x = 0; x < width; ++x) {
            const unsigned char *px = row + (size_t)x * 4;
            unsigned char rgb[3];
            rgb[0] = bgra ? px[2] : px[0];
            rgb[1] = px[1];
            rgb[2] = bgra ? px[0] : px[2];
            fwrite(rgb, 1, 3, f);
        }
    }
    fclose(f);
    return 0;
}

static int save_screenshot(const GpuContext *ctx, WGPUTexture texture, const char *path)
{
    if (!(ctx->surface_usage & WGPUTextureUsage_CopySrc)) {
        fprintf(stderr, "surface does not support CopySrc; cannot screenshot\n");
        return 1;
    }

    const uint32_t width = wgpuTextureGetWidth(texture);
    const uint32_t height = wgpuTextureGetHeight(texture);
    const uint32_t stride = ((width * 4u) + COPY_ALIGN - 1u) / COPY_ALIGN * COPY_ALIGN;
    const uint64_t size = (uint64_t)stride * height;

    WGPUBufferDescriptor desc;
    memset(&desc, 0, sizeof desc);
    desc.label = sv("screenshot readback");
    desc.usage = WGPUBufferUsage_CopyDst | WGPUBufferUsage_MapRead;
    desc.size = size;
    WGPUBuffer readback = wgpuDeviceCreateBuffer(ctx->device, &desc);
    if (!readback)
        return 1;

    WGPUTexelCopyTextureInfo source;
    memset(&source, 0, sizeof source);
    source.texture = texture;
    source.aspect = WGPUTextureAspect_All;

    WGPUTexelCopyBufferInfo destination;
    memset(&destination, 0, sizeof destination);
    destination.buffer = readback;
    destination.layout.bytesPerRow = stride;
    destination.layout.rowsPerImage = height;

    WGPUExtent3D extent = { width, height, 1 };

    WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(ctx->device, NULL);
    wgpuCommandEncoderCopyTextureToBuffer(encoder, &source, &destination, &extent);
    WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
    wgpuQueueSubmit(ctx->queue, 1, &commands);
    wgpuCommandBufferRelease(commands);
    wgpuCommandEncoderRelease(encoder);

    MapRequest req = {0};
    WGPUBufferMapCallbackInfo info;
    memset(&info, 0, sizeof info);
    info.mode = WGPUCallbackMode_AllowProcessEvents;
    info.callback = on_buffer_mapped;
    info.userdata1 = &req;
    wgpuBufferMapAsync(readback, WGPUMapMode_Read, 0, (size_t)size, info);
    /* The map completes when the client's reply is pumped in. */
    while (!req.done)
        if (ctx->pump(ctx->pump_userdata) != 0) {
            fprintf(stderr, "client disconnected during readback\n");
            break;
        }

    int rc = 1;
    if (req.status == WGPUMapAsyncStatus_Success) {
        const unsigned char *data = wgpuBufferGetConstMappedRange(readback, 0, (size_t)size);
        if (data)
            rc = write_ppm(path, data, width, height, stride,
                           format_is_bgra(ctx->surface_format));
        wgpuBufferUnmap(readback);
    }

    wgpuBufferDestroy(readback);
    wgpuBufferRelease(readback);
    return rc;
}

/* ------------------------------------------------------------------ */
/* the main loop                                                      */
/* ------------------------------------------------------------------ */

int render_run(GpuContext *ctx, const RenderOptions *options)
{
    RenderOptions opts = {0};
    if (options)
        opts = *options;

    Resources res;
    if (resources_create(ctx, &res) != 0)
        return 1;

    int rc = 0;
    unsigned frames = 0;
    const double start = now_seconds();

    /* Runs until the client disconnects (or opts.max_frames is reached). */
    for (;;) {
        /* The client dictates the frame size: its canvas, in device pixels.
         * Resize notifications arrive while waiting for present acks. */
        uint32_t fb_width, fb_height;
        gpu_remote_size(ctx, &fb_width, &fb_height);
        if (fb_width != ctx->width || fb_height != ctx->height)
            gpu_configure_surface(ctx, fb_width, fb_height);

        WGPUSurfaceTexture frame;
        memset(&frame, 0, sizeof frame);
        wgpuSurfaceGetCurrentTexture(ctx->surface, &frame);
        if (frame.status == WGPUSurfaceGetCurrentTextureStatus_Outdated
            || frame.status == WGPUSurfaceGetCurrentTextureStatus_Lost) {
            /* The swapchain went stale (resize, compositor change): rebuild it. */
            if (frame.texture)
                wgpuTextureRelease(frame.texture);
            gpu_configure_surface(ctx, fb_width, fb_height);
            continue;
        }
        if (frame.status != WGPUSurfaceGetCurrentTextureStatus_SuccessOptimal
            && frame.status != WGPUSurfaceGetCurrentTextureStatus_SuccessSuboptimal) {
            fprintf(stderr, "wgpuSurfaceGetCurrentTexture failed (status %d)\n",
                    (int)frame.status);
            rc = 1;
            break;
        }

        Uniforms uniforms;
        uniforms.angle = (float)(now_seconds() - start) * 1.5f;
        uniforms.aspect = (float)fb_width / (float)fb_height;
        uniforms.pad[0] = uniforms.pad[1] = 0.0f;
        wgpuQueueWriteBuffer(ctx->queue, res.uniforms, 0, &uniforms, sizeof uniforms);

        WGPUTextureView view = wgpuTextureCreateView(frame.texture, NULL);

        WGPURenderPassColorAttachment attachment;
        memset(&attachment, 0, sizeof attachment);
        attachment.view = view;
        attachment.depthSlice = WGPU_DEPTH_SLICE_UNDEFINED;
        attachment.loadOp = WGPULoadOp_Clear;
        attachment.storeOp = WGPUStoreOp_Store;
        attachment.clearValue = (WGPUColor){ 0.06, 0.07, 0.09, 1.0 };

        WGPURenderPassDescriptor pass_desc;
        memset(&pass_desc, 0, sizeof pass_desc);
        pass_desc.label = sv("triangle pass");
        pass_desc.colorAttachmentCount = 1;
        pass_desc.colorAttachments = &attachment;

        WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(ctx->device, NULL);
        WGPURenderPassEncoder pass = wgpuCommandEncoderBeginRenderPass(encoder, &pass_desc);
        wgpuRenderPassEncoderSetPipeline(pass, res.pipeline);
        wgpuRenderPassEncoderSetBindGroup(pass, 0, res.bind_group, 0, NULL);
        wgpuRenderPassEncoderSetVertexBuffer(pass, 0, res.vertices, 0, sizeof kVertices);
        wgpuRenderPassEncoderDraw(pass, 3, 1, 0, 0);
        wgpuRenderPassEncoderEnd(pass);
        wgpuRenderPassEncoderRelease(pass);

        WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
        wgpuQueueSubmit(ctx->queue, 1, &commands);
        wgpuCommandBufferRelease(commands);
        wgpuCommandEncoderRelease(encoder);

        ++frames;

        const int last_frame = opts.max_frames && frames >= opts.max_frames;
        if (last_frame && opts.screenshot_path)
            rc = save_screenshot(ctx, frame.texture, opts.screenshot_path);

        /* Queue the present, then pump the connection until the client
         * acknowledges the vsync; that acknowledgement paces this loop to
         * the client's refresh rate. */
        wgpuSurfacePresent(ctx->surface);
        VsyncWait vsync = {0};
        WGPURemoteVsyncCallbackInfo vsync_cb = { on_vsync, &vsync, NULL };
        wgpuRemoteSurfaceOnNextVsync(ctx->surface, vsync_cb);

        wgpuTextureViewRelease(view);
        wgpuTextureRelease(frame.texture);

        int disconnected = 0;
        while (!vsync.done)
            if (ctx->pump(ctx->pump_userdata) != 0) {
                disconnected = 1;
                break;
            }
        if (disconnected) {
            /* The client went away; that is the normal way this loop ends. */
            fprintf(stderr, "client disconnected after %u frames\n", frames);
            break;
        }
        if (last_frame)
            break;
    }

    resources_destroy(&res);
    return rc;
}
