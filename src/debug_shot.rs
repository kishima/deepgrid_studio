use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

/// Whether debug-shot mode is enabled (`DEEPGRID_DEBUG_SHOT=1`).
pub fn debug_shot_enabled() -> bool {
    std::env::var("DEEPGRID_DEBUG_SHOT").as_deref() == Ok("1")
}

/// Verification helper (`DEEPGRID_DEBUG_SHOT=1`): let rendering settle for a few
/// frames, save `debug-shot.png` to the repo root, then exit. Mirrors
/// mycity-simulator's equivalent (Bevy 0.15 `Screenshot` / `save_to_disk`).
pub fn debug_screenshot(
    mut frames: Local<u32>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
) {
    if !debug_shot_enabled() {
        return;
    }
    *frames += 1;
    // ~30 frames for the renderer to stabilize before capturing.
    if *frames == 30 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("debug-shot.png"));
    }
    // A few more frames so the screenshot's async save completes, then quit.
    if *frames == 45 {
        exit.send(AppExit::Success);
    }
}
