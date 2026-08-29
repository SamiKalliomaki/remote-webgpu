/* Textures from URLs (a remote.h extension): the client fetches and
 * decodes web/test-image.png into a texture, verified by reading the
 * pixels back; a missing URL must fail cleanly through the callback. */

#include <webgpu/remote.h>

#include "test_util.h"

typedef struct {
    int done; /* 1 = success, -1 = failure */
    WGPUTexture texture;
    char message[256];
} LoadResult;

static void on_loaded(WGPUStatus status, WGPUTexture texture,
                      WGPUStringView message, void *userdata1, void *userdata2)
{
    LoadResult *result = userdata1;
    (void)userdata2;
    result->texture = texture;
    if (message.data && message.length)
        snprintf(result->message, sizeof result->message, "%.*s",
                 (int)message.length, message.data);
    result->done = status == WGPUStatus_Success ? 1 : -1;
}

int run_test(GpuContext *ctx)
{
    WGPUDevice device = ctx->device;

    /* The URL is relative to the client's page, which run.sh serves from
     * web/ -- the same directory holding test-image.png (2x2: red, green /
     * blue, white). */
    LoadResult load = {0};
    WGPURemoteTextureLoadCallbackInfo load_cb = {0};
    load_cb.callback = on_loaded;
    load_cb.userdata1 = &load;
    wgpuRemoteDeviceLoadTextureFromURL(device, e2e_sv("test-image.png"),
                                       WGPUTextureUsage_TextureBinding
                                           | WGPUTextureUsage_CopySrc,
                                       load_cb);
    CHECK(e2e_wait_flag(ctx, &load.done) == 0);
    CHECK(load.done == 1);
    WGPUTexture texture = load.texture;
    CHECK(texture);
    CHECK(wgpuTextureGetWidth(texture) == 2);
    CHECK(wgpuTextureGetHeight(texture) == 2);
    CHECK(wgpuTextureGetFormat(texture) == WGPUTextureFormat_RGBA8Unorm);
    CHECK(wgpuTextureGetUsage(texture) & WGPUTextureUsage_CopySrc);

    /* Read the texel rows back (bytesPerRow must be 256-aligned). */
    WGPUBufferDescriptor read_desc = {0};
    read_desc.label = e2e_sv("image readback");
    read_desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    read_desc.size = 2 * 256;
    WGPUBuffer readback = wgpuDeviceCreateBuffer(device, &read_desc);

    WGPUCommandEncoder encoder = wgpuDeviceCreateCommandEncoder(device, NULL);
    WGPUTexelCopyTextureInfo src = {0};
    src.texture = texture;
    WGPUTexelCopyBufferInfo dst = {0};
    dst.buffer = readback;
    dst.layout.bytesPerRow = 256;
    dst.layout.rowsPerImage = 2;
    WGPUExtent3D size = { 2, 2, 1 };
    wgpuCommandEncoderCopyTextureToBuffer(encoder, &src, &dst, &size);
    WGPUCommandBuffer commands = wgpuCommandEncoderFinish(encoder, NULL);
    wgpuQueueSubmit(ctx->queue, 1, &commands);
    wgpuCommandBufferRelease(commands);
    wgpuCommandEncoderRelease(encoder);

    uint8_t pixels[2 * 256];
    CHECK(e2e_read_back(ctx, readback, 0, sizeof pixels, pixels) == 0);
    static const uint8_t expected[2][8] = {
        { 255, 0, 0, 255, 0, 255, 0, 255 },     /* red,  green */
        { 0, 0, 255, 255, 255, 255, 255, 255 }, /* blue, white */
    };
    for (int row = 0; row < 2; ++row) {
        const uint8_t *p = pixels + row * 256;
        if (memcmp(p, expected[row], 8) != 0) {
            fprintf(stderr, "e2e: image row %d = %u,%u,%u,%u / %u,%u,%u,%u\n",
                    row, p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
            return -1;
        }
    }
    wgpuBufferRelease(readback);
    wgpuTextureRelease(texture);

    /* A URL that does not exist must fail through the callback. */
    LoadResult missing = {0};
    load_cb.userdata1 = &missing;
    wgpuRemoteDeviceLoadTextureFromURL(device, e2e_sv("no-such-image.png"),
                                       WGPUTextureUsage_TextureBinding, load_cb);
    CHECK(e2e_wait_flag(ctx, &missing.done) == 0);
    CHECK(missing.done == -1);
    CHECK(missing.texture == NULL);
    fprintf(stderr, "e2e: missing image failed as expected: %s\n",
            missing.message);
    return 0;
}
