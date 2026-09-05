# The shared `AssetServer` leaks one set of asset loaders per join

Status: **known, not fixed.** Bounded in practice, unbounded in principle, and
there is no API that can remove the entries. Documented here because it is the
one part of the shared-`AssetServer` problem that
[`shared-asset-server-id-aliasing.md`](shared-asset-server-id-aliasing.md) does
not repair, and because its warning is easy to mistake for a real fault.

## What you see

Once a second tab joins, the server logs this, and logs it again on every
further join:

```
WARN bevy_asset::server::loaders: Duplicate AssetLoader registered for Asset
type `bevy_shader::shader::Shader` with extensions
`["spv", "wgsl", "vert", "frag", "comp", "wesl"]`. Loader must be specified in
a .meta file in order to load assets of this type with these extensions.
```

Nothing is broken at that moment. The warning is a symptom, not a failure.

## Why it happens

Each late joiner gets its render sub-app from a throwaway `App` that runs the
identical plugin stack against the *real* app's `AssetServer`
(`harvest_render_app` in `bevy_pbr_game/src/main.rs`). That stack registers its
asset loaders as it builds, and `App::register_asset_loader` /
`init_asset_loader` go straight to whichever `AssetServer` is in the world,
which for a throwaway is the shared one.

`AssetServer::register_loader` forwards to `AssetLoaders::push`, which appends
unconditionally:

* a new element in `loaders: Vec<MaybeAssetLoader>`, holding an
  `Arc<dyn ErasedAssetLoader>`;
* the new index appended to `type_id_to_loaders[A]`;
* the new index appended to `extension_to_loaders[ext]`, once per extension the
  loader claims.

Only `type_path_to_loader` is a plain overwrite. Registering the same loader
type a second time takes the `is_new` branch, because that branch is skipped
only for a loader that was *preregistered* and is now being filled in. So every
join appends another full set, and nothing ever removes one: `AssetLoaders` has
no remove or clear, and `AssetServer` exposes no way to reach it.

The throwaway app is dropped seconds later. Its loaders are not.

## What it costs

Growth is one set of loaders per join, for the life of the process. This stack
registers only a handful of real loaders, chiefly `ShaderLoader` and the image
loaders, so a long-running server accumulates slowly rather than alarmingly.
What it retains is the `Arc<dyn ErasedAssetLoader>` for each, plus a `usize` in
one list per claimed extension.

Loader *resolution* is unaffected. `AssetLoaders::find` prefers the most
recently registered candidate at every step (`indices.last()`, and
`.iter().rev().find(..)` when narrowing by asset type), so a load picks the
newest duplicate, which is an identical loader registered by an identical
plugin stack.

There is one behavioural edge. `find` has a fast path for the case where an
asset type has exactly one loader; after the first join no asset type does, so
type-directed loads fall through to extension matching. That resolves normally
for anything with a known extension. A path whose extension matches nothing
reaches the final fallback, which still returns the newest candidate but logs
`Multiple AssetLoaders found for Asset: ..; Path: ..;` each time. Watch for that
line if load-time warnings ever start repeating.

## Why it is not fixed

The id-aliasing repair works because `AssetServer::register_asset` is public, so
the real world's handle providers can simply be registered again. There is no
equivalent for loaders in any visibility: `AssetLoaders` is `pub(crate)`, its
fields are private, and it has no removal path.

Fixing it properly means one of:

* vendoring `bevy_asset` the way `bevy_render` is already vendored through
  `[patch.crates-io]`, and making `push` reuse the existing index when the same
  loader type path is registered again; or
* building the throwaway app against its own `AssetServer`, which is what
  creates the whole problem class, but which also breaks the handle identity
  that sharing the server exists to provide. See the aliasing doc for why the
  render sub-app needs the real server's ids.

Neither is worth doing for the leak alone. Revisit if `bevy_asset` is vendored
for another reason, or if a future stack registers loaders heavily enough that
the growth stops being negligible.

## If you are checking whether it got worse

The count of `Duplicate AssetLoader` lines in a log is the number of joins that
re-registered loaders, one line per loader whose extensions collide. Per the
lifetime-measurement recipe in `client-lifetime-leak.md`, run the churn under
`MALLOC_ARENA_MAX=1` so RSS is readable; this leak is far too small to see
against per-thread arena noise otherwise.
