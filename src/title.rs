//! Title screen (plan11): a full-screen bevy_ui overlay gated by [`TitleState`]
//! — the same resource-gate pattern as `DemoState` (States migration is plan12;
//! the priority rule lives in [`crate::screen::active_screen`]).
//!
//! Menu (keyboard ↑↓/Enter/Esc + mouse, both complete):
//! はじめから / つづきから (slots with timestamps) / 設定 (live `UserSettings`
//! editing + key rebinding) / クレジット (CREDITS.md read at runtime) /
//! ゲームを選ぶ (sibling-project scan → relaunch self with `--project`) / 終了.
//!
//! Unattended runs (`DEEPGRID_AUTOTEST` / `DEEPGRID_DEBUG_SHOT` except the
//! `title` scene / `--load` / `DEEPGRID_PERF`) start with the title inactive so
//! every pre-plan11 verification path is untouched.

use bevy::app::AppExit;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::ui::FocusPolicy;

use crate::character::Party;
use crate::demo::{DemoCatalog, StartDemoReq};
use crate::dungeon::{Facing, GridPos};
use crate::project::ProjectCard;
use crate::save::{self, LoadRequest};
use crate::settings::{BINDABLE_KEYS, GameAction, Keybinds, UserSettings, key_name};

const FONT_REGULAR: &str = "fonts/PixelMplus12-Regular.ttf";
const FONT_BOLD: &str = "fonts/PixelMplus12-Bold.ttf";

/// Distributed builds (`deepgrid.ron` with `play_only: true`, plan11): the
/// project is fixed and "ゲームを選ぶ" politely refuses.
#[derive(Resource, Default)]
pub struct PlayOnly(pub bool);

/// Which title sub-screen is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TitleScreen {
    #[default]
    Main,
    Continue,
    Settings,
    Credits,
    Games,
}

/// What activating a menu row does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    Start,
    Continue,
    Settings,
    Credits,
    Games,
    Quit,
    /// Load this save slot (1-based).
    Slot(usize),
    BgmVolume,
    SeVolume,
    Mute,
    Footsteps,
    Speed,
    /// Rebind this `GameAction::ORDER` index.
    Key(usize),
    /// Launch `TitleState::games[i]` by relaunching ourselves.
    Game(usize),
    Back,
}

/// One selectable menu row (built by `rebuild_rows`, rendered by
/// `sync_title_ui`, activated by keyboard or mouse).
pub struct Row {
    pub label: String,
    pub enabled: bool,
    pub kind: RowKind,
}

/// The title overlay gate + all its UI state.
#[derive(Resource, Default)]
pub struct TitleState {
    pub active: bool,
    pub screen: TitleScreen,
    pub sel: usize,
    pub rows: Vec<Row>,
    /// Startup load-failure banner (panic elimination, plan11).
    pub error: Option<String>,
    /// False when the fallback project is running (start/continue disabled).
    pub playable: bool,
    /// Shown on the main screen (project.ron v8 metadata).
    pub game_title: String,
    pub game_author: String,
    pub game_desc: String,
    /// CREDITS.md text while the credits screen is up.
    pub credits: Option<String>,
    pub scroll: f32,
    /// Waiting for a key press to rebind this `GameAction::ORDER` index.
    pub rebind: Option<usize>,
    /// Sibling projects found by the games screen.
    pub games: Vec<ProjectCard>,
    /// One-line transient message (refusals, spawn errors).
    pub status: Option<String>,
    /// UI must be rebuilt.
    pub dirty: bool,
}

impl TitleState {
    pub fn new(
        active: bool,
        playable: bool,
        error: Option<String>,
        game_title: String,
        game_author: String,
        game_desc: String,
    ) -> Self {
        Self {
            active,
            playable,
            error,
            game_title,
            game_author,
            game_desc,
            dirty: true,
            ..default()
        }
    }

    /// (Re)open the title on its main screen — used by the ED demo (plan11) and
    /// anything else that returns to the title.
    pub fn open(&mut self) {
        self.active = true;
        self.screen = TitleScreen::Main;
        self.sel = 0;
        self.rows.clear();
        self.rebind = None;
        self.status = None;
        self.dirty = true;
    }

    fn goto(&mut self, screen: TitleScreen) {
        self.screen = screen;
        self.sel = 0;
        self.rows.clear();
        self.scroll = 0.0;
        self.rebind = None;
        self.status = None;
        self.dirty = true;
    }
}

/// Rebuild the world to its authored initial state — the "はじめから" path,
/// also fired when the ED demo closes (plan11「これがゲームクリアの暫定形」).
#[derive(Event, Default)]
pub struct ResetRunReq;

/// The pristine initial run captured at startup, so a reset never depends on
/// what play mutated since.
#[derive(Resource)]
pub struct InitialRun {
    pub party: Party,
    pub initial_flags: Vec<usize>,
    pub start: GridPos,
    pub facing: Facing,
}

/// Marks the whole title overlay (despawned + rebuilt on any state change).
#[derive(Component)]
pub struct TitleOverlay;

/// A clickable menu row (index into `TitleState::rows`).
#[derive(Component)]
pub struct TitleItem(pub usize);

/// Everything a menu activation may touch, bundled for the parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TitleActions<'w> {
    pub settings: ResMut<'w, UserSettings>,
    pub binds: ResMut<'w, Keybinds>,
    pub play_only: Res<'w, PlayOnly>,
    pub demos: Res<'w, DemoCatalog>,
    pub resolver: Res<'w, crate::project::AssetResolver>,
    pub reset: EventWriter<'w, ResetRunReq>,
    pub start_demo: EventWriter<'w, StartDemoReq>,
    pub load: EventWriter<'w, LoadRequest>,
    pub exit: EventWriter<'w, AppExit>,
}

// ------------------------------------------------------------------ rows

/// Percent label for a 0..=1 volume.
fn pct(v: f32) -> String {
    format!("{:3.0}%", (v * 100.0).round())
}

fn on_off(b: bool) -> &'static str {
    if b { "ON" } else { "OFF" }
}

/// Build the current screen's rows from live state (idempotent; called whenever
/// `rows` is empty or a value changed).
fn rebuild_rows(state: &mut TitleState, acts: &TitleActions) {
    let mut rows = Vec::new();
    match state.screen {
        TitleScreen::Main => {
            rows.push(Row {
                label: "はじめから".into(),
                enabled: state.playable,
                kind: RowKind::Start,
            });
            rows.push(Row {
                label: "つづきから".into(),
                enabled: state.playable,
                kind: RowKind::Continue,
            });
            rows.push(Row { label: "設定".into(), enabled: true, kind: RowKind::Settings });
            if std::path::Path::new("CREDITS.md").is_file() {
                rows.push(Row { label: "クレジット".into(), enabled: true, kind: RowKind::Credits });
            }
            rows.push(Row { label: "ゲームを選ぶ".into(), enabled: true, kind: RowKind::Games });
            rows.push(Row { label: "終了".into(), enabled: true, kind: RowKind::Quit });
        }
        TitleScreen::Continue => {
            for slot in 1..=save::SLOTS {
                let path = save::slot_path(&acts.resolver.project_dir, slot);
                let stamp = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(fmt_jst);
                match stamp {
                    Some(s) => rows.push(Row {
                        label: format!("スロット{slot}  {s}"),
                        enabled: true,
                        kind: RowKind::Slot(slot),
                    }),
                    None => rows.push(Row {
                        label: format!("スロット{slot}  (空き)"),
                        enabled: false,
                        kind: RowKind::Slot(slot),
                    }),
                }
            }
            rows.push(Row { label: "もどる".into(), enabled: true, kind: RowKind::Back });
        }
        TitleScreen::Settings => {
            let s = &acts.settings;
            rows.push(Row {
                label: format!("BGM音量   ◀ {} ▶", pct(s.bgm_volume)),
                enabled: true,
                kind: RowKind::BgmVolume,
            });
            rows.push(Row {
                label: format!("SE音量    ◀ {} ▶", pct(s.se_volume)),
                enabled: true,
                kind: RowKind::SeVolume,
            });
            rows.push(Row {
                label: format!("ミュート   {}", on_off(s.mute)),
                enabled: true,
                kind: RowKind::Mute,
            });
            rows.push(Row {
                label: format!("足音       {}", on_off(s.footsteps)),
                enabled: true,
                kind: RowKind::Footsteps,
            });
            rows.push(Row {
                label: format!("ゲーム速度 ◀ {:.1}x ▶", s.speed),
                enabled: true,
                kind: RowKind::Speed,
            });
            for (i, action) in GameAction::ORDER.iter().enumerate() {
                let key = acts
                    .binds
                    .binds
                    .iter()
                    .rev()
                    .find(|(a, _)| a == action)
                    .and_then(|(_, k)| key_name(*k))
                    .unwrap_or("―");
                let marker = if state.rebind == Some(i) { "キーを押してください…" } else { key };
                rows.push(Row {
                    label: format!("キー設定: {}  [{}]", action.label(), marker),
                    enabled: true,
                    kind: RowKind::Key(i),
                });
            }
            rows.push(Row { label: "もどる".into(), enabled: true, kind: RowKind::Back });
        }
        TitleScreen::Games => {
            if state.games.is_empty() {
                state.status = Some("プロジェクトが見つからない".into());
            }
            let current = acts.resolver.project_dir.clone();
            for (i, card) in state.games.iter().enumerate() {
                let here = if card.dir == current { " (現在)" } else { "" };
                let author = if card.author.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", card.author)
                };
                // One-line description tail, truncated to keep rows compact.
                let desc: String = card.description.lines().next().unwrap_or("").chars().take(24).collect();
                let desc = if desc.is_empty() { desc } else { format!("  「{desc}」") };
                rows.push(Row {
                    label: format!("{}{}{}{}", card.name, author, here, desc),
                    enabled: card.dir != current,
                    kind: RowKind::Game(i),
                });
            }
            rows.push(Row { label: "もどる".into(), enabled: true, kind: RowKind::Back });
        }
        TitleScreen::Credits => {}
    }
    state.rows = rows;
    // Land the cursor on an enabled row.
    if !state.rows.is_empty() {
        let n = state.rows.len();
        state.sel = state.sel.min(n - 1);
        if !state.rows[state.sel].enabled {
            state.sel = (0..n).find(|&i| state.rows[i].enabled).unwrap_or(0);
        }
    }
}

/// Move the selection up/down to the next enabled row (wrapping).
fn move_sel(state: &mut TitleState, delta: i32) {
    let n = state.rows.len();
    if n == 0 {
        return;
    }
    let mut i = state.sel as i32;
    for _ in 0..n {
        i = (i + delta).rem_euclid(n as i32);
        if state.rows[i as usize].enabled {
            state.sel = i as usize;
            state.dirty = true;
            return;
        }
    }
}

/// Perform a row's action. Shared by the keyboard and mouse paths.
fn activate(state: &mut TitleState, acts: &mut TitleActions, kind: RowKind) {
    match kind {
        RowKind::Start => {
            acts.reset.send(ResetRunReq);
            if acts.demos.0.iter().any(|d| d.id == "op") {
                acts.start_demo.send(StartDemoReq("op".to_string()));
            }
            state.active = false;
            state.dirty = true;
        }
        RowKind::Continue => state.goto(TitleScreen::Continue),
        RowKind::Settings => state.goto(TitleScreen::Settings),
        RowKind::Credits => {
            match std::fs::read_to_string("CREDITS.md") {
                Ok(text) => {
                    state.credits = Some(text);
                    state.goto(TitleScreen::Credits);
                }
                Err(e) => {
                    state.status = Some(format!("CREDITS.md が読めない: {e}"));
                    state.dirty = true;
                }
            }
        }
        RowKind::Games => {
            if acts.play_only.0 {
                state.status =
                    Some("この配布版ではゲームの切り替えはできません".into());
                state.dirty = true;
                return;
            }
            state.games = crate::project::scan_project_cards(&acts.resolver.project_dir);
            state.goto(TitleScreen::Games);
        }
        RowKind::Quit => {
            acts.exit.send(AppExit::Success);
        }
        RowKind::Slot(slot) => {
            acts.load.send(LoadRequest(slot));
            state.active = false;
            state.dirty = true;
        }
        RowKind::BgmVolume => adjust(state, acts, kind, 1),
        RowKind::SeVolume => adjust(state, acts, kind, 1),
        RowKind::Speed => adjust(state, acts, kind, 1),
        RowKind::Mute => {
            acts.settings.mute = !acts.settings.mute;
            save_settings(state, acts);
        }
        RowKind::Footsteps => {
            acts.settings.footsteps = !acts.settings.footsteps;
            save_settings(state, acts);
        }
        RowKind::Key(i) => {
            state.rebind = Some(i);
            // Rebuild NOW (drive_title skips row-building while a rebind is
            // armed) so the "press a key" marker shows on this row.
            rebuild_rows(state, acts);
            state.dirty = true;
        }
        RowKind::Game(i) => {
            let Some(card) = state.games.get(i) else { return };
            match std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| {
                    std::process::Command::new(exe)
                        .arg("--project")
                        .arg(&card.dir)
                        .spawn()
                        .map_err(|e| e.to_string())
                }) {
                Ok(_) => {
                    acts.exit.send(AppExit::Success);
                }
                Err(e) => {
                    state.status = Some(format!("起動できない: {e}"));
                    state.dirty = true;
                }
            }
        }
        RowKind::Back => state.goto(TitleScreen::Main),
    }
}

/// Left/Right (or Enter-cycle) a value row, save, and refresh the labels.
fn adjust(state: &mut TitleState, acts: &mut TitleActions, kind: RowKind, dir: i32) {
    let step = 0.1 * dir as f32;
    match kind {
        RowKind::BgmVolume => {
            let s = &mut acts.settings;
            s.bgm_volume = cycle_volume(s.bgm_volume, step);
        }
        RowKind::SeVolume => {
            let s = &mut acts.settings;
            s.se_volume = cycle_volume(s.se_volume, step);
        }
        RowKind::Speed => {
            const SPEEDS: [f32; 3] = [0.5, 1.0, 2.0];
            let cur = SPEEDS
                .iter()
                .position(|s| (s - acts.settings.speed).abs() < 0.01)
                .unwrap_or(1) as i32;
            let next = (cur + dir).rem_euclid(SPEEDS.len() as i32) as usize;
            acts.settings.speed = SPEEDS[next];
        }
        RowKind::Mute => {
            acts.settings.mute = !acts.settings.mute;
        }
        RowKind::Footsteps => {
            acts.settings.footsteps = !acts.settings.footsteps;
        }
        _ => return,
    }
    save_settings(state, acts);
}

/// Volume stepping that wraps 100% → 0% when moving up (Enter-cycle friendly).
fn cycle_volume(v: f32, step: f32) -> f32 {
    let next = v + step;
    if step > 0.0 && next > 1.001 {
        0.0
    } else {
        next.clamp(0.0, 1.0)
    }
}

fn save_settings(state: &mut TitleState, acts: &mut TitleActions) {
    if let Err(e) = acts.settings.save() {
        state.status = Some(format!("設定の保存に失敗: {e}"));
    }
    state.rows.clear();
    state.dirty = true;
}

// ------------------------------------------------------------------ systems

/// Keyboard navigation + all menu actions.
pub fn drive_title(
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: EventReader<MouseWheel>,
    mut state: ResMut<TitleState>,
    mut acts: TitleActions,
) {
    if !state.active {
        wheel.clear();
        return;
    }

    // A key-rebind is armed: the next bindable key takes it (Esc cancels).
    if let Some(i) = state.rebind {
        if keys.just_pressed(KeyCode::Escape) {
            state.rebind = None;
            state.rows.clear();
            state.dirty = true;
            return;
        }
        for (name, key) in BINDABLE_KEYS {
            if keys.just_pressed(*key) {
                let action = GameAction::ORDER[i];
                acts.binds.rebind(action, *key);
                state.status = match acts.binds.save() {
                    Ok(()) => Some(format!("{} を {} に割当てた", action.label(), name)),
                    Err(e) => Some(format!("保存失敗: {e}")),
                };
                state.rebind = None;
                state.rows.clear();
                state.dirty = true;
                return;
            }
        }
        return;
    }

    // Credits: scroll-only screen.
    if state.screen == TitleScreen::Credits {
        let lines = state.credits.as_deref().map_or(0, |c| c.lines().count());
        let max = (lines as f32 * 24.0 - 200.0).max(0.0);
        let mut delta = 0.0;
        for ev in wheel.read() {
            delta -= ev.y * 48.0;
        }
        if keys.pressed(KeyCode::ArrowDown) {
            delta += 6.0;
        }
        if keys.pressed(KeyCode::ArrowUp) {
            delta -= 6.0;
        }
        if keys.just_pressed(KeyCode::PageDown) {
            delta += 300.0;
        }
        if keys.just_pressed(KeyCode::PageUp) {
            delta -= 300.0;
        }
        if delta != 0.0 {
            state.scroll = (state.scroll + delta).clamp(0.0, max);
            state.dirty = true;
        }
        if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Enter) {
            state.credits = None;
            state.goto(TitleScreen::Main);
        }
        return;
    }
    wheel.clear();

    if state.rows.is_empty() {
        rebuild_rows(&mut state, &acts);
        state.dirty = true;
    }

    if keys.just_pressed(KeyCode::ArrowUp) {
        move_sel(&mut state, -1);
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        move_sel(&mut state, 1);
    }
    let sel_kind = state.rows.get(state.sel).map(|r| r.kind);
    if keys.just_pressed(KeyCode::ArrowLeft)
        && let Some(kind) = sel_kind
    {
        adjust(&mut state, &mut acts, kind, -1);
    }
    if keys.just_pressed(KeyCode::ArrowRight)
        && let Some(kind) = sel_kind
    {
        adjust(&mut state, &mut acts, kind, 1);
    }
    if keys.just_pressed(KeyCode::Enter)
        && let Some(kind) = sel_kind
        && state.rows[state.sel].enabled
    {
        activate(&mut state, &mut acts, kind);
    }
    if keys.just_pressed(KeyCode::Escape) && state.screen != TitleScreen::Main {
        state.goto(TitleScreen::Main);
    }
}

/// Mouse: hover selects, click activates.
pub fn title_buttons(
    mut state: ResMut<TitleState>,
    mut acts: TitleActions,
    items: Query<(&Interaction, &TitleItem), Changed<Interaction>>,
) {
    if !state.active {
        return;
    }
    for (interaction, item) in &items {
        let Some(row) = state.rows.get(item.0) else { continue };
        if !row.enabled {
            continue;
        }
        let kind = row.kind;
        match interaction {
            Interaction::Hovered => {
                if state.sel != item.0 {
                    state.sel = item.0;
                    state.dirty = true;
                }
            }
            Interaction::Pressed => {
                state.sel = item.0;
                activate(&mut state, &mut acts, kind);
            }
            Interaction::None => {}
        }
    }
}

/// Rebuild the overlay whenever the state is dirty (menus are tiny; a full
/// rebuild per change is simpler than incremental updates and plenty fast).
pub fn sync_title_ui(
    mut commands: Commands,
    mut state: ResMut<TitleState>,
    asset_server: Res<AssetServer>,
    overlay: Query<Entity, With<TitleOverlay>>,
) {
    if !state.active {
        state.dirty = true; // rebuild on the next open
        for e in &overlay {
            commands.entity(e).despawn_recursive();
        }
        return;
    }
    if !state.dirty && !overlay.is_empty() {
        return;
    }
    state.dirty = false;
    for e in &overlay {
        commands.entity(e).despawn_recursive();
    }

    let font = asset_server.load(FONT_REGULAR);
    let font_bold = asset_server.load(FONT_BOLD);
    let dim = Color::srgb(0.45, 0.45, 0.5);
    let lit = Color::srgb(0.92, 0.92, 0.86);

    let mut root = commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(6.0),
            ..default()
        },
        BackgroundColor(Color::srgb(0.03, 0.03, 0.07)),
        FocusPolicy::Block,
        GlobalZIndex(200),
        TitleOverlay,
    ));

    root.with_children(|p| {
        // Error banner (broken project etc.).
        if let Some(err) = &state.error {
            p.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(6.0)),
                    margin: UiRect::bottom(Val::Px(12.0)),
                    max_width: Val::Percent(90.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.45, 0.08, 0.08)),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(format!("読み込みエラー: {err}\n「ゲームを選ぶ」から別のゲームを選んでください")),
                    TextFont { font: font.clone(), font_size: 18.0, ..default() },
                    TextColor(Color::srgb(1.0, 0.85, 0.8)),
                ));
            });
        }

        match state.screen {
            TitleScreen::Main => {
                p.spawn((
                    Text::new(state.game_title.clone()),
                    TextFont { font: font_bold.clone(), font_size: 54.0, ..default() },
                    TextColor(Color::srgb(0.95, 0.9, 0.65)),
                ));
                if !state.game_author.is_empty() {
                    p.spawn((
                        Text::new(format!("作: {}", state.game_author)),
                        TextFont { font: font.clone(), font_size: 20.0, ..default() },
                        TextColor(dim),
                    ));
                }
                if !state.game_desc.is_empty() {
                    p.spawn((
                        Node {
                            max_width: Val::Px(560.0),
                            margin: UiRect::bottom(Val::Px(10.0)),
                            ..default()
                        },
                        Text::new(state.game_desc.clone()),
                        TextFont { font: font.clone(), font_size: 18.0, ..default() },
                        TextLayout::new_with_justify(JustifyText::Center),
                        TextColor(dim),
                    ));
                }
            }
            TitleScreen::Continue => {
                p.spawn((
                    Text::new("つづきから"),
                    TextFont { font: font_bold.clone(), font_size: 34.0, ..default() },
                    TextColor(lit),
                ));
            }
            TitleScreen::Settings => {
                p.spawn((
                    Text::new("設定"),
                    TextFont { font: font_bold.clone(), font_size: 34.0, ..default() },
                    TextColor(lit),
                ));
            }
            TitleScreen::Games => {
                p.spawn((
                    Text::new("ゲームを選ぶ"),
                    TextFont { font: font_bold.clone(), font_size: 34.0, ..default() },
                    TextColor(lit),
                ));
            }
            TitleScreen::Credits => {
                p.spawn((
                    Text::new("クレジット"),
                    TextFont { font: font_bold.clone(), font_size: 34.0, ..default() },
                    TextColor(lit),
                ));
            }
        }

        // Credits body: a clipped window over one tall text block.
        if state.screen == TitleScreen::Credits {
            p.spawn((
                Node {
                    width: Val::Percent(72.0),
                    height: Val::Percent(62.0),
                    overflow: Overflow::clip_y(),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.05, 0.05, 0.1)),
            ))
            .with_children(|p| {
                p.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(-state.scroll),
                        left: Val::Px(12.0),
                        right: Val::Px(12.0),
                        ..default()
                    },
                    Text::new(state.credits.clone().unwrap_or_default()),
                    TextFont { font: font.clone(), font_size: 16.0, ..default() },
                    TextColor(lit),
                ));
            });
        }

        // Menu rows.
        for (i, row) in state.rows.iter().enumerate() {
            let selected = i == state.sel;
            let color = if !row.enabled {
                dim
            } else if selected {
                Color::srgb(1.0, 0.95, 0.6)
            } else {
                lit
            };
            let marker = if selected { "▶ " } else { "　 " };
            p.spawn((
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(if selected {
                    Color::srgb(0.14, 0.14, 0.24)
                } else {
                    Color::NONE
                }),
                TitleItem(i),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(format!("{marker}{}", row.label)),
                    TextFont { font: font.clone(), font_size: 24.0, ..default() },
                    TextColor(color),
                ));
            });
        }

        // Status + key hints.
        if let Some(status) = &state.status {
            p.spawn((
                Text::new(status.clone()),
                TextFont { font: font.clone(), font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.95, 0.6, 0.4)),
            ));
        }
        let hint = match state.screen {
            TitleScreen::Credits => "↑↓/ホイール: スクロール   Enter/Esc: もどる",
            TitleScreen::Settings => "↑↓: 選択   ◀▶: 変更   Enter: 決定/切替   Esc: もどる",
            TitleScreen::Main => "↑↓: 選択   Enter: 決定",
            _ => "↑↓: 選択   Enter: 決定   Esc: もどる",
        };
        p.spawn((
            Node { margin: UiRect::top(Val::Px(16.0)), ..default() },
            Text::new(hint),
            TextFont { font: font.clone(), font_size: 16.0, ..default() },
            TextColor(dim),
        ));
    });
}

// ------------------------------------------------------------------ reset

/// The world state a reset rewrites, bundled for the parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ResetWorld<'w> {
    pub states: ResMut<'w, crate::world::LevelStates>,
    pub flags: ResMut<'w, crate::event::EventFlags>,
    pub queue: ResMut<'w, crate::event::EventQueue>,
    pub writes: ResMut<'w, crate::event::WallWrites>,
    pub clock: ResMut<'w, crate::clock::GameClock>,
    pub rng: ResMut<'w, crate::rng::GameRng>,
    pub bgm: ResMut<'w, crate::audio::BgmState>,
    pub move_mode: ResMut<'w, crate::event::MoveMode>,
    pub skip_snapshot: ResMut<'w, crate::world::SkipNextSnapshot>,
    pub data: ResMut<'w, crate::game_state::DataScreen>,
    pub limits: Res<'w, crate::config::LimitsConfig>,
}

/// Handle [`ResetRunReq`]: restore every global to its authored initial value,
/// then rebuild the world through the normal transition path (same machinery as
/// a load, plan10).
pub fn apply_reset(
    mut reqs: EventReader<ResetRunReq>,
    init: Res<InitialRun>,
    mut w: ResetWorld,
    mut party: ResMut<Party>,
    mut transition: EventWriter<crate::world::LevelTransition>,
) {
    if reqs.read().last().is_none() {
        return;
    }
    party.members = init.party.members.clone();
    *w.flags = crate::event::EventFlags::new(w.limits.event_flags);
    for &f in &init.initial_flags {
        w.flags.set(f, true);
    }
    w.states.map.clear();
    w.queue.pending.clear();
    w.writes.map.clear();
    w.clock.restore(0);
    *w.rng = crate::rng::GameRng::default();
    w.bgm.override_track = None;
    *w.move_mode = default();
    w.data.open = false;
    w.skip_snapshot.0 = true;
    transition.send(crate::world::LevelTransition {
        to_level: 0,
        to: init.start,
        to_facing: init.facing,
    });
}

// ------------------------------------------------------------------ time label

/// `SystemTime` → "YYYY-MM-DD HH:MM" in JST (UTC+9), no chrono dependency.
fn fmt_jst(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + 9 * 3600;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's civil algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_epoch_and_leap() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
        // 2000-02-29 is day 11016.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn volume_cycles_and_clamps() {
        assert!((cycle_volume(0.8, 0.1) - 0.9).abs() < 1e-5);
        // 100% + up wraps to 0 (Enter-cycle), down clamps at 0.
        assert_eq!(cycle_volume(1.0, 0.1), 0.0);
        assert_eq!(cycle_volume(0.0, -0.1), 0.0);
    }

    #[test]
    fn selection_skips_disabled_rows() {
        let mut state = TitleState {
            rows: vec![
                Row { label: "a".into(), enabled: true, kind: RowKind::Quit },
                Row { label: "b".into(), enabled: false, kind: RowKind::Quit },
                Row { label: "c".into(), enabled: true, kind: RowKind::Quit },
            ],
            ..default()
        };
        move_sel(&mut state, 1);
        assert_eq!(state.sel, 2, "disabled row must be skipped");
        move_sel(&mut state, 1);
        assert_eq!(state.sel, 0, "wraps past the end");
    }
}
