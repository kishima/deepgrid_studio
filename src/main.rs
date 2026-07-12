//! DeepGrid Studio — plan2: multi-floor dungeon with falling, ladders and doors.
//!
//! `main.rs` only assembles the Bevy `App`; all logic lives in the modules.

mod config;
mod debug_shot;
mod dungeon;
mod player;
mod props;
mod render;

use bevy::prelude::*;

use config::LimitsConfig;
use dungeon::DoorStates;
use player::ScriptedInput;

/// Path (relative to the working directory / asset root) of the test map.
const TEST_MAP_PATH: &str = "assets/maps/test_level.ron";

fn main() {
    // Limits are the single source of truth for map sizing and door kinds; the
    // loader validates the test map against them.
    let limits = LimitsConfig::default();
    let dungeon = dungeon::load_dungeon(TEST_MAP_PATH, &limits);
    let doors = DoorStates::new(limits.door_kinds_per_level);

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DeepGrid Studio".to_string(),
                ..default()
            }),
            ..default()
        }))
        // Dark clear color: unlit ceiling/void reads as dungeon gloom.
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .insert_resource(limits)
        .insert_resource(dungeon)
        .insert_resource(doors)
        .insert_resource(ScriptedInput::default())
        .add_systems(
            Startup,
            (
                render::setup_dungeon,
                player::setup_player,
                props::setup_props,
                debug_shot::setup_debug_script,
            ),
        )
        .add_systems(
            Update,
            (
                player::player_movement,
                render::update_door_visibility,
                props::attach_prop_animations,
                debug_shot::debug_screenshot,
            ),
        )
        .run();
}
