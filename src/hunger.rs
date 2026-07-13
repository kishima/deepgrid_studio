//! Hunger / satiety (plan6.5) — a DeepGrid extension layered on top of the
//! original "栄養価 = HP回復" rule without changing it. Cycle-driven, like the
//! liquid hazards, and fully governed by [`RulesConfig`] so a game author can
//! disable or retune it per project. When disabled it is a complete no-op.

use bevy::prelude::*;

use crate::character::Party;
use crate::clock::{CycleTick, GameClock};
use crate::hud::MessageLog;
use crate::rules::RulesConfig;

/// Minimum cycles between hunger warning lines (throttling, like hazards).
const WARN_INTERVAL_CYCLES: u64 = 40;

/// Drain satiety over time and apply starvation damage. `drain_accum` carries the
/// sub-interval remainder so the drain rate is exact regardless of frame pacing.
pub fn hunger_tick(
    mut ticks: EventReader<CycleTick>,
    mut drain_accum: Local<u64>,
    mut last_warn: Local<u64>,
    rules: Res<RulesConfig>,
    clock: Res<GameClock>,
    mut party: ResMut<Party>,
    mut log: ResMut<MessageLog>,
) {
    let h = &rules.hunger;
    let cycles = ticks.read().count() as u64;
    if !h.enabled || cycles == 0 {
        return;
    }

    // Drain 1 satiety per full interval elapsed (drains while resting / downed).
    let interval = h.drain_interval_cycles.max(1);
    *drain_accum += cycles;
    while *drain_accum >= interval {
        *drain_accum -= interval;
        for m in &mut party.members {
            m.state.satiety = (m.state.satiety - 1).max(0);
        }
    }

    // Starvation damage + warning state.
    let mut starving = false;
    let mut hungry = false;
    let warn = h.warn_threshold();
    for m in &mut party.members {
        if m.state.satiety == 0 {
            starving = true;
            m.state.hp = (m.state.hp - h.starvation_damage * cycles as i32).max(0);
            if m.state.hp == 0 {
                m.state.down = true;
            }
        } else if m.state.satiety < warn {
            hungry = true;
        }
    }

    if (starving || hungry) && clock.cycle.saturating_sub(*last_warn) >= WARN_INTERVAL_CYCLES {
        *last_warn = clock.cycle;
        log.push(if starving {
            "うえじにしそうだ!"
        } else {
            "おなかがすいた…"
        });
    }
}
