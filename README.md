# remote-webgpu

WebGPU over the network: a native application written against the standard
`webgpu.h` API whose GPU lives on the other end of a websocket (e.g. in a
browser tab exposing `navigator.gpu`).

| Directory | What it is |
| --- | --- |
| `proto/` | The wire protocol, as protobuf messages.  One serialized `Envelope` per binary websocket message. |
| `remote_webgpu/` | C static library implementing the `webgpu.h` headers.  Transport-agnostic: the app supplies a send callback and pushes received messages in via `wgpuRemoteAdapterReceiveData()`; completions (vsync futures, buffer maps) fire from inside that call.  Everything the triangle demo needs is implemented; the rest are generated stubs that abort with `remote_webgpu: unimplemented: wgpu...`. |
| `client/` | `remote-webgpu-client`, the TypeScript client library.  Connects to the server, obtains a local adapter/device from `navigator.gpu` and performs the handshake. |
| `example_client/` | Web page with a full-screen canvas that uses the client library. |
| `example_server/` | Native example app (spinning triangle) built on `remote_webgpu`.  Owns all the websocket code, hands the library a send callback, pumps received messages into it, and renders at whatever size the client reports for its canvas — no window system needed server-side. |

## Protocol so far

1. The client (browser) connects to the server's websocket.
2. Server sends `ServerHello { protocol_version }`.
3. Client replies `ClientHello { protocol_version, adapter }`, where
   `adapter` describes the GPU it obtained locally.
4. The server then streams commands mirroring the webgpu.h calls the
   application makes (resource creation, render passes, submits); objects
   are referenced by server-assigned ids.  Two commands round-trip:
   `Present` waits for `PresentDone` (sent after the client's next
   `requestAnimationFrame`, which paces the server's render loop to the
   client's refresh rate) and `MapBufferRead` waits for `MapBufferData`
   (which is how `--screenshot` reads the frame back over the network).
   The client also sends `Event` notifications: a built-in canvas-resize
   event and user-defined named events (the example client streams
   `"mousemove"`).  The server application listens to them via
   `wgpuRemoteAdapterSetEventCallback()`; on a resize it reconfigures the
   surface, which is what actually resizes the canvas backing store -- the
   canvas's pixel size always matches the server's last `ConfigureSurface`.
   The initial canvas size travels in `ClientHello`.

The subset implemented is exactly what the triangle demo needs: buffer /
shader / bind group / pipeline creation, one render pass with color
attachments, draw, submit, present, and texture-to-buffer readback.
Everything else still aborts with `remote_webgpu: unimplemented: wgpu...`
and is listed in `remote_webgpu/src/stubs.c`.

## Running the demo

Native server (see `example_server/README.md` for dependencies):

```sh
cmake -S example_server -B example_server/build
cmake --build example_server/build
./example_server/build/spinning_triangle        # waits on port 8080
```

Web client:

```sh
cd client && npm install
cd ../example_client && npm install
npm run serve        # bundles and serves http://127.0.0.1:8000
```

Open <http://127.0.0.1:8000> (append `?server=ws://host:port` for a
non-default server).  The page connects and the spinning red triangle the
server draws appears full-screen on the page's canvas, rendered by the
browser's GPU, with a live FPS counter in the corner.  Resizing the browser
window resizes the render, and a small green triangle follows the mouse
(the pointer position travels as a user-defined event).

This also works fully headless, which is how it is tested:

```sh
./example_server/build/spinning_triangle --frames 40 --screenshot frame.ppm &
chromium --headless=new --enable-unsafe-webgpu --enable-features=Vulkan \
  --use-angle=vulkan "http://127.0.0.1:8000/?server=ws://127.0.0.1:8080"
```

`frame.ppm` then contains the triangle as rendered by Chromium (SwiftShader
if no real GPU is present) and read back over the websocket.

## Regenerating protocol code

After editing `proto/remote_webgpu.proto`:

* C: regenerated automatically by the CMake build (`protoc --c_out`, linked
  against `libprotobuf-c`).
* TypeScript: `cd client && npm run gen` (protoc + `@bufbuild/protoc-gen-es`,
  output in `client/src/gen/`).
