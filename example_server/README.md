# Spinning red triangle (C + remote WebGPU)

A minimal native WebGPU application in C. It links against
`../remote_webgpu`, our own implementation of the WebGPU headers that will
forward every call to a remote GPU over a socket (today it is a stub: only
object lifetime and adapter/device bring-up exist; every real method aborts
with `remote_webgpu: unimplemented: wgpu...`).

On startup the server listens for a websocket connection; once the remote
GPU connects, the adapter is created directly from that socket via the
`wgpuRemoteInstanceCreateAdapter()` extension (see
`../remote_webgpu/include/webgpu/remote.h`), the device is requested from it
as usual, and the main loop runs against that device.

There is no OS window and no GLFW: the server renders to the connected
client's canvas, at whatever size the client reports for it (initially in
its `ClientHello`, later via `CanvasResize` notifications).

The `remote_webgpu` library is transport-agnostic: it sends protocol
messages through a callback the app provides and receives them via
`wgpuRemoteAdapterReceiveData()`.  Everything websocket-specific lives here
in the app:

| File | Responsibility |
| --- | --- |
| `src/ws_server.c` | Listens on a TCP port, performs the RFC 6455 websocket handshake and hands back the connected fd. |
| `src/ws_transport.c` | Websocket framing on that fd (binary messages, ping/pong, fragmentation). |
| `src/connection.c` | Glues both to the library: `connection_send()` is the library's send callback, `connection_pump()` reads one message and feeds it to the library. |
| `src/gpu_setup.c` | Brings up the WebGPU instance, surface, adapter (on top of the connection) and device. Knows nothing about triangles. |
| `src/render.c` | The main loop. It is handed an **already-created device** via `GpuContext` and only borrows it; after each present it pumps the connection until the client's vsync future completes. |
| `src/gpu_context.h` | The handover struct between the two. |
| `src/main.c` | Wires them together: `connection_accept()` → `gpu_setup()` → `render_run()` → teardown. |

```c
Connection conn;
connection_accept(&conn, port);         /* blocks for the remote GPU */
GpuContext ctx;
gpu_setup(&conn, fallback_width, fallback_height, &ctx);
render_run(&ctx, &options);   /* device already exists; the loop just uses it */
gpu_teardown(&ctx);
connection_close(&conn);
```

## Dependencies

Arch packages:

```sh
sudo pacman -S --needed cmake gcc protobuf protobuf-c
```

No GPU driver, window system or `wgpu-native` is needed locally: the WebGPU
implementation comes from `../remote_webgpu`, and both the GPU and the
"window" (a canvas) live on the other end of the websocket.  `protoc`/`protobuf-c` are used to build the wire protocol
from `../proto/remote_webgpu.proto`.

## Build & run

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
./build/spinning_triangle          # waits for the remote GPU on port 8080
```

Options:

```
--port N                    websocket port to wait for the remote GPU on (default 8080)
--width N --height N        initial window size (default 800x600)
--frames N                  exit after N frames instead of running until closed
--screenshot FILE           write the final frame as a binary PPM
```

`--frames`/`--screenshot` exist so the renderer can be checked without a human
looking at the window:

```sh
./build/spinning_triangle --width 400 --height 300 --frames 400 --screenshot frame.ppm
```

## How it draws

Three vertices live in a vertex buffer. A 16-byte uniform buffer carries the
current `angle` (elapsed time × 1.5 rad/s) and the framebuffer `aspect`; the
vertex shader applies the 2-D rotation and divides x by the aspect so the
triangle stays equilateral when the client's canvas is resized. The fragment
shader returns a constant red.

Resizes (reported by the client) and `Outdated`/`Lost` swapchains are
handled by reconfiguring the surface from inside the loop; the device is
never recreated.
