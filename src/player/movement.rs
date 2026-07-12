use std::f32::consts::FRAC_PI_2;

use bevy::pbr::ClusterConfig;
use bevy::prelude::*;

use crate::dungeon::{Dungeon, Facing, GridPos};
use crate::render::BLOCK_SIZE;

/// Duration of one step or one 90° turn animation, in seconds
/// (project.md「0.2〜0.3秒程度のイージング付きアニメーション」).
pub const STEP_DURATION: f32 = 0.25;

/// Camera eye height above the floor (block center height).
pub const EYE_HEIGHT: f32 = 0.5;

/// Vertical field of view in degrees.
pub const FOV_DEGREES: f32 = 78.0;

/// A discrete player action. Movement is always tile-by-tile / 90° at a time —
/// the logical state jumps a whole tile; only the camera is interpolated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Forward,
    Backward,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
}

/// Marker for the first-person camera entity.
#[derive(Component)]
pub struct PlayerCamera;

/// The player's logical state: which tile they stand on and which way they face.
/// This is authoritative; the camera transform chases it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Player {
    pub pos: GridPos,
    pub facing: Facing,
}

/// The in-progress move/turn animation plus the one-slot input buffer.
///
/// Input buffering (project.md): while an animation plays, a key press is held
/// in `buffered` (only the latest, one slot) and consumed the instant the
/// current animation finishes, so holding a key produces uninterrupted walking.
#[derive(Resource, Default)]
pub struct MoveAnim {
    active: bool,
    elapsed: f32,
    from_pos: Vec3,
    to_pos: Vec3,
    from_yaw: f32,
    to_yaw: f32,
    buffered: Option<Action>,
}

/// Ease-in-out (smoothstep) over `t` in `[0, 1]`. Kept as its own function so
/// the easing curve is easy to swap.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// World-space camera translation for standing on `pos`.
fn eye_translation(pos: GridPos) -> Vec3 {
    Vec3::new(
        pos.x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        EYE_HEIGHT,
        pos.y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
    )
}

/// Spawn the first-person camera at the dungeon's start placement and register
/// the `Player` / `MoveAnim` resources. A `PointLight` is parented to the camera
/// so it follows the player automatically.
pub fn setup_player(mut commands: Commands, dungeon: Res<Dungeon>) {
    let player = Player {
        pos: dungeon.start_pos,
        facing: dungeon.start_facing,
    };
    let transform = Transform::from_translation(eye_translation(player.pos))
        .with_rotation(Quat::from_rotation_y(player.facing.yaw()));

    commands
        .spawn((
            PlayerCamera,
            Camera3d::default(),
            // Msaa off is friendlier to the software Vulkan (lavapipe) path.
            Msaa::Off,
            // One cluster for the whole view: with the default clustering, the
            // player's point light (whose sphere contains the camera) only gets
            // assigned to the screen-center clusters on this renderer, leaving a
            // bright screen-space rectangle. A single dungeon light makes
            // clustering pointless anyway.
            ClusterConfig::Single,
            Projection::Perspective(PerspectiveProjection {
                fov: FOV_DEGREES.to_radians(),
                ..default()
            }),
            transform,
        ))
        .with_children(|parent| {
            parent.spawn((
                PointLight {
                    // Gentle local pool that adds shape near the player; the
                    // ambient fill (render::setup_dungeon) carries base
                    // visibility so distant tiles don't sink to black.
                    intensity: 120_000.0,
                    range: 22.0,
                    shadows_enabled: false,
                    ..default()
                },
                // Slightly above eye level so floor and walls both catch light.
                Transform::from_xyz(0.0, 0.3, 0.0),
            ));
        });

    commands.insert_resource(player);
    commands.insert_resource(MoveAnim::default());
}

/// Read WASD/QE and produce at most one action this frame.
///
/// Uses *held* state (not just-pressed) so that holding a key keeps feeding the
/// input buffer and walking continues without re-tapping. Priority order is
/// fixed; only one action is emitted per frame.
fn desired_action(keys: &ButtonInput<KeyCode>) -> Option<Action> {
    if keys.pressed(KeyCode::KeyW) {
        Some(Action::Forward)
    } else if keys.pressed(KeyCode::KeyS) {
        Some(Action::Backward)
    } else if keys.pressed(KeyCode::KeyA) {
        Some(Action::StrafeLeft)
    } else if keys.pressed(KeyCode::KeyD) {
        Some(Action::StrafeRight)
    } else if keys.pressed(KeyCode::KeyQ) {
        Some(Action::TurnLeft)
    } else if keys.pressed(KeyCode::KeyE) {
        Some(Action::TurnRight)
    } else {
        None
    }
}

/// Grid step `(dx, dy)` for a movement action given the current facing.
/// Returns `None` for turn actions.
fn move_delta(action: Action, facing: Facing) -> Option<(i32, i32)> {
    let (fx, fy) = facing.delta();
    // Right vector relative to facing (facing rotated 90° clockwise).
    let (rx, ry) = facing.turn_right().delta();
    match action {
        Action::Forward => Some((fx, fy)),
        Action::Backward => Some((-fx, -fy)),
        Action::StrafeRight => Some((rx, ry)),
        Action::StrafeLeft => Some((-rx, -ry)),
        Action::TurnLeft | Action::TurnRight => None,
    }
}

/// Try to begin `action`. Movement is rejected (and no animation starts) if the
/// destination tile is a wall or off the map; turns always succeed. Returns
/// whether an animation was started.
fn start_action(
    action: Action,
    player: &mut Player,
    anim: &mut MoveAnim,
    dungeon: &Dungeon,
) -> bool {
    let from_pos = eye_translation(player.pos);
    let from_yaw = player.facing.yaw();

    match move_delta(action, player.facing) {
        Some((dx, dy)) => {
            let (nx, ny) = (player.pos.x + dx, player.pos.y + dy);
            let floor = dungeon.current_floor();
            let walkable = floor.get(nx, ny).is_some_and(|b| b.is_walkable());
            if !walkable {
                return false;
            }
            player.pos = GridPos::new(nx, ny, player.pos.floor);
            anim.from_pos = from_pos;
            anim.to_pos = eye_translation(player.pos);
            anim.from_yaw = from_yaw;
            anim.to_yaw = from_yaw;
        }
        None => {
            // Turn: rotate the facing and animate a ±90° yaw sweep. Using an
            // explicit signed delta (not the two canonical yaws) keeps the sweep
            // going the visually correct way regardless of angle wrapping.
            let (new_facing, delta_yaw) = match action {
                Action::TurnLeft => (player.facing.turn_left(), FRAC_PI_2),
                Action::TurnRight => (player.facing.turn_right(), -FRAC_PI_2),
                _ => unreachable!(),
            };
            player.facing = new_facing;
            anim.from_pos = from_pos;
            anim.to_pos = from_pos;
            anim.from_yaw = from_yaw;
            anim.to_yaw = from_yaw + delta_yaw;
        }
    }

    anim.active = true;
    anim.elapsed = 0.0;
    true
}

/// Drive input, animation, and the camera transform each frame.
pub fn player_movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    dungeon: Res<Dungeon>,
    mut player: ResMut<Player>,
    mut anim: ResMut<MoveAnim>,
    mut cameras: Query<&mut Transform, With<PlayerCamera>>,
) {
    let Ok(mut transform) = cameras.get_single_mut() else {
        return;
    };

    // 1. Advance the current animation.
    let mut just_finished = false;
    if anim.active {
        anim.elapsed += time.delta_secs();
        let t = ease(anim.elapsed / STEP_DURATION);
        let pos = anim.from_pos.lerp(anim.to_pos, t);
        let yaw = anim.from_yaw + (anim.to_yaw - anim.from_yaw) * t;
        *transform = Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw));

        if anim.elapsed >= STEP_DURATION {
            anim.active = false;
            just_finished = true;
            // Snap exactly to the canonical logical pose to erase any drift.
            *transform = Transform::from_translation(eye_translation(player.pos))
                .with_rotation(Quat::from_rotation_y(player.facing.yaw()));
        }
    }

    // 2. Sample input for this frame.
    let input = desired_action(&keys);

    // 3. Dispatch: start immediately when idle, otherwise remember one press.
    if anim.active {
        if let Some(action) = input {
            anim.buffered = Some(action);
        }
    } else {
        // Idle (possibly having just finished): consume the buffer first, else
        // the current key. This makes held keys chain seamlessly.
        let next = if just_finished {
            anim.buffered.take().or(input)
        } else {
            input
        };
        anim.buffered = None;
        if let Some(action) = next {
            start_action(action, &mut player, &mut anim, &dungeon);
        }
    }
}
