/* Rendering: writeTexture + sampler + texture bind groups, depth/stencil,
 * blending, an indexed draw, and exact pixel verification of the target. */

#include <stdlib.h>

#include "test_util.h"

static const char kRenderWGSL[] =
    "struct VSOut {\n"
    "  @builtin(position) pos : vec4<f32>,\n"
    "  @location(0) uv : vec2<f32>,\n"
    "};\n"
    "@vertex\n"
    "fn vs(@location(0) p : vec2<f32>, @location(1) uv : vec2<f32>) -> VSOut {\n"
    "  var out : VSOut;\n"
    "  out.pos = vec4<f32>(p, 0.5, 1.0);\n"
    "  out.uv = uv;\n"
    "  return out;\n"
    "}\n"
    "@group(0) @binding(0) var tex : texture_2d<f32>;\n"
    "@group(0) @binding(1) var samp : sampler;\n"
    "@fragment\n"
    "fn fs(in : VSOut) -> @location(0) vec4<f32> {\n"
    "  return textureSample(tex, samp, in.uv);\n"
    "}\n";

#define TARGET_SIZE 64

int run_test(GpuContext *ctx)
{
    WGPUDevice device = ctx->device;

    /* A 2x2 source texture, filled through wgpuQueueWriteTexture. */
    WGPUTextureDescriptor tex_desc = {0};
    tex_desc.label = e2e_sv("source texture");
    tex_desc.usage = WGPUTextureUsage_TextureBinding | WGPUTextureUsage_CopyDst;
    tex_desc.dimension = WGPUTextureDimension_2D;
    tex_desc.size.width = 2;
    tex_desc.size.height = 2;
    tex_desc.size.depthOrArrayLayers = 1;
    tex_desc.format = WGPUTextureFormat_RGBA8Unorm;
    tex_desc.mipLevelCount = 1;
    tex_desc.sampleCount = 1;
    WGPUTexture texture = wgpuDeviceCreateTexture(device, &tex_desc);
    CHECK(texture);
    CHECK(wgpuTextureGetWidth(texture) == 2);
    CHECK(wgpuTextureGetFormat(texture) == WGPUTextureFormat_RGBA8Unorm);

    const uint8_t texels[2][2][4] = {
        { { 255, 0, 0, 255 }, { 0, 255, 0, 255 } },   /* red,  green */
        { { 0, 0, 255, 255 }, { 255, 255, 255, 255 } } /* blue, white */
    };
    WGPUTexelCopyTextureInfo dst = {0};
    dst.texture = texture;
    WGPUTexelCopyBufferLayout layout = {0};
    layout.bytesPerRow = 8;
    layout.rowsPerImage = 2;
    WGPUExtent3D extent = { 2, 2, 1 };
    wgpuQueueWriteTexture(ctx->queue, &dst, texels, sizeof texels, &layout, &extent);

    WGPUSamplerDescriptor sampler_desc = {0};
    sampler_desc.label = e2e_sv("nearest");
    sampler_desc.addressModeU = WGPUAddressMode_ClampToEdge;
    sampler_desc.addressModeV = WGPUAddressMode_ClampToEdge;
    sampler_desc.addressModeW = WGPUAddressMode_ClampToEdge;
    sampler_desc.magFilter = WGPUFilterMode_Nearest;
    sampler_desc.minFilter = WGPUFilterMode_Nearest;
    sampler_desc.mipmapFilter = WGPUMipmapFilterMode_Nearest;
    sampler_desc.lodMaxClamp = 32.0f;
    sampler_desc.maxAnisotropy = 1;
    WGPUSampler sampler = wgpuDeviceCreateSampler(device, &sampler_desc);
    CHECK(sampler);

    /* Explicit bind group layout with texture + sampler entries. */
    WGPUBindGroupLayoutEntry layout_entries[2] = {0};
    layout_entries[0].binding = 0;
    layout_entries[0].visibility = WGPUShaderStage_Fragment;
    layout_entries[0].texture.sampleType = WGPUTextureSampleType_Float;
    layout_entries[0].texture.viewDimension = WGPUTextureViewDimension_2D;
    layout_entries[1].binding = 1;
    layout_entries[1].visibility = WGPUShaderStage_Fragment;
    layout_entries[1].sampler.type = WGPUSamplerBindingType_Filtering;
    WGPUBindGroupLayoutDescriptor bgl_desc = {0};
    bgl_desc.label = e2e_sv("texture bindings");
    bgl_desc.entryCount = 2;
    bgl_desc.entries = layout_entries;
    WGPUBindGroupLayout bgl = wgpuDeviceCreateBindGroupLayout(device, &bgl_desc);

    WGPUPipelineLayoutDescriptor pl_desc = {0};
    pl_desc.bindGroupLayoutCount = 1;
    pl_desc.bindGroupLayouts = &bgl;
    WGPUPipelineLayout pipeline_layout = wgpuDeviceCreatePipelineLayout(device, &pl_desc);

    WGPUTextureView view = wgpuTextureCreateView(texture, NULL);
    WGPUBindGroupEntry group_entries[2] = {0};
    group_entries[0].binding = 0;
    group_entries[0].textureView = view;
    group_entries[1].binding = 1;
    group_entries[1].sampler = sampler;
    WGPUBindGroupDescriptor group_desc = {0};
    group_desc.layout = bgl;
    group_desc.entryCount = 2;
    group_desc.entries = group_entries;
    WGPUBindGroup group = wgpuDeviceCreateBindGroup(device, &group_desc);

    /* Full-screen quad: positions + uvs, drawn indexed. */
    const float vertices[] = {
        /* x,  y,  u, v */
        -1, -1, 0, 1,
         1, -1, 1, 1,
        -1,  1, 0, 0,
         1,  1, 1, 0,
    };
    const uint16_t indices[] = { 0, 1, 2, 2, 1, 3 };

    WGPUBufferDescriptor vb_desc = {0};
    vb_desc.usage = WGPUBufferUsage_Vertex | WGPUBufferUsage_CopyDst;
    vb_desc.size = sizeof vertices;
    WGPUBuffer vertex_buffer = wgpuDeviceCreateBuffer(device, &vb_desc);
    wgpuQueueWriteBuffer(ctx->queue, vertex_buffer, 0, vertices, sizeof vertices);

    WGPUBufferDescriptor ib_desc = {0};
    ib_desc.usage = WGPUBufferUsage_Index | WGPUBufferUsage_CopyDst;
    ib_desc.size = sizeof indices;
    WGPUBuffer index_buffer = wgpuDeviceCreateBuffer(device, &ib_desc);
    wgpuQueueWriteBuffer(ctx->queue, index_buffer, 0, indices, sizeof indices);

    WGPUShaderModule module = e2e_make_shader(device, kRenderWGSL, "render");
    CHECK(module);

    /* Render pipeline with vertex layout, blending and depth/stencil. */
    WGPUVertexAttribute attributes[2] = {0};
    attributes[0].format = WGPUVertexFormat_Float32x2;
    attributes[0].offset = 0;
    attributes[0].shaderLocation = 0;
    attributes[1].format = WGPUVertexFormat_Float32x2;
    attributes[1].offset = 8;
    attributes[1].shaderLocation = 1;
    WGPUVertexBufferLayout vertex_layout = {0};
    vertex_layout.arrayStride = 16;
    vertex_layout.stepMode = WGPUVertexStepMode_Vertex;
    vertex_layout.attributeCount = 2;
    vertex_layout.attributes = attributes;

    WGPUBlendState blend = {0};
    blend.color.operation = WGPUBlendOperation_Add;
    blend.color.srcFactor = WGPUBlendFactor_One;
    blend.color.dstFactor = WGPUBlendFactor_Zero;
    blend.alpha = blend.color;
    WGPUColorTargetState target = {0};
    target.format = WGPUTextureFormat_RGBA8Unorm;
    target.blend = &blend;
    target.writeMask = WGPUColorWriteMask_All;

    WGPUDepthStencilState depth_stencil = {0};
    depth_stencil.format = WGPUTextureFormat_Depth24Plus;
    depth_stencil.depthWriteEnabled = WGPUOptionalBool_True;
    depth_stencil.depthCompare = WGPUCompareFunction_LessEqual;
    depth_stencil.stencilFront.compare = WGPUCompareFunction_Always;
    depth_stencil.stencilFront.failOp = WGPUStencilOperation_Keep;
    depth_stencil.stencilFront.depthFailOp = WGPUStencilOperation_Keep;
    depth_stencil.stencilFront.passOp = WGPUStencilOperation_Keep;
    depth_stencil.stencilBack = depth_stencil.stencilFront;

    WGPUFragmentState fragment = {0};
    fragment.module = module;
    fragment.entryPoint = e2e_sv("fs");
    fragment.targetCount = 1;
    fragment.targets = &target;

    WGPURenderPipelineDescriptor pipeline_desc = {0};
    pipeline_desc.label = e2e_sv("textured quad");
    pipeline_desc.layout = pipeline_layout;
    pipeline_desc.vertex.module = module;
    pipeline_desc.vertex.entryPoint = e2e_sv("vs");
    pipeline_desc.vertex.bufferCount = 1;
    pipeline_desc.vertex.buffers = &vertex_layout;
    pipeline_desc.primitive.topology = WGPUPrimitiveTopology_TriangleList;
    pipeline_desc.primitive.frontFace = WGPUFrontFace_CCW;
    pipeline_desc.primitive.cullMode = WGPUCullMode_None;
    pipeline_desc.depthStencil = &depth_stencil;
    pipeline_desc.multisample.count = 1;
    pipeline_desc.multisample.mask = 0xFFFFFFFF;
    pipeline_desc.fragment = &fragment;
    WGPURenderPipeline pipeline = wgpuDeviceCreateRenderPipeline(device, &pipeline_desc);
    CHECK(pipeline);

    /* Offscreen color + depth targets. */
    WGPUTextureDescriptor color_desc = {0};
    color_desc.label = e2e_sv("color target");
    color_desc.usage = WGPUTextureUsage_RenderAttachment | WGPUTextureUsage_CopySrc;
    color_desc.dimension = WGPUTextureDimension_2D;
    color_desc.size.width = TARGET_SIZE;
    color_desc.size.height = TARGET_SIZE;
    color_desc.size.depthOrArrayLayers = 1;
    color_desc.format = WGPUTextureFormat_RGBA8Unorm;
    color_desc.mipLevelCount = 1;
    color_desc.sampleCount = 1;
    WGPUTexture color_target = wgpuDeviceCreateTexture(device, &color_desc);
    WGPUTextureView color_view = wgpuTextureCreateView(color_target, NULL);

    WGPUTextureDescriptor depth_desc = color_desc;
    depth_desc.label = e2e_sv("depth target");
    depth_desc.usage = WGPUTextureUsage_RenderAttachment;
    depth_desc.format = WGPUTextureFormat_Depth24Plus;
    WGPUTexture depth_target = wgpuDeviceCreateTexture(device, &depth_desc);
    WGPUTextureView depth_view = wgpuTextureCreateView(depth_target, NULL);

    WGPUBufferDescriptor read_desc = {0};
    read_desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    read_desc.size = TARGET_SIZE * TARGET_SIZE * 4;
    WGPUBuffer readback = wgpuDeviceCreateBuffer(device, &read_desc);

    WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(device, NULL);
    WGPURenderPassColorAttachment color_attachment = {0};
    color_attachment.view = color_view;
    color_attachment.depthSlice = WGPU_DEPTH_SLICE_UNDEFINED;
    color_attachment.loadOp = WGPULoadOp_Clear;
    color_attachment.storeOp = WGPUStoreOp_Store;
    color_attachment.clearValue.a = 1.0;
    WGPURenderPassDepthStencilAttachment depth_attachment = {0};
    depth_attachment.view = depth_view;
    depth_attachment.depthLoadOp = WGPULoadOp_Clear;
    depth_attachment.depthStoreOp = WGPUStoreOp_Store;
    depth_attachment.depthClearValue = 1.0f;
    WGPURenderPassDescriptor pass_desc = {0};
    pass_desc.label = e2e_sv("render test pass");
    pass_desc.colorAttachmentCount = 1;
    pass_desc.colorAttachments = &color_attachment;
    pass_desc.depthStencilAttachment = &depth_attachment;
    WGPURenderPassEncoder pass = wgpuCommandEncoderBeginRenderPass(encoder, &pass_desc);
    wgpuRenderPassEncoderSetViewport(pass, 0, 0, TARGET_SIZE, TARGET_SIZE, 0, 1);
    wgpuRenderPassEncoderSetScissorRect(pass, 0, 0, TARGET_SIZE, TARGET_SIZE);
    wgpuRenderPassEncoderSetPipeline(pass, pipeline);
    wgpuRenderPassEncoderSetBindGroup(pass, 0, group, 0, NULL);
    wgpuRenderPassEncoderSetVertexBuffer(pass, 0, vertex_buffer, 0, sizeof vertices);
    wgpuRenderPassEncoderSetIndexBuffer(pass, index_buffer, WGPUIndexFormat_Uint16,
                                        0, sizeof indices);
    wgpuRenderPassEncoderDrawIndexed(pass, 6, 1, 0, 0, 0);
    wgpuRenderPassEncoderEnd(pass);

    WGPUTexelCopyTextureInfo copy_src = {0};
    copy_src.texture = color_target;
    WGPUTexelCopyBufferInfo copy_dst = {0};
    copy_dst.buffer = readback;
    copy_dst.layout.bytesPerRow = TARGET_SIZE * 4;
    copy_dst.layout.rowsPerImage = TARGET_SIZE;
    WGPUExtent3D copy_size = { TARGET_SIZE, TARGET_SIZE, 1 };
    wgpuCommandEncoderCopyTextureToBuffer(encoder, &copy_src, &copy_dst, &copy_size);

    WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
    wgpuQueueSubmit(ctx->queue, 1, &commands);

    uint8_t *pixels = malloc(TARGET_SIZE * TARGET_SIZE * 4);
    CHECK(pixels);
    if (e2e_read_back(ctx, readback, 0, TARGET_SIZE * TARGET_SIZE * 4, pixels) != 0) {
        free(pixels);
        return -1;
    }

    /* Quadrant centers must hold the source texels exactly. */
    struct { int x, y; const uint8_t *rgba; } checks[] = {
        { TARGET_SIZE / 4,     TARGET_SIZE / 4,     texels[0][0] }, /* red */
        { TARGET_SIZE * 3 / 4, TARGET_SIZE / 4,     texels[0][1] }, /* green */
        { TARGET_SIZE / 4,     TARGET_SIZE * 3 / 4, texels[1][0] }, /* blue */
        { TARGET_SIZE * 3 / 4, TARGET_SIZE * 3 / 4, texels[1][1] }, /* white */
    };
    for (size_t i = 0; i < sizeof checks / sizeof checks[0]; ++i) {
        const uint8_t *p = pixels + (checks[i].y * TARGET_SIZE + checks[i].x) * 4;
        if (memcmp(p, checks[i].rgba, 4) != 0) {
            fprintf(stderr, "e2e: pixel (%d,%d) = %u,%u,%u,%u; expected "
                            "%u,%u,%u,%u\n", checks[i].x, checks[i].y,
                    p[0], p[1], p[2], p[3], checks[i].rgba[0], checks[i].rgba[1],
                    checks[i].rgba[2], checks[i].rgba[3]);
            free(pixels);
            return -1;
        }
    }
    free(pixels);

    wgpuCommandBufferRelease(commands);
    wgpuCommandEncoderRelease(encoder);
    wgpuRenderPassEncoderRelease(pass);
    wgpuBufferRelease(readback);
    wgpuTextureViewRelease(depth_view);
    wgpuTextureRelease(depth_target);
    wgpuTextureViewRelease(color_view);
    wgpuTextureRelease(color_target);
    wgpuRenderPipelineRelease(pipeline);
    wgpuShaderModuleRelease(module);
    wgpuBufferRelease(index_buffer);
    wgpuBufferRelease(vertex_buffer);
    wgpuBindGroupRelease(group);
    wgpuTextureViewRelease(view);
    wgpuPipelineLayoutRelease(pipeline_layout);
    wgpuBindGroupLayoutRelease(bgl);
    wgpuSamplerRelease(sampler);
    wgpuTextureRelease(texture);
    return 0;
}
