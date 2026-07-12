use std::path::Path;

use serde::{Deserialize, Serialize};

use super::block::Block;
use super::level::{Dungeon, Facing, Floor, GridPos, Level};
use crate::config::LimitsConfig;

/// One floor of the on-disk map: `height` rows of `width` characters.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct FloorRows {
    rows: Vec<String>,
}

/// On-disk map format v2 (RON). A level is a stack of floors, `floors[0]` being
/// the lowest. Each floor is rows of characters so maps stay hand-editable.
///
/// Character legend: `#`=Wall, `.`=Empty, `~`=Water, `^`=Fire, `%`=Poison,
/// `H`=Ladder, `1`/`2`=Door(kind 0/1), and the one-way Horoscope arrows
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
        '<' => Block::Horoscope {
            pass_from: Facing::West,
        },
        '>' => Block::Horoscope {
            pass_from: Facing::East,
        },
        'n' => Block::Horoscope {
            pass_from: Facing::North,
        },
        'v' => Block::Horoscope {
            pass_from: Facing::South,
        },
        _ => return None,
    })
}

/// Parse one floor's rows into a `Floor`, validating dimensions and block chars
/// against `map`/`limits`. `fi` is the floor index (for error messages).
fn parse_floor(map: &MapFile, fi: usize, floor: &FloorRows, limits: &LimitsConfig, path: &Path) -> Floor {
    assert_eq!(
        floor.rows.len(),
        map.height,
        "map {} floor {fi} declares height {} but has {} rows",
        path.display(),
        map.height,
        floor.rows.len(),
    );

    let mut blocks = Vec::with_capacity(map.width * map.height);
    for (y, row) in floor.rows.iter().enumerate() {
        let cols: Vec<char> = row.chars().collect();
        assert_eq!(
            cols.len(),
            map.width,
            "map {} floor {fi} row {y} has {} columns, expected {}",
            path.display(),
            cols.len(),
            map.width,
        );
        for (x, &c) in cols.iter().enumerate() {
            let block = char_to_block(c).unwrap_or_else(|| {
                panic!(
                    "map {} floor {fi} row {y} col {x}: unknown block '{c}'",
                    path.display()
                )
            });
            if let Block::Door { kind } = block {
                assert!(
                    (kind as usize) < limits.door_kinds_per_level,
                    "map {} floor {fi} row {y} col {x}: door kind {kind} exceeds \
                     configured door_kinds_per_level {}",
                    path.display(),
                    limits.door_kinds_per_level,
                );
            }
            blocks.push(block);
        }
    }

    Floor {
        width: map.width,
        height: map.height,
        blocks,
    }
}

/// Load a multi-floor level from a v2 RON map file.
///
/// plan2 has no error UI: any problem (missing file, malformed RON, wrong
/// dimensions, unknown block char, too many floors, an unsupported / walled-in
/// start) is a `panic!`. The map is validated against `limits` so oversized or
/// over-tall maps fail loudly rather than silently exceeding the config.
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
    assert!(
        !map.floors.is_empty() && map.floors.len() <= limits.floors_per_level,
        "map {} has {} floors, must be 1..={}",
        path.display(),
        map.floors.len(),
        limits.floors_per_level,
    );

    let floors: Vec<Floor> = map
        .floors
        .iter()
        .enumerate()
        .map(|(fi, f)| parse_floor(&map, fi, f, limits, path))
        .collect();
    let level = Level { floors };

    assert!(
        map.start_floor < level.floor_count(),
        "map {} start_floor {} is out of range (0..{})",
        path.display(),
        map.start_floor,
        level.floor_count(),
    );
    let start = GridPos::new(map.start_x, map.start_y, map.start_floor);
    let start_block = level.block_at(start).unwrap_or_else(|| {
        panic!(
            "map {} start ({},{},floor {}) is out of bounds",
            path.display(),
            start.x,
            start.y,
            start.floor,
        )
    });
    assert!(
        !start_block.is_wall(),
        "map {} start ({},{},floor {}) is inside a wall",
        path.display(),
        start.x,
        start.y,
        start.floor,
    );
    assert!(
        level.is_supported(start.x, start.y, start.floor),
        "map {} start ({},{},floor {}) has no footing (would fall)",
        path.display(),
        start.x,
        start.y,
        start.floor,
    );

    Dungeon {
        level,
        start_pos: start,
        start_facing: map.start_facing,
    }
}
