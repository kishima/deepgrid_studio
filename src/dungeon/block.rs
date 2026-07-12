use serde::{Deserialize, Serialize};

use super::level::{DoorStates, Facing};

/// A single dungeon block (project.md「ダンジョン構造の仕様」).
///
/// The base attributes (`Wall`/`Empty`/`Water`/`Fire`/`Poison`) plus the plan2
/// terrain pieces: `Ladder`, `Door` and the one-way `Horoscope`. Liquid damage
/// (plan5) and door open/close *state* are kept out of the block itself — terrain
/// is immutable, mutable door state lives in [`DoorStates`].
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Block {
    /// Solid wall. Blocks movement; its top face becomes the floor of the cell
    /// above (see `Level::is_supported`).
    Wall,
    /// Open passage / empty space. Walkable.
    #[default]
    Empty,
    /// Water. Walkable; the damage rule waits for plan5.
    Water,
    /// Fire / lava. Walkable in plan2.
    Fire,
    /// Poison. Walkable in plan2.
    Poison,
    /// Ladder. Walkable, and it *supports* the player (no fall) and lets them
    /// climb between the floor directly above / below.
    Ladder,
    /// Door. `kind` selects which of the level's door groups it belongs to
    /// (0-based, `< LimitsConfig.door_kinds_per_level`). Open/closed state is in
    /// [`DoorStates`], keyed by `kind` — the original opens doors by kind, not
    /// individually.
    Door { kind: u8 },
    /// One-way block. The player may only travel *through* it in the `pass_from`
    /// direction — both entering and leaving are restricted to that heading, so
    /// it acts as a directional valve. (The map glyphs `< > n v` point the way:
    /// West/East/North/South.)
    Horoscope { pass_from: Facing },
}

impl Block {
    /// Is this a solid wall?
    pub fn is_wall(self) -> bool {
        matches!(self, Block::Wall)
    }

    /// Is this a ladder?
    pub fn is_ladder(self) -> bool {
        matches!(self, Block::Ladder)
    }

    /// Can the player step *out* of a cell holding `self` while moving in `dir`?
    ///
    /// Only a `Horoscope` restricts leaving: you can only exit in its `pass_from`
    /// direction (the same way you were allowed in), which is what makes the
    /// block one-way rather than a pocket you can re-enter from the far side.
    pub fn allows_exit(self, dir: Facing) -> bool {
        match self {
            Block::Horoscope { pass_from } => dir == pass_from,
            _ => true,
        }
    }

    /// Can the player step *into* a cell holding `self` while moving in `dir`,
    /// given the current door states?
    pub fn allows_enter(self, dir: Facing, doors: &DoorStates) -> bool {
        match self {
            Block::Wall => false,
            Block::Door { kind } => doors.is_open(kind),
            Block::Horoscope { pass_from } => dir == pass_from,
            _ => true,
        }
    }

    /// Can the player pass *vertically* (fall / climb) through a cell holding
    /// `self`? Vertical movement ignores the horizontal one-way rule; only walls
    /// and shut doors block it.
    pub fn allows_vertical(self, doors: &DoorStates) -> bool {
        match self {
            Block::Wall => false,
            Block::Door { kind } => doors.is_open(kind),
            _ => true,
        }
    }
}
