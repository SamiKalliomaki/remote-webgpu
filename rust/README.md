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

## Notes and limitations

- Only the API surface used by the wgpu examples is implemented; anything
  else is a compile error (missing method) or a `panic!` with an
  explanation (native-only features).
- `remote-wgpu-sys` regenerates nothing at build time except the protobuf
  code; after changing `webgpu.h`, re-run
  `python3 tools/generate_ffi.py` in `remote-wgpu-sys/`.
