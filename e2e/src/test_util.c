#include "test_util.h"

WGPUStringView e2e_sv(const char *s)
{
    WGPUStringView v = { s, s ? strlen(s) : 0 };
    return v;
}

int e2e_wait_flag(GpuContext *ctx, const int *flag)
{
    while (!*flag)
        if (ctx->pump(ctx->pump_userdata) != 0)
            return -1;
    return 0;
}

static void on_map(WGPUMapAsyncStatus status, WGPUStringView message,
                   void *userdata1, void *userdata2)
{
    (void)userdata2;
    if (status != WGPUMapAsyncStatus_Success)
        fprintf(stderr, "e2e: map failed: %.*s\n",
                (int)message.length, message.data ? message.data : "");
    *(int *)userdata1 = status == WGPUMapAsyncStatus_Success ? 1 : -1;
}

int e2e_read_back(GpuContext *ctx, WGPUBuffer buffer, uint64_t offset,
                  size_t size, void *out)
{
    int mapped = 0;
    WGPUBufferMapCallbackInfo map_cb = {0};
    map_cb.callback = on_map;
    map_cb.userdata1 = &mapped;
    wgpuBufferMapAsync(buffer, WGPUMapMode_Read, (size_t)offset, size, map_cb);
    CHECK(e2e_wait_flag(ctx, &mapped) == 0);
    CHECK(mapped == 1);
    CHECK(wgpuBufferGetMapState(buffer) == WGPUBufferMapState_Mapped);
    CHECK(wgpuBufferReadMappedRange(buffer, (size_t)offset, out, size)
          == WGPUStatus_Success);
    wgpuBufferUnmap(buffer);
    return 0;
}

WGPUShaderModule e2e_make_shader(WGPUDevice device, const char *wgsl,
                                 const char *label)
{
    WGPUShaderSourceWGSL source = {0};
    source.chain.sType = WGPUSType_ShaderSourceWGSL;
    source.code = e2e_sv(wgsl);
    WGPUShaderModuleDescriptor descriptor = {0};
    descriptor.nextInChain = &source.chain;
    descriptor.label = e2e_sv(label);
    return wgpuDeviceCreateShaderModule(device, &descriptor);
}
