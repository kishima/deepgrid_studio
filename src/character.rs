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

use crate::hud::MessageLog;
use crate::player::PlayerFell;

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
}

/// Mutable per-play state, split from the immutable `Character` definition. This
/// is the save target (plan10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterState {
    pub hp: i32,
    pub mp: i32,
    pub concentration: i32,
    /// HP reached 0 → 気絶 (knocked out). No death/revive handling yet (plan7/8).
    pub down: bool,
}

impl CharacterState {
    /// Fresh state for a character: full HP/MP/concentration, standing.
    pub fn full(character: &Character) -> Self {
        Self {
            hp: character.stats.max_hp,
            mp: character.stats.max_mp,
            concentration: character.stats.concentration,
            down: false,
        }
    }
}

/// One party slot: the character definition paired with its live state.
pub struct PartyMember {
    pub character: Character,
    pub state: CharacterState,
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
        };
        let st = CharacterState::full(&ch);
        assert_eq!((st.hp, st.mp, st.concentration, st.down), (100, 30, 40, false));
    }
}
