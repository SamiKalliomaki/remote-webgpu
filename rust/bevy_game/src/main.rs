//! A Bevy game that several people play at once, each from their own
//! browser tab.
//!
//! There is exactly one game: a single headless Bevy `App` holding one
//! world, stepped once per frame in this process.  What is per-player is
//! the *view*.  Every browser tab that connects to the websocket becomes
//! its own `winit` window backed by its own remote adapter (that tab's
//! GPU), gets a character spawned into the shared world, and renders that
//! world from a camera following its own character.  Players walk around
//! the same arena, bump into each other and race for the same coins.
//!
//! Run it, then open <http://127.0.0.1:8000> in as many tabs as you like:
//!
//! ```sh
//! cargo run -p bevy_game
//! # in another shell, serve the web client:
//! cd client && npm install && cd ../example_client && npm install && npm run serve
//! ```
//!
//! Move with WASD or the arrow keys, dash with space.
//!
//! For testing there is `--frames N --screenshot PATH`: after N frames the
//! first player's view is copied back over the websocket and written out
//! as a PPM, and the game exits.  (Browsers do not expose a WebGPU canvas
//! to page screenshots, so the picture has to come back this way.)
//!
//! Bevy's own renderer is not used here: `bevy_render` owns a single
//! `RenderDevice`, and in this backend every connected tab is a separate
//! GPU whose device can only touch its own resources and its own canvas.
//! So Bevy runs the game (ECS, systems, `Time`, `Transform`) and each view
//! draws it with a small instanced-quad pipeline of its own.

mod game;
mod render;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use game::{
    ArenaPlugin, Block, Button, Coin, Controls, HalfExtent, Player, Tint, ARENA,
};
use render::{Quad, Renderer};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// How many world units the camera shows vertically.
const VIEW_HEIGHT: f32 = 24.0;
/// The simulation is stepped at most this often, however many views are
/// asking to be redrawn.
const MIN_STEP: Duration = Duration::from_millis(4);

/// One connected player: their window, their GPU, their camera.
struct View {
    window: Arc<Window>,
    renderer: Renderer,
    /// This view's character in the shared world.
    player: Entity,
    slot: usize,
    /// Keys currently held in this browser tab.
    held: HashSet<Button>,
    /// Smoothed camera position, in world units.
    camera: Vec2,
}

/// A player, flattened out of the ECS for the renderer and the HUD.
struct PlayerView {
    entity: Entity,
    slot: usize,
    position: Vec2,
    score: u32,
    color: [f32; 4],
    /// Where this player's body landed in the scene's quad list, so the
    /// view drawing it can outline its own character.
    quad: usize,
}

struct Game {
    app: App,
    views: HashMap<WindowId, View>,
    /// Static geometry: the floor, its grid and the walls.
    backdrop: Vec<Quad>,
    next_slot: usize,
    last_step: Instant,
    /// `--frames` / `--screenshot`, for the test harness.
    screenshot: Option<PathBuf>,
    frame_limit: Option<u64>,
    frames: u64,
}

impl Game {
    fn new(screenshot: Option<PathBuf>, frame_limit: Option<u64>) -> Self {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(ArenaPlugin);
        // We drive the app ourselves from the event loop rather than
        // handing control to a Bevy runner, so finish plugin setup here.
        app.finish();
        app.cleanup();

        Self {
            app,
            views: HashMap::new(),
            backdrop: backdrop(),
            next_slot: 0,
            last_step: Instant::now(),
            screenshot,
            frame_limit,
            frames: 0,
        }
    }

    /// Turns a freshly connected client into a player.
    fn add_view(&mut self, window: Window) {
        let window = Arc::new(window);
        let renderer = Renderer::new(window.clone(), self.screenshot.is_some());
        let slot = self.next_slot;
        self.next_slot += 1;
        let player = game::spawn_player(self.app.world_mut(), slot);

        eprintln!(
            "bevy_game: player {} joined on {} ({} playing)",
            slot + 1,
            renderer.adapter_name(),
            self.views.len() + 1,
        );

        window.request_redraw();
        self.views.insert(
            window.id(),
            View {
                camera: Vec2::ZERO,
                window,
                renderer,
                player,
                slot,
                held: HashSet::new(),
            },
        );
    }

    /// Lets people join while the game is running.
    fn accept_new_players(&mut self, event_loop: &ActiveEventLoop) {
        while let Some(window) =
            event_loop.create_window_for_new_client(Window::default_attributes())
        {
            self.add_view(window);
        }
    }

    /// Advances the shared world.  Called from whichever view is drawing;
    /// the rate limit keeps it to one step per frame's worth of time no
    /// matter how many views there are.
    fn step(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_step) < MIN_STEP {
            return;
        }
        self.last_step = now;

        // Hand each character the keys its own tab is holding.
        for view in self.views.values() {
            if let Some(mut controls) = self.app.world_mut().get_mut::<Controls>(view.player) {
                controls.held.clone_from(&view.held);
            }
        }

        self.app.update();
    }

    /// Flattens the world into quads, plus the player list the HUD needs.
    fn snapshot(&mut self) -> (Vec<Quad>, Vec<PlayerView>) {
        let mut quads = self.backdrop.clone();
        let world = self.app.world_mut();

        let mut blocks = world.query_filtered::<(&Transform, &HalfExtent, &Tint), With<Block>>();
        for (transform, half, tint) in blocks.iter(world) {
            quads.push(
                Quad::new(transform.translation.truncate(), half.0, tint.0)
                    .with_border([0.32, 0.36, 0.47, 1.0]),
            );
        }

        let mut coins = world.query::<(&Transform, &HalfExtent, &Tint, &Coin)>();
        for (transform, half, tint, coin) in coins.iter(world) {
            // A gentle bob, so the coins read as pickups and not scenery.
            let pulse = 1.0 + 0.18 * coin.phase.sin();
            quads.push(
                Quad::new(transform.translation.truncate(), half.0 * pulse, tint.0)
                    .with_border([1.0, 0.96, 0.75, 1.0]),
            );
        }

        let mut players = Vec::new();
        let mut query = world.query::<(Entity, &Transform, &HalfExtent, &Tint, &Player)>();
        for (entity, transform, half, tint, player) in query.iter(world) {
            let position = transform.translation.truncate();
            players.push(PlayerView {
                entity,
                slot: player.slot,
                position,
                score: player.score,
                color: tint.0,
                quad: quads.len(),
            });
            quads.push(Quad::new(position, half.0, tint.0));
            // A pip showing which way this character is facing (and so
            // which way a dash would go).
            quads.push(Quad::new(
                position + player.facing * half.0.x * 0.9,
                Vec2::splat(half.0.x * 0.28),
                [1.0, 1.0, 1.0, 0.9],
            ));
        }

        (quads, players)
    }

    /// Draws one view.  Returns whether this was the `--screenshot` frame.
    fn draw(&mut self, window_id: WindowId) -> bool {
        let (mut quads, players) = self.snapshot();

        let Some(view) = self.views.get_mut(&window_id) else {
            return false;
        };
        let Some(mine) = players.iter().find(|p| p.entity == view.player) else {
            return false;
        };

        // Outline the local player in white and everyone else in black, so
        // each tab can tell at a glance which character is theirs.
        for player in &players {
            quads[player.quad].border = if player.entity == view.player {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.05, 0.06, 0.09, 1.0]
            };
        }

        let aspect = view.renderer.aspect();
        let half_extent = Vec2::new(VIEW_HEIGHT * 0.5 * aspect, VIEW_HEIGHT * 0.5);
        // The camera follows the character rather than staying inside the
        // arena: clamping it to the play field pushes a player who is up
        // against a wall out to the edge of their own view.  Beyond the
        // walls there is simply nothing to see.
        view.camera = view.camera.lerp(mine.position, 0.18);
        let center = view.camera;

        let hud = hud(&players, mine, aspect);

        // `--frames` ends the run; with `--screenshot` the last frame of the
        // first player's view is read back and written out before it does.
        let done = self.frame_limit.is_some_and(|limit| self.frames >= limit);
        let capture = match &self.screenshot {
            Some(path) if done && view.slot == 0 => Some(path.as_path()),
            _ => None,
        };
        match capture {
            Some(path) => view.renderer.capture(center, half_extent, &quads, &hud, path),
            None => view.renderer.render(center, half_extent, &quads, &hud),
        }
        self.frames += 1;
        done && (self.screenshot.is_none() || capture.is_some())
    }

    fn remove_view(&mut self, window_id: WindowId, event_loop: &ActiveEventLoop) {
        let Some(view) = self.views.remove(&window_id) else {
            return;
        };
        self.app.world_mut().despawn(view.player);
        eprintln!(
            "bevy_game: player {} left ({} playing)",
            view.slot + 1,
            self.views.len()
        );
        if self.views.is_empty() {
            eprintln!("bevy_game: no players left, exiting");
            event_loop.exit();
        }
    }
}

impl ApplicationHandler for Game {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        eprintln!("bevy_game: waiting for the first player to connect ...");
        let window = event_loop
            .create_window(Window::default_attributes())
            .expect("create window");
        self.add_view(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(view) = self.views.get_mut(&window_id) {
                    view.renderer.resize(size.width, size.height);
                    view.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let Some(button) = button_for(code) else {
                    return;
                };
                if let Some(view) = self.views.get_mut(&window_id) {
                    match event.state {
                        ElementState::Pressed => view.held.insert(button),
                        ElementState::Released => view.held.remove(&button),
                    };
                }
            }
            WindowEvent::CloseRequested => self.remove_view(window_id, event_loop),
            WindowEvent::RedrawRequested => {
                self.accept_new_players(event_loop);
                self.step();
                if self.draw(window_id) {
                    event_loop.exit();
                    return;
                }
                if let Some(view) = self.views.get(&window_id) {
                    view.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Nobody is drawing (every tab is hidden, say) -- keep the world
        // ticking and keep accepting players anyway.
        self.accept_new_players(event_loop);
        self.step();
    }
}

fn button_for(code: KeyCode) -> Option<Button> {
    Some(match code {
        KeyCode::KeyW | KeyCode::ArrowUp => Button::Up,
        KeyCode::KeyS | KeyCode::ArrowDown => Button::Down,
        KeyCode::KeyA | KeyCode::ArrowLeft => Button::Left,
        KeyCode::KeyD | KeyCode::ArrowRight => Button::Right,
        KeyCode::Space | KeyCode::ShiftLeft => Button::Dash,
        _ => return None,
    })
}

/// The floor, its grid and the walls -- the same for every view, so it is
/// built once.
fn backdrop() -> Vec<Quad> {
    let mut quads = vec![Quad::new(Vec2::ZERO, ARENA, [0.09, 0.10, 0.14, 1.0])];

    let line = [0.13, 0.15, 0.20, 1.0];
    let step = 6.0;
    let mut x = -ARENA.x + step;
    while x < ARENA.x {
        quads.push(Quad::new(Vec2::new(x, 0.0), Vec2::new(0.04, ARENA.y), line));
        x += step;
    }
    let mut y = -ARENA.y + step;
    while y < ARENA.y {
        quads.push(Quad::new(Vec2::new(0.0, y), Vec2::new(ARENA.x, 0.04), line));
        y += step;
    }

    let wall = [0.30, 0.34, 0.45, 1.0];
    let thickness = 0.5;
    for (center, half) in [
        (Vec2::new(0.0, ARENA.y), Vec2::new(ARENA.x + thickness, thickness)),
        (Vec2::new(0.0, -ARENA.y), Vec2::new(ARENA.x + thickness, thickness)),
        (Vec2::new(-ARENA.x, 0.0), Vec2::new(thickness, ARENA.y + thickness)),
        (Vec2::new(ARENA.x, 0.0), Vec2::new(thickness, ARENA.y + thickness)),
    ] {
        quads.push(Quad::new(center, half, wall));
    }

    quads
}

/// The scoreboard, in clip space: one row per player, longest bar wins.
/// There is no font here, so a score is a bar rather than a number.
fn hud(players: &[PlayerView], mine: &PlayerView, aspect: f32) -> Vec<Quad> {
    let mut ranked: Vec<&PlayerView> = players.iter().collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.slot.cmp(&b.slot)));

    let row_height = 0.075;
    let swatch = 0.022;
    let left = -0.97;
    let top = 0.94;
    let bar_max = 0.26;
    // A bar is full at this score, after which every bar rescales.
    let best = ranked.first().map_or(1, |p| p.score).max(8) as f32;

    let panel_height = row_height * ranked.len() as f32 + 0.03;
    let mut quads = vec![Quad::new(
        Vec2::new(left + bar_max * 0.5 + 0.05, top + swatch - panel_height * 0.5),
        Vec2::new(bar_max * 0.5 + 0.09, panel_height * 0.5),
        [0.0, 0.0, 0.0, 0.45],
    )];

    for (row, player) in ranked.iter().enumerate() {
        let y = top - row as f32 * row_height;
        let swatch_center = Vec2::new(left + swatch / aspect, y);
        quads.push(
            Quad::new(swatch_center, Vec2::new(swatch / aspect, swatch), player.color)
                .with_border(if player.entity == mine.entity {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [0.0, 0.0, 0.0, 0.6]
                }),
        );

        // The track, then the filled part of it.
        let track_left = left + 3.0 * swatch / aspect;
        quads.push(Quad::new(
            Vec2::new(track_left + bar_max * 0.5, y),
            Vec2::new(bar_max * 0.5, swatch * 0.45),
            [1.0, 1.0, 1.0, 0.12],
        ));
        let filled = bar_max * (player.score as f32 / best).min(1.0);
        if filled > 0.0 {
            quads.push(Quad::new(
                Vec2::new(track_left + filled * 0.5, y),
                Vec2::new(filled * 0.5, swatch * 0.45),
                player.color,
            ));
        }
    }

    quads
}

fn main() {
    let mut screenshot = None;
    let mut frames = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--frames" => frames = args.next().and_then(|n| n.parse().ok()),
            other => {
                eprintln!("bevy_game: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    if screenshot.is_some() && frames.is_none() {
        frames = Some(300);
    }

    let event_loop = EventLoop::new().expect("event loop");
    let mut game = Game::new(screenshot, frames);
    event_loop.run_app(&mut game).expect("run");
}
