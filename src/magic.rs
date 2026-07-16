//! Magic (plan7): the data model (`MagicDef` / `MagicCatalog`), the casting
//! pipeline (`CastMagic` event → `cast_magic` system), ally-effect application,
//! attack spells with flying light-bullets, the player-light boost of the
//! lighting spells, and the potion (liquefy / drink) helpers.
//!
//! Effects reuse the existing [`ActiveEffect`] framework (never touching base
//! stats) so equipment / eaten-food / spell buffs all compose the same way and a
//! future save (plan10) only has to persist `CharacterState`. Randomness (the
//! anti-magic resistance roll) goes through the shared [`GameRng`] so the
//! autotest stays deterministic. Provisional numbers (range, light multipliers,
//! resistance formula) are constants here — plan9 may move them to `RulesConfig`.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::character::{ActiveEffect, Party, PartyMember, StatKind};
use crate::clock::{CycleTick, GameClock};
use crate::combat;
use crate::dungeon::{Block, DoorStates, Dungeon, GridPos};
use crate::game_state::DataScreen;
use crate::hud::MessageLog;
use crate::item::{ItemCatalog, ItemDef, ItemInstance, ItemKind, SlotRef};
use crate::monster::{AnimKind, Monster, MonsterCatalog};
use crate::player::Player;
use crate::render::BLOCK_SIZE;
use crate::rng::GameRng;
use crate::rules::{HungerRules, RulesConfig};

// ------------------------------------------------------------------ data model

/// Magic kind. The original has 27 (dandan_spec); the four whose breakdown is
/// unknown are added to this enum once identified (updating plan7.md's status
/// table). An unknown variant in data is a hard load error — data is hand-written
/// until the plan9 editor, so a typo should stop the build, not silently drop.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MagicKind {
    /// Ability change (StatKind, 15 kinds).
    StatChange(StatKind),
    /// HP change (栄養価 slot). With hunger enabled it also moves satiety.
    HpChange,
    MpChange,
    /// Resurrect a downed member to `ratio_percent`% of max HP (33 | 50 | 100).
    Revive { ratio_percent: u8 },
    /// Brighten the player light: 1 = weak, 2 = medium, 3 = strong.
    Light { strength: u8 },
}

/// A magic definition (`magics.ron`; immutable, authored data). Original numeric
/// ranges (MP 0..3000 etc.) are reference values and are not enforced.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MagicDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub mp_cost: i32,
    /// 必要魔法知識 (magic knowledge needed to learn / cast).
    pub difficulty: i32,
    pub kind: MagicKind,
    /// Change value (StatChange / HpChange / MpChange; attack magic uses |value|).
    #[serde(default)]
    pub value: i32,
    /// Duration in cycles; 0 = permanent (ignored by instant kinds).
    #[serde(default)]
    pub duration_cycles: u64,
    #[serde(default)]
    pub liquefiable: bool,
    /// Light-bullet count 0..2; > 0 marks an enemy-targeted attack spell.
    #[serde(default)]
    pub projectiles: u8,
    /// 1–2 character symbol shown in the magic list (graphic swap is plan10).
    #[serde(default)]
    pub symbol: String,
}

impl MagicDef {
    /// Whether this spell targets the enemy ahead (an attack spell).
    pub fn is_attack(&self) -> bool {
        self.projectiles > 0
    }
}

/// All magic definitions, keyed by id (Bevy resource). Empty for a pre-v5 project.
#[derive(Resource, Default)]
pub struct MagicCatalog {
    defs: HashMap<String, MagicDef>,
}

impl MagicCatalog {
    /// Build from a list, rejecting duplicate ids (and warning on the
    /// liquefiable-only-for-non-Light/Revive data rule).
    pub fn from_defs(defs: Vec<MagicDef>, what: &str) -> Result<Self, String> {
        let mut map = HashMap::with_capacity(defs.len());
        for def in defs {
            if def.liquefiable && matches!(def.kind, MagicKind::Light { .. } | MagicKind::Revive { .. }) {
                eprintln!(
                    "deepgrid_studio: {what}: magic '{}' is Light/Revive but liquefiable — \
                     potions of it are disallowed at cast time",
                    def.id
                );
            }
            if map.contains_key(&def.id) {
                return Err(format!("{what}: duplicate magic id '{}'", def.id));
            }
            map.insert(def.id.clone(), def);
        }
        Ok(Self { defs: map })
    }

    pub fn get(&self, id: &str) -> Option<&MagicDef> {
        self.defs.get(id)
    }

    /// All definitions, map order (UIs sort by id for determinism).
    pub fn iter(&self) -> impl Iterator<Item = &MagicDef> {
        self.defs.values()
    }
}

// ------------------------------------------------------------------ light boost

/// Base player-light values (the fixed numbers `setup_player` used before plan7).
pub const BASE_LIGHT_INTENSITY: f32 = 120_000.0;
pub const BASE_LIGHT_RANGE: f32 = 22.0;
/// Attack-magic reach in tiles (provisional; plan9 → RulesConfig).
pub const MAGIC_RANGE: i32 = 8;

/// Multiplier a lighting spell of `strength` applies to the player light
/// (provisional: 弱×1.5 / 中×2.5 / 強×4.0).
fn light_multiplier(strength: u8) -> f32 {
    match strength {
        1 => 1.5,
        2 => 2.5,
        3 => 4.0,
        _ => 1.0,
    }
}

/// The active lighting-spell boost on the player light. `multiplier` scales both
/// intensity and range; it decays each cycle and reverts to 1.0 when `remaining`
/// reaches 0.
#[derive(Resource)]
pub struct LightBoost {
    pub multiplier: f32,
    pub remaining: u64,
}

impl Default for LightBoost {
    fn default() -> Self {
        Self { multiplier: 1.0, remaining: 0 }
    }
}

/// Marks the player's follow light so the boost system (and the autotest) can
/// find it among any other point lights.
#[derive(Component)]
pub struct PlayerLight;

/// Decay the light boost each cycle and drive the player light's intensity/range.
pub fn drive_player_light(
    mut ticks: EventReader<CycleTick>,
    mut boost: ResMut<LightBoost>,
    mut log: ResMut<MessageLog>,
    mut lights: Query<&mut PointLight, With<PlayerLight>>,
) {
    let cycles = ticks.read().count() as u64;
    if cycles > 0 && boost.remaining > 0 {
        boost.remaining = boost.remaining.saturating_sub(cycles);
        if boost.remaining == 0 {
            boost.multiplier = 1.0;
            log.push("あかりが きえていく…");
        }
    }
    for mut light in &mut lights {
        light.intensity = BASE_LIGHT_INTENSITY * boost.multiplier;
        light.range = BASE_LIGHT_RANGE * boost.multiplier;
    }
}

// ------------------------------------------------------------------ projectiles

/// A cosmetic light-bullet: lerps from `from` to `to` over `dur` seconds after
/// `delay`, then despawns. Purely visual — spell damage is resolved instantly in
/// `cast_magic` so the autotest stays deterministic.
#[derive(Component)]
pub struct MagicProjectile {
    from: Vec3,
    to: Vec3,
    elapsed: f32,
    delay: f32,
    dur: f32,
}

/// Fly and expire light-bullets (real-time; cycle-independent).
pub fn animate_projectiles(
    time: Res<Time>,
    mut commands: Commands,
    mut se: EventWriter<crate::audio::PlaySe>,
    mut projectiles: Query<(Entity, &mut MagicProjectile, &mut Transform)>,
) {
    for (e, mut p, mut tf) in &mut projectiles {
        p.elapsed += time.delta_secs();
        let t = ((p.elapsed - p.delay) / p.dur).clamp(0.0, 1.0);
        tf.translation = p.from.lerp(p.to, t);
        if p.elapsed - p.delay >= p.dur {
            se.send(crate::audio::PlaySe(crate::audio::Se::Impact));
            commands.entity(e).despawn();
        }
    }
}

/// Eye-height centre of a tile (light-bullets travel at roughly eye level).
fn shot_point(pos: GridPos) -> Vec3 {
    Vec3::new(
        pos.x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        pos.floor as f32 * BLOCK_SIZE + 0.5,
        pos.y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
    )
}

/// Resources needed to spawn floor items / light-bullets, bundled to keep the
/// cast system within Bevy's 16-parameter limit.
#[derive(SystemParam)]
pub struct SpawnGfx<'w> {
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub asset_server: Res<'w, AssetServer>,
    /// Sound-effect requests (plan10: cast chime).
    pub se: EventWriter<'w, crate::audio::PlaySe>,
}

// ------------------------------------------------------------------ ally effects

/// Apply a non-combat spell (HP / MP / stat / — Light/Revive handled elsewhere)
/// to `member`, reusing the [`ActiveEffect`] framework. Recasting the same magic
/// id resets its duration instead of stacking. Returns a log fragment naming the
/// target and outcome.
pub fn apply_ally_magic(
    member: &mut PartyMember,
    def: &MagicDef,
    item_catalog: &ItemCatalog,
    hunger: &HungerRules,
) -> String {
    let tname = member.character.first_name.clone();
    match &def.kind {
        MagicKind::HpChange => {
            let max_hp = member.effective_stats(item_catalog).get(StatKind::MaxHp).max(0);
            member.state.hp = (member.state.hp + def.value).clamp(0, max_hp);
            if hunger.enabled {
                member.state.satiety = (member.state.satiety
                    + def.value * hunger.satiety_per_nutrition)
                    .clamp(0, hunger.satiety_max);
            }
            if member.state.hp == 0 {
                member.state.down = true;
            }
            if def.value >= 0 {
                format!("{tname}の HPが {} かいふくした", def.value)
            } else {
                format!("{tname}は {}の ダメージ", -def.value)
            }
        }
        MagicKind::MpChange => {
            let max_mp = member.effective_stats(item_catalog).get(StatKind::MaxMp).max(0);
            member.state.mp = (member.state.mp + def.value).clamp(0, max_mp);
            format!("{tname}の MPが {} かいふくした", def.value)
        }
        MagicKind::StatChange(stat) => {
            member
                .state
                .effects
                .retain(|e| e.source.as_deref() != Some(def.id.as_str()));
            member.state.effects.push(ActiveEffect {
                stat: *stat,
                delta: def.value,
                remaining: if def.duration_cycles == 0 { None } else { Some(def.duration_cycles) },
                source: Some(def.id.clone()),
            });
            format!("{tname}の {}が {:+}", stat.label(), def.value)
        }
        MagicKind::Revive { .. } | MagicKind::Light { .. } => {
            format!("{tname}には こうかが なかった")
        }
    }
}

// ------------------------------------------------------------------ learn / potion

/// Try to learn the magic a scroll teaches. Ok consumes the scroll (caller);
/// Err (too hard / already known / not a teaching scroll) leaves it in hand.
pub fn learn_scroll(
    member: &mut PartyMember,
    scroll: &ItemDef,
    magics: &MagicCatalog,
    item_catalog: &ItemCatalog,
) -> Result<String, String> {
    let Some(magic_id) = &scroll.teaches else {
        return Err("この巻物には 魔法が 書かれていない".into());
    };
    let Some(def) = magics.get(magic_id) else {
        return Err("巻物の魔法が 見つからない".into());
    };
    if member.state.learned.iter().any(|m| m == magic_id) {
        return Err("すでに おぼえている".into());
    }
    let knowledge = member.effective_stats(item_catalog).get(StatKind::MagicKnowledge);
    if knowledge < def.difficulty {
        return Err("むずかしくて 理解できない".into());
    }
    member.state.learned.push(magic_id.clone());
    Ok(format!("{}は 『{}』を おぼえた!", member.character.first_name, def.name))
}

/// Liquefy a spell into an empty container the caster carries, spending MP.
pub fn liquefy(
    member: &mut PartyMember,
    def: &MagicDef,
    item_catalog: &ItemCatalog,
) -> Result<String, String> {
    if !def.liquefiable {
        return Err(format!("{}は 液体化できない", def.name));
    }
    if member.state.mp < def.mp_cost {
        return Err("MPが たりない".into());
    }
    let slot = member
        .inventory
        .iter()
        .find(|(_, it)| {
            it.potion_of.is_none()
                && item_catalog.get(&it.def_id).is_some_and(|d| d.kind == ItemKind::EmptyContainer)
        })
        .map(|(s, _)| s);
    let Some(slot) = slot else {
        return Err("からのビンを もっていない".into());
    };
    member.state.mp = (member.state.mp - def.mp_cost).max(0);
    if let Some(inst) = member.inventory.get_mut(slot) {
        inst.potion_of = Some(def.id.clone());
    }
    Ok(format!("{}は {}を ビンに 詰めた", member.character.first_name, def.name))
}

/// Drink the potion at `slot`: apply its spell to the drinker (no MP — paid at
/// creation) and revert the bottle to an empty container.
pub fn drink_potion(
    member: &mut PartyMember,
    slot: SlotRef,
    magics: &MagicCatalog,
    item_catalog: &ItemCatalog,
    hunger: &HungerRules,
) -> Result<String, String> {
    let Some(magic_id) = member.inventory.get(slot).and_then(|it| it.potion_of.clone()) else {
        return Err("それは 秘薬では ない".into());
    };
    let Some(def) = magics.get(&magic_id).cloned() else {
        return Err("秘薬の魔法が 見つからない".into());
    };
    let msg = apply_ally_magic(member, &def, item_catalog, hunger);
    if let Some(inst) = member.inventory.get_mut(slot) {
        inst.potion_of = None;
    }
    Ok(format!("{}は 秘薬を 飲んだ。{msg}", member.character.first_name))
}

// ------------------------------------------------------------------ casting

/// Who a cast targets. Attack spells override this with the enemy ahead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CastTarget {
    /// The caster (self-cast).
    Caster,
    /// A specific party slot (ally magic).
    Member(usize),
    /// The enemy directly ahead (attack magic).
    FrontEnemy,
    /// The first downed member (Revive).
    DownedAuto,
}

/// A request to cast `magic_id` from party slot `caster` at `target`. Emitted by
/// the magic tab, the M key path, the autotest, and the debug-shot driver.
#[derive(Event, Clone)]
pub struct CastMagic {
    pub caster: usize,
    pub magic_id: String,
    pub target: CastTarget,
}

/// The magic the magic tab currently has selected, plus the chosen ally target
/// slot (defaults to the caster).
#[derive(Resource, Default)]
pub struct SelectedMagic {
    pub id: Option<String>,
    pub ally_target: usize,
}

/// The unified casting system: knowledge / learned / MP gating, then per-kind
/// effect application. Ally effects go through [`apply_ally_magic`]; attack
/// spells scan ahead, roll anti-magic resistance per bullet, damage / kill the
/// monster, and spawn cosmetic light-bullets.
#[allow(clippy::too_many_arguments)]
pub fn cast_magic(
    mut commands: Commands,
    mut events: EventReader<CastMagic>,
    mut party: ResMut<Party>,
    magics: Res<MagicCatalog>,
    item_catalog: Res<ItemCatalog>,
    rules: Res<RulesConfig>,
    mut rng: ResMut<GameRng>,
    mut log: ResMut<MessageLog>,
    player: Res<Player>,
    clock: Res<GameClock>,
    mut boost: ResMut<LightBoost>,
    monster_catalog: Res<MonsterCatalog>,
    doors: Res<DoorStates>,
    dungeon: Res<Dungeon>,
    mut monsters: Query<(Entity, &mut Monster)>,
    mut gfx: SpawnGfx,
) {
    for ev in events.read() {
        let Some(def) = magics.get(&ev.magic_id).cloned() else {
            log.push("その魔法は 存在しない");
            continue;
        };
        let Some(caster) = party.members.get(ev.caster) else {
            continue;
        };
        let cname = caster.character.first_name.clone();
        if !caster.state.learned.iter().any(|m| m == &def.id) {
            log.push(format!("{cname}は その魔法を おぼえていない"));
            continue;
        }
        let knowledge = caster.effective_stats(&item_catalog).get(StatKind::MagicKnowledge);
        if knowledge < def.difficulty {
            log.push("むずかしくて となえられない");
            continue;
        }
        if caster.state.mp < def.mp_cost {
            log.push("MPが たりない");
            continue;
        }

        // Charge MP once the cast is going to happen; target-less kinds bail out
        // *before* this so no MP is wasted.
        let charge = |party: &mut Party| {
            let m = &mut party.members[ev.caster];
            m.state.mp = (m.state.mp - def.mp_cost).max(0);
        };

        if def.is_attack() {
            let (fdx, fdy) = player.facing.delta();
            let mut target = None;
            for step in 1..=MAGIC_RANGE {
                let p = GridPos::new(
                    player.pos.x + fdx * step,
                    player.pos.y + fdy * step,
                    player.pos.floor,
                );
                if let Some(e) = crate::monster::monster_at(p, &monsters, &monster_catalog) {
                    target = Some((e, p));
                    break;
                }
                match dungeon.level.block_at(p) {
                    Some(Block::Wall) | None => break,
                    Some(Block::Door { kind }) if !doors.is_open(kind) => break,
                    _ => {}
                }
            }
            let Some((e, tpos)) = target else {
                log.push("目の前に敵はいない");
                continue;
            };
            charge(&mut party);
            gfx.se.send(crate::audio::PlaySe(crate::audio::Se::Cast));
            log.push(format!("{cname}は 『{}』を となえた!", def.name));
            let base = def.value.abs();
            let (_, mut mon) = monsters.get_mut(e).unwrap();
            let mdef = monster_catalog.get(&mon.def_id).cloned();
            let anti = mdef.as_ref().map(|d| d.anti_magic).unwrap_or(0);
            let rate = combat::magic_hit_rate(anti);
            let hits = (0..def.projectiles).filter(|_| rng.chance(rate)).count() as i32;
            let dmg = hits * base;
            mon.hp -= dmg;
            mon.anim = AnimKind::Hit;
            mon.anim_hold = 0.4;
            mon.facing = crate::monster::facing_toward(mon.pos, player.pos);
            let mname = mdef.as_ref().map(|d| d.name.clone()).unwrap_or_default();
            if dmg == 0 {
                log.push(format!("{mname}は 魔法を はじいた!"));
            } else {
                log.push(format!("{mname}に {dmg}の ダメージ"));
            }
            spawn_bullets_cmd(&mut commands, &mut gfx, shot_point(player.pos), shot_point(tpos), def.projectiles);
            if mon.hp <= 0
                && let Some(mdef) = mdef
            {
                crate::monster::kill_monster(
                    &mut commands, &mut mon, &mdef, clock.cycle, &mut party, &mut log,
                    &mut gfx.meshes, &mut gfx.materials, &gfx.asset_server, &item_catalog,
                );
            }
            continue;
        }

        match &def.kind {
            MagicKind::Light { strength } => {
                charge(&mut party);
            gfx.se.send(crate::audio::PlaySe(crate::audio::Se::Cast));
                let mult = light_multiplier(*strength);
                boost.multiplier = if boost.remaining > 0 { boost.multiplier.max(mult) } else { mult };
                boost.remaining = def.duration_cycles;
                log.push("あたりが あかるくなった");
            }
            MagicKind::Revive { ratio_percent } => {
                let ti = match ev.target {
                    CastTarget::Member(i) if party.members.get(i).is_some_and(|m| m.state.down) => Some(i),
                    _ => party.members.iter().position(|m| m.state.down),
                };
                let Some(ti) = ti else {
                    log.push("たおれている仲間はいない");
                    continue;
                };
                charge(&mut party);
            gfx.se.send(crate::audio::PlaySe(crate::audio::Se::Cast));
                let max_hp = party.members[ti].effective_stats(&item_catalog).get(StatKind::MaxHp).max(1);
                let hp = (max_hp * *ratio_percent as i32 / 100).clamp(1, max_hp);
                let m = &mut party.members[ti];
                m.state.down = false;
                m.state.hp = hp;
                let tn = m.character.first_name.clone();
                log.push(format!("{cname}は 『{}』を となえた! {tn}は よみがえった!", def.name));
            }
            MagicKind::StatChange(_) | MagicKind::HpChange | MagicKind::MpChange => {
                let ti = match ev.target {
                    CastTarget::Member(i) => i,
                    CastTarget::Caster => ev.caster,
                    CastTarget::DownedAuto => {
                        party.members.iter().position(|m| m.state.down).unwrap_or(ev.caster)
                    }
                    CastTarget::FrontEnemy => ev.caster,
                };
                if party.members.get(ti).is_none() {
                    continue;
                }
                charge(&mut party);
            gfx.se.send(crate::audio::PlaySe(crate::audio::Se::Cast));
                let msg = apply_ally_magic(&mut party.members[ti], &def, &item_catalog, &rules.hunger);
                log.push(format!("{cname}は 『{}』を となえた! {msg}", def.name));
            }
        }
    }
}

/// Spawn `count` staggered light-bullets via `commands` (0.15 s apart).
fn spawn_bullets_cmd(commands: &mut Commands, gfx: &mut SpawnGfx, from: Vec3, to: Vec3, count: u8) {
    let mesh = gfx.meshes.add(Sphere::new(0.11));
    for i in 0..count {
        let mat = gfx.materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.4),
            emissive: LinearRgba::rgb(3.0, 2.2, 0.7),
            unlit: true,
            ..default()
        });
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_translation(from),
            MagicProjectile {
                from,
                to,
                elapsed: 0.0,
                delay: i as f32 * 0.15,
                dur: 0.3,
            },
        ));
    }
}

// ------------------------------------------------------------------ debug scenes

/// Debug-shot driver for the `magic` / `light` / `potion` verification scenes:
/// once the scripted movement has settled, cast the scene's spell (or brew and
/// show a potion) exactly once. Cosmetic; gated on `DEEPGRID_DEBUG_SHOT`.
#[allow(clippy::too_many_arguments)]
pub fn debug_magic_driver(
    mut done: Local<bool>,
    script: Res<crate::player::ScriptedInput>,
    anim: Res<crate::player::MoveAnim>,
    mut party: ResMut<Party>,
    magics: Res<MagicCatalog>,
    item_catalog: Res<ItemCatalog>,
    mut cast_ev: EventWriter<CastMagic>,
    mut data: ResMut<DataScreen>,
    mut view: ResMut<crate::game_state::DataView>,
    mut selected: ResMut<crate::game_state::SelectedMember>,
) {
    let Some(scene) = crate::debug_shot::debug_shot_value() else {
        return;
    };
    if *done {
        return;
    }
    if !matches!(scene.as_str(), "magic" | "light" | "potion") {
        *done = true;
        return;
    }
    if !(script.queue.is_empty() && anim.is_idle()) {
        return;
    }
    let knower = |party: &Party, id: &str| party.members.iter().position(|m| m.state.learned.iter().any(|l| l == id));
    match scene.as_str() {
        "magic" => {
            if let Some(c) = knower(&party, "firebolt") {
                cast_ev.send(CastMagic { caster: c, magic_id: "firebolt".into(), target: CastTarget::FrontEnemy });
            }
        }
        "light" => {
            if let Some(c) = knower(&party, "light2") {
                cast_ev.send(CastMagic { caster: c, magic_id: "light2".into(), target: CastTarget::Caster });
            }
        }
        "potion" => {
            if let Some(c) = knower(&party, "heal") {
                let m = &mut party.members[c];
                let _ = m.inventory.pickup(ItemInstance::new("bottle_empty"));
                if let Some(def) = magics.get("heal").cloned() {
                    let _ = liquefy(m, &def, &item_catalog);
                }
                // Show the brewer's tab so the freshly-brewed 秘薬 is on screen.
                selected.index = c;
            }
            data.open = true;
            view.magic = false;
        }
        _ => {}
    }
    *done = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn magic(id: &str, kind: MagicKind) -> MagicDef {
        MagicDef {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            mp_cost: 5,
            difficulty: 10,
            kind,
            value: 0,
            duration_cycles: 0,
            liquefiable: false,
            projectiles: 0,
            symbol: String::new(),
        }
    }

    #[test]
    fn round_trip_and_catalog() {
        let defs = vec![
            MagicDef { value: 30, liquefiable: true, ..magic("heal", MagicKind::HpChange) },
            MagicDef { value: -25, projectiles: 2, ..magic("fire", MagicKind::HpChange) },
            magic("shield", MagicKind::StatChange(StatKind::Defense)),
            magic("rev", MagicKind::Revive { ratio_percent: 50 }),
            magic("lamp", MagicKind::Light { strength: 2 }),
        ];
        let ron = ron::ser::to_string_pretty(&defs, ron::ser::PrettyConfig::default()).unwrap();
        let back: Vec<MagicDef> = ron::from_str(&ron).unwrap();
        assert_eq!(defs, back);
        let cat = MagicCatalog::from_defs(defs, "test").unwrap();
        assert!(cat.get("heal").is_some());
        assert!(cat.get("fire").unwrap().is_attack());
        assert!(!cat.get("heal").unwrap().is_attack());
    }

    #[test]
    fn duplicate_ids_rejected() {
        let defs = vec![magic("a", MagicKind::MpChange), magic("a", MagicKind::MpChange)];
        assert!(MagicCatalog::from_defs(defs, "test").is_err());
    }

    #[test]
    fn light_multipliers() {
        assert_eq!(light_multiplier(1), 1.5);
        assert_eq!(light_multiplier(2), 2.5);
        assert_eq!(light_multiplier(3), 4.0);
        assert_eq!(light_multiplier(9), 1.0);
    }
}
