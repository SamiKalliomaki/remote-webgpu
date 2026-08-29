/* Compute: a pipeline with an implicit ("auto") layout obtained through
 * getBindGroupLayout, a dispatch and a verified readback. */

#include "test_util.h"

static const char kComputeWGSL[] =
    "@group(0) @binding(0) var<storage, read> input : array<u32>;\n"
    "@group(0) @binding(1) var<storage, read_write> output : array<u32>;\n"
    "@compute @workgroup_size(64)\n"
    "fn main(@builtin(global_invocation_id) gid : vec3<u32>) {\n"
    "  if (gid.x < arrayLength(&output)) {\n"
    "    output[gid.x] = input[gid.x] * 2u + 1u;\n"
    "  }\n"
    "}\n";

#define N 256

int run_test(GpuContext *ctx)
{
    WGPUDevice device = ctx->device;

    WGPUBufferDescriptor input_desc = {0};
    input_desc.label = e2e_sv("compute input");
    input_desc.usage = WGPUBufferUsage_Storage | WGPUBufferUsage_CopyDst;
    input_desc.size = N * 4;
    WGPUBuffer input = wgpuDeviceCreateBuffer(device, &input_desc);
    uint32_t values[N];
    for (uint32_t i = 0; i < N; ++i)
        values[i] = i;
    wgpuQueueWriteBuffer(ctx->queue, input, 0, values, sizeof values);

    WGPUBufferDescriptor output_desc = {0};
    output_desc.label = e2e_sv("compute output");
    output_desc.usage = WGPUBufferUsage_Storage | WGPUBufferUsage_CopySrc;
    output_desc.size = N * 4;
    WGPUBuffer output = wgpuDeviceCreateBuffer(device, &output_desc);

    WGPUBufferDescriptor read_desc = {0};
    read_desc.label = e2e_sv("compute readback");
    read_desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    read_desc.size = N * 4;
    WGPUBuffer readback = wgpuDeviceCreateBuffer(device, &read_desc);

    WGPUShaderModule module = e2e_make_shader(device, kComputeWGSL, "compute");
    CHECK(module);

    WGPUComputePipelineDescriptor pipeline_desc = {0};
    pipeline_desc.label = e2e_sv("double-and-one");
    pipeline_desc.compute.module = module;
    pipeline_desc.compute.entryPoint = e2e_sv("main");
    WGPUComputePipeline pipeline = wgpuDeviceCreateComputePipeline(device, &pipeline_desc);
    CHECK(pipeline);
    WGPUBindGroupLayout layout = wgpuComputePipelineGetBindGroupLayout(pipeline, 0);
    CHECK(layout);

    WGPUBindGroupEntry entries[2] = {0};
    entries[0].binding = 0;
    entries[0].buffer = input;
    entries[0].size = WGPU_WHOLE_SIZE;
    entries[1].binding = 1;
    entries[1].buffer = output;
    entries[1].size = WGPU_WHOLE_SIZE;
    WGPUBindGroupDescriptor group_desc = {0};
    group_desc.label = e2e_sv("compute bindings");
    group_desc.layout = layout;
    group_desc.entryCount = 2;
    group_desc.entries = entries;
    WGPUBindGroup group = wgpuDeviceCreateBindGroup(device, &group_desc);

    WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(device, NULL);
    wgpuCommandEncoderPushDebugGroup(encoder, e2e_sv("compute test"));
    WGPUComputePassEncoder pass = wgpuCommandEncoderBeginComputePass(encoder, NULL);
    wgpuComputePassEncoderSetPipeline(pass, pipeline);
    wgpuComputePassEncoderSetBindGroup(pass, 0, group, 0, NULL);
    wgpuComputePassEncoderDispatchWorkgroups(pass, (N + 63) / 64, 1, 1);
    wgpuComputePassEncoderEnd(pass);
    wgpuCommandEncoderCopyBufferToBuffer(encoder, output, 0, readback, 0, N * 4);
    wgpuCommandEncoderPopDebugGroup(encoder);
    WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
    wgpuQueueSubmit(ctx->queue, 1, &commands);

    uint32_t results[N] = {0};
    CHECK(e2e_read_back(ctx, readback, 0, sizeof results, results) == 0);
    for (uint32_t i = 0; i < N; ++i)
        CHECK(results[i] == i * 2 + 1);

    wgpuCommandBufferRelease(commands);
    wgpuCommandEncoderRelease(encoder);
    wgpuComputePassEncoderRelease(pass);
    wgpuBindGroupRelease(group);
    wgpuBindGroupLayoutRelease(layout);
    wgpuComputePipelineRelease(pipeline);
    wgpuShaderModuleRelease(module);
    wgpuBufferRelease(readback);
    wgpuBufferRelease(output);
    wgpuBufferRelease(input);
    return 0;
}
