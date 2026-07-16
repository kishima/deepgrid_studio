//! Multi-level runtime (plan8): which level is loaded, per-level saved state, and
//! the level-transition machinery (stairs / cross-level warp).
//!
//! Entities that belong to the current level (dungeon mesh, floor items,
//! monsters, gimmick markers) carry [`LevelScoped`] so a transition can despawn
//! them wholesale. Each level's mutable runtime state (monster snapshots, floor
//! items, door states, fired triggers, block diffs) is stashed in [`LevelStates`]
//! so walking back finds the room as you left it. Flags / move-mode / party are
//! global and are **not** saved here.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::config::LimitsConfig;
use crate::dungeon::level::Level;
use crate::dungeon::{Block, DoorStates, Dungeon, Facing, GridPos};
use crate::event::{SpawnAssets, TriggerStates};
use crate::floor_items::{FloorItem, spawn_loose_item};
use crate::item::ItemInstance;
use crate::monster::{Monster, spawn_monster_entity};
use crate::player::{MoveAnim, Player};
use crate::project::LevelData;
use crate::render::{Palette, spawn_level_mesh};

/// The level index currently loaded into the runtime.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct CurrentLevel(pub usize);

/// Marks an entity as belonging to the loaded level — despawned on transition.
#[derive(Component)]
pub struct LevelScoped;

/// Every level's authored data, available at runtime for transitions.
#[derive(Resource, Default)]
pub struct GameLevels {
    pub levels: Vec<LevelData>,
}

/// A frozen monster for the saved level state.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct MonsterSnapshot {
    pub def_id: String,
    pub hp: i32,
    pub pos: GridPos,
    pub facing: Facing,
    pub dead: bool,
    pub dead_cycle: u64,
    pub fleeing: bool,
    pub carry: Vec<String>,
}

/// A saved floor item.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FloorItemSnapshot {
    pub instance: ItemInstance,
    pub pos: GridPos,
}

/// One level's saved runtime state (plan8). Absent = never visited (build fresh).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LevelState {
    pub monsters: Vec<MonsterSnapshot>,
    pub items: Vec<FloorItemSnapshot>,
    pub doors_open: Vec<u8>,
    pub triggers: TriggerStates,
    /// Block overrides applied by SetBlock / SetLiquid, as `((x,y,floor), block)`.
    pub block_diffs: Vec<((i32, i32, usize), Block)>,
}

/// Saved state for every visited level.
#[derive(Resource, Default)]
pub struct LevelStates {
    pub map: HashMap<usize, LevelState>,
}

/// A request to move the party to another level (stairs / cross-level warp).
#[derive(Event, Clone, Copy, Debug)]
pub struct LevelTransition {
    pub to_level: usize,
    pub to: GridPos,
    pub to_facing: Facing,
}

/// Build the initial `DoorStates` for a level: its `open_doors`, unless a saved
/// state overrides them.
pub fn doors_for(level: &LevelData, saved: Option<&LevelState>, kinds: usize) -> DoorStates {
    let open = saved.map(|s| s.doors_open.clone()).unwrap_or_else(|| level.open_doors.clone());
    DoorStates::with_open(kinds, &open)
}

/// Cells where `cur` differs from `orig` (the SetBlock/SetLiquid diffs to save).
fn block_diffs(orig: &Level, cur: &Level) -> Vec<((i32, i32, usize), Block)> {
    let mut out = Vec::new();
    for f in 0..cur.floor_count() {
        let (Some(of), Some(cf)) = (orig.floor(f), cur.floor(f)) else { continue };
        for y in 0..cf.height {
            for x in 0..cf.width {
                let (xi, yi) = (x as i32, y as i32);
                let cb = cf.get(xi, yi);
                if of.get(xi, yi) != cb
                    && let Some(b) = cb
                {
                    out.push(((xi, yi, f), b));
                }
            }
        }
    }
    out
}

/// Freeze the loaded level's runtime state (monsters / floor items / doors /
/// triggers / block diffs). Used on every level transition and by saves
/// (plan10), so both always agree on what "level state" means.
pub fn snapshot_level<'a>(
    orig: &LevelData,
    dungeon: &Dungeon,
    doors: &DoorStates,
    triggers: &TriggerStates,
    kinds: usize,
    monsters: impl IntoIterator<Item = &'a Monster>,
    items: impl IntoIterator<Item = &'a FloorItem>,
) -> LevelState {
    LevelState {
        monsters: monsters
            .into_iter()
            .map(|m| MonsterSnapshot {
                def_id: m.def_id.clone(),
                hp: m.hp,
                pos: m.pos,
                facing: m.facing,
                dead: m.dead,
                dead_cycle: m.dead_cycle,
                fleeing: m.fleeing,
                carry: m.carry.clone(),
            })
            .collect(),
        items: items
            .into_iter()
            .map(|it| FloorItemSnapshot { instance: it.instance.clone(), pos: it.pos })
            .collect(),
        doors_open: (0..kinds).filter(|&k| doors.is_open(k as u8)).map(|k| k as u8).collect(),
        triggers: triggers.clone(),
        block_diffs: block_diffs(&orig.level, &dungeon.level),
    }
}

/// When set, the next `level_transition` skips snapshotting the level being
/// left. Load (plan10) sets this: the pre-load runtime state must not clobber
/// the just-restored `LevelStates`.
#[derive(Resource, Default)]
pub struct SkipNextSnapshot(pub bool);

/// The core level resources a transition rewrites, bundled to fit the parameter
/// limit.
#[derive(SystemParam)]
pub struct LevelRes<'w> {
    pub current: ResMut<'w, CurrentLevel>,
    pub states: ResMut<'w, LevelStates>,
    pub dungeon: ResMut<'w, Dungeon>,
    pub doors: ResMut<'w, DoorStates>,
    pub triggers: ResMut<'w, TriggerStates>,
}

/// Handle a `LevelTransition`: snapshot the level being left, despawn its scoped
/// entities, then load the target (from saved state or fresh) and rebuild mesh /
/// monsters / floor items. Flags / move-mode / party carry over untouched.
#[allow(clippy::too_many_arguments)]
pub fn level_transition(
    mut commands: Commands,
    mut ev: EventReader<LevelTransition>,
    mut lr: LevelRes,
    game_levels: Res<GameLevels>,
    limits: Res<LimitsConfig>,
    mut player: ResMut<Player>,
    mut anim: ResMut<MoveAnim>,
    palette: Option<Res<Palette>>,
    mut assets: SpawnAssets,
    scoped: Query<Entity, With<LevelScoped>>,
    monsters_q: Query<&Monster>,
    items_q: Query<&FloorItem>,
    mut skip_snapshot: ResMut<SkipNextSnapshot>,
) {
    let Some(t) = ev.read().last().copied() else { return };
    let kinds = limits.door_kinds_per_level;

    // --- snapshot the level being left (skipped right after a load, which has
    //     already restored the authoritative LevelStates) ---
    let from = lr.current.0;
    if skip_snapshot.0 {
        skip_snapshot.0 = false;
    } else if let Some(orig) = game_levels.levels.get(from) {
        let st = snapshot_level(orig, &lr.dungeon, &lr.doors, &lr.triggers, kinds, &monsters_q, &items_q);
        lr.states.map.insert(from, st);
    }

    // --- despawn everything scoped to the old level ---
    for e in &scoped {
        commands.entity(e).despawn_recursive();
    }

    // --- load the target level (saved state if visited, else fresh) ---
    let target = t.to_level.min(game_levels.levels.len().saturating_sub(1));
    let Some(target_data) = game_levels.levels.get(target) else { return };
    let saved = lr.states.map.get(&target).cloned();

    let mut level = target_data.level.clone();
    if let Some(s) = &saved {
        for (coord, b) in &s.block_diffs {
            level.set_block(GridPos::new(coord.0, coord.1, coord.2), *b);
        }
    }
    *lr.dungeon = Dungeon { level, start_pos: t.to, start_facing: t.to_facing };
    *lr.doors = doors_for(target_data, saved.as_ref(), kinds);
    *lr.triggers = saved.as_ref().map(|s| s.triggers.clone()).unwrap_or_default();
    lr.current.0 = target;

    // Rebuild mesh.
    if let Some(palette) = &palette {
        spawn_level_mesh(
            &mut commands,
            palette,
            &mut assets.meshes,
            &mut assets.materials,
            &lr.dungeon.level,
        );
    }

    // Rebuild monsters + floor items.
    match &saved {
        Some(s) => {
            for ms in &s.monsters {
                if let Some(def) = assets.monster_catalog.get(&ms.def_id) {
                    let e = spawn_monster_entity(&mut commands, &assets.asset_server, def, ms.pos, ms.facing);
                    let mut m = Monster::new_at(ms.def_id.clone(), ms.hp, ms.pos, ms.facing).with_carry(ms.carry.clone());
                    m.dead = ms.dead;
                    m.dead_cycle = ms.dead_cycle;
                    m.fleeing = ms.fleeing;
                    commands.entity(e).insert(m);
                }
            }
            for it in &s.items {
                if let Some(def) = assets.item_catalog.get(&it.instance.def_id).cloned() {
                    spawn_loose_item(&mut commands, &assets.asset_server, &mut assets.meshes, &mut assets.materials, &def, it.instance.clone(), it.pos);
                }
            }
        }
        None => {
            for p in &target_data.monsters {
                if let Some(def) = assets.monster_catalog.get(&p.id) {
                    spawn_monster_entity(&mut commands, &assets.asset_server, def, GridPos::new(p.x, p.y, p.floor), p.facing);
                }
            }
            for p in &target_data.items {
                if let Some(def) = assets.item_catalog.get(&p.id).cloned() {
                    spawn_loose_item(&mut commands, &assets.asset_server, &mut assets.meshes, &mut assets.materials, &def, ItemInstance::new(p.id.clone()), GridPos::new(p.x, p.y, p.floor));
                }
            }
        }
    }

    // Move the party and reset any in-flight animation (camera snaps next frame).
    player.pos = t.to;
    player.facing = t.to_facing;
    anim.reset();
}
