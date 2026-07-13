//! Environmental hazards (plan5): liquid tiles now actually hurt. Cycle-driven
//! via `CycleTick`, keyed on the party's tile and each member's resistances. HP
//! floors at 0 and a downed member is marked 気絶, exactly like fall damage.
//!
//! Thresholds and amounts are **provisional** (the original's exact rules are
//! unknown); they're constants here and this file / plan5.md must be updated if
//! they change.

use bevy::prelude::*;

use crate::character::{Party, StatKind};
use crate::clock::{CycleTick, GameClock};
use crate::dungeon::{Block, Dungeon};
use crate::hud::MessageLog;
use crate::item::ItemCatalog;
use crate::player::Player;

/// Below this lung capacity, water drowns you (1 dmg/cycle). — 暫定
/// (pub: the autotest driver derives which members it expects to be hurt.)
pub const WATER_LUNG_THRESHOLD: i32 = 100;
const WATER_DAMAGE: i32 = 1;
/// Below this heat resistance, fire burns you (3 dmg/cycle). — 暫定
const FIRE_HEAT_THRESHOLD: i32 = 100;
const FIRE_DAMAGE: i32 = 3;
/// Below this poison resistance, poison afflicts you. — 暫定 (pub: 同上)
pub const POISON_RESIST_THRESHOLD: i32 = 100;
const POISON_DAMAGE: i32 = 1;
/// Poison lingers this many cycles after leaving the tile. — 暫定
const POISON_LINGER_CYCLES: u32 = 32;

/// Don't spam the log with a hazard line more often than this (cycles).
const HAZARD_MSG_INTERVAL: u64 = 8;

/// Apply liquid + lingering-poison damage each cycle to the whole party.
#[allow(clippy::too_many_arguments)]
pub fn hazard_tick(
    mut ticks: EventReader<CycleTick>,
    mut last_msg: Local<u64>,
    player: Res<Player>,
    dungeon: Res<Dungeon>,
    clock: Res<GameClock>,
    catalog: Res<ItemCatalog>,
    mut party: ResMut<Party>,
    mut log: ResMut<MessageLog>,
) {
    let cycles = ticks.read().count() as u32;
    if cycles == 0 {
        return;
    }
    let n = cycles as i32;
    let block = dungeon.level.block_at(player.pos);

    let mut hurt_water = false;
    let mut hurt_fire = false;
    let mut hurt_poison = false;

    for member in &mut party.members {
        let eff = member.effective_stats(&catalog);
        // Tile hazard for this member (resistance-gated).
        match block {
            Some(Block::Water) if eff.get(StatKind::LungCapacity) < WATER_LUNG_THRESHOLD => {
                member.state.hp -= WATER_DAMAGE * n;
                hurt_water = true;
            }
            Some(Block::Fire) if eff.get(StatKind::HeatResist) < FIRE_HEAT_THRESHOLD => {
                member.state.hp -= FIRE_DAMAGE * n;
                hurt_fire = true;
            }
            Some(Block::Poison) if eff.get(StatKind::PoisonResist) < POISON_RESIST_THRESHOLD => {
                // Refresh the lingering timer; the damage is applied below so
                // standing in poison and walking out both use one code path.
                member.state.poison_remaining = POISON_LINGER_CYCLES;
            }
            _ => {}
        }
        // Lingering poison (independent of the current tile).
        if member.state.poison_remaining > 0 {
            let applied = cycles.min(member.state.poison_remaining);
            member.state.hp -= POISON_DAMAGE * applied as i32;
            member.state.poison_remaining -= applied;
            hurt_poison = true;
        }

        if member.state.hp <= 0 {
            member.state.hp = 0;
            member.state.down = true;
        }
    }

    // One throttled line so the log isn't flooded at 10 cycles/second.
    if (hurt_water || hurt_fire || hurt_poison)
        && clock.cycle.saturating_sub(*last_msg) >= HAZARD_MSG_INTERVAL
    {
        *last_msg = clock.cycle;
        let line = if hurt_fire {
            "燃えている! ダメージを受けた"
        } else if hurt_water {
            "水に浸かっている! ダメージを受けた"
        } else {
            "毒がまわっている! ダメージを受けた"
        };
        log.push(line);
    }
}
