/*
 * Entry point shared by every per-feature test executable: wait for the
 * web client on --port, bring the remote device up, run the test file's
 * run_test() and report PASS/FAIL through the exit code.
 */

#include <stdlib.h>

#include "connection.h"
#include "gpu_setup.h"
#include "test_util.h"

int main(int argc, char **argv)
{
    int port = 8080;
    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "--port") && i + 1 < argc)
            port = atoi(argv[++i]);
    }

    Connection conn;
    if (connection_accept(&conn, (uint16_t)port) != 0)
        return 1;

    GpuContext ctx;
    if (gpu_setup(&conn, 640, 480, &ctx) != 0) {
        connection_close(&conn);
        return 1;
    }

    int rc = run_test(&ctx) == 0 ? 0 : 1;
    fprintf(stderr, "e2e: %s\n", rc == 0 ? "PASS" : "FAIL");

    gpu_teardown(&ctx);
    connection_close(&conn);
    return rc;
}
