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

impl Dungeon {
    /// The floor the player currently stands on.
    pub fn current_floor(&self) -> &Floor {
        self.level
            .floor(self.start_pos.floor)
            .expect("start floor exists")
    }
}
