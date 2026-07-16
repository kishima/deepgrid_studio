//! DeepGrid Studio — plan3: project format + minimal map editor.
//!
//! `main.rs` parses the CLI (and `deepgrid.ron`, the plan11 distribution
//! config), loads the project, and dispatches to play mode (the 3D runtime) or
//! edit mode (the egui map editor). All logic lives in the modules.

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
mod perf;
mod player;
mod portrait;
mod project;
mod render;
mod rng;
mod rules;
mod save;
mod screen;
mod settings;
mod title;
mod world;

use std::path::PathBuf;

use bevy::prelude::*;
use serde::Deserialize;

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

const HELP: &str = "\
DeepGrid Studio — だんだんダンジョン オマージュ (Rust + Bevy)

使い方: deepgrid_studio [オプション]

オプション:
  --edit             エディターを開く (配布版 play_only では不可)
  --project <dir>    読み込むプロジェクトディレクトリ (既定: assets/projects/sample)
  --load <slot>      セーブスロット 1..=3 から直接再開 (タイトルを出さない)
  --help             このヘルプを表示して終了

起動設定ファイル:
  ./deepgrid.ron     ( play_only: true, project: \"assets/projects/<name>\" )
                     があるとプロジェクトを固定。play_only では --edit と
                     タイトルの「ゲームを選ぶ」を無効化 (配布用, plan11)

環境変数:
  DEEPGRID_DEBUG_SHOT=<scene>  検証シーンを描画して debug-shot.png を出力し終了
                               (シーン一覧は README「検証用スクリーンショット」)
  DEEPGRID_AUTOTEST=1          無人受け入れテストを実行して終了
  DEEPGRID_PERF=<secs|1>       起動後の平均/最悪フレーム時間を出力して終了
  DEEPGRID_WINDOW=<WxH>        起動時ウィンドウサイズ (既定 1280x720。
                               lavapipe はフィルレート律速のため docker の
                               対話プレイは 960x540 が既定)
  WGPU_BACKEND=<backend>       レンダラー指定 (docker 経路は vulkan/lavapipe)
";

/// Parsed command line (plan3「起動モード」; plan10 adds `--load <slot>`,
/// plan11 adds `--help`).
struct Cli {
    edit: bool,
    project_dir: Option<PathBuf>,
    load_slot: Option<usize>,
    help: bool,
}

fn parse_cli(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli { edit: false, project_dir: None, load_slot: None, help: false };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--edit" => cli.edit = true,
            "--help" | "-h" => cli.help = true,
            "--project" => {
                cli.project_dir = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--project requires a directory argument")?,
                );
            }
            "--load" => {
                let slot = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|s| (1..=save::SLOTS).contains(s))
                    .ok_or(format!("--load requires a slot number 1..={}", save::SLOTS))?;
                cli.load_slot = Some(slot);
            }
            other => eprintln!(
                "deepgrid_studio: ignoring unknown argument '{other}' (--help for usage)"
            ),
        }
    }
    Ok(cli)
}

/// `./deepgrid.ron` — the plan11 distribution launch config. Absent = normal
/// development launch.
#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
struct LaunchConfig {
    play_only: bool,
    project: String,
}

impl LaunchConfig {
    fn load() -> Self {
        let Ok(text) = std::fs::read_to_string("deepgrid.ron") else {
            return Self::default();
        };
        match ron::from_str(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("deepgrid_studio: deepgrid.ron is broken, ignoring it: {e}");
                Self::default()
            }
        }
    }
}

/// How this process should run, resolved from the CLI + `deepgrid.ron`.
/// Pure so the play_only refusal rules are unit-testable (plan11).
fn resolve_mode(cli_edit: bool, cli_project: Option<PathBuf>, launch: &LaunchConfig)
-> (bool, PathBuf, Option<String>) {
    let mut warning = None;
    let mut edit = cli_edit;
    let project_dir = if launch.project.is_empty() {
        cli_project.unwrap_or_else(|| PathBuf::from(DEFAULT_PROJECT))
    } else {
        if let Some(p) = cli_project
            && p.as_os_str() != launch.project.as_str()
        {
            warning = Some(format!(
                "deepgrid.ron がプロジェクトを {} に固定しているため --project は無視します",
                launch.project
            ));
        }
        PathBuf::from(&launch.project)
    };
    if launch.play_only && edit {
        warning = Some(
            "この配布版ではエディターは使えません (play_only)。プレイモードで起動します"
                .into(),
        );
        edit = false;
    }
    (edit, project_dir, warning)
}

fn main() {
    let cli = match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("deepgrid_studio: {e}");
            std::process::exit(1);
        }
    };
    if cli.help {
        print!("{HELP}");
        return;
    }
    let launch = LaunchConfig::load();
    let (edit, project_dir, warning) = resolve_mode(cli.edit, cli.project_dir, &launch);
    if let Some(w) = warning {
        eprintln!("deepgrid_studio: {w}");
    }

    // The title shows on a normal play launch. Unattended verification
    // (autotest / debug shots except the `title` scene / perf) and `--load`
    // go straight to the game, exactly as before plan11.
    let title_scene = debug_shot::debug_shot_value().as_deref() == Some("title");
    let unattended =
        autotest::enabled() || perf::enabled() || (debug_shot::debug_shot_enabled() && !title_scene);
    let show_title = !edit && !debug_shot::wants_editor() && cli.load_slot.is_none() && !unattended;

    // Project-load failures no longer panic (plan11): play mode falls back to a
    // stub world and shows the error on the title; anything that can't show a
    // title reports to stderr and exits 1.
    let (project, load_error) = match project::load_project(&project_dir) {
        Ok(p) => (p, None),
        Err(e) => {
            eprintln!("deepgrid_studio: failed to load project {}: {e}", project_dir.display());
            if !show_title || edit || debug_shot::wants_editor() {
                std::process::exit(1);
            }
            (Project::fallback(project_dir.clone()), Some(e))
        }
    };

    // `DEEPGRID_DEBUG_SHOT=editor` forces edit mode so the editor screen can be
    // captured without also passing `--edit`.
    if edit || debug_shot::wants_editor() {
        editor::run(project);
    } else {
        // plan10 `override` scene: drop a temporary wall-texture override into
        // the project (the ceiling rock — visibly different from the bricks)
        // so the shot proves the swap, then clean it up so other scenes and
        // the repo stay untouched.
        let override_scene = debug_shot::debug_shot_value().as_deref() == Some("override");
        let override_file = project_dir.join("override/textures/wall_bricks066_color.png");
        if override_scene {
            if let Some(parent) = override_file.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::copy("assets/textures/ceiling_rock058_color.png", &override_file).ok();
        }
        run_play(project, cli.load_slot, PlayLaunch {
            show_title,
            load_error,
            play_only: launch.play_only,
        });
        if override_scene {
            std::fs::remove_file(&override_file).ok();
            let _ = std::fs::remove_dir(project_dir.join("override/textures"));
            let _ = std::fs::remove_dir(project_dir.join("override"));
        }
    }
}

/// Title-related launch options for play mode (plan11).
struct PlayLaunch {
    show_title: bool,
    load_error: Option<String>,
    play_only: bool,
}

/// Parse `"WxH"` (e.g. `960x540`), rejecting nonsense sizes.
fn parse_window_size(v: &str) -> Option<(f32, f32)> {
    let (w, h) = v.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    ((320..=7680).contains(&w) && (200..=4320).contains(&h)).then_some((w as f32, h as f32))
}

/// The initial window size: `DEEPGRID_WINDOW=WxH` or Bevy's default 1280x720.
/// The plan11 lavapipe measurement showed frame time scales with pixel count,
/// so the docker run script uses this to size interactive play down.
fn window_resolution() -> bevy::window::WindowResolution {
    std::env::var("DEEPGRID_WINDOW")
        .ok()
        .and_then(|v| parse_window_size(&v))
        .map(|(w, h)| bevy::window::WindowResolution::new(w, h))
        .unwrap_or_default()
}

/// Build and run the play-mode app: load the project's level 0 into the 3D
/// runtime (plan1/plan2 systems).
fn run_play(project: Project, load_slot: Option<usize>, launch: PlayLaunch) {
    // Doors start closed unless the level's `!`/`@` glyphs mark a kind open (v6).
    let doors = world::doors_for(&project.levels[0], None, project.limits.door_kinds_per_level);
    let dungeon = project.levels[0].to_dungeon();
    let party = project.build_party();
    // The pristine run captured for「はじめから」/ ED-demo resets (plan11).
    let initial_run = title::InitialRun {
        party: character::Party { members: party.members.clone() },
        initial_flags: project.initial_flags.clone(),
        start: project.levels[0].start,
        facing: project.levels[0].start_facing,
    };
    let title_state = title::TitleState::new(
        launch.load_error.is_none(),
        launch.load_error,
        project.name.clone(),
        project.author.clone(),
        project.description.clone(),
    );
    // plan12: the title/demo/play screens are a Bevy `State`. A normal launch
    // opens on the title; unattended verification and `--load` start in Playing
    // (exactly as the pre-plan12 `TitleState.active` flag decided).
    let initial_screen = if launch.show_title {
        screen::GameScreen::Title
    } else {
        screen::GameScreen::Playing
    };
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
            // plan11 計測: lavapipe はフィルレート律速(解像度にほぼ比例)。
            // docker の対話プレイは run スクリプトが DEEPGRID_WINDOW=960x540 を
            // 既定にして 20fps 目標を満たす。検証ショット/autotest は従来の
            // 既定 1280x720 のまま。
            resolution: window_resolution(),
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
    // plan11: title screen + run reset. plan12: the screen is a Bevy `State`.
    .insert_state(initial_screen)
    .insert_resource(title_state)
    .insert_resource(title::PlayOnly(launch.play_only))
    .insert_resource(initial_run)
    .add_event::<title::ResetRunReq>()
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
            demo::drive_demo
                .after(demo::start_demo)
                .run_if(in_state(screen::GameScreen::Demo)),
        ),
    )
    .add_systems(OnExit(screen::GameScreen::Demo), demo::teardown_demo)
    // plan11: title screen. Menu actions run before the reset/load/demo systems
    // that consume their events this same frame; the reset rebuilds through the
    // normal level transition.
    // plan11: title screen. drive_title / title_buttons / sync_title_ui only run
    // while in the Title state (plan12); apply_reset stays ungated — it is
    // event-driven (ResetRunReq) and fires while transitioning out of Demo.
    .add_systems(
        Update,
        (
            title::drive_title
                .before(demo::start_demo)
                .before(save::handle_load)
                .run_if(in_state(screen::GameScreen::Title)),
            title::title_buttons
                .after(title::drive_title)
                .before(demo::start_demo)
                .before(save::handle_load)
                .run_if(in_state(screen::GameScreen::Title)),
            title::apply_reset
                .after(title::title_buttons)
                .after(demo::drive_demo)
                .before(world::level_transition),
            title::sync_title_ui
                .after(title::title_buttons)
                .after(demo::drive_demo)
                .run_if(in_state(screen::GameScreen::Title)),
        ),
    )
    .add_systems(OnExit(screen::GameScreen::Title), title::teardown_title)
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

    // Frame-time measurement mode (plan11): print avg/worst and exit.
    if perf::enabled() {
        app.add_systems(Update, perf::measure);
    }

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
            )
            // plan11 title steps: key injections must land before the systems
            // that consume the edges this same frame (ordering it after the
            // other autotest systems would cycle through level_transition).
            .add_systems(
                Update,
                autotest::run_title
                    .before(title::drive_title)
                    .before(demo::start_demo),
            );
    }

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> impl Iterator<Item = String> + use<> {
        a.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn cli_parses_all_options() {
        let cli = parse_cli(args(&["--edit", "--project", "p", "--load", "2"])).unwrap();
        assert!(cli.edit);
        assert_eq!(cli.project_dir.as_deref(), Some(std::path::Path::new("p")));
        assert_eq!(cli.load_slot, Some(2));
        assert!(parse_cli(args(&["--help"])).unwrap().help);
    }

    #[test]
    fn cli_rejects_bad_load_without_panicking() {
        assert!(parse_cli(args(&["--load", "9"])).is_err());
        assert!(parse_cli(args(&["--load"])).is_err());
        assert!(parse_cli(args(&["--project"])).is_err());
    }

    #[test]
    fn launch_config_parses_and_defaults() {
        let cfg: LaunchConfig =
            ron::from_str(r#"( play_only: true, project: "assets/projects/sample" )"#).unwrap();
        assert!(cfg.play_only);
        assert_eq!(cfg.project, "assets/projects/sample");
        let empty: LaunchConfig = ron::from_str("()").unwrap();
        assert_eq!(empty, LaunchConfig::default());
    }

    #[test]
    fn play_only_politely_refuses_edit_and_pins_the_project() {
        let launch = LaunchConfig { play_only: true, project: "assets/projects/game".into() };
        // --edit is downgraded to play with a warning.
        let (edit, dir, warning) = resolve_mode(true, None, &launch);
        assert!(!edit);
        assert_eq!(dir, PathBuf::from("assets/projects/game"));
        assert!(warning.is_some());
        // --project is overridden by the pinned project.
        let (_, dir, warning) = resolve_mode(false, Some(PathBuf::from("elsewhere")), &launch);
        assert_eq!(dir, PathBuf::from("assets/projects/game"));
        assert!(warning.is_some());
        // Without deepgrid.ron nothing changes.
        let (edit, dir, warning) = resolve_mode(true, None, &LaunchConfig::default());
        assert!(edit);
        assert_eq!(dir, PathBuf::from(DEFAULT_PROJECT));
        assert!(warning.is_none());
    }

    #[test]
    fn window_size_parses_and_rejects_nonsense() {
        assert_eq!(parse_window_size("960x540"), Some((960.0, 540.0)));
        assert_eq!(parse_window_size("1280X720"), Some((1280.0, 720.0)));
        assert_eq!(parse_window_size("0x0"), None);
        assert_eq!(parse_window_size("abc"), None);
        assert_eq!(parse_window_size("99999x2"), None);
    }
}
