# remote-webgpu end-to-end tests

Everything-in-one-command tests for the remote WebGPU stack: a native
server built on `../remote_webgpu` talks over a real websocket to the
TypeScript client (`../client`) running in a headless browser, and the
results are verified server-side by reading them back over the wire.

```sh
./run.sh
```

`run.sh` builds the native targets and the web bundle, serves the page,
launches headless chromium once per test and reports `PASS`/`FAIL` (exit
code to match).  No GPU or display is needed -- chromium's SwiftShader
fallback is enough.  Override the browser or ports with
`CHROMIUM=/path/to/browser PORT=9000 ./run.sh`.

Each feature is its own executable (`src/test_<name>.c` defines
`run_test()`; the shared `test_main.c` owns the websocket accept and device
bring-up), run against a fresh browser session.  `./run.sh compute queries`
runs a subset.

| Test | What it proves |
| --- | --- |
| `limits` | Adapter/device limits and features from the `ClientHello` answer the local getters. |
| `buffers` | Mapped-at-creation writes, `writeBuffer`, `copyBufferToBuffer`, `clearBuffer`, read mapping and the buffer getters, all verified byte-for-byte. |
| `compute` | A compute pipeline with an implicit "auto" layout via `getBindGroupLayout`, a dispatch and a verified readback. |
| `render` | `writeTexture` + samplers + texture bind groups, depth/stencil, blending and an indexed draw, with exact pixel verification of the render target. |
| `queries` | Occlusion queries around draws, `resolveQuerySet`, and verified sample counts (positive for a full-screen draw, zero for none). |
| `image` | The texture-from-URL extension: the client fetches and decodes `web/test-image.png` into a texture (verified texel-by-texel over a readback), and a missing URL fails cleanly through the callback. |
| `async` | The asynchronous round-trips: clean and dirty error scopes (a too-large buffer must surface as a caught validation error), `onSubmittedWorkDone` and `getCompilationInfo`. |
| `golden` | The rendering/present path end to end: `spinning_triangle` draws 30 frames paced by the client's vsync acks and reads the final frame back; the triangle rotates a fixed angle per frame and `web/index.html` pins the canvas size, so the PPM is compared **bit-for-bit** against `golden/triangle.ppm`. |

The golden image is tied to the rendering stack (chromium/SwiftShader
version); when it legitimately changes, re-record it with
`UPDATE_GOLDEN=1 ./run.sh` and commit the new file.

The tests reuse the example server's websocket plumbing
(`../example_server/src/{connection,ws_server,ws_transport,gpu_setup}.c`);
`web/` is a minimal page that hands its canvas and GPU to the server, with
progress on the console.
