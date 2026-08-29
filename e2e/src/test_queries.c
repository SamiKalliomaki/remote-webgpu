/* Occlusion queries: begin/end around a draw, resolveQuerySet into a
 * buffer, and a verified readback of the sample count. */

#include "test_util.h"

static const char kSolidWGSL[] =
    "@vertex\n"
    "fn vs(@builtin(vertex_index) i : u32) -> @builtin(position) vec4<f32> {\n"
    "  var p = array<vec2<f32>, 3>(\n"
    "    vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));\n"
    "  return vec4<f32>(p[i], 0.0, 1.0);\n"
    "}\n"
    "@fragment\n"
    "fn fs() -> @location(0) vec4<f32> {\n"
    "  return vec4<f32>(1.0, 0.0, 1.0, 1.0);\n"
    "}\n";

#define TARGET_SIZE 32

int run_test(GpuContext *ctx)
{
    WGPUDevice device = ctx->device;

    WGPUShaderModule module = e2e_make_shader(device, kSolidWGSL, "solid");
    CHECK(module);

    WGPUColorTargetState target = {0};
    target.format = WGPUTextureFormat_RGBA8Unorm;
    target.writeMask = WGPUColorWriteMask_All;
    WGPUFragmentState fragment = {0};
    fragment.module = module;
    fragment.entryPoint = e2e_sv("fs");
    fragment.targetCount = 1;
    fragment.targets = &target;
    WGPURenderPipelineDescriptor pipeline_desc = {0};
    pipeline_desc.label = e2e_sv("full-screen solid");
    pipeline_desc.vertex.module = module;
    pipeline_desc.vertex.entryPoint = e2e_sv("vs");
    pipeline_desc.primitive.topology = WGPUPrimitiveTopology_TriangleList;
    pipeline_desc.multisample.count = 1;
    pipeline_desc.multisample.mask = 0xFFFFFFFF;
    pipeline_desc.fragment = &fragment;
    WGPURenderPipeline pipeline = wgpuDeviceCreateRenderPipeline(device, &pipeline_desc);
    CHECK(pipeline);

    WGPUTextureDescriptor color_desc = {0};
    color_desc.label = e2e_sv("query color target");
    color_desc.usage = WGPUTextureUsage_RenderAttachment;
    color_desc.dimension = WGPUTextureDimension_2D;
    color_desc.size.width = TARGET_SIZE;
    color_desc.size.height = TARGET_SIZE;
    color_desc.size.depthOrArrayLayers = 1;
    color_desc.format = WGPUTextureFormat_RGBA8Unorm;
    color_desc.mipLevelCount = 1;
    color_desc.sampleCount = 1;
    WGPUTexture color_target = wgpuDeviceCreateTexture(device, &color_desc);
    WGPUTextureView color_view = wgpuTextureCreateView(color_target, NULL);

    WGPUQuerySetDescriptor query_desc = {0};
    query_desc.label = e2e_sv("occlusion");
    query_desc.type = WGPUQueryType_Occlusion;
    query_desc.count = 2;
    WGPUQuerySet query_set = wgpuDeviceCreateQuerySet(device, &query_desc);
    CHECK(query_set);
    CHECK(wgpuQuerySetGetType(query_set) == WGPUQueryType_Occlusion);
    CHECK(wgpuQuerySetGetCount(query_set) == 2);

    WGPUBufferDescriptor resolve_desc = {0};
    resolve_desc.label = e2e_sv("query resolve");
    resolve_desc.usage = WGPUBufferUsage_QueryResolve | WGPUBufferUsage_CopySrc;
    resolve_desc.size = 2 * 8;
    WGPUBuffer resolve_buffer = wgpuDeviceCreateBuffer(device, &resolve_desc);

    WGPUBufferDescriptor read_desc = {0};
    read_desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    read_desc.size = 2 * 8;
    WGPUBuffer readback = wgpuDeviceCreateBuffer(device, &read_desc);

    WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(device, NULL);
    WGPURenderPassColorAttachment color_attachment = {0};
    color_attachment.view = color_view;
    color_attachment.depthSlice = WGPU_DEPTH_SLICE_UNDEFINED;
    color_attachment.loadOp = WGPULoadOp_Clear;
    color_attachment.storeOp = WGPUStoreOp_Store;
    WGPURenderPassDescriptor pass_desc = {0};
    pass_desc.label = e2e_sv("query pass");
    pass_desc.colorAttachmentCount = 1;
    pass_desc.colorAttachments = &color_attachment;
    pass_desc.occlusionQuerySet = query_set;
    WGPURenderPassEncoder pass = wgpuCommandEncoderBeginRenderPass(encoder, &pass_desc);
    wgpuRenderPassEncoderSetPipeline(pass, pipeline);

    /* Query 0: a full-screen draw -- must see samples. */
    wgpuRenderPassEncoderBeginOcclusionQuery(pass, 0);
    wgpuRenderPassEncoderDraw(pass, 3, 1, 0, 0);
    wgpuRenderPassEncoderEndOcclusionQuery(pass);

    /* Query 1: no draw at all -- must see no samples. */
    wgpuRenderPassEncoderBeginOcclusionQuery(pass, 1);
    wgpuRenderPassEncoderEndOcclusionQuery(pass);
    wgpuRenderPassEncoderEnd(pass);

    wgpuCommandEncoderResolveQuerySet(encoder, query_set, 0, 2, resolve_buffer, 0);
    wgpuCommandEncoderCopyBufferToBuffer(encoder, resolve_buffer, 0, readback, 0, 2 * 8);
    WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
    wgpuQueueSubmit(ctx->queue, 1, &commands);

    uint64_t counts[2] = { 0, 0 };
    CHECK(e2e_read_back(ctx, readback, 0, sizeof counts, counts) == 0);
    fprintf(stderr, "e2e: occlusion query counts: %llu, %llu\n",
            (unsigned long long)counts[0], (unsigned long long)counts[1]);
    CHECK(counts[0] > 0);
    CHECK(counts[1] == 0);

    wgpuCommandBufferRelease(commands);
    wgpuCommandEncoderRelease(encoder);
    wgpuRenderPassEncoderRelease(pass);
    wgpuBufferRelease(readback);
    wgpuBufferRelease(resolve_buffer);
    wgpuQuerySetDestroy(query_set);
    wgpuQuerySetRelease(query_set);
    wgpuTextureViewRelease(color_view);
    wgpuTextureRelease(color_target);
    wgpuRenderPipelineRelease(pipeline);
    wgpuShaderModuleRelease(module);
    return 0;
}
