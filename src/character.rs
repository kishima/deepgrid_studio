//! Character data model (plan4): the party's definition (`Character`) and its
//! mutable play state (`CharacterState`), kept deliberately separate.
//!
//! `Character` is authored data (loaded from `characters.ron`, edited in plan9's
//! character editor) and never mutates during play. `CharacterState` is the
//! runtime HP/MP/concentration that the HUD bars reflect and that fall damage /
//! cycle recovery change; it is what a save file (plan10) will persist.
//!
//! Numeric ranges from the original (level 0..=255, name lengths, etc.) are
//! PC98-era reference values only and are **not** enforced here — the types are
//! wide (`i32` / `String`) per project.md「上限値の扱い」.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::clock::CycleTick;
use crate::hud::MessageLog;
use crate::item::{Inventory, ItemCatalog};
use crate::player::PlayerFell;

/// Identifies one ability field for item stat-effects (item.rs `StatEffect`).
/// Mirrors the numeric fields of [`Stats`].
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatKind {
    MaxHp,
    MaxMp,
    Attack,
    Defense,
    Agility,
    Throwing,
    Carrying,
    LungCapacity,
    HeatResist,
    PoisonResist,
    MagicKnowledge,
    Concentration,
    Appraisal,
    Stealing,
    Bite,
}

impl StatKind {
    /// Short Japanese label (magic messages, the data-screen detail).
    pub fn label(self) -> &'static str {
        match self {
            StatKind::MaxHp => "最大HP",
            StatKind::MaxMp => "最大MP",
            StatKind::Attack => "こうげき",
            StatKind::Defense => "ぼうぎょ",
            StatKind::Agility => "すばやさ",
            StatKind::Throwing => "とおなげ",
            StatKind::Carrying => "うんぱん",
            StatKind::LungCapacity => "はいかつりょう",
            StatKind::HeatResist => "たいねつ",
            StatKind::PoisonResist => "たいどく",
            StatKind::MagicKnowledge => "まほうちしき",
            StatKind::Concentration => "しゅうちゅう",
            StatKind::Appraisal => "かんてい",
            StatKind::Stealing => "ぬすみ",
            StatKind::Bite => "はのつよさ",
        }
    }
}

/// Ability values (project.md「キャラクターの仕様」+ dandan_spec_things_editor.md).
///
/// plan4 only actually reads `max_hp` / `max_mp` / `concentration`, but the whole
/// set is defined now so later plans (items, combat, magic) have a home for every
/// stat without reshaping saved data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    /// Original was initial 0..=99 / max 255, but that is a reference value only
    /// — the type stays `u32` and nothing clamps it (project.md「上限値の扱い」).
    pub level: u32,
    pub max_hp: i32,
    pub max_mp: i32,
    pub attack: i32,
    pub defense: i32,
    /// すばやさ.
    pub agility: i32,
    /// 遠投力.
    pub throwing: i32,
    /// 運搬力.
    pub carrying: i32,
    /// 肺活量.
    pub lung_capacity: i32,
    /// 耐熱力.
    pub heat_resist: i32,
    /// 耐毒性.
    pub poison_resist: i32,
    /// 魔法知識.
    pub magic_knowledge: i32,
    /// 集中力(最大値).
    pub concentration: i32,
    /// 鑑定力.
    pub appraisal: i32,
    /// 盗みのうで.
    pub stealing: i32,
    /// 歯の強さ.
    pub bite: i32,
}

impl Stats {
    /// Mutable access to the field named by `k`.
    fn field_mut(&mut self, k: StatKind) -> &mut i32 {
        match k {
            StatKind::MaxHp => &mut self.max_hp,
            StatKind::MaxMp => &mut self.max_mp,
            StatKind::Attack => &mut self.attack,
            StatKind::Defense => &mut self.defense,
            StatKind::Agility => &mut self.agility,
            StatKind::Throwing => &mut self.throwing,
            StatKind::Carrying => &mut self.carrying,
            StatKind::LungCapacity => &mut self.lung_capacity,
            StatKind::HeatResist => &mut self.heat_resist,
            StatKind::PoisonResist => &mut self.poison_resist,
            StatKind::MagicKnowledge => &mut self.magic_knowledge,
            StatKind::Concentration => &mut self.concentration,
            StatKind::Appraisal => &mut self.appraisal,
            StatKind::Stealing => &mut self.stealing,
            StatKind::Bite => &mut self.bite,
        }
    }

    pub fn get(&self, k: StatKind) -> i32 {
        let mut copy = self.clone();
        *copy.field_mut(k)
    }

    /// Add `delta` to the field named by `k` (used when folding effects in).
    pub fn add(&mut self, k: StatKind, delta: i32) {
        *self.field_mut(k) += delta;
    }

    /// 総合レベル: the mean of every ability parameter (dandan_spec: a rough
    /// "strength" gauge). Derived on demand, never stored. Displayed from plan5's
    /// data screen; unused by the plan4 HUD (hence allowed-dead until then).
    #[allow(dead_code)]
    pub fn overall_level(&self) -> i32 {
        let values = [
            self.max_hp,
            self.max_mp,
            self.attack,
            self.defense,
            self.agility,
            self.throwing,
            self.carrying,
            self.lung_capacity,
            self.heat_resist,
            self.poison_resist,
            self.magic_knowledge,
            self.concentration,
            self.appraisal,
            self.stealing,
            self.bite,
        ];
        let sum: i32 = values.iter().sum();
        sum / values.len() as i32
    }
}

/// Growth type (dandan_spec: 平均型/早期開花型/大器晩成型/天才型/才能なし).
/// Shapes how stats rise on level-up. plan4 only carries it as data.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrowthType {
    Average,
    EarlyBloomer,
    LateBloomer,
    Genius,
    Talentless,
}

impl GrowthType {
    /// Per-level-up growth multiplier (plan6, provisional). Early/late bloomers
    /// swing on either side of level 20.
    pub fn multiplier(self, level: u32) -> f32 {
        match self {
            GrowthType::Average => 1.0,
            GrowthType::EarlyBloomer => if level < 20 { 1.5 } else { 0.5 },
            GrowthType::LateBloomer => if level < 20 { 0.5 } else { 1.5 },
            GrowthType::Genius => 1.5,
            GrowthType::Talentless => 0.2,
        }
    }
}

/// Experience needed to reach the level after `level` (plan6, provisional).
pub fn level_up_threshold(level: u32) -> i32 {
    (level.max(1) as i32) * 100
}

/// A registered character: profile + stats + which model shows their portrait.
///
/// The profile fields exist for player attachment (dandan_spec_things_editor.md);
/// plan4 uses none of them in game logic beyond `first_name` (shown on the HUD)
/// and `model` (portrait source). Character/number-range limits are reference
/// values and are not enforced.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Character {
    /// Unique within the project ("knight", …); referenced by `party`.
    pub id: String,
    /// Shown in game.
    pub first_name: String,
    /// Family name; hidden in game.
    pub last_name: String,
    pub gender: String,
    pub height_cm: f32,
    pub weight_kg: f32,
    /// "YYYY-MM-DD".
    pub birth_date: String,
    pub age: u32,
    /// 好きなもの.
    pub likes: String,
    /// 嫌いなもの.
    pub dislikes: String,
    /// 経歴 (may span lines; the Wizardry-flavour class label lives here).
    pub background: String,
    pub growth: GrowthType,
    pub stats: Stats,
    /// Portrait source: a project-relative or `assets/`-relative `.glb` path.
    pub model: String,
    /// Optional static portrait image (`assets/`-relative path). When set, the
    /// HUD shows this image instead of the live 3D bust rendered from `model`
    /// (user feedback 2026-07-14: the KayKit busts were too cute — engravings
    /// give the party a darker look).
    #[serde(default)]
    pub portrait: String,
    /// Starting item ids (plan5). Equippable ones are auto-equipped at party
    /// build; the rest go to hands/pouch/backpack. `#[serde(default)]` so
    /// pre-plan5 `characters.ron` still parses.
    #[serde(default)]
    pub items: Vec<String>,
    /// Initially-known magic ids (plan7). Poured into `CharacterState.learned` at
    /// party build (mirrors `items`). `#[serde(default)]` so pre-plan7 data parses.
    #[serde(default)]
    pub magics: Vec<String>,
}

/// A stat change currently in force from an *eaten* item. Equipment effects are
/// derived from worn items instead (see [`Inventory::equipment_effects`]) so
/// unequipping restores stats automatically without touching base values.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActiveEffect {
    pub stat: StatKind,
    pub delta: i32,
    /// Cycles left, or `None` for a permanent effect (duration 0).
    pub remaining: Option<u64>,
    /// The magic id that applied this effect (plan7), or `None` for eaten-item
    /// effects. Recasting the same magic replaces (resets) its effect rather than
    /// stacking, keyed on this.
    pub source: Option<String>,
}

/// Mutable per-play state, split from the immutable `Character` definition. This
/// is the save target (plan10). No longer `Copy` — it owns the active-effect list.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct CharacterState {
    pub hp: i32,
    pub mp: i32,
    pub concentration: i32,
    /// HP reached 0 → 気絶 (knocked out). No death/revive handling yet (plan7/8).
    pub down: bool,
    /// Temporary/permanent stat effects from eaten items.
    pub effects: Vec<ActiveEffect>,
    /// Remaining cycles of lingering poison (plan5 liquid damage).
    pub poison_remaining: u32,
    /// Satiety / 満腹度 (plan6.5). Only meaningful when hunger rules are enabled;
    /// starts full. Drains over time, refilled by eating.
    pub satiety: i32,
    /// Accumulated experience toward the next level (plan6).
    pub exp: i32,
    /// 防ぐ: halve the next incoming hit, then clear (plan6).
    pub guarding: bool,
    /// 精神統一: concentration recovers 5× until acting / being hit (plan6).
    pub concentrating: bool,
    /// Known magic ids (plan7). Seeded from `Character.magics` at party build and
    /// grown by reading teaching scrolls. A save target (plan10).
    pub learned: Vec<String>,
}

impl CharacterState {
    /// Fresh state for a character: full HP/MP/concentration, standing.
    pub fn full(character: &Character) -> Self {
        Self {
            hp: character.stats.max_hp,
            mp: character.stats.max_mp,
            concentration: character.stats.concentration,
            down: false,
            effects: Vec::new(),
            poison_remaining: 0,
            // Full by default; build_party clamps it to the project's satiety_max.
            satiety: crate::rules::DEFAULT_SATIETY_MAX,
            exp: 0,
            guarding: false,
            concentrating: false,
            learned: Vec::new(),
        }
    }
}

/// One party slot: the character definition, its live state, and its inventory.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PartyMember {
    pub character: Character,
    pub state: CharacterState,
    pub inventory: Inventory,
}

impl PartyMember {
    /// Effective stats = base + equipment effects + active (eaten) effects. Base
    /// values are never mutated, so removing an effect restores them exactly.
    pub fn effective_stats(&self, catalog: &ItemCatalog) -> Stats {
        let mut s = self.character.stats.clone();
        for (kind, delta) in self.inventory.equipment_effects(catalog) {
            s.add(kind, delta);
        }
        for e in &self.state.effects {
            s.add(e.stat, e.delta);
        }
        s
    }

    /// Try to eat the item defined by `def`: fails on important items or when
    /// too hard to bite. On success applies nutrition (HP, the original rule) plus
    /// — when hunger is enabled — satiety, and its effects; returns the log line.
    /// The caller removes the consumed instance.
    pub fn eat(
        &mut self,
        def: &crate::item::ItemDef,
        catalog: &ItemCatalog,
        hunger: &crate::rules::HungerRules,
    ) -> Result<String, String> {
        if def.important {
            return Err(format!("{}は だいじなものだ", def.name));
        }
        let bite = self.effective_stats(catalog).get(StatKind::Bite);
        if def.hardness > bite {
            return Err(format!("{}は かたくて食べられない", def.name));
        }
        let max_hp = self.effective_stats(catalog).get(StatKind::MaxHp);
        self.state.hp = (self.state.hp + def.nutrition).clamp(0, max_hp.max(0));
        // Satiety moves with nutrition (negative nutrition lowers it too).
        if hunger.enabled {
            self.state.satiety = (self.state.satiety
                + def.nutrition * hunger.satiety_per_nutrition)
                .clamp(0, hunger.satiety_max);
        }
        for e in &def.effects {
            self.state.effects.push(ActiveEffect {
                stat: e.stat,
                delta: e.delta,
                remaining: if e.duration_cycles == 0 {
                    None
                } else {
                    Some(e.duration_cycles)
                },
                source: None,
            });
        }
        if self.state.hp == 0 {
            self.state.down = true;
        }
        Ok(format!("{}を食べた", def.name))
    }

    /// Award `amount` experience and apply any resulting level-ups (plan6). Growth
    /// scales the flat per-level gains by [`GrowthType::multiplier`]. Returns one
    /// message per level gained (base values never mutated except stats here).
    pub fn gain_exp(&mut self, amount: i32) -> Vec<String> {
        const GROWABLE: [StatKind; 13] = [
            StatKind::Attack,
            StatKind::Defense,
            StatKind::Agility,
            StatKind::Throwing,
            StatKind::Carrying,
            StatKind::LungCapacity,
            StatKind::HeatResist,
            StatKind::PoisonResist,
            StatKind::MagicKnowledge,
            StatKind::Concentration,
            StatKind::Appraisal,
            StatKind::Stealing,
            StatKind::Bite,
        ];
        let name = self.character.first_name.clone();
        let mut msgs = Vec::new();
        self.state.exp += amount;
        loop {
            let level = self.character.stats.level;
            let need = level_up_threshold(level);
            if self.state.exp < need {
                break;
            }
            self.state.exp -= need;
            let mult = self.character.growth.multiplier(level);
            let dhp = (4.0 * mult).floor() as i32;
            let dmp = (2.0 * mult).floor() as i32;
            let d1 = mult.floor() as i32;
            {
                let s = &mut self.character.stats;
                s.level += 1;
                s.max_hp += dhp;
                s.max_mp += dmp;
                for k in GROWABLE {
                    s.add(k, d1);
                }
            }
            // Heal by the max gains so the new headroom is filled.
            self.state.hp += dhp;
            self.state.mp += dmp;
            msgs.push(format!("{name}は レベル {} になった!", self.character.stats.level));
        }
        msgs
    }
}

/// The active party (≤ `LimitsConfig.party_size`), resolved from the project's
/// `party` id list. Empty for version-1 projects that predate characters — the
/// HUD's status window is then hidden.
#[derive(Resource, Default)]
pub struct Party {
    pub members: Vec<PartyMember>,
}

impl Party {
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// The first member whose carried weight exceeds their (effective) carrying
    /// capacity, if any. Party movement is a group action, so one overloaded
    /// member stops everyone (plan5).
    pub fn overweight_member(&self, catalog: &ItemCatalog) -> Option<usize> {
        self.members.iter().position(|m| {
            m.inventory.total_weight(catalog) > m.effective_stats(catalog).get(StatKind::Carrying)
        })
    }
}

/// Fall damage per floor², before any mitigation (plan4). **Provisional**: the
/// original's formula is unknown; agility/etc. reductions are deferred to the
/// plan6 calculation system. If this changes, update plan4.md「落下ダメージ」.
const FALL_DAMAGE_PER_FLOOR_SQ: i32 = 10;

/// Damage each party member by `n²·10` when a fall lands, clamp HP at 0, mark the
/// downed, and log the event (project.md「メッセージウインドー」). The leading line
/// carries the floor count so both the fall and the damage stay visible in the
/// 4-line message window for a full party.
pub fn apply_fall_damage(
    mut fell: EventReader<PlayerFell>,
    mut party: ResMut<Party>,
    mut log: ResMut<MessageLog>,
) {
    for event in fell.read() {
        let n = event.floors as i32;
        let damage = FALL_DAMAGE_PER_FLOOR_SQ * n * n;
        for (i, member) in party.members.iter_mut().enumerate() {
            member.state.hp = (member.state.hp - damage).max(0);
            if member.state.hp == 0 {
                member.state.down = true;
            }
            let name = &member.character.first_name;
            let line = if i == 0 {
                format!("{n}フロア落下した! {name}は {damage} のダメージ!")
            } else {
                format!("{name}は {damage} のダメージ!")
            };
            log.push(line);
        }
    }
}

/// Age out temporary eaten-item effects each cycle; permanent effects (duration
/// 0 → `remaining: None`) are kept forever.
pub fn tick_effects(mut ticks: EventReader<CycleTick>, mut party: ResMut<Party>) {
    let cycles = ticks.read().count() as u64;
    if cycles == 0 {
        return;
    }
    for member in &mut party.members {
        member.state.effects.retain_mut(|e| match &mut e.remaining {
            None => true,
            Some(remaining) => {
                *remaining = remaining.saturating_sub(cycles);
                *remaining > 0
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(v: i32) -> Stats {
        Stats {
            level: 1,
            max_hp: v,
            max_mp: v,
            attack: v,
            defense: v,
            agility: v,
            throwing: v,
            carrying: v,
            lung_capacity: v,
            heat_resist: v,
            poison_resist: v,
            magic_knowledge: v,
            concentration: v,
            appraisal: v,
            stealing: v,
            bite: v,
        }
    }

    #[test]
    fn overall_level_is_mean_of_abilities() {
        // All 15 ability fields equal 20 → mean is 20.
        assert_eq!(stats(20).overall_level(), 20);
    }

    fn member(growth: GrowthType, level: u32) -> PartyMember {
        let ch = Character {
            id: "t".into(),
            first_name: "テスト".into(),
            last_name: "".into(),
            gender: "".into(),
            height_cm: 170.0,
            weight_kg: 60.0,
            birth_date: "0-1-1".into(),
            age: 20,
            likes: "".into(),
            dislikes: "".into(),
            background: "".into(),
            growth,
            stats: Stats { level, max_hp: 100, attack: 10, ..stats(5) },
            model: "".into(),
            portrait: "".into(),
            items: vec![],
            magics: vec![],
        };
        let state = CharacterState::full(&ch);
        PartyMember { character: ch, state, inventory: Inventory::new(3, 3) }
    }

    #[test]
    fn level_up_grows_stats_by_growth_type() {
        // Average at level 1: threshold 100, multiplier 1.0 → +4 HP, +1 to each.
        let mut m = member(GrowthType::Average, 1);
        let msgs = m.gain_exp(100);
        assert_eq!(msgs.len(), 1);
        assert_eq!(m.character.stats.level, 2);
        assert_eq!(m.character.stats.max_hp, 104);
        assert_eq!(m.character.stats.attack, 11);
        assert_eq!(m.state.exp, 0);

        // Talentless at level 1: multiplier 0.2 → floor(4*0.2)=0 HP, floor(0.2)=0.
        let mut t = member(GrowthType::Talentless, 1);
        t.gain_exp(100);
        assert_eq!(t.character.stats.level, 2);
        assert_eq!(t.character.stats.max_hp, 100); // no growth
        assert_eq!(t.character.stats.attack, 10);
    }

    #[test]
    fn eat_feeds_satiety_only_when_enabled() {
        use crate::item::{ItemCatalog, ItemDef};
        use crate::rules::HungerRules;
        let cat = ItemCatalog::default();
        let bread: ItemDef =
            ron::from_str(r#"(id:"b",name:"パン",kind:General,nutrition:20)"#).unwrap();

        // Enabled: satiety += nutrition × factor, clamped to max.
        let mut m = member(GrowthType::Average, 1);
        m.state.satiety = 100;
        let on = HungerRules { enabled: true, satiety_per_nutrition: 10, satiety_max: 1000, ..Default::default() };
        m.eat(&bread, &cat, &on).unwrap();
        assert_eq!(m.state.satiety, 300);

        // Disabled: satiety untouched (the original HP rule is unaffected either way).
        let mut m2 = member(GrowthType::Average, 1);
        m2.state.satiety = 100;
        m2.eat(&bread, &cat, &HungerRules::default()).unwrap();
        assert_eq!(m2.state.satiety, 100);
    }

    #[test]
    fn growth_multiplier_swings_at_20() {
        assert_eq!(GrowthType::EarlyBloomer.multiplier(10), 1.5);
        assert_eq!(GrowthType::EarlyBloomer.multiplier(25), 0.5);
        assert_eq!(GrowthType::LateBloomer.multiplier(10), 0.5);
        assert_eq!(GrowthType::LateBloomer.multiplier(25), 1.5);
    }

    #[test]
    fn full_state_starts_at_maxima() {
        let ch = Character {
            id: "t".into(),
            first_name: "テスト".into(),
            last_name: "".into(),
            gender: "".into(),
            height_cm: 170.0,
            weight_kg: 60.0,
            birth_date: "0000-01-01".into(),
            age: 20,
            likes: "".into(),
            dislikes: "".into(),
            background: "".into(),
            growth: GrowthType::Average,
            stats: Stats {
                max_hp: 100,
                max_mp: 30,
                concentration: 40,
                ..stats(1)
            },
            model: "models/party/knight.glb".into(),
            portrait: "".into(),
            items: Vec::new(),
            magics: Vec::new(),
        };
        let st = CharacterState::full(&ch);
        assert_eq!((st.hp, st.mp, st.concentration, st.down), (100, 30, 40, false));
    }
}
