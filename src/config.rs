use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Original-faithful pouch capacity (dandan_spec: 3).
fn default_pouch_size() -> usize {
    3
}
/// Original-faithful backpack capacity (dandan_spec: 24).
fn default_backpack_size() -> usize {
    24
}
/// Demo count guideline: OP + ED + 4 mid-game demos (plan10).
fn default_max_demos() -> usize {
    6
}

/// Quantity limits for a project (the game being authored). Defaults match the
/// original "Dandan Dungeon" (project.md「上限値の扱い」), but every value is
/// mutable — nothing here is a hard-coded structural constraint.
///
/// plan1 only actually varies `floor_width` / `floor_height`, but the full set
/// of limits is defined now so later plans (editor, monsters, items, …) already
/// have a place to read them from. Code must reach these values through the
/// `LimitsConfig` resource rather than hard-coding constants.
#[derive(Resource, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LimitsConfig {
    /// Number of levels in a dungeon.
    pub max_levels: usize,
    /// Stacked floors per level (bottom to top).
    pub floors_per_level: usize,
    /// Floor width in blocks.
    pub floor_width: usize,
    /// Floor height (depth) in blocks.
    pub floor_height: usize,
    /// Door kinds definable per level ("door 1" / "door 2").
    pub door_kinds_per_level: usize,
    /// Registerable characters.
    pub max_characters: usize,
    /// Party size.
    pub party_size: usize,
    /// Pouch slots per character (plan5). Defaulted so pre-plan5 projects load.
    #[serde(default = "default_pouch_size")]
    pub pouch_size: usize,
    /// Backpack slots per character (plan5).
    #[serde(default = "default_backpack_size")]
    pub backpack_size: usize,
    /// Item kinds definable.
    pub max_item_kinds: usize,
    /// Item placements per level.
    pub item_placements_per_level: usize,
    /// Monster kinds definable.
    pub max_monster_kinds: usize,
    /// Monster kinds appearing per level.
    pub monster_kinds_per_level: usize,
    /// Monster placements per level.
    pub monster_placements_per_level: usize,
    /// Magic kinds definable.
    pub max_magic_kinds: usize,
    /// Event flags available.
    pub event_flags: usize,
    /// Maximum event execution delay in cycles (0..=max_event_delay).
    pub max_event_delay: usize,
    /// Maximum message lines in a demo.
    pub demo_message_lines: usize,
    /// Maximum demos per project (plan10). Defaulted so v6 projects load.
    #[serde(default = "default_max_demos")]
    pub max_demos: usize,
}

impl Default for LimitsConfig {
    /// Original-faithful defaults (project.md table).
    fn default() -> Self {
        Self {
            max_levels: 14,
            floors_per_level: 5,
            floor_width: 40,
            floor_height: 40,
            door_kinds_per_level: 2,
            max_characters: 20,
            party_size: 4,
            pouch_size: 3,
            backpack_size: 24,
            max_item_kinds: 199,
            item_placements_per_level: 1023,
            max_monster_kinds: 56,
            monster_kinds_per_level: 4,
            monster_placements_per_level: 48,
            max_magic_kinds: 80,
            event_flags: 64,
            max_event_delay: 63,
            demo_message_lines: 160,
            max_demos: 6,
        }
    }
}
