//! Project format (plan3): a game is authored as one directory.
//!
//! ```text
//! <dir>/
//! ├── project.ron          ← metadata + LimitsConfig
//! └── levels/
//!     └── levelNN.ron      ← map format v2 (same layout the runtime reads)
//! ```
//!
//! This module is the single read/write path for both the runtime (`main.rs`)
//! and the editor: `load_project` / `save_level`. Writing a level and reading it
//! back must reproduce the same data (round-trip; see the tests) — comments are
//! not preserved.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::LimitsConfig;
use crate::dungeon::level::{Floor, Level};
use crate::dungeon::{Block, Dungeon, Facing, GridPos};

/// `project.ron` on disk.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ProjectMeta {
    name: String,
    /// Project-format version (bumped when the schema changes).
    version: u32,
    limits: LimitsConfig,
    /// Level file paths relative to the project dir; order = level number.
    levels: Vec<String>,
}

/// One floor of the on-disk map: `height` rows of `width` characters.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct FloorRows {
    rows: Vec<String>,
}

/// On-disk level (map format v2). A level is a stack of floors, `floors[0]`
/// lowest. Character legend: `#`=Wall, `.`=Empty, `~`=Water, `^`=Fire,
/// `%`=Poison, `H`=Ladder, `1`/`2`=Door(kind 0/1), one-way Horoscope arrows
/// `<`=West, `>`=East, `n`=North, `v`=South (the direction you may travel).
#[derive(Serialize, Deserialize, Clone, Debug)]
struct MapFile {
    width: usize,
    height: usize,
    start_x: i32,
    start_y: i32,
    start_floor: usize,
    start_facing: Facing,
    floors: Vec<FloorRows>,
}

/// An in-memory level: the mutable block grids plus the player's start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelData {
    pub start: GridPos,
    pub start_facing: Facing,
    pub level: Level,
}

impl LevelData {
    pub fn width(&self) -> usize {
        self.level.floor(0).map(|f| f.width).unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.level.floor(0).map(|f| f.height).unwrap_or(0)
    }

    pub fn floor_count(&self) -> usize {
        self.level.floor_count()
    }

    /// A runtime `Dungeon` view of this level (play mode starts here).
    pub fn to_dungeon(&self) -> Dungeon {
        Dungeon {
            level: self.level.clone(),
            start_pos: self.start,
            start_facing: self.start_facing,
        }
    }
}

/// Project-format version this build reads/writes.
pub const PROJECT_VERSION: u32 = 1;

/// A loaded project: metadata, limits, and every level in memory.
pub struct Project {
    pub dir: PathBuf,
    pub name: String,
    pub limits: LimitsConfig,
    pub level_paths: Vec<String>,
    pub levels: Vec<LevelData>,
}

fn char_to_block(c: char) -> Option<Block> {
    Some(match c {
        '#' => Block::Wall,
        '.' => Block::Empty,
        '~' => Block::Water,
        '^' => Block::Fire,
        '%' => Block::Poison,
        'H' => Block::Ladder,
        '1' => Block::Door { kind: 0 },
        '2' => Block::Door { kind: 1 },
        '<' => Block::Horoscope { pass_from: Facing::West },
        '>' => Block::Horoscope { pass_from: Facing::East },
        'n' => Block::Horoscope { pass_from: Facing::North },
        'v' => Block::Horoscope { pass_from: Facing::South },
        _ => return None,
    })
}

/// Inverse of [`char_to_block`]. Every `Block` has a glyph, so map files stay
/// hand-editable and round-trips are lossless.
pub fn block_to_char(block: Block) -> char {
    match block {
        Block::Wall => '#',
        Block::Empty => '.',
        Block::Water => '~',
        Block::Fire => '^',
        Block::Poison => '%',
        Block::Ladder => 'H',
        Block::Door { kind: 0 } => '1',
        Block::Door { .. } => '2',
        Block::Horoscope { pass_from: Facing::West } => '<',
        Block::Horoscope { pass_from: Facing::East } => '>',
        Block::Horoscope { pass_from: Facing::North } => 'n',
        Block::Horoscope { pass_from: Facing::South } => 'v',
    }
}

/// Parse + validate a `MapFile` into a `LevelData`, checking it against
/// `limits`. Returns a human-readable error (shown in the editor status bar;
/// the runtime turns it into a panic).
fn level_from_map(map: &MapFile, limits: &LimitsConfig, what: &str) -> Result<LevelData, String> {
    if map.width > limits.floor_width || map.height > limits.floor_height {
        return Err(format!(
            "{what} is {}x{}, exceeds configured floor size {}x{}",
            map.width, map.height, limits.floor_width, limits.floor_height
        ));
    }
    if map.floors.is_empty() || map.floors.len() > limits.floors_per_level {
        return Err(format!(
            "{what} has {} floors, must be 1..={}",
            map.floors.len(),
            limits.floors_per_level
        ));
    }

    let mut floors = Vec::with_capacity(map.floors.len());
    for (fi, floor) in map.floors.iter().enumerate() {
        if floor.rows.len() != map.height {
            return Err(format!(
                "{what} floor {fi} declares height {} but has {} rows",
                map.height,
                floor.rows.len()
            ));
        }
        let mut blocks = Vec::with_capacity(map.width * map.height);
        for (y, row) in floor.rows.iter().enumerate() {
            let cols: Vec<char> = row.chars().collect();
            if cols.len() != map.width {
                return Err(format!(
                    "{what} floor {fi} row {y} has {} columns, expected {}",
                    cols.len(),
                    map.width
                ));
            }
            for (x, &c) in cols.iter().enumerate() {
                let block = char_to_block(c)
                    .ok_or_else(|| format!("{what} floor {fi} row {y} col {x}: unknown block '{c}'"))?;
                if let Block::Door { kind } = block
                    && (kind as usize) >= limits.door_kinds_per_level
                {
                    return Err(format!(
                        "{what} floor {fi} row {y} col {x}: door kind {kind} exceeds \
                         door_kinds_per_level {}",
                        limits.door_kinds_per_level
                    ));
                }
                blocks.push(block);
            }
        }
        floors.push(Floor {
            width: map.width,
            height: map.height,
            blocks,
        });
    }

    let level = Level { floors };
    if map.start_floor >= level.floor_count() {
        return Err(format!(
            "{what} start_floor {} is out of range (0..{})",
            map.start_floor,
            level.floor_count()
        ));
    }
    let start = GridPos::new(map.start_x, map.start_y, map.start_floor);
    match level.block_at(start) {
        None => return Err(format!("{what} start ({},{}) is out of bounds", start.x, start.y)),
        Some(b) if b.is_wall() => {
            return Err(format!("{what} start ({},{}) is inside a wall", start.x, start.y));
        }
        Some(_) => {}
    }
    if !level.is_supported(start.x, start.y, start.floor) {
        return Err(format!(
            "{what} start ({},{},floor {}) has no footing (would fall)",
            start.x, start.y, start.floor
        ));
    }

    Ok(LevelData {
        start,
        start_facing: map.start_facing,
        level,
    })
}

/// Serialize a `LevelData` back to the on-disk `MapFile` (block grids -> rows).
fn map_from_level(data: &LevelData) -> MapFile {
    let floors = data
        .level
        .floors
        .iter()
        .map(|floor| {
            let rows = (0..floor.height)
                .map(|y| {
                    (0..floor.width)
                        .map(|x| block_to_char(floor.blocks[y * floor.width + x]))
                        .collect::<String>()
                })
                .collect();
            FloorRows { rows }
        })
        .collect();
    MapFile {
        width: data.width(),
        height: data.height(),
        start_x: data.start.x,
        start_y: data.start.y,
        start_floor: data.start.floor,
        start_facing: data.start_facing,
        floors,
    }
}

/// Render a level to a RON string (pretty). Public so the editor can preview /
/// the tests can round-trip without touching the filesystem.
pub fn level_to_ron(data: &LevelData) -> Result<String, String> {
    let pretty = ron::ser::PrettyConfig::default();
    ron::ser::to_string_pretty(&map_from_level(data), pretty)
        .map_err(|e| format!("failed to serialize level: {e}"))
}

/// Parse a level from a RON string, validating against `limits`.
pub fn level_from_ron(text: &str, limits: &LimitsConfig, what: &str) -> Result<LevelData, String> {
    let map: MapFile = ron::from_str(text).map_err(|e| format!("failed to parse {what}: {e}"))?;
    level_from_map(&map, limits, what)
}

/// Load a project directory: read `project.ron`, then every level file.
pub fn load_project(dir: impl AsRef<Path>) -> Result<Project, String> {
    let dir = dir.as_ref().to_path_buf();
    let meta_path = dir.join("project.ron");
    let text = std::fs::read_to_string(&meta_path)
        .map_err(|e| format!("failed to read {}: {e}", meta_path.display()))?;
    let meta: ProjectMeta =
        ron::from_str(&text).map_err(|e| format!("failed to parse {}: {e}", meta_path.display()))?;

    if meta.version != PROJECT_VERSION {
        return Err(format!(
            "{}: unsupported project version {} (this build reads version {PROJECT_VERSION})",
            meta_path.display(),
            meta.version,
        ));
    }
    if meta.levels.is_empty() {
        return Err(format!("{} lists no levels", meta_path.display()));
    }
    if meta.levels.len() > meta.limits.max_levels {
        return Err(format!(
            "{} lists {} levels, exceeds max_levels {}",
            meta_path.display(),
            meta.levels.len(),
            meta.limits.max_levels
        ));
    }

    let mut levels = Vec::with_capacity(meta.levels.len());
    for rel in &meta.levels {
        let path = dir.join(rel);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        levels.push(level_from_ron(&text, &meta.limits, rel)?);
    }

    Ok(Project {
        dir,
        name: meta.name,
        limits: meta.limits,
        level_paths: meta.levels,
        levels,
    })
}

/// Write one level back to disk, backing up the previous file to `*.ron.bak`
/// (one generation). Does not panic on I/O failure — returns the error for the
/// editor's status bar.
pub fn save_level(dir: impl AsRef<Path>, rel_path: &str, data: &LevelData) -> Result<(), String> {
    let path = dir.as_ref().join(rel_path);
    let ron = level_to_ron(data)?;
    if path.exists() {
        let bak = path.with_extension("ron.bak");
        std::fs::copy(&path, &bak)
            .map_err(|e| format!("failed to back up {}: {e}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, ron).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built level exercising every block glyph, for round-trip tests.
    fn sample_level() -> LevelData {
        // 3x3 floor 0 (all wall = footing) under a floor 1 that uses each glyph.
        let floor0 = Floor {
            width: 4,
            height: 3,
            blocks: vec![Block::Wall; 12],
        };
        let f1 = vec![
            // row 0
            Block::Empty, Block::Wall, Block::Water, Block::Fire,
            // row 1
            Block::Poison, Block::Ladder, Block::Door { kind: 0 }, Block::Door { kind: 1 },
            // row 2
            Block::Horoscope { pass_from: Facing::West },
            Block::Horoscope { pass_from: Facing::East },
            Block::Horoscope { pass_from: Facing::North },
            Block::Horoscope { pass_from: Facing::South },
        ];
        let floor1 = Floor { width: 4, height: 3, blocks: f1 };
        LevelData {
            start: GridPos::new(0, 0, 1),
            start_facing: Facing::East,
            level: Level { floors: vec![floor0, floor1] },
        }
    }

    #[test]
    fn all_glyphs_round_trip_by_char() {
        for c in ['#', '.', '~', '^', '%', 'H', '1', '2', '<', '>', 'n', 'v'] {
            let block = char_to_block(c).expect("known glyph");
            assert_eq!(block_to_char(block), c, "glyph {c} did not round-trip");
        }
    }

    #[test]
    fn level_ron_round_trip() {
        let limits = LimitsConfig::default();
        let original = sample_level();
        let ron = level_to_ron(&original).expect("serialize");
        let restored = level_from_ron(&ron, &limits, "roundtrip").expect("parse");
        assert_eq!(original, restored);
    }

    #[test]
    fn rejects_start_in_wall() {
        let limits = LimitsConfig::default();
        let mut lvl = sample_level();
        // Put a wall at the start cell (floor 1, 0,0) and expect rejection.
        lvl.level.floors[1].blocks[0] = Block::Wall;
        let ron = level_to_ron(&lvl).expect("serialize");
        assert!(level_from_ron(&ron, &limits, "bad").is_err());
    }
}
