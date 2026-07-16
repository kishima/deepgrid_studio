//! Save / load (plan10): serialize the whole run state to
//! `<project>/saves/slot{1..3}.ron` and restore it.
//!
//! The save is deterministic-complete: party, player pose, every visited
//! level's [`LevelState`] (the plan8 transition snapshot, reused verbatim),
//! event flags / queue / wall writes, the game clock and the RNG state. Loading
//! and replaying the same inputs reproduces the same outcomes (autotest
//! `save-load` asserts this).
//!
//! Loading rebuilds the world by sending a normal [`LevelTransition`] to the
//! saved level — with [`SkipNextSnapshot`] set so the pre-load runtime doesn't
//! clobber the restored `LevelStates`.

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::character::{Party, PartyMember};
use crate::clock::GameClock;
use crate::dungeon::{Facing, GridPos};
use crate::event::{EventFlags, EventQueue, MoveMode, QueuedEvent, TriggerStates, WallWrites};
use crate::hud::MessageLog;
use crate::player::Player;
use crate::rng::GameRng;
use crate::world::{
    CurrentLevel, GameLevels, LevelState, LevelStates, LevelTransition, SkipNextSnapshot,
    snapshot_level,
};

/// Save-format version this build writes. A mismatch is refused with a message
/// (no migration in plan10).
pub const SAVE_VERSION: u32 = 1;

/// Number of save slots (1-based in file names and UI).
pub const SLOTS: usize = 3;

/// Everything a run needs to resume (see module docs).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveData {
    pub save_version: u32,
    pub current_level: usize,
    pub player_pos: GridPos,
    pub player_facing: Facing,
    #[serde(default)]
    pub move_mode: MoveMode,
    pub party: Vec<PartyMember>,
    #[serde(default)]
    pub level_states: HashMap<usize, LevelState>,
    #[serde(default)]
    pub flags: Vec<bool>,
    #[serde(default)]
    pub queue: Vec<QueuedEvent>,
    /// Wall-write overrides as `((level, x, y, floor), text)`.
    #[serde(default)]
    pub wall_writes: Vec<((usize, i32, i32, usize), String)>,
    #[serde(default)]
    pub cycle: u64,
    #[serde(default)]
    pub rng_state: u64,
    /// The BGM override in effect, if any (level BGM re-derives on load).
    #[serde(default)]
    pub bgm_override: Option<String>,
}

/// UI/CLI request to write the current state to a slot (1..=SLOTS).
#[derive(Event, Clone, Copy)]
pub struct SaveRequest(pub usize);

/// UI/CLI request to load a slot (1..=SLOTS).
#[derive(Event, Clone, Copy)]
pub struct LoadRequest(pub usize);

/// A slot requested via `--load <slot>`; consumed on the first frame.
#[derive(Resource, Default)]
pub struct PendingCliLoad(pub Option<usize>);

/// The slot file path for `slot` (1-based).
pub fn slot_path(project_dir: &std::path::Path, slot: usize) -> PathBuf {
    project_dir.join("saves").join(format!("slot{slot}.ron"))
}

/// Which slots have a save file (for the data-screen buttons).
pub fn slot_exists(project_dir: &std::path::Path, slot: usize) -> bool {
    slot_path(project_dir, slot).is_file()
}

/// The read-mostly world state a save gathers, bundled for the parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SaveWorld<'w> {
    pub current: Res<'w, CurrentLevel>,
    pub states: Res<'w, LevelStates>,
    pub dungeon: Res<'w, crate::dungeon::Dungeon>,
    pub doors: Res<'w, crate::dungeon::DoorStates>,
    pub triggers: Res<'w, TriggerStates>,
    pub flags: Res<'w, EventFlags>,
    pub queue: Res<'w, EventQueue>,
    pub writes: Res<'w, WallWrites>,
    pub clock: Res<'w, GameClock>,
    pub rng: Res<'w, GameRng>,
    pub bgm: Res<'w, crate::audio::BgmState>,
    pub move_mode: Res<'w, MoveMode>,
    pub limits: Res<'w, crate::config::LimitsConfig>,
    pub resolver: Res<'w, crate::project::AssetResolver>,
}

/// Persistence (save/load) is disabled while test-playing an unsaved editor
/// project (plan13): the live world was built from memory, so it and the on-disk
/// saves would disagree. The single source of truth for the gate — the two
/// handlers and the data-screen slots all consult it.
pub fn persistence_disabled(test_play: &crate::editor::TestPlay) -> bool {
    test_play.0
}

/// Handle [`SaveRequest`]: snapshot the current level (same function as a
/// transition), then serialize everything to the slot file.
#[allow(clippy::too_many_arguments)]
pub fn handle_save(
    mut reqs: EventReader<SaveRequest>,
    w: SaveWorld,
    party: Res<Party>,
    player: Res<Player>,
    game_levels: Res<GameLevels>,
    monsters: Query<&crate::monster::Monster>,
    items: Query<&crate::floor_items::FloorItem>,
    test_play: Res<crate::editor::TestPlay>,
    mut log: ResMut<MessageLog>,
) {
    for req in reqs.read() {
        let slot = req.0;
        if !(1..=SLOTS).contains(&slot) {
            continue;
        }
        if persistence_disabled(&test_play) {
            log.push("テストプレイ中はセーブできません");
            continue;
        }
        // Freeze the loaded level into a copy of the visited-level map.
        let mut level_states = w.states.map.clone();
        if let Some(orig) = game_levels.levels.get(w.current.0) {
            let st = snapshot_level(
                orig,
                &w.dungeon,
                &w.doors,
                &w.triggers,
                w.limits.door_kinds_per_level,
                &monsters,
                &items,
            );
            level_states.insert(w.current.0, st);
        }
        let data = SaveData {
            save_version: SAVE_VERSION,
            current_level: w.current.0,
            player_pos: player.pos,
            player_facing: player.facing,
            move_mode: *w.move_mode,
            party: party.members.clone(),
            level_states,
            flags: w.flags.to_vec(),
            queue: w.queue.pending.clone(),
            wall_writes: w.writes.map.iter().map(|(k, v)| (*k, v.clone())).collect(),
            cycle: w.clock.cycle,
            rng_state: w.rng.state(),
            bgm_override: w.bgm.override_track.clone(),
        };
        let path = slot_path(&w.resolver.project_dir, slot);
        let result = ron::ser::to_string_pretty(&data, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("serialize: {e}"))
            .and_then(|text| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
                }
                std::fs::write(&path, text).map_err(|e| format!("write: {e}"))
            });
        match result {
            Ok(()) => log.push(format!("スロット{slot}に セーブした。")),
            Err(e) => log.push(format!("セーブ失敗: {e}")),
        }
    }
}

/// Read + version-check a slot file.
pub fn read_slot(project_dir: &std::path::Path, slot: usize) -> Result<SaveData, String> {
    let path = slot_path(project_dir, slot);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let data: SaveData = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.save_version != SAVE_VERSION {
        return Err(format!(
            "セーブバージョン {} は読めない (対応: {SAVE_VERSION})",
            data.save_version
        ));
    }
    Ok(data)
}

/// The mutable world state a load rewrites, bundled for the parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct LoadWorld<'w> {
    pub states: ResMut<'w, LevelStates>,
    pub flags: ResMut<'w, EventFlags>,
    pub queue: ResMut<'w, EventQueue>,
    pub writes: ResMut<'w, WallWrites>,
    pub clock: ResMut<'w, GameClock>,
    pub rng: ResMut<'w, GameRng>,
    pub bgm: ResMut<'w, crate::audio::BgmState>,
    pub move_mode: ResMut<'w, MoveMode>,
    pub skip_snapshot: ResMut<'w, SkipNextSnapshot>,
    pub resolver: Res<'w, crate::project::AssetResolver>,
}

/// Handle [`LoadRequest`] (and the `--load` CLI on the first frame): restore all
/// global state, then rebuild the world via a normal level transition.
#[allow(clippy::too_many_arguments)]
pub fn handle_load(
    mut reqs: EventReader<LoadRequest>,
    mut cli: ResMut<PendingCliLoad>,
    mut w: LoadWorld,
    mut party: ResMut<Party>,
    mut transition: EventWriter<LevelTransition>,
    test_play: Res<crate::editor::TestPlay>,
    mut log: ResMut<MessageLog>,
) {
    let mut slots: Vec<usize> = Vec::new();
    if let Some(s) = cli.0.take() {
        slots.push(s);
    }
    slots.extend(reqs.read().map(|r| r.0));
    let Some(&slot) = slots.last() else { return };
    if persistence_disabled(&test_play) {
        log.push("テストプレイ中はロードできません");
        return;
    }
    let data = match read_slot(&w.resolver.project_dir, slot) {
        Ok(d) => d,
        Err(e) => {
            log.push(format!("ロード失敗: {e}"));
            return;
        }
    };
    party.members = data.party;
    w.states.map = data.level_states;
    w.flags.restore(&data.flags);
    w.queue.pending = data.queue;
    w.writes.map = data.wall_writes.into_iter().collect();
    w.clock.restore(data.cycle);
    w.rng.set_state(data.rng_state);
    w.bgm.override_track = data.bgm_override;
    *w.move_mode = data.move_mode;
    // Rebuild through the standard transition path; the restored LevelStates
    // must survive it, so the leave-snapshot is skipped once.
    w.skip_snapshot.0 = true;
    transition.send(LevelTransition {
        to_level: data.current_level,
        to: data.player_pos,
        to_facing: data.player_facing,
    });
    log.push(format!("スロット{slot}を ロードした。"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::{Character, CharacterState};
    use crate::item::Inventory;
    use crate::world::{FloorItemSnapshot, MonsterSnapshot};

    #[test]
    fn test_play_gates_persistence() {
        // plan13: saving/loading is refused only while a test play is running.
        assert!(persistence_disabled(&crate::editor::TestPlay(true)));
        assert!(!persistence_disabled(&crate::editor::TestPlay(false)));
    }

    fn sample_character() -> Character {
        use crate::character::{GrowthType, Stats};
        Character {
            id: "hero".into(),
            first_name: "テスト".into(),
            last_name: String::new(),
            gender: String::new(),
            height_cm: 170.0,
            weight_kg: 60.0,
            birth_date: "0-1-1".into(),
            age: 20,
            likes: String::new(),
            dislikes: String::new(),
            background: String::new(),
            growth: GrowthType::Average,
            stats: Stats {
                level: 3,
                max_hp: 100,
                max_mp: 10,
                attack: 10,
                defense: 5,
                agility: 5,
                throwing: 5,
                carrying: 5,
                lung_capacity: 5,
                heat_resist: 5,
                poison_resist: 5,
                magic_knowledge: 5,
                concentration: 30,
                appraisal: 5,
                stealing: 5,
                bite: 5,
            },
            model: String::new(),
            portrait: String::new(),
            items: vec![],
            magics: vec![],
        }
    }

    fn sample_save() -> SaveData {
        let character = sample_character();
        let mut state = CharacterState::full(&character);
        state.hp = 42;
        state.learned = vec!["fire".into()];
        let mut inventory = Inventory::new(3, 24);
        inventory
            .pickup(crate::item::ItemInstance::new("potion".to_string()))
            .unwrap();
        let mut level_states = HashMap::new();
        level_states.insert(
            1,
            LevelState {
                monsters: vec![MonsterSnapshot {
                    def_id: "skel".into(),
                    hp: 7,
                    pos: GridPos::new(1, 2, 0),
                    facing: Facing::East,
                    dead: false,
                    dead_cycle: 0,
                    fleeing: true,
                    carry: vec!["bone".into()],
                }],
                items: vec![FloorItemSnapshot {
                    instance: crate::item::ItemInstance::new("sword".to_string()),
                    pos: GridPos::new(3, 3, 1),
                }],
                doors_open: vec![0],
                triggers: TriggerStates::default(),
                block_diffs: vec![((4, 4, 0), crate::dungeon::Block::Water)],
            },
        );
        SaveData {
            save_version: SAVE_VERSION,
            current_level: 1,
            player_pos: GridPos::new(5, 6, 1),
            player_facing: Facing::South,
            move_mode: MoveMode::Free,
            party: vec![PartyMember { character, state, inventory }],
            level_states,
            flags: vec![true, false, true],
            queue: vec![QueuedEvent { event_id: "ev1".into(), level: 1, fire_cycle: 99 }],
            wall_writes: vec![((0, 1, 2, 0), "らくがき".into())],
            cycle: 1234,
            rng_state: 0xDEADBEEF,
            bgm_override: Some("bgm_battle.ogg".into()),
        }
    }

    #[test]
    fn save_data_round_trips() {
        let data = sample_save();
        let text = ron::ser::to_string_pretty(&data, ron::ser::PrettyConfig::default()).unwrap();
        let back: SaveData = ron::from_str(&text).unwrap();
        assert_eq!(back.save_version, SAVE_VERSION);
        assert_eq!(back.current_level, 1);
        assert_eq!(back.player_pos, data.player_pos);
        assert_eq!(back.party[0].state.hp, 42);
        assert_eq!(back.party[0].state.learned, vec!["fire".to_string()]);
        assert_eq!(back.flags, data.flags);
        assert_eq!(back.queue[0].fire_cycle, 99);
        assert_eq!(back.wall_writes, data.wall_writes);
        assert_eq!(back.cycle, 1234);
        assert_eq!(back.rng_state, 0xDEADBEEF);
        assert_eq!(back.bgm_override.as_deref(), Some("bgm_battle.ogg"));
        let ls = &back.level_states[&1];
        assert_eq!(ls.monsters[0].def_id, "skel");
        assert!(ls.monsters[0].fleeing);
        assert_eq!(ls.items[0].pos, GridPos::new(3, 3, 1));
        assert_eq!(ls.block_diffs, vec![((4, 4, 0), crate::dungeon::Block::Water)]);
    }

    #[test]
    fn version_mismatch_is_refused() {
        let dir = std::env::temp_dir().join("deepgrid_save_test");
        std::fs::create_dir_all(dir.join("saves")).unwrap();
        let mut data = sample_save();
        data.save_version = 999;
        let text = ron::ser::to_string_pretty(&data, ron::ser::PrettyConfig::default()).unwrap();
        std::fs::write(slot_path(&dir, 2), text).unwrap();
        let err = read_slot(&dir, 2).unwrap_err();
        assert!(err.contains("999"), "{err}");
    }
}
