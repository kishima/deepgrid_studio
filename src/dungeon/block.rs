use serde::{Deserialize, Serialize};

/// A single dungeon block (project.md「ダンジョン構造の仕様」— 基本属性).
///
/// plan1 only distinguishes `Wall` (blocks movement, drawn) from everything
/// else (walkable, empty space). The liquid variants exist in the data model so
/// test maps and later plans can reference them, but their damage rules and
/// distinct rendering are deferred to plan2+.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Block {
    /// Solid wall. Blocks movement; rendered as a cube.
    Wall,
    /// Open passage / empty space. Walkable.
    #[default]
    Empty,
    /// Water. Walkable in plan1 (damage rules come later).
    Water,
    /// Fire / lava. Walkable in plan1.
    Fire,
    /// Poison. Walkable in plan1.
    Poison,
}

impl Block {
    /// Whether the player can enter a tile of this block.
    ///
    /// plan1 rule: only `Wall` blocks movement; every other block is passable.
    pub fn is_walkable(self) -> bool {
        !matches!(self, Block::Wall)
    }
}
