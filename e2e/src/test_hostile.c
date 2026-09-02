/*
 * The client is untrusted.  This test replaces the browser with
 * tools/hostile_client.py, a peer that completes the handshake with absurd
 * values and then sends every malformed, saturated, repeated and
 * unsolicited message the protocol allows.  The library must clamp what it
 * reports to the application, reject replies it cannot honour, and survive
 * the rest; the test passes if the server is still standing (and still
 * telling the truth) when the peer hangs up.
 *
 * Not covered here, because it cannot share a connection with a working
 * handshake: garbage sent *before* the ClientHello, which is expected to
 * fail the bring-up rather than be tolerated.
 */

#include "test_util.h"

#include <webgpu/remote.h>

/* Sizes the library promises never to pass through unclamped. */
#define MAX_CANVAS_DIM 16384u
#define MAX_TEXTURE_DIM 65536u
#define MAX_FEATURES 1024u

static int is_power_of_two(uint32_t value)
{
    return value != 0 && (value & (value - 1)) == 0;
}

static int map_done;
static WGPUMapAsyncStatus map_status;

static void on_map(WGPUMapAsyncStatus status, WGPUStringView message,
                   void *userdata1, void *userdata2)
{
    (void)message; (void)userdata1; (void)userdata2;
    map_status = status;
    map_done = 1;
}

int run_test(GpuContext *ctx)
{
    /* The hello reported a 4294967295 x 4294967295 canvas. */
    CHECK(ctx->width >= 1 && ctx->width <= MAX_CANVAS_DIM);
    CHECK(ctx->height >= 1 && ctx->height <= MAX_CANVAS_DIM);

    /* ... zeroed counts, a zero and a non-power-of-two alignment, and
     * saturated 64-bit sizes.  Anything the application divides by, sizes
     * an allocation from or compares a request against must be sane. */
    WGPULimits limits = {0};
    CHECK(wgpuAdapterGetLimits(ctx->adapter, &limits) == WGPUStatus_Success);
    CHECK(is_power_of_two(limits.minUniformBufferOffsetAlignment));
    CHECK(is_power_of_two(limits.minStorageBufferOffsetAlignment));
    CHECK(limits.minUniformBufferOffsetAlignment <= 256);
    CHECK(limits.minStorageBufferOffsetAlignment <= 256);
    CHECK(limits.maxBindGroups >= 4);
    CHECK(limits.maxTextureDimension2D >= 8192);
    CHECK(limits.maxTextureDimension2D <= MAX_TEXTURE_DIM);
    CHECK(limits.maxBufferSize >= 268435456u);
    CHECK(limits.maxBufferSize <= (1ull << 34));
    CHECK(limits.maxUniformBufferBindingSize <= (1ull << 32));
    CHECK(limits.maxStorageBufferBindingSize <= (1ull << 34));

    /* The hello also claimed 50000 features. */
    WGPUSupportedFeatures features = {0};
    wgpuAdapterGetFeatures(ctx->adapter, &features);
    CHECK(features.featureCount <= MAX_FEATURES);
    wgpuSupportedFeaturesFreeMembers(features);

    /*
     * Map a buffer.  The peer answers every map request with fewer bytes
     * than were asked for; a short reply must fail the map rather than
     * leave a mapping shorter than the range the application will ask for
     * (a NULL from GetMappedRange is a crash in most bindings).
     */
    WGPUBufferDescriptor desc = {0};
    desc.size = 256;
    desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    WGPUBuffer buffer = wgpuDeviceCreateBuffer(ctx->device, &desc);
    CHECK(buffer != NULL);

    WGPUBufferMapCallbackInfo callback = {0};
    callback.mode = WGPUCallbackMode_AllowProcessEvents;
    callback.callback = on_map;
    wgpuBufferMapAsync(buffer, WGPUMapMode_Read, 0, desc.size, callback);
    CHECK(e2e_wait_flag(ctx, &map_done) == 0);
    CHECK(map_status != WGPUMapAsyncStatus_Success);
    CHECK(wgpuBufferGetMapState(buffer) == WGPUBufferMapState_Unmapped);
    CHECK(wgpuBufferGetConstMappedRange(buffer, 0, desc.size) == NULL);
    wgpuBufferRelease(buffer);

    /* Eat the rest of the abuse until the peer hangs up. */
    while (ctx->pump(ctx->pump_userdata) == 0)
        ;

    /* The resize storm must not have moved the canvas out of range. */
    uint32_t width = 0, height = 0;
    wgpuRemoteAdapterGetCanvasSize(ctx->adapter, &width, &height);
    CHECK(width <= MAX_CANVAS_DIM && height <= MAX_CANVAS_DIM);
    return 0;
}
