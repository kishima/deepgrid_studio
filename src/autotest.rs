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
    // plan10 save-load determinism scratch.
    rng_seq: Vec<usize>,
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
    rules: Res<crate::rules::RulesConfig>,
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
            match m.eat(&def, &catalog, &rules.hunger) {
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
            match party.members[idx].eat(&def, &catalog, &rules.hunger) {
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
                            // Clean up the sentinel and hand off to hunger steps.
                            resting.active = false;
                            data.open = false;
                            if let Some(e) = t.monster_ent.take() {
                                commands.entity(e).despawn();
                            }
                            t.next_step("rest is blocked near a monster");
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

// ==================================================================== hunger steps

use crate::rules::RulesConfig;

/// Steps 23–26 (plan6.5): satiety drain, feeding, starvation, and rest-while-
/// starving. Kept separate from `run_combat` so neither exceeds the 16-parameter
/// system limit. Runs once `run_combat` advances `step` past 22.
#[allow(clippy::too_many_arguments)]
pub fn run_hunger(
    mut t: ResMut<AutoTest>,
    mut party: ResMut<Party>,
    item_catalog: Res<ItemCatalog>,
    clock: Res<GameClock>,
    log: Res<MessageLog>,
    rules: Res<RulesConfig>,
    mut screen: ResMut<crate::game_state::DataScreen>,
    mut resting: ResMut<crate::data_screen::Resting>,
    mut exit: EventWriter<AppExit>,
) {
    if t.step < 23 || t.fatal.is_some() {
        return;
    }
    let fail = |t: &AutoTest, name: &str, why: String, exit: &mut EventWriter<AppExit>| {
        eprintln!("[autotest] FAIL {name}: {why}");
        eprintln!("[autotest] {} step(s) passed before the failure", t.step);
        exit.send(AppExit::error());
    };
    let h = &rules.hunger;
    if !h.enabled {
        fail(&t, "hunger", "sample の hunger が無効(project.ron の rules を確認)".into(), &mut exit);
        return;
    }

    match t.step {
        // ---- 23: satiety drains over time ------------------------------------
        23 => match t.phase {
            0 => {
                for m in &mut party.members {
                    m.state.satiety = h.satiety_max;
                }
                t.baseline = h.satiety_max;
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                // After ~5 drain intervals, satiety must have fallen accordingly.
                let elapsed = clock.cycle - t.mark_cycle;
                if elapsed >= h.drain_interval_cycles * 5 {
                    let sat = party.members[0].state.satiety;
                    let expected_drop = (elapsed / h.drain_interval_cycles) as i32;
                    // Allow ±1 for where the frame boundary lands.
                    if sat <= t.baseline - expected_drop + 1 && sat < t.baseline {
                        t.next_step("hunger-drain: satiety falls over time");
                    } else {
                        fail(&t, "hunger-drain", format!("satiety {sat}, 期待 ≈ {}", t.baseline - expected_drop), &mut exit);
                    }
                }
            }
        },

        // ---- 24: eating restores satiety (and HP, as before) -----------------
        24 => {
            // Find an edible, nutritious item and a member who can bite it.
            let mut defs: Vec<_> = item_catalog.iter().filter(|d| d.nutrition > 0 && !d.important).collect();
            defs.sort_by(|a, b| a.id.cmp(&b.id));
            let pick = defs.iter().find_map(|d| {
                party
                    .members
                    .iter()
                    .position(|m| m.effective_stats(&item_catalog).get(StatKind::Bite) >= d.hardness)
                    .map(|i| (i, (*d).clone()))
            });
            let Some((idx, def)) = pick else {
                fail(&t, "hunger-eat", "食べられる栄養アイテムが無い".into(), &mut exit);
                return;
            };
            let m = &mut party.members[idx];
            m.state.satiety = 100;
            m.state.hp = (m.state.hp - def.nutrition.max(5)).max(1);
            let (sat_before, hp_before) = (m.state.satiety, m.state.hp);
            match m.eat(&def, &item_catalog, h) {
                Ok(_) => {
                    let expected = (sat_before + def.nutrition * h.satiety_per_nutrition).min(h.satiety_max);
                    if m.state.satiety == expected && m.state.hp > hp_before {
                        t.next_step("hunger-eat: eating restores satiety + HP");
                    } else {
                        fail(&t, "hunger-eat", format!("satiety {} 期待 {expected}, hp {} → {}", m.state.satiety, hp_before, m.state.hp), &mut exit);
                    }
                }
                Err(e) => fail(&t, "hunger-eat", format!("食べられなかった: {e}"), &mut exit),
            }
        }

        // ---- 25: starvation damages HP + freezes concentration + warns -------
        25 => match t.phase {
            0 => {
                for m in &mut party.members {
                    m.state.satiety = 0;
                    m.state.down = false;
                    m.state.concentration = 0; // room to (not) recover
                }
                t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                if clock.cycle >= t.mark_cycle + 12 {
                    let hurt = party.members.iter().zip(&t.hp_before).all(|(m, &b)| m.state.hp < b || m.state.hp == 0);
                    let no_focus = party.members.iter().all(|m| m.state.concentration == 0);
                    let warned = log.contains("うえじ") || log.contains("おなか");
                    if hurt && no_focus && warned {
                        t.next_step("hunger-starve: HP falls, focus frozen, warned");
                    } else {
                        fail(&t, "hunger-starve", format!("hurt={hurt} no_focus={no_focus} warned={warned}"), &mut exit);
                    }
                }
            }
        },

        // ---- 26: resting while starving does not heal ------------------------
        26 => match t.phase {
            0 => {
                for m in &mut party.members {
                    m.state.satiety = 0;
                    m.state.down = false;
                    m.state.hp = 20; // mid, room to heal if the gate failed
                }
                screen.open = true;
                resting.active = true;
                t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                if clock.cycle >= t.mark_cycle + h.drain_interval_cycles * 3 {
                    // Rest must not raise HP while starving (starvation may lower it).
                    let no_heal = party.members.iter().zip(&t.hp_before).all(|(m, &b)| m.state.hp <= b);
                    if no_heal {
                        screen.open = false;
                        resting.active = false;
                        // Hand off to the magic steps (run_magic, step ≥ 27).
                        t.next_step("hunger-rest: no healing while starving");
                    } else {
                        fail(&t, "hunger-rest", "飢餓中の休息でHPが回復した".into(), &mut exit);
                    }
                }
            }
        },

        _ => {}
    }
}

// ==================================================================== magic steps

use crate::magic::{
    BASE_LIGHT_INTENSITY, CastMagic, CastTarget, LightBoost, MagicCatalog, PlayerLight,
};

/// Steps 27–33 (plan7): scroll learning, buff/attack/revive casting, the MP gate,
/// the lighting boost, and the potion round-trip. Runs once `run_hunger` advances
/// `step` past 26. Casting is driven through the real `CastMagic` event, and the
/// non-combat helpers (`learn_scroll` / `liquefy` / `drink_potion`) are exercised
/// directly — the same functions the data-screen UI calls.
#[allow(clippy::too_many_arguments)]
pub fn run_magic(
    mut t: ResMut<AutoTest>,
    mut commands: Commands,
    mut party: ResMut<Party>,
    item_catalog: Res<ItemCatalog>,
    magic_catalog: Res<MagicCatalog>,
    rules: Res<RulesConfig>,
    clock: Res<GameClock>,
    log: Res<MessageLog>,
    player: Res<Player>,
    mut cast_ev: EventWriter<CastMagic>,
    mut boost: ResMut<LightBoost>,
    lights: Query<&PointLight, With<PlayerLight>>,
    monsters: Query<(Entity, &Monster)>,
    mut exit: EventWriter<AppExit>,
) {
    if t.step < 27 || t.fatal.is_some() {
        return;
    }
    let fail = |t: &AutoTest, name: &str, why: String, exit: &mut EventWriter<AppExit>| {
        eprintln!("[autotest] FAIL {name}: {why}");
        eprintln!("[autotest] {} step(s) passed before the failure", t.step);
        exit.send(AppExit::error());
    };
    let learns = |m: &crate::character::PartyMember, id: &str| m.state.learned.iter().any(|l| l == id);
    let ensure_learned = |m: &mut crate::character::PartyMember, id: &str| {
        if !m.state.learned.iter().any(|l| l == id) {
            m.state.learned.push(id.to_string());
        }
    };

    match t.step {
        // ---- 27: reading a scroll teaches a known spell (or fails, kept) -----
        27 => {
            use crate::item::{ItemInstance, SlotRef};
            let scroll_heal = item_catalog.get("scroll_heal").cloned();
            let scroll_fire = item_catalog.get("scroll_fire").cloned();
            let (Some(scroll_heal), Some(scroll_fire)) = (scroll_heal, scroll_fire) else {
                fail(&t, "learn-scroll", "巻物アイテムが定義されていない".into(), &mut exit);
                return;
            };
            // Pick a caster who does NOT yet know `heal` (learns it) and one whose
            // magic knowledge is below firebolt's difficulty (fails).
            let learner = party.members.iter().position(|m| !learns(m, "heal"));
            let weak = party
                .members
                .iter()
                .position(|m| m.effective_stats(&item_catalog).get(StatKind::MagicKnowledge) < 20);
            let (Some(li), Some(wi)) = (learner, weak) else {
                fail(&t, "learn-scroll", "適切な学習者/知識不足キャラがいない".into(), &mut exit);
                return;
            };

            // Success path: scroll in hand → learn → consume.
            let ok;
            let learned_now;
            let scroll_gone;
            {
                let m = &mut party.members[li];
                let slot = m.inventory.pickup(ItemInstance::new("scroll_heal")).unwrap_or(SlotRef::Hand(0));
                match crate::magic::learn_scroll(m, &scroll_heal, &magic_catalog, &item_catalog) {
                    Ok(_) => {
                        m.inventory.take(slot);
                        ok = true;
                    }
                    Err(_) => ok = false,
                }
                learned_now = learns(m, "heal");
                scroll_gone = !m.inventory.iter().any(|(_, it)| it.def_id == "scroll_heal");
            }
            // Failure path: knowledge too low → error, scroll stays, no learn.
            let fail_kept;
            let no_learn;
            {
                let m = &mut party.members[wi];
                let slot = m.inventory.pickup(ItemInstance::new("scroll_fire")).unwrap_or(SlotRef::Hand(0));
                let res = crate::magic::learn_scroll(m, &scroll_fire, &magic_catalog, &item_catalog);
                if res.is_ok() {
                    m.inventory.take(slot);
                }
                fail_kept = res.is_err() && m.inventory.iter().any(|(_, it)| it.def_id == "scroll_fire");
                no_learn = !learns(m, "firebolt");
            }
            if ok && learned_now && scroll_gone && fail_kept && no_learn {
                t.next_step("learn-scroll: success learns+consumes, low-knowledge fails+keeps");
            } else {
                fail(&t, "learn-scroll",
                    format!("ok={ok} learned={learned_now} gone={scroll_gone} kept={fail_kept} no_learn={no_learn}"),
                    &mut exit);
            }
        }

        // ---- 28: a buff spell raises the stat, then wears off -----------------
        28 => match t.phase {
            0 => {
                let mage = party.members.iter().position(|m| learns(m, "firebolt")).unwrap_or(1);
                t.saved_pos = None;
                t.monster_ent = None;
                {
                    let m = &mut party.members[mage];
                    ensure_learned(m, "shield");
                    m.state.mp = 60;
                }
                t.baseline = party.members[mage].effective_stats(&item_catalog).get(StatKind::Defense);
                t.hp_before = vec![party.members[mage].state.mp, mage as i32];
                cast_ev.send(CastMagic { caster: mage, magic_id: "shield".into(), target: CastTarget::Member(mage) });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                let mage = t.hp_before[1] as usize;
                let mp_now = party.members[mage].state.mp;
                let def_now = party.members[mage].effective_stats(&item_catalog).get(StatKind::Defense);
                if mp_now < t.hp_before[0] && def_now == t.baseline + 10 {
                    for e in &mut party.members[mage].state.effects {
                        if e.source.as_deref() == Some("shield") {
                            e.remaining = Some(1);
                        }
                    }
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "cast-buff", format!("mp {} def {def_now} (base {})", mp_now, t.baseline), &mut exit);
                }
            }
            _ => {
                let mage = t.hp_before[1] as usize;
                let def_now = party.members[mage].effective_stats(&item_catalog).get(StatKind::Defense);
                if def_now == t.baseline {
                    t.next_step("cast-buff: MP spent, defence up, then expires");
                } else if clock.cycle >= t.mark_cycle + 6 {
                    fail(&t, "cast-buff", format!("持続切れ後も def {def_now} ≠ {}", t.baseline), &mut exit);
                }
            }
        },

        // ---- 29: attack magic damages a monster; a warded one resists --------
        29 => match t.phase {
            0 => {
                let mage = party.members.iter().position(|m| learns(m, "firebolt")).unwrap_or(1);
                party.members[mage].state.mp = 60;
                t.saved_pos = Some(GridPos::new(mage as i32, 0, 0)); // stash mage index in .x
                // skel_guard: immobile, never attacks, 0 anti-magic → full damage.
                let (e, _) = spawn_subject(&mut commands, &player, "skel_guard", 999);
                t.monster_ent = Some(e);
                cast_ev.send(CastMagic { caster: mage, magic_id: "firebolt".into(), target: CastTarget::FrontEnemy });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                let e = t.monster_ent.unwrap();
                let hp = monsters.get(e).map(|(_, m)| m.hp).unwrap_or(999);
                if hp < 999 {
                    t.baseline = 999 - hp; // damage against a 0-resist target
                    commands.entity(e).despawn();
                    let mage = t.saved_pos.unwrap().x as usize;
                    party.members[mage].state.mp = 60;
                    let (e2, _) = spawn_subject(&mut commands, &player, "skel_warded", 999);
                    t.monster_ent = Some(e2);
                    cast_ev.send(CastMagic { caster: mage, magic_id: "firebolt".into(), target: CastTarget::FrontEnemy });
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "cast-attack", "通常個体にダメージが入らない".into(), &mut exit);
                }
            }
            _ => {
                // Give the warded cast a few cycles to resolve, then compare.
                if clock.cycle >= t.mark_cycle + 4 {
                    let e = t.monster_ent.unwrap();
                    let hp = monsters.get(e).map(|(_, m)| m.hp).unwrap_or(999);
                    let dmg_b = 999 - hp;
                    if dmg_b < t.baseline {
                        commands.entity(e).despawn();
                        t.next_step("cast-attack: normal takes damage, warded resists");
                    } else {
                        fail(&t, "cast-attack", format!("warded dmg {dmg_b} ≥ normal dmg {}", t.baseline), &mut exit);
                    }
                }
            }
        },

        // ---- 30: reviving a downed member restores HP ------------------------
        30 => match t.phase {
            0 => {
                let healer = party.members.iter().position(|m| learns(m, "revive50"));
                let Some(healer) = healer else {
                    fail(&t, "cast-revive", "revive を覚えたキャラがいない".into(), &mut exit);
                    return;
                };
                let victim = (0..party.members.len()).find(|&i| i != healer).unwrap_or(0);
                party.members[healer].state.mp = 60;
                party.members[victim].state.hp = 0;
                party.members[victim].state.down = true;
                t.baseline = victim as i32;
                cast_ev.send(CastMagic { caster: healer, magic_id: "revive50".into(), target: CastTarget::DownedAuto });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                let victim = t.baseline as usize;
                let m = &party.members[victim];
                if !m.state.down {
                    let max_hp = m.effective_stats(&item_catalog).get(StatKind::MaxHp).max(1);
                    let expected = (max_hp * 50 / 100).clamp(1, max_hp);
                    if m.state.hp == expected {
                        t.next_step("cast-revive: downed member back at 50% HP");
                    } else {
                        fail(&t, "cast-revive", format!("hp {} ≠ {expected}", m.state.hp), &mut exit);
                    }
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "cast-revive", "気絶が解除されない".into(), &mut exit);
                }
            }
        },

        // ---- 31: casting with no MP is refused (no effect, no MP change) -----
        31 => match t.phase {
            0 => {
                let mage = party.members.iter().position(|m| learns(m, "firebolt")).unwrap_or(1);
                {
                    let m = &mut party.members[mage];
                    ensure_learned(m, "shield");
                    m.state.mp = 0;
                }
                t.baseline = party.members[mage].effective_stats(&item_catalog).get(StatKind::Defense);
                t.hp_before = vec![0, mage as i32];
                cast_ev.send(CastMagic { caster: mage, magic_id: "shield".into(), target: CastTarget::Member(mage) });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                if clock.cycle >= t.mark_cycle + 3 {
                    let mage = t.hp_before[1] as usize;
                    let mp_now = party.members[mage].state.mp;
                    let def_now = party.members[mage].effective_stats(&item_catalog).get(StatKind::Defense);
                    let refused = log.contains("たりない");
                    if mp_now == 0 && def_now == t.baseline && refused {
                        t.next_step("mp-gate: no-MP cast refused, nothing changes");
                    } else {
                        fail(&t, "mp-gate", format!("mp {mp_now} def {def_now} refused={refused}"), &mut exit);
                    }
                }
            }
        },

        // ---- 32: a lighting spell boosts the player light, then fades --------
        32 => match t.phase {
            0 => {
                let mage = party.members.iter().position(|m| learns(m, "light2")).unwrap_or(1);
                party.members[mage].state.mp = 60;
                cast_ev.send(CastMagic { caster: mage, magic_id: "light2".into(), target: CastTarget::Caster });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                let intensity = lights.get_single().map(|l| l.intensity).unwrap_or(0.0);
                let want = BASE_LIGHT_INTENSITY * 2.5;
                if (boost.multiplier - 2.5).abs() < 0.01 && (intensity - want).abs() < 1.0 {
                    boost.remaining = 1; // force a quick expiry for the test
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "light", format!("mult {} intensity {intensity} (want {want})", boost.multiplier), &mut exit);
                }
            }
            _ => {
                if clock.cycle >= t.mark_cycle + 3 {
                    let intensity = lights.get_single().map(|l| l.intensity).unwrap_or(0.0);
                    if (boost.multiplier - 1.0).abs() < 0.01 && (intensity - BASE_LIGHT_INTENSITY).abs() < 1.0 {
                        t.next_step("light: intensity boosts ×2.5 then returns to base");
                    } else {
                        fail(&t, "light", format!("expiry mult {} intensity {intensity}", boost.multiplier), &mut exit);
                    }
                }
            }
        },

        // ---- 33: liquefy a spell, then drink it ------------------------------
        33 => {
            use crate::item::ItemInstance;
            let brewer = party.members.iter().position(|m| learns(m, "heal"));
            let Some(bi) = brewer else {
                fail(&t, "potion", "heal を覚えたキャラがいない".into(), &mut exit);
                return;
            };
            let heal_def = magic_catalog.get("heal").cloned().unwrap();
            let m = &mut party.members[bi];
            m.state.mp = 40;
            let _ = m.inventory.pickup(ItemInstance::new("bottle_empty"));
            let mp_before = m.state.mp;
            let liq = crate::magic::liquefy(m, &heal_def, &item_catalog);
            let potion_slot = m
                .inventory
                .iter()
                .find(|(_, it)| it.potion_of.as_deref() == Some("heal"))
                .map(|(s, _)| s);
            let mp_after_liq = m.state.mp;
            let max_hp = m.effective_stats(&item_catalog).get(StatKind::MaxHp);
            m.state.hp = (max_hp / 2).max(1);
            let hp_before = m.state.hp;
            let drink_ok = match potion_slot {
                Some(s) => crate::magic::drink_potion(m, s, &magic_catalog, &item_catalog, &rules.hunger).is_ok(),
                None => false,
            };
            let hp_after = m.state.hp;
            let reverted = potion_slot
                .and_then(|s| m.inventory.get(s))
                .map(|it| it.potion_of.is_none())
                .unwrap_or(false);
            if liq.is_ok() && potion_slot.is_some() && mp_after_liq < mp_before && drink_ok && hp_after > hp_before && reverted {
                // Hand off to the gimmick steps (run_gimmick, step ≥ 34).
                t.next_step("potion: liquefy → drink heals → bottle empties");
            } else {
                fail(&t, "potion",
                    format!("liq={} slot={} mp {}→{} drink={drink_ok} hp {hp_before}→{hp_after} reverted={reverted}",
                        liq.is_ok(), potion_slot.is_some(), mp_before, mp_after_liq),
                    &mut exit);
            }
        }

        _ => {}
    }
}

// ==================================================================== gimmick steps

use crate::event::{EventFlags, EventQueue, FrontInteract, TriggerStates};
use crate::world::CurrentLevel;

/// Read-only world refs for the gimmick steps, bundled to fit the parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct GimmickRefs<'w, 's> {
    pub flags: ResMut<'w, EventFlags>,
    pub triggers: Res<'w, TriggerStates>,
    pub current: Res<'w, CurrentLevel>,
    pub dungeon: Res<'w, Dungeon>,
    pub doors: Res<'w, DoorStates>,
    // plan10 additions (steps 44–47).
    pub bgm: ResMut<'w, crate::audio::BgmState>,
    pub demo: ResMut<'w, crate::demo::DemoState>,
    pub rng: ResMut<'w, crate::rng::GameRng>,
    pub settings: ResMut<'w, crate::settings::UserSettings>,
    pub save_req: EventWriter<'w, crate::save::SaveRequest>,
    pub load_req: EventWriter<'w, crate::save::LoadRequest>,
    pub queue: ResMut<'w, EventQueue>,
    pub bgm_channels: Query<'w, 's, &'static crate::audio::BgmChannel>,
    pub demo_overlays: Query<'w, 's, Entity, With<crate::demo::DemoOverlay>>,
}

/// Steps 34–42 (plan8): floor plates, flag AND/OR, delay/Loop/EndChain, keyhole,
/// switch forms, hidden warp, SetBlock/SetLiquid, stairs state, hole + vertical
/// horoscope. Everything runs through the real event pipeline (teleport onto a
/// trigger tile, or send a FrontInteract / scripted command) — no direct flag or
/// queue pokes except clearing a deliberately-infinite Loop.
#[allow(clippy::too_many_arguments)]
pub fn run_gimmick(
    mut t: ResMut<AutoTest>,
    mut commands: Commands,
    mut party: ResMut<Party>,
    mut player: ResMut<Player>,
    clock: Res<GameClock>,
    log: Res<MessageLog>,
    item_catalog: Res<ItemCatalog>,
    mut refs: GimmickRefs,
    mut script: ResMut<ScriptedInput>,
    mut interact: EventWriter<FrontInteract>,
    mut wall_write: EventWriter<crate::event::WallWriteRequest>,
    mut data: ResMut<DataScreen>,
    monsters: Query<(Entity, &Monster)>,
    floor_items: Query<&FloorItem>,
    mut exit: EventWriter<AppExit>,
) {
    if t.step < 34 || t.fatal.is_some() {
        return;
    }
    let fail = |t: &AutoTest, name: &str, why: String, exit: &mut EventWriter<AppExit>| {
        eprintln!("[autotest] FAIL {name}: {why}");
        eprintln!("[autotest] {} step(s) passed before the failure", t.step);
        exit.send(AppExit::error());
    };
    let item_count = |x: i32, y: i32, f: usize, id: &str| {
        floor_items
            .iter()
            .filter(|it| it.pos == GridPos::new(x, y, f) && it.instance.def_id == id)
            .count()
    };
    let monster_at = |x: i32, y: i32, f: usize| monsters.iter().any(|(_, m)| !m.dead && m.pos == GridPos::new(x, y, f));

    match t.step {
        // ---- 34: a floor plate (Step) spawns a monster ----------------------
        34 => match t.phase {
            0 => {
                data.open = false;
                script.active = false;
                // Clean slate: remove every existing monster.
                for (e, _) in &monsters {
                    commands.entity(e).despawn_recursive();
                }
                player.pos = GridPos::new(26, 19, 0); // neutral tile (arm the edge)
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                // Step onto the plate at (27,20,0) → SpawnMonster at (29,20,0).
                if clock.cycle > t.mark_cycle {
                    player.pos = GridPos::new(27, 20, 0);
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                }
            }
            _ => {
                if monster_at(29, 20, 0) {
                    t.next_step("plate-step: stepping the plate spawns a monster");
                } else if clock.cycle >= t.mark_cycle + 8 {
                    fail(&t, "plate-step", "しかけ床でモンスターが湧かない".into(), &mut exit);
                }
            }
        },

        // ---- 35: flag AND/OR gating ------------------------------------------
        35 => match t.phase {
            0 => {
                player.pos = GridPos::new(26, 19, 0);
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                // Step onto the AND-gated plate with flag 10 OFF → no spawn.
                player.pos = GridPos::new(27, 21, 0);
                t.mark_cycle = clock.cycle;
                t.phase = 2;
            }
            2 => {
                if clock.cycle >= t.mark_cycle + 3 {
                    if item_count(29, 21, 0, "glow_stone") != 0 {
                        fail(&t, "flags-andor", "フラグOFFなのに発火した".into(), &mut exit);
                        return;
                    }
                    // Turn flag 10 on via the switch, then leave the plate.
                    interact.send(FrontInteract { pos: GridPos::new(27, 22, 0) });
                    player.pos = GridPos::new(26, 19, 0);
                    t.mark_cycle = clock.cycle;
                    t.phase = 3;
                }
            }
            3 => {
                // Once flag 10 is on, re-step the AND plate → spawn.
                if refs.flags.get(10) {
                    player.pos = GridPos::new(27, 21, 0);
                    t.mark_cycle = clock.cycle;
                    t.phase = 4;
                } else if clock.cycle >= t.mark_cycle + 8 {
                    fail(&t, "flags-andor", "SetFlagが効かない".into(), &mut exit);
                }
            }
            4 => {
                if item_count(29, 21, 0, "glow_stone") >= 1 {
                    // OR plate: flag 10 on satisfies the OR.
                    player.pos = GridPos::new(27, 23, 0);
                    t.mark_cycle = clock.cycle;
                    t.phase = 5;
                } else if clock.cycle >= t.mark_cycle + 8 {
                    fail(&t, "flags-andor", "AND成立後も発火しない".into(), &mut exit);
                }
            }
            _ => {
                if item_count(29, 23, 0, "bread") >= 1 {
                    t.next_step("flags-andor: AND gates, OR fires, SetFlag flips");
                } else if clock.cycle >= t.mark_cycle + 8 {
                    fail(&t, "flags-andor", "OR条件で発火しない".into(), &mut exit);
                }
            }
        },

        // ---- 36: delay + Loop + EndChain -------------------------------------
        36 => match t.phase {
            0 => {
                // Fire the delayed switch (flag 20 after 5 cycles), the looping
                // switch, and the end-chained switch — all via a front press.
                interact.send(FrontInteract { pos: GridPos::new(28, 20, 0) });
                interact.send(FrontInteract { pos: GridPos::new(28, 21, 0) });
                interact.send(FrontInteract { pos: GridPos::new(28, 22, 0) });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                // Within the delay window, flag 20 must still be off.
                if clock.cycle >= t.mark_cycle + 2 {
                    if refs.flags.get(20) {
                        fail(&t, "delay-loop", "delay前にフラグが立った".into(), &mut exit);
                        return;
                    }
                    t.phase = 2;
                }
            }
            _ => {
                let loops = item_count(31, 21, 0, "glow_stone");
                let ends = item_count(31, 22, 0, "bread");
                if refs.flags.get(20) && loops >= 2 && ends == 1 {
                    // Stop the infinite loop before moving on.
                    refs.queue.pending.retain(|q| q.event_id != "at_loop");
                    t.next_step("delay-loop: delay elapses, Loop repeats, EndChain stops at 1");
                } else if clock.cycle >= t.mark_cycle + 30 {
                    fail(&t, "delay-loop", format!("flag20={} loops={loops} ends={ends}", refs.flags.get(20)), &mut exit);
                }
            }
        },

        // ---- 37: keyhole needs the key, fires once ---------------------------
        37 => match t.phase {
            0 => {
                // No key yet: press the keyhole → door stays shut, message shown.
                player.pos = GridPos::new(26, 23, 0);
                player.facing = Facing::South; // front = (26,24,0)
                interact.send(FrontInteract { pos: GridPos::new(26, 24, 0) });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                if clock.cycle >= t.mark_cycle + 2 {
                    if refs.doors.is_open(0) {
                        fail(&t, "keyhole", "鍵なしで開いた".into(), &mut exit);
                        return;
                    }
                    if !log.contains("あう鍵がない") {
                        fail(&t, "keyhole", "鍵なしメッセージが出ない".into(), &mut exit);
                        return;
                    }
                    // Give the party the key, then press again (scripted Interact).
                    let _ = party.members[0].inventory.pickup(ItemInstance::new("key_bronze"));
                    script.queue.push_back(Command::Interact);
                    script.active = true;
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                }
            }
            _ => {
                if refs.doors.is_open(0) && refs.triggers.fired.contains("at_keyhole") {
                    script.active = false;
                    t.next_step("keyhole: key opens the door, fires once");
                } else if clock.cycle >= t.mark_cycle + 12 {
                    fail(&t, "keyhole", format!("open={} fired={}", refs.doors.is_open(0), refs.triggers.fired.contains("at_keyhole")), &mut exit);
                }
            }
        },

        // ---- 38: switch forms fire different counts --------------------------
        38 => match t.phase {
            0 => {
                // Press each switch three times in one frame.
                for _ in 0..3 {
                    interact.send(FrontInteract { pos: GridPos::new(32, 20, 0) }); // OneWay
                    interact.send(FrontInteract { pos: GridPos::new(32, 21, 0) }); // Toggle
                    interact.send(FrontInteract { pos: GridPos::new(32, 22, 0) }); // Push
                }
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                let one = item_count(33, 20, 0, "glow_stone");
                let tog = item_count(33, 21, 0, "glow_stone");
                let push = item_count(33, 22, 0, "glow_stone");
                if one == 1 && tog == 3 && push == 3 {
                    t.next_step("switch-forms: OneWay=1, Toggle=3, Push=3");
                } else if clock.cycle >= t.mark_cycle + 10 {
                    fail(&t, "switch-forms", format!("one={one} tog={tog} push={push}"), &mut exit);
                }
            }
        },

        // ---- 39: a hidden warp jumps the party -------------------------------
        39 => match t.phase {
            0 => {
                player.pos = GridPos::new(30, 24, 0); // step onto the hidden warp
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                if player.pos == GridPos::new(26, 19, 0) && player.facing == Facing::South {
                    t.next_step("warp-hidden: entering the hidden warp jumps position + facing");
                } else if clock.cycle >= t.mark_cycle + 10 {
                    fail(&t, "warp-hidden", format!("pos {:?} facing {:?}", player.pos, player.facing), &mut exit);
                }
            }
        },

        // ---- 40: SetBlock walls a tile; SetLiquid floods it ------------------
        40 => match t.phase {
            0 => {
                player.pos = GridPos::new(34, 20, 0);
                player.facing = Facing::West; // front = (33,20,0)
                interact.send(FrontInteract { pos: GridPos::new(33, 20, 0) });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                if refs.dungeon.level.block_at(GridPos::new(33, 20, 0)) == Some(Block::Wall) {
                    // Try to walk into the new wall — it must be refused.
                    t.saved_pos = Some(player.pos);
                    script.queue.push_back(Command::Move(Action::Forward));
                    script.active = true;
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                } else if clock.cycle >= t.mark_cycle + 8 {
                    fail(&t, "setblock", "SetBlockで壁にならない".into(), &mut exit);
                }
            }
            2 => {
                if clock.cycle >= t.mark_cycle + 4 {
                    script.active = false;
                    if player.pos != t.saved_pos.unwrap() {
                        fail(&t, "setblock", "壁に入れてしまった".into(), &mut exit);
                        return;
                    }
                    // Now flood a tile and stand in it.
                    interact.send(FrontInteract { pos: GridPos::new(33, 22, 0) });
                    t.mark_cycle = clock.cycle;
                    t.phase = 3;
                }
            }
            3 => {
                if refs.dungeon.level.block_at(GridPos::new(33, 22, 0)) == Some(Block::Water) {
                    for m in &mut party.members {
                        m.state.hp = m.character.stats.max_hp;
                        m.state.down = false;
                    }
                    player.pos = GridPos::new(33, 22, 0);
                    t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                    t.mark_cycle = clock.cycle;
                    t.phase = 4;
                } else if clock.cycle >= t.mark_cycle + 8 {
                    fail(&t, "setblock", "SetLiquidで水にならない".into(), &mut exit);
                }
            }
            _ => {
                let vulnerable: Vec<usize> = party
                    .members
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.effective_stats(&item_catalog).get(StatKind::LungCapacity) < crate::hazard::WATER_LUNG_THRESHOLD)
                    .map(|(i, _)| i)
                    .collect();
                if clock.cycle >= t.mark_cycle + 14 {
                    let hurt = !vulnerable.is_empty() && vulnerable.iter().all(|&i| party.members[i].state.hp < t.hp_before[i]);
                    if hurt {
                        player.pos = GridPos::new(26, 19, 0); // step out of the water
                        t.next_step("setblock: wall refuses movement, liquid triggers hazard");
                    } else {
                        fail(&t, "setblock", "水hazardが作動しない".into(), &mut exit);
                    }
                }
            }
        },

        // ---- 41: level state persists across a round trip --------------------
        41 => match t.phase {
            0 => {
                if refs.current.0 != 0 {
                    return;
                }
                // Leave a marker item + a damaged monster on level0.
                commands.spawn((
                    FloorItem { instance: ItemInstance::new("glow_stone"), pos: GridPos::new(34, 24, 0) },
                    crate::world::LevelScoped,
                ));
                commands.spawn((
                    Monster::new_at("skel_rogue", 7, GridPos::new(32, 24, 0), Facing::North),
                    crate::world::LevelScoped,
                ));
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                // Give the spawns a couple frames, then take the stairs to level1.
                if clock.cycle > t.mark_cycle {
                    player.pos = GridPos::new(25, 22, 0); // level0 stairs
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                }
            }
            2 => {
                if refs.current.0 == 1 {
                    player.pos = GridPos::new(2, 1, 0); // level1 return stairs
                    t.mark_cycle = clock.cycle;
                    t.phase = 3;
                } else if clock.cycle >= t.mark_cycle + 15 {
                    fail(&t, "stairs-state", "level1へ遷移しない".into(), &mut exit);
                }
            }
            _ => {
                if refs.current.0 == 0 {
                    let item_ok = item_count(34, 24, 0, "glow_stone") >= 1;
                    let mon_ok = monsters.iter().any(|(_, m)| m.hp == 7 && m.def_id == "skel_rogue");
                    if item_ok && mon_ok {
                        t.next_step("stairs-state: item + monster state survive the round trip");
                    } else if clock.cycle >= t.mark_cycle + 15 {
                        fail(&t, "stairs-state", format!("item={item_ok} mon={mon_ok}"), &mut exit);
                    }
                } else if clock.cycle >= t.mark_cycle + 15 {
                    fail(&t, "stairs-state", "level0へ戻れない".into(), &mut exit);
                }
            }
        },

        // ---- 42: a hole drops you; a vertical horoscope blocks the wrong way --
        42 => match t.phase {
            0 => {
                player.pos = GridPos::new(26, 21, 1);
                player.facing = Facing::North; // front = the hole (26,20,1)
                script.queue.push_back(Command::Move(Action::Forward));
                script.active = true;
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                if player.pos.floor == 0 {
                    script.active = false;
                    // Forbidden vertical horoscope above a ladder: climb refused.
                    player.pos = GridPos::new(30, 21, 1);
                    script.queue.push_back(Command::ClimbUp);
                    script.active = true;
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                } else if clock.cycle >= t.mark_cycle + 20 {
                    fail(&t, "hole-and-vert", "穴で落ちない".into(), &mut exit);
                }
            }
            2 => {
                if clock.cycle >= t.mark_cycle + 6 {
                    script.active = false;
                    if player.pos.floor != 1 {
                        fail(&t, "hole-and-vert", "禁止方向のホロスコープを登れてしまった".into(), &mut exit);
                        return;
                    }
                    // Allowed vertical horoscope above a ladder: climb succeeds.
                    player.pos = GridPos::new(31, 21, 1);
                    script.queue.push_back(Command::ClimbUp);
                    script.active = true;
                    t.mark_cycle = clock.cycle;
                    t.phase = 3;
                }
            }
            _ => {
                if player.pos.floor == 2 {
                    script.active = false;
                    t.next_step("hole-and-vert: hole drops, vertical horoscope is one-way");
                } else if clock.cycle >= t.mark_cycle + 12 {
                    fail(&t, "hole-and-vert", format!("許可方向を登れない (floor {})", player.pos.floor), &mut exit);
                }
            }
        },

        // ---- 43: writing on a writable wall, then reading it back -------------
        43 => match t.phase {
            0 => {
                // Give a member a pencil, face the writable wall (34,21,0).
                let _ = party.members[0].inventory.pickup(ItemInstance::new("pencil"));
                player.pos = GridPos::new(33, 21, 0);
                player.facing = Facing::East; // front = (34,21,0) WritableWall
                wall_write.send(crate::event::WallWriteRequest { text: "テストのかきこみ".into() });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                // After the write applies, read the wall (Space / 見る).
                if clock.cycle > t.mark_cycle {
                    interact.send(FrontInteract { pos: GridPos::new(34, 21, 0) });
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                }
            }
            _ => {
                if log.contains("テストのかきこみ") {
                    t.next_step("wall-write: pencil writes, 見る reads it back");
                } else if clock.cycle >= t.mark_cycle + 10 {
                    fail(&t, "wall-write", "書いた本文が読めない".into(), &mut exit);
                }
            }
        },

        // ---- 44 (plan10): ChangeBgm overrides the level track -----------------
        44 => match t.phase {
            0 => {
                if refs.bgm.level_track != "bgm_dungeon1.ogg" {
                    fail(&t, "bgm-change", format!("レベルBGMが違う: {:?}", refs.bgm.level_track), &mut exit);
                    return;
                }
                interact.send(FrontInteract { pos: GridPos::new(34, 20, 0) });
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            _ => {
                let want = Some("bgm_battle.ogg".to_string());
                let channel_up = refs.bgm_channels.iter().any(|c| c.track == "bgm_battle.ogg" && c.active);
                if refs.bgm.override_track == want && channel_up {
                    t.next_step("bgm-change: override set, channel crossfading in");
                } else if clock.cycle >= t.mark_cycle + 10 {
                    fail(
                        &t,
                        "bgm-change",
                        format!("override {:?} / channel {channel_up}", refs.bgm.override_track),
                        &mut exit,
                    );
                }
            }
        },

        // ---- 45 (plan10): StartDemo shows the overlay and freezes the clock ---
        45 => match t.phase {
            0 => {
                interact.send(FrontInteract { pos: GridPos::new(34, 23, 0) });
                t.mark_cycle = clock.cycle;
                t.frames = 0;
                t.phase = 1;
            }
            1 => {
                if refs.demo.playing() && !refs.demo_overlays.is_empty() {
                    t.mark_cycle = clock.cycle;
                    t.frames = 0;
                    t.phase = 2;
                } else if clock.cycle >= t.mark_cycle + 10 {
                    fail(&t, "demo-start-skip", "デモが始まらない".into(), &mut exit);
                }
            }
            2 => {
                // 20 frames with the demo up: the cycle clock must not advance.
                t.frames += 1;
                if t.frames >= 20 {
                    if clock.cycle != t.mark_cycle {
                        fail(&t, "demo-start-skip", "デモ中にサイクルが進んだ".into(), &mut exit);
                        return;
                    }
                    // Close it the way Escape+input would (restore BGM, drop overlay).
                    let prev = refs.demo.active.as_ref().and_then(|a| a.prev_override.clone());
                    refs.bgm.override_track = prev;
                    refs.demo.active = None;
                    for e in &refs.demo_overlays {
                        commands.entity(e).despawn_recursive();
                    }
                    t.mark_cycle = clock.cycle;
                    t.phase = 3;
                }
            }
            _ => {
                if clock.cycle > t.mark_cycle {
                    t.next_step("demo-start-skip: overlay up, clock frozen, resumes on close");
                }
            }
        },

        // ---- 46 (plan10): save → mutate → load restores exactly ---------------
        46 => match t.phase {
            0 => {
                data.open = false;
                // No monsters: their AI would draw from the RNG on its own
                // schedule and break the exact-replay comparison.
                for (e, _) in &monsters {
                    commands.entity(e).despawn_recursive();
                }
                player.pos = GridPos::new(26, 19, 0);
                player.facing = Facing::North;
                refs.flags.set(25, true);
                refs.save_req.send(crate::save::SaveRequest(1));
                t.mark_cycle = clock.cycle;
                t.phase = 1;
            }
            1 => {
                // Give the save a frame, then record the post-save RNG draws and
                // wreck everything the load must restore.
                if clock.cycle > t.mark_cycle {
                    t.saved_pos = Some(player.pos);
                    t.hp_before = party.members.iter().map(|m| m.state.hp).collect();
                    t.rng_seq = (0..5).map(|_| refs.rng.below(1000)).collect();
                    player.pos = GridPos::new(27, 19, 0);
                    refs.flags.set(25, false);
                    for m in &mut party.members {
                        m.state.hp = (m.state.hp - 3).max(1);
                    }
                    refs.load_req.send(crate::save::LoadRequest(1));
                    t.mark_cycle = clock.cycle;
                    t.phase = 2;
                }
            }
            _ => {
                if Some(player.pos) == t.saved_pos && refs.flags.get(25) {
                    let hp_now: Vec<i32> = party.members.iter().map(|m| m.state.hp).collect();
                    if hp_now != t.hp_before {
                        fail(&t, "save-load", format!("HP不一致 {hp_now:?} vs {:?}", t.hp_before), &mut exit);
                        return;
                    }
                    let replay: Vec<usize> = (0..5).map(|_| refs.rng.below(1000)).collect();
                    if replay != t.rng_seq {
                        fail(&t, "save-load", "乱数列が一致しない".into(), &mut exit);
                        return;
                    }
                    t.rng_seq.clear();
                    t.next_step("save-load: position/HP/flags/RNG restored exactly");
                } else if clock.cycle >= t.mark_cycle + 20 {
                    fail(
                        &t,
                        "save-load",
                        format!("復元されない pos {:?} flag25 {}", player.pos, refs.flags.get(25)),
                        &mut exit,
                    );
                }
            }
        },

        // ---- 47 (plan10): user_settings.speed scales the cycle clock ----------
        47 => match t.phase {
            0 => {
                refs.settings.speed = 2.0;
                t.mark_cycle = clock.cycle;
                t.frames = 0;
                t.phase = 1;
            }
            1 => {
                t.frames += 1;
                if t.frames >= 30 {
                    t.baseline = (clock.cycle - t.mark_cycle) as i32; // cycles at 2.0×
                    refs.settings.speed = 0.5;
                    t.mark_cycle = clock.cycle;
                    t.frames = 0;
                    t.phase = 2;
                }
            }
            _ => {
                t.frames += 1;
                if t.frames >= 30 {
                    let slow = (clock.cycle - t.mark_cycle) as i32; // cycles at 0.5×
                    refs.settings.speed = 1.0;
                    // 2.0× vs 0.5× is a 4× ratio; assert a comfortable margin.
                    if t.baseline >= slow * 2 {
                        let msg = format!("speed: 2.0x ticks {}, 0.5x ticks {slow}", t.baseline);
                        t.next_step(&msg);
                    } else {
                        fail(&t, "speed", format!("速度倍率が効かない (2x:{} 0.5x:{slow})", t.baseline), &mut exit);
                    }
                }
            }
        },

        _ => {}
    }
}

// ==================================================================== title steps

/// Steps 48–49 (plan11): the ED demo returns to the title with the run reset,
/// and the title's「つづきから」loads a slot. Both drive the *real* input path:
/// key presses are injected into `ButtonInput<KeyCode>` (this system is ordered
/// before `drive_demo` / `drive_title`, so an injected edge is seen the same
/// frame and cleared by bevy_input on the next).
#[allow(clippy::too_many_arguments)]
pub fn run_title(
    mut t: ResMut<AutoTest>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut demo_req: EventWriter<crate::demo::StartDemoReq>,
    demo: Res<crate::demo::DemoState>,
    title: Res<crate::title::TitleState>,
    init: Res<crate::title::InitialRun>,
    flags: Res<crate::event::EventFlags>,
    clock: Res<GameClock>,
    player: Res<Player>,
    party: Res<Party>,
    mut exit: EventWriter<AppExit>,
) {
    if t.step < 48 || t.fatal.is_some() {
        return;
    }
    let fail = |t: &AutoTest, name: &str, why: String, exit: &mut EventWriter<AppExit>| {
        eprintln!("[autotest] FAIL {name}: {why}");
        eprintln!("[autotest] {} step(s) passed before the failure", t.step);
        exit.send(AppExit::error());
    };
    // One key edge per frame: press now; release on the next call so the next
    // press is a fresh `just_pressed`.
    let tap = |keys: &mut ButtonInput<KeyCode>, key: KeyCode| {
        keys.release_all();
        keys.press(key);
    };

    match t.step {
        // ---- 48: the ED demo ends → title opens, run is reset ---------------
        48 => match t.phase {
            0 => {
                demo_req.send(crate::demo::StartDemoReq("ed".to_string()));
                t.frames = 0;
                t.phase = 1;
            }
            1 => {
                t.frames += 1;
                if demo.playing() {
                    tap(&mut keys, KeyCode::Escape); // skip to the END marker
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "ed-to-title", "edデモが始まらない".into(), &mut exit);
                }
            }
            2 => {
                keys.release_all(); // let the Escape edge clear
                t.phase = 3;
            }
            3 => {
                tap(&mut keys, KeyCode::Space); // close the END marker
                t.frames = 0;
                t.phase = 4;
            }
            _ => {
                keys.release_all();
                t.frames += 1;
                if title.active && !demo.playing() {
                    // The run must be back at its authored initial state.
                    if player.pos != init.start {
                        fail(&t, "ed-to-title", format!("開始位置に戻らない {:?}", player.pos), &mut exit);
                        return;
                    }
                    if clock.cycle != 0 {
                        fail(&t, "ed-to-title", format!("クロックが残っている ({})", clock.cycle), &mut exit);
                        return;
                    }
                    if flags.get(25) {
                        fail(&t, "ed-to-title", "フラグがリセットされない".into(), &mut exit);
                        return;
                    }
                    if party.members.len() != init.party.members.len()
                        || party.members.iter().zip(&init.party.members).any(|(a, b)| {
                            a.state.hp != b.state.hp || a.character.stats.level != b.character.stats.level
                        })
                    {
                        fail(&t, "ed-to-title", "パーティが初期状態でない".into(), &mut exit);
                        return;
                    }
                    t.next_step("ed-to-title: ED demo returns to the title, run reset");
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(
                        &t,
                        "ed-to-title",
                        format!("タイトルが開かない (title {} demo {})", title.active, demo.playing()),
                        &mut exit,
                    );
                }
            }
        },

        // ---- 49: title「つづきから」loads the step-46 save -------------------
        49 => match t.phase {
            0 => {
                // Main menu: row 0 = はじめから, row 1 = つづきから.
                if title.rows.is_empty() {
                    return; // drive_title populates on its first frame
                }
                tap(&mut keys, KeyCode::ArrowDown);
                t.frames = 0;
                t.phase = 1;
            }
            1 => {
                keys.release_all();
                if title.sel == 1 {
                    t.phase = 2;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "title-continue", format!("選択が動かない (sel {})", title.sel), &mut exit);
                }
                t.frames += 1;
            }
            2 => {
                tap(&mut keys, KeyCode::Enter); // enter the slot list
                t.phase = 3;
            }
            3 => {
                keys.release_all();
                // Wait for the slot list (rows repopulate a frame after goto).
                if title.screen == crate::title::TitleScreen::Continue && !title.rows.is_empty() {
                    // Slot 1 (saved in step 46) is row 0 and must be enabled.
                    if !title.rows.first().is_some_and(|r| r.enabled) {
                        fail(&t, "title-continue", "スロット1が空き扱い".into(), &mut exit);
                        return;
                    }
                    t.phase = 4;
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(&t, "title-continue", "スロット一覧が開かない".into(), &mut exit);
                }
                t.frames += 1;
            }
            4 => {
                tap(&mut keys, KeyCode::Enter); // load slot 1
                t.frames = 0;
                t.phase = 5;
            }
            _ => {
                keys.release_all();
                t.frames += 1;
                // Step 46 saved at (26,19,0) facing North with flag 25 on.
                if !title.active && player.pos == GridPos::new(26, 19, 0) && flags.get(25) {
                    println!("[autotest] PASS title-continue: つづきから loads the slot");
                    println!("[autotest] ALL PASS (49 steps)");
                    exit.send(AppExit::Success);
                } else if t.frames > STEP_TIMEOUT_FRAMES {
                    fail(
                        &t,
                        "title-continue",
                        format!("ロードされない (title {} pos {:?} flag25 {})", title.active, player.pos, flags.get(25)),
                        &mut exit,
                    );
                }
            }
        },

        _ => {}
    }
}
