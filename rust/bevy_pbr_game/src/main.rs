//! One Bevy game, many players, each rendering with `bevy_pbr`'s standard
//! mesh pipeline on their own remote GPU.
//!
//! Run with `cargo run -p bevy_pbr_game`, then open <http://localhost:8000>
//! in as many tabs as you like: the game serves the web client
//! (`example_client/`) on the same port the client connects to.  The first tab starts the game;
//! every further tab gets its own character and its own camera into the same
//! world.  Move with WASD or the arrow keys, dash with space.
//!
//! How it works: the main [`App`] holds the one shared world and one render
//! sub-app per connected player.  Each render sub-app is a complete
//! `bevy_render` render world built against that player's own
//! adapter/device (in this backend every browser tab is a separate GPU), so
//! all sub-apps run identical systems while owning fully independent GPU
//! state.  Later joiners get their render sub-app by building the identical
//! plugin stack in a throwaway [`App`] that shares the real app's
//! [`AssetServer`] (so shader handles line up), then harvesting the built
//! sub-app into the running game.  The forked `bevy_render` in this
//! workspace fans entity-sync records out to every render world; see
//! `bevy_render/src/sync_world.rs`.
//!
//! For headless testing, `--frames N --screenshot PATH` captures player 0's
//! view after N frames via bevy's own screenshot readback and then exits.

mod game;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::app::{AppLabel, PluginGroup, PluginsState};
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::camera::RenderTarget;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::error_handler::{RenderError, RenderErrorHandler, RenderErrorPolicy};
use bevy::render::settings::RenderCreation;
use bevy::render::sync_world::{register_render_world, unregister_render_world, SyncQueueIndex};
use bevy::render::view::window::screenshot::{save_to_disk, share_screenshot_channel, Screenshot};
use bevy::render::view::window::OwnedWindow;
use bevy::render::RenderApp;
use bevy::tasks::ComputeTaskPool;
use bevy::render::RenderPlugin;
use bevy::window::{ExitCondition, PresentMode, RawHandleWrapper, WindowRef, WindowWrapper};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WebDisplayHandle, WebWindowHandle, WindowHandle,
};
use remote_wgpu_runtime::{runtime, Client, ClientEvent};

use game::{spawn_player, ArenaPlugin, Button, Controls, FollowPlayer};
use bevy_pbr_game::shared_assets::{audit_asset_registrations, restore_asset_registrations};

/// The render sub-app label for players that join after the first one.
#[derive(AppLabel, Clone, Copy, Hash, PartialEq, Eq, Debug)]
struct PlayerRenderApp(u64);

/// The fake window behind `RawHandleWrapper`: the remote backend has no OS
/// windows, so the client id rides in a `WebWindowHandle` and the wgpu shim
/// decodes it again in `create_surface_unsafe`.
struct RemoteWindowHandle {
    client_id: u32,
}

impl HasWindowHandle for RemoteWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Web(WebWindowHandle::new(self.client_id));
        // SAFETY: there is no underlying window object to outlive.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for RemoteWindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Web(WebDisplayHandle::new());
        // SAFETY: as above.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

/// Everything the runner tracks about one connected player.
struct PlayerView {
    client: Arc<Client>,
    label: bevy::app::InternedAppLabel,
    sync_queue: usize,
    window: Entity,
    avatar: Entity,
    camera: Entity,
    held: HashSet<Button>,
    slot: usize,
    gone: bool,
}

/// Copies the real app's `AssetServer` into a throwaway app, right after
/// `AssetPlugin` created its own.  Sharing the server means embedded assets
/// (bevy's internal shaders above all) resolve to the same handles in every
/// app, so a harvested render sub-app finds its shaders in the real world.
struct SharedAssetsPlugin {
    server: AssetServer,
}

impl Plugin for SharedAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.server.clone());
    }
}

/// Marks a render world that has already reported a render error.
#[derive(Resource)]
struct RenderErrorReported;

/// What to do when wgpu reports an error against one player's device.
///
/// Bevy's default handler asks the whole app to quit for any render error,
/// which suits a single-window game but not a server where every browser tab is
/// a separate device: one tab's bad frame would take the game down for everyone
/// else.  It is also misleading here, because this app's runner never reads
/// `AppExit`, so the default just logs "Quitting the application" once per frame
/// forever while the game keeps running.
///
/// Stop the offending render world instead, and say so once.  The runner already
/// skips a player whose frames stop being acknowledged, and cleans the player up
/// when their tab goes away.
fn stop_this_players_rendering(
    error: &RenderError,
    _main_world: &mut World,
    render_world: &mut World,
) -> RenderErrorPolicy {
    if !render_world.contains_resource::<RenderErrorReported>() {
        render_world.insert_resource(RenderErrorReported);
        error!(
            "stopping this player's rendering after a {:?} render error: {}",
            error.ty, error.description
        );
    }
    RenderErrorPolicy::StopRendering
}

/// The GPU half of one player: their tab's instance, adapter and device.
fn create_gpu(client: &Arc<Client>) -> RenderCreation {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let surface = instance
        .create_surface(client.clone())
        .expect("failed to create the client's surface");
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(&surface),
    }))
    .expect("failed to get the client's adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: adapter.features(),
        required_limits: adapter.limits(),
        experimental_features: Default::default(),
        memory_hints: Default::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("failed to get the client's device");
    let adapter_info = adapter.get_info();
    RenderCreation::manual(
        RenderDevice::from(device),
        RenderQueue(Arc::new(WgpuWrapper::new(queue))),
        RenderAdapterInfo(WgpuWrapper::new(adapter_info)),
        RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
        RenderInstance(Arc::new(WgpuWrapper::new(instance))),
    )
}

/// The plugin stack every app (real or throwaway) is built from.  It must be
/// identical between them so a harvested render sub-app matches the real
/// main world.
fn build_app(render_creation: RenderCreation, shared_assets: Option<AssetServer>) -> App {
    let mut app = App::new();
    let mut plugins = DefaultPlugins
        .build()
        // Windows are created by the runner when clients connect.
        .set(bevy::window::WindowPlugin {
            primary_window: None,
            primary_cursor_options: None,
            exit_condition: ExitCondition::DontExit,
            close_when_requested: false,
        })
        .set(RenderPlugin {
            render_creation,
            synchronous_pipeline_compilation: true,
            ..default()
        })
        // One render world per player is paced by the runner itself.
        .disable::<PipelinedRenderingPlugin>();
    if let Some(server) = shared_assets {
        // Throwaway app: share the real asset server, and don't fight over
        // the global logger.
        plugins = plugins
            .add_after::<AssetPlugin>(SharedAssetsPlugin { server })
            .disable::<LogPlugin>();
    }
    app.add_plugins(plugins);
    app.insert_resource(RenderErrorHandler(stop_this_players_rendering));
    app.get_sub_app_mut(RenderApp)
        .expect("RenderPlugin built the render sub-app")
        .add_systems(
            bevy::render::Render,
            // After ExtractCommands: extraction's inserts are deferred and
            // only land there, not during `SubApp::extract` itself.
            drop_unowned_view_clusters
                .after(bevy::render::RenderSystems::ExtractCommands)
                .before(bevy::render::RenderSystems::PrepareResources),
        );
    app
}

/// bevy_pbr's cluster extraction copies cluster configs for *every* active
/// camera in the main world; it knows nothing about this workspace's
/// one-render-world-per-player split.  Left alone, every render world then
/// re-uploads ~380 KiB of (zeroed) cluster buffers per camera per frame, so
/// each client's traffic grows linearly with the player count.  The fork
/// already strips `ExtractedCamera` from views a world doesn't own; dropping
/// the cluster components alongside it (after extract, before the world
/// updates) keeps `prepare_clusters_for_gpu_clustering` to the world's own
/// view.
fn drop_unowned_view_clusters(world: &mut World) {
    let unowned: Vec<Entity> = world
        .query_filtered::<Entity, (
            With<bevy::pbr::ExtractedClusterConfig>,
            Without<bevy::render::camera::ExtractedCamera>,
        )>()
        .iter(world)
        .collect();
    for entity in unowned {
        world.entity_mut(entity).remove::<(
            bevy::pbr::ExtractedClusterConfig,
            bevy::pbr::ExtractedClusterableObjects,
        )>();
    }
}

/// Spawns a window entity + camera + character for a connected client, and
/// hands back the bookkeeping for the runner.
fn attach_view(
    app: &mut App,
    client: Arc<Client>,
    label: bevy::app::InternedAppLabel,
    sync_queue: usize,
    slot: usize,
) -> PlayerView {
    let (width, height) = client.canvas_size();
    let (width, height) = (width.max(1), height.max(1));

    let mut window = Window {
        present_mode: PresentMode::Fifo,
        ..default()
    };
    window.resolution.set_scale_factor(1.0);
    window.resolution.set_physical_resolution(width, height);

    let wrapper = WindowWrapper::new(RemoteWindowHandle { client_id: client.id() as u32 });
    let raw_handle =
        RawHandleWrapper::new(&wrapper).expect("remote window handles are always available");

    let window_entity = app.world_mut().spawn((window, raw_handle)).id();
    let avatar = spawn_player(app.world_mut(), slot);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Entity(window_entity)),
            AmbientLight {
                color: Color::WHITE,
                brightness: 250.0,
                ..default()
            },
            FollowPlayer(avatar),
            Transform::from_xyz(0.0, 13.0, 15.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();

    let render_world = app
        .get_sub_app_mut(label)
        .expect("render sub-app exists")
        .world_mut();
    render_world.insert_resource(OwnedWindow(window_entity));
    render_world.insert_resource(SyncQueueIndex(sync_queue));

    info!(
        "player {slot} joined (client {} from {})",
        client.id(),
        client.addr()
    );
    PlayerView {
        client,
        label,
        sync_queue,
        window: window_entity,
        avatar,
        camera,
        held: HashSet::new(),
        slot,
        gone: false,
    }
}

/// Builds a complete render sub-app for a client that joined mid-game, by
/// running the identical plugin stack in a throwaway app and harvesting the
/// result.
fn harvest_render_app(app: &mut App, client: &Arc<Client>) -> bevy::app::SubApp {
    let shared_server = app.world().resource::<AssetServer>().clone();
    let mut throwaway = build_app(create_gpu(client), Some(shared_server));
    // Drive the plugin lifecycle to completion without ever updating: the
    // throwaway main world must not run (the real one owns the game).
    while throwaway.plugins_state() == PluginsState::Adding {
        std::thread::sleep(Duration::from_millis(1));
    }
    throwaway.finish();
    throwaway.cleanup();
    let sub_app = throwaway
        .remove_sub_app(RenderApp)
        .expect("the throwaway app built a render sub-app");
    drop(throwaway);
    // Building on the shared `AssetServer` pointed its handle providers at the
    // throwaway's `Assets<A>` collections, which have just been dropped.  Left
    // alone, the server would keep minting asset ids from those dead, zero-based
    // allocators and alias live assets; see `shared_assets`.
    restore_asset_registrations(app.world());
    sub_app
}

fn translate_key(payload: &[u8]) -> Option<Button> {
    let text = std::str::from_utf8(payload).ok()?;
    let code = text.lines().next()?;
    match code {
        "KeyW" | "ArrowUp" => Some(Button::Up),
        "KeyS" | "ArrowDown" => Some(Button::Down),
        "KeyA" | "ArrowLeft" => Some(Button::Left),
        "KeyD" | "ArrowRight" => Some(Button::Right),
        "Space" | "ShiftLeft" => Some(Button::Dash),
        _ => None,
    }
}

struct RunSettings {
    frame_limit: Option<u64>,
    screenshot: Option<PathBuf>,
}

fn run_settings() -> RunSettings {
    let mut frame_limit = None;
    let mut screenshot = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => frame_limit = args.next().and_then(|v| v.parse().ok()),
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            other => panic!("unknown argument: {other}"),
        }
    }
    if screenshot.is_some() && frame_limit.is_none() {
        frame_limit = Some(300);
    }
    RunSettings { frame_limit, screenshot }
}

/// The shared simulation never steps faster than this; render worlds are
/// additionally paced by their own tab's vsync acknowledgements.
const MIN_STEP: Duration = Duration::from_millis(6);

fn runner(mut app: App) -> AppExit {
    // A custom runner owns the plugin lifecycle: finish building before
    // anything runs.
    while app.plugins_state() == PluginsState::Adding {
        std::thread::sleep(Duration::from_millis(1));
    }
    app.finish();
    app.cleanup();
    // Catches an asset type added later that `shared_assets` does not repair.
    audit_asset_registrations(app.world());

    let settings = run_settings();
    let rt = runtime();
    let mut players: Vec<PlayerView> = Vec::new();
    let mut next_slot = 1usize;
    let mut frames: u64 = 0;
    let mut screenshot_taken = false;
    let mut shutdown_at: Option<u64> = None;

    // Run Startup once so the arena and the shared game assets exist
    // before the first character spawns.
    app.sub_apps_mut().main.run_default_schedule();
    let mut last_step = std::time::Instant::now();

    // The app was built against the first client; adopt it as player 0.
    let first = app.world_mut().remove_resource::<FirstClient>().unwrap().0;
    players.push(attach_view(&mut app, first, RenderApp.intern(), 0, 0));

    loop {
        // New tabs become new players.
        while let Some(client) = rt.try_next_client() {
            let mut sub_app = harvest_render_app(&mut app, &client);
            // Change detection compares ticks across worlds during
            // extraction: a freshly built render world's tick counter sits
            // near zero while the long-running main world's is far ahead,
            // making every main-world change look ancient.  Spin the new
            // world's counter up to match.
            let main_tick = app.world_mut().change_tick();
            while sub_app.world_mut().change_tick().get() < main_tick.get() {
                sub_app.world_mut().increment_change_tick();
            }
            // The harvested world's screenshot channel still points at the
            // throwaway app; re-point it at the real app's receiver.
            share_screenshot_channel(
                app.get_sub_app(RenderApp).unwrap().world(),
                sub_app.world_mut(),
            );
            // MipGenerationPlugin bakes its format-specialized shaders with
            // `Assets::add`, whose runtime handles are only meaningful in the
            // world that allocated them — the throwaway's, which is about to
            // be dropped.  Point the harvested world at the real app's
            // handles instead (the shader contents arrive through the asset
            // event replay below).
            let downsample_shaders = app
                .world()
                .resource::<bevy::core_pipeline::mip_generation::DownsampleShaders>()
                .clone();
            sub_app.world_mut().insert_resource(downsample_shaders);
            let label = PlayerRenderApp(client.id()).intern();
            app.insert_sub_app(label, sub_app);
            let sync_queue = register_render_world(app.world_mut());
            replay_asset_events(app.world_mut());
            players.push(attach_view(&mut app, client, label, sync_queue, next_slot));
            next_slot += 1;
        }

        // Departures and input.
        for player in &mut players {
            if player.gone {
                continue;
            }
            if player.client.is_disconnected() {
                info!("player {} left", player.slot);
                player.gone = true;
                app.world_mut().entity_mut(player.window).despawn();
                app.world_mut().entity_mut(player.camera).despawn();
                app.world_mut().entity_mut(player.avatar).despawn();
                if player.label != RenderApp.intern() {
                    app.remove_sub_app(player.label);
                    unregister_render_world(app.world_mut(), player.sync_queue);
                }
                continue;
            }
            let mut resized = None;
            while let Some(event) = player.client.poll_event() {
                match event {
                    ClientEvent::Resize { width, height } => resized = Some((width, height)),
                    ClientEvent::User { name, payload } => match name.as_str() {
                        "keydown" => {
                            if let Some(button) = translate_key(&payload) {
                                player.held.insert(button);
                            }
                        }
                        "keyup" => {
                            if let Some(button) = translate_key(&payload) {
                                player.held.remove(&button);
                            }
                        }
                        _ => {}
                    },
                    ClientEvent::Disconnected => {}
                }
            }
            if let Some((width, height)) = resized {
                if let Some(mut window) = app.world_mut().get_mut::<Window>(player.window) {
                    window.resolution.set_physical_resolution(width.max(1), height.max(1));
                }
            }
            let held = player.held.clone();
            if let Some(mut controls) = app.world_mut().get_mut::<Controls>(player.avatar) {
                controls.held = held;
            }
        }
        players.retain(|player| !player.gone);

        // Trigger the verification screenshot once the frame limit is hit,
        // then give the readback a little time to complete.
        if let Some(limit) = settings.frame_limit {
            if frames >= limit && !screenshot_taken {
                screenshot_taken = true;
                if let Some(path) = &settings.screenshot {
                    // One capture per player, so every render world's output
                    // can be inspected: "shot.png", "shot.1.png", ...
                    for player in &players {
                        let mut path = path.clone();
                        if player.slot > 0 {
                            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                            let ext = path.extension().unwrap().to_string_lossy().to_string();
                            path.set_file_name(format!("{stem}.{}.{ext}", player.slot));
                        }
                        app.world_mut()
                            .spawn(Screenshot::window(player.window))
                            .observe(save_to_disk(path));
                    }
                    shutdown_at = Some(frames + 240);
                } else {
                    return AppExit::Success;
                }
            }
            if shutdown_at.is_some_and(|at| frames >= at) {
                return AppExit::Success;
            }
        }

        // The runtime's wait wakes on every websocket message, far more
        // often than a frame is worth; hold the full step to MIN_STEP.
        let since_last = last_step.elapsed();
        if since_last < MIN_STEP {
            rt.wait(MIN_STEP - since_last);
            continue;
        }
        last_step = std::time::Instant::now();

        // One shared simulation step, then one render step per player whose
        // browser has acknowledged the previous frame.
        let sub_apps = app.sub_apps_mut();
        sub_apps.main.run_default_schedule();

        // The render worlds ready for a frame right now.
        let ready: HashSet<bevy::app::InternedAppLabel> = players
            .iter()
            .filter(|p| {
                let vsync_pending = p.client.vsync_frames_pending() >= 3;
                !vsync_pending && !p.client.is_disconnected()
            })
            .map(|p| p.label)
            .collect();
        let mut ready_apps: Vec<&mut bevy::app::SubApp> = sub_apps
            .sub_apps
            .iter_mut()
            .filter(|(label, _)| ready.contains(*label))
            .map(|(_, sub_app)| sub_app)
            .collect();

        // Extract the main world into each of them, one at a time:
        // extraction re-points the main world's `RenderEntity` rows at the
        // extracting world, so extracts cannot overlap.
        for sub_app in &mut ready_apps {
            sub_app.extract(sub_apps.main.world_mut());
        }

        // Render all views in parallel on bevy's compute pool; each render
        // world drives only its own client's GPU.
        ComputeTaskPool::get().scope(|scope| {
            for sub_app in ready_apps {
                scope.spawn(async move { sub_app.update() });
            }
        });
        sub_apps.main.world_mut().clear_trackers();
        frames += 1;

        // Sleep until something happens (an event, a vsync ack) or a few
        // milliseconds pass, whichever is first; the MIN_STEP gate above
        // decides whether the wake-up becomes a simulation step.
        rt.wait(MIN_STEP);
    }
}

/// The claimed client of the first player, handed from `main` to the runner.
#[derive(Resource)]
struct FirstClient(Arc<Client>);

/// The web client (`example_client/`), bundled by `build.rs` and served on
/// the websocket port so players only need the one URL.
const CLIENT_INDEX: &[u8] = include_bytes!("../../../example_client/index.html");
const CLIENT_BUNDLE: &[u8] = include_bytes!("../../../example_client/dist/main.js");
const CLIENT_BUNDLE_MAP: &[u8] = include_bytes!("../../../example_client/dist/main.js.map");

fn main() {
    remote_wgpu_runtime::set_port(8000);
    let runtime = runtime();
    runtime.serve_static("/", "text/html; charset=utf-8", CLIENT_INDEX);
    runtime.serve_static("/dist/main.js", "text/javascript; charset=utf-8", CLIENT_BUNDLE);
    runtime.serve_static("/dist/main.js.map", "application/json", CLIENT_BUNDLE_MAP);
    println!(
        "open http://localhost:{}/ in a WebGPU-capable browser; waiting for the first player...",
        runtime.port()
    );
    let first = runtime.next_client();

    let mut app = build_app(create_gpu(&first), None);
    app.insert_resource(FirstClient(first));
    app.add_plugins(ArenaPlugin);
    app.set_runner(runner);
    app.run();
}

/// Render worlds learn about asset contents from `AssetEvent` messages,
/// which only live for two frames; a render world that joins later missed
/// them all.  Replaying `Modified` for every live asset makes the new world
/// extract everything (established worlds just re-prepare once).
fn replay_asset_events(world: &mut World) {
    fn replay<A: Asset>(world: &mut World) {
        let ids: Vec<AssetId<A>> = world.resource::<Assets<A>>().ids().collect();
        let mut messages = world.resource_mut::<bevy::ecs::message::Messages<AssetEvent<A>>>();
        for id in ids {
            messages.write(AssetEvent::Modified { id });
        }
    }
    replay::<Mesh>(world);
    replay::<Image>(world);
    replay::<StandardMaterial>(world);
    replay::<bevy::shader::Shader>(world);
}
