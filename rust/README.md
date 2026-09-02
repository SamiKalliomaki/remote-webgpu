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
| `wgpu` | The wgpu-compatible API, implementing the wgpu **29** public API on top of the real [`wgpu-types`](https://crates.io/crates/wgpu-types) 29 crate (so all plain data types are shared with any other crate compiled against wgpu 29 — bevy above all): instance/adapter/device/queue, buffers with mapping, textures/views/samplers, bind groups, render + compute pipelines and passes, render bundles, query sets, error scopes, the surface swapchain, `ShaderSource::Wgsl` and `ShaderSource::Naga` (Naga IR is written back out as WGSL for the browser), and `wgpu::util` (`DeviceExt`, `StagingBelt`, `TextureBlitter`, `include_wgsl!`, `vertex_attr_array!`). |
| `bevy_render` | A vendored fork of bevy 0.19's renderer with **multi-render-world** support: one complete render world (device, pipeline cache, render graph) per connected browser tab, all extracting from the one main world.  See [The bevy_render fork](#the-bevy_render-fork). |
| `bevy_pbr_game` | The same idea on bevy's real 3D pipeline: `bevy_pbr` renders the shared world once per player, each on that player's own GPU, through the `bevy_render` fork.  See [The bevy_pbr example game](#the-bevy_pbr-example-game). |
| `winit` | A winit-compatible event loop (version `0.30.999`).  `ActiveEventLoop::create_window` claims the next connected client (blocking until one connects), so each `Window` is its own browser tab.  Client events are translated into `WindowEvent`s for that window — canvas resizes to `Resized`, `keydown`/`keyup` user events to `KeyboardInput` (browser `code`/`key` names mapped to `KeyCode`/`Key`), `mousemove` to `CursorMoved`, a disconnect to `CloseRequested` — and each window's `RedrawRequested` is paced to its client's vsync acknowledgements. |

## Running the upstream wgpu demos

`../run_wgpu_example.sh` checks out the wgpu v29.0.4 release (the same
API generation this shim implements), points the
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

## The example game (flat renderer)

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
rendering: each view has a small instanced-quad pipeline of its own
(`src/render.rs`), which is why the game is flat rather than 3D.  It
predates the `bevy_render` fork below, and stays as the minimal example
of driving several GPUs by hand.

`--frames N --screenshot PATH` writes the first player's view out as a
PPM and exits, by copying the canvas texture back over the websocket:
browsers do not expose a WebGPU canvas to page screenshots, so that is
the only way to see what a client actually drew.

Note that `bevy = { default-features = false }` alone leaves
`bevy_platform` without `std`, which silently substitutes a stub clock and
makes every `Time` delta meaningless; the `std` feature is required.

## The bevy_render fork

Upstream `bevy_render` assumes exactly one render world on one
`RenderDevice`.  On this backend every browser tab is a separate GPU whose
device can only touch its own resources and its own canvas, so the
`bevy_render` directory vendors bevy 0.19.1's renderer with the minimal
changes that let one bevy `App` hold **one render sub-app per connected
tab** — identical systems, fully independent GPU state:

- **Entity sync** (`sync_world.rs`): the pending-sync queue fans records
  out to one queue per registered render world
  (`register_render_world` / `unregister_render_world`), each keeping its
  own main→render entity map and re-pointing the main world's
  `RenderEntity` components at itself before its extraction runs (render
  worlds extract sequentially, so this is sound).  A world registered
  mid-game is seeded with the already-synced entities.
- **Per-render-world asset cursors** (`render_asset.rs`,
  `erased_render_asset.rs`): the `AssetEvent` reader used for extraction
  moved from a shared main-world resource into per-render-world system
  state, so one render world draining the events no longer starves the
  others.
- **Window ownership** (`view/window/mod.rs`, `camera.rs`,
  `screenshot.rs`): a render world holding the `OwnedWindow` resource
  extracts only that window and only cameras targeting it, since another
  tab's surface belongs to another GPU.
- **Surfaces without window handles** (`renderer/mod.rs` plus the wgpu
  shim): a window's remote client id travels through
  `RawWindowHandle::Web`, and the shim's `create_surface_unsafe` decodes
  it back into that client's canvas surface.
- `share_screenshot_channel` re-points a harvested render world's
  screenshot sender at the running app's receiver.

Everything else — `bevy_pbr`, `bevy_core_pipeline`, and the rest of the
bevy 0.19 crates — is used unmodified from crates.io via the
`[patch.crates-io]` entry in `Cargo.toml`.

## The bevy_pbr example game

`cargo run -p bevy_pbr_game` is the multiplayer-viewport game rendered by
bevy's standard PBR mesh pipeline.  One `App` holds the shared world; the
first tab's GPU becomes the built-in `RenderApp` (created with
`RenderCreation::Manual` on that client's adapter/device), and every later
tab gets a complete render sub-app of its own, built by running the
identical plugin stack in a throwaway `App` and harvesting the built
sub-app into the running game.  A custom runner steps the shared
simulation and then extracts + updates each player's render world only
when that tab's vsync acknowledgement has arrived, so every view is paced
by its own browser.

Things the runner does at each join, all of which exist because a render
world can now be born mid-game (`src/main.rs` has the details):

- the throwaway app shares the real app's `AssetServer`, so embedded
  shader handles line up between the harvested render world and the real
  main world;
- the harvested world's change-tick counter is spun forward to the main
  world's, because cross-world change detection otherwise sees the whole
  main world as "unchanged" through tick wraparound;
- `AssetEvent::Modified` is replayed for every live mesh, image, material
  and shader, because those events expired long before the new render
  world could extract them;
- resources that hold `Assets::add`-created handles from plugin build time
  (`DownsampleShaders` is the one such plugin in this stack) are re-copied
  from the real app, because runtime asset handles are only meaningful in
  the world whose storage allocated them — embedded assets are safe, since
  the shared `AssetServer` dedupes them by path.

GPU preprocessing runs in its `PreprocessingOnly` mode (compute-shader
`MeshUniform` building with direct draws); the `Culling` mode needs
multi-draw-indirect and immediates, which WebGPU does not have.

`--frames N --screenshot PATH` captures every player's view through
bevy's own screenshot readback (`shot.png`, `shot.1.png`, ...) and exits.

## Notes and limitations

- Only the API surface used by the wgpu examples is implemented; anything
  else is a compile error (missing method) or a `panic!` with an
  explanation (native-only features).
- `remote-wgpu-sys` regenerates nothing at build time except the protobuf
  code; after changing `webgpu.h`, re-run
  `python3 tools/generate_ffi.py` in `remote-wgpu-sys/`.
