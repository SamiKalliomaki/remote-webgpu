//! The game: one Bevy 3D world, shared by everyone who is connected.
//!
//! This is a straight 3D re-imagining of the flat `bevy_game` arena, but
//! rendered by `bevy_pbr`'s standard mesh pipeline.  Characters are capsules
//! racing for coins on an obstacle-strewn field.  Nothing in this module
//! knows about windows, render worlds or websockets; the runner in `main.rs`
//! fills in [`Controls`] from each player's browser tab and steps the app.

use std::collections::HashSet;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

/// Half extents of the play field on the XZ plane, in world units.
pub const ARENA: Vec2 = Vec2::new(30.0, 20.0);
pub const PLAYER_RADIUS: f32 = 0.9;
const COIN_COUNT: usize = 12;

const SPEED: f32 = 14.0;
const DASH_SPEED: f32 = 40.0;
const DASH_TIME: f32 = 0.14;
const DASH_COOLDOWN: f32 = 0.9;

/// The colors handed out to players, in join order.
pub const PALETTE: [Color; 8] = [
    Color::srgb(0.31, 0.76, 0.97), // sky
    Color::srgb(0.98, 0.45, 0.35), // coral
    Color::srgb(0.55, 0.90, 0.51), // green
    Color::srgb(0.96, 0.76, 0.30), // amber
    Color::srgb(0.79, 0.55, 0.98), // violet
    Color::srgb(0.99, 0.55, 0.78), // pink
    Color::srgb(0.45, 0.95, 0.85), // teal
    Color::srgb(0.85, 0.85, 0.60), // sand
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

/// The buttons this player is currently holding.  The runner overwrites
/// this from the player's browser tab each step.
#[derive(Component, Default)]
pub struct Controls {
    pub held: HashSet<Button>,
}

/// Velocity on the XZ plane.
#[derive(Component)]
pub struct Velocity(pub Vec2);

/// Half extents of an axis-aligned box on the XZ plane, for collision.
#[derive(Component)]
pub struct HalfExtent(pub Vec2);

/// A collectible.  Bumping into one scores a point and moves it elsewhere.
#[derive(Component)]
pub struct Coin {
    pub phase: f32,
}

/// A solid piece of scenery.  Players cannot walk through these.
#[derive(Component)]
pub struct Block;

/// Marks a camera as following a player's character.
#[derive(Component)]
pub struct FollowPlayer(pub Entity);

/// The little score pips floating above a character.
#[derive(Component)]
pub struct ScorePip {
    owner: Entity,
    index: u32,
}

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

/// Shared handles for things spawned at runtime (characters, score pips).
#[derive(Resource)]
pub struct GameAssets {
    player_mesh: Handle<Mesh>,
    pip_mesh: Handle<Mesh>,
    pip_material: Handle<StandardMaterial>,
    player_materials: Vec<Handle<StandardMaterial>>,
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
                    update_score_pips,
                    follow_cameras,
                )
                    .chain(),
            );
    }
}

/// Where players appear, cycling as more of them join.
fn spawn_point(slot: usize) -> Vec2 {
    const POINTS: [Vec2; 8] = [
        Vec2::new(-22.0, -13.0),
        Vec2::new(22.0, 13.0),
        Vec2::new(22.0, -13.0),
        Vec2::new(-22.0, 13.0),
        Vec2::new(0.0, -16.0),
        Vec2::new(0.0, 16.0),
        Vec2::new(-26.0, 0.0),
        Vec2::new(26.0, 0.0),
    ];
    POINTS[slot % POINTS.len()]
}

/// Adds a character for a newly connected client.  Called by the runner,
/// not by a system, because players arrive with their windows.
pub fn spawn_player(world: &mut World, slot: usize) -> Entity {
    let position = spawn_point(slot);
    let assets = world.resource::<GameAssets>();
    let mesh = assets.player_mesh.clone();
    let material = assets.player_materials[slot % assets.player_materials.len()].clone();
    world
        .spawn((
            Player {
                slot,
                score: 0,
                facing: Vec2::new(0.0, -1.0),
                dash_left: 0.0,
                dash_cooldown: 0.0,
            },
            Controls::default(),
            Velocity(Vec2::ZERO),
            HalfExtent(Vec2::splat(PLAYER_RADIUS)),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(position.x, PLAYER_RADIUS + 0.6, position.y),
        ))
        .id()
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rng: ResMut<Rng>,
) {
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            ..default()
        },
        Transform::from_xyz(18.0, 30.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // The floor, with a subtle checker made of slightly raised tiles.
    let floor = materials.add(StandardMaterial {
        base_color: Color::srgb(0.13, 0.15, 0.21),
        perceptual_roughness: 0.95,
        ..default()
    });
    let tile = materials.add(StandardMaterial {
        base_color: Color::srgb(0.17, 0.20, 0.27),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(ARENA.x * 2.0, 1.0, ARENA.y * 2.0))),
        MeshMaterial3d(floor),
        Transform::from_xyz(0.0, -0.5, 0.0),
    ));
    let tile_mesh = meshes.add(Cuboid::new(5.0, 0.12, 5.0));
    for ix in -3i32..=3 {
        for iz in -2i32..=2 {
            if (ix + iz).rem_euclid(2) == 0 {
                continue;
            }
            commands.spawn((
                Mesh3d(tile_mesh.clone()),
                MeshMaterial3d(tile.clone()),
                Transform::from_xyz(ix as f32 * 8.0, 0.0, iz as f32 * 8.0),
            ));
        }
    }

    // Walls around the arena.
    let wall = materials.add(StandardMaterial {
        base_color: Color::srgb(0.30, 0.34, 0.45),
        perceptual_roughness: 0.8,
        ..default()
    });
    let walls: [(Vec2, Vec2); 4] = [
        (Vec2::new(0.0, -ARENA.y - 0.5), Vec2::new(ARENA.x + 1.0, 0.5)),
        (Vec2::new(0.0, ARENA.y + 0.5), Vec2::new(ARENA.x + 1.0, 0.5)),
        (Vec2::new(-ARENA.x - 0.5, 0.0), Vec2::new(0.5, ARENA.y + 1.0)),
        (Vec2::new(ARENA.x + 0.5, 0.0), Vec2::new(0.5, ARENA.y + 1.0)),
    ];
    for (center, half) in walls {
        commands.spawn((
            Block,
            HalfExtent(half),
            Mesh3d(meshes.add(Cuboid::new(half.x * 2.0, 2.5, half.y * 2.0))),
            MeshMaterial3d(wall.clone()),
            Transform::from_xyz(center.x, 1.25, center.y),
        ));
    }

    // Obstacles to hide behind and bump into.
    let block_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.36, 0.55),
        perceptual_roughness: 0.7,
        ..default()
    });
    let blocks: [(Vec2, Vec2, f32); 7] = [
        (Vec2::new(0.0, 0.0), Vec2::new(3.5, 1.4), 2.6),
        (Vec2::new(0.0, 8.5), Vec2::new(1.4, 3.2), 1.8),
        (Vec2::new(0.0, -8.5), Vec2::new(1.4, 3.2), 1.8),
        (Vec2::new(-15.0, 5.5), Vec2::new(4.2, 1.2), 1.5),
        (Vec2::new(15.0, -5.5), Vec2::new(4.2, 1.2), 1.5),
        (Vec2::new(-14.0, -9.0), Vec2::new(1.2, 4.0), 2.2),
        (Vec2::new(14.0, 9.0), Vec2::new(1.2, 4.0), 2.2),
    ];
    for (center, half, height) in blocks {
        commands.spawn((
            Block,
            HalfExtent(half),
            Mesh3d(meshes.add(Cuboid::new(half.x * 2.0, height, half.y * 2.0))),
            MeshMaterial3d(block_material.clone()),
            Transform::from_xyz(center.x, height / 2.0, center.y),
        ));
    }

    // Coins.
    let coin_mesh = meshes.add(Sphere::new(0.55));
    let coin_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.84, 0.25),
        emissive: LinearRgba::new(0.6, 0.45, 0.05, 1.0),
        perceptual_roughness: 0.3,
        metallic: 0.8,
        ..default()
    });
    for index in 0..COIN_COUNT {
        let position = Vec2::new(
            rng.range(-ARENA.x + 3.0, ARENA.x - 3.0),
            rng.range(-ARENA.y + 3.0, ARENA.y - 3.0),
        );
        commands.spawn((
            Coin { phase: index as f32 * 0.7 },
            HalfExtent(Vec2::splat(0.8)),
            Mesh3d(coin_mesh.clone()),
            MeshMaterial3d(coin_material.clone()),
            NotShadowCaster,
            Transform::from_xyz(position.x, 1.3, position.y),
        ));
    }

    // Handles for things spawned when players join.
    let player_mesh = meshes.add(Capsule3d::new(PLAYER_RADIUS, 1.2));
    let player_materials = PALETTE
        .iter()
        .map(|&color| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.5,
                ..default()
            })
        })
        .collect();
    commands.insert_resource(GameAssets {
        player_mesh,
        pip_mesh: meshes.add(Cuboid::new(0.35, 0.35, 0.35)),
        pip_material: coin_material,
        player_materials,
    });
}

/// XZ position helper.
fn flat(translation: Vec3) -> Vec2 {
    Vec2::new(translation.x, translation.z)
}

fn drive_players(time: Res<Time>, mut players: Query<(&mut Player, &mut Velocity, &Controls)>) {
    let dt = time.delta_secs();
    for (mut player, mut velocity, controls) in &mut players {
        let mut direction = Vec2::ZERO;
        if controls.held.contains(&Button::Up) {
            direction.y -= 1.0;
        }
        if controls.held.contains(&Button::Down) {
            direction.y += 1.0;
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

/// Moves each player on the XZ plane and resolves walls and scenery.
fn move_players(
    time: Res<Time>,
    blocks: Query<(&Transform, &HalfExtent), (With<Block>, Without<Player>)>,
    mut players: Query<(&mut Transform, &mut Velocity, &HalfExtent, &Player)>,
) {
    let dt = time.delta_secs();
    let scenery: Vec<(Vec2, Vec2)> = blocks
        .iter()
        .map(|(transform, half)| (flat(transform.translation), half.0))
        .collect();

    for (mut transform, mut velocity, half, player) in &mut players {
        let mut position = flat(transform.translation) + velocity.0 * dt;

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
        transform.translation.z = position.y;
        // Face the direction of travel.
        let facing = player.facing;
        transform.rotation = Quat::from_rotation_y(f32::atan2(-facing.x, -facing.y));
    }
}

/// Players are solid to each other, so bumping shoves both of them.
fn separate_players(mut players: Query<(&mut Transform, &HalfExtent), With<Player>>) {
    let mut combinations = players.iter_combinations_mut();
    while let Some([(mut a_transform, a_half), (mut b_transform, b_half)]) =
        combinations.fetch_next()
    {
        let a = flat(a_transform.translation);
        let b = flat(b_transform.translation);
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
        a_transform.translation += Vec3::new(shove.x, 0.0, shove.y);
        b_transform.translation -= Vec3::new(shove.x, 0.0, shove.y);
    }
}

fn collect_coins(
    mut rng: ResMut<Rng>,
    mut players: Query<(&mut Player, &Transform, &HalfExtent)>,
    mut coins: Query<(&mut Transform, &HalfExtent), (With<Coin>, Without<Player>)>,
) {
    for (mut player, player_transform, player_half) in &mut players {
        let position = flat(player_transform.translation);
        for (mut coin_transform, coin_half) in &mut coins {
            let delta = position - flat(coin_transform.translation);
            let combined = player_half.0 + coin_half.0;
            if delta.x.abs() < combined.x && delta.y.abs() < combined.y {
                player.score += 1;
                let fresh = Vec2::new(
                    rng.range(-ARENA.x + 3.0, ARENA.x - 3.0),
                    rng.range(-ARENA.y + 3.0, ARENA.y - 3.0),
                );
                coin_transform.translation.x = fresh.x;
                coin_transform.translation.z = fresh.y;
            }
        }
    }
}

fn animate_coins(time: Res<Time>, mut coins: Query<(&mut Coin, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut coin, mut transform) in &mut coins {
        coin.phase += dt * 3.0;
        transform.translation.y = 1.3 + coin.phase.sin() * 0.25;
        transform.rotation = Quat::from_rotation_y(coin.phase * 0.7);
    }
}

/// Keeps a little column of gold pips above each character, one per point.
fn update_score_pips(
    mut commands: Commands,
    assets: Res<GameAssets>,
    players: Query<(Entity, &Player, &Transform)>,
    mut pips: Query<(Entity, &ScorePip, &mut Transform), Without<Player>>,
) {
    let mut counts: Vec<(Entity, u32, Vec3)> = Vec::new();
    for (entity, player, transform) in &players {
        counts.push((entity, player.score.min(8), transform.translation));
    }

    let mut seen: Vec<(Entity, u32)> = Vec::new();
    for (pip_entity, pip, mut transform) in &mut pips {
        match counts.iter().find(|(owner, ..)| *owner == pip.owner) {
            Some((_, score, position)) if pip.index < *score => {
                transform.translation =
                    *position + Vec3::new(0.0, 2.4 + pip.index as f32 * 0.5, 0.0);
                seen.push((pip.owner, pip.index));
            }
            _ => commands.entity(pip_entity).despawn(),
        }
    }
    for (owner, score, position) in counts {
        for index in 0..score {
            if seen.contains(&(owner, index)) {
                continue;
            }
            commands.spawn((
                ScorePip { owner, index },
                Mesh3d(assets.pip_mesh.clone()),
                MeshMaterial3d(assets.pip_material.clone()),
                NotShadowCaster,
                Transform::from_translation(
                    position + Vec3::new(0.0, 2.4 + index as f32 * 0.5, 0.0),
                ),
            ));
        }
    }
}

/// Third-person chase camera per player.
fn follow_cameras(
    time: Res<Time>,
    players: Query<&Transform, (With<Player>, Without<FollowPlayer>)>,
    mut cameras: Query<(&FollowPlayer, &mut Transform)>,
) {
    let blend = (time.delta_secs() * 5.0).min(1.0);
    for (follow, mut transform) in &mut cameras {
        let Ok(target) = players.get(follow.0) else {
            continue;
        };
        let goal = target.translation + Vec3::new(0.0, 13.0, 15.0);
        transform.translation = transform.translation.lerp(goal, blend);
        let look = target.translation + Vec3::new(0.0, 1.0, 0.0);
        *transform = transform.looking_at(look, Vec3::Y);
    }
}
