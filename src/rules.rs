//! Per-project game rules (plan6.5): the `rules` section of `project.ron`.
//!
//! This is the home for DeepGrid's own extensions to the original "Dandan
//! Dungeon" — rules a game author can turn on/off and tune per project. Hunger
//! (satiety) is the first resident. Everything is `#[serde(default)]`, so a
//! pre-plan6.5 project (no `rules` block) loads with hunger disabled and behaves
//! exactly as before.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Default satiety used before rules are known (matches [`HungerRules`] default).
pub const DEFAULT_SATIETY_MAX: i32 = 1000;

/// Hunger / satiety rules. Numbers are data, not hard-coded constants
/// (plan6.5「数値のハードコード禁止」).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HungerRules {
    /// Master switch. Default off for backward compatibility.
    pub enabled: bool,
    /// Full satiety.
    pub satiety_max: i32,
    /// Satiety drops by 1 every this many cycles.
    pub drain_interval_cycles: u64,
    /// HP lost per cycle while starving (satiety 0).
    pub starvation_damage: i32,
    /// Satiety gained per point of an eaten item's nutrition.
    pub satiety_per_nutrition: i32,
    /// Warn below `satiety_max × warn_ratio`.
    pub warn_ratio: f32,
}

impl Default for HungerRules {
    fn default() -> Self {
        Self {
            enabled: false,
            satiety_max: DEFAULT_SATIETY_MAX,
            drain_interval_cycles: 10,
            starvation_damage: 1,
            satiety_per_nutrition: 10,
            warn_ratio: 0.25,
        }
    }
}

impl HungerRules {
    /// Warning threshold in satiety points.
    pub fn warn_threshold(&self) -> i32 {
        (self.satiety_max as f32 * self.warn_ratio) as i32
    }
}

/// All per-project rules (`project.ron` `rules`). One resident so far.
#[derive(Resource, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct RulesConfig {
    pub hunger: HungerRules,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let r = RulesConfig::default();
        assert!(!r.hunger.enabled);
        assert_eq!(r.hunger.satiety_max, DEFAULT_SATIETY_MAX);
    }

    #[test]
    fn absent_fields_fall_back_to_default() {
        // An empty rules block, and a partial hunger block, both fill in.
        let full: RulesConfig = ron::from_str("(hunger: (enabled: true))").unwrap();
        assert!(full.hunger.enabled);
        assert_eq!(full.hunger.satiety_max, DEFAULT_SATIETY_MAX);

        let empty: RulesConfig = ron::from_str("()").unwrap();
        assert_eq!(empty, RulesConfig::default());
    }

    #[test]
    fn round_trip() {
        let r = RulesConfig {
            hunger: HungerRules {
                enabled: true,
                satiety_max: 500,
                drain_interval_cycles: 5,
                starvation_damage: 2,
                satiety_per_nutrition: 8,
                warn_ratio: 0.3,
            },
        };
        let ron = ron::ser::to_string(&r).unwrap();
        let back: RulesConfig = ron::from_str(&ron).unwrap();
        assert_eq!(r, back);
        assert_eq!(r.hunger.warn_threshold(), 150);
    }
}
