#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "connection.h"
#include "gpu_setup.h"
#include "render.h"

static void usage(const char *argv0)
{
    fprintf(stderr,
            "usage: %s [--port N] [--width N] [--height N] [--frames N] [--screenshot FILE]\n"
            "\n"
            "  --port N          websocket port to wait for the remote GPU on (default 8080)\n"
            "  --width/--height  fallback frame size if the client reports no canvas size\n"
            "  --frames N        exit after N frames instead of running until disconnect\n"
            "  --screenshot FILE write the final frame as a binary PPM\n",
            argv0);
}

int main(int argc, char **argv)
{
    int width = 800;
    int height = 600;
    int port = 8080;
    RenderOptions options = {0};

    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "--port") && i + 1 < argc)
            port = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--width") && i + 1 < argc)
            width = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--height") && i + 1 < argc)
            height = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--frames") && i + 1 < argc)
            options.max_frames = (unsigned)strtoul(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--screenshot") && i + 1 < argc)
            options.screenshot_path = argv[++i];
        else {
            usage(argv[0]);
            return 2;
        }
    }

    if (port <= 0 || port > 65535) {
        fprintf(stderr, "invalid port %d\n", port);
        return 2;
    }

    /* Step 1: wait for the remote GPU to connect over a websocket.  The
     * connection (and all websocket knowledge) stays here, in the app. */
    Connection conn;
    if (connection_accept(&conn, (uint16_t)port) != 0)
        return 1;

    /* Step 2: WebGPU device on top of that connection; the frame size
     * comes from the client's canvas.  Nothing about the triangle here. */
    GpuContext ctx;
    if (gpu_setup(&conn, (uint32_t)width, (uint32_t)height, &ctx) != 0) {
        connection_close(&conn);
        return 1;
    }

    /* Step 3: the main loop, handed a device it did not create. */
    int rc = render_run(&ctx, &options);

    gpu_teardown(&ctx);
    connection_close(&conn);
    return rc;
}
