#ifndef E2E_TEST_UTIL_H
#define E2E_TEST_UTIL_H

/*
 * Shared scaffolding for the per-feature e2e tests.  Each test is one
 * executable: a file defining run_test(), linked with test_main.c (which
 * owns the websocket accept / device bring-up) and this helper code.
 */

#include <stdio.h>
#include <string.h>

#include <webgpu/webgpu.h>

#include "gpu_context.h"

#define CHECK(cond)                                                          \
    do {                                                                     \
        if (!(cond)) {                                                       \
            fprintf(stderr, "e2e: FAILED at %s:%d: %s\n", __FILE__,          \
                    __LINE__, #cond);                                        \
            return -1;                                                       \
        }                                                                    \
    } while (0)

/* Implemented by each test file; 0 = pass. */
int run_test(GpuContext *ctx);

WGPUStringView e2e_sv(const char *s);

/* Pump the connection until *flag becomes non-zero; -1 on disconnect. */
int e2e_wait_flag(GpuContext *ctx, const int *flag);

/* Map `buffer` for reading and copy `size` bytes at `offset` into `out`;
 * 0 on success. */
int e2e_read_back(GpuContext *ctx, WGPUBuffer buffer, uint64_t offset,
                  size_t size, void *out);

/* Create a WGSL shader module (NULL on failure). */
WGPUShaderModule e2e_make_shader(WGPUDevice device, const char *wgsl,
                                 const char *label);

#endif /* E2E_TEST_UTIL_H */
