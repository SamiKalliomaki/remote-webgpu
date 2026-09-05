//! Every player who joins builds a throwaway `App` -- the same plugin stack the
//! real app was built from -- against the real app's `AssetServer`, so that
//! stack's asset loaders are registered on the one long-lived server again on
//! every join.  Upstream `bevy_asset` appends them unconditionally, with no way
//! at any visibility to remove them again; the vendored fork in `rust/bevy_asset`
//! makes `AssetLoaders::push` reuse the slot of a loader type it already holds.
//!
//! This exercises the real plugin stack, so it covers the loaders that stack
//! actually brings (`ShaderLoader` and the image loaders above all) rather than
//! a stand-in.  See `docs/shared-asset-server-loader-growth.md`.

use std::sync::Arc;

use bevy::app::{PluginGroup, PluginsState};
use bevy::asset::{AssetPlugin, AssetServer};
use bevy::prelude::*;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::RenderCreation;
use bevy::render::RenderPlugin;
use bevy::window::ExitCondition;

/// A device on wgpu's no-op backend: real objects in the C library, but no
/// browser and no socket.  Plugin build only creates GPU resources, which is
/// all this backend supports (see `docs/noop-backend.md`).
fn noop_render_creation() -> RenderCreation {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true },
            ..default()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&default()))
        .expect("the noop backend always produces an adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features(),
        required_limits: adapter.limits(),
        ..default()
    }))
    .expect("the noop backend always produces a device");
    let adapter_info = adapter.get_info();
    RenderCreation::manual(
        RenderDevice::from(device),
        RenderQueue(Arc::new(WgpuWrapper::new(queue))),
        RenderAdapterInfo(WgpuWrapper::new(adapter_info)),
        RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
        RenderInstance(Arc::new(WgpuWrapper::new(instance))),
    )
}

/// `main.rs`'s `build_app`, minus the bits that need a client: the same
/// `DefaultPlugins` stack, and the same "share the real server" plugin ordering
/// for a throwaway.
fn build_app(shared_assets: Option<AssetServer>) -> App {
    struct SharedAssetsPlugin(AssetServer);
    impl Plugin for SharedAssetsPlugin {
        fn build(&self, app: &mut App) {
            app.insert_resource(self.0.clone());
        }
    }

    let mut app = App::new();
    let mut plugins = DefaultPlugins
        .build()
        .set(bevy::window::WindowPlugin {
            primary_window: None,
            primary_cursor_options: None,
            exit_condition: ExitCondition::DontExit,
            close_when_requested: false,
        })
        .set(RenderPlugin {
            render_creation: noop_render_creation(),
            synchronous_pipeline_compilation: true,
            ..default()
        })
        .disable::<PipelinedRenderingPlugin>()
        .disable::<bevy::log::LogPlugin>();
    if let Some(server) = shared_assets {
        plugins = plugins.add_after::<AssetPlugin>(SharedAssetsPlugin(server));
    }
    app.add_plugins(plugins);
    while app.plugins_state() == PluginsState::Adding {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    app.finish();
    app.cleanup();
    app
}

#[test]
fn joining_does_not_grow_the_real_stacks_loader_table() {
    let real = build_app(None);
    let server = real.world().resource::<AssetServer>().clone();
    let before = server.registered_loader_count();
    assert!(before > 0, "the plugin stack registered no loaders at all");

    for join in 1..=3 {
        drop(build_app(Some(server.clone())));
        assert_eq!(
            server.registered_loader_count(),
            before,
            "join {join} left another set of asset loaders on the shared server"
        );
        // Reusing a slot must not lose the loader that lives in it: a harvested
        // render world loads its shaders through this same server.
        for extension in ["wgsl", "png"] {
            let loader = pollster::block_on(server.get_asset_loader_with_extension(extension));
            assert!(
                loader.is_ok(),
                "after join {join} the shared server no longer resolves a .{extension} loader"
            );
        }
    }
}
