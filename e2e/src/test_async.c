/* The asynchronous round-trips: error scopes (both clean and catching a
 * real validation error), onSubmittedWorkDone and getCompilationInfo. */

#include "test_util.h"

static void on_scope_result(WGPUPopErrorScopeStatus status, WGPUErrorType type,
                            WGPUStringView message, void *userdata1, void *userdata2)
{
    (void)userdata2;
    if (status != WGPUPopErrorScopeStatus_Success)
        fprintf(stderr, "e2e: popErrorScope failed\n");
    if (type != WGPUErrorType_NoError)
        fprintf(stderr, "e2e: error scope caught (type %d): %.*s\n",
                (int)type, (int)message.length, message.data ? message.data : "");
    *(int *)userdata1 = (int)type;
}

static void on_work_done(WGPUQueueWorkDoneStatus status, WGPUStringView message,
                         void *userdata1, void *userdata2)
{
    (void)message; (void)userdata2;
    *(int *)userdata1 = status == WGPUQueueWorkDoneStatus_Success ? 1 : -1;
}

static void on_compilation_info(WGPUCompilationInfoRequestStatus status,
                                WGPUCompilationInfo const *info,
                                void *userdata1, void *userdata2)
{
    (void)userdata2;
    if (status != WGPUCompilationInfoRequestStatus_Success) {
        *(int *)userdata1 = -1;
        return;
    }
    for (size_t i = 0; i < info->messageCount; ++i)
        fprintf(stderr, "e2e: compilation message (type %d): %.*s\n",
                (int)info->messages[i].type, (int)info->messages[i].message.length,
                info->messages[i].message.data ? info->messages[i].message.data : "");
    *(int *)userdata1 = 1;
}

int run_test(GpuContext *ctx)
{
    WGPUDevice device = ctx->device;

    /* A clean scope: valid work inside, NoError out. */
    wgpuDevicePushErrorScope(device, WGPUErrorFilter_Validation);
    WGPUBufferDescriptor good_desc = {0};
    good_desc.label = e2e_sv("valid buffer");
    good_desc.usage = WGPUBufferUsage_CopyDst;
    good_desc.size = 64;
    WGPUBuffer good = wgpuDeviceCreateBuffer(device, &good_desc);
    int clean = 0;
    WGPUPopErrorScopeCallbackInfo scope_cb = {0};
    scope_cb.callback = on_scope_result;
    scope_cb.userdata1 = &clean;
    wgpuDevicePopErrorScope(device, scope_cb);
    CHECK(e2e_wait_flag(ctx, &clean) == 0);
    CHECK(clean == (int)WGPUErrorType_NoError);
    wgpuBufferRelease(good);

    /* A dirty scope: an impossible buffer must surface as a validation
     * error inside the scope (and never as an uncaptured error). */
    WGPULimits limits = {0};
    CHECK(wgpuAdapterGetLimits(ctx->adapter, &limits) == WGPUStatus_Success);
    wgpuDevicePushErrorScope(device, WGPUErrorFilter_Validation);
    WGPUBufferDescriptor bad_desc = {0};
    bad_desc.label = e2e_sv("too large");
    bad_desc.usage = WGPUBufferUsage_CopyDst;
    bad_desc.size = limits.maxBufferSize * 2;
    WGPUBuffer bad = wgpuDeviceCreateBuffer(device, &bad_desc);
    int caught = 0;
    scope_cb.userdata1 = &caught;
    wgpuDevicePopErrorScope(device, scope_cb);
    CHECK(e2e_wait_flag(ctx, &caught) == 0);
    CHECK(caught == (int)WGPUErrorType_Validation);
    if (bad)
        wgpuBufferRelease(bad);

    /* Work-done fires after an (empty) submission. */
    wgpuQueueSubmit(ctx->queue, 0, NULL);
    int done = 0;
    WGPUQueueWorkDoneCallbackInfo done_cb = {0};
    done_cb.callback = on_work_done;
    done_cb.userdata1 = &done;
    wgpuQueueOnSubmittedWorkDone(ctx->queue, done_cb);
    CHECK(e2e_wait_flag(ctx, &done) == 0);
    CHECK(done == 1);

    /* Compilation info round-trips for a valid module. */
    WGPUShaderModule module = e2e_make_shader(
        device, "@compute @workgroup_size(1) fn main() {}\n", "trivial");
    CHECK(module);
    int info = 0;
    WGPUCompilationInfoCallbackInfo info_cb = {0};
    info_cb.callback = on_compilation_info;
    info_cb.userdata1 = &info;
    wgpuShaderModuleGetCompilationInfo(module, info_cb);
    CHECK(e2e_wait_flag(ctx, &info) == 0);
    CHECK(info == 1);
    wgpuShaderModuleRelease(module);

    return 0;
}
