//! Combat math (plan6). Pure functions only — every gameplay system calls these,
//! and the unit tests below pin the boundary cases. All formulas are
//! **provisional** (project.md「計算システム」); update plan6.md if they change.

/// Hit chance as a percent, clamped to [10, 95].
///
/// `50 + (attacker agility − defender agility)/10 + grip/10`. Grip is the held
/// item's 持ちやすさ (50 when unarmed).
pub fn hit_chance(attacker_agility: i32, defender_agility: i32, grip: i32) -> i32 {
    (50 + (attacker_agility - defender_agility) / 10 + grip / 10).clamp(10, 95)
}

/// Base damage before the concentration bonus: `max(1, sharpness + attack/10 −
/// defense/20)`. Sharpness is the weapon's するどさ (0 when unarmed).
pub fn base_damage(sharpness: i32, attack: i32, defense: i32) -> i32 {
    (sharpness + attack / 10 - defense / 20).max(1)
}

/// Final damage: base plus a concentration bonus of up to +50% at full focus.
/// The attacker's concentration is spent (set to 0) by the caller afterwards.
pub fn final_damage(
    sharpness: i32,
    attack: i32,
    defense: i32,
    current_concentration: i32,
    max_concentration: i32,
) -> i32 {
    let base = base_damage(sharpness, attack, defense);
    let ratio = current_concentration as f32 / max_concentration.max(1) as f32;
    let bonus = base as f32 * ratio.clamp(0.0, 1.0) * 0.5;
    (base as f32 + bonus).round() as i32
}

/// Halve incoming damage while guarding (rounding down, but never below 1).
pub fn guarded(damage: i32, guarding: bool) -> i32 {
    if guarding { (damage / 2).max(1) } else { damage }
}

/// Unarmed sharpness / grip constants (project.md: 素手 = するどさ0・持ちやすさ50).
pub const UNARMED_SHARPNESS: i32 = 0;
pub const UNARMED_GRIP: i32 = 50;

/// Thrown-item damage basis: `sharpness + throwability/5` (plan6「投げる」).
pub fn throw_damage(sharpness: i32, throwability: i32) -> i32 {
    (sharpness + throwability / 5).max(1)
}

/// Thrown-item range in tiles: `clamp(throwing/20 + throwability/50, 1, 6)`.
pub fn throw_range(throwing: i32, throwability: i32) -> i32 {
    (throwing / 20 + throwability / 50).clamp(1, 6)
}

/// Steal success percent: `clamp(50 + stealing − wariness, 5, 95)`.
pub fn steal_chance(stealing: i32, wariness: i32) -> i32 {
    (50 + stealing - wariness).clamp(5, 95)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_chance_clamps() {
        // Huge agility gap clamps at the ceiling / floor.
        assert_eq!(hit_chance(10_000, 0, 50), 95);
        assert_eq!(hit_chance(0, 10_000, 0), 10);
        // Even match, unarmed grip 50 → 50 + 0 + 5 = 55.
        assert_eq!(hit_chance(20, 20, UNARMED_GRIP), 55);
    }

    #[test]
    fn base_damage_floor_is_one() {
        // Heavy defense can't push damage below 1.
        assert_eq!(base_damage(0, 0, 10_000), 1);
        // sharpness 10 + attack 100/10 (=10) − defense 20/20 (=1) = 19.
        assert_eq!(base_damage(10, 100, 20), 19);
    }

    #[test]
    fn concentration_scales_damage() {
        // Zero concentration → just the base.
        assert_eq!(final_damage(10, 0, 0, 0, 40), 10);
        // Full concentration → base × 1.5 = 15.
        assert_eq!(final_damage(10, 0, 0, 40, 40), 15);
        // max_concentration 0 must not divide by zero (no concentration → base).
        assert_eq!(final_damage(10, 0, 0, 0, 0), 10);
    }

    #[test]
    fn guard_halves() {
        assert_eq!(guarded(12, true), 6);
        assert_eq!(guarded(12, false), 12);
        assert_eq!(guarded(1, true), 1); // never below 1
    }

    #[test]
    fn throw_and_steal_bounds() {
        assert_eq!(throw_range(0, 0), 1);
        assert_eq!(throw_range(10_000, 10_000), 6);
        assert_eq!(steal_chance(10_000, 0), 95);
        assert_eq!(steal_chance(0, 10_000), 5);
    }
}
