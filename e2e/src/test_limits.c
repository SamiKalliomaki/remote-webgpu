/* Adapter limits and features travel in the ClientHello and are answered
 * locally by the library. */

#include "test_util.h"

int run_test(GpuContext *ctx)
{
    WGPULimits limits = {0};
    CHECK(wgpuAdapterGetLimits(ctx->adapter, &limits) == WGPUStatus_Success);
    CHECK(limits.maxTextureDimension2D >= 2048);
    CHECK(limits.maxBufferSize >= 1u << 20);
    CHECK(limits.maxBindGroups >= 4);
    CHECK(limits.minUniformBufferOffsetAlignment > 0);

    /* The device answers with the same data. */
    WGPULimits device_limits = {0};
    CHECK(wgpuDeviceGetLimits(ctx->device, &device_limits) == WGPUStatus_Success);
    CHECK(device_limits.maxTextureDimension2D == limits.maxTextureDimension2D);
    CHECK(device_limits.maxBufferSize == limits.maxBufferSize);

    WGPUSupportedFeatures features = {0};
    wgpuAdapterGetFeatures(ctx->adapter, &features);
    for (size_t i = 0; i < features.featureCount; ++i)
        CHECK(wgpuAdapterHasFeature(ctx->adapter, features.features[i]));
    fprintf(stderr, "e2e: limits (maxTextureDimension2D=%u) and %zu features "
                    "received\n",
            limits.maxTextureDimension2D, features.featureCount);
    wgpuSupportedFeaturesFreeMembers(features);
    return 0;
}
