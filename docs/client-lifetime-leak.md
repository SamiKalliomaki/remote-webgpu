# The client-lifetime leak

**Status:** fixed 2026-09-02.  Identified the same day while hardening the
server against hostile clients.  This document records what was actually
leaking (it was not quite what the first analysis listed), the fix, and how
to measure it -- the measurement has a trap.

## Summary

Nothing released a client when its connection ended.  Every browser tab
that completed the handshake permanently retained its `Client`, the C
instance/adapter behind it, the surface built on it and (in
`bevy_pbr_game`) the GPU-side state of the render sub-app harvested for
it.  Reconnecting in a loop therefore grew the server's memory without
bound, and handed an unauthenticated peer an OOM: connect, complete the
handshake, disconnect, repeat.

## What was retaining a disconnected client

`Client` is reference-counted (`Arc<Client>`), and the count never reached
zero.  The holders, in the order they were found:

| Retainer | Where | Fixed by |
| --- | --- | --- |
| Registry `clients: Mutex<Vec<Arc<Client>>>` | `rust/remote-wgpu-runtime/src/lib.rs` | Now `Vec<Weak<Client>>`, entry removed on disconnect. |
| `unclaimed: VecDeque<Arc<Client>>` | same | Entry removed on disconnect. |
| `SendCtx` handed to the C adapter | same, was `Box::leak` | Owned by `Client` (`Box<SendCtx>`), freed in `Client::drop`. |
| Strong `Arc<Client>` handed to the C event callback | same, was `Arc::into_raw` | A plain pointer; valid because only the reader thread fires events, and it clears the callback before letting go. |
| `SURFACES: HashMap<u64, Arc<SurfaceShared>>` | `rust/wgpu/src/api.rs` | Now `Weak<SurfaceShared>`: the `Surface` handles own it, the table only finds it. |
| **`ErrorScopeGuard::pop` leaked its `Device`** | `rust/wgpu/src/api.rs` | `mem::forget(self)` (to skip the guard's own pop) also forgot the `Device` clone inside.  bevy pushes one scope per shader compile, so every render world leaked ~22 device handles, each holding the client. |
| **Requests issued after the disconnect** | `remote_webgpu/src/remote_webgpu.c` | A present/map/work-done issued in the window between the socket closing and the application noticing queued a request nothing could ever answer, pinning its buffer (and through it the session).  A vsync wait registered this way also held a leaked `Arc<Client>`. |
| Uncaptured-error handler table, keyed by raw device | `rust/wgpu/src/api.rs` | Entry removed in `OwnedDevice::drop` (under the C lock, before the address can be recycled). |

The bold rows were not in the original analysis.  The first five retainers
were real, but fixing them alone made no measurable difference: the
`ErrorScopeGuard` leak kept every client alive regardless, and it was only
visible once the count of live `Device` clones was instrumented (~30 per
player, 24 of them surviving the render world's drop).

## The fix

Ownership is now explicit.  The runtime does not own clients: the
connection's reader thread holds one strong reference while the socket is
open, the `unclaimed` queue holds one until a window claims it, and after
that only the application's handles (`Window`, `Adapter`, `Device`,
`Surface`) keep a client alive.  `Client::drop` releases the C adapter and
instance.

* **`wgpuRemoteAdapterDisconnect()`** (new, `remote_webgpu/include/webgpu/remote.h`)
  severs an adapter from its transport: stops calling `send`, forgets the
  event callback, abandons pending requests, and -- the part that makes
  the Rust side sound -- fails any request issued *afterwards* on the spot
  (`rw_send_envelope` abandons the queue when there is no transport;
  `wgpuRemoteSurfaceOnNextVsync` fires its callback immediately).  The
  runtime calls it from `mark_disconnected`, and `Client::drop` calls it
  again defensively.
* **The C library refcounts across objects** (a device holds its adapter,
  a surface its instance and configured device), so Rust-side drop order
  is *not* load-bearing: `Client::drop` releasing the adapter while a
  `Buffer` is still alive is fine.  What *was* load-bearing is that a late
  `wgpuBufferRelease` sends `DestroyObject` through `send` -- hence
  `Disconnect` must run before `SendCtx` is freed, which `Client::drop`
  guarantees.
* **`HasRemoteClient for Client`** no longer scans the registry (which
  used to `.expect()`): clients are built with `Arc::new_cyclic` and carry
  their own `Weak`.
* **Every connection exit runs the same teardown.**  `connect_client`
  builds the client, then `pump_client` does the handshake and reader loop,
  and `mark_disconnected` always follows -- a peer that hangs up
  mid-handshake used to skip it.
* `Runtime::clients()` now lists *connected* clients only.
* `bevy_pbr_game` needed no change: it already dropped the render sub-app
  and its `PlayerView` when a player left.  (It never drops player 0's
  render app; that one client is kept by design.)

## Measurement, and its trap

```sh
cargo run -p bevy_pbr_game            # then open a browser tab as player 0
ps -o rss= -C bevy_pbr_game           # baseline

for i in $(seq 1 8); do
    python3 e2e/tools/hostile_client.py --port 8000 --seconds 0
done
ps -o rss= -C bevy_pbr_game           # after
```

(`hostile_client.py --seconds 600` works as player 0 too, for a headless
run.)

**RSS is dominated by glibc, not by live objects.**  bevy renders on a
thread pool, every pool thread gets its own malloc arena, and memory freed
in a non-main arena is rarely returned to the OS.  With all retainers
fixed and every client logged as `released`, 2026-09-02, dev profile:

| glibc settings | Baseline | 40 cycles | 80 | 120 | 160 | 200 |
| --- | --- | --- | --- | --- | --- | --- |
| default | 112 MB | 351 MB | 394 MB | 415 MB | 438 MB | 447 MB |
| `MALLOC_ARENA_MAX=1 MALLOC_TRIM_THRESHOLD_=0 MALLOC_MMAP_THRESHOLD_=65536 MALLOC_TOP_PAD_=0` (80 cycles) | 98 MB | 116 MB | 117 MB | | | |

Before the fix, the same loop grew ~15 MB per cycle in both configurations
and never flattened.  After it, the default-settings curve decelerates
(+239, +43, +21, +23, +9 MB per 40 cycles) and the tuned one is flat from
cycle 20 on; the tuned run is the one that shows the leak is gone (a
decelerating curve under default settings is arena growth, a straight line
is a leak).  If the default-settings growth
matters for a deployment, the answer is an allocator (`mimalloc`) or
`MALLOC_ARENA_MAX`, not this code.

## Verification

* `cargo test -p remote-wgpu-runtime -p wgpu --test client_lifetime`: a
  fake client (`remote-wgpu-runtime/tests/support/fake_client.rs`, shared
  by both crates via `#[path]`) connects, handshakes and hangs up; the
  tests assert the registry forgets it, an unclaimed client is freed by
  the disconnect alone, a mid-handshake hang-up leaves nothing behind, and
  that a surface, adapter and device built on a client release it when
  dropped in an order that exercises the C-side refcounts.  The three
  runtime tests were run against the pre-fix code and all fail there (they
  time out waiting for the `Weak` to die).
* The churn above, watching for one `remote-wgpu: client #N released` per
  cycle.
* `./e2e/run.sh` passes end to end (`hostile`, `fuzz` with ASan/UBSan,
  and `golden` bit-for-bit).

## Hazards that remain worth knowing

* **`Client::drop` runs on whichever thread drops the last reference** and
  takes the reentrant C lock.  Never drop an `Arc<Client>` (or anything
  holding one -- a `Device`, a `SurfaceShared`) while holding another lock
  that a thread inside the C library might want; `surface_shared` takes
  care to only touch `Weak`s under the `SURFACES` mutex, and
  `OwnedDevice::drop` drops the removed error handler with no locks held.
* **`AbandonRequests` fires application callbacks synchronously**, now
  also from inside a request call made after disconnect.  Callbacks must
  therefore tolerate running before the issuing call returns;
  `SurfaceTexture::present` counts the frame *before* registering the
  vsync wait for exactly this reason.
* **A pending vsync wait holds a strong `Arc<Client>`** (reclaimed in the
  callback).  That is what makes `Client::drop` safe to call `Disconnect`:
  no wait can be pending when the count is zero.
