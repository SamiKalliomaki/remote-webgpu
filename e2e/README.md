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
bring-up), run against a fresh browser session -- except `hostile`, whose
client is a python script rather than a browser.  `./run.sh compute queries`
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
| `hostile` | Untrusted-client handling: `tools/hostile_client.py` replaces the browser with a peer that completes the handshake with a 4-billion-pixel canvas, zeroed and saturated limits and 50 000 features, then sends repeated hellos, resize storms, event floods, unsolicited replies, fabricated errors and truncated envelopes, and answers every buffer map with fewer bytes than were asked for.  The server must clamp what it reports, fail the short maps, and still be standing when the peer hangs up. |
| `golden` | The rendering/present path end to end: `spinning_triangle` draws 30 frames paced by the client's vsync acks and reads the final frame back; the triangle rotates a fixed angle per frame and `web/index.html` pins the canvas size, so the PPM is compared **bit-for-bit** against `golden/triangle.ppm`. |

## Fuzzing

Everything a client sends reaches the library through one function --
`wgpuRemoteAdapterReceiveData()` -- which makes it a natural fuzz entry
point.  `fuzz/fuzz_target.c` is a libFuzzer target around it: each input is
one hostile client session, run against an application that has a device, a
configured surface and one request of every asynchronous kind outstanding
(a buffer map, a work-done, an error scope, compilation info, a texture
load, a presented frame awaiting its vsync ack), so the generated messages
have live state to complete, corrupt or contradict.  An input passes if
nothing crashed, nothing leaked, and the promises in `src/client_invariants.h`
still hold afterwards.

```sh
fuzz/run_libfuzzer.sh                      # build with clang, fuzz for 60s
fuzz/run_libfuzzer.sh -max_total_time=3600 -jobs=8
fuzz/run_libfuzzer.sh artifacts/crash-...  # reproduce a finding
```

`run_libfuzzer.sh` builds the target and the library with libFuzzer,
AddressSanitizer and UndefinedBehaviorSanitizer, seeds the corpus with one
well-formed message of every kind a client may send, and feeds the mutator
`fuzz/remote_webgpu.dict` (the protobuf tag of every `Envelope` variant --
without it the mutator rarely gets past the parser).  The corpus in
`fuzz/corpus/` persists between runs, which is what makes a long campaign
worth more than a short one; `-print_coverage=1` shows what it reaches.

`./run.sh fuzz` runs a 20-second pass as part of the suite (`FUZZ_SECONDS`
raises it).  Where clang is not available it falls back to `build/fuzz_receive`
-- the same target driven by the seeded generator in `fuzz/fuzz_main.c`,
which needs no tooling at all and replays saved inputs anywhere:

```sh
build/fuzz_receive --quiet --seconds 60 --seed 7    # generate and run
build/fuzz_receive some-input.bin                   # replay
```

Inputs that have found something live in `fuzz/regressions/` and are
replayed at the start of every fuzz stage; see the README there.

The golden image is tied to the rendering stack (chromium/SwiftShader
version); when it legitimately changes, re-record it with
`UPDATE_GOLDEN=1 ./run.sh` and commit the new file.

The tests reuse the example server's websocket plumbing
(`../example_server/src/{connection,ws_server,ws_transport,gpu_setup}.c`);
`web/` is a minimal page that hands its canvas and GPU to the server, with
progress on the console.
