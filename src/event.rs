//! Events & gimmicks (plan8): the data model (`EventDef` and its trigger /
//! condition / action pieces), the runtime flag store + delayed execution queue,
//! and the systems that fire triggers and run actions.
//!
//! Everything is `CycleTick`-driven so behaviour is frame-rate independent and
//! deterministic. Flags and the move mode are global; per-level state (which
//! one-shot triggers have fired, block diffs, …) is snapshotted by `world.rs`
//! when levels swap. Numeric ranges (64 flags, 0–63 delay) are reference values
//! (project.md「上限値の扱い」); the types are wide and `limits.event_flags`
//! sizes the flag store.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::character::Party;
use crate::clock::{CycleTick, GameClock};
use crate::dungeon::{Block, DoorStates, Dungeon, Facing, GridPos};
use crate::floor_items::{FloorItem, spawn_loose_item};
use crate::hud::MessageLog;
use crate::item::{ItemCatalog, ItemInstance};
use crate::monster::{Monster, MonsterCatalog, spawn_monster_entity};
use crate::player::Player;
use crate::render::TileDirty;
use crate::world::{CurrentLevel, GameLevels, LevelTransition};

// ------------------------------------------------------------------ data model

/// What makes an event fire. Trigger blocks (`Keyhole`/`Switch`/`FloorPlate`/
/// `WarpPoint`) are matched to events by the `at` coordinate.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// Pressed (Space, facing) when the party carries `key_item` (not consumed).
    Keyhole { key_item: String },
    /// A switch that fires once and stays on.
    SwitchOneWay,
    /// A switch that fires on every ON/OFF flip.
    SwitchToggle,
    /// A momentary switch: fires each press, holds no state.
    SwitchPush,
    /// A pressure plate: fires when the party enters the tile and `cond` holds.
    FloorPlate { cond: PlateCond },
    /// Fires on entering the tile; `hidden` hides its 3D marker.
    WarpPoint { hidden: bool },
    /// No self-trigger — fired only by another event's `OperateSwitch`.
    None,
}

/// Floor-plate firing condition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum PlateCond {
    /// Any entry.
    Step,
    /// The party's combined carried weight (×100 g) is at least `min_x100g`.
    Weight { min_x100g: i32 },
    /// An item (a specific id, or `None` = anything) is placed on the tile.
    ItemPlaced { item: Option<String> },
}

/// One flag reference in an event's condition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FlagCond {
    pub flag: usize,
    pub must_be_on: bool,
}

/// How multiple `FlagCond`s combine.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlagJoin {
    #[default]
    And,
    Or,
}

/// A liquid kind for water-level events.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidKind {
    Water,
    Fire,
    Poison,
}

impl LiquidKind {
    pub fn block(self) -> Block {
        match self {
            LiquidKind::Water => Block::Water,
            LiquidKind::Fire => Block::Fire,
            LiquidKind::Poison => Block::Poison,
        }
    }
}

/// Party movement mode (an `EventAction` sets it; play-mode reads it).
#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MoveMode {
    /// Normal grid movement with footing / falling.
    #[default]
    Normal,
    /// Free flight: ignore footing (no falling; move between floors freely).
    Free,
    /// Movement commands are refused (cutscene lock).
    Locked,
}

/// One action in an event's action list. BGM / demo are stubs until plan10.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum EventAction {
    Warp { level: usize, x: i32, y: i32, floor: usize, facing: Facing },
    /// Set (or clear, `kind: None`) the liquid at a cell.
    SetLiquid { x: i32, y: i32, floor: usize, kind: Option<LiquidKind> },
    ChangeBgm { bgm: String },
    /// Revive every downed member and refill HP/MP (original 準拠).
    ReviveParty,
    SpawnMonster { monster: String, x: i32, y: i32, floor: usize },
    SpawnItem { item: String, x: i32, y: i32, floor: usize },
    SetBlock { x: i32, y: i32, floor: usize, block: Block },
    /// Open or close a door kind remotely (the 14-event「ドアの開閉」).
    SetDoor { kind: u8, open: bool },
    StartDemo { demo: String },
    SetMoveMode { mode: MoveMode },
    /// Force another event's switch on/off (fires it if turning on).
    OperateSwitch { event: String, on: bool },
    SetFlag { flag: usize, on: bool },
    EndChain,
    /// Restart this event's chain after re-applying its delay (min 1 cycle).
    Loop,
}

/// One authored event (`LevelData.events`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EventDef {
    pub id: String,
    pub trigger: TriggerKind,
    /// The trigger block's coordinate (x, y, floor).
    pub at: (i32, i32, usize),
    #[serde(default)]
    pub delay_cycles: u64,
    #[serde(default)]
    pub flags: Vec<FlagCond>,
    #[serde(default)]
    pub join: FlagJoin,
    pub actions: Vec<EventAction>,
}

// ------------------------------------------------------------------ runtime resources

/// Global event flags (sized to `limits.event_flags`). Global by design — flags
/// persist across level transitions (plan8).
#[derive(Resource, Default)]
pub struct EventFlags {
    bits: Vec<bool>,
}

impl EventFlags {
    pub fn new(count: usize) -> Self {
        Self { bits: vec![false; count] }
    }
    pub fn get(&self, i: usize) -> bool {
        self.bits.get(i).copied().unwrap_or(false)
    }
    pub fn set(&mut self, i: usize, on: bool) {
        if let Some(b) = self.bits.get_mut(i) {
            *b = on;
        }
    }
}

/// Evaluate an event's flag condition. Empty conditions always hold.
pub fn flags_satisfied(flags: &EventFlags, conds: &[FlagCond], join: FlagJoin) -> bool {
    if conds.is_empty() {
        return true;
    }
    match join {
        FlagJoin::And => conds.iter().all(|c| flags.get(c.flag) == c.must_be_on),
        FlagJoin::Or => conds.iter().any(|c| flags.get(c.flag) == c.must_be_on),
    }
}

/// A scheduled event run: execute `event_id`'s actions once `fire_cycle` is
/// reached, if the player is still on `level`.
#[derive(Clone, Debug)]
pub struct QueuedEvent {
    pub event_id: String,
    pub level: usize,
    pub fire_cycle: u64,
}

/// The delayed-execution queue (`CycleTick`-driven).
#[derive(Resource, Default)]
pub struct EventQueue {
    pub pending: Vec<QueuedEvent>,
}

/// Per-level trigger state: which one-shot triggers have fired, and toggle
/// switch positions. Snapshotted into `LevelState` on transition.
#[derive(Resource, Default, Clone)]
pub struct TriggerStates {
    /// Event ids that have fired and must not fire again (OneWay / Keyhole).
    pub fired: std::collections::HashSet<String>,
    /// Toggle switch positions by event id.
    pub toggled_on: std::collections::HashMap<String, bool>,
    /// Floor-plate "was satisfied last check" edge state (fire on rising edge).
    pub plate_armed: std::collections::HashMap<String, bool>,
}

// ------------------------------------------------------------------ trigger events

/// A "press the block ahead" request (Space / Command::Interact), carrying the
/// front cell. `front_interact` resolves it to keyhole / switch / writable wall.
#[derive(Event, Clone, Copy)]
pub struct FrontInteract {
    pub pos: GridPos,
}

/// Push an event onto the delayed queue if its flag condition holds. Returns
/// whether it was enqueued.
fn enqueue(queue: &mut EventQueue, ev: &EventDef, level: usize, now: u64, flags: &EventFlags) -> bool {
    if !flags_satisfied(flags, &ev.flags, ev.join) {
        return false;
    }
    queue.pending.push(QueuedEvent {
        event_id: ev.id.clone(),
        level,
        fire_cycle: now + ev.delay_cycles,
    });
    info!("event queued: {} (delay {})", ev.id, ev.delay_cycles);
    true
}

// ------------------------------------------------------------------ front interact

/// Resolve `FrontInteract` presses: read a writable wall, or fire the keyhole /
/// switch event at the faced cell.
#[allow(clippy::too_many_arguments)]
pub fn front_interact(
    mut reader: EventReader<FrontInteract>,
    game_levels: Res<GameLevels>,
    current: Res<CurrentLevel>,
    party: Res<Party>,
    item_catalog: Res<ItemCatalog>,
    flags: Res<EventFlags>,
    mut triggers: ResMut<TriggerStates>,
    mut queue: ResMut<EventQueue>,
    clock: Res<GameClock>,
    mut log: ResMut<MessageLog>,
) {
    let level = current.0;
    let Some(ld) = game_levels.levels.get(level) else { return };
    for fi in reader.read() {
        let (px, py, pf) = (fi.pos.x, fi.pos.y, fi.pos.floor);
        // Writable wall: show its text.
        if matches!(ld.level.block_at(fi.pos), Some(Block::WritableWall)) {
            match ld.wall_text_at(px, py, pf) {
                Some(t) => log.push(t.to_string()),
                None => log.push("壁に 何か 書かれているが 読めない"),
            }
        }
        for ev in ld.events.iter().filter(|e| e.at == (px, py, pf)) {
            match &ev.trigger {
                TriggerKind::Keyhole { key_item } => {
                    if triggers.fired.contains(&ev.id) {
                        continue;
                    }
                    let has_key = party.members.iter().any(|m| {
                        m.inventory.iter().any(|(_, it)| &it.def_id == key_item)
                    });
                    let _ = &item_catalog;
                    if !has_key {
                        log.push("かぎあなが ある。あう鍵がない");
                    } else if enqueue(&mut queue, ev, level, clock.cycle, &flags) {
                        triggers.fired.insert(ev.id.clone());
                        log.push("かぎを つかった。");
                    }
                }
                TriggerKind::SwitchOneWay => {
                    if !triggers.fired.contains(&ev.id)
                        && enqueue(&mut queue, ev, level, clock.cycle, &flags)
                    {
                        triggers.fired.insert(ev.id.clone());
                        log.push("スイッチを おした。");
                    }
                }
                TriggerKind::SwitchToggle => {
                    let on = !triggers.toggled_on.get(&ev.id).copied().unwrap_or(false);
                    triggers.toggled_on.insert(ev.id.clone(), on);
                    if enqueue(&mut queue, ev, level, clock.cycle, &flags) {
                        log.push(if on { "スイッチ ON" } else { "スイッチ OFF" });
                    }
                }
                TriggerKind::SwitchPush => {
                    let fired = enqueue(&mut queue, ev, level, clock.cycle, &flags);
                    if fired {
                        log.push("スイッチを おした。");
                    }
                }
                _ => {}
            }
        }
    }
}

// ------------------------------------------------------------------ entry triggers

/// Detect the party entering a tile: stairs → level transition; warp points and
/// floor plates → fire their events. Floor plates fire on the rising edge of
/// their condition so they re-arm.
#[allow(clippy::too_many_arguments)]
pub fn entry_triggers(
    player: Res<Player>,
    mut last: Local<Option<GridPos>>,
    game_levels: Res<GameLevels>,
    current: Res<CurrentLevel>,
    party: Res<Party>,
    item_catalog: Res<ItemCatalog>,
    flags: Res<EventFlags>,
    mut triggers: ResMut<TriggerStates>,
    mut queue: ResMut<EventQueue>,
    clock: Res<GameClock>,
    floor_items: Query<&FloorItem>,
    mut transition: EventWriter<LevelTransition>,
    mut log: ResMut<MessageLog>,
) {
    let pos = player.pos;
    let entered = *last != Some(pos);
    *last = Some(pos);
    let level = current.0;
    let Some(ld) = game_levels.levels.get(level) else { return };

    // Stairs: entering triggers a linked level transition.
    if entered && matches!(ld.level.block_at(pos), Some(Block::Stairs { .. })) {
        if let Some(link) = ld.stairs_link_at(pos.x, pos.y, pos.floor) {
            transition.send(LevelTransition {
                to_level: link.to_level,
                to: GridPos::new(link.to.0, link.to.1, link.to.2),
                to_facing: link.to_facing,
            });
            let up = matches!(ld.level.block_at(pos), Some(Block::Stairs { up: true }));
            log.push(if up { "階段を のぼった。" } else { "階段を おりた。" });
        } else {
            log.push("くずれていて 通れない");
        }
    }

    let party_weight: i32 = party.members.iter().map(|m| m.inventory.total_weight(&item_catalog)).sum();
    for ev in &ld.events {
        match &ev.trigger {
            TriggerKind::WarpPoint { .. } => {
                if entered && ev.at == (pos.x, pos.y, pos.floor) {
                    enqueue(&mut queue, ev, level, clock.cycle, &flags);
                }
            }
            TriggerKind::FloorPlate { cond } => {
                let on_tile = ev.at == (pos.x, pos.y, pos.floor);
                let satisfied = match cond {
                    PlateCond::Step => on_tile,
                    PlateCond::Weight { min_x100g } => on_tile && party_weight >= *min_x100g,
                    PlateCond::ItemPlaced { item } => floor_items.iter().any(|it| {
                        it.pos == GridPos::new(ev.at.0, ev.at.1, ev.at.2)
                            && item.as_ref().is_none_or(|id| &it.instance.def_id == id)
                    }),
                };
                let was = triggers.plate_armed.get(&ev.id).copied().unwrap_or(false);
                if satisfied && !was {
                    enqueue(&mut queue, ev, level, clock.cycle, &flags);
                }
                triggers.plate_armed.insert(ev.id.clone(), satisfied);
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ debug scenes

/// Debug-shot driver for the `plate` / `warp` / `stairs` / `hole` scenes: once
/// the world settles, teleport the party to a viewpoint on the autotest fixtures
/// so the gimmick is on screen (plan8). Cosmetic; gated on `DEEPGRID_DEBUG_SHOT`.
pub fn debug_gimmick_driver(
    mut done: Local<bool>,
    script: Res<crate::player::ScriptedInput>,
    anim: Res<crate::player::MoveAnim>,
    mut player: ResMut<Player>,
) {
    let Some(scene) = crate::debug_shot::debug_shot_value() else { return };
    if *done {
        return;
    }
    if !matches!(scene.as_str(), "plate" | "warp" | "stairs" | "hole") {
        *done = true;
        return;
    }
    if !(script.queue.is_empty() && anim.is_idle()) {
        return;
    }
    match scene.as_str() {
        // Step on the plate (fires the SpawnMonster) and look at the spawn tile.
        "plate" => {
            player.pos = GridPos::new(27, 20, 0);
            player.facing = Facing::East;
        }
        // Look at the visible warp marker two tiles ahead.
        "warp" => {
            player.pos = GridPos::new(30, 21, 0);
            player.facing = Facing::North;
        }
        // Look toward the stairs marker.
        "stairs" => {
            player.pos = GridPos::new(27, 22, 0);
            player.facing = Facing::West;
        }
        // Stand at the hole's edge on floor 1, looking into it.
        "hole" => {
            player.pos = GridPos::new(26, 21, 1);
            player.facing = Facing::North;
        }
        _ => {}
    }
    *done = true;
}

// ------------------------------------------------------------------ execution

/// Resources SetBlock/SetLiquid/SpawnMonster/SpawnItem mutate, bundled to keep
/// the executor within Bevy's parameter limit.
#[derive(SystemParam)]
pub struct EventWorld<'w> {
    pub dungeon: ResMut<'w, Dungeon>,
    pub doors: ResMut<'w, DoorStates>,
    pub move_mode: ResMut<'w, MoveMode>,
    pub flags: ResMut<'w, EventFlags>,
    pub triggers: ResMut<'w, TriggerStates>,
}

#[derive(SystemParam)]
pub struct SpawnAssets<'w> {
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub asset_server: Res<'w, AssetServer>,
    pub monster_catalog: Res<'w, MonsterCatalog>,
    pub item_catalog: Res<'w, ItemCatalog>,
}

/// Run every queued event whose delay has elapsed on the current level. Actions
/// run in one cycle; `Loop` re-schedules after ≥1 cycle, `EndChain` stops.
#[allow(clippy::too_many_arguments)]
pub fn run_events(
    mut commands: Commands,
    mut ticks: EventReader<CycleTick>,
    clock: Res<GameClock>,
    mut queue: ResMut<EventQueue>,
    game_levels: Res<GameLevels>,
    current: Res<CurrentLevel>,
    mut party: ResMut<Party>,
    mut player: ResMut<Player>,
    mut anim: ResMut<crate::player::MoveAnim>,
    mut log: ResMut<MessageLog>,
    mut world: EventWorld,
    mut assets: SpawnAssets,
    mut tile_dirty: EventWriter<TileDirty>,
    mut transition: EventWriter<LevelTransition>,
    mut monsters: Query<(Entity, &mut Monster)>,
) {
    if ticks.read().count() == 0 {
        return;
    }
    let now = clock.cycle;
    let level = current.0;
    // Take the entries ready to run this cycle on this level.
    let ready: Vec<QueuedEvent> = queue
        .pending
        .iter()
        .filter(|q| q.level == level && q.fire_cycle <= now)
        .cloned()
        .collect();
    queue.pending.retain(|q| !(q.level == level && q.fire_cycle <= now));

    let Some(ld) = game_levels.levels.get(level) else { return };
    for q in ready {
        let Some(ev) = ld.events.iter().find(|e| e.id == q.event_id).cloned() else { continue };
        info!("event fired: {}", ev.id);
        for action in &ev.actions {
            match action {
                EventAction::SetFlag { flag, on } => world.flags.set(*flag, *on),
                EventAction::SetDoor { kind, open } => world.doors.set(*kind, *open),
                EventAction::SetMoveMode { mode } => *world.move_mode = *mode,
                EventAction::ReviveParty => {
                    for m in &mut party.members {
                        let full = m.character.stats.clone();
                        m.state.hp = full.max_hp;
                        m.state.mp = full.max_mp;
                        m.state.down = false;
                    }
                    log.push("パーティは 復活した!");
                }
                EventAction::ChangeBgm { bgm } => log.push(format!("♪ BGMが かわった: {bgm} (未実装)")),
                EventAction::StartDemo { demo } => log.push(format!("デモ「{demo}」を さいせい (未実装)")),
                EventAction::Warp { level: tl, x, y, floor, facing } => {
                    if *tl == level {
                        player.pos = GridPos::new(*x, *y, *floor);
                        player.facing = *facing;
                        anim.reset(); // snap the camera to the warp (plan8)
                    } else {
                        transition.send(LevelTransition {
                            to_level: *tl,
                            to: GridPos::new(*x, *y, *floor),
                            to_facing: *facing,
                        });
                    }
                }
                EventAction::SetLiquid { x, y, floor, kind } => {
                    let b = kind.map(|k| k.block()).unwrap_or(Block::Empty);
                    world.dungeon.level.set_block(GridPos::new(*x, *y, *floor), b);
                    tile_dirty.send(TileDirty { x: *x, y: *y, floor: *floor });
                }
                EventAction::SetBlock { x, y, floor, block } => {
                    let pos = GridPos::new(*x, *y, *floor);
                    world.dungeon.level.set_block(pos, *block);
                    tile_dirty.send(TileDirty { x: *x, y: *y, floor: *floor });
                    // Push anyone standing where a solid block appeared up a floor.
                    if block.is_solid() {
                        let top = world.dungeon.level.floor_count();
                        if player.pos == pos && pos.floor + 1 < top {
                            player.pos.floor += 1;
                        }
                        for (_, mut mon) in &mut monsters {
                            if !mon.dead && mon.pos == pos && pos.floor + 1 < top {
                                mon.pos.floor += 1;
                            }
                        }
                    }
                }
                EventAction::SpawnMonster { monster, x, y, floor } => {
                    if let Some(def) = assets.monster_catalog.get(monster) {
                        spawn_monster_entity(&mut commands, &assets.asset_server, def, GridPos::new(*x, *y, *floor), Facing::North);
                    }
                }
                EventAction::SpawnItem { item, x, y, floor } => {
                    if let Some(def) = assets.item_catalog.get(item).cloned() {
                        spawn_loose_item(
                            &mut commands, &assets.asset_server, &mut assets.meshes, &mut assets.materials,
                            &def, ItemInstance::new(item.clone()), GridPos::new(*x, *y, *floor),
                        );
                    }
                }
                EventAction::OperateSwitch { event, on } => {
                    if *on
                        && let Some(target) = ld.events.iter().find(|e| &e.id == event)
                    {
                        enqueue(&mut queue, target, level, now, &world.flags);
                    }
                    world.triggers.toggled_on.insert(event.clone(), *on);
                }
                EventAction::EndChain => break,
                EventAction::Loop => {
                    queue.pending.push(QueuedEvent {
                        event_id: ev.id.clone(),
                        level,
                        fire_cycle: now + ev.delay_cycles.max(1),
                    });
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_condition_always_holds() {
        let flags = EventFlags::new(64);
        assert!(flags_satisfied(&flags, &[], FlagJoin::And));
        assert!(flags_satisfied(&flags, &[], FlagJoin::Or));
    }

    #[test]
    fn and_or_evaluation() {
        let mut flags = EventFlags::new(64);
        flags.set(1, true);
        let c1 = FlagCond { flag: 1, must_be_on: true };
        let c2 = FlagCond { flag: 2, must_be_on: true };
        // AND: needs both; flag 2 is off → false.
        assert!(!flags_satisfied(&flags, &[c1.clone(), c2.clone()], FlagJoin::And));
        // OR: flag 1 is on → true.
        assert!(flags_satisfied(&flags, &[c1.clone(), c2.clone()], FlagJoin::Or));
        // must_be_on:false matches an off flag.
        let c3 = FlagCond { flag: 2, must_be_on: false };
        assert!(flags_satisfied(&flags, &[c1, c3], FlagJoin::And));
    }

    #[test]
    fn flags_clamp_out_of_range() {
        let mut flags = EventFlags::new(4);
        flags.set(100, true); // no-op, no panic
        assert!(!flags.get(100));
    }
}
