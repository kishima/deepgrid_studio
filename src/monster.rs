//! Monsters (plan6): data model, data-driven 3D display with a name-based
//! animation state machine, cycle-driven wander/chase/flee AI, and the real-time
//! combat systems (player attack / guard / concentrate / throw / steal, monster
//! attacks, death / drop / exp / regen). Combat *math* lives in `combat.rs`; this
//! module drives it against live entities.
//!
//! All AI and combat run off `CycleTick`, so behaviour is frame-rate independent.
//! Randomness comes from the shared [`GameRng`] so runs are reproducible.

use std::collections::{HashMap, HashSet};

use bevy::gltf::Gltf;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::character::{Party, StatKind};
use crate::clock::{CycleTick, GameClock};
use crate::combat;
use crate::dungeon::{Block, DoorStates, Dungeon, Facing, GridPos};
use crate::floor_items::spawn_loose_item;
use crate::hud::MessageLog;
use crate::item::{ItemCatalog, ItemInstance, SlotRef};
use crate::player::Player;
use crate::render::BLOCK_SIZE;
use crate::rng::GameRng;

// ------------------------------------------------------------------ data model

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveType {
    Ground,
    Air,
    None,
}

fn default_255() -> u32 {
    255
}
fn default_true() -> bool {
    true
}
fn default_facing() -> Facing {
    Facing::North
}

/// glTF animation clip **names** (robust to pack re-indexing). KayKit skeletons
/// have e.g. "Idle" "Walking_A" "1H_Melee_Attack_Slice_Diagonal" "Hit_A"
/// "Death_A" (assets/models/README.md).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonsterAnims {
    pub idle: String,
    pub walk: String,
    pub attack: String,
    pub hit: String,
    pub death: String,
}

/// A monster definition (`monsters.ron`). Original ranges (defense 0..32767, …)
/// are reference values and unenforced.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonsterDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub profile: String,
    pub max_hp: i32,
    pub attack: i32,
    #[serde(default)]
    pub defense: i32,
    #[serde(default)]
    pub agility: i32,
    /// Attack interval in cycles; 255 = never attacks (original convention).
    #[serde(default = "default_255")]
    pub attack_freq: u32,
    #[serde(default)]
    pub anti_magic: i32,
    #[serde(default)]
    pub body_temp: i32,
    #[serde(default)]
    pub resist_air: i32,
    #[serde(default)]
    pub resist_water: i32,
    #[serde(default)]
    pub resist_heat: i32,
    #[serde(default)]
    pub resist_poison: i32,
    #[serde(default)]
    pub wariness: i32,
    #[serde(default)]
    pub regen_cycles: u64,
    pub move_type: MoveType,
    #[serde(default)]
    pub can_use_ladder: bool,
    #[serde(default = "default_true")]
    pub fits_narrow: bool,
    #[serde(default)]
    pub sight: i32,
    /// Move interval in cycles; 255 = never moves.
    #[serde(default = "default_255")]
    pub action_unit: u32,
    #[serde(default)]
    pub flee_hp: i32,
    #[serde(default)]
    pub large: bool,
    #[serde(default)]
    pub carry_items: Vec<String>,
    #[serde(default)]
    pub attack_items: Vec<String>,
    /// Experience awarded on kill (provisional field; editor in plan9).
    #[serde(default)]
    pub exp: i32,
    pub model: String,
    pub anim: MonsterAnims,
}

impl MonsterDef {
    /// The resistance value guarding a liquid tile (≥128 means it likes it).
    fn liquid_resist(&self, block: Block) -> Option<i32> {
        match block {
            Block::Water => Some(self.resist_water),
            Block::Fire => Some(self.resist_heat),
            Block::Poison => Some(self.resist_poison),
            _ => None,
        }
    }
}

/// One placed monster (project format v4).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MonsterPlacement {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub floor: usize,
    #[serde(default = "default_facing")]
    pub facing: Facing,
}

/// All monster definitions, keyed by id (Bevy resource).
#[derive(Resource, Default)]
pub struct MonsterCatalog {
    defs: HashMap<String, MonsterDef>,
}

impl MonsterCatalog {
    pub fn from_defs(defs: Vec<MonsterDef>, what: &str) -> Result<Self, String> {
        let mut map = HashMap::with_capacity(defs.len());
        for def in defs {
            if map.contains_key(&def.id) {
                return Err(format!("{what}: duplicate monster id '{}'", def.id));
            }
            map.insert(def.id.clone(), def);
        }
        Ok(Self { defs: map })
    }

    pub fn get(&self, id: &str) -> Option<&MonsterDef> {
        self.defs.get(id)
    }
}

/// Monster placements to spawn at startup (inserted by `main`).
#[derive(Resource, Default)]
pub struct InitialMonsters(pub Vec<MonsterPlacement>);

/// Fires when the party is wiped out (all members down). Play locks up (revive is
/// plan7/8) unless `DEEPGRID_REVIVE=1` restores them for the autotest.
#[derive(Resource, Default)]
pub struct PartyWiped(pub bool);

// ------------------------------------------------------------------ components

/// Which animation a monster is currently showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnimKind {
    Idle,
    Walk,
    Attack,
    Hit,
    Death,
}

/// The live state of one monster entity. The def is referenced by id.
#[derive(Component)]
pub struct Monster {
    pub def_id: String,
    pub hp: i32,
    pub pos: GridPos,
    pub facing: Facing,
    /// Next cycle this monster may move / act.
    pub next_action: u64,
    /// Next cycle this monster may attack.
    pub next_attack: u64,
    pub dead: bool,
    pub dead_cycle: u64,
    pub fleeing: bool,
    /// Remaining carry items (dropped on death / removed on steal).
    pub carry: Vec<String>,
    pub anim: AnimKind,
    /// Seconds to hold a one-shot animation (attack/hit) before reverting.
    pub anim_hold: f32,
    pub moved_this_cycle: bool,
}

impl Monster {
    /// A live monster at `pos` (no carry items). Used to spawn placed monsters
    /// and by the autotest to inject combat subjects.
    pub fn new_at(def_id: impl Into<String>, hp: i32, pos: GridPos, facing: Facing) -> Self {
        Self {
            def_id: def_id.into(),
            hp,
            pos,
            facing,
            next_action: 0,
            next_attack: 0,
            dead: false,
            dead_cycle: 0,
            fleeing: false,
            carry: Vec::new(),
            anim: AnimKind::Idle,
            anim_hold: 0.0,
            moved_this_cycle: false,
        }
    }

    /// The tiles this monster occupies (2×2 for `large`, else 1).
    fn footprint(&self, large: bool) -> Vec<GridPos> {
        if large {
            let mut v = Vec::with_capacity(4);
            for dy in 0..2 {
                for dx in 0..2 {
                    v.push(GridPos::new(self.pos.x + dx, self.pos.y + dy, self.pos.floor));
                }
            }
            v
        } else {
            vec![self.pos]
        }
    }
}

/// Animation binding for a monster's glTF: the graph + resolved clip nodes.
#[derive(Component)]
pub struct MonsterGraph {
    gltf: Handle<Gltf>,
    anims: MonsterAnims,
    graph: Option<Handle<AnimationGraph>>,
    nodes: HashMap<u8, AnimationNodeIndex>,
    player: Option<Entity>,
    playing: Option<AnimKind>,
}

fn anim_key(k: AnimKind) -> u8 {
    match k {
        AnimKind::Idle => 0,
        AnimKind::Walk => 1,
        AnimKind::Attack => 2,
        AnimKind::Hit => 3,
        AnimKind::Death => 4,
    }
}

/// Marks an `AnimationPlayer` already wired to its monster graph.
#[derive(Component)]
pub struct AnimBound;

// ------------------------------------------------------------------ spawning

fn tile_center(pos: GridPos) -> Vec3 {
    Vec3::new(
        pos.x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        pos.floor as f32 * BLOCK_SIZE,
        pos.y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
    )
}

/// KayKit humanoids are ~1.7 units tall; scale to sit inside a cell.
const MONSTER_SCALE: f32 = 0.45;
/// Smooth-move speed in tiles/sec for a monster's visual interpolation.
const MONSTER_MOVE_SPEED: f32 = 4.0;

pub fn setup_monsters(
    mut commands: Commands,
    initial: Res<InitialMonsters>,
    catalog: Res<MonsterCatalog>,
    asset_server: Res<AssetServer>,
) {
    for p in &initial.0 {
        let Some(def) = catalog.get(&p.id) else {
            continue;
        };
        let pos = GridPos::new(p.x, p.y, p.floor);
        let transform = Transform::from_translation(tile_center(pos))
            .with_rotation(Quat::from_rotation_y(model_yaw(p.facing)))
            .with_scale(Vec3::splat(MONSTER_SCALE));
        commands.spawn((
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(def.model.clone()))),
            transform,
            Monster {
                def_id: def.id.clone(),
                hp: def.max_hp,
                pos,
                facing: p.facing,
                next_action: 0,
                next_attack: 0,
                dead: false,
                dead_cycle: 0,
                fleeing: false,
                carry: def.carry_items.clone(),
                anim: AnimKind::Idle,
                anim_hold: 0.0,
                moved_this_cycle: false,
            },
            MonsterGraph {
                gltf: asset_server.load(def.model.clone()),
                anims: def.anim.clone(),
                graph: None,
                nodes: HashMap::new(),
                player: None,
                playing: None,
            },
        ));
    }
}

// -------------------------------------------------------------- animation wiring

/// Once a monster's glTF is loaded, resolve its clip names into an
/// `AnimationGraph` (missing names fall back to idle / the first clip).
pub fn build_monster_graphs(
    mut graphs: Query<&mut MonsterGraph>,
    gltfs: Res<Assets<Gltf>>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
) {
    for mut mg in &mut graphs {
        if mg.graph.is_some() {
            continue;
        }
        let Some(gltf) = gltfs.get(&mg.gltf) else {
            continue;
        };
        let clip = |name: &str| -> Option<Handle<AnimationClip>> {
            gltf.named_animations
                .get(name)
                .cloned()
                .or_else(|| gltf.animations.first().cloned())
        };
        let mut graph = AnimationGraph::new();
        let anims = mg.anims.clone();
        let specs = [
            (AnimKind::Idle, &anims.idle),
            (AnimKind::Walk, &anims.walk),
            (AnimKind::Attack, &anims.attack),
            (AnimKind::Hit, &anims.hit),
            (AnimKind::Death, &anims.death),
        ];
        let mut nodes = HashMap::new();
        for (kind, name) in specs {
            if let Some(handle) = clip(name) {
                let node = graph.add_clip(handle, 1.0, graph.root);
                nodes.insert(anim_key(kind), node);
            }
        }
        mg.graph = Some(graph_assets.add(graph));
        mg.nodes = nodes;
    }
}

/// Wire each monster's spawned `AnimationPlayer` to its graph and start idle.
pub fn bind_monster_players(
    mut commands: Commands,
    players: Query<Entity, (With<AnimationPlayer>, Without<AnimBound>)>,
    parents: Query<&Parent>,
    mut graphs: Query<&mut MonsterGraph>,
) {
    for player_entity in &players {
        // Walk up to the monster root carrying a (ready) graph.
        let mut current = player_entity;
        loop {
            if let Ok(mut mg) = graphs.get_mut(current) {
                let Some(graph) = mg.graph.clone() else {
                    break; // graph not built yet; try again next frame
                };
                commands
                    .entity(player_entity)
                    .insert((AnimationGraphHandle(graph), AnimBound));
                mg.player = Some(player_entity);
                break;
            }
            match parents.get(current) {
                Ok(p) => current = p.get(),
                Err(_) => break,
            }
        }
    }
}

/// Play the animation matching each monster's current `AnimKind`.
pub fn drive_monster_anim(
    time: Res<Time>,
    mut monsters: Query<(&mut Monster, &mut MonsterGraph)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (mut m, mut mg) in &mut monsters {
        // One-shot animations (attack/hit) revert once their hold expires.
        if m.anim_hold > 0.0 {
            m.anim_hold -= time.delta_secs();
            if m.anim_hold <= 0.0 && !m.dead {
                m.anim = if m.moved_this_cycle { AnimKind::Walk } else { AnimKind::Idle };
            }
        }
        let Some(player_entity) = mg.player else {
            continue;
        };
        if mg.playing == Some(m.anim) {
            continue;
        }
        // Fall back to idle if the requested clip wasn't in the glTF.
        let key = anim_key(m.anim);
        let node = mg
            .nodes
            .get(&key)
            .or_else(|| mg.nodes.get(&anim_key(AnimKind::Idle)))
            .copied();
        let Some(node) = node else { continue };
        if let Ok(mut player) = players.get_mut(player_entity) {
            let looping = matches!(m.anim, AnimKind::Idle | AnimKind::Walk);
            player.stop_all();
            let active = player.play(node);
            if looping {
                active.repeat();
            }
            mg.playing = Some(m.anim);
        }
    }
}

/// Visual yaw for a monster model looking along `facing`.
///
/// `Facing::yaw()` is the *camera* convention (forward = -Z, so North = 0), but
/// KayKit models are authored facing **+Z**; applying the camera yaw directly
/// turns the mesh 180° away from its walk direction (the "crab/moon-walk" the
/// user reported 2026-07-14). Offsetting by π points the visual front along the
/// facing for all four directions.
fn model_yaw(facing: Facing) -> f32 {
    facing.yaw() + std::f32::consts::PI
}

/// Smoothly slide each monster's transform toward its logical tile + facing.
pub fn interpolate_monsters(time: Res<Time>, mut monsters: Query<(&Monster, &mut Transform)>) {
    for (m, mut tf) in &mut monsters {
        let target = tile_center(m.pos);
        let delta = target - tf.translation;
        let step = MONSTER_MOVE_SPEED * BLOCK_SIZE * time.delta_secs();
        if delta.length() <= step {
            tf.translation = target;
        } else {
            tf.translation += delta.normalize() * step;
        }
        let target_yaw = Quat::from_rotation_y(model_yaw(m.facing));
        tf.rotation = tf.rotation.slerp(target_yaw, (step * 4.0).min(1.0));
        tf.scale = Vec3::splat(MONSTER_SCALE);
    }
}

// ------------------------------------------------------------------ occupancy

/// Tiles occupied by living monsters (updated each frame for movement + the
/// Space-attack multiplex).
#[derive(Resource, Default)]
pub struct MonsterOccupancy {
    tiles: HashSet<GridPos>,
}

impl MonsterOccupancy {
    pub fn contains(&self, pos: GridPos) -> bool {
        self.tiles.contains(&pos)
    }
}

pub fn update_occupancy(
    mut occ: ResMut<MonsterOccupancy>,
    catalog: Res<MonsterCatalog>,
    monsters: Query<&Monster>,
) {
    occ.tiles.clear();
    for m in &monsters {
        if m.dead {
            continue;
        }
        let large = catalog.get(&m.def_id).is_some_and(|d| d.large);
        for tile in m.footprint(large) {
            occ.tiles.insert(tile);
        }
    }
}

// ------------------------------------------------------------------ line of sight

fn chebyshev(a: GridPos, b: GridPos) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// The cardinal facing that points from `from` most directly toward `to`
/// (ties prefer the horizontal axis).
pub(crate) fn facing_toward(from: GridPos, to: GridPos) -> Facing {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx.abs() >= dy.abs() {
        if dx >= 0 { Facing::East } else { Facing::West }
    } else if dy >= 0 {
        Facing::South
    } else {
        Facing::North
    }
}

/// Straight-line visibility on one floor (Bresenham; blocked by walls / shut
/// doors). Endpoints themselves are not required to be open.
fn line_of_sight(level: &crate::dungeon::level::Level, doors: &DoorStates, a: GridPos, b: GridPos) -> bool {
    if a.floor != b.floor {
        return false;
    }
    let (mut x0, mut y0) = (a.x, a.y);
    let (x1, y1) = (b.x, b.y);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if (x0, y0) != (a.x, a.y) && (x0, y0) != (x1, y1) {
            let p = GridPos::new(x0, y0, a.floor);
            let blocks = match level.block_at(p) {
                Some(Block::Wall) => true,
                Some(Block::Door { kind }) => !doors.is_open(kind),
                _ => false,
            };
            if blocks {
                return false;
            }
        }
        if (x0, y0) == (x1, y1) {
            return true;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Whether a living monster currently sees the party (updated each frame). The
/// data screen reads this to block / interrupt ZZZ resting.
#[derive(Resource, Default)]
pub struct EnemyNear(pub bool);

pub fn update_enemy_near(
    mut near: ResMut<EnemyNear>,
    player: Res<Player>,
    monsters: Query<&Monster>,
    catalog: Res<MonsterCatalog>,
    dungeon: Res<Dungeon>,
    doors: Res<DoorStates>,
) {
    near.0 = any_monster_sees(player.pos, &monsters, &catalog, &dungeon, &doors);
}

/// Whether any living monster can see `pos` (within its sight and line of sight).
/// Used to block ZZZ resting near enemies.
pub fn any_monster_sees(
    pos: GridPos,
    monsters: &Query<&Monster>,
    catalog: &MonsterCatalog,
    dungeon: &Dungeon,
    doors: &DoorStates,
) -> bool {
    monsters.iter().any(|m| {
        if m.dead {
            return false;
        }
        let Some(def) = catalog.get(&m.def_id) else {
            return false;
        };
        def.sight > 0
            && chebyshev(m.pos, pos) <= def.sight
            && line_of_sight(&dungeon.level, doors, m.pos, pos)
    })
}

// ------------------------------------------------------------------ passability

#[allow(clippy::too_many_arguments)]
fn can_enter(
    def: &MonsterDef,
    level: &crate::dungeon::level::Level,
    dest: GridPos,
    doors: &DoorStates,
    occ: &MonsterOccupancy,
    player: GridPos,
    self_pos: GridPos,
) -> bool {
    if dest == player || (occ.contains(dest) && dest != self_pos) {
        return false;
    }
    let Some(block) = level.block_at(dest) else {
        return false;
    };
    match block {
        Block::Wall => return false,
        Block::Door { kind }
            if !doors.is_open(kind) => {
                return false;
            }
        _ => {}
    }
    // Liquid tiles: refuse unless the matching resistance is ≥ 128.
    if let Some(r) = def.liquid_resist(block)
        && r < 128 {
            return false;
        }
    // Narrow-corridor rule: some monsters won't enter a tile pinched by walls.
    if !def.fits_narrow {
        let wall = |p: GridPos| matches!(level.block_at(p), Some(Block::Wall) | None);
        let ew = wall(GridPos::new(dest.x - 1, dest.y, dest.floor))
            || wall(GridPos::new(dest.x + 1, dest.y, dest.floor));
        let ns = wall(GridPos::new(dest.x, dest.y - 1, dest.floor))
            || wall(GridPos::new(dest.x, dest.y + 1, dest.floor));
        if ew || ns {
            return false;
        }
    }
    // Footing: ground monsters must be supported; air ignores it; none never moves.
    match def.move_type {
        MoveType::None => false,
        MoveType::Air => true,
        MoveType::Ground => level.is_supported(dest.x, dest.y, dest.floor),
    }
}

// ------------------------------------------------------------------ AI

/// Cycle-driven monster movement AI: wander / chase / flee.
#[allow(clippy::too_many_arguments)]
pub fn monster_ai(
    mut ticks: EventReader<CycleTick>,
    clock: Res<GameClock>,
    catalog: Res<MonsterCatalog>,
    dungeon: Res<Dungeon>,
    doors: Res<DoorStates>,
    occ: Res<MonsterOccupancy>,
    player: Res<Player>,
    mut rng: ResMut<GameRng>,
    mut monsters: Query<&mut Monster>,
) {
    if ticks.read().count() == 0 {
        return;
    }
    let cycle = clock.cycle;
    for mut m in &mut monsters {
        m.moved_this_cycle = false;
        if m.dead {
            continue;
        }
        let Some(def) = catalog.get(&m.def_id) else {
            continue;
        };
        if def.action_unit >= 255 || def.move_type == MoveType::None {
            continue;
        }
        if cycle < m.next_action {
            continue;
        }
        m.next_action = cycle + def.action_unit as u64;

        let sees = def.sight > 0
            && chebyshev(m.pos, player.pos) <= def.sight
            && line_of_sight(&dungeon.level, &doors, m.pos, player.pos);
        if m.hp < def.flee_hp {
            m.fleeing = true;
        }

        // Decide a desired step direction.
        let dirs = [Facing::North, Facing::East, Facing::South, Facing::West];
        let step = if m.fleeing && sees {
            // Move to maximise distance from the player.
            dirs.into_iter()
                .filter(|d| {
                    let (dx, dy) = d.delta();
                    can_enter(def, &dungeon.level, GridPos::new(m.pos.x + dx, m.pos.y + dy, m.pos.floor), &doors, &occ, player.pos, m.pos)
                })
                .max_by_key(|d| {
                    let (dx, dy) = d.delta();
                    chebyshev(GridPos::new(m.pos.x + dx, m.pos.y + dy, m.pos.floor), player.pos)
                })
        } else if sees {
            // Greedy chase: prefer the larger axis, fall back to the other.
            let dx = player.pos.x - m.pos.x;
            let dy = player.pos.y - m.pos.y;
            let horiz = if dx > 0 { Some(Facing::East) } else if dx < 0 { Some(Facing::West) } else { None };
            let vert = if dy > 0 { Some(Facing::South) } else if dy < 0 { Some(Facing::North) } else { None };
            let order = if dx.abs() >= dy.abs() { [horiz, vert] } else { [vert, horiz] };
            order.into_iter().flatten().find(|d| {
                let (ddx, ddy) = d.delta();
                can_enter(def, &dungeon.level, GridPos::new(m.pos.x + ddx, m.pos.y + ddy, m.pos.floor), &doors, &occ, player.pos, m.pos)
            })
        } else if rng.coin() {
            // Wander: 50% chance to try a random open neighbour.
            let mut open: Vec<Facing> = dirs
                .into_iter()
                .filter(|d| {
                    let (dx, dy) = d.delta();
                    can_enter(def, &dungeon.level, GridPos::new(m.pos.x + dx, m.pos.y + dy, m.pos.floor), &doors, &occ, player.pos, m.pos)
                })
                .collect();
            if open.is_empty() { None } else { Some(open.remove(rng.below(open.len()))) }
        } else {
            None
        };

        if let Some(dir) = step {
            let (dx, dy) = dir.delta();
            m.pos = GridPos::new(m.pos.x + dx, m.pos.y + dy, m.pos.floor);
            m.facing = dir;
            m.moved_this_cycle = true;
            if m.anim_hold <= 0.0 {
                m.anim = AnimKind::Walk;
            }
        } else if m.anim_hold <= 0.0 && !m.dead {
            m.anim = AnimKind::Idle;
        }
        // Always face the player when adjacent, sight or no sight — being in
        // melee range is impossible to miss, and attacks/counterattacks should
        // visually point at their target (user feedback 2026-07-14).
        if chebyshev(m.pos, player.pos) == 1 && m.pos.floor == player.pos.floor {
            m.facing = facing_toward(m.pos, player.pos);
        }
    }
}

// ------------------------------------------------------------------ combat: helpers

/// Index of the next living, non-down party member to act, rotating.
#[derive(Resource, Default)]
pub struct AttackRotation {
    next: usize,
}

fn next_actor(party: &Party, rot: &mut AttackRotation) -> Option<usize> {
    let n = party.members.len();
    for _ in 0..n {
        let i = rot.next % n.max(1);
        rot.next = rot.next.wrapping_add(1);
        if party.members.get(i).is_some_and(|m| !m.state.down) {
            return Some(i);
        }
    }
    None
}

/// The living monster whose (single-tile) position is `pos`, if any.
pub(crate) fn monster_at(
    pos: GridPos,
    monsters: &Query<(Entity, &mut Monster)>,
    catalog: &MonsterCatalog,
) -> Option<Entity> {
    monsters.iter().find_map(|(e, m)| {
        if m.dead {
            return None;
        }
        let large = catalog.get(&m.def_id).is_some_and(|d| d.large);
        m.footprint(large).into_iter().any(|t| t == pos).then_some(e)
    })
}

// ------------------------------------------------------------------ combat: player

/// The player's discrete combat actions (from keys / icons / scripts).
#[derive(Event, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerAction {
    Attack,
    Guard,
    Concentrate,
    Throw,
    Steal,
}

/// Which party slot is selected (shared with the data screen) — the thrower /
/// thief. Defined in game_state; re-exported access via that module.
use crate::game_state::SelectedMember;

#[allow(clippy::too_many_arguments)]
pub fn player_actions(
    mut commands: Commands,
    mut actions: EventReader<PlayerAction>,
    mut party: ResMut<Party>,
    item_catalog: Res<ItemCatalog>,
    catalog: Res<MonsterCatalog>,
    clock: Res<GameClock>,
    selected: Res<SelectedMember>,
    player: Res<Player>,
    dungeon: Res<Dungeon>,
    mut rng: ResMut<GameRng>,
    mut rot: ResMut<AttackRotation>,
    mut log: ResMut<MessageLog>,
    mut monsters: Query<(Entity, &mut Monster)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let (fdx, fdy) = player.facing.delta();
    let front = GridPos::new(player.pos.x + fdx, player.pos.y + fdy, player.pos.floor);

    for action in actions.read() {
        match action {
            PlayerAction::Guard => {
                for m in &mut party.members {
                    if !m.state.down {
                        m.state.guarding = true;
                    }
                }
                log.push("身がまえた");
            }
            PlayerAction::Concentrate => {
                for m in &mut party.members {
                    if !m.state.down {
                        m.state.concentrating = true;
                    }
                }
                log.push("精神を統一している");
            }
            PlayerAction::Attack => {
                let Some(target) = monster_at(front, &monsters, &catalog) else {
                    log.push("目の前に敵はいない");
                    continue;
                };
                let Some(ai) = next_actor(&party, &mut rot) else {
                    continue;
                };
                let attacker = &mut party.members[ai];
                attacker.state.concentrating = false;
                let eff = attacker.effective_stats(&item_catalog);
                // Weapon in a hand (left preferred), else unarmed.
                let (sharp, grip) = weapon_stats(attacker, &item_catalog);
                let a_name = attacker.character.first_name.clone();
                let a_agi = eff.get(StatKind::Agility);
                let a_atk = eff.get(StatKind::Attack);
                let cur_conc = attacker.state.concentration;
                let max_conc = eff.get(StatKind::Concentration).max(1);
                attacker.state.concentration = 0;

                let (_, mut mon) = monsters.get_mut(target).unwrap();
                let mdef = catalog.get(&mon.def_id).cloned();
                let Some(mdef) = mdef else { continue };
                let hit = combat::hit_chance(a_agi, mdef.agility, grip);
                if !rng.chance(hit) {
                    log.push(format!("{a_name}の こうげき! ミス!"));
                    continue;
                }
                let dmg = combat::final_damage(sharp, a_atk, mdef.defense, cur_conc, max_conc);
                mon.hp -= dmg;
                mon.anim = AnimKind::Hit;
                mon.anim_hold = 0.4;
                // Whirl to face whoever just hit it.
                mon.facing = facing_toward(mon.pos, player.pos);
                log.push(format!("{a_name}の こうげき! {}に {dmg}のダメージ", mdef.name));
                if mon.hp <= 0 {
                    kill_monster(
                        &mut commands, &mut mon, &mdef, clock.cycle, &mut party, &mut log,
                        &mut meshes, &mut materials, &asset_server, &item_catalog,
                    );
                }
            }
            PlayerAction::Throw => {
                throw_item(
                    &mut commands, &mut party, selected.index, &item_catalog, &catalog,
                    &player, &dungeon, clock.cycle, &mut rng, &mut log, &mut monsters,
                    &mut meshes, &mut materials, &asset_server,
                );
            }
            PlayerAction::Steal => {
                steal_from(
                    front, &mut party, selected.index, &item_catalog, &catalog, clock.cycle,
                    &mut rng, &mut log, &mut monsters,
                );
            }
        }
    }
}

/// Sharpness + grip of the member's held weapon (left hand preferred), or unarmed.
fn weapon_stats(member: &crate::character::PartyMember, items: &ItemCatalog) -> (i32, i32) {
    for slot in [SlotRef::Hand(0), SlotRef::Hand(1)] {
        if let Some(it) = member.inventory.get(slot)
            && let Some(def) = items.get(&it.def_id)
            && def.sharpness > 0
        {
            return (def.sharpness, def.grip.max(1));
        }
    }
    (combat::UNARMED_SHARPNESS, combat::UNARMED_GRIP)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn kill_monster(
    commands: &mut Commands,
    mon: &mut Monster,
    def: &MonsterDef,
    cycle: u64,
    party: &mut Party,
    log: &mut MessageLog,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    items: &ItemCatalog,
) {
    mon.hp = 0;
    mon.dead = true;
    mon.dead_cycle = cycle;
    mon.anim = AnimKind::Death;
    mon.anim_hold = 0.0;
    // Scatter remaining carry items at the monster's feet.
    for id in mon.carry.drain(..) {
        if let Some(idef) = items.get(&id) {
            spawn_loose_item(commands, asset_server, meshes, materials, idef, ItemInstance::new(id.clone()), mon.pos);
        }
    }
    // Award experience, split (rounded up) among the living members.
    let alive: Vec<usize> = party
        .members
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.state.down)
        .map(|(i, _)| i)
        .collect();
    let share = if alive.is_empty() { 0 } else { (def.exp + alive.len() as i32 - 1) / alive.len() as i32 };
    log.push(format!("{}を たおした! (経験値 {})", def.name, def.exp));
    for i in alive {
        for msg in party.members[i].gain_exp(share) {
            log.push(msg);
        }
    }
}

// ------------------------------------------------------------------ combat: throw

#[allow(clippy::too_many_arguments)]
fn throw_item(
    commands: &mut Commands,
    party: &mut Party,
    idx: usize,
    items: &ItemCatalog,
    catalog: &MonsterCatalog,
    player: &Player,
    dungeon: &Dungeon,
    cycle: u64,
    _rng: &mut GameRng,
    log: &mut MessageLog,
    monsters: &mut Query<(Entity, &mut Monster)>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
) {
    let Some(member) = party.members.get_mut(idx) else { return };
    // Held item, left hand preferred.
    let slot = [SlotRef::Hand(0), SlotRef::Hand(1)]
        .into_iter()
        .find(|s| member.inventory.get(*s).is_some());
    let Some(slot) = slot else {
        log.push("投げるものが無い");
        return;
    };
    let instance = member.inventory.get(slot).cloned().unwrap();
    let Some(idef) = items.get(&instance.def_id).cloned() else { return };
    if idef.important {
        log.push(format!("{}は 投げられない", idef.name));
        return;
    }
    let eff = member.effective_stats(items);
    let range = combat::throw_range(eff.get(StatKind::Throwing), idef.throwability);
    let dmg = combat::throw_damage(idef.sharpness, idef.throwability);
    let name = member.character.first_name.clone();
    member.inventory.take(slot);

    // Fly straight until a monster or wall; land on the last open tile.
    let (dx, dy) = player.facing.delta();
    let mut land = player.pos;
    let mut hit_entity = None;
    for step in 1..=range {
        let p = GridPos::new(player.pos.x + dx * step, player.pos.y + dy * step, player.pos.floor);
        if let Some(e) = monster_at(p, monsters, catalog) {
            hit_entity = Some(e);
            land = GridPos::new(player.pos.x + dx * (step - 1).max(0), player.pos.y + dy * (step - 1).max(0), player.pos.floor);
            break;
        }
        match dungeon.level.block_at(p) {
            Some(Block::Wall) | None => break,
            Some(Block::Door { kind }) if !crate::dungeon::DoorStates::default().is_open(kind) => break,
            _ => land = p,
        }
    }
    log.push(format!("{name}は {}を 投げた", idef.name));
    if let Some(e) = hit_entity
        && let Ok((_, mut mon)) = monsters.get_mut(e) {
            let mdef = catalog.get(&mon.def_id).cloned();
            mon.hp -= dmg;
            mon.anim = AnimKind::Hit;
            mon.anim_hold = 0.4;
            // Face the thrower.
            mon.facing = facing_toward(mon.pos, player.pos);
            if let Some(mdef) = mdef {
                log.push(format!("{}に {dmg}のダメージ", mdef.name));
                if mon.hp <= 0 {
                    kill_monster(commands, &mut mon, &mdef, cycle, party, log, meshes, materials, asset_server, items);
                }
            }
        }
    // The thrown item lands on the floor.
    spawn_loose_item(commands, asset_server, meshes, materials, &idef, instance, land);
}

// ------------------------------------------------------------------ combat: steal

#[allow(clippy::too_many_arguments)]
fn steal_from(
    front: GridPos,
    party: &mut Party,
    idx: usize,
    items: &ItemCatalog,
    catalog: &MonsterCatalog,
    cycle: u64,
    rng: &mut GameRng,
    log: &mut MessageLog,
    monsters: &mut Query<(Entity, &mut Monster)>,
) {
    let Some(target) = monster_at(front, monsters, catalog) else {
        log.push("目の前に敵はいない");
        return;
    };
    let Some(member) = party.members.get(idx) else { return };
    let stealing = member.effective_stats(items).get(StatKind::Stealing);
    let name = member.character.first_name.clone();
    let (_, mut mon) = monsters.get_mut(target).unwrap();
    let Some(mdef) = catalog.get(&mon.def_id).cloned() else { return };
    let chance = combat::steal_chance(stealing, mdef.wariness);
    if rng.chance(chance) && !mon.carry.is_empty() {
        let take = mon.carry.remove(0);
        let item_name = items.get(&take).map(|d| d.name.clone()).unwrap_or_else(|| take.clone());
        let member = &mut party.members[idx];
        if member.inventory.pickup(ItemInstance::new(take)).is_ok() {
            log.push(format!("{name}は {item_name}を ぬすんだ!"));
        } else {
            log.push(format!("{name}は 盗もうとしたが 持ちきれない"));
        }
    } else {
        log.push(format!("{name}は 盗みに失敗した!"));
        // Immediate counterattack (ignores attack_freq).
        mon.next_attack = cycle; // let the attack system fire next cycle
        monster_hit_party(&mdef, party, rng, log);
    }
}

// ------------------------------------------------------------------ combat: monster attacks

/// Deal one monster attack to a random living member (guard halves).
fn monster_hit_party(def: &MonsterDef, party: &mut Party, rng: &mut GameRng, log: &mut MessageLog) {
    let alive: Vec<usize> = party
        .members
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.state.down)
        .map(|(i, _)| i)
        .collect();
    if alive.is_empty() {
        return;
    }
    let victim = alive[rng.below(alive.len())];
    let target = &mut party.members[victim];
    let d_agi = target.character.stats.agility;
    let hit = combat::hit_chance(def.agility, d_agi, combat::UNARMED_GRIP);
    let vname = target.character.first_name.clone();
    if !rng.chance(hit) {
        log.push(format!("{}の こうげき! {vname}は かわした", def.name));
        return;
    }
    let mut dmg = combat::final_damage(0, def.attack, target.character.stats.defense, 0, 1);
    if target.state.guarding {
        dmg = combat::guarded(dmg, true);
        target.state.guarding = false;
    }
    target.state.concentrating = false;
    target.state.hp = (target.state.hp - dmg).max(0);
    log.push(format!("{}の こうげき! {vname}に {dmg}のダメージ", def.name));
    if target.state.hp == 0 {
        target.state.down = true;
        log.push(format!("{vname}は 気絶した!"));
    }
}

/// Adjacent monsters attack the party on their `attack_freq` cadence.
#[allow(clippy::too_many_arguments)]
pub fn monster_attacks(
    mut ticks: EventReader<CycleTick>,
    clock: Res<GameClock>,
    catalog: Res<MonsterCatalog>,
    player: Res<Player>,
    mut rng: ResMut<GameRng>,
    mut party: ResMut<Party>,
    mut log: ResMut<MessageLog>,
    mut monsters: Query<&mut Monster>,
) {
    if ticks.read().count() == 0 {
        return;
    }
    let cycle = clock.cycle;
    for mut m in &mut monsters {
        if m.dead {
            continue;
        }
        let Some(def) = catalog.get(&m.def_id).cloned() else { continue };
        if def.attack_freq >= 255 || m.fleeing {
            continue;
        }
        // Must be on the same floor and orthogonally/diagonally adjacent
        // (chebyshev ignores floor, so gate on it explicitly).
        if m.pos.floor != player.pos.floor
            || chebyshev(m.pos, player.pos) != 1
            || cycle < m.next_attack
        {
            continue;
        }
        m.next_attack = cycle + def.attack_freq as u64;
        m.anim = AnimKind::Attack;
        m.anim_hold = 0.4;
        monster_hit_party(&def, &mut party, &mut rng, &mut log);
    }
}

// ------------------------------------------------------------------ death / regen / wipe

/// Regenerate or despawn dead monsters; detect a party wipe.
#[allow(clippy::too_many_arguments)]
pub fn monster_lifecycle(
    mut commands: Commands,
    mut ticks: EventReader<CycleTick>,
    clock: Res<GameClock>,
    catalog: Res<MonsterCatalog>,
    player: Res<Player>,
    mut party: ResMut<Party>,
    mut wiped: ResMut<PartyWiped>,
    mut log: ResMut<MessageLog>,
    mut monsters: Query<(Entity, &mut Monster)>,
) {
    if ticks.read().count() == 0 {
        return;
    }
    let cycle = clock.cycle;
    for (entity, mut m) in &mut monsters {
        if !m.dead {
            continue;
        }
        let Some(def) = catalog.get(&m.def_id) else { continue };
        if def.regen_cycles == 0 {
            commands.entity(entity).despawn_recursive();
        } else if cycle >= m.dead_cycle + def.regen_cycles && m.pos != player.pos {
            m.dead = false;
            m.hp = def.max_hp;
            m.fleeing = false;
            m.carry = def.carry_items.clone();
            m.anim = AnimKind::Idle;
            log.push(format!("{}が よみがえった!", def.name));
        }
    }

    // Party-wipe check.
    if !party.is_empty() && party.members.iter().all(|m| m.state.down) && !wiped.0 {
        if std::env::var("DEEPGRID_REVIVE").is_ok_and(|v| !v.is_empty()) {
            for m in &mut party.members {
                let full = m.character.stats.clone();
                m.state.hp = full.max_hp;
                m.state.mp = full.max_mp;
                m.state.down = false;
            }
            log.push("パーティは復活した(REVIVE)");
        } else {
            wiped.0 = true;
            log.push("全滅した…");
        }
    }
}
