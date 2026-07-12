use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use super::block::Block;

/// One floor of a level: a `width` × `height` grid of blocks stored row-major.
///
/// The grid is a plain `Vec<Block>` (not a fixed-size array) so floor sizes stay
/// data-driven — the dimensions come from `LimitsConfig` / the map file, never a
/// compile-time constant.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Floor {
    pub width: usize,
    pub height: usize,
    /// `width * height` blocks, row-major: index = `y * width + x`.
    pub blocks: Vec<Block>,
}

impl Floor {
    /// Block at `(x, y)`, or `None` if out of bounds.
    pub fn get(&self, x: i32, y: i32) -> Option<Block> {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        self.blocks
            .get(y as usize * self.width + x as usize)
            .copied()
    }
}

/// A level: floors stacked bottom (index 0) to top.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Level {
    pub floors: Vec<Floor>,
}

impl Level {
    pub fn floor(&self, index: usize) -> Option<&Floor> {
        self.floors.get(index)
    }

    /// Number of stacked floors.
    pub fn floor_count(&self) -> usize {
        self.floors.len()
    }

    /// Block at a grid position, or `None` if the floor / cell is out of bounds.
    pub fn block_at(&self, pos: GridPos) -> Option<Block> {
        self.floor(pos.floor).and_then(|f| f.get(pos.x, pos.y))
    }

    /// Whether the player can stand on cell `(x, y)` of `floor` without falling
    /// (plan2「足場の判定」): a ladder always supports; otherwise the lowest floor
    /// rests on bedrock, and higher floors need a `Wall` directly below (a wall's
    /// top face is the floor above it).
    pub fn is_supported(&self, x: i32, y: i32, floor: usize) -> bool {
        if self.floor(floor).and_then(|f| f.get(x, y)) == Some(Block::Ladder) {
            return true;
        }
        if floor == 0 {
            return true;
        }
        self.floor(floor - 1).and_then(|f| f.get(x, y)) == Some(Block::Wall)
    }

    /// The floor the player comes to rest on after entering an unsupported cell
    /// at `(x, y, floor)` — they drop one floor at a time until supported. Floor
    /// 0 always supports, so this terminates.
    pub fn landing_floor(&self, x: i32, y: i32, floor: usize) -> usize {
        let mut f = floor;
        while f > 0 && !self.is_supported(x, y, f) {
            f -= 1;
        }
        f
    }
}

/// Open/closed state of each door *kind* in the current level, keyed by kind
/// index. The original "Dandan Dungeon" opens doors by kind ("door 1" / "door
/// 2"), not individually, so a single flag per kind is the whole model. Terrain
/// (the `Block::Door` cells) stays immutable; only this resource changes.
#[derive(Resource, Clone, Debug, Default)]
pub struct DoorStates {
    /// `open[kind]` — doors start closed.
    open: Vec<bool>,
}

impl DoorStates {
    /// All-closed state for `kinds` door kinds.
    pub fn new(kinds: usize) -> Self {
        Self {
            open: vec![false; kinds],
        }
    }

    /// Is the given door kind currently open? Unknown kinds read as closed.
    pub fn is_open(&self, kind: u8) -> bool {
        self.open.get(kind as usize).copied().unwrap_or(false)
    }

    /// Flip the open/closed state of a door kind (no-op for unknown kinds).
    pub fn toggle(&mut self, kind: u8) {
        if let Some(state) = self.open.get_mut(kind as usize) {
            *state = !*state;
        }
    }
}

/// A logical grid position: tile `(x, y)` on `floor`.
///
/// plan1 keeps movement within a single floor, but `floor` is carried so the
/// ladder / fall mechanics of later plans slot in without reshaping this type.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
    pub floor: usize,
}

impl GridPos {
    pub fn new(x: i32, y: i32, floor: usize) -> Self {
        Self { x, y, floor }
    }
}

/// The four cardinal facings. `North` is -Z, `East` is +X (see [`Facing::delta`]).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Facing {
    North,
    East,
    South,
    West,
}

impl Facing {
    /// The `(dx, dy)` grid step for moving one tile forward in this facing.
    /// North decreases `y`; East increases `x`.
    pub fn delta(self) -> (i32, i32) {
        match self {
            Facing::North => (0, -1),
            Facing::East => (1, 0),
            Facing::South => (0, 1),
            Facing::West => (-1, 0),
        }
    }

    /// Rotate 90° clockwise (turn right).
    pub fn turn_right(self) -> Self {
        match self {
            Facing::North => Facing::East,
            Facing::East => Facing::South,
            Facing::South => Facing::West,
            Facing::West => Facing::North,
        }
    }

    /// Rotate 90° counter-clockwise (turn left).
    pub fn turn_left(self) -> Self {
        match self {
            Facing::North => Facing::West,
            Facing::West => Facing::South,
            Facing::South => Facing::East,
            Facing::East => Facing::North,
        }
    }

    /// The reverse heading (180°).
    pub fn opposite(self) -> Self {
        self.turn_right().turn_right()
    }

    /// Yaw angle (radians) about +Y for a camera looking in this facing.
    ///
    /// In Bevy's right-handed Y-up space, "forward" is -Z, which is our North,
    /// so North = 0. East (+X) is a -90° yaw, and so on.
    pub fn yaw(self) -> f32 {
        use std::f32::consts::FRAC_PI_2;
        match self {
            Facing::North => 0.0,
            Facing::East => -FRAC_PI_2,
            Facing::South => std::f32::consts::PI,
            Facing::West => FRAC_PI_2,
        }
    }
}

/// The loaded dungeon plus the player's starting placement, held as a Bevy
/// resource. plan1 loads exactly one level from a single RON file.
#[derive(Resource, Clone, Debug)]
pub struct Dungeon {
    pub level: Level,
    pub start_pos: GridPos,
    pub start_facing: Facing,
}
