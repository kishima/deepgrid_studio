//! Which full-screen UI owns the screen right now (plan11).
//!
//! Play mode has four mutually exclusive "screens": the title overlay, a demo
//! playback, the data screen, and normal play. [`active_screen`] is the single
//! priority rule every system must consult (directly or through the
//! [`CurrentScreen`] system param) instead of reading the individual gate
//! resources — plan12 will replace this helper with Bevy `States`, and keeping
//! the judgement in one function is what makes that a one-file swap.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::demo::DemoState;
use crate::game_state::DataScreen;
use crate::title::TitleState;

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
}

/// The one priority rule: Title > Demo > Data > Play. Systems that already hold
/// (possibly mutable) references to the gate resources call this directly;
/// read-only systems use [`CurrentScreen`].
pub fn active_screen(title: &TitleState, demo: &DemoState, data: &DataScreen) -> ActiveScreen {
    if title.active {
        ActiveScreen::Title
    } else if demo.playing() {
        ActiveScreen::Demo
    } else if data.open {
        ActiveScreen::Data
    } else {
        ActiveScreen::Play
    }
}

/// Read-only view of the active screen for systems that don't mutate any of the
/// gate resources.
#[derive(SystemParam)]
pub struct CurrentScreen<'w> {
    title: Res<'w, TitleState>,
    demo: Res<'w, DemoState>,
    data: Res<'w, DataScreen>,
}

impl CurrentScreen<'_> {
    pub fn get(&self) -> ActiveScreen {
        active_screen(&self.title, &self.demo, &self.data)
    }

    /// Game time (the cycle clock) runs only in Play / Data — the data screen
    /// deliberately keeps the world simulating (plan5), while the title and
    /// demos freeze it.
    pub fn freezes_clock(&self) -> bool {
        matches!(self.get(), ActiveScreen::Title | ActiveScreen::Demo)
    }
}
