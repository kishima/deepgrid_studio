use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_2;

use bevy::pbr::ClusterConfig;
use bevy::prelude::*;

use crate::dungeon::{Dungeon, DoorStates, Facing, GridPos};
use crate::render::BLOCK_SIZE;

/// Duration of one step or one 90° turn, in seconds
/// (project.md「0.2〜0.3秒程度のイージング付きアニメーション」).
pub const STEP_DURATION: f32 = 0.25;
/// Duration of one floor of ladder climbing (smooth vertical interpolation).
pub const CLIMB_DURATION: f32 = 0.25;
/// Duration of one floor of falling. Each floor is a separate ease-in segment so
/// a multi-floor drop reads as an accelerating tumble (plan2 Step 4).
pub const FALL_DURATION: f32 = 0.15;

/// Camera eye height above the floor surface (block center height).
pub const EYE_HEIGHT: f32 = 0.5;

/// Vertical field of view in degrees.
pub const FOV_DEGREES: f32 = 78.0;

/// A discrete grid movement / turn. The logical state jumps a whole tile or 90°;
/// only the camera is interpolated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Forward,
    Backward,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
}

/// A player command: a grid move, a ladder climb, a door toggle, an item
/// pick-up, or a data-screen toggle. This is the single vocabulary shared by
/// keyboard input and the debug scripts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Move(Action),
    ClimbUp,
    ClimbDown,
    ToggleDoor,
    /// Pick up the item under/ahead of the party (plan5).
    Get,
    /// Toggle the data screen (plan5).
    ToggleData,
    /// Attack the monster ahead (plan6).
    Attack,
    /// Guard: halve the next incoming hit (plan6).
    Guard,
    /// Concentrate: 5× concentration recovery until acting (plan6).
    Concentrate,
    /// Throw the selected member's held item ahead (plan6).
    Throw,
    /// Steal from the monster ahead (plan6).
    Steal,
}

/// Emitted once when a fall finishes, carrying how many floors were dropped
/// (≥1). Fall damage (character.rs) reads this; the animation itself stays in
/// real seconds.
#[derive(Event, Clone, Copy)]
pub struct PlayerFell {
    pub floors: u32,
}

/// Marker for the first-person camera entity.
#[derive(Component)]
pub struct PlayerCamera;

/// The player's logical state: which tile they stand on and which way they face.
/// Authoritative; the camera transform chases it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Player {
    pub pos: GridPos,
    pub facing: Facing,
}

/// Easing curve for an animation segment.
#[derive(Clone, Copy)]
enum Ease {
    /// Smoothstep — steps, turns, ladder climbs.
    InOut,
    /// Quadratic ease-in — falling, for an accelerating feel.
    In,
}

/// One leg of an animation: interpolate the camera from one pose to another.
#[derive(Clone, Copy)]
struct Segment {
    from_pos: Vec3,
    to_pos: Vec3,
    from_yaw: f32,
    to_yaw: f32,
    duration: f32,
    ease: Ease,
}

/// The in-progress animation (a queue of segments) plus the one-slot input
/// buffer.
///
/// Most commands enqueue a single segment; a fall enqueues one segment per floor
/// dropped, played back-to-back. While `input_locked` (during a fall) input is
/// ignored and the buffer cleared, so the player can't act mid-plummet
/// (project.md / plan2 Step 4).
#[derive(Resource, Default)]
pub struct MoveAnim {
    segments: VecDeque<Segment>,
    elapsed: f32,
    buffered: Option<Command>,
    input_locked: bool,
    /// Floors dropped by the fall currently in flight; a `PlayerFell` fires with
    /// this when the animation finishes, then it resets to 0.
    pending_fall: u32,
}

impl MoveAnim {
    fn is_animating(&self) -> bool {
        !self.segments.is_empty()
    }

    /// No animation in flight — used by the debug-shot driver to know the scene
    /// has settled.
    pub fn is_idle(&self) -> bool {
        self.segments.is_empty()
    }

    fn push(&mut self, seg: Segment) {
        self.segments.push_back(seg);
    }
}

/// A queued sequence of commands that drives the player automatically, standing
/// in for keyboard input during a `DEEPGRID_DEBUG_SHOT` scene.
#[derive(Resource, Default)]
pub struct ScriptedInput {
    pub queue: VecDeque<Command>,
    pub active: bool,
}

fn ease(kind: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        Ease::InOut => t * t * (3.0 - 2.0 * t),
        Ease::In => t * t,
    }
}

/// World-space camera translation for standing on `(x, y)` of `floor`.
fn eye_at(x: i32, y: i32, floor: usize) -> Vec3 {
    Vec3::new(
        x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        floor as f32 * BLOCK_SIZE + EYE_HEIGHT,
        y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
    )
}

fn eye_of(pos: GridPos) -> Vec3 {
    eye_at(pos.x, pos.y, pos.floor)
}

fn canonical_transform(player: &Player) -> Transform {
    Transform::from_translation(eye_of(player.pos))
        .with_rotation(Quat::from_rotation_y(player.facing.yaw()))
}

/// Spawn the first-person camera at the dungeon's start placement and register
/// the `Player` / `MoveAnim` resources. A `PointLight` is parented to the camera
/// so it follows the player automatically.
pub fn setup_player(mut commands: Commands, dungeon: Res<Dungeon>) {
    let player = Player {
        pos: dungeon.start_pos,
        facing: dungeon.start_facing,
    };

    commands
        .spawn((
            PlayerCamera,
            Camera3d::default(),
            // Msaa off is friendlier to the software Vulkan (lavapipe) path.
            Msaa::Off,
            // One cluster for the whole view: with default clustering the
            // player's point light (whose sphere contains the camera) only gets
            // assigned to screen-center clusters on this renderer, leaving a
            // bright screen-space rectangle. A single dungeon light makes
            // clustering pointless anyway.
            ClusterConfig::Single,
            Projection::Perspective(PerspectiveProjection {
                fov: FOV_DEGREES.to_radians(),
                ..default()
            }),
            canonical_transform(&player),
        ))
        .with_children(|parent| {
            parent.spawn((
                PointLight {
                    intensity: 120_000.0,
                    range: 22.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, 0.3, 0.0),
            ));
        });

    commands.insert_resource(player);
    commands.insert_resource(MoveAnim::default());
}

/// Map the movement/climb keys to at most one command via `is_active` (usually
/// `pressed` or `just_pressed`). Door toggling is handled separately (edge-only)
/// so holding the key can't flap the door.
fn desired_command(mut is_active: impl FnMut(KeyCode) -> bool) -> Option<Command> {
    // Arrow keys mirror WASD/QE with a classic-crawler layout (user feedback
    // 2026-07-14): ↑/↓ = forward/backward, ←/→ = turn in place (strafing stays
    // on A/D only).
    if is_active(KeyCode::KeyW) || is_active(KeyCode::ArrowUp) {
        Some(Command::Move(Action::Forward))
    } else if is_active(KeyCode::KeyS) || is_active(KeyCode::ArrowDown) {
        Some(Command::Move(Action::Backward))
    } else if is_active(KeyCode::KeyA) {
        Some(Command::Move(Action::StrafeLeft))
    } else if is_active(KeyCode::KeyD) {
        Some(Command::Move(Action::StrafeRight))
    } else if is_active(KeyCode::KeyQ) || is_active(KeyCode::ArrowLeft) {
        Some(Command::Move(Action::TurnLeft))
    } else if is_active(KeyCode::KeyE) || is_active(KeyCode::ArrowRight) {
        Some(Command::Move(Action::TurnRight))
    } else if is_active(KeyCode::KeyR) {
        Some(Command::ClimbUp)
    } else if is_active(KeyCode::KeyF) {
        Some(Command::ClimbDown)
    } else {
        None
    }
}

/// The cardinal heading a movement action produces given the current facing, or
/// `None` for turns.
fn move_facing(action: Action, facing: Facing) -> Option<Facing> {
    match action {
        Action::Forward => Some(facing),
        Action::Backward => Some(facing.opposite()),
        Action::StrafeRight => Some(facing.turn_right()),
        Action::StrafeLeft => Some(facing.turn_left()),
        Action::TurnLeft | Action::TurnRight => None,
    }
}

/// Enqueue fall segments dropping straight down `(x, y)` from `from_floor` to
/// `to_floor`, one per floor, holding `yaw` (the player doesn't rotate while
/// falling), and flag the animation as input-locked.
fn push_fall(anim: &mut MoveAnim, x: i32, y: i32, from_floor: usize, to_floor: usize, yaw: f32) {
    for f in (to_floor..from_floor).rev() {
        anim.push(Segment {
            from_pos: eye_at(x, y, f + 1),
            to_pos: eye_at(x, y, f),
            from_yaw: yaw,
            to_yaw: yaw,
            duration: FALL_DURATION,
            ease: Ease::In,
        });
    }
    anim.input_locked = true;
    anim.buffered = None;
    anim.pending_fall = (from_floor - to_floor) as u32;
}

/// Begin a grid move. Rejected (no animation) if the exit/entry rules forbid it.
/// A successful entry into an unsupported cell chains a fall.
fn start_move(action: Action, player: &mut Player, anim: &mut MoveAnim, dungeon: &Dungeon, doors: &DoorStates) {
    let level = &dungeon.level;
    let from_pos = eye_of(player.pos);
    let from_yaw = player.facing.yaw();

    let Some(dir) = move_facing(action, player.facing) else {
        // Turn in place.
        let (new_facing, delta_yaw) = match action {
            Action::TurnLeft => (player.facing.turn_left(), FRAC_PI_2),
            Action::TurnRight => (player.facing.turn_right(), -FRAC_PI_2),
            _ => unreachable!("non-turn action has a move_facing"),
        };
        player.facing = new_facing;
        anim.push(Segment {
            from_pos,
            to_pos: from_pos,
            from_yaw,
            to_yaw: from_yaw + delta_yaw,
            duration: STEP_DURATION,
            ease: Ease::InOut,
        });
        return;
    };

    // Leaving the current cell (only a one-way horoscope constrains this)...
    let here = level.block_at(player.pos);
    if !here.is_some_and(|b| b.allows_exit(dir)) {
        return;
    }
    // ...and entering the destination.
    let (dx, dy) = dir.delta();
    let dest_pos = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
    if !level.block_at(dest_pos).is_some_and(|b| b.allows_enter(dir, doors)) {
        return;
    }

    // Horizontal step first (at the entering floor).
    player.pos = dest_pos;
    anim.push(Segment {
        from_pos,
        to_pos: eye_of(dest_pos),
        from_yaw,
        to_yaw: from_yaw,
        duration: STEP_DURATION,
        ease: Ease::InOut,
    });

    // Then fall if the tile has no footing.
    let land = level.landing_floor(dest_pos.x, dest_pos.y, dest_pos.floor);
    if land < dest_pos.floor {
        push_fall(anim, dest_pos.x, dest_pos.y, dest_pos.floor, land, from_yaw);
        player.pos.floor = land;
    }
}

/// Begin a ladder climb up or down, per plan2's height/support rules. Rejected
/// (no animation) if the move isn't allowed.
fn start_climb(up: bool, player: &mut Player, anim: &mut MoveAnim, dungeon: &Dungeon, doors: &DoorStates) {
    let level = &dungeon.level;
    // Must be standing on a ladder to climb.
    if !level.block_at(player.pos).is_some_and(|b| b.is_ladder()) {
        return;
    }

    let target_floor = if up {
        let tf = player.pos.floor + 1;
        // Up requires the ceiling to be open, i.e. the cell directly above is a
        // ladder to continue on.
        if tf >= level.floor_count() {
            return;
        }
        if !level
            .block_at(GridPos::new(player.pos.x, player.pos.y, tf))
            .is_some_and(|b| b.is_ladder())
        {
            return;
        }
        tf
    } else {
        if player.pos.floor == 0 {
            return;
        }
        let tf = player.pos.floor - 1;
        let below = level.block_at(GridPos::new(player.pos.x, player.pos.y, tf));
        // Down requires the cell below to be a ladder or otherwise enterable.
        if !below.is_some_and(|b| b.is_ladder() || b.allows_vertical(doors)) {
            return;
        }
        tf
    };

    let from_pos = eye_of(player.pos);
    let yaw = player.facing.yaw();
    player.pos.floor = target_floor;
    anim.push(Segment {
        from_pos,
        to_pos: eye_of(player.pos),
        from_yaw: yaw,
        to_yaw: yaw,
        duration: CLIMB_DURATION,
        ease: Ease::InOut,
    });

    // Climbing down onto an unsupported (non-ladder) cell keeps falling.
    let land = level.landing_floor(player.pos.x, player.pos.y, player.pos.floor);
    if land < player.pos.floor {
        push_fall(anim, player.pos.x, player.pos.y, player.pos.floor, land, yaw);
        player.pos.floor = land;
    }
}

/// Toggle the door kind of the cell directly in front of the player, if any.
/// Instant (no animation) — the original opens doors by kind, not individually.
fn toggle_front_door(player: &Player, dungeon: &Dungeon, doors: &mut DoorStates) {
    let (dx, dy) = player.facing.delta();
    let front = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
    if let Some(crate::dungeon::Block::Door { kind }) = dungeon.level.block_at(front) {
        doors.toggle(kind);
    }
}

fn start_command(cmd: Command, player: &mut Player, anim: &mut MoveAnim, dungeon: &Dungeon, doors: &mut DoorStates) {
    match cmd {
        Command::Move(action) => start_move(action, player, anim, dungeon, doors),
        Command::ClimbUp => start_climb(true, player, anim, dungeon, doors),
        Command::ClimbDown => start_climb(false, player, anim, dungeon, doors),
        Command::ToggleDoor => toggle_front_door(player, dungeon, doors),
        // Handled instantly in `player_movement` before reaching here.
        Command::Get
        | Command::ToggleData
        | Command::Attack
        | Command::Guard
        | Command::Concentrate
        | Command::Throw
        | Command::Steal => {}
    }
}

/// The action-event writers, bundled so `player_movement` stays within Bevy's
/// 16-parameter system limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ActionEvents<'w> {
    pickup: EventWriter<'w, crate::floor_items::PickupRequest>,
    combat: EventWriter<'w, crate::monster::PlayerAction>,
    /// Movement command buffered by a move-icon click (hud.rs).
    icon_move: ResMut<'w, crate::hud::IconMove>,
}

/// Drive input, animation, and the camera transform each frame.
// A Bevy system's parameters are its dependency list; splitting it up to satisfy
// the arg-count lint would only obscure that.
#[allow(clippy::too_many_arguments)]
pub fn player_movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    dungeon: Res<Dungeon>,
    mut doors: ResMut<DoorStates>,
    mut player: ResMut<Player>,
    mut anim: ResMut<MoveAnim>,
    mut script: ResMut<ScriptedInput>,
    mut fell: EventWriter<PlayerFell>,
    party: Res<crate::character::Party>,
    catalog: Res<crate::item::ItemCatalog>,
    mut log: ResMut<crate::hud::MessageLog>,
    mut events: ActionEvents,
    occ: Res<crate::monster::MonsterOccupancy>,
    mut data: ResMut<crate::game_state::DataScreen>,
    mut overweight_warned: Local<bool>,
    mut cameras: Query<&mut Transform, With<PlayerCamera>>,
) {
    use crate::floor_items::PickupRequest;
    use crate::monster::PlayerAction;

    let front_tile = |p: &Player| {
        let (dx, dy) = p.facing.delta();
        GridPos::new(p.pos.x + dx, p.pos.y + dy, p.pos.floor)
    };

    let Ok(mut transform) = cameras.get_single_mut() else {
        return;
    };

    // 1. Advance the current animation segment.
    if let Some(seg) = anim.segments.front().copied() {
        anim.elapsed += time.delta_secs();
        let t = ease(seg.ease, anim.elapsed / seg.duration);
        let pos = seg.from_pos.lerp(seg.to_pos, t);
        let yaw = seg.from_yaw + (seg.to_yaw - seg.from_yaw) * t;
        *transform = Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw));

        if anim.elapsed >= seg.duration {
            anim.segments.pop_front();
            anim.elapsed = 0.0;
            if anim.segments.is_empty() {
                anim.input_locked = false;
                // A fall just landed — announce it so fall damage can apply.
                if anim.pending_fall > 0 {
                    fell.send(PlayerFell {
                        floors: anim.pending_fall,
                    });
                    anim.pending_fall = 0;
                }
                // Snap exactly to the canonical logical pose to erase drift.
                *transform = canonical_transform(&player);
            }
        }
    }

    // 2. Instant, edge-triggered keyboard actions (ignored while a script drives
    //    input). Tab/I toggle the data screen anytime; the rest are suppressed
    //    while it's open.
    if !script.active {
        if keys.just_pressed(KeyCode::Tab) || keys.just_pressed(KeyCode::KeyI) {
            data.open = !data.open;
        }
        if !data.open {
            if keys.just_pressed(KeyCode::Space) {
                // Space multiplexes: attack a monster ahead, else toggle a door.
                if occ.contains(front_tile(&player)) {
                    events.combat.send(PlayerAction::Attack);
                } else {
                    toggle_front_door(&player, &dungeon, &mut doors);
                }
            }
            if keys.just_pressed(KeyCode::KeyG) {
                events.pickup.send(PickupRequest);
            }
            if keys.just_pressed(KeyCode::KeyB) {
                events.combat.send(PlayerAction::Guard);
            }
            if keys.just_pressed(KeyCode::KeyC) {
                events.combat.send(PlayerAction::Concentrate);
            }
            if keys.just_pressed(KeyCode::KeyT) {
                events.combat.send(PlayerAction::Throw);
            }
            if keys.just_pressed(KeyCode::KeyV) {
                events.combat.send(PlayerAction::Steal);
            }
        }
    }

    // 3. Dispatch the next command.
    if anim.is_animating() {
        // Buffer only keys newly pressed *during* the animation (buffering the
        // held state would turn a short tap into two steps). Never buffer while
        // falling or with the data screen up.
        let can_buffer = !script.active && !anim.input_locked && !data.open;
        if let Some(cmd) = desired_command(|k| keys.just_pressed(k)).filter(|_| can_buffer) {
            anim.buffered = Some(cmd);
        }
    } else {
        let next = if script.active {
            script.queue.pop_front()
        } else if data.open {
            None
        } else {
            events
                .icon_move
                .0
                .take()
                .or_else(|| anim.buffered.take())
                .or_else(|| desired_command(|k| keys.pressed(k)))
        };
        if let Some(cmd) = next {
            match cmd {
                // Inventory / screen commands are instant and world-independent.
                Command::Get => {
                    if !data.open {
                        events.pickup.send(PickupRequest);
                    }
                }
                Command::ToggleData => data.open = !data.open,
                // Combat commands become `PlayerAction` events (ignored while the
                // data screen is up).
                Command::Attack if !data.open => {
                    events.combat.send(PlayerAction::Attack);
                }
                Command::Guard if !data.open => {
                    events.combat.send(PlayerAction::Guard);
                }
                Command::Concentrate if !data.open => {
                    events.combat.send(PlayerAction::Concentrate);
                }
                Command::Throw if !data.open => {
                    events.combat.send(PlayerAction::Throw);
                }
                Command::Steal if !data.open => {
                    events.combat.send(PlayerAction::Steal);
                }
                // World commands are ignored while the data screen is open.
                _ if data.open => {}
                // A translation is refused when any member is overloaded or a
                // monster blocks the destination; turns are always allowed.
                Command::Move(action) if move_facing(action, player.facing).is_some() => {
                    let dir = move_facing(action, player.facing).unwrap();
                    let (dx, dy) = dir.delta();
                    let dest = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
                    if let Some(idx) = party.overweight_member(&catalog) {
                        if !*overweight_warned {
                            log.push(format!(
                                "{}は動けない! 荷物が重すぎる",
                                party.members[idx].character.first_name
                            ));
                            *overweight_warned = true;
                        }
                    } else if occ.contains(dest) {
                        // A monster blocks the way — attack it instead of moving.
                        events.combat.send(PlayerAction::Attack);
                    } else {
                        *overweight_warned = false;
                        start_command(cmd, &mut player, &mut anim, &dungeon, &mut doors);
                    }
                }
                _ => {
                    *overweight_warned = false;
                    start_command(cmd, &mut player, &mut anim, &dungeon, &mut doors);
                }
            }
        }
    }
}
