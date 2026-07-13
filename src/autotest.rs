//! Unattended acceptance tests (`DEEPGRID_AUTOTEST=1`).
//!
//! Automates plan5's formerly-manual checks by driving the *real* play-mode
//! systems — scripted input, the pickup/place events, the inventory API the
//! data screen calls, and the cycle-driven hazard system — then asserting on
//! game state. Prints `[autotest] PASS/FAIL <step>` lines and exits with code
//! 0 (all pass) or 1 (first failure), so it can run headlessly in docker:
//!
//! ```sh
//! DEEPGRID_AUTOTEST=1 ./docker/deepgrid-run.sh
//! ```
//!
//! Test subjects are picked from the loaded project by *property*, not by id
//! (first equippable item with a stat effect, an edible item, the heaviest
//! item, a scanned water/poison tile), so sample-data edits don't silently
//! break the suite — they change what it exercises.

use bevy::app::AppExit;
use bevy::prelude::*;

use crate::character::{Party, StatKind};
use crate::clock::GameClock;
use crate::dungeon::{Block, DoorStates, Dungeon, Facing, GridPos};
use crate::floor_items::{FloorItem, InitialItems, PickupRequest, PlaceRequest};
use crate::hud::MessageLog;
use crate::item::{Inventory, ItemCatalog, ItemInstance, ItemPlacement};
use crate::player::{Action, Command, MoveAnim, Player, ScriptedInput};

/// Whether autotest mode is on (any non-empty `DEEPGRID_AUTOTEST`).
pub fn enabled() -> bool {
    std::env::var("DEEPGRID_AUTOTEST").is_ok_and(|v| !v.is_empty())
}

/// Global timeout for the whole suite, in frames (~3 minutes at 60 fps; the
/// software renderer is slower, but cycle-based waits dominate anyway).
const SUITE_TIMEOUT_FRAMES: u32 = 60 * 180;
/// Per-step timeout for event-pipeline waits, in frames.
const STEP_TIMEOUT_FRAMES: u32 = 600;

#[derive(Resource, Default)]
pub struct AutoTest {
    step: usize,
    frames: u32,
    total_frames: u32,
    fatal: Option<String>,
    // Subjects discovered from the project data (see module docs).
    equip_item: String,
    equip_stat: Option<(StatKind, i32)>,
    heavy_item: String,
    water_tile: Option<GridPos>,
    poison_tile: Option<GridPos>,
    // Scratch shared between a step's phases.
    start_pos: Option<GridPos>,
    baseline: i32,
    phase: u32,
    hp_before: Vec<i32>,
    mark_cycle: u64,
    saved_pos: Option<GridPos>,
    saved_inventory: Option<Inventory>,
    acted: bool,
    // Combat-step scratch (steps 13+).
    monster_ent: Option<Entity>,
    attempts: u32,
}

impl AutoTest {
    fn next_step(&mut self, name: &str) {
        println!("[autotest] PASS {name}");
        self.step += 1;
        self.frames = 0;
        self.phase = 0;
        self.acted = false;
        self.attempts = 0;
        self.monster_ent = None;
        self.hp_before.clear();
        self.saved_pos = None;
    }
}

/// Startup (before `setup_floor_items`): pick the test subjects and inject the
/// pickup target onto the party's starting tile.
pub fn prepare(
    mut t: ResMut<AutoTest>,
    catalog: Res<ItemCatalog>,
    dungeon: Res<Dungeon>,
    mut initial: ResMut<InitialItems>,
) {
    // Deterministic selection: scan defs sorted by id.
    let mut defs: Vec<_> = catalog.iter().collect();
    defs.sort_by(|a, b| a.id.cmp(&b.id));

    for def in &defs {
        if t.equip_item.is_empty()
            && def.is_equippable()
            && let Some(e) = def.effects.first()
        {
            t.equip_item = def.id.clone();
            t.equip_stat = Some((e.stat, e.delta));
        }
    }
    t.heavy_item = defs
        .iter()
        .max_by_key(|d| d.weight)
        .map(|d| d.id.clone())
        .unwrap_or_default();

    let level = &dungeon.level;
    'scan: for f in 0..level.floor_count() {
        for y in 0..level.floor(f).map(|fl| fl.height).unwrap_or(0) {
            for x in 0..level.floor(f).map(|fl| fl.width).unwrap_or(0) {
                let pos = GridPos::new(x as i32, y as i32, f);
                match level.block_at(pos) {
                    Some(Block::Water) if t.water_tile.is_none() => t.water_tile = Some(pos),
                    Some(Block::Poison) if t.poison_tile.is_none() => t.poison_tile = Some(pos),
                    _ => {}
                }
                if t.water_tile.is_some() && t.poison_tile.is_some() {
                    break 'scan;
                }
            }
        }
    }

    if t.equip_item.is_empty() {
        t.fatal = Some("items.ron に効果付き装備アイテムが1つも無い".into());
    } else if t.heavy_item.is_empty() {
        t.fatal = Some("items.ron が空".into());
    } else {
        // The pickup subject appears under the party's feet.
        let s = dungeon.start_pos;
        initial.0.push(ItemPlacement {
            id: t.equip_item.clone(),
            x: s.x,
            y: s.y,
            floor: s.floor,
        });
    }
}

/// The step driver. One big sequential state machine; each arm either waits
/// (returns), fails (prints + exits 1), or passes on to the next step.
#[allow(clippy::too_many_arguments)]
pub fn run(
    mut t: ResMut<AutoTest>,
    mut party: ResMut<Party>,
    catalog: Res<ItemCatalog>,
    mut player: ResMut<Player>,
    dungeon: Res<Dungeon>,
    doors: Res<DoorStates>,
    clock: Res<GameClock>,
    anim: Res<MoveAnim>,
    log: Res<MessageLog>,
    mut script: ResMut<ScriptedInput>,
    mut pickup_ev: EventWriter<PickupRequest>,
    mut place_ev: EventWriter<PlaceRequest>,
    floor_items: Query<&FloorItem>,
    mut exit: EventWriter<AppExit>,
) {
    t.frames += 1;
    t.total_frames += 1;

    let fail = |t: &AutoTest, name: &str, why: String, exit: &mut EventWriter<AppExit>| {
        eprintln!("[autotest] FAIL {name}: {why}");
        eprintln!("[autotest] {} step(s) passed before the failure", t.step);
        exit.send(AppExit::error());
    };

    if let Some(why) = t.fatal.take() {
        fail(&t, "prepare", why, &mut exit);
        return;
    }
    if t.total_frames > SUITE_TIMEOUT_FRAMES {
        fail(&t, "suite", "全体タイムアウト".into(), &mut exit);
        return;
    }

    let item_at = |pos: GridPos, id: &str| {
        floor_items
            .iter()
            .any(|it| it.pos == pos && it.instance.def_id == id)
    };

    match t.step {
        // ---- 0: wait for the world to settle -------------------------------
        0 => {
            if t.frames < 20 || party.is_empty() {
                return;
            }
            t.start_pos = Some(player.pos);
            t.next_step("setup (party ready, subjects selected)");
        }

        // ---- 1: pick the injected item up off the floor ---------------------
        1 => {
            let start = t.start_pos.unwrap();
            if !t.acted {
                pickup_ev.send(PickupRequest);
                t.acted = true;
                return;
            }
            let held = party.members[0]
                .inventory
                .iter()
                .any(|(_, it)| it.def_id == t.equip_item);
            if held && !item_at(start, &t.equip_item) {
                t.next_step("pickup (G) puts the floor item into the inventory");
            } else if t.frames > STEP_TIMEOUT_FRAMES {
                fail(&t, "pickup", format!("拾えていない (held={held})"), &mut exit);
            }
        }

        // ---- 2: equip → the stat effect appears -----------------------------
        2 => {
            let (stat, delta) = t.equip_stat.unwrap();
            let m = &mut party.members[0];
            t.baseline = m.effective_stats(&catalog).get(stat);
            let Some((slot, _)) = m
                .inventory
                .iter()
                .find(|(_, it)| it.def_id == t.equip_item)
                .map(|(s, it)| (s, it.clone()))
            else {
                fail(&t, "equip", "対象アイテムを持っていない".into(), &mut exit);
                return;
            };
            if let Err(e) = m.inventory.equip(slot, &catalog) {
                fail(&t, "equip", e, &mut exit);
                return;
            }
            let after = m.effective_stats(&catalog).get(stat);
            if after == t.baseline + delta {
                t.next_step("equip reflects the stat effect");
            } else {
                fail(
                    &t,
                    "equip",
                    format!("{stat:?}: {} → {after}, 期待 {}", t.baseline, t.baseline + delta),
                    &mut exit,
                );
            }
        }

        // ---- 3: unequip → the effect is gone --------------------------------
        3 => {
            let (stat, _) = t.equip_stat.unwrap();
            let m = &mut party.members[0];
            let slot = catalog
                .get(&t.equip_item)
                .and_then(|d| d.equip_slots.first().copied())
                .expect("subject is equippable");
            if let Err(e) = m.inventory.unequip(slot) {
                fail(&t, "unequip", e, &mut exit);
                return;
            }
            let after = m.effective_stats(&catalog).get(stat);
            if after == t.baseline {
                t.next_step("unequip restores the baseline stat");
            } else {
                fail(&t, "unequip", format!("{stat:?} {after} ≠ {}", t.baseline), &mut exit);
            }
        }

        // ---- 4: place the item back on the floor ----------------------------
        4 => {
            let start = t.start_pos.unwrap();
            if !t.acted {
                let Some((slot, _)) = party.members[0]
                    .inventory
                    .iter()
                    .find(|(_, it)| it.def_id == t.equip_item)
                else {
                    fail(&t, "place", "対象アイテムを持っていない".into(), &mut exit);
                    return;
                };
                place_ev.send(PlaceRequest { slot });
                t.acted = true;
                return;
            }
            let held = party.members[0]
                .inventory
                .iter()
                .any(|(_, it)| it.def_id == t.equip_item);
            if !held && item_at(start, &t.equip_item) {
                t.next_step("place drops the item at the party's feet");
            } else if t.frames > STEP_TIMEOUT_FRAMES {
                fail(&t, "place", "床に戻っていない".into(), &mut exit);
            }
        }

        // ---- 5: eating heals (a member whose teeth are strong enough) -------
        5 => {
            let mut defs: Vec<_> = catalog
                .iter()
                .filter(|d| d.nutrition > 0 && !d.important)
                .collect();
            defs.sort_by(|a, b| a.id.cmp(&b.id));
            let pair = defs.iter().find_map(|d| {
                party
                    .members
                    .iter()
                    .position(|m| m.effective_stats(&catalog).get(StatKind::Bite) >= d.hardness)
                    .map(|i| (i, (*d).clone()))
            });
            let Some((idx, def)) = pair else {
                fail(&t, "eat", "食べられる(栄養価>0)アイテム/キャラの組が無い".into(), &mut exit);
                return;
            };
            let m = &mut party.members[idx];
            m.state.hp = (m.state.hp - def.nutrition.max(5)).max(1);
            let before = m.state.hp;
            match m.eat(&def, &catalog) {
                Ok(_) if m.state.hp > before => {
                    t.next_step("eating heals HP");
                }
                Ok(_) => fail(&t, "eat", format!("HPが回復していない ({before} → {})", m.state.hp), &mut exit),
                Err(e) => fail(&t, "eat", format!("食べられるはずが拒否: {e}"), &mut exit),
            }
        }

        // ---- 6: too hard to bite is refused ---------------------------------
        6 => {
            let mut defs: Vec<_> = catalog.iter().filter(|d| !d.important).collect();
            defs.sort_by(|a, b| a.id.cmp(&b.id));
            let pair = defs.iter().find_map(|d| {
                party
                    .members
                    .iter()
                    .position(|m| m.effective_stats(&catalog).get(StatKind::Bite) < d.hardness)
                    .map(|i| (i, (*d).clone()))
            });
            let Some((idx, def)) = pair else {
                fail(
                    &t,
                    "eat-hard",
                    "歯が立たない組み合わせが無い(サンプルデータに硬い食料か歯の弱いキャラを用意)".into(),
                    &mut exit,
                );
                return;
            };
            match party.members[idx].eat(&def, &catalog) {
                Err(e) if e.contains("かたくて") => t.next_step("too-hard food is refused"),
                Err(e) => fail(&t, "eat-hard", format!("想定外の拒否理由: {e}"), &mut exit),
                Ok(_) => fail(&t, "eat-hard", format!("{}を食べられてしまった", def.name), &mut exit),
            }
        }

        // ---- 7: overweight blocks walking ------------------------------------
        7 => {
            if !t.acted {
                let heavy = t.heavy_item.clone();
                {
                    let m = &mut party.members[0];
                    t.saved_inventory = Some(m.inventory.clone());
                    for _ in 0..500 {
                        if party.overweight_member(&catalog).is_some() {
                            break;
                        }
                        if party.members[0]
                            .inventory
                            .pickup(ItemInstance::new(heavy.clone()))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                if party.overweight_member(&catalog).is_none() {
                    fail(&t, "overweight", "最重量アイテムでも超過にならない(容量不足)".into(), &mut exit);
                    return;
                }
                // Face a direction that is enterable *and* supported, so the
                // post-unload step walks without falling.
                let start = t.start_pos.unwrap();
                let level = &dungeon.level;
                let dir = [Facing::North, Facing::East, Facing::South, Facing::West]
                    .into_iter()
                    .find(|d| {
                        let (dx, dy) = d.delta();
                        let dest = GridPos::new(start.x + dx, start.y + dy, start.floor);
                        level.block_at(start).is_some_and(|b| b.allows_exit(*d))
                            && level.block_at(dest).is_some_and(|b| b.allows_enter(*d, &doors))
                            && level.landing_floor(dest.x, dest.y, dest.floor) == dest.floor
                    });
                let Some(dir) = dir else {
                    fail(&t, "overweight", "落下しない隣接タイルが無い".into(), &mut exit);
                    return;
                };
                player.facing = dir;
                t.saved_pos = Some(player.pos);
                script.queue.push_back(Command::Move(Action::Forward));
                script.active = true;
                t.acted = true;
                return;
            }
            if t.frames > 90 {
                let stayed = player.pos == t.saved_pos.unwrap();
                let warned = log.contains("重すぎる");
                if stayed && warned {
                    t.next_step("overweight blocks movement (+ message)");
                } else {
                    fail(&t, "overweight", format!("stayed={stayed} warned={warned}"), &mut exit);
                }
            }
        }

        // ---- 8: unloading lets the party walk again ---------------------------
        8 => {
            if !t.acted {
                party.members[0].inventory = t.saved_inventory.take().expect("saved in step 7");
                t.saved_pos = Some(player.pos);
                script.queue.push_back(Command::Move(Action::Forward));
                t.acted = true;
                return;
            }
            if t.frames > 40 && anim.is_idle() {
                if player.pos != t.saved_pos.unwrap() {
                    script.active = false;
                    player.pos = t.start_pos.unwrap(); // teleport home for the hazard steps
                    t.next_step("unloading restores movement");
                } else {
                    fail(&t, "unload", "荷物を降ろしても動けない".into(), &mut exit);
                }
            }
        }

        // ---- 9: water hurts the low-lung members while standing in it --------
        9 => {
            let Some(water) = t.water_tile else {
                fail(&t, "water", "マップに水タイルが無い".into(), &mut exit);
                return;
            };
            let vulnerable: Vec<usize> = party
                .members
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    m.effective_stats(&catalog).get(StatKind::LungCapacity)
                        < crate::hazard::WATER_LUNG_THRESHOLD
                })
                .map(|(i, _)| i)
                .collect();
            if vulnerable.is_empty() {
                fail(&t, "water", "肺活量が閾値未満のキャラがいない(サンプル調整が必要)".into(), &mut exit);
                return;
            }
            if !t.acted {
                t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                player.pos = water;
                t.mark_cycle = clock.cycle;
                t.acted = true;
                return;
            }
            if clock.cycle >= t.mark_cycle + 12 {
                let all_hurt = vulnerable
                    .iter()
                    .all(|&i| party.members[i].state.hp < t.hp_before[i]);
                if all_hurt {
                    t.next_step("water damages low-lung members");
                } else {
                    fail(&t, "water", "水中なのにHPが減っていない".into(), &mut exit);
                }
            }
        }

        // ---- 10: leaving the water stops the damage ---------------------------
        10 => {
            match t.phase {
                0 => {
                    player.pos = t.start_pos.unwrap();
                    t.mark_cycle = clock.cycle;
                    t.phase = 1;
                }
                1 => {
                    // A few cycles of settling, then snapshot.
                    if clock.cycle >= t.mark_cycle + 3 {
                        t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                        t.mark_cycle = clock.cycle;
                        t.phase = 2;
                    }
                }
                _ => {
                    if clock.cycle >= t.mark_cycle + 10 {
                        let stable = party
                            .members
                            .iter()
                            .zip(&t.hp_before)
                            .all(|(m, &b)| m.state.hp == b);
                        if stable {
                            t.next_step("water damage stops on dry land");
                        } else {
                            fail(&t, "water-stop", "水から出てもHPが減り続けている".into(), &mut exit);
                        }
                    }
                }
            }
        }

        // ---- 11: poison lingers after leaving the tile ------------------------
        11 => {
            let Some(poison) = t.poison_tile else {
                fail(&t, "poison", "マップに毒タイルが無い".into(), &mut exit);
                return;
            };
            let vulnerable: Vec<usize> = party
                .members
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    m.effective_stats(&catalog).get(StatKind::PoisonResist)
                        < crate::hazard::POISON_RESIST_THRESHOLD
                })
                .map(|(i, _)| i)
                .collect();
            if vulnerable.is_empty() {
                fail(&t, "poison", "耐毒性が閾値未満のキャラがいない(サンプル調整が必要)".into(), &mut exit);
                return;
            }
            match t.phase {
                0 => {
                    player.pos = poison;
                    t.mark_cycle = clock.cycle;
                    t.phase = 1;
                }
                1 => {
                    if clock.cycle >= t.mark_cycle + 5 {
                        // Step out, then verify the damage keeps ticking.
                        player.pos = t.start_pos.unwrap();
                        t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                        t.mark_cycle = clock.cycle;
                        t.phase = 2;
                    }
                }
                _ => {
                    if clock.cycle >= t.mark_cycle + 6 {
                        let lingering = vulnerable.iter().all(|&i| {
                            let m = &party.members[i];
                            m.state.hp < t.hp_before[i] || m.state.hp == 0
                        });
                        if lingering {
                            t.next_step("poison lingers after leaving the pool");
                        } else {
                            fail(&t, "poison", "毒タイルを離れたら即座に止まった(残留していない)".into(), &mut exit);
                        }
                    }
                }
            }
        }

        // ---- 12: ...and eventually wears off ----------------------------------
        12 => {
            match t.phase {
                0 => {
                    let clear = party.members.iter().all(|m| m.state.poison_remaining == 0);
                    if clear {
                        t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                        t.mark_cycle = clock.cycle;
                        t.phase = 1;
                    } else if clock.cycle >= t.mark_cycle + 90 {
                        fail(&t, "poison-end", "残留毒がいつまでも切れない".into(), &mut exit);
                    }
                }
                _ => {
                    if clock.cycle >= t.mark_cycle + 8 {
                        let stable = party
                            .members
                            .iter()
                            .zip(&t.hp_before)
                            .all(|(m, &b)| m.state.hp == b);
                        if stable {
                            // Hand off to the combat steps (run_combat, step ≥13).
                            t.next_step("poison wears off");
                        } else {
                            fail(&t, "poison-end", "毒が切れた後もHPが減っている".into(), &mut exit);
                        }
                    }
                }
            }
        }

        _ => {}
    }
}

// ==================================================================== combat steps

use crate::data_screen::Resting;
use crate::game_state::{DataScreen, SelectedMember};
use crate::monster::{EnemyNear, Monster, MonsterCatalog};
use crate::combat;

/// Spawn a bare (logic-only, no model) monster in front of the party for a
/// combat step. Returns the entity.
fn spawn_subject(commands: &mut Commands, player: &Player, def_id: &str, hp: i32) -> (Entity, GridPos) {
    let (dx, dy) = player.facing.delta();
    let front = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
    let e = commands.spawn(Monster::new_at(def_id, hp, front, player.facing.opposite())).id();
    (e, front)
}

/// Push a scripted player command (used to drive Attack/Guard/Throw/Steal through
/// the real input → PlayerAction → combat pipeline).
fn push_cmd(script: &mut ScriptedInput, cmd: Command) {
    script.queue.push_back(cmd);
    script.active = true;
}

/// Steps 13+ : combat, throw, steal, flee, regen, rest-blocking. Runs only once
/// the pickup/hazard suite (`run`) has advanced `step` past 12.
#[allow(clippy::too_many_arguments)]
pub fn run_combat(
    mut t: ResMut<AutoTest>,
    mut commands: Commands,
    mut party: ResMut<Party>,
    item_catalog: Res<ItemCatalog>,
    monster_catalog: Res<MonsterCatalog>,
    mut player: ResMut<Player>,
    clock: Res<GameClock>,
    log: Res<MessageLog>,
    mut script: ResMut<ScriptedInput>,
    mut selected: ResMut<SelectedMember>,
    enemy_near: Res<EnemyNear>,
    mut resting: ResMut<Resting>,
    mut data: ResMut<DataScreen>,
    mut monsters: Query<(Entity, &mut Monster)>,
    floor_items: Query<&FloorItem>,
    mut exit: EventWriter<AppExit>,
) {
    if t.step < 13 || t.fatal.is_some() {
        return;
    }
    let fail = |t: &AutoTest, name: &str, why: String, exit: &mut EventWriter<AppExit>| {
        eprintln!("[autotest] FAIL {name}: {why}");
        eprintln!("[autotest] {} step(s) passed before the failure", t.step);
        exit.send(AppExit::error());
    };
    let hp_of = |e: Entity, q: &Query<(Entity, &mut Monster)>| q.get(e).map(|(_, m)| m.hp).ok();
    let dead_of = |e: Entity, q: &Query<(Entity, &mut Monster)>| q.get(e).map(|(_, m)| m.dead).ok();

    match t.step {
        // ---- 13: attacking a monster reduces its HP (retry on a miss) --------
        13 => {
            if t.monster_ent.is_none() {
                let (e, _) = spawn_subject(&mut commands, &player, "skel_guard", 999);
                t.monster_ent = Some(e);
                t.baseline = 999;
                // Concentrate first (also covers the Concentrate command), then hit.
                push_cmd(&mut script, Command::Concentrate);
                push_cmd(&mut script, Command::Attack);
                return;
            }
            let e = t.monster_ent.unwrap();
            match hp_of(e, &monsters) {
                Some(hp) if hp < t.baseline => {
                    script.active = false;
                    commands.entity(e).despawn();
                    t.next_step("combat-hit: Attack lowers monster HP");
                }
                _ => {
                    // Miss or not resolved yet — retry every ~20 frames.
                    if t.frames.is_multiple_of(20) && t.frames > 0 {
                        push_cmd(&mut script, Command::Attack);
                    }
                    if t.frames > STEP_TIMEOUT_FRAMES {
                        fail(&t, "combat-hit", "攻撃してもHPが減らない".into(), &mut exit);
                    }
                }
            }
        }

        // ---- 14: a kill drops carry items and grants experience --------------
        14 => {
            // A regen-0 monster despawns the instant it dies, so we verify by the
            // lasting effects (loot on the floor, exp, log), not the entity.
            match t.phase {
                0 => {
                    let (e, pos) = spawn_subject(&mut commands, &player, "skel_minion", 1);
                    t.monster_ent = Some(e);
                    t.saved_pos = Some(pos);
                    t.baseline = party.members.iter().map(|m| m.state.exp).sum();
                    t.phase = 1;
                }
                1 => {
                    // Entity now exists: give it loot, then strike.
                    let e = t.monster_ent.unwrap();
                    if let Ok((_, mut m)) = monsters.get_mut(e) {
                        m.carry = vec!["glow_stone".into()];
                        push_cmd(&mut script, Command::Attack);
                        t.phase = 2;
                    } else if t.frames > STEP_TIMEOUT_FRAMES {
                        fail(&t, "combat-kill", "敵がスポーンしない".into(), &mut exit);
                    }
                }
                _ => {
                    let pos = t.saved_pos.unwrap();
                    let exp_now: i32 = party.members.iter().map(|m| m.state.exp).sum();
                    let dropped = floor_items
                        .iter()
                        .any(|it| it.pos == pos && it.instance.def_id == "glow_stone");
                    if dropped && log.contains("たおした") && exp_now > t.baseline {
                        script.active = false;
                        t.next_step("combat-kill drops loot + grants exp");
                    } else if t.frames.is_multiple_of(20) && t.frames > 0 {
                        push_cmd(&mut script, Command::Attack);
                    }
                    if t.frames > STEP_TIMEOUT_FRAMES {
                        fail(&t, "combat-kill", format!("drop={dropped} exp {} → {exp_now}", t.baseline), &mut exit);
                    }
                }
            }
        }

        // ---- 15: enough experience levels a member up ------------------------
        15 => match t.phase {
            0 => {
                // Tip every member to one exp short of the next level, so whoever
                // the exp share reaches will cross it.
                for m in &mut party.members {
                    let need = crate::character::level_up_threshold(m.character.stats.level);
                    m.state.exp = need - 1;
                }
                t.baseline = party.members[0].character.stats.level as i32;
                let (e, _) = spawn_subject(&mut commands, &player, "skel_warrior", 1);
                t.monster_ent = Some(e);
                t.phase = 1;
            }
            1 => {
                let e = t.monster_ent.unwrap();
                if monsters.get(e).is_ok() {
                    push_cmd(&mut script, Command::Attack);
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "levelup", "敵がスポーンしない".into(), &mut exit);
                }
            }
            _ => {
                let lvl = party.members[0].character.stats.level as i32;
                if lvl > t.baseline && log.contains("レベル") {
                    script.active = false;
                    t.next_step("levelup: killing over the threshold raises level");
                } else if t.frames.is_multiple_of(20) && t.frames > 0 {
                    push_cmd(&mut script, Command::Attack);
                }
                if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "levelup", format!("level {} → {lvl}", t.baseline), &mut exit);
                }
            }
        },

        // ---- 16: guarding halves the next incoming hit -----------------------
        16 => match t.phase {
            0 => {
                // Guard the party, and place an attacker adjacent but not yet
                // ready to strike (so guard is applied first).
                push_cmd(&mut script, Command::Guard);
                let (e, _) = spawn_subject(&mut commands, &player, "skel_warrior", 999);
                if let Ok((_, mut m)) = monsters.get_mut(e) {
                    m.next_attack = u64::MAX;
                    m.fleeing = false;
                }
                t.monster_ent = Some(e);
                t.phase = 1;
            }
            1 => {
                // Once guard is in force, arm the monster and snapshot HP.
                if party.members.iter().any(|m| m.state.guarding) {
                    let e = t.monster_ent.unwrap();
                    if let Ok((_, mut m)) = monsters.get_mut(e) {
                        m.next_attack = clock.cycle;
                    }
                    t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "guard", "防ぐが適用されない".into(), &mut exit);
                }
            }
            _ => {
                // Detect the first member whose HP dropped, check the halving.
                if let Some((i, drop)) = party.members.iter().enumerate().find_map(|(i, m)| {
                    let before = *t.hp_before.get(i).unwrap_or(&m.state.hp);
                    (m.state.hp < before).then_some((i, before - m.state.hp))
                }) {
                    let def = monster_catalog.get("skel_warrior").unwrap();
                    let full = combat::final_damage(
                        0,
                        def.attack,
                        party.members[i].character.stats.defense,
                        0,
                        1,
                    );
                    let expected = combat::guarded(full, true);
                    if drop == expected {
                        if let Some(e) = t.monster_ent.take() {
                            commands.entity(e).despawn();
                        }
                        t.next_step("guard halves the incoming hit");
                    } else {
                        fail(&t, "guard", format!("drop {drop} ≠ {expected} (full {full})"), &mut exit);
                    }
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "guard", "敵が一度も当ててこない".into(), &mut exit);
                }
            }
        },

        // ---- 17: throwing an item damages a monster and drops the item -------
        17 => {
            if t.monster_ent.is_none() {
                use crate::item::{ItemInstance, SlotRef};
                // Put a throwing knife in member 0's left hand.
                selected.index = 0;
                party.members[0].inventory.take(SlotRef::Hand(0));
                let _ = party.members[0]
                    .inventory
                    .put(SlotRef::Hand(0), ItemInstance::new("throwing_knife"));
                let (e, _) = spawn_subject(&mut commands, &player, "skel_guard", 999);
                t.monster_ent = Some(e);
                t.baseline = 999;
                push_cmd(&mut script, Command::Throw);
                return;
            }
            let e = t.monster_ent.unwrap();
            let hurt = hp_of(e, &monsters).is_some_and(|hp| hp < t.baseline);
            let dropped = floor_items.iter().any(|it| it.instance.def_id == "throwing_knife");
            if hurt && dropped {
                script.active = false;
                commands.entity(e).despawn();
                t.next_step("throw damages a monster and lands the item");
            } else if t.frames > STEP_TIMEOUT_FRAMES {
                fail(&t, "throw", format!("hurt={hurt} dropped={dropped}"), &mut exit);
            }
        }

        // ---- 18: a skilled thief succeeds (item moves) -----------------------
        18 => {
            use crate::item::ItemInstance;
            if t.monster_ent.is_none() {
                // Select the best thief.
                let thief = party
                    .members
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, m)| m.effective_stats(&item_catalog).get(StatKind::Stealing))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                selected.index = thief;
                let (e, _) = spawn_subject(&mut commands, &player, "skel_minion", 999);
                if let Ok((_, mut m)) = monsters.get_mut(e) {
                    m.carry = vec!["glow_stone".into()];
                }
                t.monster_ent = Some(e);
                t.baseline = party.members[thief]
                    .inventory
                    .iter()
                    .filter(|(_, it)| it.def_id == "glow_stone")
                    .count() as i32;
                push_cmd(&mut script, Command::Steal);
                return;
            }
            let thief = selected.index;
            let have = party.members[thief]
                .inventory
                .iter()
                .filter(|(_, it)| it.def_id == "glow_stone")
                .count() as i32;
            if have > t.baseline && log.contains("ぬすんだ") {
                script.active = false;
                if let Some(e) = t.monster_ent.take() {
                    commands.entity(e).despawn();
                }
                t.next_step("steal succeeds for a skilled thief");
            } else {
                // Retry (deterministic RNG still advances each attempt).
                if t.frames.is_multiple_of(15) && t.frames > 0 {
                    // Refill carry if a prior failed attempt emptied it (it won't
                    // on failure, but keep it robust) and retry.
                    let e = t.monster_ent.unwrap();
                    if let Ok((_, mut m)) = monsters.get_mut(e)
                        && m.carry.is_empty() {
                            m.carry = vec!["glow_stone".into()];
                        }
                    let _ = ItemInstance::new("x");
                    push_cmd(&mut script, Command::Steal);
                    t.attempts += 1;
                }
                if t.attempts > 40 {
                    fail(&t, "steal", "熟練でも一度も盗めない".into(), &mut exit);
                }
            }
        }

        // ---- 19: stealing from a wary/empty monster fails + counterattack ----
        19 => {
            if t.monster_ent.is_none() {
                // Empty carry ⇒ the steal always takes the fail path.
                let (e, _) = spawn_subject(&mut commands, &player, "skel_warrior", 999);
                if let Ok((_, mut m)) = monsters.get_mut(e) {
                    m.carry.clear();
                    m.fleeing = false;
                }
                t.monster_ent = Some(e);
                push_cmd(&mut script, Command::Steal);
                return;
            }
            if log.contains("盗みに失敗") && log.contains("こうげき") {
                script.active = false;
                if let Some(e) = t.monster_ent.take() {
                    commands.entity(e).despawn();
                }
                t.next_step("failed steal provokes a counterattack");
            } else if t.frames > STEP_TIMEOUT_FRAMES {
                fail(&t, "steal-fail", "失敗/反撃のログが出ない".into(), &mut exit);
            }
        }

        // ---- 20: a badly hurt monster flees (distance grows) -----------------
        // Run in the open floor-0 room (the cramped start room boxes a monster in).
        20 => {
            match t.phase {
                0 => {
                    player.pos = GridPos::new(5, 5, 0);
                    let mut m = Monster::new_at("skel_warrior", 3, GridPos::new(6, 5, 0), Facing::West);
                    m.fleeing = true; // hp below flee_hp
                    let e = commands.spawn(m).id();
                    t.monster_ent = Some(e);
                    t.baseline = 1; // initial chebyshev distance
                    t.mark_cycle = clock.cycle;
                    t.phase = 1;
                }
                _ => {
                    let e = t.monster_ent.unwrap();
                    if let Ok((_, m)) = monsters.get(e) {
                        let dist = (m.pos.x - player.pos.x).abs().max((m.pos.y - player.pos.y).abs());
                        if dist > t.baseline {
                            commands.entity(e).despawn();
                            player.pos = t.start_pos.unwrap();
                            t.next_step("a fleeing monster increases its distance");
                        } else if clock.cycle >= t.mark_cycle + 60 {
                            fail(&t, "flee", format!("距離が広がらない (dist {dist})"), &mut exit);
                        }
                    }
                }
            }
        }

        // ---- 21: a monster with regen revives after its delay ----------------
        21 => {
            if t.monster_ent.is_none() {
                // Somewhere the player is not standing.
                let pos = GridPos::new(player.pos.x, player.pos.y + 2, player.pos.floor);
                let mut m = Monster::new_at("skel_rogue", 0, pos, Facing::North);
                m.dead = true;
                m.dead_cycle = clock.cycle;
                let e = commands.spawn(m).id();
                t.monster_ent = Some(e);
                t.mark_cycle = clock.cycle;
                return;
            }
            let e = t.monster_ent.unwrap();
            if dead_of(e, &monsters) == Some(false) {
                commands.entity(e).despawn();
                t.next_step("regen revives the monster");
            } else if clock.cycle >= t.mark_cycle + 120 {
                fail(&t, "regen", "regen_cycles 経過後も復活しない".into(), &mut exit);
            }
        }

        // ---- 22: ZZZ resting is blocked / interrupted near a monster ---------
        22 => {
            if t.monster_ent.is_none() {
                // skel_minion has sight 6, so it sees the adjacent party.
                let (dx, dy) = player.facing.delta();
                let front = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
                let e = commands.spawn(Monster::new_at("skel_minion", 999, front, Facing::North)).id();
                t.monster_ent = Some(e);
                return;
            }
            match t.phase {
                0 => {
                    // Let occupancy / enemy-near update, then try to rest.
                    if enemy_near.0 {
                        data.open = true;
                        resting.active = true;
                        t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                        t.mark_cycle = clock.cycle;
                        t.phase = 1;
                    } else if t.frames > STEP_TIMEOUT_FRAMES {
                        fail(&t, "rest-blocked", "モンスターを視認できていない".into(), &mut exit);
                    }
                }
                _ => {
                    if clock.cycle >= t.mark_cycle + 12 {
                        let interrupted = !resting.active;
                        let no_heal = party
                            .members
                            .iter()
                            .zip(&t.hp_before)
                            .all(|(m, &b)| m.state.hp <= b);
                        if interrupted && no_heal && log.contains("モンスター") {
                            println!("[autotest] PASS rest is blocked near a monster");
                            println!("[autotest] ALL PASS (22 steps)");
                            exit.send(AppExit::Success);
                        } else {
                            fail(&t, "rest-blocked", format!("interrupted={interrupted} no_heal={no_heal}"), &mut exit);
                        }
                    }
                }
            }
        }

        _ => {}
    }
}
