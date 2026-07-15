//! The 3D-edit-mode walker (plan9.5). A deliberately light re-implementation of
//! first-person grid movement — it does **not** reuse the play-mode `movement.rs`
//! (which is tangled with party/doors/data-screen). Only `Wall` blocks a step;
//! everything else (doors, liquids, holes, unsupported cells) is passable and
//! there is no falling, so an author can walk freely to reach any cell.

use bevy::prelude::*;

use crate::dungeon::level::Level;
use crate::dungeon::{Facing, GridPos};
use crate::player::movement::EYE_HEIGHT;
use crate::render::BLOCK_SIZE;

/// One grid intent from the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WalkAction {
    Forward,
    Backward,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
}

/// The editor walker's logical state (tile + heading). The camera smoothly
/// follows this; edits target [`EditWalk::front`].
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditWalk {
    pub pos: GridPos,
    pub facing: Facing,
}

impl EditWalk {
    pub fn new(pos: GridPos, facing: Facing) -> Self {
        Self { pos, facing }
    }

    /// The tile directly ahead (the target of place/erase).
    pub fn front(&self) -> GridPos {
        let (dx, dy) = self.facing.delta();
        GridPos::new(self.pos.x + dx, self.pos.y + dy, self.pos.floor)
    }

    /// Can the walker stand on `pos`? Only a `Wall` (or out-of-bounds) blocks it.
    pub fn passable(level: &Level, pos: GridPos) -> bool {
        match level.block_at(pos) {
            Some(b) => !b.is_wall(),
            None => false,
        }
    }

    /// The heading a move action produces from the current facing, or `None` for
    /// a turn.
    fn move_dir(action: WalkAction, facing: Facing) -> Option<Facing> {
        match action {
            WalkAction::Forward => Some(facing),
            WalkAction::Backward => Some(facing.opposite()),
            WalkAction::StrafeLeft => Some(facing.turn_left()),
            WalkAction::StrafeRight => Some(facing.turn_right()),
            WalkAction::TurnLeft | WalkAction::TurnRight => None,
        }
    }

    /// Apply `action`. Turns rotate in place; moves succeed only into a passable
    /// tile (facing is unchanged by a move). Returns whether state changed.
    pub fn step(&mut self, action: WalkAction, level: &Level) -> bool {
        match Self::move_dir(action, self.facing) {
            None => {
                self.facing = match action {
                    WalkAction::TurnLeft => self.facing.turn_left(),
                    WalkAction::TurnRight => self.facing.turn_right(),
                    _ => self.facing,
                };
                true
            }
            Some(dir) => {
                let (dx, dy) = dir.delta();
                let dest = GridPos::new(self.pos.x + dx, self.pos.y + dy, self.pos.floor);
                if Self::passable(level, dest) {
                    self.pos = dest;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Move up/down a floor (no ladder needed), clamped to the level's floor
    /// range and blocked when the destination cell is a wall.
    pub fn climb(&mut self, up: bool, level: &Level) -> bool {
        let floor_count = level.floor_count();
        let target = if up {
            (self.pos.floor + 1).min(floor_count.saturating_sub(1))
        } else {
            self.pos.floor.saturating_sub(1)
        };
        let dest = GridPos::new(self.pos.x, self.pos.y, target);
        if target != self.pos.floor && Self::passable(level, dest) {
            self.pos.floor = target;
            true
        } else {
            false
        }
    }

    /// The canonical eye position + yaw the camera targets.
    pub fn camera_target(&self) -> (Vec3, f32) {
        let eye = Vec3::new(
            self.pos.x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
            self.pos.floor as f32 * BLOCK_SIZE + EYE_HEIGHT,
            self.pos.y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        );
        (eye, self.facing.yaw())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::Block;
    use crate::dungeon::level::{Floor, Level};

    fn level_3x3(center: Block) -> Level {
        // 3×3 floor, all Empty except the center cell = `center`.
        let mut blocks = vec![Block::Empty; 9];
        blocks[4] = center; // (1,1)
        Level { floors: vec![Floor { width: 3, height: 3, blocks }] }
    }

    #[test]
    fn wall_blocks_but_door_and_hole_pass() {
        let mut w = EditWalk::new(GridPos::new(1, 0, 0), Facing::South); // face (1,1)
        let wall = level_3x3(Block::Wall);
        assert!(!w.step(WalkAction::Forward, &wall)); // wall ahead → blocked
        assert_eq!(w.pos, GridPos::new(1, 0, 0));

        for passable in [Block::Door { kind: 0 }, Block::Hole, Block::Water, Block::WritableWall] {
            let mut w2 = EditWalk::new(GridPos::new(1, 0, 0), Facing::South);
            let lvl = level_3x3(passable);
            assert!(w2.step(WalkAction::Forward, &lvl), "{passable:?} should be passable");
            assert_eq!(w2.pos, GridPos::new(1, 1, 0));
        }
    }

    #[test]
    fn turn_changes_facing_not_position() {
        let lvl = level_3x3(Block::Empty);
        let mut w = EditWalk::new(GridPos::new(1, 1, 0), Facing::North);
        assert!(w.step(WalkAction::TurnRight, &lvl));
        assert_eq!(w.facing, Facing::East);
        assert_eq!(w.pos, GridPos::new(1, 1, 0));
    }

    #[test]
    fn front_cell_follows_facing() {
        let w = EditWalk::new(GridPos::new(2, 2, 1), Facing::East);
        assert_eq!(w.front(), GridPos::new(3, 2, 1));
    }

    #[test]
    fn climb_clamps_to_floor_range_and_walls_block() {
        // 1×1 tower: empty, empty, wall.
        let f = |b| Floor { width: 1, height: 1, blocks: vec![b] };
        let lvl = Level { floors: vec![f(Block::Empty), f(Block::Empty), f(Block::Wall)] };
        let mut w = EditWalk::new(GridPos::new(0, 0, 0), Facing::North);
        assert!(!w.climb(false, &lvl)); // already at floor 0
        assert!(w.climb(true, &lvl));
        assert_eq!(w.pos.floor, 1);
        assert!(!w.climb(true, &lvl)); // wall in the cell above → blocked
        assert_eq!(w.pos.floor, 1);

        let open = Level { floors: vec![f(Block::Empty), f(Block::Empty)] };
        let mut top = EditWalk::new(GridPos::new(0, 0, 1), Facing::North);
        assert!(!top.climb(true, &open)); // clamp at the top floor
        assert_eq!(top.pos.floor, 1);
    }

    #[test]
    fn out_of_bounds_is_not_passable() {
        let lvl = level_3x3(Block::Empty);
        let mut w = EditWalk::new(GridPos::new(0, 0, 0), Facing::West); // off the map
        assert!(!w.step(WalkAction::Forward, &lvl));
    }
}
