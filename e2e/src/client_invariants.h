#ifndef E2E_CLIENT_INVARIANTS_H
#define E2E_CLIENT_INVARIANTS_H

/*
 * What the library promises to the application no matter what the client
 * says.  The client is untrusted: everything it reports about itself
 * (canvas size, limits, features) is clamped into a range the application
 * can safely divide by, allocate from and compare against before any of it
 * is answered from a getter.
 *
 * Shared by the two tests that abuse a client: test_hostile.c (a scripted
 * peer) and fuzz/fuzz_target.c (a generated one).  Returns NULL when
 * everything holds, or a message naming the first violation.
 */

#include <stdint.h>

#include <webgpu/remote.h>
#include <webgpu/webgpu.h>

/* Mirrors the RW_MAX_* clamps in remote_webgpu/src/remote_webgpu.c. */
#define E2E_MAX_CANVAS_DIM 16384u
#define E2E_MAX_TEXTURE_DIM 65536u
#define E2E_MAX_FEATURES 1024u

static int e2e_is_power_of_two(uint32_t value)
{
    return value != 0 && (value & (value - 1)) == 0;
}

/*
 * `handshake_done` says whether a ClientHello was ever accepted: before
 * that the adapter reports zeroed capabilities, which is legitimate.
 */
static const char *client_invariants_check(WGPUAdapter adapter, int handshake_done)
{
    uint32_t width = 0, height = 0;
    wgpuRemoteAdapterGetCanvasSize(adapter, &width, &height);
    if (width > E2E_MAX_CANVAS_DIM || height > E2E_MAX_CANVAS_DIM)
        return "canvas size exceeds the clamp";

    if (!handshake_done)
        return NULL;

    WGPULimits limits = {0};
    if (wgpuAdapterGetLimits(adapter, &limits) != WGPUStatus_Success)
        return "wgpuAdapterGetLimits failed";
    if (!e2e_is_power_of_two(limits.minUniformBufferOffsetAlignment)
        || limits.minUniformBufferOffsetAlignment > 256)
        return "minUniformBufferOffsetAlignment is not a power of two in [1, 256]";
    if (!e2e_is_power_of_two(limits.minStorageBufferOffsetAlignment)
        || limits.minStorageBufferOffsetAlignment > 256)
        return "minStorageBufferOffsetAlignment is not a power of two in [1, 256]";
    if (limits.maxBindGroups < 4)
        return "maxBindGroups below the mandated minimum";
    if (limits.maxTextureDimension2D < 8192
        || limits.maxTextureDimension2D > E2E_MAX_TEXTURE_DIM)
        return "maxTextureDimension2D out of range";
    if (limits.maxBufferSize < 268435456u || limits.maxBufferSize > (1ull << 34))
        return "maxBufferSize out of range";
    if (limits.maxUniformBufferBindingSize > (1ull << 32))
        return "maxUniformBufferBindingSize out of range";
    if (limits.maxStorageBufferBindingSize > (1ull << 34))
        return "maxStorageBufferBindingSize out of range";
    if (limits.maxComputeWorkgroupsPerDimension < 65535u)
        return "maxComputeWorkgroupsPerDimension below the mandated minimum";

    WGPUSupportedFeatures features = {0};
    wgpuAdapterGetFeatures(adapter, &features);
    size_t feature_count = features.featureCount;
    wgpuSupportedFeaturesFreeMembers(features);
    if (feature_count > E2E_MAX_FEATURES)
        return "feature count exceeds the clamp";

    return NULL;
}

#endif /* E2E_CLIENT_INVARIANTS_H */
