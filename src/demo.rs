//! Demos (plan10): full-screen message-scroll cutscenes (OP / ED / mid-game),
//! authored in `demos.ron` and started by the `StartDemo` event action.
//!
//! Playback is the [`GameScreen::Demo`] state (plan12): while it is active
//! `tick_clock` emits no `CycleTick` (stopping monsters / hazards / queued
//! events) and `player_movement` ignores input. [`DemoState`] carries only the
//! playback *progress*; `start_demo` performs the transition *into* Demo (from
//! Title or Playing) and `drive_demo` the transition out. The overlay is bevy_ui
//! (so the `demo` debug shot captures it) and is torn down by [`teardown_demo`]
//! on `OnExit(Demo)`. Click / Space advance a line early; Escape skips to the
//! end. After the last line an "— END —" marker waits for one input, then the
//! overlay closes and play resumes (or, for the ED demo, the title reopens).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audio::BgmState;
use crate::screen::GameScreen;

/// Seconds a line stays before auto-advancing.
const LINE_SECS: f32 = 2.5;
/// How many trailing lines stay visible on the overlay.
const VISIBLE_LINES: usize = 14;

/// One authored demo (`demos.ron`, `Vec<DemoDef>`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DemoDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The lines shown one by one (limit: `limits.demo_message_lines`).
    pub lines: Vec<String>,
    /// BGM file name under `assets/audio/bgm/` while playing ("" = keep).
    #[serde(default)]
    pub bgm: String,
    /// Overlay background colour (r, g, b in 0..=1).
    #[serde(default)]
    pub bg_color: (f32, f32, f32),
}

/// All demos of the loaded project (runtime resource).
#[derive(Resource, Default, Clone)]
pub struct DemoCatalog(pub Vec<DemoDef>);

/// A request to start a demo by id (sent by `run_events`' `StartDemo`).
#[derive(Event, Clone, Debug)]
pub struct StartDemoReq(pub String);

/// The live playback state. `None` = no demo running.
#[derive(Resource, Default)]
pub struct DemoState {
    pub active: Option<ActiveDemo>,
}

pub struct ActiveDemo {
    pub id: String,
    /// Lines revealed so far (1 = first line visible).
    pub shown: usize,
    /// Seconds until the next auto-advance.
    pub timer: f32,
    /// All lines are out and the END marker waits for input.
    pub at_end: bool,
    /// The BGM override that was in effect before the demo (restored on close).
    pub prev_override: Option<String>,
}

impl DemoState {
    pub fn playing(&self) -> bool {
        self.active.is_some()
    }
}

/// Marks the overlay root (despawned when the demo ends).
#[derive(Component)]
pub struct DemoOverlay;

/// Marks the text node whose content follows the revealed lines.
#[derive(Component)]
pub struct DemoText;

/// Start requested demos: set up [`DemoState`], swap the BGM, spawn the overlay,
/// and enter the Demo screen.
#[allow(clippy::too_many_arguments)]
pub fn start_demo(
    mut commands: Commands,
    mut reqs: EventReader<StartDemoReq>,
    catalog: Res<DemoCatalog>,
    mut state: ResMut<DemoState>,
    mut bgm: ResMut<BgmState>,
    mut log: ResMut<crate::hud::MessageLog>,
    mut next_screen: ResMut<NextState<GameScreen>>,
    existing: Query<Entity, With<DemoOverlay>>,
) {
    let Some(req) = reqs.read().last() else { return };
    let Some(def) = catalog.0.iter().find(|d| d.id == req.0) else {
        log.push(format!("デモ「{}」が みつからない", req.0));
        return;
    };
    // Restarting while one runs: tear the old overlay down first.
    for e in &existing {
        commands.entity(e).despawn_recursive();
    }
    let prev_override = bgm.override_track.clone();
    if !def.bgm.is_empty() {
        bgm.override_track = Some(def.bgm.clone());
    }
    state.active = Some(ActiveDemo {
        id: def.id.clone(),
        shown: 1.min(def.lines.len()),
        timer: LINE_SECS,
        at_end: def.lines.is_empty(),
        prev_override,
    });
    // Enter the Demo screen (from Title on「はじめから」, or from Playing on a
    // mid-game StartDemo event). start_demo is the sole authority for this
    // transition, so it can only ever fire when a demo was actually set up.
    next_screen.set(GameScreen::Demo);
    let (r, g, b) = def.bg_color;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgb(r, g, b)),
            GlobalZIndex(100),
            DemoOverlay,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(String::new()),
                TextFont { font_size: 26.0, ..default() },
                TextColor(Color::srgb(0.92, 0.92, 0.88)),
                TextLayout::new_with_justify(JustifyText::Center),
                DemoText,
            ));
        });
}

/// Advance the running demo: timed line reveal, click/Space to hurry, Escape to
/// skip to the END marker, any input at END to close. Closing the `"ed"` demo
/// returns to the title with the world rebuilt from scratch (plan11 — the
/// provisional "game clear"); every other demo resumes play.
#[allow(clippy::too_many_arguments)]
pub fn drive_demo(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    catalog: Res<DemoCatalog>,
    mut state: ResMut<DemoState>,
    mut bgm: ResMut<BgmState>,
    mut title: ResMut<crate::title::TitleState>,
    mut reset: EventWriter<crate::title::ResetRunReq>,
    mut next_screen: ResMut<NextState<GameScreen>>,
    test_play: Res<crate::editor::TestPlay>,
    mut text: Query<&mut Text, With<DemoText>>,
) {
    let Some(active) = &mut state.active else { return };
    let Some(def) = catalog.0.iter().find(|d| d.id == active.id) else {
        // Demo id vanished from the catalog: resume play (the overlay is torn
        // down on OnExit(Demo)).
        state.active = None;
        next_screen.set(GameScreen::Playing);
        return;
    };
    let advance = keys.just_pressed(KeyCode::Space) || mouse.just_pressed(MouseButton::Left);
    let skip = keys.just_pressed(KeyCode::Escape);

    if active.at_end {
        // END marker: one more input closes the overlay and resumes play —
        // except the ED demo, which resets the run and returns to the title.
        if advance || skip {
            let was_ed = active.id == "ed";
            bgm.override_track = active.prev_override.clone();
            state.active = None;
            if was_ed {
                if test_play.0 {
                    // plan13: an ED reached while test-playing returns to the
                    // editor (the play world is discarded, not reset for title).
                    next_screen.set(GameScreen::Editor);
                } else {
                    reset.send(crate::title::ResetRunReq);
                    title.open();
                    next_screen.set(GameScreen::Title);
                }
            } else {
                next_screen.set(GameScreen::Playing);
            }
            return;
        }
    } else if skip {
        active.shown = def.lines.len();
        active.at_end = true;
    } else {
        active.timer -= time.delta_secs();
        if advance || active.timer <= 0.0 {
            active.timer = LINE_SECS;
            if active.shown < def.lines.len() {
                active.shown += 1;
            } else {
                active.at_end = true;
            }
        }
    }

    // Render the tail of the revealed lines (+ END marker).
    if let Ok(mut t) = text.get_single_mut() {
        let shown = &def.lines[..active.shown.min(def.lines.len())];
        let start = shown.len().saturating_sub(VISIBLE_LINES);
        let mut s = shown[start..].join("\n");
        if active.at_end {
            if !s.is_empty() {
                s.push_str("\n\n");
            }
            s.push_str("— END —");
        }
        **t = s;
    }
}

/// Tear the overlay down when leaving demo playback (plan12, `OnExit(Demo)`).
/// `drive_demo` / the autotest close paths clear `DemoState` and drive the
/// transition; the overlay entity is removed here.
pub fn teardown_demo(mut commands: Commands, overlay: Query<Entity, With<DemoOverlay>>) {
    for e in &overlay {
        commands.entity(e).despawn_recursive();
    }
}

/// `DEEPGRID_DEBUG_SHOT=demo` driver: start the sample's "op" demo once the
/// scene has settled, so the screenshot captures the playback overlay (bevy_ui
/// IS captured by Bevy's `Screenshot`, unlike egui).
pub fn debug_demo_driver(
    mut frames: Local<u32>,
    mut fired: Local<bool>,
    mut req: EventWriter<StartDemoReq>,
) {
    if *fired || crate::debug_shot::debug_shot_value().as_deref() != Some("demo") {
        return;
    }
    *frames += 1;
    if *frames >= 10 {
        req.send(StartDemoReq("op".to_string()));
        *fired = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_def_round_trips() {
        let d = DemoDef {
            id: "op".into(),
            name: "オープニング".into(),
            lines: vec!["やみの ダンジョンに".into(), "ゆうしゃは たった".into()],
            bgm: "op.ogg".into(),
            bg_color: (0.0, 0.0, 0.1),
        };
        let text = ron::ser::to_string(&vec![d.clone()]).unwrap();
        let back: Vec<DemoDef> = ron::from_str(&text).unwrap();
        assert_eq!(back, vec![d]);
    }

    #[test]
    fn minimal_demo_parses_with_defaults() {
        // Only id + lines: name/bgm/bg_color default (serde-default fields).
        let back: Vec<DemoDef> =
            ron::from_str(r#"[(id: "mid", lines: ["a"])]"#).unwrap();
        assert_eq!(back[0].bgm, "");
        assert_eq!(back[0].bg_color, (0.0, 0.0, 0.0));
    }
}
