//! Repairing the shared [`AssetServer`] after a throwaway app is built on it.
//!
//! Late joiners get their render sub-app from a throwaway [`App`] that runs the
//! identical plugin stack while sharing the real app's [`AssetServer`], so that
//! embedded shaders resolve to the same handles in both.  Sharing the server has
//! a side effect that is easy to miss: every `init_asset::<A>()` in that stack
//! calls [`AssetServer::register_asset`], which *overwrites* the server's handle
//! provider for `A` with the throwaway's brand new, zero-based
//! `AssetIndexAllocator`.
//!
//! The throwaway is dropped moments later, but the shared server keeps pointing
//! at its dead allocator.  From then on the real world has two independent
//! allocators minting indices into one `Assets<A>` storage: `Assets::add` uses
//! the real one, while anything that goes through the server (a harvested render
//! world's `RenderStartup` loading an embedded shader, say) uses the dead one and
//! starts again from index 0.  The two eventually hand out the same
//! `AssetId<A>`, and the second asset silently overwrites the first.
//!
//! For shaders that is fatal rather than cosmetic.  `ShaderCache` keys its
//! `import_path -> AssetId` table and its `AssetId -> Shader` table separately,
//! so an aliased id makes it hand naga_oil the wrong source for an import:
//! `#define` in a composable module, `#{MATERIAL_BIND_GROUP}` left
//! unsubstituted, `required import 'bevy_core_pipeline::fullscreen_vertex_shader'
//! not found`.  Pipelines then fail to build, the render error handler quits the
//! app, and the server dies after hours of uptime.
//!
//! [`restore_asset_registrations`] puts the real world's providers back.

use bevy::asset::{AssetServer, Assets};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use std::collections::BTreeSet;

/// Every asset type this app's plugin stack initializes.
///
/// `init_asset::<A>()` is what clobbers the shared server, so this list must
/// name every `A` the stack initializes.  [`audit_asset_registrations`] checks
/// it against the type registry at startup and logs anything missing, so a
/// future plugin that adds an asset type is caught instead of silently
/// corrupting that type's ids.
macro_rules! for_each_shared_asset_type {
    ($apply:ident) => {
        $apply!(());
        $apply!(bevy::asset::LoadedFolder);
        $apply!(bevy::asset::LoadedUntypedAsset);
        $apply!(bevy::image::Image);
        $apply!(bevy::image::TextureAtlasLayout);
        $apply!(bevy::mesh::Mesh);
        $apply!(bevy::mesh::skinning::SkinnedMeshInverseBindposes);
        $apply!(bevy::light::atmosphere::ScatteringMedium);
        $apply!(bevy::shader::Shader);
        $apply!(bevy::render::storage::ShaderBuffer);
        $apply!(bevy::pbr::StandardMaterial);
    };
}

/// Re-registers the real world's [`Assets<A>`] collections with the shared
/// [`AssetServer`], undoing the handle providers a throwaway app installed.
///
/// Call this every time a throwaway app has been built on the shared server.
pub fn restore_asset_registrations(world: &World) {
    let server = world.resource::<AssetServer>();
    macro_rules! restore {
        ($asset:ty) => {
            if let Some(assets) = world.get_resource::<Assets<$asset>>() {
                server.register_asset::<$asset>(assets);
            }
        };
    }
    for_each_shared_asset_type!(restore);
}

/// The `Handle<..>` type paths [`restore_asset_registrations`] covers.
fn restored_handle_type_paths() -> BTreeSet<&'static str> {
    let mut paths = BTreeSet::new();
    macro_rules! remember {
        ($asset:ty) => {
            paths.insert(<Handle<$asset> as TypePath>::type_path());
        };
    }
    for_each_shared_asset_type!(remember);
    paths
}

/// Asset types the app registered that [`restore_asset_registrations`] misses.
///
/// `init_asset::<A>()` also registers `Handle<A>` for reflection, so the type
/// registry is a faithful census of the asset types in the plugin stack.
pub fn unrestored_asset_types(world: &World) -> Vec<String> {
    let restored = restored_handle_type_paths();
    let prefix = {
        let sample = <Handle<()> as TypePath>::type_path();
        &sample[..=sample.find('<').expect("Handle is a generic type")]
    };
    let registry = world.resource::<AppTypeRegistry>().read();
    registry
        .iter()
        .map(|registration| registration.type_info().type_path())
        .filter(|path| path.starts_with(prefix) && !restored.contains(path))
        .map(str::to_owned)
        .collect()
}

/// Logs any asset type that a throwaway app would clobber and this module would
/// not repair.  Cheap, and runs once at startup.
pub fn audit_asset_registrations(world: &World) {
    let unrestored = unrestored_asset_types(world);
    if !unrestored.is_empty() {
        error!(
            "these asset types are not restored after a throwaway app is built, so their \
             asset ids will alias once a second player joins; add them to \
             `for_each_shared_asset_type!`: {}",
            unrestored.join(", ")
        );
    }
}
