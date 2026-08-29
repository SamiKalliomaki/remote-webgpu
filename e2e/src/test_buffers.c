/* Buffer plumbing: mapped-at-creation writes, queue writes, buffer-to-
 * buffer copies, clearBuffer, read mapping and the local getters. */

#include "test_util.h"

#define N 256

int run_test(GpuContext *ctx)
{
    WGPUDevice device = ctx->device;

    /* Buffer A: filled through the mapped-at-creation path, then a range
     * cleared GPU-side. */
    WGPUBufferDescriptor a_desc = {0};
    a_desc.label = e2e_sv("buffer a");
    a_desc.usage = WGPUBufferUsage_CopySrc | WGPUBufferUsage_CopyDst;
    a_desc.size = N;
    a_desc.mappedAtCreation = 1;
    WGPUBuffer a = wgpuDeviceCreateBuffer(device, &a_desc);
    CHECK(a);
    CHECK(wgpuBufferGetSize(a) == N);
    CHECK(wgpuBufferGetUsage(a) == a_desc.usage);
    CHECK(wgpuBufferGetMapState(a) == WGPUBufferMapState_Mapped);
    uint8_t *mapped = wgpuBufferGetMappedRange(a, 0, N);
    CHECK(mapped);
    for (int i = 0; i < N; ++i)
        mapped[i] = (uint8_t)i;
    /* wgpuBufferWriteMappedRange is the copying flavor of the same thing. */
    const uint8_t marker[4] = { 0xAA, 0xBB, 0xCC, 0xDD };
    CHECK(wgpuBufferWriteMappedRange(a, 0, marker, sizeof marker)
          == WGPUStatus_Success);
    wgpuBufferUnmap(a);
    CHECK(wgpuBufferGetMapState(a) == WGPUBufferMapState_Unmapped);

    /* Buffer B: filled through wgpuQueueWriteBuffer. */
    WGPUBufferDescriptor b_desc = {0};
    b_desc.label = e2e_sv("buffer b");
    b_desc.usage = WGPUBufferUsage_CopySrc | WGPUBufferUsage_CopyDst;
    b_desc.size = N;
    WGPUBuffer b = wgpuDeviceCreateBuffer(device, &b_desc);
    uint8_t pattern[N];
    for (int i = 0; i < N; ++i)
        pattern[i] = (uint8_t)(255 - i);
    wgpuQueueWriteBuffer(ctx->queue, b, 0, pattern, N);

    WGPUBufferDescriptor read_desc = {0};
    read_desc.label = e2e_sv("readback");
    read_desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    read_desc.size = 2 * N;
    WGPUBuffer readback = wgpuDeviceCreateBuffer(device, &read_desc);

    WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(device, NULL);
    wgpuCommandEncoderClearBuffer(encoder, a, 64, 64);
    wgpuCommandEncoderCopyBufferToBuffer(encoder, a, 0, readback, 0, N);
    wgpuCommandEncoderCopyBufferToBuffer(encoder, b, 0, readback, N, N);
    WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
    wgpuQueueSubmit(ctx->queue, 1, &commands);
    wgpuCommandBufferRelease(commands);
    wgpuCommandEncoderRelease(encoder);

    uint8_t results[2 * N];
    CHECK(e2e_read_back(ctx, readback, 0, sizeof results, results) == 0);
    for (int i = 0; i < N; ++i) {
        uint8_t expected;
        if (i < 4)
            expected = marker[i];
        else if (i >= 64 && i < 128)
            expected = 0; /* cleared */
        else
            expected = (uint8_t)i;
        CHECK(results[i] == expected);
    }
    for (int i = 0; i < N; ++i)
        CHECK(results[N + i] == pattern[i]);

    wgpuBufferRelease(readback);
    wgpuBufferRelease(b);
    wgpuBufferRelease(a);
    return 0;
}
