//! User settings (plan9): rebindable movement keys, persisted to
//! `user_settings.ron` at the repo root. This is **user** config, not project
//! data (it's `.gitignore`d), so it lives outside the project format.
//!
//! The keybind table is the single source of truth `desired_command` consults;
//! swapping a binding changes which `Command` a key produces (unit-tested).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::player::{Action, Command};

/// A rebindable movement/climb action.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum GameAction {
    Forward,
    Backward,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
    ClimbUp,
    ClimbDown,
}

impl GameAction {
    /// The movement command this action issues.
    pub fn command(self) -> Command {
        match self {
            GameAction::Forward => Command::Move(Action::Forward),
            GameAction::Backward => Command::Move(Action::Backward),
            GameAction::StrafeLeft => Command::Move(Action::StrafeLeft),
            GameAction::StrafeRight => Command::Move(Action::StrafeRight),
            GameAction::TurnLeft => Command::Move(Action::TurnLeft),
            GameAction::TurnRight => Command::Move(Action::TurnRight),
            GameAction::ClimbUp => Command::ClimbUp,
            GameAction::ClimbDown => Command::ClimbDown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GameAction::Forward => "前進",
            GameAction::Backward => "後退",
            GameAction::StrafeLeft => "左移動",
            GameAction::StrafeRight => "右移動",
            GameAction::TurnLeft => "左回転",
            GameAction::TurnRight => "右回転",
            GameAction::ClimbUp => "上る",
            GameAction::ClimbDown => "下る",
        }
    }

    /// Priority order (checked first-to-last, matching the original hardcoding).
    pub const ORDER: [GameAction; 8] = [
        GameAction::Forward,
        GameAction::Backward,
        GameAction::StrafeLeft,
        GameAction::StrafeRight,
        GameAction::TurnLeft,
        GameAction::TurnRight,
        GameAction::ClimbUp,
        GameAction::ClimbDown,
    ];
}

/// The live key bindings (a Bevy resource). Multiple keys may map to one action;
/// `command_for` returns the highest-priority action whose key is active.
#[derive(Resource, Clone, Debug)]
pub struct Keybinds {
    /// `(action, key)` pairs, kept in [`GameAction::ORDER`] priority order.
    pub binds: Vec<(GameAction, KeyCode)>,
}

impl Default for Keybinds {
    /// The original hardcoded layout (WASD/QE + arrows + R/F).
    fn default() -> Self {
        use GameAction::*;
        use KeyCode::*;
        Self {
            binds: vec![
                (Forward, KeyW),
                (Forward, ArrowUp),
                (Backward, KeyS),
                (Backward, ArrowDown),
                (StrafeLeft, KeyA),
                (StrafeRight, KeyD),
                (TurnLeft, KeyQ),
                (TurnLeft, ArrowLeft),
                (TurnRight, KeyE),
                (TurnRight, ArrowRight),
                (ClimbUp, KeyR),
                (ClimbDown, KeyF),
            ],
        }
    }
}

impl Keybinds {
    /// The first bound command whose key satisfies `is_active`, in the priority
    /// order of [`GameAction::ORDER`]. Ties within an action fall to whichever
    /// key comes first in `binds`.
    pub fn command_for(&self, mut is_active: impl FnMut(KeyCode) -> bool) -> Option<Command> {
        for action in GameAction::ORDER {
            if self
                .binds
                .iter()
                .any(|(a, k)| *a == action && is_active(*k))
            {
                return Some(action.command());
            }
        }
        None
    }

    /// Replace every binding for `action` with a single `key` (used by the
    /// settings UI / when applying user overrides).
    pub fn rebind(&mut self, action: GameAction, key: KeyCode) {
        self.binds.retain(|(a, _)| *a != action);
        // Insert keeping ORDER priority stable enough for lookups.
        self.binds.push((action, key));
    }

    /// Build from defaults, then apply the user's overrides (each replaces its
    /// action's keys). Falls back to defaults when the file is missing/invalid.
    pub fn load() -> Self {
        let mut binds = Keybinds::default();
        for (action, name) in UserSettings::load().keybinds {
            if let Some(key) = key_by_name(&name) {
                binds.rebind(action, key);
            }
        }
        binds
    }

    /// Persist the current bindings as user overrides (one key per action —
    /// the last-bound key wins for display/persistence). Rewrites only the
    /// keybind field; audio/speed settings in the same file are preserved.
    pub fn save(&self) -> Result<(), String> {
        let mut keybinds: Vec<(GameAction, String)> = Vec::new();
        for action in GameAction::ORDER {
            if let Some((_, key)) = self.binds.iter().rev().find(|(a, _)| *a == action)
                && let Some(name) = key_name(*key)
            {
                keybinds.push((action, name.to_string()));
            }
        }
        let mut settings = UserSettings::load();
        settings.keybinds = keybinds;
        settings.save()
    }
}

/// Repo-root user settings file (gitignored).
const USER_SETTINGS_PATH: &str = "user_settings.ron";

/// On-disk user settings (plan10: audio volumes / footsteps / game speed join
/// the plan9 keybinds). This is a Bevy resource too — play mode inserts the
/// loaded value so audio and the clock read the user's preferences. All fields
/// default individually, so files written by older builds still parse.
#[derive(Resource, Serialize, Deserialize, Clone)]
pub struct UserSettings {
    /// One key name per overridden action.
    #[serde(default)]
    pub keybinds: Vec<(GameAction, String)>,
    /// BGM volume, `0.0..=1.0`.
    #[serde(default = "default_volume")]
    pub bgm_volume: f32,
    /// Sound-effect volume, `0.0..=1.0`.
    #[serde(default = "default_volume")]
    pub se_volume: f32,
    /// Master mute (overrides both volumes).
    #[serde(default)]
    pub mute: bool,
    /// Whether footstep sounds play.
    #[serde(default = "default_true")]
    pub footsteps: bool,
    /// Game-time speed multiplier (0.5 / 1.0 / 2.0), applied in `tick_clock`.
    #[serde(default = "default_speed")]
    pub speed: f32,
}

fn default_volume() -> f32 {
    0.8
}
fn default_true() -> bool {
    true
}
fn default_speed() -> f32 {
    1.0
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            keybinds: Vec::new(),
            bgm_volume: default_volume(),
            se_volume: default_volume(),
            mute: false,
            footsteps: default_true(),
            speed: default_speed(),
        }
    }
}

impl UserSettings {
    /// Read `user_settings.ron`, falling back to defaults if missing/invalid.
    pub fn load() -> Self {
        std::fs::read_to_string(USER_SETTINGS_PATH)
            .ok()
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("serialize settings: {e}"))?;
        std::fs::write(USER_SETTINGS_PATH, text).map_err(|e| format!("write settings: {e}"))
    }
}

/// The keys the settings UI can bind, as `(name, KeyCode)`. A modest,
/// serialisation-stable subset (Bevy's `KeyCode` isn't serde by default here).
pub const BINDABLE_KEYS: &[(&str, KeyCode)] = &[
    ("W", KeyCode::KeyW),
    ("A", KeyCode::KeyA),
    ("S", KeyCode::KeyS),
    ("D", KeyCode::KeyD),
    ("Q", KeyCode::KeyQ),
    ("E", KeyCode::KeyE),
    ("R", KeyCode::KeyR),
    ("F", KeyCode::KeyF),
    ("Up", KeyCode::ArrowUp),
    ("Down", KeyCode::ArrowDown),
    ("Left", KeyCode::ArrowLeft),
    ("Right", KeyCode::ArrowRight),
    ("Num8", KeyCode::Numpad8),
    ("Num2", KeyCode::Numpad2),
    ("Num4", KeyCode::Numpad4),
    ("Num6", KeyCode::Numpad6),
    ("Num7", KeyCode::Numpad7),
    ("Num9", KeyCode::Numpad9),
    ("Num1", KeyCode::Numpad1),
    ("Num3", KeyCode::Numpad3),
];

pub fn key_by_name(name: &str) -> Option<KeyCode> {
    BINDABLE_KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

pub fn key_name(key: KeyCode) -> Option<&'static str> {
    BINDABLE_KEYS.iter().find(|(_, k)| *k == key).map(|(n, _)| *n)
}

/// In-game keybind config mode (plan9). `O` toggles it on / advances the selected
/// action; the next bindable key rebinds that action and saves `user_settings.ron`.
/// `Esc` exits. A lightweight flow (no title screen yet); prompts go to the log.
#[derive(Resource, Default)]
pub struct KeyConfig {
    pub active: bool,
    pub sel: usize,
}

/// Drive keybind config from the keyboard (plan9). Kept out of `player_movement`
/// so it can rebind the very keys movement reads.
pub fn keyconfig_input(
    keys: Res<ButtonInput<KeyCode>>,
    data: Res<crate::game_state::DataScreen>,
    mut cfg: ResMut<KeyConfig>,
    mut binds: ResMut<Keybinds>,
    mut log: ResMut<crate::hud::MessageLog>,
) {
    if data.open {
        return; // don't rebind while the data screen captures input
    }
    let cur_key_name = |binds: &Keybinds, a: GameAction| -> String {
        binds
            .binds
            .iter()
            .rev()
            .find(|(x, _)| *x == a)
            .and_then(|(_, k)| key_name(*k))
            .unwrap_or("―")
            .to_string()
    };
    if keys.just_pressed(KeyCode::KeyO) {
        if !cfg.active {
            cfg.active = true;
            cfg.sel = 0;
        } else {
            cfg.sel = (cfg.sel + 1) % GameAction::ORDER.len();
        }
        let a = GameAction::ORDER[cfg.sel];
        log.push(format!("キー設定: {} (現在 {}) — 割当てるキーを押す / O次へ / Esc終了", a.label(), cur_key_name(&binds, a)));
        return;
    }
    if !cfg.active {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cfg.active = false;
        log.push("キー設定を終了");
        return;
    }
    for (name, key) in BINDABLE_KEYS {
        if keys.just_pressed(*key) {
            let action = GameAction::ORDER[cfg.sel];
            binds.rebind(action, *key);
            match binds.save() {
                Ok(()) => log.push(format!("{} を {} に割当てた (保存)", action.label(), name)),
                Err(e) => log.push(format!("{} を {} に割当て (保存失敗: {e})", action.label(), name)),
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_binds_map_wasd() {
        let kb = Keybinds::default();
        assert_eq!(kb.command_for(|k| k == KeyCode::KeyW), Some(Command::Move(Action::Forward)));
        assert_eq!(kb.command_for(|k| k == KeyCode::ArrowUp), Some(Command::Move(Action::Forward)));
        assert_eq!(kb.command_for(|k| k == KeyCode::KeyE), Some(Command::Move(Action::TurnRight)));
        assert_eq!(kb.command_for(|_| false), None);
    }

    #[test]
    fn rebind_changes_command() {
        // Bind Forward to Numpad8; W no longer moves forward.
        let mut kb = Keybinds::default();
        kb.rebind(GameAction::Forward, KeyCode::Numpad8);
        assert_eq!(kb.command_for(|k| k == KeyCode::Numpad8), Some(Command::Move(Action::Forward)));
        // W is now unbound → no command from W alone.
        assert_eq!(kb.command_for(|k| k == KeyCode::KeyW), None);
    }

    #[test]
    fn key_name_round_trips() {
        for (name, key) in BINDABLE_KEYS {
            assert_eq!(key_by_name(name), Some(*key));
            assert_eq!(key_name(*key), Some(*name));
        }
    }
}
