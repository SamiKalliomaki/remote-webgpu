/*
 * Fuzz target for the untrusted half of the protocol.
 *
 * Everything a client sends reaches the library through exactly one
 * function -- wgpuRemoteAdapterReceiveData() -- which makes it a natural
 * fuzz entry point: one input is one hostile client session.
 *
 * Each run builds a complete application-side session (instance, surface,
 * adapter, device, queue) and leaves one request of every asynchronous kind
 * outstanding -- a buffer map, a work-done, an error scope, compilation
 * info, a texture load, a presented frame awaiting its vsync ack -- so the
 * generated messages have live state to complete, corrupt or contradict.
 * Then the input is framed into envelopes and fed in.  The run passes if
 * the library did not crash, and if the invariants in
 * ../src/client_invariants.h plus the mapping rule below still hold
 * afterwards.
 *
 * The entry point is libFuzzer's, so `clang -fsanitize=fuzzer,address` or
 * AFL++ can drive this file directly; fuzz_main.c is the built-in driver
 * used when neither is available.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <webgpu/remote.h>
#include <webgpu/webgpu.h>

#include "client_invariants.h"
#include "proto_writer.h"

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size);

/* The size of the buffer mapping the session leaves outstanding. */
#define MAP_SIZE 256u

static WGPUStringView sv(const char *s)
{
    WGPUStringView view = { s, s ? strlen(s) : 0 };
    return view;
}

/* Envelope{ client_hello: ClientHello{...} } with plausible capabilities. */
static size_t build_hello(uint8_t *out, size_t capacity)
{
    uint8_t limits[256];
    size_t l = 0;
    l += pw_field(limits + l, 1, 8192);        /* max_texture_dimension_1d */
    l += pw_field(limits + l, 2, 8192);        /* max_texture_dimension_2d */
    l += pw_field(limits + l, 3, 2048);
    l += pw_field(limits + l, 4, 256);
    l += pw_field(limits + l, 5, 4);           /* max_bind_groups */
    l += pw_field(limits + l, 15, 65536);      /* max_uniform_buffer_binding_size */
    l += pw_field(limits + l, 16, 134217728);  /* max_storage_buffer_binding_size */
    l += pw_field(limits + l, 17, 256);        /* min_uniform_buffer_offset_alignment */
    l += pw_field(limits + l, 18, 256);        /* min_storage_buffer_offset_alignment */
    l += pw_field(limits + l, 20, 268435456);  /* max_buffer_size */
    l += pw_field(limits + l, 28, 256);         /* max_compute_workgroup_size_x */
    l += pw_field(limits + l, 29, 256);
    l += pw_field(limits + l, 30, 64);
    l += pw_field(limits + l, 31, 65535);       /* max_compute_workgroups_per_dimension */

    uint8_t info[64];
    size_t i = 0;
    i += pw_bytes(info + i, 1, (const uint8_t *)"fuzz", 4);
    i += pw_bytes(info + i, 3, (const uint8_t *)"fuzz", 4);

    uint8_t hello[512];
    size_t h = 0;
    h += pw_field(hello + h, 1, 6);            /* protocol_version */
    h += pw_bytes(hello + h, 2, info, i);      /* adapter */
    h += pw_field(hello + h, 3, 640);          /* canvas_width */
    h += pw_field(hello + h, 4, 480);          /* canvas_height */
    h += pw_bytes(hello + h, 5, limits, l);    /* limits */

    if (capacity < h + 16)
        abort();
    return pw_bytes(out, 2, hello, h);         /* Envelope.client_hello */
}

/* ------------------------------------------------------------------ */
/* the application side of one session                                */
/* ------------------------------------------------------------------ */

typedef struct {
    WGPUInstance instance;
    WGPUSurface surface;
    WGPUAdapter adapter;
    WGPUDevice device;
    WGPUQueue queue;
    WGPUBuffer buffer;
    WGPUShaderModule shader;
    /* Set from the callbacks the fuzzed input can complete. */
    int map_done;
    WGPUMapAsyncStatus map_status;
    int handshake_done;
} Session;

/* Outgoing protocol messages go nowhere: the client is the fuzzer. */
static void discard_send(void const *data, size_t size, void *userdata)
{
    (void)data; (void)size; (void)userdata;
}

static void on_map(WGPUMapAsyncStatus status, WGPUStringView message,
                   void *ud1, void *ud2)
{
    Session *session = (Session *)ud1;
    (void)message; (void)ud2;
    session->map_status = status;
    session->map_done = 1;
}

static void on_work_done(WGPUQueueWorkDoneStatus status, WGPUStringView message,
                         void *ud1, void *ud2)
{
    (void)status; (void)message; (void)ud1; (void)ud2;
}

static void on_error_scope(WGPUPopErrorScopeStatus status, WGPUErrorType type,
                           WGPUStringView message, void *ud1, void *ud2)
{
    (void)status; (void)type; (void)message; (void)ud1; (void)ud2;
}

static void on_compilation(WGPUCompilationInfoRequestStatus status,
                           WGPUCompilationInfo const *info, void *ud1, void *ud2)
{
    (void)ud1; (void)ud2;
    /* Touch every message the client claimed: a bad length or a NULL
     * string would show up here (and under a sanitizer, loudly). */
    if (status != WGPUCompilationInfoRequestStatus_Success || !info)
        return;
    volatile size_t sink = 0;
    for (size_t i = 0; i < info->messageCount; ++i)
        sink += info->messages[i].message.length;
    (void)sink;
}

static void on_texture_loaded(WGPUStatus status, WGPUTexture texture,
                              WGPUStringView message, void *ud1, void *ud2)
{
    (void)message; (void)ud1; (void)ud2;
    if (status == WGPUStatus_Success && texture) {
        /* The dimensions came from the client; they must be clamped. */
        if (wgpuTextureGetWidth(texture) > E2E_MAX_TEXTURE_DIM
            || wgpuTextureGetHeight(texture) > E2E_MAX_TEXTURE_DIM) {
            fprintf(stderr, "fuzz: texture dimensions escaped the clamp\n");
            abort();
        }
        wgpuTextureRelease(texture);
    }
}

static void on_device(WGPURequestDeviceStatus status, WGPUDevice device,
                      WGPUStringView message, void *ud1, void *ud2)
{
    (void)message; (void)ud2;
    if (status == WGPURequestDeviceStatus_Success)
        *(WGPUDevice *)ud1 = device;
}

static void on_device_lost(WGPUDevice const *device, WGPUDeviceLostReason reason,
                           WGPUStringView message, void *ud1, void *ud2)
{
    (void)device; (void)reason; (void)message; (void)ud1; (void)ud2;
}

static void on_uncaptured_error(WGPUDevice const *device, WGPUErrorType type,
                                WGPUStringView message, void *ud1, void *ud2)
{
    (void)device; (void)type; (void)ud1; (void)ud2;
    /* A client-supplied string: read all of it. */
    volatile size_t sink = 0;
    for (size_t i = 0; i < message.length; ++i)
        sink += (unsigned char)message.data[i];
    (void)sink;
}

static void on_event(const WGPURemoteEvent *event, void *ud1, void *ud2)
{
    (void)ud1; (void)ud2;
    if (!event)
        return;
    /* Same for event names and payloads. */
    volatile size_t sink = 0;
    for (const char *p = event->name; p && *p; ++p)
        sink += (unsigned char)*p;
    for (size_t i = 0; i < event->payload_size; ++i)
        sink += ((const unsigned char *)event->payload)[i];
    (void)sink;
}

static void session_teardown(Session *session)
{
    /* The session ends the way a real one does when the client vanishes:
     * requests the client never answered are abandoned, which fires their
     * callbacks and drops the references they hold.  Without this the
     * pending map alone keeps buffer -> device -> adapter -> request alive
     * as a cycle, and every fuzz input leaks a whole session. */
    if (session->adapter)  wgpuRemoteAdapterAbandonRequests(session->adapter);
    if (session->shader)   wgpuShaderModuleRelease(session->shader);
    if (session->buffer)   wgpuBufferRelease(session->buffer);
    if (session->queue)    wgpuQueueRelease(session->queue);
    if (session->device)   wgpuDeviceRelease(session->device);
    if (session->adapter)  wgpuAdapterRelease(session->adapter);
    if (session->surface)  wgpuSurfaceRelease(session->surface);
    if (session->instance) wgpuInstanceRelease(session->instance);
    memset(session, 0, sizeof *session);
}

/* Bring the session up and leave one request of every kind outstanding. */
static void session_setup(Session *session, int with_handshake)
{
    memset(session, 0, sizeof *session);
    session->instance = wgpuCreateInstance(NULL);

    WGPUSurfaceDescriptor surface_desc = {0};
    surface_desc.label = sv("fuzz canvas");
    session->surface = wgpuInstanceCreateSurface(session->instance, &surface_desc);

    session->adapter = wgpuRemoteInstanceCreateAdapter(session->instance,
                                                       discard_send, NULL);
    WGPURemoteEventCallbackInfo events = {0};
    events.callback = on_event;
    wgpuRemoteAdapterSetEventCallback(session->adapter, events);

    if (!with_handshake)
        return;

    uint8_t hello[1024];
    size_t hello_len = build_hello(hello, sizeof hello);
    wgpuRemoteAdapterReceiveData(session->adapter, hello, hello_len);
    if (!wgpuRemoteAdapterIsReady(session->adapter)) {
        fprintf(stderr, "fuzz: the canonical ClientHello was rejected\n");
        abort();
    }
    session->handshake_done = 1;

    WGPUDeviceDescriptor device_desc = {0};
    device_desc.label = sv("fuzz device");
    device_desc.deviceLostCallbackInfo.mode = WGPUCallbackMode_AllowProcessEvents;
    device_desc.deviceLostCallbackInfo.callback = on_device_lost;
    device_desc.uncapturedErrorCallbackInfo.callback = on_uncaptured_error;
    WGPURequestDeviceCallbackInfo device_cb = {0};
    device_cb.mode = WGPUCallbackMode_AllowProcessEvents;
    device_cb.callback = on_device;
    device_cb.userdata1 = &session->device;
    wgpuAdapterRequestDevice(session->adapter, &device_desc, device_cb);
    if (!session->device) {
        fprintf(stderr, "fuzz: the device request did not resolve\n");
        abort();
    }
    session->queue = wgpuDeviceGetQueue(session->device);

    /* A configured surface, a presented frame and a vsync wait, so
     * PresentDone has something to acknowledge. */
    WGPUSurfaceConfiguration config = {0};
    config.device = session->device;
    config.format = WGPUTextureFormat_BGRA8Unorm;
    config.usage = WGPUTextureUsage_RenderAttachment;
    config.width = 640;
    config.height = 480;
    config.presentMode = WGPUPresentMode_Fifo;
    config.alphaMode = WGPUCompositeAlphaMode_Auto;
    wgpuSurfaceConfigure(session->surface, &config);
    WGPUSurfaceTexture frame = {0};
    wgpuSurfaceGetCurrentTexture(session->surface, &frame);
    if (frame.texture)
        wgpuTextureRelease(frame.texture);
    wgpuSurfacePresent(session->surface);
    WGPURemoteVsyncCallbackInfo vsync = {0};
    wgpuRemoteSurfaceOnNextVsync(session->surface, vsync);

    /* An outstanding map, whose reply the input gets to forge. */
    WGPUBufferDescriptor buffer_desc = {0};
    buffer_desc.size = MAP_SIZE;
    buffer_desc.usage = WGPUBufferUsage_MapRead | WGPUBufferUsage_CopyDst;
    session->buffer = wgpuDeviceCreateBuffer(session->device, &buffer_desc);
    WGPUBufferMapCallbackInfo map_cb = {0};
    map_cb.mode = WGPUCallbackMode_AllowProcessEvents;
    map_cb.callback = on_map;
    map_cb.userdata1 = session;
    wgpuBufferMapAsync(session->buffer, WGPUMapMode_Read, 0, MAP_SIZE, map_cb);

    /* ... and one of every other asynchronous kind. */
    WGPUQueueWorkDoneCallbackInfo work_cb = {0};
    work_cb.mode = WGPUCallbackMode_AllowProcessEvents;
    work_cb.callback = on_work_done;
    wgpuQueueOnSubmittedWorkDone(session->queue, work_cb);

    wgpuDevicePushErrorScope(session->device, WGPUErrorFilter_Validation);
    WGPUPopErrorScopeCallbackInfo scope_cb = {0};
    scope_cb.mode = WGPUCallbackMode_AllowProcessEvents;
    scope_cb.callback = on_error_scope;
    wgpuDevicePopErrorScope(session->device, scope_cb);

    WGPUShaderSourceWGSL wgsl = {0};
    wgsl.chain.sType = WGPUSType_ShaderSourceWGSL;
    wgsl.code = sv("@compute @workgroup_size(1) fn main() {}");
    WGPUShaderModuleDescriptor shader_desc = {0};
    shader_desc.nextInChain = &wgsl.chain;
    session->shader = wgpuDeviceCreateShaderModule(session->device, &shader_desc);
    WGPUCompilationInfoCallbackInfo compilation_cb = {0};
    compilation_cb.mode = WGPUCallbackMode_AllowProcessEvents;
    compilation_cb.callback = on_compilation;
    wgpuShaderModuleGetCompilationInfo(session->shader, compilation_cb);

    WGPURemoteTextureLoadCallbackInfo texture_cb = {0};
    texture_cb.callback = on_texture_loaded;
    wgpuRemoteDeviceLoadTextureFromURL(session->device, sv("fuzz://image.png"),
                                       WGPUTextureUsage_TextureBinding, texture_cb);
}

/*
 * The one rule the mapping has to obey: a buffer the library reports as
 * mapped must hand out the whole range that was asked for.  A client
 * answering a 256-byte map with fewer bytes has to fail the map instead --
 * bindings that assume a non-NULL pointer here (the Rust shim asserts)
 * would otherwise take the process down.
 */
static void check_mapping(Session *session)
{
    if (!session->buffer || !session->map_done)
        return;
    if (session->map_status != WGPUMapAsyncStatus_Success) {
        if (wgpuBufferGetMapState(session->buffer) == WGPUBufferMapState_Mapped) {
            fprintf(stderr, "fuzz: failed map left the buffer mapped\n");
            abort();
        }
        return;
    }
    if (wgpuBufferGetMapState(session->buffer) != WGPUBufferMapState_Mapped) {
        fprintf(stderr, "fuzz: successful map left the buffer unmapped\n");
        abort();
    }
    const void *range = wgpuBufferGetConstMappedRange(session->buffer, 0, MAP_SIZE);
    if (!range) {
        fprintf(stderr, "fuzz: mapped buffer refuses the range that was mapped\n");
        abort();
    }
    /* Read it all: a short mapping shows up as a heap overflow here. */
    volatile size_t sink = 0;
    for (size_t i = 0; i < MAP_SIZE; ++i)
        sink += ((const unsigned char *)range)[i];
    (void)sink;
    wgpuBufferUnmap(session->buffer);
}

/* ------------------------------------------------------------------ */
/* entry point                                                        */
/* ------------------------------------------------------------------ */

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size)
{
    /*
     * The first byte picks the shape of the session, so one corpus covers
     * both sides of the handshake: bit 0 fuzzes an established session
     * (the interesting one), bit 1 fuzzes the bring-up itself, where the
     * input has to be a ClientHello -- or be rejected without damage.
     */
    int with_handshake = size == 0 || (data[0] & 1) == 0;
    if (size) {
        ++data;
        --size;
    }

    Session session;
    session_setup(&session, with_handshake);

    /*
     * Frame the rest the way the transport does: u32 little-endian sizes,
     * one envelope each.  A prefix that does not fit is the end of the
     * stream -- except that a stream which yields nothing at all is fed
     * whole, so that unframed inputs (a raw protobuf message, a file from
     * some other corpus) still reach the parser.
     */
    size_t pos = 0, fed = 0;
    while (pos + 4 <= size) {
        uint32_t length = (uint32_t)data[pos] | ((uint32_t)data[pos + 1] << 8)
                        | ((uint32_t)data[pos + 2] << 16) | ((uint32_t)data[pos + 3] << 24);
        pos += 4;
        if (length > size - pos)
            break;
        wgpuRemoteAdapterReceiveData(session.adapter, data + pos, length);
        pos += length;
        ++fed;
        /*
         * Sticky, and checked after every envelope: an input that builds
         * its own ClientHello has to have its capabilities checked too,
         * and a later envelope may knock the adapter out of "ready" again
         * (a protocol error does) without un-reporting those limits.
         */
        session.handshake_done |= wgpuRemoteAdapterIsReady(session.adapter) != 0;
    }
    if (fed == 0 && size) {
        wgpuRemoteAdapterReceiveData(session.adapter, data, size);
        session.handshake_done |= wgpuRemoteAdapterIsReady(session.adapter) != 0;
    }

    const char *violation = client_invariants_check(session.adapter,
                                                    session.handshake_done);
    if (violation) {
        fprintf(stderr, "fuzz: invariant violated: %s\n", violation);
        abort();
    }
    check_mapping(&session);

    session_teardown(&session);
    return 0;
}
