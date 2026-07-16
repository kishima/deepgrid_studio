//! A tiny deterministic RNG resource (plan6). One source of randomness for
//! combat hit rolls and monster wandering, so runs are reproducible — the
//! autotest suite relies on this (no flaky tests), and it's a step toward
//! deterministic saves (plan10). In-crate xorshift64* keeps the dependency
//! footprint at zero.

use bevy::prelude::Resource;

/// Fixed seed. Deterministic play is fine for now; a per-save seed arrives with
/// saving in plan10.
const SEED: u64 = 0x9E3779B97F4A7C15;

#[derive(Resource)]
pub struct GameRng {
    state: u64,
}

impl Default for GameRng {
    fn default() -> Self {
        Self { state: SEED }
    }
}

impl GameRng {
    /// The raw generator state (plan10 save target). Saving and restoring it
    /// makes "load → same inputs → same outcomes" hold exactly.
    pub fn state(&self) -> u64 {
        self.state
    }

    pub fn set_state(&mut self, state: u64) {
        // Guard against a zero state (xorshift's absorbing point).
        self.state = if state == 0 { SEED } else { state };
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform integer in `0..n` (returns 0 if `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }

    /// True with probability `percent`/100 (clamped).
    pub fn chance(&mut self, percent: i32) -> bool {
        let p = percent.clamp(0, 100) as u64;
        self.next_u64() % 100 < p
    }

    /// A coin flip.
    pub fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        let mut a = GameRng::default();
        let mut b = GameRng::default();
        for _ in 0..1000 {
            let (va, vb) = (a.below(7), b.below(7));
            assert_eq!(va, vb);
            assert!(va < 7);
        }
        // 0% never, 100% always.
        assert!(!a.chance(0));
        assert!(a.chance(100));
    }
}
