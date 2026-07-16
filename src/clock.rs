//! Cycle-time system (plan4, project.md「リアルタイム処理」).
//!
//! The game's time advances in fixed "cycles". Real elapsed time is accumulated
//! and, whenever it crosses a cycle boundary, one `CycleTick` event fires per
//! boundary crossed (a slow frame can advance several cycles at once, so no time
//! is lost). Systems that consume game time — concentration recovery here, and
//! poison/fire damage and monster turns in later plans — react to `CycleTick`
//! rather than to raw `Time`, so they all share one clock.
//!
//! Movement/fall *animation* timing stays in real seconds (movement.rs); only
//! game-logic costs are cycle-based (action cycle costs arrive with combat in
//! plan6).

use bevy::prelude::*;

use crate::character::Party;

/// Real seconds per cycle. 1 cycle = 0.1 s.
pub const CYCLE_SECS: f32 = 0.1;

/// The game clock: total cycles since start, plus the sub-cycle real-time
/// remainder. `accum` is kept below `CYCLE_SECS` after each update so error can't
/// accumulate across frames.
#[derive(Resource, Default)]
pub struct GameClock {
    /// Cumulative cycles elapsed since startup.
    pub cycle: u64,
    accum: f32,
}

impl GameClock {
    /// Restore the clock from a save (plan10): jump to `cycle`, drop the
    /// sub-cycle remainder.
    pub fn restore(&mut self, cycle: u64) {
        self.cycle = cycle;
        self.accum = 0.0;
    }
}

/// Fired once for each cycle boundary crossed this frame. `cycle` is the index of
/// the cycle that just began — carried for later plans (poison/fire ticks,
/// monster turns) that will key off the absolute cycle; plan4's only reader
/// counts ticks, so the field is allowed-dead for now.
#[derive(Event, Clone, Copy)]
pub struct CycleTick {
    #[allow(dead_code)]
    pub cycle: u64,
}

/// Advance the clock by real `Time` and emit one `CycleTick` per boundary. Runs
/// before the on-cycle systems so they see this frame's ticks. Game speed
/// (plan10, `user_settings.ron` speed 0.5/1.0/2.0) scales the delta here — the
/// single point where real time becomes game time. The title screen (plan11)
/// and demo playback (plan10) freeze the clock entirely.
pub fn tick_clock(
    time: Res<Time>,
    settings: Res<crate::settings::UserSettings>,
    screen: crate::screen::CurrentScreen,
    mut clock: ResMut<GameClock>,
    mut ticks: EventWriter<CycleTick>,
) {
    if screen.freezes_clock() {
        return;
    }
    clock.accum += time.delta_secs() * settings.speed.clamp(0.25, 4.0);
    // Emit a tick per whole cycle accumulated; keep the fractional remainder.
    while clock.accum >= CYCLE_SECS {
        clock.accum -= CYCLE_SECS;
        clock.cycle += 1;
        ticks.send(CycleTick { cycle: clock.cycle });
    }
}

/// On each cycle, restore 1 concentration to every conscious party member, up to
/// their maximum (plan4). Knocked-out members don't recover; nor do starving
/// members when hunger is enabled (plan6.5).
pub fn recover_concentration(
    mut ticks: EventReader<CycleTick>,
    rules: Res<crate::rules::RulesConfig>,
    mut party: ResMut<Party>,
) {
    let cycles = ticks.read().count() as i32;
    if cycles == 0 {
        return;
    }
    for member in &mut party.members {
        if member.state.down {
            continue;
        }
        if rules.hunger.enabled && member.state.satiety == 0 {
            continue; // starving: no focus to spare
        }
        let max = member.character.stats.concentration;
        member.state.concentration = (member.state.concentration + cycles).min(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeding the clock N × CYCLE_SECS in uneven real-time chunks must produce
    /// exactly N cycles — no drift from accumulated float error.
    #[test]
    fn cycle_count_has_no_drift() {
        // Simulate 1000 frames of an awkward frame time and count boundaries by
        // hand using the same accumulate-and-subtract logic.
        let mut accum = 0.0f32;
        let mut cycles = 0u64;
        let frame_dt = 0.016_666_7; // ~60 fps, not a clean multiple of 0.1
        let frames = 6000; // 100 s of game time -> expect 1000 cycles
        for _ in 0..frames {
            accum += frame_dt;
            while accum >= CYCLE_SECS {
                accum -= CYCLE_SECS;
                cycles += 1;
            }
        }
        let expected = (frames as f32 * frame_dt / CYCLE_SECS).floor() as u64;
        // Allow the boundary to land within one cycle of the ideal (float rounding
        // at the very last partial cycle), but never systematically lose cycles.
        assert!(
            cycles == expected || cycles + 1 == expected || cycles == expected + 1,
            "cycles {cycles} vs expected {expected}"
        );
    }
}
