# Aliased asset ids from the shared `AssetServer` (fixed 2026-09-05)

## Symptom

After hours of uptime and a few tab joins, `bevy_pbr_game` stopped rendering for
everyone. The logs show naga_oil rejecting shaders it had compiled happily since
startup:

```
error: expected expression, found "#"
   ┌─ embedded://bevy_pbr/render/pbr_bindings.wgsl:45:8
45 │ @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: ...

error: #define statements are only allowed at the start of the top-level shaders
  ┌─ embedded://bevy_core_pipeline/tonemapping/tonemapping.wgsl:1:1

error: required import 'bevy_core_pipeline::fullscreen_vertex_shader' not found
error: no definition in scope for identifier: `result`   (downsample.wgsl)
```

Every pipeline that depended on those shaders then failed to build, the browser
reported invalid pipelines and command buffers, and bevy's render error handler
logged `Quitting the application due to Validation RenderError` once per frame
forever.

## Root cause

Two different `Shader` assets had ended up sharing one `AssetId<Shader>`.

`ShaderCache` keeps `import_path -> AssetId` and `AssetId -> Shader` in separate
tables. When a second shader overwrites the first at the same id, the first
table still points an import at that id while the second now yields the wrong
source, so the composer is handed the wrong module. That is precisely the shape
of all four errors above: `tonemapping.wgsl` (a top-level shader) being added as
a composable module, `fullscreen_vertex_shader` never registering under its own
name, and shader defs missing from the sources that needed them.

The aliasing came from how late joiners get their render sub-app. Each join
builds a throwaway `App` on the *real* app's `AssetServer`, so embedded shaders
resolve to the same handles in both. But every `init_asset::<A>()` in that
plugin stack calls `AssetServer::register_asset`, which **overwrites** the
server's handle provider for `A`:

```rust
pub(crate) fn register_handle_provider(&self, handle_provider: AssetHandleProvider) {
    self.write_infos().handle_providers.insert(handle_provider.type_id, handle_provider);
}
```

The throwaway is dropped seconds later, but the shared server keeps pointing at
its dead, zero-based `AssetIndexAllocator`. From then on the real world has two
independent allocators minting indices into one `Assets<A>` storage:
`Assets::add` uses the real one, while anything going through the server (a
harvested render world's `RenderStartup` loading an embedded shader, say) uses
the dead one and starts again from index 0. Eventually both hand out the same
`AssetId<A>` and the second asset silently overwrites the first.

This affects every asset type in the stack, not just `Shader`; shaders are
simply where the corruption turns into a hard failure instead of a wrong-looking
mesh or texture.

## Fix

`bevy_pbr_game/src/shared_assets.rs`:

* `restore_asset_registrations` re-registers the real world's `Assets<A>` with
  the shared server. `harvest_render_app` calls it as soon as the throwaway app
  has been dropped.
* `for_each_shared_asset_type!` lists the asset types the stack initializes.
* `audit_asset_registrations` runs once at startup and logs an error naming any
  asset type in the type registry that the list misses, so a plugin added later
  is caught rather than silently corrupting that type's ids.

Regression tests: `cargo test -p bevy_pbr_game --test asset_registry`. One test
asserts the unrepaired path still aliases, so the suite notices if `bevy_asset`
changes and the repair becomes unnecessary.

## Related change

Bevy's default `RenderErrorHandler` asks the whole app to quit on any wgpu
error. Here each browser tab is a separate device, so one tab's error must not
take the game down for everyone; the runner also never reads `AppExit`, which is
why the default only produced endless `Quitting the application` lines while the
game kept running. `stop_this_players_rendering` in `main.rs` stops just the
offending render world and reports it once.

## Not fixed

`AssetServer::register_loader` appends unconditionally, so each join adds
another full set of asset loaders to the shared server; this is the source of
the `Duplicate AssetLoader registered for Asset type Shader` warning. The
loaders are identical, so resolution still picks an equivalent one, but the list
grows for the life of the process and there is no API at any visibility that can
remove entries. Written up in
[`shared-asset-server-loader-growth.md`](shared-asset-server-loader-growth.md).
