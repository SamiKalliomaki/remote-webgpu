# The shared `AssetServer` leaked one set of asset loaders per join (fixed 2026-09-05)

Status: **fixed**, by vendoring `bevy_asset` and making loader registration
reuse the slot a loader type already holds. This was the one part of the
shared-`AssetServer` problem that
[`shared-asset-server-id-aliasing.md`](shared-asset-server-id-aliasing.md) did
not repair.

## What you saw

Once a second tab joined, the server logged this, and logged it again on every
further join:

```
WARN bevy_asset::server::loaders: Duplicate AssetLoader registered for Asset
type `bevy_shader::shader::Shader` with extensions
`["spv", "wgsl", "vert", "frag", "comp", "wesl"]`. Loader must be specified in
a .meta file in order to load assets of this type with these extensions.
```

Nothing broke at that moment. The warning was a symptom, not a failure.

## Why it happened

Each late joiner gets its render sub-app from a throwaway `App` that runs the
identical plugin stack against the *real* app's `AssetServer`
(`harvest_render_app` in `bevy_pbr_game/src/main.rs`). That stack registers its
asset loaders as it builds, and `App::register_asset_loader` /
`init_asset_loader` go straight to whichever `AssetServer` is in the world,
which for a throwaway is the shared one.

Upstream's `AssetLoaders` appends unconditionally, through both of its entry
points:

* `push` (from `AssetServer::register_loader`) adds a new element to
  `loaders: Vec<MaybeAssetLoader>` holding an `Arc<dyn ErasedAssetLoader>`, and
  appends that index to `type_id_to_loaders[A]` and to
  `extension_to_loaders[ext]` once per extension the loader claims. Only
  `type_path_to_loader` is a plain overwrite. Registering the same loader type
  a second time takes the `is_new` branch, because that branch is skipped only
  for a loader that was *preregistered* and is now being filled in.
* `reserve` (from `AssetServer::preregister_asset_loader`) appends a `Pending`
  slot the same way and shadows the previous entry. `ImagePlugin` preregisters
  `ImageLoader` in `build` and registers it in `finish`, so a join went through
  both paths.

Nothing ever removed one: `AssetLoaders` had no remove or clear, and
`AssetServer` exposed no way to reach it. The throwaway app is dropped seconds
later. Its loaders were not.

Growth was one set of loaders per join, for the life of the process. This stack
registers only a handful of real loaders, chiefly `ShaderLoader` and the image
loaders, so it accumulated slowly rather than alarmingly; what it retained was
the `Arc<dyn ErasedAssetLoader>` for each, plus a `usize` in one list per
claimed extension.

Loader *resolution* stayed correct throughout — `AssetLoaders::find` prefers the
most recently registered candidate at every step — but the duplicates cost the
fast path for an asset type with exactly one loader, so type-directed loads fell
through to extension matching, and a path whose extension matched nothing logged
`Multiple AssetLoaders found for Asset: ..; Path: ..;` on every load.

## Fix

`bevy_asset` 0.19.1 is now vendored at `rust/bevy_asset` and patched in through
`[patch.crates-io]`, exactly as `bevy_render` already was. Two changes, both
marked `FORK:` in `src/server/loaders.rs`:

* `push` replaces the loader in place when `type_path_to_loader` already holds
  a `Ready` slot for that loader type, instead of appending a second copy. A
  slot listed in `type_path_to_preregistered_loader` is a reservation waiting
  to be filled in, so it still takes the existing fill-in branch.
* `reserve` returns without doing anything when the loader type already has a
  slot. An existing slot is either still `Pending` — a reservation nobody has
  filled in yet, which is what the call wanted anyway — or already `Ready`, in
  which case there is nothing left to wait for. (Re-reserving a `Ready` loader
  must *not* push it back to `Pending`; that would block every load of the type
  forever.)

`AssetServer::registered_loader_count` is exposed alongside them so an app can
assert the property from outside the crate.

Since a loader type path identifies the loader type, the instance taking over
the slot is by construction an instance of the same loader, registered by the
same plugin from the same stack; last-registration-wins resolution order is
preserved.

Regression tests:

* `cargo test -p bevy_asset --lib server::loaders` — three fork tests covering
  re-`push`, re-`push` after a `reserve`, and re-`reserve`, each asserting that
  all three tables stay at one entry and that the newest instance is the one
  stored.
* `cargo test -p bevy_pbr_game --test shared_asset_loaders` — the real
  `DefaultPlugins` stack, built on a no-op-backend device (see
  [`noop-backend.md`](noop-backend.md)) and then rebuilt three times against
  the first app's `AssetServer`, asserting the loader count never moves and
  that `.wgsl` and `.png` still resolve.
* `cargo test -p bevy_pbr_game --test asset_registry` — the same property in
  the smaller harness that already covers the id-aliasing repair.

## If you are checking whether it came back

The count of `Duplicate AssetLoader` lines in a log should now be zero no matter
how many tabs join; the same goes for `Multiple AssetLoaders found` at load
time. Per the lifetime-measurement recipe in
[`client-lifetime-leak.md`](client-lifetime-leak.md), run join churn under
`MALLOC_ARENA_MAX=1` if you want to watch RSS as well — this leak was always far
too small to see against per-thread arena noise.
