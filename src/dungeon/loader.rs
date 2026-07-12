use std::path::Path;

use serde::{Deserialize, Serialize};

use super::block::Block;
use super::level::{Dungeon, Facing, Floor, GridPos, Level};
use crate::config::LimitsConfig;

/// On-disk map format (RON). One floor described as rows of characters, so maps
/// stay hand-editable. Character legend: `#`=Wall, `.`=Empty, `~`=Water,
/// `^`=Fire, `%`=Poison.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct MapFile {
    width: usize,
    height: usize,
    start_x: i32,
    start_y: i32,
    start_facing: Facing,
    rows: Vec<String>,
}

fn char_to_block(c: char) -> Option<Block> {
    match c {
        '#' => Some(Block::Wall),
        '.' => Some(Block::Empty),
        '~' => Some(Block::Water),
        '^' => Some(Block::Fire),
        '%' => Some(Block::Poison),
        _ => None,
    }
}

/// Load a single-floor test level from a RON map file.
///
/// plan1 has no error UI: any problem (missing file, malformed RON, wrong
/// dimensions, unknown block char, out-of-bounds start) is a `panic!`. The map
/// is validated against `limits` so an oversized map fails loudly rather than
/// silently exceeding the configured floor size.
pub fn load_dungeon(path: impl AsRef<Path>, limits: &LimitsConfig) -> Dungeon {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read map {}: {e}", path.display()));
    let map: MapFile = ron::from_str(&text)
        .unwrap_or_else(|e| panic!("failed to parse map {}: {e}", path.display()));

    assert!(
        map.width <= limits.floor_width && map.height <= limits.floor_height,
        "map {} is {}x{}, exceeds configured floor size {}x{}",
        path.display(),
        map.width,
        map.height,
        limits.floor_width,
        limits.floor_height,
    );
    assert_eq!(
        map.rows.len(),
        map.height,
        "map {} declares height {} but has {} rows",
        path.display(),
        map.height,
        map.rows.len(),
    );

    let mut blocks = Vec::with_capacity(map.width * map.height);
    for (y, row) in map.rows.iter().enumerate() {
        let cols: Vec<char> = row.chars().collect();
        assert_eq!(
            cols.len(),
            map.width,
            "map {} row {y} has {} columns, expected {}",
            path.display(),
            cols.len(),
            map.width,
        );
        for (x, &c) in cols.iter().enumerate() {
            let block = char_to_block(c).unwrap_or_else(|| {
                panic!(
                    "map {} row {y} col {x}: unknown block '{c}'",
                    path.display()
                )
            });
            blocks.push(block);
        }
    }

    let floor = Floor {
        width: map.width,
        height: map.height,
        blocks,
    };

    let start = GridPos::new(map.start_x, map.start_y, 0);
    let start_block = floor.get(start.x, start.y).unwrap_or_else(|| {
        panic!(
            "map {} start ({},{}) is out of bounds",
            path.display(),
            start.x,
            start.y
        )
    });
    assert!(
        start_block.is_walkable(),
        "map {} start ({},{}) is inside a wall",
        path.display(),
        start.x,
        start.y,
    );

    Dungeon {
        level: Level {
            floors: vec![floor],
        },
        start_pos: start,
        start_facing: map.start_facing,
    }
}
