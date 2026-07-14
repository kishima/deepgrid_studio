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

use crate::character::{Character, CharacterState, Party, PartyMember};
use crate::config::LimitsConfig;
use crate::dungeon::level::{Floor, Level};
use crate::dungeon::{Block, Dungeon, Facing, GridPos};
use crate::event::EventDef;
use crate::item::{Inventory, ItemCatalog, ItemDef, ItemInstance, ItemPlacement};
use crate::magic::{MagicCatalog, MagicDef};
use crate::monster::{MonsterCatalog, MonsterDef, MonsterPlacement};
use crate::rules::RulesConfig;

/// `project.ron` on disk.
///
/// `characters` / `party` are `#[serde(default)]` so a version-1 project (which
/// predates characters) still parses: they come back empty and the runtime
/// starts with no party (project.md「上限値の扱い」/ plan4 backward-compat).
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ProjectMeta {
    name: String,
    /// Project-format version (bumped when the schema changes).
    version: u32,
    limits: LimitsConfig,
    /// Level file paths relative to the project dir; order = level number.
    levels: Vec<String>,
    /// Registered-characters file, project-relative (v2+). Empty string (the
    /// default when the field is absent, as in v1) means "no characters file".
    #[serde(default)]
    characters: String,
    /// Party roster: character ids, in slot order (v2+). Empty in v1.
    #[serde(default)]
    party: Vec<String>,
    /// Item-definitions file, project-relative (v3+). Empty = no items.
    #[serde(default)]
    items: String,
    /// Monster-definitions file, project-relative (v4+). Empty = no monsters.
    #[serde(default)]
    monsters: String,
    /// Magic-definitions file, project-relative (v5+). Empty = no magic.
    #[serde(default)]
    magics: String,
    /// Per-project game rules (plan6.5). `#[serde(default)]` so pre-plan6.5
    /// projects (no `rules` block) load with everything at its default.
    #[serde(default)]
    rules: RulesConfig,
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
    /// Item placements (map format v3; `#[serde(default)]` so v2 files load).
    #[serde(default)]
    items: Vec<ItemPlacement>,
    /// Monster placements (map format v4; `#[serde(default)]` so v3 files load).
    #[serde(default)]
    monsters: Vec<MonsterPlacement>,
    /// Writable-wall texts (map v6): `(x, y, floor, text)`.
    #[serde(default)]
    wall_texts: Vec<WallText>,
    /// Stairs / 連絡通路 links (map v6).
    #[serde(default)]
    stairs_links: Vec<StairsLink>,
    /// Events attached to this level (map v6).
    #[serde(default)]
    events: Vec<EventDef>,
}

/// One writable-wall message (plan8). Kept out of `Block` so the block stays
/// `Copy`; matched to a `WritableWall` cell by coordinate.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WallText {
    pub x: i32,
    pub y: i32,
    pub floor: usize,
    pub text: String,
}

/// A 連絡通路 link: entering the `Stairs` at `from` moves the party to `to` on
/// `to_level`, facing `to_facing` (plan8).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StairsLink {
    pub from: (i32, i32, usize),
    pub to_level: usize,
    pub to: (i32, i32, usize),
    pub to_facing: Facing,
}

/// An in-memory level: the mutable block grids, the player's start, and item
/// placements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelData {
    pub start: GridPos,
    pub start_facing: Facing,
    pub level: Level,
    /// Items placed on this level (plan5).
    pub items: Vec<ItemPlacement>,
    /// Monsters placed on this level (plan6).
    pub monsters: Vec<MonsterPlacement>,
    /// Writable-wall texts (plan8), keyed by coordinate.
    pub wall_texts: Vec<WallText>,
    /// Stairs links out of this level (plan8).
    pub stairs_links: Vec<StairsLink>,
    /// Events attached to this level (plan8).
    pub events: Vec<EventDef>,
    /// Door kinds that start open on this level (derived from `!`/`@` glyphs).
    pub open_doors: Vec<u8>,
}

impl LevelData {
    /// The writable-wall text at `(x, y, floor)`, if any.
    pub fn wall_text_at(&self, x: i32, y: i32, floor: usize) -> Option<&str> {
        self.wall_texts
            .iter()
            .find(|w| w.x == x && w.y == y && w.floor == floor)
            .map(|w| w.text.as_str())
    }

    /// The stairs link out of `(x, y, floor)`, if any.
    pub fn stairs_link_at(&self, x: i32, y: i32, floor: usize) -> Option<&StairsLink> {
        self.stairs_links.iter().find(|s| s.from == (x, y, floor))
    }
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

/// Project-format version this build writes. Older versions are still accepted
/// on load (v1: no characters; v2: no items; v3: no monsters; v4: no magic;
/// v5: no events/gimmicks).
pub const PROJECT_VERSION: u32 = 6;

/// A loaded project: metadata, limits, levels, registered characters + party
/// roster (v2+), and item definitions (v3+).
pub struct Project {
    pub dir: PathBuf,
    pub name: String,
    pub limits: LimitsConfig,
    pub level_paths: Vec<String>,
    pub levels: Vec<LevelData>,
    /// All registered characters (empty for a v1 project).
    pub characters: Vec<Character>,
    /// Party roster as character ids, validated against `characters`.
    pub party: Vec<String>,
    /// All item definitions (empty for a pre-v3 project).
    pub items: Vec<ItemDef>,
    /// All monster definitions (empty for a pre-v4 project).
    pub monsters: Vec<MonsterDef>,
    /// All magic definitions (empty for a pre-v5 project).
    pub magics: Vec<MagicDef>,
    /// Per-project game rules (defaults for a pre-plan6.5 project).
    pub rules: RulesConfig,
}

impl Project {
    /// Build the item catalog from the loaded definitions (ids already unique).
    pub fn build_catalog(&self) -> ItemCatalog {
        ItemCatalog::from_defs(self.items.clone(), "items").unwrap_or_default()
    }

    /// Build the monster catalog from the loaded definitions.
    pub fn build_monster_catalog(&self) -> MonsterCatalog {
        MonsterCatalog::from_defs(self.monsters.clone(), "monsters").unwrap_or_default()
    }

    /// Build the magic catalog from the loaded definitions (ids already unique).
    pub fn build_magic_catalog(&self) -> MagicCatalog {
        MagicCatalog::from_defs(self.magics.clone(), "magics").unwrap_or_default()
    }

    /// Build the runtime [`Party`] resource: resolve each roster id to its
    /// character, give it full starting state and an inventory holding its
    /// starting items (equippable ones auto-equipped). Ids are guaranteed present
    /// by load-time validation.
    pub fn build_party(&self) -> Party {
        let catalog = self.build_catalog();
        let members = self
            .party
            .iter()
            .filter_map(|id| self.characters.iter().find(|c| &c.id == id))
            .map(|character| {
                let mut inventory =
                    Inventory::new(self.limits.pouch_size, self.limits.backpack_size);
                for item_id in &character.items {
                    let instance = ItemInstance::new(item_id.clone());
                    match inventory.pickup(instance) {
                        Ok(slot) => {
                            // Auto-equip equippable starting gear so it shows worn.
                            if catalog
                                .get(item_id)
                                .is_some_and(|d| d.is_equippable())
                            {
                                let _ = inventory.equip(slot, &catalog);
                            }
                        }
                        Err(_) => {
                            eprintln!(
                                "deepgrid_studio: {}'s starting item '{item_id}' didn't fit",
                                character.id
                            );
                        }
                    }
                }
                let mut state = CharacterState::full(character);
                state.satiety = self.rules.hunger.satiety_max;
                // Seed initially-known magic (plan7), mirroring starting items.
                state.learned = character.magics.clone();
                PartyMember {
                    character: character.clone(),
                    state,
                    inventory,
                }
            })
            .collect();
        Party { members }
    }
}

/// Parse and validate an `items.ron` (a `Vec<ItemDef>`) against `limits`: count
/// ≤ `max_item_kinds`. (Id uniqueness is checked when the catalog is built.)
fn items_from_ron(text: &str, limits: &LimitsConfig, what: &str) -> Result<Vec<ItemDef>, String> {
    let items: Vec<ItemDef> =
        ron::from_str(text).map_err(|e| format!("failed to parse {what}: {e}"))?;
    if items.len() > limits.max_item_kinds {
        return Err(format!(
            "{what} has {} item kinds, exceeds max_item_kinds {}",
            items.len(),
            limits.max_item_kinds
        ));
    }
    // Surface duplicate ids here for a clearer message than catalog build.
    ItemCatalog::from_defs(items.clone(), what)?;
    Ok(items)
}

/// Parse and validate a `monsters.ron` (a `Vec<MonsterDef>`) against `limits`:
/// count ≤ `max_monster_kinds`, ids unique.
fn monsters_from_ron(
    text: &str,
    limits: &LimitsConfig,
    what: &str,
) -> Result<Vec<MonsterDef>, String> {
    let monsters: Vec<MonsterDef> =
        ron::from_str(text).map_err(|e| format!("failed to parse {what}: {e}"))?;
    if monsters.len() > limits.max_monster_kinds {
        return Err(format!(
            "{what} has {} monster kinds, exceeds max_monster_kinds {}",
            monsters.len(),
            limits.max_monster_kinds
        ));
    }
    MonsterCatalog::from_defs(monsters.clone(), what)?;
    Ok(monsters)
}

/// Parse and validate a `magics.ron` (a `Vec<MagicDef>`) against `limits`:
/// count ≤ `max_magic_kinds`, ids unique (checked at catalog build).
fn magics_from_ron(text: &str, limits: &LimitsConfig, what: &str) -> Result<Vec<MagicDef>, String> {
    let magics: Vec<MagicDef> =
        ron::from_str(text).map_err(|e| format!("failed to parse {what}: {e}"))?;
    if magics.len() > limits.max_magic_kinds {
        return Err(format!(
            "{what} has {} magic kinds, exceeds max_magic_kinds {}",
            magics.len(),
            limits.max_magic_kinds
        ));
    }
    MagicCatalog::from_defs(magics.clone(), what)?;
    Ok(magics)
}

/// Parse and validate a `characters.ron` (a `Vec<Character>`) against `limits`:
/// count ≤ `max_characters` and ids unique.
fn characters_from_ron(
    text: &str,
    limits: &LimitsConfig,
    what: &str,
) -> Result<Vec<Character>, String> {
    let characters: Vec<Character> =
        ron::from_str(text).map_err(|e| format!("failed to parse {what}: {e}"))?;
    if characters.len() > limits.max_characters {
        return Err(format!(
            "{what} has {} characters, exceeds max_characters {}",
            characters.len(),
            limits.max_characters
        ));
    }
    for (i, c) in characters.iter().enumerate() {
        if characters[..i].iter().any(|o| o.id == c.id) {
            return Err(format!("{what}: duplicate character id '{}'", c.id));
        }
    }
    Ok(characters)
}

/// Validate the party roster against the loaded characters and `limits`: size ≤
/// `party_size` and every id exists.
fn validate_party(party: &[String], characters: &[Character], limits: &LimitsConfig) -> Result<(), String> {
    if party.len() > limits.party_size {
        return Err(format!(
            "party has {} members, exceeds party_size {}",
            party.len(),
            limits.party_size
        ));
    }
    for id in party {
        if !characters.iter().any(|c| &c.id == id) {
            return Err(format!("party references unknown character id '{id}'"));
        }
    }
    Ok(())
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
        // plan8 terrain / gimmick glyphs.
        'o' => Block::Hole,
        'u' => Block::Stairs { up: true },
        'd' => Block::Stairs { up: false },
        'W' => Block::WritableWall,
        'A' => Block::HoroscopeVert { from_below: true },
        'V' => Block::HoroscopeVert { from_below: false },
        'K' => Block::Keyhole,
        'S' => Block::Switch,
        'P' => Block::FloorPlate,
        'T' => Block::WarpPoint,
        // Note: the door-initial-open glyphs '!' / '@' are handled by the caller
        // (they map to Door{kind} *and* mark the kind open), not here.
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
        Block::Hole => 'o',
        Block::Stairs { up: true } => 'u',
        Block::Stairs { up: false } => 'd',
        Block::WritableWall => 'W',
        Block::HoroscopeVert { from_below: true } => 'A',
        Block::HoroscopeVert { from_below: false } => 'V',
        Block::Keyhole => 'K',
        Block::Switch => 'S',
        Block::FloorPlate => 'P',
        Block::WarpPoint => 'T',
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
    let mut open_doors: Vec<u8> = Vec::new();
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
                // '!' / '@' are Door{kind 0/1} that additionally start open.
                let block = match c {
                    '!' => {
                        if !open_doors.contains(&0) {
                            open_doors.push(0);
                        }
                        Block::Door { kind: 0 }
                    }
                    '@' => {
                        if !open_doors.contains(&1) {
                            open_doors.push(1);
                        }
                        Block::Door { kind: 1 }
                    }
                    _ => char_to_block(c).ok_or_else(|| {
                        format!("{what} floor {fi} row {y} col {x}: unknown block '{c}'")
                    })?,
                };
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
    open_doors.sort_unstable();

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

    if map.items.len() > limits.item_placements_per_level {
        return Err(format!(
            "{what} places {} items, exceeds item_placements_per_level {}",
            map.items.len(),
            limits.item_placements_per_level
        ));
    }
    for p in &map.items {
        if level.block_at(GridPos::new(p.x, p.y, p.floor)).is_none() {
            return Err(format!(
                "{what} item '{}' at ({},{},floor {}) is out of bounds",
                p.id, p.x, p.y, p.floor
            ));
        }
    }

    if map.monsters.len() > limits.monster_placements_per_level {
        return Err(format!(
            "{what} places {} monsters, exceeds monster_placements_per_level {}",
            map.monsters.len(),
            limits.monster_placements_per_level
        ));
    }
    let kinds: std::collections::HashSet<&str> =
        map.monsters.iter().map(|m| m.id.as_str()).collect();
    if kinds.len() > limits.monster_kinds_per_level {
        return Err(format!(
            "{what} uses {} monster kinds, exceeds monster_kinds_per_level {}",
            kinds.len(),
            limits.monster_kinds_per_level
        ));
    }
    for m in &map.monsters {
        if level.block_at(GridPos::new(m.x, m.y, m.floor)).is_none() {
            return Err(format!(
                "{what} monster '{}' at ({},{},floor {}) is out of bounds",
                m.id, m.x, m.y, m.floor
            ));
        }
    }

    Ok(LevelData {
        start,
        start_facing: map.start_facing,
        level,
        items: map.items.clone(),
        monsters: map.monsters.clone(),
        wall_texts: map.wall_texts.clone(),
        stairs_links: map.stairs_links.clone(),
        events: map.events.clone(),
        open_doors,
    })
}

/// Serialize a `LevelData` back to the on-disk `MapFile` (block grids -> rows).
fn map_from_level(data: &LevelData) -> MapFile {
    // A door of an initially-open kind serialises to '!' / '@' so the open state
    // round-trips (the DoorStates model is per-kind, so all cells of an open kind
    // are written open).
    let glyph = |block: Block| -> char {
        if let Block::Door { kind } = block
            && data.open_doors.contains(&kind)
        {
            return if kind == 0 { '!' } else { '@' };
        }
        block_to_char(block)
    };
    let floors = data
        .level
        .floors
        .iter()
        .map(|floor| {
            let rows = (0..floor.height)
                .map(|y| {
                    (0..floor.width)
                        .map(|x| glyph(floor.blocks[y * floor.width + x]))
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
        items: data.items.clone(),
        monsters: data.monsters.clone(),
        wall_texts: data.wall_texts.clone(),
        stairs_links: data.stairs_links.clone(),
        events: data.events.clone(),
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

    if meta.version == 0 || meta.version > PROJECT_VERSION {
        return Err(format!(
            "{}: unsupported project version {} (this build reads versions 1..={PROJECT_VERSION})",
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

    // Characters + party (v2). A v1 project (or a v2 one that omits the
    // characters file) loads with no party; the HUD then hides the status window.
    let characters = if meta.characters.is_empty() {
        if meta.party.is_empty() {
            eprintln!(
                "deepgrid_studio: {} has no characters/party — starting with an empty party",
                meta_path.display()
            );
        }
        Vec::new()
    } else {
        let path = dir.join(&meta.characters);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        characters_from_ron(&text, &meta.limits, &meta.characters)?
    };
    validate_party(&meta.party, &characters, &meta.limits)?;

    // Items (v3). A pre-v3 project (or one omitting the items file) loads with no
    // items.
    let items = if meta.items.is_empty() {
        Vec::new()
    } else {
        let path = dir.join(&meta.items);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        items_from_ron(&text, &meta.limits, &meta.items)?
    };

    // Monsters (v4). Pre-v4 projects load with no monsters.
    let monsters = if meta.monsters.is_empty() {
        Vec::new()
    } else {
        let path = dir.join(&meta.monsters);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        monsters_from_ron(&text, &meta.limits, &meta.monsters)?
    };

    // Magics (v5). Pre-v5 projects load with no magic.
    let magics = if meta.magics.is_empty() {
        Vec::new()
    } else {
        let path = dir.join(&meta.magics);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        magics_from_ron(&text, &meta.limits, &meta.magics)?
    };

    // Cross-check placements + starting items reference known item ids.
    let known = |id: &str| items.iter().any(|d| d.id == id);
    let known_monster = |id: &str| monsters.iter().any(|d| d.id == id);
    let known_magic = |id: &str| magics.iter().any(|d| d.id == id);
    for (rel, level) in meta.levels.iter().zip(&levels) {
        for p in &level.items {
            if !known(&p.id) {
                return Err(format!("{rel}: placed item '{}' is not defined in {}", p.id, meta.items));
            }
        }
        for m in &level.monsters {
            if !known_monster(&m.id) {
                return Err(format!(
                    "{rel}: placed monster '{}' is not defined in {}",
                    m.id, meta.monsters
                ));
            }
        }
    }
    for c in &characters {
        for id in &c.items {
            if !known(id) {
                return Err(format!(
                    "character '{}' starting item '{id}' is not defined in {}",
                    c.id, meta.items
                ));
            }
        }
        for id in &c.magics {
            if !known_magic(id) {
                return Err(format!(
                    "character '{}' starting magic '{id}' is not defined in {}",
                    c.id, meta.magics
                ));
            }
        }
    }
    // Scroll `teaches` must reference a defined magic.
    for d in &items {
        if let Some(id) = &d.teaches
            && !known_magic(id)
        {
            return Err(format!(
                "item '{}' teaches magic '{id}' not defined in {}",
                d.id, meta.magics
            ));
        }
    }
    for def in &monsters {
        for id in def.carry_items.iter().chain(&def.attack_items) {
            if !known(id) {
                return Err(format!(
                    "monster '{}' references unknown item '{id}' in {}",
                    def.id, meta.items
                ));
            }
        }
    }

    Ok(Project {
        dir,
        name: meta.name,
        limits: meta.limits,
        level_paths: meta.levels,
        levels,
        characters,
        party: meta.party,
        items,
        monsters,
        magics,
        rules: meta.rules,
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
            items: Vec::new(),
            monsters: Vec::new(),
            wall_texts: Vec::new(),
            stairs_links: Vec::new(),
            events: Vec::new(),
            open_doors: Vec::new(),
        }
    }

    #[test]
    fn all_glyphs_round_trip_by_char() {
        // Every glyph except the door-open aliases '!'/'@' (which collapse to
        // Door{kind} and re-emit as '1'/'2' unless open — see the level test).
        for c in [
            '#', '.', '~', '^', '%', 'H', '1', '2', '<', '>', 'n', 'v',
            'o', 'u', 'd', 'W', 'A', 'V', 'K', 'S', 'P', 'T',
        ] {
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
    fn open_door_glyph_round_trips() {
        // A Door{kind 0} marked open serialises to '!' and reads back open.
        let limits = LimitsConfig::default();
        let mut lvl = sample_level();
        // Put an open kind-0 door in floor 1 and mark it open.
        lvl.level.floors[1].blocks[6] = Block::Door { kind: 0 };
        lvl.open_doors = vec![0];
        let ron = level_to_ron(&lvl).expect("serialize");
        assert!(ron.contains('!'), "open door should serialise as '!'");
        let restored = level_from_ron(&ron, &limits, "roundtrip").expect("parse");
        assert_eq!(restored.open_doors, vec![0]);
        assert_eq!(restored.level.floors[1].blocks[6], Block::Door { kind: 0 });
    }

    #[test]
    fn plan8_glyphs_and_events_round_trip() {
        use crate::event::{EventAction, EventDef, TriggerKind};
        let limits = LimitsConfig::default();
        let mut lvl = sample_level();
        // Sprinkle new terrain into floor 1 (avoiding the start cell 0,0).
        lvl.level.floors[1].blocks[3] = Block::Hole; // (3,0)
        lvl.level.floors[1].blocks[7] = Block::Stairs { up: true }; // (3,1)
        lvl.level.floors[1].blocks[11] = Block::WritableWall; // (3,2)
        lvl.wall_texts = vec![WallText { x: 3, y: 2, floor: 1, text: "壁の文字".into() }];
        lvl.stairs_links = vec![StairsLink {
            from: (3, 1, 1),
            to_level: 0,
            to: (2, 2, 0),
            to_facing: Facing::North,
        }];
        lvl.events = vec![EventDef {
            id: "ev1".into(),
            trigger: TriggerKind::SwitchPush,
            at: (0, 1, 1),
            delay_cycles: 3,
            flags: vec![],
            join: crate::event::FlagJoin::And,
            actions: vec![EventAction::SetFlag { flag: 2, on: true }],
        }];
        let ron = level_to_ron(&lvl).expect("serialize");
        let restored = level_from_ron(&ron, &limits, "roundtrip").expect("parse");
        assert_eq!(lvl, restored);
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

    use crate::character::{Character, GrowthType, Stats};

    fn sample_character(id: &str) -> Character {
        Character {
            id: id.to_string(),
            first_name: "テスト".into(),
            last_name: "姓".into(),
            gender: "男".into(),
            height_cm: 175.0,
            weight_kg: 68.0,
            birth_date: "1000-01-01".into(),
            age: 22,
            likes: "剣".into(),
            dislikes: "毒".into(),
            background: "戦士。\n二行目。".into(),
            growth: GrowthType::Average,
            items: Vec::new(),
            magics: Vec::new(),
            stats: Stats {
                level: 3,
                max_hp: 120,
                max_mp: 20,
                attack: 15,
                defense: 12,
                agility: 10,
                throwing: 8,
                carrying: 14,
                lung_capacity: 9,
                heat_resist: 7,
                poison_resist: 6,
                magic_knowledge: 4,
                concentration: 30,
                appraisal: 5,
                stealing: 3,
                bite: 2,
            },
            model: "models/party/knight.glb".into(),
            portrait: "projects/sample/portraits/knight.png".into(),
        }
    }

    #[test]
    fn characters_ron_round_trip() {
        let limits = LimitsConfig::default();
        let original = vec![sample_character("knight"), sample_character("mage")];
        let ron = ron::ser::to_string_pretty(&original, ron::ser::PrettyConfig::default())
            .expect("serialize");
        let restored = characters_from_ron(&ron, &limits, "characters.ron").expect("parse");
        assert_eq!(original, restored);
    }

    #[test]
    fn rejects_duplicate_character_ids() {
        let limits = LimitsConfig::default();
        let dup = vec![sample_character("knight"), sample_character("knight")];
        let ron = ron::ser::to_string_pretty(&dup, ron::ser::PrettyConfig::default()).unwrap();
        assert!(characters_from_ron(&ron, &limits, "characters.ron").is_err());
    }

    #[test]
    fn rejects_too_many_characters() {
        let limits = LimitsConfig {
            max_characters: 1,
            ..LimitsConfig::default()
        };
        let two = vec![sample_character("a"), sample_character("b")];
        let ron = ron::ser::to_string_pretty(&two, ron::ser::PrettyConfig::default()).unwrap();
        assert!(characters_from_ron(&ron, &limits, "characters.ron").is_err());
    }

    #[test]
    fn party_validation() {
        let limits = LimitsConfig::default();
        let chars = vec![sample_character("knight"), sample_character("mage")];
        // Good roster resolves.
        assert!(validate_party(&["knight".into(), "mage".into()], &chars, &limits).is_ok());
        // Unknown id is rejected.
        assert!(validate_party(&["ghost".into()], &chars, &limits).is_err());
        // Oversize roster is rejected.
        let small = LimitsConfig {
            party_size: 1,
            ..LimitsConfig::default()
        };
        assert!(validate_party(&["knight".into(), "mage".into()], &chars, &small).is_err());
    }

    #[test]
    fn level_items_round_trip() {
        let limits = LimitsConfig::default();
        let mut lvl = sample_level();
        lvl.items = vec![
            ItemPlacement { id: "sword".into(), x: 1, y: 0, floor: 1 },
            ItemPlacement { id: "bread".into(), x: 2, y: 1, floor: 1 },
        ];
        let ron = level_to_ron(&lvl).expect("serialize");
        let restored = level_from_ron(&ron, &limits, "roundtrip").expect("parse");
        assert_eq!(lvl.items, restored.items);
    }

    #[test]
    fn level_monsters_round_trip() {
        let limits = LimitsConfig::default();
        let mut lvl = sample_level();
        lvl.monsters = vec![crate::monster::MonsterPlacement {
            id: "skel".into(),
            x: 1,
            y: 0,
            floor: 1,
            facing: Facing::South,
        }];
        let ron = level_to_ron(&lvl).expect("serialize");
        let restored = level_from_ron(&ron, &limits, "roundtrip").expect("parse");
        assert_eq!(lvl.monsters, restored.monsters);
    }

    #[test]
    fn sample_project_loads() {
        // Guards every sample RON file (project/characters/items/level) against
        // schema drift — the same load path the runtime uses.
        let project = load_project("assets/projects/sample").expect("sample project loads");
        assert_eq!(project.party.len(), 4);
        assert!(project.items.iter().any(|d| d.id == "sword_iron"));
        assert!(!project.levels[0].items.is_empty());
        assert!(project.monsters.iter().any(|d| d.id == "skel_minion"));
        assert!(!project.levels[0].monsters.is_empty());
        // Knight starts armed + armoured.
        let party = project.build_party();
        let knight = &party.members[0];
        assert!(knight.inventory.get(crate::item::SlotRef::Equip(
            crate::item::EquipSlot::Head
        )).is_some());
    }

    #[test]
    fn build_party_gives_full_state() {
        let chars = vec![sample_character("knight")];
        let project = Project {
            dir: PathBuf::from("/tmp/x"),
            name: "t".into(),
            limits: LimitsConfig::default(),
            level_paths: vec![],
            levels: vec![],
            characters: chars,
            party: vec!["knight".into()],
            items: vec![],
            monsters: vec![],
            magics: vec![],
            rules: RulesConfig::default(),
        };
        let party = project.build_party();
        assert_eq!(party.len(), 1);
        assert_eq!(party.members[0].state.hp, 120);
        assert_eq!(party.members[0].state.concentration, 30);
        assert!(!party.members[0].state.down);
    }
}
