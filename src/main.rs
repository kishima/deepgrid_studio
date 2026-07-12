//! DeepGrid Studio — plan3: project format + minimal map editor.
//!
//! `main.rs` parses the CLI, loads the project, and dispatches to play mode
//! (the 3D runtime) or edit mode (the egui map editor). All logic lives in the
//! modules.

mod config;
mod debug_shot;
mod dungeon;
mod editor;
mod player;
mod project;
mod props;
mod render;

use std::path::PathBuf;

use bevy::prelude::*;

use dungeon::DoorStates;
use player::ScriptedInput;
use project::Project;

/// Default project loaded when `--project` is not given.
const DEFAULT_PROJECT: &str = "assets/projects/sample";

/// Parsed command line (plan3「起動モード」).
struct Cli {
    edit: bool,
    project_dir: PathBuf,
}

fn parse_cli() -> Cli {
    let mut edit = false;
    let mut project_dir = PathBuf::from(DEFAULT_PROJECT);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--edit" => edit = true,
            "--project" => {
                project_dir = args
                    .next()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| panic!("--project requires a directory argument"));
            }
            other => eprintln!("deepgrid_studio: ignoring unknown argument '{other}'"),
        }
    }
    Cli { edit, project_dir }
}

fn main() {
    let cli = parse_cli();
    let project = project::load_project(&cli.project_dir).unwrap_or_else(|e| {
        panic!("failed to load project {}: {e}", cli.project_dir.display())
    });

    // `DEEPGRID_DEBUG_SHOT=editor` forces edit mode so the editor screen can be
    // captured without also passing `--edit`.
    if cli.edit || debug_shot::wants_editor() {
        editor::run(project);
    } else {
        run_play(project);
    }
}

/// Build and run the play-mode app: load the project's level 0 into the 3D
/// runtime (plan1/plan2 systems).
fn run_play(project: Project) {
    let doors = DoorStates::new(project.limits.door_kinds_per_level);
    let dungeon = project.levels[0].to_dungeon();

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
        .insert_resource(project.limits.clone())
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
