//! DeepGrid Studio — plan3: project format + minimal map editor.
//!
//! `main.rs` parses the CLI, loads the project, and dispatches to play mode
//! (the 3D runtime) or edit mode (the egui map editor). All logic lives in the
//! modules.

mod audio;
mod autotest;
mod character;
mod clock;
mod combat;
mod config;
mod data_screen;
mod debug_shot;
mod demo;
mod dungeon;
mod editor;
mod event;
mod floor_items;
mod game_state;
mod hazard;
mod hud;
mod hunger;
mod item;
mod magic;
mod monster;
mod player;
mod portrait;
mod project;
mod render;
mod rng;
mod rules;
mod save;
mod settings;
mod world;

use std::path::PathBuf;

use bevy::prelude::*;

use clock::{CycleTick, GameClock};
use floor_items::{InitialItems, PickupRequest};
use game_state::{DataScreen, SelectedMember};
use hud::MessageLog;
use monster::{
    AttackRotation, InitialMonsters, MonsterOccupancy, PartyWiped, PlayerAction,
};
use player::{PlayerFell, ScriptedInput};
use project::Project;
use rng::GameRng;

/// Default project loaded when `--project` is not given.
const DEFAULT_PROJECT: &str = "assets/projects/sample";

/// Parsed command line (plan3「起動モード」; plan10 adds `--load <slot>`).
struct Cli {
    edit: bool,
    project_dir: PathBuf,
    load_slot: Option<usize>,
}

fn parse_cli() -> Cli {
    let mut edit = false;
    let mut project_dir = PathBuf::from(DEFAULT_PROJECT);
    let mut load_slot = None;
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
            "--load" => {
                load_slot = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|s| (1..=save::SLOTS).contains(s));
                if load_slot.is_none() {
                    panic!("--load requires a slot number 1..={}", save::SLOTS);
                }
            }
            other => eprintln!("deepgrid_studio: ignoring unknown argument '{other}'"),
        }
    }
    Cli { edit, project_dir, load_slot }
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
        // plan10 `override` scene: drop a temporary wall-texture override into
        // the project (the ceiling rock — visibly different from the bricks)
        // so the shot proves the swap, then clean it up so other scenes and
        // the repo stay untouched.
        let override_scene = debug_shot::debug_shot_value().as_deref() == Some("override");
        let override_file = cli.project_dir.join("override/textures/wall_bricks066_color.png");
        if override_scene {
            if let Some(parent) = override_file.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::copy("assets/textures/ceiling_rock058_color.png", &override_file).ok();
        }
        run_play(project, cli.load_slot);
        if override_scene {
            std::fs::remove_file(&override_file).ok();
            let _ = std::fs::remove_dir(cli.project_dir.join("override/textures"));
            let _ = std::fs::remove_dir(cli.project_dir.join("override"));
        }
    }
}

/// Build and run the play-mode app: load the project's level 0 into the 3D
/// runtime (plan1/plan2 systems).
fn run_play(project: Project, load_slot: Option<usize>) {
    // Doors start closed unless the level's `!`/`@` glyphs mark a kind open (v6).
    let doors = world::doors_for(&project.levels[0], None, project.limits.door_kinds_per_level);
    let dungeon = project.levels[0].to_dungeon();
    let party = project.build_party();
    let catalog = project.build_catalog();
    let monster_catalog = project.build_monster_catalog();
    let magic_catalog = project.build_magic_catalog();
    let initial_items = InitialItems(project.levels[0].items.clone());
    let initial_monsters = InitialMonsters(project.levels[0].monsters.clone());
    let mut event_flags = event::EventFlags::new(project.limits.event_flags);
    for &f in &project.initial_flags {
        event_flags.set(f, true); // plan9: initial-on flags
    }
    let game_levels = world::GameLevels { levels: project.levels.clone() };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
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
    .insert_resource(party)
    .insert_resource(catalog)
    .insert_resource(monster_catalog)
    .insert_resource(magic_catalog)
    .insert_resource(project.rules.clone())
    .insert_resource(initial_items)
    .insert_resource(initial_monsters)
    .insert_resource(ScriptedInput::default())
    .insert_resource(settings::Keybinds::load())
    .insert_resource(settings::UserSettings::load())
    .insert_resource(project::AssetResolver { project_dir: project.dir.clone() })
    // plan10: audio + demos.
    .init_resource::<audio::BgmState>()
    .init_resource::<demo::DemoState>()
    .insert_resource(demo::DemoCatalog(project.demos.clone()))
    .insert_resource(save::PendingCliLoad(load_slot))
    .init_resource::<world::SkipNextSnapshot>()
    .add_event::<audio::PlaySe>()
    .add_event::<demo::StartDemoReq>()
    .add_event::<save::SaveRequest>()
    .add_event::<save::LoadRequest>()
    .init_resource::<settings::KeyConfig>()
    .insert_resource(MessageLog::default())
    .insert_resource(GameClock::default())
    .init_resource::<DataScreen>()
    .init_resource::<SelectedMember>()
    .init_resource::<MonsterOccupancy>()
    .init_resource::<AttackRotation>()
    .init_resource::<PartyWiped>()
    .init_resource::<monster::EnemyNear>()
    .init_resource::<hud::IconMove>()
    .init_resource::<GameRng>()
    .init_resource::<magic::LightBoost>()
    .init_resource::<magic::SelectedMagic>()
    .init_resource::<game_state::DataView>()
    // plan8: events / gimmicks / multi-level.
    .insert_resource(event_flags)
    .insert_resource(game_levels)
    .init_resource::<event::EventQueue>()
    .init_resource::<event::TriggerStates>()
    .init_resource::<event::MoveMode>()
    .init_resource::<world::CurrentLevel>()
    .init_resource::<world::LevelStates>()
    .init_resource::<event::WallWrites>()
    .add_event::<PlayerFell>()
    .add_event::<CycleTick>()
    .add_event::<PickupRequest>()
    .add_event::<PlayerAction>()
    .add_event::<magic::CastMagic>()
    .add_event::<event::FrontInteract>()
    .add_event::<event::WallWriteRequest>()
    .add_event::<render::TileDirty>()
    .add_event::<world::LevelTransition>();
    data_screen::init(&mut app);

    app.add_systems(
        Startup,
        (
            render::setup_dungeon,
            player::setup_player,
            floor_items::setup_floor_items,
            monster::setup_monsters,
            debug_shot::setup_debug_script,
            hud::greet,
            // Portraits build the render-target images the HUD cards show, so
            // the HUD must spawn after them.
            (portrait::setup_portraits, hud::setup_hud).chain(),
        ),
    )
    .add_systems(
        Update,
        (
            player::player_movement,
            render::update_door_visibility,
            portrait::attach_portrait_anim,
            floor_items::spin_floor_items,
            debug_shot::debug_screenshot,
        ),
    )
    // Monster display + occupancy (frame-rate driven visuals).
    .add_systems(
        Update,
        (
            monster::update_occupancy,
            monster::update_enemy_near,
            monster::build_monster_graphs,
            monster::bind_monster_players,
            monster::drive_monster_anim,
            monster::interpolate_monsters,
        ),
    )
    // Cycle time: tick the clock, then run every on-cycle system this frame.
    .add_systems(
        Update,
        (
            clock::tick_clock,
            clock::recover_concentration,
            character::tick_effects,
            hazard::hazard_tick,
            hunger::hunger_tick,
            data_screen::rest_tick,
            monster::monster_ai,
            monster::monster_attacks,
            monster::monster_lifecycle,
        )
            .chain(),
    )
    // Pickup/place/combat read this frame's requests (written by movement / the
    // data screen), so order them after their producers.
    .add_systems(
        Update,
        (
            character::apply_fall_damage.after(player::player_movement),
            floor_items::handle_pickup.after(player::player_movement),
            monster::player_actions.after(player::player_movement),
            data_screen::data_screen_interactions,
            data_screen::data_screen_drag.after(data_screen::data_screen_interactions),
            data_screen::data_magic_interactions.after(data_screen::data_screen_interactions),
            floor_items::handle_place.after(data_screen::data_screen_interactions),
            data_screen::save_slot_clicks,
            data_screen::toggle_data_screen,
            data_screen::refresh_data_screen.after(data_screen::toggle_data_screen),
            data_screen::refresh_magic_screen.after(data_screen::toggle_data_screen),
        ),
    )
    // Magic runtime (plan7): drive casts requested by the magic tab / M key /
    // debug driver, animate light-bullets, and decay the lighting boost.
    .add_systems(
        Update,
        (
            magic::debug_magic_driver,
            magic::cast_magic
                .after(magic::debug_magic_driver)
                .after(data_screen::data_magic_interactions)
                .after(monster::player_actions),
            magic::drive_player_light.after(magic::cast_magic),
            magic::animate_projectiles,
        ),
    )
    // plan8: events / gimmicks / multi-level. Triggers read this frame's player
    // position + interact events; run_events drives the queue on cycle ticks;
    // transitions and mesh rebuilds follow.
    .add_systems(
        Update,
        (
            event::front_interact.after(player::player_movement),
            event::entry_triggers.after(player::player_movement),
            event::apply_wall_write.before(event::front_interact),
            event::debug_gimmick_driver.before(event::entry_triggers),
            event::run_events
                .after(event::front_interact)
                .after(event::entry_triggers)
                .after(monster::monster_lifecycle),
            // plan10: saves. Save freezes after this frame's game systems; load
            // rewrites globals then rebuilds via the normal transition below.
            save::handle_save
                .after(event::run_events)
                .after(monster::monster_lifecycle),
            save::handle_load.after(save::handle_save).before(world::level_transition),
            world::level_transition
                .after(event::run_events)
                .after(event::entry_triggers),
            render::rebuild_dirty_tiles
                .after(event::run_events)
                .after(world::level_transition),
            player::snap_camera_on_teleport
                .after(event::run_events)
                .after(world::level_transition)
                .after(player::player_movement),
        ),
    )
    // plan10: audio (SE after their producers, BGM follows level/override) and
    // demo playback (start requests come from run_events).
    .add_systems(
        Update,
        (
            audio::sync_level_bgm.after(world::level_transition),
            audio::update_bgm.after(audio::sync_level_bgm).after(demo::drive_demo),
            audio::play_se
                .after(player::player_movement)
                .after(monster::player_actions)
                .after(monster::monster_attacks)
                .after(magic::cast_magic)
                .after(magic::animate_projectiles)
                .after(floor_items::handle_pickup)
                .after(event::run_events)
                .after(character::apply_fall_damage),
            audio::level_up_se
                .after(monster::player_actions)
                .after(monster::monster_lifecycle)
                .before(audio::play_se),
            demo::debug_demo_driver,
            demo::start_demo.after(event::run_events).after(demo::debug_demo_driver),
            demo::drive_demo.after(demo::start_demo),
        ),
    )
    .add_systems(
        Update,
        (
            hud::update_status_bars,
            hud::update_cards,
            hud::update_messages,
            hud::action_icon_clicks,
            hud::move_icon_clicks,
            hud::magic_button_clicks,
            settings::keyconfig_input,
            portrait::freeze_portraits,
        ),
    );

    // Unattended acceptance tests (DEEPGRID_AUTOTEST=1): inject subjects before
    // the floor items spawn, then drive/assert after the frame's game systems.
    if autotest::enabled() {
        app.init_resource::<autotest::AutoTest>()
            .add_systems(
                Startup,
                autotest::prepare.before(floor_items::setup_floor_items),
            )
            .add_systems(
                Update,
                autotest::run
                    .after(floor_items::handle_place)
                    .after(hazard::hazard_tick),
            )
            .add_systems(
                Update,
                autotest::run_combat
                    .after(autotest::run)
                    .after(monster::player_actions)
                    .after(monster::monster_attacks)
                    .after(monster::monster_lifecycle),
            )
            .add_systems(
                Update,
                autotest::run_hunger
                    .after(autotest::run_combat)
                    .after(hunger::hunger_tick)
                    .after(clock::recover_concentration)
                    .after(data_screen::rest_tick),
            )
            .add_systems(
                Update,
                autotest::run_magic
                    .after(autotest::run_hunger)
                    .after(magic::cast_magic)
                    .after(magic::drive_player_light)
                    .after(character::tick_effects),
            )
            .add_systems(
                Update,
                autotest::run_gimmick
                    .after(autotest::run_magic)
                    .after(event::run_events)
                    .after(world::level_transition)
                    .after(event::entry_triggers),
            );
    }

    app.run();
}
