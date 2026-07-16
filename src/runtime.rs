//! Project → runtime resource derivation (plan13).
//!
//! `run_play` used to inline the whole "turn a loaded [`Project`] into the
//! play-mode resources" step. plan13 unifies the editor and play into one App and
//! lets the editor rebuild the world from its (unsaved) project, so this
//! derivation must run in three places: the initial App build (`run_play`), an
//! editor test-play start, and an editor → title return. It lives here as one
//! function feeding a [`RuntimeBundle`], so all three paths agree on what
//! "the world derived from a project" means.

use bevy::prelude::*;

use crate::audio::BgmState;
use crate::character::Party;
use crate::config::LimitsConfig;
use crate::demo::DemoCatalog;
use crate::dungeon::{DoorStates, Dungeon};
use crate::event::{EventFlags, EventQueue, MoveMode, WallWrites};
use crate::floor_items::InitialItems;
use crate::game_state::DataScreen;
use crate::item::ItemCatalog;
use crate::magic::MagicCatalog;
use crate::monster::{InitialMonsters, MonsterCatalog};
use crate::project::{AssetResolver, Project};
use crate::rng::GameRng;
use crate::rules::RulesConfig;
use crate::title::InitialRun;
use crate::world::{GameLevels, LevelStates, LevelTransition, SkipNextSnapshot};

/// Every resource derived purely from a loaded [`Project`]. Built once by
/// [`build_runtime`] and either inserted into the App at startup ([`insert`])
/// or written over a running world when the editor rebuilds it ([`apply`]).
///
/// [`insert`]: RuntimeBundle::insert
/// [`apply`]: RuntimeBundle::apply
pub struct RuntimeBundle {
    pub limits: LimitsConfig,
    pub rules: RulesConfig,
    pub dungeon: Dungeon,
    pub doors: DoorStates,
    pub party: Party,
    pub catalog: ItemCatalog,
    pub monster_catalog: MonsterCatalog,
    pub magic_catalog: MagicCatalog,
    pub initial_items: InitialItems,
    pub initial_monsters: InitialMonsters,
    pub event_flags: EventFlags,
    pub game_levels: GameLevels,
    pub initial_run: InitialRun,
    pub demo_catalog: DemoCatalog,
    pub resolver: AssetResolver,
    /// Title-screen metadata (project.ron v8), so the title can be rebuilt when
    /// the editor returns to it.
    pub game_title: String,
    pub game_author: String,
    pub game_desc: String,
}

/// Derive every play-mode resource from a loaded project. Pure (no I/O beyond
/// what the `build_*` catalog helpers already do), so it is safe to call at
/// startup, on test-play start, and on editor → title return.
pub fn build_runtime(project: &Project) -> RuntimeBundle {
    // Doors start closed unless the level's `!`/`@` glyphs mark a kind open (v6).
    let doors = crate::world::doors_for(&project.levels[0], None, project.limits.door_kinds_per_level);
    let dungeon = project.levels[0].to_dungeon();
    let party = project.build_party();
    // The pristine run captured for「はじめから」/ ED-demo resets (plan11).
    let initial_run = InitialRun {
        party: Party { members: party.members.clone() },
        initial_flags: project.initial_flags.clone(),
        start: project.levels[0].start,
        facing: project.levels[0].start_facing,
    };
    let mut event_flags = EventFlags::new(project.limits.event_flags);
    for &f in &project.initial_flags {
        event_flags.set(f, true); // plan9: initial-on flags
    }
    RuntimeBundle {
        limits: project.limits.clone(),
        rules: project.rules.clone(),
        dungeon,
        doors,
        party,
        catalog: project.build_catalog(),
        monster_catalog: project.build_monster_catalog(),
        magic_catalog: project.build_magic_catalog(),
        initial_items: InitialItems(project.levels[0].items.clone()),
        initial_monsters: InitialMonsters(project.levels[0].monsters.clone()),
        event_flags,
        game_levels: GameLevels { levels: project.levels.clone() },
        initial_run,
        demo_catalog: DemoCatalog(project.demos.clone()),
        resolver: AssetResolver { project_dir: project.dir.clone() },
        game_title: project.name.clone(),
        game_author: project.author.clone(),
        game_desc: project.description.clone(),
    }
}

impl RuntimeBundle {
    /// Insert the derived resources into the App at startup (initial play build).
    pub fn insert(self, app: &mut App) {
        app.insert_resource(self.limits)
            .insert_resource(self.rules)
            .insert_resource(self.dungeon)
            .insert_resource(self.doors)
            .insert_resource(self.party)
            .insert_resource(self.catalog)
            .insert_resource(self.monster_catalog)
            .insert_resource(self.magic_catalog)
            .insert_resource(self.initial_items)
            .insert_resource(self.initial_monsters)
            .insert_resource(self.event_flags)
            .insert_resource(self.game_levels)
            .insert_resource(self.initial_run)
            .insert_resource(self.demo_catalog)
            .insert_resource(self.resolver);
    }

    /// Rewrite a running world with a freshly-derived runtime and rebuild it from
    /// level 0's start — the same "restore globals, then transition" machinery a
    /// reset/load uses (plan10/plan11), but also swapping the catalogs and levels
    /// because the *project itself* changed (editor test-play / title return,
    /// plan13). Every level-scoped entity is despawned and rebuilt by the
    /// `LevelTransition` this sends.
    pub fn apply(self, w: &mut ApplyWorld, transition: &mut EventWriter<LevelTransition>) {
        let start = self.initial_run.start;
        let facing = self.initial_run.facing;
        // Level 0's BGM (the world rebuilds at level 0). Captured before the move.
        let bgm_track = self.game_levels.levels.first().map(|l| l.bgm.clone()).unwrap_or_default();
        // Swap the project-derived catalogs / levels / limits / rules.
        *w.limits = self.limits;
        *w.rules = self.rules;
        *w.catalog = self.catalog;
        *w.monster_catalog = self.monster_catalog;
        *w.magic_catalog = self.magic_catalog;
        *w.initial_items = self.initial_items;
        *w.initial_monsters = self.initial_monsters;
        *w.game_levels = self.game_levels;
        *w.demo_catalog = self.demo_catalog;
        *w.resolver = self.resolver;
        *w.initial_run = self.initial_run;
        *w.party = self.party;
        // Restore every mutable global to its authored initial value (reset).
        *w.event_flags = self.event_flags;
        w.states.map.clear();
        w.queue.pending.clear();
        w.writes.map.clear();
        w.clock.restore(0);
        *w.rng = GameRng::default();
        w.bgm.override_track = None;
        w.bgm.level_track = bgm_track;
        *w.move_mode = default();
        w.data.open = false;
        // The pre-transition runtime state must not clobber the fresh LevelStates.
        w.skip_snapshot.0 = true;
        transition.send(LevelTransition { to_level: 0, to: start, to_facing: facing });
    }
}

/// Everything [`RuntimeBundle::apply`] overwrites when the editor rebuilds the
/// world, bundled to fit the system parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ApplyWorld<'w> {
    pub limits: ResMut<'w, LimitsConfig>,
    pub rules: ResMut<'w, RulesConfig>,
    pub catalog: ResMut<'w, ItemCatalog>,
    pub monster_catalog: ResMut<'w, MonsterCatalog>,
    pub magic_catalog: ResMut<'w, MagicCatalog>,
    pub initial_items: ResMut<'w, InitialItems>,
    pub initial_monsters: ResMut<'w, InitialMonsters>,
    pub game_levels: ResMut<'w, GameLevels>,
    pub demo_catalog: ResMut<'w, DemoCatalog>,
    pub resolver: ResMut<'w, AssetResolver>,
    pub initial_run: ResMut<'w, InitialRun>,
    pub party: ResMut<'w, Party>,
    pub event_flags: ResMut<'w, EventFlags>,
    pub states: ResMut<'w, LevelStates>,
    pub queue: ResMut<'w, EventQueue>,
    pub writes: ResMut<'w, WallWrites>,
    pub clock: ResMut<'w, crate::clock::GameClock>,
    pub rng: ResMut<'w, GameRng>,
    pub bgm: ResMut<'w, BgmState>,
    pub move_mode: ResMut<'w, MoveMode>,
    pub data: ResMut<'w, DataScreen>,
    pub skip_snapshot: ResMut<'w, SkipNextSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// build_runtime derives level 0's dungeon, party, initial flags and title
    /// metadata straight from the project (the invariant the editor rebuild and
    /// the startup insert both rely on).
    #[test]
    fn build_runtime_derives_from_project() {
        let project = crate::project::load_project(std::path::Path::new(crate::DEFAULT_PROJECT))
            .expect("sample project loads");
        let rt = build_runtime(&project);
        assert_eq!(rt.game_title, project.name);
        assert_eq!(rt.game_levels.levels.len(), project.levels.len());
        assert_eq!(rt.dungeon.start_pos, project.levels[0].start);
        assert_eq!(rt.initial_run.start, project.levels[0].start);
        assert_eq!(rt.party.members.len(), project.party.len());
        // Initial-on flags are reflected in the derived EventFlags.
        for &f in &project.initial_flags {
            assert!(rt.event_flags.get(f));
        }
    }
}
