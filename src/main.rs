//! DeepGrid Studio — plan1: a walkable first-person grid dungeon prototype.
//!
//! `main.rs` only assembles the Bevy `App`; all logic lives in the modules.

mod config;
mod debug_shot;
mod dungeon;
mod player;
mod render;

use bevy::prelude::*;

use config::LimitsConfig;

/// Path (relative to the working directory / asset root) of the plan1 test map.
const TEST_MAP_PATH: &str = "assets/maps/test_level.ron";

fn main() {
    // Limits are the single source of truth for map sizing; the loader validates
    // the test map against them rather than assuming a hard-coded 40×40.
    let limits = LimitsConfig::default();
    let dungeon = dungeon::load_dungeon(TEST_MAP_PATH, &limits);

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
        .add_systems(Startup, (render::setup_dungeon, player::setup_player))
        .add_systems(
            Update,
            (player::player_movement, debug_shot::debug_screenshot),
        )
        .run();
}
