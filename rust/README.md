# Rust crates: wgpu and winit over remote WebGPU

Drop-in replacements for the [`wgpu`](https://crates.io/crates/wgpu) and
[`winit`](https://crates.io/crates/winit) crates that make an unmodified
wgpu application render on a remote browser's GPU: the "window" is the
connected web client's canvas, and every GPU call travels over the
websocket protocol implemented by `../remote_webgpu`.

| Crate | What it is |
| --- | --- |
| `remote-wgpu-sys` | Raw FFI bindings to the `remote_webgpu` C static library (built by `build.rs` with `cc` + `protoc`).  `src/ffi.rs` is generated from `webgpu.h` by `tools/generate_ffi.py`, together with a C-vs-Rust struct-size self-check (`cargo test -p remote-wgpu-sys`). |
| `remote-wgpu-runtime` | The shared runtime both API crates rendezvous through: a websocket server (`REMOTE_WEBGPU_PORT`, default 8080) accepting any number of browser clients.  Each connection gets its own remote instance/adapter, reader thread, event queue (resizes, keys, pointer) and present/vsync pacing.  All C calls happen under one reentrant lock. |
| `wgpu` | The wgpu-compatible API (version `30.0.0-remote`), matching the upstream API surface the wgpu examples exercise: instance/adapter/device/queue, buffers with mapping, textures/views/samplers, bind groups, render + compute pipelines and passes, render bundles, query sets, error scopes, the surface swapchain, and `wgpu::util` (`DeviceExt`, `StagingBelt`, `TextureBlitter`, `include_wgsl!`, `vertex_attr_array!`). |
| `bevy_game` | An example game built on these crates: one Bevy world, one view per connected browser tab.  See [Multiple clients](#multiple-clients). |
| `winit` | A winit-compatible event loop (version `0.30.999`).  `ActiveEventLoop::create_window` claims the next connected client (blocking until one connects), so each `Window` is its own browser tab.  Client events are translated into `WindowEvent`s for that window — canvas resizes to `Resized`, `keydown`/`keyup` user events to `KeyboardInput` (browser `code`/`key` names mapped to `KeyCode`/`Key`), `mousemove` to `CursorMoved`, a disconnect to `CloseRequested` — and each window's `RedrawRequested` is paced to its client's vsync acknowledgements. |

## Running the upstream wgpu demos

`../run_wgpu_example.sh` checks out a pinned wgpu commit, points the
workspace's `wgpu` and `winit` dependencies at these crates, and runs any
demo from `examples/features/src`:

```sh
./run_wgpu_example.sh            # list the demos
./run_wgpu_example.sh cube       # start "cube", waiting on port 8080
```

Then connect the web client (same as for the C example server):

```sh
cd client && npm install
cd ../example_client && npm install
npm run serve                    # http://127.0.0.1:8000
```

Open <http://127.0.0.1:8000> (append `?server=ws://host:port` for a
non-default port) and the demo renders full-screen in the browser.  Key
presses and mouse movement in the page are forwarded back to the demo.

Demos that need capabilities browser WebGPU does not have (ray tracing,
mesh shaders, cooperative matrices, multiview) are removed from the build;
a few others (`texture_arrays`, `conservative_raster`,
`big_compute_buffers`) need native-only wgpu features and exit with a clear
message.

## Multiple clients

Any number of browser tabs can connect; each one becomes its own `Window`
with its own `Adapter` (that tab's GPU):

- `ActiveEventLoop::create_window` claims the next connected client,
  blocking until one connects.
- `Instance::create_surface(window)` refers to that window's canvas, and
  `Instance::request_adapter` with that surface as `compatible_surface`
  resolves to that client's adapter (`Adapter::is_surface_supported`
  pairs them up, which is what the wgpu examples' adapter selection uses).
  Without a surface — compute-only apps — the first connected client's
  adapter is returned.  `Instance::enumerate_adapters` lists one adapter
  per connected client.
- A disconnect delivers `CloseRequested` for that window only; other
  windows keep running.

`cargo run -p wgpu --example multi_client` demonstrates this: it waits for
two tabs and animates a differently-colored clear on each, driven by two
independent adapters/devices, until both tabs are closed.  A device can
only drive resources of its own client, so the upstream `hello_windows`
demo (one device, many windows) does not apply to this backend.

`ActiveEventLoop::create_window_for_new_client` is a remote-webgpu
extension for applications that let clients join while they are running:
it claims a connected-but-unclaimed client if there is one and returns
`None` otherwise, where `create_window` would block.

## The example game

`cargo run -p bevy_game` is a small multiplayer-viewport game: one Bevy
world in one process, and one *view* per connected browser tab.  Every tab
that connects becomes a `Window` on its own remote adapter, gets a
character spawned into the shared world, and renders that world from a
camera following its own character -- so the players walk around the same
arena, bump into each other and race for the same coins, each watching
from their own machine's GPU.  Players can join and leave at any time;
the game exits when the last one goes.  Move with WASD or the arrow keys,
dash with space.

Bevy runs the game -- ECS, systems, `Time`, `Transform` -- but not the
rendering.  `bevy_render` owns a single `RenderDevice`, while here each
tab is a separate GPU whose device can only touch its own resources and
its own canvas, so no one Bevy renderer can draw into several tabs.  Each
view instead has a small instanced-quad pipeline of its own
(`src/render.rs`), which is also why the game is flat rather than 3D.

`--frames N --screenshot PATH` writes the first player's view out as a
PPM and exits, by copying the canvas texture back over the websocket:
browsers do not expose a WebGPU canvas to page screenshots, so that is
the only way to see what a client actually drew.

Note that `bevy = { default-features = false }` alone leaves
`bevy_platform` without `std`, which silently substitutes a stub clock and
makes every `Time` delta meaningless; the `std` feature is required.

## Notes and limitations

- Only the API surface used by the wgpu examples is implemented; anything
  else is a compile error (missing method) or a `panic!` with an
  explanation (native-only features).
- `remote-wgpu-sys` regenerates nothing at build time except the protobuf
  code; after changing `webgpu.h`, re-run
  `python3 tools/generate_ffi.py` in `remote-wgpu-sys/`.
