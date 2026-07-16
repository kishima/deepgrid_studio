use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_2;

use bevy::pbr::ClusterConfig;
use bevy::prelude::*;

use crate::dungeon::{Dungeon, DoorStates, Facing, GridPos};
use crate::event::MoveMode;
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
    /// Press the block ahead: keyhole / switch / read a writable wall (plan8).
    Interact,
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
    /// Set on teleport / level transition (plan10): held movement keys are
    /// ignored until released, so walking into stairs while holding W doesn't
    /// carry one step forward on arrival — where the return stairs usually sit
    /// — and bounce straight back. A fresh key press still works immediately.
    require_release: bool,
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

    /// Clear any in-flight animation (plan8: teleport / level transition snaps).
    /// Held movement keys stop repeating until released (plan10 stairs fix).
    pub fn reset(&mut self) {
        self.segments.clear();
        self.elapsed = 0.0;
        self.buffered = None;
        self.input_locked = false;
        self.pending_fall = 0;
        self.require_release = true;
    }
}

/// Snap the camera to the player's canonical pose when the `Player` resource
/// changes with no animation in flight — i.e. after a teleport / level
/// transition (plan8), where no movement animation ran.
pub fn snap_camera_on_teleport(
    player: Res<Player>,
    anim: Res<MoveAnim>,
    mut cameras: Query<&mut Transform, With<PlayerCamera>>,
) {
    if !player.is_changed() || !anim.is_idle() {
        return;
    }
    if let Ok(mut t) = cameras.get_single_mut() {
        *t = canonical_transform(&player);
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
            crate::world::PlayScoped,
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
                    intensity: crate::magic::BASE_LIGHT_INTENSITY,
                    range: crate::magic::BASE_LIGHT_RANGE,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, 0.3, 0.0),
                // Tagged so the lighting-spell boost (magic.rs) can find it.
                crate::magic::PlayerLight,
            ));
        });

    commands.insert_resource(player);
    commands.insert_resource(MoveAnim::default());
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
fn start_move(action: Action, free: bool, player: &mut Player, anim: &mut MoveAnim, dungeon: &Dungeon, doors: &DoorStates) {
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

    // Then fall if the tile has no footing — unless free-flying (plan8).
    if !free {
        let land = level.landing_floor(dest_pos.x, dest_pos.y, dest_pos.floor);
        if land < dest_pos.floor {
            push_fall(anim, dest_pos.x, dest_pos.y, dest_pos.floor, land, from_yaw);
            player.pos.floor = land;
        }
    }
}

/// Begin a ladder climb up or down, per plan2's height/support rules. Rejected
/// (no animation) if the move isn't allowed.
fn start_climb(up: bool, free: bool, player: &mut Player, anim: &mut MoveAnim, dungeon: &Dungeon, doors: &DoorStates) {
    let level = &dungeon.level;
    // Free flight (plan8 MoveMode::Free) rises/descends anywhere in bounds.
    if free {
        let tf = if up {
            let t = player.pos.floor + 1;
            if t >= level.floor_count() { return; }
            t
        } else {
            if player.pos.floor == 0 { return; }
            player.pos.floor - 1
        };
        let from_pos = eye_of(player.pos);
        let yaw = player.facing.yaw();
        player.pos.floor = tf;
        anim.push(Segment { from_pos, to_pos: eye_of(player.pos), from_yaw: yaw, to_yaw: yaw, duration: CLIMB_DURATION, ease: Ease::InOut });
        return;
    }
    // Must be standing on a ladder (or vertical horoscope) to climb.
    if !level.block_at(player.pos).is_some_and(|b| b.is_ladder() || matches!(b, crate::dungeon::Block::HoroscopeVert { .. })) {
        return;
    }

    let target_floor = if up {
        let tf = player.pos.floor + 1;
        // Up requires the cell above to admit an upward climb (a ladder, or a
        // vertical horoscope allowing the upward direction).
        if tf >= level.floor_count() {
            return;
        }
        if !level
            .block_at(GridPos::new(player.pos.x, player.pos.y, tf))
            .is_some_and(|b| b.allows_climb(true))
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
        // Down requires the cell below to admit a downward move: a ladder, a
        // vertical horoscope allowing down, or an otherwise-enterable cell.
        let ok = match below {
            Some(b) if matches!(b, crate::dungeon::Block::HoroscopeVert { .. }) => b.allows_climb(false),
            Some(b) => b.is_ladder() || b.allows_vertical(doors),
            None => false,
        };
        if !ok {
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
/// Returns the door's new state (`Some(open)`) when one was toggled, so the
/// caller can pick the open/close SE (plan10).
fn toggle_front_door(player: &Player, dungeon: &Dungeon, doors: &mut DoorStates) -> Option<bool> {
    let (dx, dy) = player.facing.delta();
    let front = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
    if let Some(crate::dungeon::Block::Door { kind }) = dungeon.level.block_at(front) {
        doors.toggle(kind);
        Some(doors.is_open(kind))
    } else {
        None
    }
}

/// The door SE for its new state (plan10).
fn door_se(open: bool) -> crate::audio::Se {
    if open { crate::audio::Se::DoorOpen } else { crate::audio::Se::DoorClose }
}

fn start_command(cmd: Command, free: bool, player: &mut Player, anim: &mut MoveAnim, dungeon: &Dungeon, doors: &mut DoorStates) {
    match cmd {
        Command::Move(action) => start_move(action, free, player, anim, dungeon, doors),
        Command::ClimbUp => start_climb(true, free, player, anim, dungeon, doors),
        Command::ClimbDown => start_climb(false, free, player, anim, dungeon, doors),
        Command::ToggleDoor => {
            toggle_front_door(player, dungeon, doors);
        }
        // Handled instantly in `player_movement` before reaching here.
        Command::Get
        | Command::ToggleData
        | Command::Attack
        | Command::Guard
        | Command::Concentrate
        | Command::Throw
        | Command::Steal
        | Command::Interact => {}
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
    /// Which data-screen tab to show (plan7): the M key jumps to the magic tab.
    data_view: ResMut<'w, crate::game_state::DataView>,
    /// "Press the block ahead" requests (plan8 keyhole / switch / writable wall).
    interact: EventWriter<'w, crate::event::FrontInteract>,
    /// Party movement mode (plan8): Free ignores footing, Locked refuses moves.
    move_mode: Res<'w, crate::event::MoveMode>,
    /// Rebindable movement keys (plan9).
    keybinds: Res<'w, crate::settings::Keybinds>,
    /// Sound-effect requests (plan10: footsteps, door, landing).
    se: EventWriter<'w, crate::audio::PlaySe>,
    /// Top-level screen (plan12): play input/animation only advance while
    /// `Playing` — the title and demo screens freeze them.
    screen: Res<'w, State<crate::screen::GameScreen>>,
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

    // Title / demo (plan12): freeze all play input/animation; those screens own
    // the display until they close. The data screen is `Playing` + `data.open`
    // (an overlay that keeps simulating), so it is handled below, not here.
    if *events.screen.get() != crate::screen::GameScreen::Playing {
        return;
    }

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
                    events.se.send(crate::audio::PlaySe(crate::audio::Se::Land));
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
        // M jumps straight to the magic tab (plan7): open the data screen if
        // needed and switch its view to magic.
        if keys.just_pressed(KeyCode::KeyM) {
            data.open = true;
            events.data_view.magic = true;
        }
        if !data.open {
            if keys.just_pressed(KeyCode::Space) {
                // Space multiplexes: attack a monster ahead, else toggle a door
                // and press any keyhole / switch / writable wall (plan8).
                let front = front_tile(&player);
                if occ.contains(front) {
                    events.combat.send(PlayerAction::Attack);
                } else {
                    if let Some(open) = toggle_front_door(&player, &dungeon, &mut doors) {
                        events.se.send(crate::audio::PlaySe(door_se(open)));
                    }
                    events.interact.send(crate::event::FrontInteract { pos: front });
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
        if let Some(cmd) = events.keybinds.command_for(|k| keys.just_pressed(k)).filter(|_| can_buffer) {
            anim.buffered = Some(cmd);
        }
    } else {
        // After a teleport/transition, held keys re-arm only once fully released
        // (a fresh press below still moves — just_pressed implies pressed).
        if anim.require_release && events.keybinds.command_for(|k| keys.pressed(k)).is_none() {
            anim.require_release = false;
        }
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
                .or_else(|| {
                    let held_ok = !anim.require_release;
                    events.keybinds.command_for(|k| {
                        if held_ok { keys.pressed(k) } else { keys.just_pressed(k) }
                    })
                })
        };
        if next.is_some() {
            // Any deliberate command (fresh press / icon / script) re-arms holds.
            anim.require_release = false;
        }
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
                Command::Interact if !data.open => {
                    events.interact.send(crate::event::FrontInteract { pos: front_tile(&player) });
                }
                // World commands are ignored while the data screen is open.
                _ if data.open => {}
                // Movement is refused entirely while locked (cutscene, plan8).
                Command::Move(_) | Command::ClimbUp | Command::ClimbDown
                    if matches!(*events.move_mode, MoveMode::Locked) => {}
                // A translation is refused when any member is overloaded or a
                // monster blocks the destination; turns are always allowed.
                Command::Move(action) if move_facing(action, player.facing).is_some() => {
                    let dir = move_facing(action, player.facing).unwrap();
                    let (dx, dy) = dir.delta();
                    let dest = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
                    let free = matches!(*events.move_mode, MoveMode::Free);
                    if !free && let Some(idx) = party.overweight_member(&catalog) {
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
                        let before = player.pos;
                        start_command(cmd, free, &mut player, &mut anim, &dungeon, &mut doors);
                        if player.pos != before {
                            events.se.send(crate::audio::PlaySe(crate::audio::Se::Footstep));
                        }
                    }
                }
                // A door toggle is instant; the SE fires only when a door was hit.
                Command::ToggleDoor => {
                    if let Some(open) = toggle_front_door(&player, &dungeon, &mut doors) {
                        events.se.send(crate::audio::PlaySe(door_se(open)));
                    }
                }
                _ => {
                    *overweight_warned = false;
                    let free = matches!(*events.move_mode, MoveMode::Free);
                    let before = player.pos;
                    start_command(cmd, free, &mut player, &mut anim, &dungeon, &mut doors);
                    if player.pos != before {
                        events.se.send(crate::audio::PlaySe(crate::audio::Se::Footstep));
                    }
                }
            }
        }
    }
}
