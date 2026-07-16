//! Which full-screen UI owns the screen right now (plan12).
//!
//! Play mode's top-level screens are a Bevy [`States`] value — [`GameScreen`]
//! (Title / Demo / Playing). Transitions go through `NextState<GameScreen>` and
//! per-screen construction/teardown hangs off `OnEnter`/`OnExit`, so the second
//! wave of screens (options, a wasm menu, …) can be added declaratively.
//!
//! The data screen is deliberately **not** a state: it is an overlay that keeps
//! the world simulating (plan5), and its open/close is routed *within a single
//! frame* (Tab toggles `data.open`, later systems branch on it the same frame).
//! `NextState` only lands at the next `StateTransition`, which would break that
//! same-frame semantics — so `DataScreen` stays a plain resource and
//! [`ActiveScreen::Data`] is the *derived* value "Playing and the overlay is
//! open". [`active_screen`] keeps the one priority rule (Title > Demo > Data >
//! Play) that every system consults directly or through [`CurrentScreen`].

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game_state::DataScreen;

/// Play mode's top-level screen (Bevy `States`, plan12). Startup picks the
/// initial value with `insert_state` — `Playing` for unattended verification /
/// `--load`, `Title` for a normal launch — and every later change goes through
/// `NextState<GameScreen>`.
#[derive(States, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GameScreen {
    /// The title menu covers everything (plan11).
    #[default]
    Title,
    /// A demo cutscene is playing (plan10).
    Demo,
    /// Normal play (the data-screen overlay, when open, lives on top of this).
    Playing,
    /// The Studio editor (plan13). A single-process sibling of play: the play
    /// world is torn down on `OnEnter` and rebuilt on `OnExit`, and only the
    /// editor's egui / edit3d systems run here.
    Editor,
}

/// Run condition (plan13): true in every play-mode state (Title / Demo /
/// Playing), false in [`GameScreen::Editor`]. The play world's Update systems
/// hang off this so the editor state is the sole writer of `Dungeon` / `Palette`
/// / `TileDirty` while it owns the window.
pub fn not_editor(screen: Res<State<GameScreen>>) -> bool {
    !matches!(*screen.get(), GameScreen::Editor)
}

/// The screen that owns input right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActiveScreen {
    /// The title menu covers everything (plan11).
    Title,
    /// A demo cutscene is playing (plan10).
    Demo,
    /// The data screen overlay is open (the world keeps simulating).
    Data,
    /// Normal play.
    Play,
    /// The Studio editor (plan13).
    Editor,
}

/// The one priority rule: Title > Demo > Editor > Data > Play. `Data` is the
/// derived overlay state "`GameScreen::Playing` and the data screen is open";
/// everything else follows directly from the state.
pub fn active_screen(screen: GameScreen, data: &DataScreen) -> ActiveScreen {
    match screen {
        GameScreen::Title => ActiveScreen::Title,
        GameScreen::Demo => ActiveScreen::Demo,
        GameScreen::Editor => ActiveScreen::Editor,
        GameScreen::Playing if data.open => ActiveScreen::Data,
        GameScreen::Playing => ActiveScreen::Play,
    }
}

/// Read-only view of the active screen for systems that only need the priority
/// verdict (the state plus the data-screen overlay), not the raw state resource.
#[derive(SystemParam)]
pub struct CurrentScreen<'w> {
    screen: Res<'w, State<GameScreen>>,
    data: Res<'w, DataScreen>,
}

impl CurrentScreen<'_> {
    pub fn get(&self) -> ActiveScreen {
        active_screen(*self.screen.get(), &self.data)
    }

    /// Game time (the cycle clock) runs only in Play / Data — the data screen
    /// deliberately keeps the world simulating (plan5), while the title and
    /// demos freeze it.
    pub fn freezes_clock(&self) -> bool {
        matches!(self.get(), ActiveScreen::Title | ActiveScreen::Demo | ActiveScreen::Editor)
    }
}
