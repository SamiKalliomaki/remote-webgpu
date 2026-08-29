#ifndef WEBGPU_WGPU_COMPAT_H
#define WEBGPU_WGPU_COMPAT_H

/*
 * Minimal compatibility shim for code written against wgpu-native's
 * <webgpu/wgpu.h>.  Only the extensions the example server actually uses are
 * declared here.
 */

#include <webgpu/webgpu.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint64_t WGPUSubmissionIndex;

WGPU_EXPORT WGPUBool wgpuDevicePoll(WGPUDevice device, WGPUBool wait,
                                    WGPUSubmissionIndex const *wrappedSubmissionIndex);

#ifdef __cplusplus
}
#endif

#endif /* WEBGPU_WGPU_COMPAT_H */
