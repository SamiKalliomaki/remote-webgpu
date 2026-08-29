//! The game: one Bevy world, shared by everyone who is connected.
//!
//! Nothing here knows about rendering, windows or websockets.  It is an
//! ordinary headless Bevy app: components, a `setup` system that builds the
//! arena, and an `Update` schedule that moves the characters around.  The
//! only concession to the outside world is [`Controls`], which the event
//! loop fills in from each player's keyboard before stepping the app.

use std::collections::HashSet;

use bevy::prelude::*;

/// Half extents of the play field, in world units.
pub const ARENA: Vec2 = Vec2::new(34.0, 22.0);
pub const PLAYER_HALF: f32 = 0.9;
pub const COIN_HALF: f32 = 0.5;
pub const COIN_COUNT: usize = 14;

const SPEED: f32 = 17.0;
const DASH_SPEED: f32 = 46.0;
const DASH_TIME: f32 = 0.14;
const DASH_COOLDOWN: f32 = 0.9;

/// The colors handed out to players, in join order.
pub const PALETTE: [[f32; 4]; 8] = [
    [0.31, 0.76, 0.97, 1.0], // sky
    [0.98, 0.45, 0.35, 1.0], // coral
    [0.55, 0.90, 0.51, 1.0], // green
    [0.96, 0.76, 0.30, 1.0], // amber
    [0.79, 0.55, 0.98, 1.0], // violet
    [0.99, 0.55, 0.78, 1.0], // pink
    [0.45, 0.95, 0.85, 1.0], // teal
    [0.85, 0.85, 0.60, 1.0], // sand
];

/// A key a player can hold down, as the game thinks of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Dash,
}

/// One connected player's character.
#[derive(Component)]
pub struct Player {
    /// Join order; picks the color and the spawn corner.
    pub slot: usize,
    pub score: u32,
    pub facing: Vec2,
    dash_left: f32,
    dash_cooldown: f32,
}

/// The buttons this player is currently holding.  The event loop
/// overwrites this from the player's browser tab each step.
#[derive(Component, Default)]
pub struct Controls {
    pub held: HashSet<Button>,
}

#[derive(Component)]
pub struct Velocity(pub Vec2);

/// Half extents of an axis-aligned box, for collision and for drawing.
#[derive(Component)]
pub struct HalfExtent(pub Vec2);

#[derive(Component)]
pub struct Tint(pub [f32; 4]);

/// A collectible.  Bumping into one scores a point and moves it elsewhere.
#[derive(Component)]
pub struct Coin {
    /// Animation phase, so the coins bob out of sync with each other.
    pub phase: f32,
}

/// A solid piece of scenery.  Players cannot walk through these.
#[derive(Component)]
pub struct Block;

/// A tiny LCG, so coin placement needs no dependency.
#[derive(Resource)]
struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / ((1u64 << 31) as f32)
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + self.next_f32() * (max - min)
    }
}

pub struct ArenaPlugin;

impl Plugin for ArenaPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Rng(0x5eed_1234_abcd_0001))
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    drive_players,
                    move_players,
                    separate_players,
                    collect_coins,
                    animate_coins,
                )
                    .chain(),
            );
    }
}

/// Where players appear, cycling as more of them join.
fn spawn_point(slot: usize) -> Vec2 {
    const POINTS: [Vec2; 8] = [
        Vec2::new(-24.0, -14.0),
        Vec2::new(24.0, 14.0),
        Vec2::new(24.0, -14.0),
        Vec2::new(-24.0, 14.0),
        Vec2::new(0.0, -17.0),
        Vec2::new(0.0, 17.0),
        Vec2::new(-29.0, 0.0),
        Vec2::new(29.0, 0.0),
    ];
    POINTS[slot % POINTS.len()]
}

/// Adds a character for a newly connected client.  Called by the event
/// loop, not by a system, because players arrive with their windows.
pub fn spawn_player(world: &mut World, slot: usize) -> Entity {
    let position = spawn_point(slot);
    world
        .spawn((
            Player {
                slot,
                score: 0,
                facing: Vec2::new(0.0, 1.0),
                dash_left: 0.0,
                dash_cooldown: 0.0,
            },
            Controls::default(),
            Velocity(Vec2::ZERO),
            HalfExtent(Vec2::splat(PLAYER_HALF)),
            Tint(PALETTE[slot % PALETTE.len()]),
            Transform::from_xyz(position.x, position.y, 0.0),
        ))
        .id()
}

fn setup(mut commands: Commands, mut rng: ResMut<Rng>) {
    // Scenery: a few blocks to hide behind and bump into.
    let blocks: [(Vec2, Vec2); 9] = [
        (Vec2::new(0.0, 0.0), Vec2::new(4.0, 1.5)),
        (Vec2::new(0.0, 9.0), Vec2::new(1.5, 4.0)),
        (Vec2::new(0.0, -9.0), Vec2::new(1.5, 4.0)),
        (Vec2::new(-16.0, 6.0), Vec2::new(5.0, 1.2)),
        (Vec2::new(16.0, -6.0), Vec2::new(5.0, 1.2)),
        (Vec2::new(-16.0, -8.0), Vec2::new(1.2, 5.0)),
        (Vec2::new(16.0, 8.0), Vec2::new(1.2, 5.0)),
        (Vec2::new(-26.0, 16.0), Vec2::new(3.0, 3.0)),
        (Vec2::new(26.0, -16.0), Vec2::new(3.0, 3.0)),
    ];
    for (center, half) in blocks {
        commands.spawn((
            Block,
            HalfExtent(half),
            Tint([0.24, 0.27, 0.36, 1.0]),
            Transform::from_xyz(center.x, center.y, 0.0),
        ));
    }

    for index in 0..COIN_COUNT {
        let position = Vec2::new(
            rng.range(-ARENA.x + 3.0, ARENA.x - 3.0),
            rng.range(-ARENA.y + 3.0, ARENA.y - 3.0),
        );
        commands.spawn((
            Coin { phase: index as f32 * 0.7 },
            HalfExtent(Vec2::splat(COIN_HALF)),
            Tint([0.99, 0.83, 0.33, 1.0]),
            Transform::from_xyz(position.x, position.y, 0.0),
        ));
    }
}

fn drive_players(time: Res<Time>, mut players: Query<(&mut Player, &mut Velocity, &Controls)>) {
    let dt = time.delta_secs();
    for (mut player, mut velocity, controls) in &mut players {
        let mut direction = Vec2::ZERO;
        if controls.held.contains(&Button::Up) {
            direction.y += 1.0;
        }
        if controls.held.contains(&Button::Down) {
            direction.y -= 1.0;
        }
        if controls.held.contains(&Button::Left) {
            direction.x -= 1.0;
        }
        if controls.held.contains(&Button::Right) {
            direction.x += 1.0;
        }
        let direction = direction.normalize_or_zero();
        if direction != Vec2::ZERO {
            player.facing = direction;
        }

        player.dash_cooldown = (player.dash_cooldown - dt).max(0.0);
        if player.dash_left > 0.0 {
            // A dash keeps its own velocity until it runs out.
            player.dash_left -= dt;
            continue;
        }
        if controls.held.contains(&Button::Dash) && player.dash_cooldown == 0.0 {
            player.dash_left = DASH_TIME;
            player.dash_cooldown = DASH_COOLDOWN;
            velocity.0 = player.facing * DASH_SPEED;
            continue;
        }

        // Ease toward the requested velocity: snappy, but not instant.
        let target = direction * SPEED;
        let blend = (dt * 14.0).min(1.0);
        velocity.0 = velocity.0.lerp(target, blend);
    }
}

/// Moves each player and resolves the arena walls and the scenery.
fn move_players(
    time: Res<Time>,
    blocks: Query<(&Transform, &HalfExtent), (With<Block>, Without<Player>)>,
    mut players: Query<(&mut Transform, &mut Velocity, &HalfExtent), With<Player>>,
) {
    let dt = time.delta_secs();
    let scenery: Vec<(Vec2, Vec2)> = blocks
        .iter()
        .map(|(transform, half)| (transform.translation.truncate(), half.0))
        .collect();

    for (mut transform, mut velocity, half) in &mut players {
        let mut position = transform.translation.truncate() + velocity.0 * dt;

        for (block_center, block_half) in &scenery {
            let combined = *block_half + half.0;
            let delta = position - *block_center;
            let overlap = combined - delta.abs();
            if overlap.x <= 0.0 || overlap.y <= 0.0 {
                continue;
            }
            // Push out along whichever axis is the shallower intrusion.
            if overlap.x < overlap.y {
                position.x = block_center.x + combined.x * delta.x.signum();
                velocity.0.x = 0.0;
            } else {
                position.y = block_center.y + combined.y * delta.y.signum();
                velocity.0.y = 0.0;
            }
        }

        let limit = ARENA - half.0;
        if position.x.abs() > limit.x {
            position.x = limit.x * position.x.signum();
            velocity.0.x = 0.0;
        }
        if position.y.abs() > limit.y {
            position.y = limit.y * position.y.signum();
            velocity.0.y = 0.0;
        }

        transform.translation.x = position.x;
        transform.translation.y = position.y;
    }
}

/// Players are solid to each other, so bumping shoves both of them.
fn separate_players(mut players: Query<(&mut Transform, &HalfExtent), With<Player>>) {
    let mut combinations = players.iter_combinations_mut();
    while let Some([(mut a_transform, a_half), (mut b_transform, b_half)]) =
        combinations.fetch_next()
    {
        let a = a_transform.translation.truncate();
        let b = b_transform.translation.truncate();
        let combined = a_half.0 + b_half.0;
        let delta = a - b;
        let overlap = combined - delta.abs();
        if overlap.x <= 0.0 || overlap.y <= 0.0 {
            continue;
        }
        let (axis, push) = if overlap.x < overlap.y {
            (Vec2::X, overlap.x)
        } else {
            (Vec2::Y, overlap.y)
        };
        let sign = if (delta * axis).element_sum() < 0.0 { -1.0 } else { 1.0 };
        let shove = axis * push * sign * 0.5;
        a_transform.translation += shove.extend(0.0);
        b_transform.translation -= shove.extend(0.0);
    }
}

fn collect_coins(
    mut rng: ResMut<Rng>,
    mut players: Query<(&mut Player, &Transform, &HalfExtent)>,
    mut coins: Query<(&mut Transform, &HalfExtent), (With<Coin>, Without<Player>)>,
) {
    for (mut player, player_transform, player_half) in &mut players {
        let position = player_transform.translation.truncate();
        for (mut coin_transform, coin_half) in &mut coins {
            let delta = position - coin_transform.translation.truncate();
            let combined = player_half.0 + coin_half.0;
            if delta.x.abs() < combined.x && delta.y.abs() < combined.y {
                player.score += 1;
                let fresh = Vec2::new(
                    rng.range(-ARENA.x + 3.0, ARENA.x - 3.0),
                    rng.range(-ARENA.y + 3.0, ARENA.y - 3.0),
                );
                coin_transform.translation.x = fresh.x;
                coin_transform.translation.y = fresh.y;
            }
        }
    }
}

fn animate_coins(time: Res<Time>, mut coins: Query<&mut Coin>) {
    let dt = time.delta_secs();
    for mut coin in &mut coins {
        coin.phase += dt * 3.0;
    }
}
