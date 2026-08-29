#include "render.h"
#include "gpu_setup.h"

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <webgpu/remote.h>
#include <webgpu/wgpu.h>

/* Rotation per frame, in radians.  Advancing by frame count rather than
 * wall-clock time keeps every frame's content deterministic, which is what
 * lets the e2e tests compare screenshots against a golden image (the vsync
 * pacing still ties the visible speed to the client's refresh rate). */
#define ANGLE_PER_FRAME 0.025f

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
    "    center : vec2<f32>,\n"  /* clip-space position of the triangle */
    "    angle : f32,\n"
    "    aspect : f32,\n"
    "    color : vec4<f32>,\n"
    "    scale : f32,\n"
    "    _pad0 : f32,\n"
    "    _pad1 : vec2<f32>,\n"
    "};\n"
    "@group(0) @binding(0) var<uniform> u : Uniforms;\n"
    "\n"
    "@vertex\n"
    "fn vs_main(@location(0) pos : vec2<f32>) -> @builtin(position) vec4<f32> {\n"
    "    let s = sin(u.angle);\n"
    "    let c = cos(u.angle);\n"
    "    let rotated = vec2<f32>(pos.x * c - pos.y * s, pos.x * s + pos.y * c) * u.scale;\n"
    "    return vec4<f32>(rotated.x / u.aspect + u.center.x, rotated.y + u.center.y,\n"
    "                     0.0, 1.0);\n"
    "}\n"
    "\n"
    "@fragment\n"
    "fn fs_main() -> @location(0) vec4<f32> {\n"
    "    return u.color;\n"
    "}\n";

/* An equilateral triangle centred on the origin. */
static const float kVertices[] = {
     0.0f,      0.75f,
    -0.6495f,  -0.375f,
     0.6495f,  -0.375f,
};

/* Must match the WGSL Uniforms struct above (std140-style layout). */
typedef struct {
    float center[2];
    float angle;
    float aspect;
    float color[4];
    float scale;
    float pad[3];
} Uniforms;

/* One triangle: the big spinning one, and the one under the pointer. */
enum { TRIANGLE_MAIN, TRIANGLE_MOUSE, TRIANGLE_COUNT };

/* ------------------------------------------------------------------ */
/* GPU resources owned by the loop (not by the device)                */
/* ------------------------------------------------------------------ */

typedef struct {
    WGPUShaderModule shader;
    WGPUBuffer vertices;
    WGPUBuffer uniforms[TRIANGLE_COUNT];
    WGPUBindGroupLayout bind_group_layout;
    WGPUBindGroup bind_groups[TRIANGLE_COUNT];
    WGPUPipelineLayout pipeline_layout;
    WGPURenderPipeline pipeline;
} Resources;

static void resources_destroy(Resources *r)
{
    if (r->pipeline)           wgpuRenderPipelineRelease(r->pipeline);
    if (r->pipeline_layout)    wgpuPipelineLayoutRelease(r->pipeline_layout);
    for (int i = 0; i < TRIANGLE_COUNT; ++i)
        if (r->bind_groups[i])  wgpuBindGroupRelease(r->bind_groups[i]);
    if (r->bind_group_layout)  wgpuBindGroupLayoutRelease(r->bind_group_layout);
    for (int i = 0; i < TRIANGLE_COUNT; ++i)
        if (r->uniforms[i])   { wgpuBufferDestroy(r->uniforms[i]); wgpuBufferRelease(r->uniforms[i]); }
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

    for (int i = 0; i < TRIANGLE_COUNT; ++i) {
        WGPUBufferDescriptor ub_desc;
        memset(&ub_desc, 0, sizeof ub_desc);
        ub_desc.label = sv(i == TRIANGLE_MAIN ? "main uniforms" : "mouse uniforms");
        ub_desc.usage = WGPUBufferUsage_Uniform | WGPUBufferUsage_CopyDst;
        ub_desc.size = sizeof(Uniforms);
        r->uniforms[i] = wgpuDeviceCreateBuffer(ctx->device, &ub_desc);
        if (!r->uniforms[i])
            goto fail;
    }

    WGPUBindGroupLayoutEntry bgl_entry;
    memset(&bgl_entry, 0, sizeof bgl_entry);
    bgl_entry.binding = 0;
    bgl_entry.visibility = WGPUShaderStage_Vertex | WGPUShaderStage_Fragment;
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

    for (int i = 0; i < TRIANGLE_COUNT; ++i) {
        WGPUBindGroupEntry bg_entry;
        memset(&bg_entry, 0, sizeof bg_entry);
        bg_entry.binding = 0;
        bg_entry.buffer = r->uniforms[i];
        bg_entry.size = sizeof(Uniforms);

        WGPUBindGroupDescriptor bg_desc;
        memset(&bg_desc, 0, sizeof bg_desc);
        bg_desc.label = sv("uniform bind group");
        bg_desc.layout = r->bind_group_layout;
        bg_desc.entryCount = 1;
        bg_desc.entries = &bg_entry;
        r->bind_groups[i] = wgpuDeviceCreateBindGroup(ctx->device, &bg_desc);
        if (!r->bind_groups[i])
            goto fail;
    }

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
/* client events                                                      */
/* ------------------------------------------------------------------ */

/*
 * State fed by the client's events, which fire from inside ctx->pump()
 * while the loop waits for present acknowledgements.  Resizes are not
 * applied here: the loop reconfigures the surface at the top of the next
 * frame, which is what actually resizes the client's canvas.
 */
typedef struct {
    /* Size the next frame should be rendered at, in device pixels. */
    uint32_t width, height;
    /* Latest pointer position in device pixels; valid once has_mouse. */
    int has_mouse;
    float mouse_x, mouse_y;
} InputState;

static void on_client_event(const WGPURemoteEvent *event, void *ud1, void *ud2)
{
    InputState *input = ud1;
    (void)ud2;

    switch (event->type) {
    case WGPURemoteEventType_CanvasResize:
        input->width = event->width;
        input->height = event->height;
        break;

    case WGPURemoteEventType_User:
        /* "mousemove": two little-endian float32s, device pixels (the
         * payload encoding is defined by the example client). */
        if (!strcmp(event->name, "mousemove")
            && event->payload_size >= 2 * sizeof(float)) {
            float position[2];
            memcpy(position, event->payload, sizeof position);
            input->mouse_x = position[0];
            input->mouse_y = position[1];
            input->has_mouse = 1;
        }
        break;
    }
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

    /* Listen for the client's events: canvas resizes and the example
     * client's "mousemove".  They fire from inside ctx->pump(). */
    InputState input = {0};
    input.width = ctx->width;
    input.height = ctx->height;
    WGPURemoteEventCallbackInfo event_cb = { on_client_event, &input, NULL };
    wgpuRemoteAdapterSetEventCallback(ctx->adapter, event_cb);

    int rc = 0;
    unsigned frames = 0;

    /* Runs until the client disconnects (or opts.max_frames is reached). */
    for (;;) {
        /* The client dictates the frame size: its canvas, in device pixels.
         * Resize events arrive while waiting for present acks; reconfiguring
         * the surface here is what resizes the client's canvas, so the two
         * always match. */
        if (input.width != ctx->width || input.height != ctx->height)
            gpu_configure_surface(ctx, input.width, input.height);

        WGPUSurfaceTexture frame;
        memset(&frame, 0, sizeof frame);
        wgpuSurfaceGetCurrentTexture(ctx->surface, &frame);
        if (frame.status == WGPUSurfaceGetCurrentTextureStatus_Outdated
            || frame.status == WGPUSurfaceGetCurrentTextureStatus_Lost) {
            /* The swapchain went stale (resize, compositor change): rebuild it. */
            if (frame.texture)
                wgpuTextureRelease(frame.texture);
            gpu_configure_surface(ctx, input.width, input.height);
            continue;
        }
        if (frame.status != WGPUSurfaceGetCurrentTextureStatus_SuccessOptimal
            && frame.status != WGPUSurfaceGetCurrentTextureStatus_SuccessSuboptimal) {
            fprintf(stderr, "wgpuSurfaceGetCurrentTexture failed (status %d)\n",
                    (int)frame.status);
            rc = 1;
            break;
        }

        const float aspect = (float)ctx->width / (float)ctx->height;
        const float angle = (float)frames * ANGLE_PER_FRAME;

        Uniforms uniforms;
        memset(&uniforms, 0, sizeof uniforms);
        uniforms.angle = angle;
        uniforms.aspect = aspect;
        uniforms.scale = 1.0f;
        uniforms.color[0] = 0.75f; uniforms.color[1] = 0.01f;
        uniforms.color[2] = 0.01f; uniforms.color[3] = 1.0f;
        wgpuQueueWriteBuffer(ctx->queue, res.uniforms[TRIANGLE_MAIN], 0,
                             &uniforms, sizeof uniforms);

        if (input.has_mouse) {
            /* A small green copy under the pointer: device pixels to clip
             * space (y flipped). */
            uniforms.center[0] = 2.0f * input.mouse_x / (float)ctx->width - 1.0f;
            uniforms.center[1] = 1.0f - 2.0f * input.mouse_y / (float)ctx->height;
            uniforms.scale = 0.25f;
            uniforms.color[0] = 0.05f; uniforms.color[1] = 0.65f;
            uniforms.color[2] = 0.20f; uniforms.color[3] = 1.0f;
            wgpuQueueWriteBuffer(ctx->queue, res.uniforms[TRIANGLE_MOUSE], 0,
                                 &uniforms, sizeof uniforms);
        }

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
        wgpuRenderPassEncoderSetVertexBuffer(pass, 0, res.vertices, 0, sizeof kVertices);
        wgpuRenderPassEncoderSetBindGroup(pass, 0, res.bind_groups[TRIANGLE_MAIN], 0, NULL);
        wgpuRenderPassEncoderDraw(pass, 3, 1, 0, 0);
        if (input.has_mouse) {
            wgpuRenderPassEncoderSetBindGroup(pass, 0, res.bind_groups[TRIANGLE_MOUSE], 0, NULL);
            wgpuRenderPassEncoderDraw(pass, 3, 1, 0, 0);
        }
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

    /* `input` is about to go out of scope. */
    WGPURemoteEventCallbackInfo no_events = {0};
    wgpuRemoteAdapterSetEventCallback(ctx->adapter, no_events);

    resources_destroy(&res);
    return rc;
}
