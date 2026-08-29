# remote-webgpu

WebGPU over the network: a native application written against the standard
`webgpu.h` API whose GPU lives on the other end of a websocket (e.g. in a
browser tab exposing `navigator.gpu`).

| Directory | What it is |
| --- | --- |
| `proto/` | The wire protocol, as protobuf messages.  One serialized `Envelope` per binary websocket message. |
| `remote_webgpu/` | C static library implementing the `webgpu.h` headers.  Transport-agnostic: the app supplies a send callback and pushes received messages in via `wgpuRemoteAdapterReceiveData()`; completions (vsync futures, buffer maps, error scopes, work-done, compilation info) fire from inside that call.  The full WebGPU API is forwarded -- everything expressible in the browser's `navigator.gpu` works remotely. |
| `client/` | `remote-webgpu-client`, the TypeScript client library.  Connects to the server, obtains a local adapter/device from `navigator.gpu` and performs the handshake. |
| `example_client/` | Web page with a full-screen canvas that uses the client library. |
| `example_server/` | Native example app (spinning triangle) built on `remote_webgpu`.  Owns all the websocket code, hands the library a send callback, pumps received messages into it, and renders at whatever size the client reports for its canvas — no window system needed server-side. |
| `rust/` | Rust crates: a drop-in `wgpu` replacement backed by `remote_webgpu`, a `winit`-compatible event loop that runs the websocket server instead of opening a window, plus the FFI/runtime crates underneath (see `rust/README.md`).  Multiple clients can connect; each becomes its own `Window` with its own `Adapter`.  `./run_wgpu_example.sh <name>` runs any upstream wgpu demo against them. |
| `e2e/` | End-to-end tests, one executable per feature: `./e2e/run.sh` builds everything, starts a native test server and a headless chromium per test, and verifies buffers, compute, rendering, queries, error scopes and readbacks over a real websocket -- plus a bit-for-bit golden-screenshot comparison of the spinning-triangle demo. |

## Protocol so far

1. The client (browser) connects to the server's websocket.
2. Server sends `ServerHello { protocol_version }`.
3. Client replies `ClientHello { protocol_version, adapter }`, where
   `adapter` describes the GPU it obtained locally.
3. (cont.) The `ClientHello` also carries the adapter's supported limits,
   features and WGSL language features, which is what the server answers
   `wgpuAdapterGetLimits()` / `GetFeatures()` / `HasFeature()` from.
4. The server then streams commands mirroring the webgpu.h calls the
   application makes (resource creation, render and compute passes,
   copies, submits); objects are referenced by server-assigned ids.  Most
   commands are one-way; the round-trips carry a request id and complete
   in any order:
   * `Present` -> `PresentDone` (sent after the client's next
     `requestAnimationFrame`, which paces the server's render loop to the
     client's refresh rate),
   * `MapBuffer` -> `MapBufferData` (buffer readback and write mapping;
     `--screenshot` uses it to read the frame back over the network),
   * `PopErrorScope` -> `ErrorScopeResult`,
   * `OnSubmittedWorkDone` -> `WorkDone`,
   * `GetCompilationInfo` -> `CompilationInfoResult`, and
   * `LoadTextureFromUrl` -> `TextureLoaded` (the
     `wgpuRemoteDeviceLoadTextureFromURL()` extension: the client fetches
     and decodes an image into a texture and reports its dimensions).
   The client also sends `Event` notifications: a built-in canvas-resize
   event and user-defined named events (the example client streams
   `"mousemove"` and `"keydown"`/`"keyup"`), plus unsolicited `UncapturedError` / `DeviceLost`
   reports that fire the callbacks from the device descriptor.  The server
   application listens to events via `wgpuRemoteAdapterSetEventCallback()`;
   on a resize it reconfigures the surface, which is what actually resizes
   the canvas backing store -- the canvas's pixel size always matches the
   server's last `ConfigureSurface`.  The initial canvas size travels in
   `ClientHello`.

The whole WebGPU API is forwarded: buffers (including `mappedAtCreation`
and write mapping), textures / views / samplers, bind groups of every
binding kind (with dynamic offsets), render and compute pipelines (with
blend, depth/stencil, pipeline constants and implicit "auto" layouts via
`getBindGroupLayout`), render bundles, query sets with occlusion queries
and `resolveQuerySet`, every copy command, `writeTexture`, indexed and
indirect draws, viewport/scissor/blend-constant/stencil-reference state,
debug groups and labels, error scopes and `requestDevice` with required
features and limits.  Beyond webgpu.h, `webgpu/remote.h` adds
`wgpuRemoteDeviceLoadTextureFromURL()`: the server names a URL, the client
fetches and decodes the image where it runs (its network, its codecs) and
hands back a ready rgba8unorm texture -- the natural way to get assets to
the GPU without streaming pixels over the websocket.  The only entry points that abort are the three
`wgpuExternalTexture*` methods, which cannot exist here (webgpu.h has no
way to create an external texture); the wgpu-native `SetImmediates`
extensions warn and do nothing, as the browser has no equivalent.
The end-to-end tests in `e2e/` exercise the lot headlessly and verify
the results by reading them back over the websocket.

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
