use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::player::{Action, Command, MoveAnim, ScriptedInput};

/// The verification scene selected by `DEEPGRID_DEBUG_SHOT`.
///
/// Every non-`Start` scene runs a scripted command sequence through the real
/// movement logic (no teleport shortcuts), then screenshots once the scene has
/// settled. The command sequences are tuned to `assets/maps/test_level.ron`.
fn scene_script(value: &str) -> VecDeque<Command> {
    use Action::*;
    use Command::*;
    let steps: Vec<Command> = match value {
        // Start placement: look across the pit at the floor below.
        "1" => vec![],
        // Step East off the ledge and drop into the floor-0 room.
        "fall" => vec![Move(Forward)],
        // Turn South, walk to the ladder, climb one floor up to floor 2.
        "ladder" => vec![Move(TurnRight), Move(Forward), Move(Forward), Move(Forward), ClimbUp],
        // Turn to face the door and open it (kind 0).
        "door" => vec![Move(TurnRight), Move(TurnRight), ToggleDoor],
        // Turn South to face the sentinel monster (skel_guard) + floor items.
        "monster" => vec![Move(TurnRight)],
        // Step up to the sentinel and attack it (shows a combat message).
        "combat" => vec![Move(TurnRight), Move(Forward), Attack],
        // Turn South to face the floor items (sword, chest, barrel, glow stone).
        "items" => vec![Move(TurnRight)],
        // Step onto the sword tile to the south and pick it up.
        "pickup" => vec![Move(TurnRight), Move(Forward), Get],
        // Pick the sword up, then open the data screen over it.
        "data" => vec![Move(TurnRight), Move(Forward), Get, ToggleData],
        // Walk the corridor to the water tile and stand in it (takes cycle damage).
        "liquid" => vec![
            Move(TurnRight),
            Move(Forward),
            Move(Forward),
            Move(TurnLeft),
            Move(Forward),
            Move(Forward),
            Move(Forward),
            Move(TurnRight),
            Move(Forward),
        ],
        // Unknown value: fall back to the start scene.
        _ => vec![],
    };
    steps.into()
}

/// The raw `DEEPGRID_DEBUG_SHOT` value, if debug-shot mode is on (any non-empty
/// value enables it).
pub fn debug_shot_value() -> Option<String> {
    match std::env::var("DEEPGRID_DEBUG_SHOT") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Whether debug-shot mode is enabled at all.
pub fn debug_shot_enabled() -> bool {
    debug_shot_value().is_some()
}

/// Whether the requested scene is the editor screen (`DEEPGRID_DEBUG_SHOT=editor`),
/// which forces edit mode regardless of `--edit`. Editor capture lives in
/// `editor::shot` (egui on the window isn't captured by Bevy screenshots).
pub fn wants_editor() -> bool {
    debug_shot_value().as_deref() == Some("editor")
}

/// Startup: if a debug-shot scene is requested, load its command script so the
/// player auto-runs it. `ScriptedInput` starts inactive otherwise.
pub fn setup_debug_script(mut script: ResMut<ScriptedInput>) {
    let Some(value) = debug_shot_value() else {
        return;
    };
    script.queue = scene_script(&value);
    script.active = true;
}

/// Verification driver (`DEEPGRID_DEBUG_SHOT=…`): wait until the scripted scene
/// has finished and rendering has settled for ~30 idle frames, save
/// `debug-shot.png` to the repo root, then exit. Mirrors plan1's approach (Bevy
/// 0.15 `Screenshot` / `save_to_disk`) but is gated on the script completing.
pub fn debug_screenshot(
    mut idle_frames: Local<u32>,
    mut shot_frame: Local<Option<u32>>,
    mut total: Local<u32>,
    script: Res<ScriptedInput>,
    anim: Res<MoveAnim>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
) {
    if !debug_shot_enabled() {
        return;
    }
    *total += 1;
    let idle = script.queue.is_empty() && anim.is_idle();

    match *shot_frame {
        None => {
            if idle {
                *idle_frames += 1;
            } else {
                *idle_frames = 0;
            }
            if *idle_frames >= 30 {
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk("debug-shot.png"));
                *shot_frame = Some(*total);
            }
        }
        // A few more frames so the async save completes, then quit.
        Some(f) => {
            if *total >= f + 15 {
                exit.send(AppExit::Success);
            }
        }
    }
}
