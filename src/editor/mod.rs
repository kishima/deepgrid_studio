//! The Studio editor (plan3 map editor → plan9 full content suite).
//!
//! `EditorState` owns the whole editable [`Project`] plus editor UI state (active
//! tab, per-tab list selection, map floor/level, palette). Map block edits keep
//! their fine-grained per-level Undo/Redo (`EditOp`); every other edit
//! (characters/items/monsters/magics/events/settings and structural map changes)
//! is undone by whole-project snapshots. The egui layer (`ui`) only *reads* state
//! and calls these methods, so edits stay centralised and undoable.

mod edit3d;
mod labels;
pub mod ops;
mod shot;
mod ui;
mod walk;

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

use crate::debug_shot;
use crate::dungeon::{Block, Dungeon, Facing, GridPos};
use crate::item::ItemPlacement;
use crate::monster::MonsterPlacement;
use crate::project::{self, LevelData, Project};

/// Cap on the undo history (plan3). Older ops are dropped past this.
const UNDO_LIMIT: usize = 256;

/// The paintable blocks shown in the left palette, in display order.
pub const PALETTE: &[Block] = &[
    Block::Wall,
    Block::Empty,
    Block::Water,
    Block::Fire,
    Block::Poison,
    Block::Ladder,
    Block::Door { kind: 0 },
    Block::Door { kind: 1 },
    Block::Horoscope { pass_from: Facing::West },
    Block::Horoscope { pass_from: Facing::East },
    Block::Horoscope { pass_from: Facing::North },
    Block::Horoscope { pass_from: Facing::South },
    Block::Hole,
    Block::Stairs { up: true },
    Block::Stairs { up: false },
    Block::WritableWall,
    Block::HoroscopeVert { from_below: true },
    Block::HoroscopeVert { from_below: false },
    Block::Keyhole,
    Block::Switch,
    Block::FloorPlate,
    Block::WarpPoint,
];

/// Which editor tab is active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Map,
    Characters,
    Items,
    Monsters,
    Magics,
    Events,
    Demos,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Map,
        Tab::Characters,
        Tab::Items,
        Tab::Monsters,
        Tab::Magics,
        Tab::Events,
        Tab::Demos,
        Tab::Settings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Map => "マップ",
            Tab::Characters => "キャラ",
            Tab::Items => "アイテム",
            Tab::Monsters => "モンスター",
            Tab::Magics => "魔法",
            Tab::Events => "イベント",
            Tab::Demos => "デモ",
            Tab::Settings => "設定",
        }
    }
}

/// What a left/right click paints on the map (plan9 placement layers).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaceLayer {
    Block,
    Item,
    Monster,
    Trigger,
}

/// The player's start placement (position + facing), the unit of a start edit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StartPlacement {
    pub pos: GridPos,
    pub facing: Facing,
}

/// One undoable map edit: cell changes, a start move, and/or the current
/// level's placement lists (items / monsters / events) as before→after. The
/// list changes make item/monster/trigger placement undoable from the map tab
/// itself (Map-tab undo never reaches the whole-project content snapshots).
#[derive(Default)]
pub struct EditOp {
    cells: Vec<(GridPos, Block, Block)>,
    start_change: Option<(StartPlacement, StartPlacement)>,
    items_change: Option<(Vec<ItemPlacement>, Vec<ItemPlacement>)>,
    monsters_change: Option<(Vec<MonsterPlacement>, Vec<MonsterPlacement>)>,
    events_change: Option<(Vec<crate::event::EventDef>, Vec<crate::event::EventDef>)>,
}

struct Stroke {
    floor: usize,
    before: HashMap<(i32, i32), Block>,
}

/// The editor's authoritative state (Bevy resource).
#[derive(Resource)]
pub struct EditorState {
    /// The whole editable project. Edits go through methods here.
    pub proj: Project,
    pub tab: Tab,
    // Map view state.
    pub level_index: usize,
    pub floor_index: usize,
    pub selected: Block,
    pub place_layer: PlaceLayer,
    pub place_item: String,
    pub place_monster: String,
    pub place_trigger: Block,
    // Per-tab list selections.
    pub sel_char: usize,
    pub sel_item: usize,
    pub sel_monster: usize,
    pub sel_magic: usize,
    pub sel_event: usize,
    pub sel_demo: usize,
    /// Lazily-scanned `assets/audio/bgm/` file names (for the BGM combos).
    pub bgm_files: Option<Vec<String>>,
    /// Lazily-scanned `<project>/override/` relative paths (settings tab list).
    pub override_files: Option<Vec<String>>,
    pub status: String,
    pub cursor: Option<(i32, i32)>,
    pub warnings: Vec<String>,
    /// Shared scratch buffer for the id-rename box.
    pub rename_buf: String,
    /// Whether the Japanese egui font has been installed on this context yet.
    pub fonts_installed: bool,
    /// Whether the map tab shows the 3D walk view (plan9.5).
    pub mode_3d: bool,
    /// Cells whose terrain the 3D view must rebuild (drained by `edit3d`).
    pub d3_terrain_dirty: Vec<GridPos>,
    /// The 3D view must respawn its placement markers.
    pub d3_markers_dirty: bool,
    /// The 3D view must fully rebuild (level switch / undo / redo).
    pub d3_full: bool,
    /// The 3D walker's coordinate/facing readout (shown in the top bar).
    pub d3_coord: String,
    /// Whether anything is unsaved (Save All writes everything).
    dirty: bool,
    // Map Undo/Redo (per level; cleared on level switch).
    undo: Vec<EditOp>,
    redo: Vec<EditOp>,
    stroke: Option<Stroke>,
    // Whole-project snapshot Undo/Redo for content + structural map edits.
    content_undo: Vec<Project>,
    content_redo: Vec<Project>,
}

impl EditorState {
    pub fn new(project: Project) -> Self {
        let mut s = Self {
            proj: project,
            tab: Tab::Map,
            level_index: 0,
            floor_index: 0,
            selected: Block::Wall,
            place_layer: PlaceLayer::Block,
            place_item: String::new(),
            place_monster: String::new(),
            place_trigger: Block::Switch,
            sel_char: 0,
            sel_item: 0,
            sel_monster: 0,
            sel_magic: 0,
            sel_event: 0,
            sel_demo: 0,
            bgm_files: None,
            override_files: None,
            status: "Ready".to_string(),
            cursor: None,
            warnings: Vec::new(),
            rename_buf: String::new(),
            fonts_installed: false,
            mode_3d: false,
            d3_terrain_dirty: Vec::new(),
            d3_markers_dirty: false,
            d3_full: false,
            d3_coord: String::new(),
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            content_undo: Vec::new(),
            content_redo: Vec::new(),
        };
        s.place_item = s.proj.items.first().map(|d| d.id.clone()).unwrap_or_default();
        s.place_monster = s.proj.monsters.first().map(|d| d.id.clone()).unwrap_or_default();
        s.recompute_warnings();
        s
    }

    pub fn cur(&self) -> &LevelData {
        &self.proj.levels[self.level_index]
    }

    fn cur_mut(&mut self) -> &mut LevelData {
        &mut self.proj.levels[self.level_index]
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn can_undo(&self) -> bool {
        if self.tab == Tab::Map { !self.undo.is_empty() } else { !self.content_undo.is_empty() }
    }
    pub fn can_redo(&self) -> bool {
        if self.tab == Tab::Map { !self.redo.is_empty() } else { !self.content_redo.is_empty() }
    }

    pub fn block_at(&self, x: i32, y: i32) -> Option<Block> {
        self.cur().level.block_at(GridPos::new(x, y, self.floor_index))
    }

    pub fn has_footing(&self, x: i32, y: i32) -> bool {
        if self.floor_index == 0 {
            return true;
        }
        self.cur()
            .level
            .floor(self.floor_index - 1)
            .and_then(|f| f.get(x, y))
            .is_some_and(|b| b.is_solid())
    }

    pub fn wall_below(&self, x: i32, y: i32) -> bool {
        self.floor_index > 0 && self.has_footing(x, y)
    }

    // --- snapshot undo (content + structural) ---------------------------------

    /// Snapshot the whole project for undo (call *before* a mutation). Sets dirty.
    pub fn snapshot(&mut self) {
        self.content_undo.push(self.proj.clone());
        if self.content_undo.len() > UNDO_LIMIT {
            self.content_undo.remove(0);
        }
        self.content_redo.clear();
        self.mark_dirty();
    }

    fn content_undo(&mut self) {
        if let Some(prev) = self.content_undo.pop() {
            self.content_redo.push(std::mem::replace(&mut self.proj, prev));
            self.clamp_selections();
            self.recompute_warnings();
            self.dirty = true;
            self.status = "undo".into();
        } else {
            self.status = "nothing to undo".into();
        }
    }
    fn content_redo(&mut self) {
        if let Some(next) = self.content_redo.pop() {
            self.content_undo.push(std::mem::replace(&mut self.proj, next));
            self.clamp_selections();
            self.recompute_warnings();
            self.dirty = true;
            self.status = "redo".into();
        } else {
            self.status = "nothing to redo".into();
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Mark a field edit (dirty only; field edits aren't individually undoable —
    /// structural ops snapshot instead). Called by the UI on `response.changed()`.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    fn clamp_selections(&mut self) {
        self.level_index = self.level_index.min(self.proj.levels.len().saturating_sub(1));
        self.floor_index = self.floor_index.min(self.cur().floor_count().saturating_sub(1));
        self.sel_char = self.sel_char.min(self.proj.characters.len().saturating_sub(1));
        self.sel_item = self.sel_item.min(self.proj.items.len().saturating_sub(1));
        self.sel_monster = self.sel_monster.min(self.proj.monsters.len().saturating_sub(1));
        self.sel_magic = self.sel_magic.min(self.proj.magics.len().saturating_sub(1));
        self.sel_demo = self.sel_demo.min(self.proj.demos.len().saturating_sub(1));
    }

    /// Recompute the validation warning list (on save / tab switch / structural edit).
    pub fn recompute_warnings(&mut self) {
        self.warnings = ops::validate(&self.proj);
    }

    // --- map editing ----------------------------------------------------------

    pub fn paint(&mut self, x: i32, y: i32) {
        let floor = self.floor_index;
        let (w, h) = (self.cur().width() as i32, self.cur().height() as i32);
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let selected = self.selected;
        let start = self.cur().start;
        if selected.is_wall() && start.floor == floor && start.x == x && start.y == y {
            self.status = "can't wall over the start cell".to_string();
            return;
        }
        if self.stroke.as_ref().map(|s| s.floor) != Some(floor) {
            self.stroke = Some(Stroke { floor, before: HashMap::new() });
        }
        let idx = y as usize * w as usize + x as usize;
        let before = self.cur().level.floors[floor].blocks[idx];
        if before == selected {
            return;
        }
        self.stroke.as_mut().unwrap().before.entry((x, y)).or_insert(before);
        self.cur_mut().level.floors[floor].blocks[idx] = selected;
        self.d3_terrain_dirty.push(GridPos::new(x, y, floor));
    }

    pub fn end_stroke(&mut self) {
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        let floor = stroke.floor;
        let w = self.cur().width();
        let mut cells = Vec::new();
        for ((x, y), before) in stroke.before {
            let after = self.cur().level.floors[floor].blocks[y as usize * w + x as usize];
            if after != before {
                cells.push((GridPos::new(x, y, floor), before, after));
            }
        }
        if !cells.is_empty() {
            self.push_op(EditOp { cells, ..Default::default() });
        }
    }

    pub fn set_start(&mut self, x: i32, y: i32) {
        let floor = self.floor_index;
        match self.block_at(x, y) {
            Some(b) if b.is_solid() => {
                self.status = "start can't be on a wall".to_string();
                return;
            }
            None => return,
            _ => {}
        }
        if !self.has_footing(x, y) && !self.block_at(x, y).is_some_and(|b| b.self_supports()) {
            self.status = "start needs footing (wall below or floor 0)".to_string();
            return;
        }
        let before = StartPlacement { pos: self.cur().start, facing: self.cur().start_facing };
        let after = if before.pos == GridPos::new(x, y, floor) {
            StartPlacement { pos: before.pos, facing: before.facing.turn_right() }
        } else {
            StartPlacement { pos: GridPos::new(x, y, floor), facing: before.facing }
        };
        self.apply_start(after);
        self.push_op(EditOp { start_change: Some((before, after)), ..Default::default() });
    }

    fn apply_start(&mut self, sp: StartPlacement) {
        let lvl = self.cur_mut();
        lvl.start = sp.pos;
        lvl.start_facing = sp.facing;
    }

    fn push_op(&mut self, op: EditOp) {
        self.undo.push(op);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.mark_dirty();
    }

    fn apply_op(&mut self, op: &EditOp, forward: bool) {
        let w = self.cur().width();
        for (pos, before, after) in &op.cells {
            let block = if forward { *after } else { *before };
            let floor = &mut self.proj.levels[self.level_index].level.floors[pos.floor];
            floor.blocks[pos.y as usize * w + pos.x as usize] = block;
        }
        if let Some((before, after)) = &op.start_change {
            self.apply_start(if forward { *after } else { *before });
        }
        if let Some((before, after)) = &op.items_change {
            self.cur_mut().items = if forward { after.clone() } else { before.clone() };
            self.d3_markers_dirty = true;
        }
        if let Some((before, after)) = &op.monsters_change {
            self.cur_mut().monsters = if forward { after.clone() } else { before.clone() };
            self.d3_markers_dirty = true;
        }
        if let Some((before, after)) = &op.events_change {
            self.cur_mut().events = if forward { after.clone() } else { before.clone() };
            self.d3_markers_dirty = true;
            self.recompute_warnings();
        }
    }

    /// Undo the active tab's stack.
    pub fn undo(&mut self) {
        self.d3_full = true;
        if self.tab != Tab::Map {
            self.content_undo();
            return;
        }
        let Some(op) = self.undo.pop() else {
            self.status = "nothing to undo".to_string();
            return;
        };
        self.apply_op(&op, false);
        self.redo.push(op);
        self.mark_dirty();
        self.status = "undo".to_string();
    }

    pub fn redo(&mut self) {
        self.d3_full = true;
        if self.tab != Tab::Map {
            self.content_redo();
            return;
        }
        let Some(op) = self.redo.pop() else {
            self.status = "nothing to redo".to_string();
            return;
        };
        self.apply_op(&op, true);
        self.undo.push(op);
        self.mark_dirty();
        self.status = "redo".to_string();
    }

    pub fn select_level(&mut self, index: usize) {
        if index >= self.proj.levels.len() || index == self.level_index {
            return;
        }
        self.level_index = index;
        self.floor_index = self.floor_index.min(self.cur().floor_count() - 1);
        self.undo.clear();
        self.redo.clear();
        self.stroke = None;
        self.d3_full = true;
    }

    pub fn select_floor(&mut self, floor: usize) {
        if floor < self.cur().floor_count() {
            self.floor_index = floor;
        }
    }

    // --- map placement (items / monsters / triggers) --------------------------

    /// Place the selected content at `(x, y)` on the current floor per `place_layer`.
    pub fn place_at(&mut self, x: i32, y: i32) {
        let floor = self.floor_index;
        match self.place_layer {
            PlaceLayer::Block => self.paint(x, y),
            PlaceLayer::Item => {
                if self.place_item.is_empty() {
                    return;
                }
                let id = self.place_item.clone();
                let before = self.cur().items.clone();
                let lvl = self.cur_mut();
                lvl.items.retain(|p| !(p.x == x && p.y == y && p.floor == floor));
                lvl.items.push(ItemPlacement { id, x, y, floor });
                let after = self.cur().items.clone();
                if after != before {
                    self.push_op(EditOp { items_change: Some((before, after)), ..Default::default() });
                }
                self.d3_markers_dirty = true;
            }
            PlaceLayer::Monster => {
                if self.place_monster.is_empty() {
                    return;
                }
                let id = self.place_monster.clone();
                let before = self.cur().monsters.clone();
                let lvl = self.cur_mut();
                lvl.monsters.retain(|p| !(p.x == x && p.y == y && p.floor == floor));
                lvl.monsters.push(MonsterPlacement { id, x, y, floor, facing: Facing::North });
                let after = self.cur().monsters.clone();
                if after != before {
                    self.push_op(EditOp { monsters_change: Some((before, after)), ..Default::default() });
                }
                self.d3_markers_dirty = true;
            }
            PlaceLayer::Trigger => {
                let block = self.place_trigger;
                let w = self.cur().width();
                let before_block = self.cur().level.floors[floor].blocks[y as usize * w + x as usize];
                let before_events = self.cur().events.clone();
                self.cur_mut().level.floors[floor].blocks[y as usize * w + x as usize] = block;
                self.d3_terrain_dirty.push(GridPos::new(x, y, floor));
                self.d3_markers_dirty = true;
                // Auto-create an empty event template at this coordinate.
                let already = self.cur().events.iter().any(|e| e.at == (x, y, floor));
                if !already {
                    let id = ops::next_id(
                        &self.cur().events.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
                        "event",
                    );
                    let trigger = trigger_for(block);
                    self.cur_mut().events.push(crate::event::EventDef {
                        id,
                        trigger,
                        at: (x, y, floor),
                        delay_cycles: 0,
                        flags: Vec::new(),
                        join: crate::event::FlagJoin::And,
                        actions: Vec::new(),
                    });
                }
                // One op = block + template, so a single undo removes both.
                let cells = if before_block == block {
                    Vec::new()
                } else {
                    vec![(GridPos::new(x, y, floor), before_block, block)]
                };
                let events_change = (self.cur().events != before_events)
                    .then(|| (before_events, self.cur().events.clone()));
                if !cells.is_empty() || events_change.is_some() {
                    self.push_op(EditOp { cells, events_change, ..Default::default() });
                }
                self.recompute_warnings();
            }
        }
    }

    /// Right-click erase at `(x, y)` per layer (block → Empty, else remove
    /// placements / the trigger block).
    pub fn erase_at(&mut self, x: i32, y: i32) {
        let floor = self.floor_index;
        match self.place_layer {
            PlaceLayer::Block | PlaceLayer::Trigger => {
                let w = self.cur().width();
                let before = self.cur().level.floors[floor].blocks[y as usize * w + x as usize];
                if before == Block::Empty {
                    return;
                }
                self.cur_mut().level.floors[floor].blocks[y as usize * w + x as usize] = Block::Empty;
                self.push_op(EditOp {
                    cells: vec![(GridPos::new(x, y, floor), before, Block::Empty)],
                    ..Default::default()
                });
                self.d3_terrain_dirty.push(GridPos::new(x, y, floor));
                self.d3_markers_dirty = true;
            }
            PlaceLayer::Item => {
                let before = self.cur().items.clone();
                self.cur_mut().items.retain(|p| !(p.x == x && p.y == y && p.floor == floor));
                if self.cur().items.len() != before.len() {
                    let after = self.cur().items.clone();
                    self.push_op(EditOp { items_change: Some((before, after)), ..Default::default() });
                }
                self.d3_markers_dirty = true;
            }
            PlaceLayer::Monster => {
                let before = self.cur().monsters.clone();
                self.cur_mut().monsters.retain(|p| !(p.x == x && p.y == y && p.floor == floor));
                if self.cur().monsters.len() != before.len() {
                    let after = self.cur().monsters.clone();
                    self.push_op(EditOp { monsters_change: Some((before, after)), ..Default::default() });
                }
                self.d3_markers_dirty = true;
            }
        }
    }

    // --- level management -----------------------------------------------------

    /// Add a fresh all-wall level (same size as the current one).
    pub fn add_level(&mut self) {
        self.snapshot();
        let (w, h, f) = {
            let l = self.cur();
            (l.width(), l.height(), l.floor_count().max(1))
        };
        use crate::dungeon::level::{Floor, Level};
        let floors = (0..f)
            .map(|_| Floor { width: w, height: h, blocks: vec![Block::Wall; w * h] })
            .collect();
        // Carve a single standable start cell on floor 0.
        let mut level = Level { floors };
        level.set_block(GridPos::new(1, 1, 0), Block::Empty);
        let data = LevelData {
            start: GridPos::new(1, 1, 0),
            start_facing: Facing::North,
            level,
            items: Vec::new(),
            monsters: Vec::new(),
            wall_texts: Vec::new(),
            stairs_links: Vec::new(),
            events: Vec::new(),
            open_doors: Vec::new(),
            bgm: String::new(),
        };
        let n = self.proj.levels.len();
        self.proj.level_paths.push(format!("levels/level{n:02}.ron"));
        self.proj.levels.push(data);
        self.status = format!("added level {n}");
        self.d3_full = true;
        self.recompute_warnings();
    }

    /// Delete level `index` (keeps at least one), warning about references.
    pub fn delete_level(&mut self, index: usize) {
        if self.proj.levels.len() <= 1 || index >= self.proj.levels.len() {
            self.status = "cannot delete the last level".into();
            return;
        }
        let warns = ops::level_delete_warnings(&self.proj, index);
        self.snapshot();
        self.proj.levels.remove(index);
        self.proj.level_paths.remove(index);
        self.level_index = self.level_index.min(self.proj.levels.len() - 1);
        self.floor_index = self.floor_index.min(self.cur().floor_count() - 1);
        self.status = if warns.is_empty() {
            format!("deleted level {index}")
        } else {
            format!("deleted level {index} — 注意: {}", warns.join("; "))
        };
        self.d3_full = true;
        self.recompute_warnings();
    }

    // --- save -----------------------------------------------------------------

    /// Save All: write the whole project (project.ron + data files + levels).
    pub fn save(&mut self) {
        self.recompute_warnings();
        match project::save_project(&self.proj) {
            Ok(()) => {
                self.dirty = false;
                self.status = if self.warnings.is_empty() {
                    "Save All 完了".into()
                } else {
                    format!("Save All 完了 (警告 {}件)", self.warnings.len())
                };
            }
            Err(errs) => self.status = format!("save failed: {}", errs.join("; ")),
        }
    }
}

/// The default trigger kind for a freshly-placed trigger block.
fn trigger_for(block: Block) -> crate::event::TriggerKind {
    use crate::event::{PlateCond, TriggerKind};
    match block {
        Block::Keyhole => TriggerKind::Keyhole { key_item: String::new() },
        Block::FloorPlate => TriggerKind::FloorPlate { cond: PlateCond::Step },
        Block::WarpPoint => TriggerKind::WarpPoint { hidden: false },
        _ => TriggerKind::SwitchPush,
    }
}

/// Build and run the editor app (edit mode).
pub fn run(project: Project) {
    let state = EditorState::new(project);

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "DeepGrid Studio — Editor".to_string(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin)
    .insert_resource(ClearColor(Color::srgb(0.10, 0.10, 0.12)))
    .insert_resource(state)
    .add_systems(Startup, setup_editor_camera);

    if let Some(tab) = debug_shot::editor_shot_tab() {
        // Verification: render a given egui tab into an image and capture it.
        shot::setup(&mut app, tab);
    } else {
        // Interactive editor (and the editor-3d Bevy-screenshot scene): the 2D
        // egui editor plus the 3D walk mode (plan9.5).
        edit3d::register(&mut app);
        app.add_systems(Startup, edit3d_setup_resources);
        app.add_systems(Update, ui::editor_ui_window);
        if debug_shot::wants_editor_3d() {
            app.add_systems(Update, edit3d::edit3d_shot_driver);
        }
    }

    app.run();
}

fn setup_editor_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Build the 3D-edit palette + the `Dungeon` mirror (from level 0) at startup.
fn edit3d_setup_resources(
    mut commands: Commands,
    state: Res<EditorState>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let resolver = crate::project::AssetResolver { project_dir: state.proj.dir.clone() };
    let palette = crate::render::build_palette(&asset_server, &resolver, &mut meshes, &mut materials);
    commands.insert_resource(resolver);
    commands.insert_resource(palette);
    let lvl = &state.proj.levels[0];
    commands.insert_resource(Dungeon {
        level: lvl.level.clone(),
        start_pos: lvl.start,
        start_facing: lvl.start_facing,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LimitsConfig;
    use crate::dungeon::level::{Floor, Level};

    fn two_floor_project() -> Project {
        let floor0 = Floor { width: 3, height: 3, blocks: vec![Block::Wall; 9] };
        let floor1 = Floor { width: 3, height: 3, blocks: vec![Block::Empty; 9] };
        let level = LevelData {
            start: GridPos::new(0, 0, 1),
            start_facing: Facing::North,
            level: Level { floors: vec![floor0, floor1] },
            items: Vec::new(),
            monsters: Vec::new(),
            wall_texts: Vec::new(),
            stairs_links: Vec::new(),
            events: Vec::new(),
            open_doors: Vec::new(),
            bgm: String::new(),
        };
        Project {
            dir: std::path::PathBuf::from("/tmp/does-not-exist"),
            name: "T".to_string(),
            author: String::new(),
            description: String::new(),
            limits: LimitsConfig::default(),
            level_paths: vec!["levels/level00.ron".to_string()],
            levels: vec![level],
            characters: Vec::new(),
            party: Vec::new(),
            items: Vec::new(),
            monsters: Vec::new(),
            magics: Vec::new(),
            rules: crate::rules::RulesConfig::default(),
            initial_flags: Vec::new(),
            demos: Vec::new(),
            characters_path: String::new(),
            items_path: String::new(),
            monsters_path: String::new(),
            magics_path: String::new(),
            demos_path: String::new(),
        }
    }

    #[test]
    fn paint_stroke_is_one_undo_op() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.selected = Block::Water;
        s.paint(1, 1);
        s.paint(2, 1);
        s.paint(1, 1);
        s.end_stroke();
        assert_eq!(s.block_at(1, 1), Some(Block::Water));
        assert_eq!(s.block_at(2, 1), Some(Block::Water));
        assert_eq!(s.undo.len(), 1);
        assert_eq!(s.undo[0].cells.len(), 2);
        s.undo();
        assert_eq!(s.block_at(1, 1), Some(Block::Empty));
        s.redo();
        assert_eq!(s.block_at(1, 1), Some(Block::Water));
    }

    #[test]
    fn cannot_wall_over_start() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.selected = Block::Wall;
        s.paint(0, 0);
        s.end_stroke();
        assert_eq!(s.block_at(0, 0), Some(Block::Empty));
        assert!(s.undo.is_empty());
    }

    #[test]
    fn set_start_moves_then_cycles_facing() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.set_start(2, 2);
        assert_eq!(s.cur().start, GridPos::new(2, 2, 1));
        s.set_start(2, 2);
        assert_eq!(s.cur().start_facing, Facing::East);
        s.undo();
        assert_eq!(s.cur().start_facing, Facing::North);
        s.undo();
        assert_eq!(s.cur().start, GridPos::new(0, 0, 1));
    }

    #[test]
    fn trigger_place_is_one_undo_op() {
        // Keyhole placement = block + auto event template; one Map-tab undo
        // must remove both (they ride the same EditOp, not a content snapshot).
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.place_layer = PlaceLayer::Trigger;
        s.place_trigger = Block::Keyhole;
        s.place_at(1, 1);
        assert_eq!(s.block_at(1, 1), Some(Block::Keyhole));
        assert_eq!(s.cur().events.len(), 1);
        s.undo();
        assert_eq!(s.block_at(1, 1), Some(Block::Empty));
        assert!(s.cur().events.is_empty());
        s.redo();
        assert_eq!(s.block_at(1, 1), Some(Block::Keyhole));
        assert_eq!(s.cur().events.len(), 1);
    }

    #[test]
    fn item_place_and_erase_undo_from_map_tab() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.place_layer = PlaceLayer::Item;
        s.place_item = "potion".to_string();
        s.place_at(1, 1);
        assert_eq!(s.cur().items.len(), 1);
        s.erase_at(1, 1);
        assert!(s.cur().items.is_empty());
        s.undo(); // undo the erase
        assert_eq!(s.cur().items.len(), 1);
        s.undo(); // undo the placement
        assert!(s.cur().items.is_empty());
    }

    #[test]
    fn walk_front_place_matches_2d_edit() {
        // plan9.5: the 3D place path (EditWalk::front → place_at/end_stroke,
        // exactly what edit3d_place calls) must produce the same LevelData as
        // clicking the same cell in the 2D grid, and share its undo stack.
        let mut s3 = EditorState::new(two_floor_project());
        s3.floor_index = 1;
        s3.selected = Block::Wall;
        let w = walk::EditWalk::new(GridPos::new(1, 1, 1), Facing::East);
        let front = w.front();
        assert_eq!(front, GridPos::new(2, 1, 1));
        s3.place_at(front.x, front.y);
        s3.end_stroke();
        assert!(s3.d3_terrain_dirty.contains(&front));

        let mut s2 = EditorState::new(two_floor_project());
        s2.floor_index = 1;
        s2.selected = Block::Wall;
        s2.paint(2, 1);
        s2.end_stroke();
        assert_eq!(s3.cur().level, s2.cur().level);

        // Erase parity, then undo through the common stacks: both routes must
        // keep producing identical levels.
        s3.erase_at(front.x, front.y);
        s2.erase_at(2, 1);
        assert_eq!(s3.cur().level, s2.cur().level);
        assert_eq!(s3.block_at(2, 1), Some(Block::Empty));
        s3.undo();
        s2.undo();
        assert_eq!(s3.cur().level, s2.cur().level);
        assert!(s3.d3_full, "undo must trigger a full 3D rebuild");
    }

    #[test]
    fn add_and_delete_level_snapshots_undo() {
        let mut s = EditorState::new(two_floor_project());
        s.tab = Tab::Map;
        s.add_level();
        assert_eq!(s.proj.levels.len(), 2);
        // add_level snapshots → content undo restores.
        s.tab = Tab::Settings; // route undo to the content stack
        s.undo();
        assert_eq!(s.proj.levels.len(), 1);
    }
}
