# The no-op backend

`wgpu::Instance` in this workspace normally means "one connected browser tab".
That makes any test needing a `Device` need a browser, a port and a websocket.
The no-op backend removes all three: it produces a real device that talks to
nobody.

## Using it

Exactly the opt-in upstream wgpu requires, so code written against real wgpu
works unchanged:

```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::NOOP,
    backend_options: wgpu::BackendOptions {
        noop: wgpu::NoopBackendOptions { enable: true },
        ..Default::default()
    },
    ..
});
```

`wgpu::Instance::new_noop()` is the shorthand. Both need `enable`, which is
false unless set explicitly or by `WGPU_NOOP_BACKEND=1`, so the real server
cannot reach this path by accident.

## What it does and does not do

It is not a software renderer. The C library runs exactly as it does for a real
connection: objects get ids, refcounts are tracked, `Drop` releases them, and
the adapter reports limits. The only difference is the transport.

Works:

* creating and dropping every kind of GPU object;
* recording and submitting commands;
* `queue.write_buffer` and friends;
* adapter and device limits, which come back as the WebGPU baseline.

Does not work, and will hang rather than fail:

* anything that waits for the client to answer. `Buffer::map_async`,
  `Queue::on_submitted_work_done` and screenshot readback never complete,
  because nothing ever replies.
* anything that needs a surface. There is no canvas.

So it suits tests that exercise CPU-side bookkeeping over real GPU handles,
which is what the vendored `bevy_render`'s slab and mesh allocator tests do.

## How it works

Three pieces, each small:

* `Runtime::start_offline` in `remote-wgpu-runtime` builds the runtime with no
  listener and no accept thread. It still owns the C lock and the progress
  counter, so every object in the shim behaves normally. `runtime_offline()`
  reaches it, and returns an already-running listening runtime if there is one.
* `Client::loopback` builds a client whose send callback drops what it is
  given, then completes the handshake in-process by feeding the C library a
  synthetic `ClientHello`. The hello carries only the protocol version;
  `sanitize_limits` in the C library clamps every unset limit up to the WebGPU
  default, which is where the baseline limits come from.
* `Instance` in the `wgpu` shim holds that client when the no-op backend was
  asked for, and hands it to every adapter it produces. Nothing downstream
  changes, because an `Adapter` was always just a `Client`.

A process cannot mix the two. `Runtime` is a process-wide singleton, so if the
offline one starts first, anything that needs a browser panics through
`Runtime::require_listening` rather than blocking forever waiting for a client
that can never connect.

The protocol version in the synthetic hello comes from
`remote_wgpu_sys::PROTOCOL_VERSION`, which `remote-wgpu-sys`'s build script
lifts out of `proto/remote_webgpu.proto`. Nothing hard-codes it any more,
including the fake client the lifetime tests use.

## Tests

`cargo test -p wgpu --test noop_backend` covers the backend itself: baseline
limits, buffer create/write/drop, two independent devices, and the guard that
keeps a default instance from quietly becoming a no-op one.

`cargo test -p bevy_render --lib` is what motivated it. Fourteen vendored
upstream tests build a device through `test_utils::create_dummy_device`; they
were dead in this workspace until the shim grew a no-op backend.
