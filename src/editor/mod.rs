//! The map editor (plan3): a 2D top-down block editor with Undo/Redo.
//!
//! The core (`EditorState` and the edit operations) is deliberately independent
//! of egui so it can be unit-tested without a GUI; `ui` holds the egui layer.

mod shot;
mod ui;

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

use crate::debug_shot;
use crate::dungeon::{Block, Facing, GridPos};
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
    // plan8 terrain / gimmicks. Event parameters (wall texts, stairs links,
    // event defs) stay hand-authored in RON until the plan9 editor UI.
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

/// The player's start placement (position + facing), the unit of a start edit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StartPlacement {
    pub pos: GridPos,
    pub facing: Facing,
}

/// One undoable edit: a set of cell changes and/or a start-placement change.
/// A drag stroke collapses into a single op (one entry per distinct cell).
pub struct EditOp {
    cells: Vec<(GridPos, Block, Block)>,
    start_change: Option<(StartPlacement, StartPlacement)>,
}

/// In-progress drag stroke: remembers each touched cell's *before* block so the
/// whole stroke becomes one op on release.
struct Stroke {
    floor: usize,
    before: HashMap<(i32, i32), Block>,
}

/// The editor's authoritative state (Bevy resource). UI code mutates the level
/// only through these methods so all edits are undoable and validated.
#[derive(Resource)]
pub struct EditorState {
    pub project_dir: PathBuf,
    pub project_name: String,
    pub level_paths: Vec<String>,
    pub levels: Vec<LevelData>,
    pub level_index: usize,
    pub floor_index: usize,
    pub selected: Block,
    pub status: String,
    /// Cell under the cursor last frame (UI-only; drives the status bar).
    pub cursor: Option<(i32, i32)>,
    /// Per-level unsaved flag.
    dirty: Vec<bool>,
    undo: Vec<EditOp>,
    redo: Vec<EditOp>,
    stroke: Option<Stroke>,
}

impl EditorState {
    pub fn new(project: Project) -> Self {
        let dirty = vec![false; project.levels.len()];
        Self {
            project_dir: project.dir,
            project_name: project.name,
            level_paths: project.level_paths,
            levels: project.levels,
            level_index: 0,
            floor_index: 0,
            selected: Block::Wall,
            status: "Ready".to_string(),
            cursor: None,
            dirty,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
        }
    }

    pub fn cur(&self) -> &LevelData {
        &self.levels[self.level_index]
    }

    fn cur_mut(&mut self) -> &mut LevelData {
        &mut self.levels[self.level_index]
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty[self.level_index]
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Block at `(x, y)` on the current floor, or `None` if out of bounds.
    pub fn block_at(&self, x: i32, y: i32) -> Option<Block> {
        self.cur().level.block_at(GridPos::new(x, y, self.floor_index))
    }

    /// Whether cell `(x, y)` on the current floor has footing from below (bedrock
    /// on floor 0, or a wall directly beneath) — used to shade holes vs floor and
    /// to draw the "floor below" underlay.
    pub fn has_footing(&self, x: i32, y: i32) -> bool {
        if self.floor_index == 0 {
            return true;
        }
        self.cur()
            .level
            .floor(self.floor_index - 1)
            .and_then(|f| f.get(x, y))
            == Some(Block::Wall)
    }

    /// Whether the cell directly below `(x, y)` (on `floor_index - 1`) is a wall —
    /// drives the faint underlay showing support relationships.
    pub fn wall_below(&self, x: i32, y: i32) -> bool {
        self.floor_index > 0 && self.has_footing(x, y)
    }

    // --- editing --------------------------------------------------------------

    /// Paint the selected block at `(x, y)` on the current floor, extending the
    /// current stroke (started lazily). Painting a wall onto the start cell is
    /// rejected.
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
            // Switching floor mid-drag shouldn't happen, but re-anchor cleanly.
            self.stroke = Some(Stroke {
                floor,
                before: HashMap::new(),
            });
        }
        let idx = y as usize * w as usize + x as usize;
        let before = self.cur().level.floors[floor].blocks[idx];
        if before == selected {
            return;
        }
        self.stroke
            .as_mut()
            .unwrap()
            .before
            .entry((x, y))
            .or_insert(before);
        self.cur_mut().level.floors[floor].blocks[idx] = selected;
    }

    /// Finalize the current stroke into one undo op (no-op if nothing changed).
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
            self.push_op(EditOp {
                cells,
                start_change: None,
            });
        }
    }

    /// Right-click a cell: move the start there, or (if it's already the start)
    /// cycle the facing N→E→S→W. Rejected if the target is a wall or has no
    /// footing (so saved levels always load).
    pub fn set_start(&mut self, x: i32, y: i32) {
        let floor = self.floor_index;
        match self.block_at(x, y) {
            Some(b) if b.is_wall() => {
                self.status = "start can't be on a wall".to_string();
                return;
            }
            None => return,
            _ => {}
        }
        if !self.has_footing(x, y) && !matches!(self.block_at(x, y), Some(Block::Ladder)) {
            self.status = "start needs footing (wall below or floor 0)".to_string();
            return;
        }

        let before = StartPlacement {
            pos: self.cur().start,
            facing: self.cur().start_facing,
        };
        let after = if before.pos == GridPos::new(x, y, floor) {
            StartPlacement {
                pos: before.pos,
                facing: before.facing.turn_right(),
            }
        } else {
            StartPlacement {
                pos: GridPos::new(x, y, floor),
                facing: before.facing,
            }
        };
        self.apply_start(after);
        self.push_op(EditOp {
            cells: Vec::new(),
            start_change: Some((before, after)),
        });
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
        self.dirty[self.level_index] = true;
    }

    fn apply_op(&mut self, op: &EditOp, forward: bool) {
        let w = self.cur().width();
        for (pos, before, after) in &op.cells {
            let block = if forward { *after } else { *before };
            let floor = &mut self.levels[self.level_index].level.floors[pos.floor];
            floor.blocks[pos.y as usize * w + pos.x as usize] = block;
        }
        if let Some((before, after)) = &op.start_change {
            self.apply_start(if forward { *after } else { *before });
        }
    }

    pub fn undo(&mut self) {
        let Some(op) = self.undo.pop() else {
            self.status = "nothing to undo".to_string();
            return;
        };
        self.apply_op(&op, false);
        self.redo.push(op);
        self.dirty[self.level_index] = true;
        self.status = "undo".to_string();
    }

    pub fn redo(&mut self) {
        let Some(op) = self.redo.pop() else {
            self.status = "nothing to redo".to_string();
            return;
        };
        self.apply_op(&op, true);
        self.undo.push(op);
        self.dirty[self.level_index] = true;
        self.status = "redo".to_string();
    }

    /// Switch the edited level. Undo history is per-level, so it's cleared;
    /// unsaved edits stay in memory.
    pub fn select_level(&mut self, index: usize) {
        if index >= self.levels.len() || index == self.level_index {
            return;
        }
        self.level_index = index;
        self.floor_index = self.floor_index.min(self.cur().floor_count() - 1);
        self.undo.clear();
        self.redo.clear();
        self.stroke = None;
    }

    pub fn select_floor(&mut self, floor: usize) {
        if floor < self.cur().floor_count() {
            self.floor_index = floor;
        }
    }

    /// Save the current level, first checking the start is valid so the written
    /// file always reloads. Errors go to the status bar (no panic).
    pub fn save(&mut self) {
        let start = self.cur().start;
        match self.cur().level.block_at(start) {
            Some(b) if b.is_wall() => {
                self.status = "cannot save: start is inside a wall".to_string();
                return;
            }
            None => {
                self.status = "cannot save: start is out of bounds".to_string();
                return;
            }
            _ => {}
        }
        if !self
            .cur()
            .level
            .is_supported(start.x, start.y, start.floor)
        {
            self.status = "cannot save: start has no footing".to_string();
            return;
        }

        // plan8: warn (don't block) when trigger blocks and their event
        // definitions don't line up — both directions.
        let warn = self.trigger_event_mismatch();

        let rel = self.level_paths[self.level_index].clone();
        match project::save_level(&self.project_dir, &rel, self.cur()) {
            Ok(()) => {
                self.dirty[self.level_index] = false;
                self.status = match warn {
                    Some(w) => format!("saved {rel} — 注意: {w}"),
                    None => format!("saved {rel}"),
                };
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    /// A human-readable warning if a trigger block lacks a matching `EventDef`
    /// (or an event points at a non-trigger cell). `None` when consistent.
    fn trigger_event_mismatch(&self) -> Option<String> {
        use crate::dungeon::Block;
        let lvl = self.cur();
        let is_trigger = |b: Block| matches!(b, Block::Keyhole | Block::Switch | Block::FloorPlate | Block::WarpPoint);
        // Trigger blocks without an event at their coordinate.
        for f in 0..lvl.level.floor_count() {
            let Some(floor) = lvl.level.floor(f) else { continue };
            for y in 0..floor.height {
                for x in 0..floor.width {
                    let (xi, yi) = (x as i32, y as i32);
                    if floor.get(xi, yi).is_some_and(is_trigger)
                        && !lvl.events.iter().any(|e| e.at == (xi, yi, f))
                    {
                        return Some(format!("トリガーブロック ({xi},{yi},f{f}) にイベント未設定"));
                    }
                }
            }
        }
        None
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

    if debug_shot::wants_editor() {
        // Verification: render the editor into an image and capture that (egui
        // on the window isn't captured by Bevy screenshots).
        shot::setup(&mut app);
    } else {
        app.add_systems(Update, ui::editor_ui_window);
    }

    app.run();
}

/// A 2D camera so the window has a render target (egui draws over it).
fn setup_editor_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LimitsConfig;
    use crate::dungeon::level::{Floor, Level};

    fn two_floor_project() -> Project {
        let floor0 = Floor {
            width: 3,
            height: 3,
            blocks: vec![Block::Wall; 9],
        };
        let floor1 = Floor {
            width: 3,
            height: 3,
            blocks: vec![Block::Empty; 9],
        };
        let level = LevelData {
            start: GridPos::new(0, 0, 1),
            start_facing: Facing::North,
            level: Level {
                floors: vec![floor0, floor1],
            },
            items: Vec::new(),
            monsters: Vec::new(),
            wall_texts: Vec::new(),
            stairs_links: Vec::new(),
            events: Vec::new(),
            open_doors: Vec::new(),
        };
        Project {
            dir: PathBuf::from("/tmp/does-not-exist"),
            name: "T".to_string(),
            limits: LimitsConfig::default(),
            level_paths: vec!["levels/level00.ron".to_string()],
            levels: vec![level],
            characters: Vec::new(),
            party: Vec::new(),
            items: Vec::new(),
            monsters: Vec::new(),
            magics: Vec::new(),
            rules: crate::rules::RulesConfig::default(),
        }
    }

    #[test]
    fn paint_stroke_is_one_undo_op() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.selected = Block::Water;
        s.paint(1, 1);
        s.paint(2, 1);
        s.paint(1, 1); // repaint same cell — must not create a second op entry
        s.end_stroke();
        assert_eq!(s.block_at(1, 1), Some(Block::Water));
        assert_eq!(s.block_at(2, 1), Some(Block::Water));
        assert_eq!(s.undo.len(), 1);
        assert_eq!(s.undo[0].cells.len(), 2);

        s.undo();
        assert_eq!(s.block_at(1, 1), Some(Block::Empty));
        assert_eq!(s.block_at(2, 1), Some(Block::Empty));
        s.redo();
        assert_eq!(s.block_at(1, 1), Some(Block::Water));
    }

    #[test]
    fn cannot_wall_over_start() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.selected = Block::Wall;
        s.paint(0, 0); // start cell
        s.end_stroke();
        assert_eq!(s.block_at(0, 0), Some(Block::Empty));
        assert!(s.undo.is_empty());
    }

    #[test]
    fn set_start_moves_then_cycles_facing() {
        let mut s = EditorState::new(two_floor_project());
        s.floor_index = 1;
        s.set_start(2, 2); // move (start cell was 0,0)
        assert_eq!(s.cur().start, GridPos::new(2, 2, 1));
        assert_eq!(s.cur().start_facing, Facing::North);
        s.set_start(2, 2); // same cell -> cycle facing N->E
        assert_eq!(s.cur().start_facing, Facing::East);
        s.undo(); // undo the facing cycle
        assert_eq!(s.cur().start_facing, Facing::North);
        s.undo(); // undo the move
        assert_eq!(s.cur().start, GridPos::new(0, 0, 1));
    }
}
