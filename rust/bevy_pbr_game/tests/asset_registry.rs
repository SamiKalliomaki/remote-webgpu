//! Building a throwaway app on a *shared* `AssetServer` hijacks that server's
//! per-asset-type handle provider, leaving the long-lived world with two
//! independent index allocators feeding one `Assets<A>` storage.  The second
//! allocator restarts at index 0 and hands out ids that alias live assets.

use bevy::asset::io::Reader;
use bevy::asset::{AssetApp, AssetLoader, AssetPlugin, AssetServer, Assets, LoadContext};
use bevy::prelude::*;
use bevy::shader::Shader;

fn wgsl(name: &str) -> Shader {
    Shader::from_wgsl(format!("// {name}\n"), name.to_string())
}

/// Mirrors `bevy_pbr_game`'s `build_app`: `AssetPlugin` first, then (for a
/// throwaway) the real server is dropped in, then the asset types initialize.
fn build(shared: Option<AssetServer>) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default());
    if let Some(server) = shared {
        app.insert_resource(server);
    }
    app.init_asset::<Shader>();
    // Loaders come from the plugin stack too, and a throwaway registers the
    // same ones the real app already did.
    app.register_asset_loader(NoteLoader);
    app
}

/// Stands in for the real stack's `ShaderLoader`: one loader type, registered
/// once per app built, claiming a fixed set of extensions.
#[derive(TypePath)]
struct NoteLoader;

impl AssetLoader for NoteLoader {
    type Asset = Shader;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        _: &mut dyn Reader,
        _: &Self::Settings,
        _: &mut LoadContext<'_>,
    ) -> Result<Shader, Self::Error> {
        Ok(wgsl("loaded"))
    }

    fn extensions(&self) -> &[&str] {
        &["wgsl"]
    }
}

fn live_shaders(app: &mut App, count: usize) -> Vec<Handle<Shader>> {
    (0..count)
        .map(|i| {
            app.world_mut()
                .resource_mut::<Assets<Shader>>()
                .add(wgsl(&format!("live{i}.wgsl")))
        })
        .collect()
}

/// A player joins: a throwaway app is built on the shared server, harvested,
/// and dropped.
fn simulate_join(real: &App) {
    let server = real.world().resource::<AssetServer>().clone();
    drop(build(Some(server)));
}

#[test]
fn throwaway_app_aliases_live_shader_ids_without_the_repair() {
    let mut real = build(None);
    let live = live_shaders(&mut real, 8);

    simulate_join(&real);

    // Afterwards the real world keeps loading shaders through that server, as
    // each harvested render world's `RenderStartup` does.
    let loaded: Handle<Shader> = real.world().resource::<AssetServer>().load("new.wgsl");

    assert!(
        live.iter().any(|existing| existing.id() == loaded.id()),
        "expected the unrepaired server to alias a live shader id; if this now \
         passes, bevy_asset changed and `shared_assets` may be obsolete"
    );
}

#[test]
fn restoring_registrations_keeps_shader_ids_unique() {
    let mut real = build(None);
    let mut live = live_shaders(&mut real, 8);

    for round in 0..4 {
        simulate_join(&real);
        // What `harvest_render_app` now does once the throwaway is dropped.
        bevy_pbr_game::shared_assets::restore_asset_registrations(real.world());

        // A distinct path each round, so this is a fresh id rather than a
        // cache hit on the previous round's.
        let loaded: Handle<Shader> = real
            .world()
            .resource::<AssetServer>()
            .load(format!("joined{round}.wgsl"));
        for existing in &live {
            assert_ne!(
                loaded.id(),
                existing.id(),
                "server minted an id that aliases a live shader"
            );
        }
        live.push(loaded);
        live.extend(live_shaders(&mut real, 2));
    }
}

#[derive(Asset, TypePath)]
struct UnlistedAsset;

#[test]
fn the_audit_names_an_asset_type_the_repair_would_miss() {
    let mut real = build(None);
    assert!(
        bevy_pbr_game::shared_assets::unrestored_asset_types(real.world()).is_empty(),
        "a listed asset type was reported as unrestored"
    );

    // A plugin added later brings its own asset type along.
    real.init_asset::<UnlistedAsset>();

    let unrestored = bevy_pbr_game::shared_assets::unrestored_asset_types(real.world());
    assert!(
        unrestored.iter().any(|path| path.contains("UnlistedAsset")),
        "the audit missed an asset type outside `for_each_shared_asset_type!`: {unrestored:?}"
    );
}

#[test]
fn joining_does_not_grow_the_shared_servers_loader_table() {
    let real = build(None);
    let before = real
        .world()
        .resource::<AssetServer>()
        .registered_loader_count();
    assert!(before > 0, "the app registered no loaders at all");

    for _ in 0..4 {
        simulate_join(&real);
        assert_eq!(
            real.world()
                .resource::<AssetServer>()
                .registered_loader_count(),
            before,
            "a join left another set of asset loaders on the shared server; \
             `AssetLoaders::push` in the vendored bevy_asset is supposed to \
             reuse the slot of a loader type it already holds. See \
             docs/shared-asset-server-loader-growth.md"
        );
    }
}
