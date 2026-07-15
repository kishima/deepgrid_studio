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
        // Turn South to face the sentinel, then cast an attack spell at it
        // (the actual cast is fired by magic::debug_magic_driver once settled).
        "magic" => vec![Move(TurnRight)],
        // Cast a lighting spell in place (driver handles the cast); the scene
        // just needs the world to settle, so no movement.
        "light" => vec![],
        // Brew a potion and open the data screen (driver handles it).
        "potion" => vec![],
        // plan8 gimmick scenes: the driver teleports to a viewpoint on the
        // autotest fixtures once the world settles (no scripted movement).
        "plate" | "warp" | "stairs" | "hole" => vec![],
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

/// Whether the requested scene is an editor screen (`DEEPGRID_DEBUG_SHOT=editor`
/// or `editor-<tab>`), which forces edit mode regardless of `--edit`. Editor
/// capture lives in `editor::shot` (egui on the window isn't captured by Bevy
/// screenshots).
pub fn wants_editor() -> bool {
    editor_shot_tab().is_some() || wants_editor_3d()
}

/// Whether the requested scene is the 3D-edit-mode Bevy screenshot
/// (`DEEPGRID_DEBUG_SHOT=editor-3d`, plan9.5). Handled outside `editor_shot_tab`
/// because it captures the 3D view via Bevy's `Screenshot`, not the egui image.
pub fn wants_editor_3d() -> bool {
    debug_shot_value().as_deref() == Some("editor-3d")
}

/// Which editor tab a `DEEPGRID_DEBUG_SHOT=editor[-tab]` scene opens, or `None`
/// for a non-editor scene (plan9). Bare `editor` = the map tab.
pub fn editor_shot_tab() -> Option<crate::editor::Tab> {
    use crate::editor::Tab;
    match debug_shot_value().as_deref() {
        Some("editor") | Some("editor-map") => Some(Tab::Map),
        Some("editor-chars") => Some(Tab::Characters),
        Some("editor-items") => Some(Tab::Items),
        Some("editor-monsters") => Some(Tab::Monsters),
        Some("editor-magics") => Some(Tab::Magics),
        Some("editor-events") => Some(Tab::Events),
        Some("editor-settings") => Some(Tab::Settings),
        _ => None,
    }
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
