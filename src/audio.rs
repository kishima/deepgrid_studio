//! Audio (plan10): sound effects and per-level BGM with crossfade.
//!
//! Design constraints:
//! - **No audio device must never crash** (docker/CI): Bevy's audio plugin only
//!   warns, and everything here tolerates `AudioSink` never appearing — fades
//!   are timed on our own `gain`, not on sink state. Autotest asserts on
//!   [`BgmState`] and channel entities, not on audible output.
//! - SE requests go through the [`PlaySe`] event so game systems don't touch
//!   asset/entity plumbing; duplicate requests of one sound within a frame are
//!   collapsed (many monsters hit at once ≠ clipping).
//! - User preferences (volumes / mute / footsteps) come from
//!   [`UserSettings`](crate::settings::UserSettings) (`user_settings.ron`).

use bevy::audio::{AudioSink, PlaybackMode, Volume};
use bevy::prelude::*;

use crate::settings::UserSettings;
use crate::world::{CurrentLevel, GameLevels};

/// Seconds a BGM crossfade takes.
const CROSSFADE_SECS: f32 = 1.0;

// ------------------------------------------------------------------ sound effects

/// Every sound effect the game can request (plan10).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Se {
    Footstep,
    DoorOpen,
    DoorClose,
    AttackHit,
    Cast,
    Impact,
    Land,
    Pickup,
    LevelUp,
    Switch,
    Warp,
}

impl Se {
    /// Asset path of this effect's ogg (see assets/audio/README.md / CREDITS.md).
    fn path(self) -> &'static str {
        match self {
            Se::Footstep => "audio/se/footstep.ogg",
            Se::DoorOpen => "audio/se/door_open.ogg",
            Se::DoorClose => "audio/se/door_close.ogg",
            Se::AttackHit => "audio/se/melee_hit.ogg",
            Se::Cast => "audio/se/spell_cast.ogg",
            Se::Impact => "audio/se/magic_impact.ogg",
            Se::Land => "audio/se/fall_thud.ogg",
            Se::Pickup => "audio/se/item_pickup.ogg",
            Se::LevelUp => "audio/se/level_up.ogg",
            Se::Switch => "audio/se/switch_click.ogg",
            Se::Warp => "audio/se/warp.ogg",
        }
    }
}

/// Request one sound effect this frame.
#[derive(Event, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlaySe(pub Se);

/// Play this frame's requested effects (deduplicated per kind).
pub fn play_se(
    mut commands: Commands,
    mut reader: EventReader<PlaySe>,
    settings: Res<UserSettings>,
    asset_server: Res<AssetServer>,
) {
    let mut played: Vec<Se> = Vec::new();
    for ev in reader.read() {
        if played.contains(&ev.0) {
            continue;
        }
        played.push(ev.0);
        if settings.mute || settings.se_volume <= 0.0 {
            continue;
        }
        if ev.0 == Se::Footstep && !settings.footsteps {
            continue;
        }
        commands.spawn((
            AudioPlayer::new(asset_server.load(ev.0.path())),
            PlaybackSettings {
                mode: PlaybackMode::Despawn,
                volume: Volume::new(settings.se_volume),
                ..default()
            },
        ));
    }
}

/// Play the level-up jingle when any member's level rises (watching the party
/// keeps `gain_exp`'s many call sites free of audio plumbing).
pub fn level_up_se(
    mut last: Local<Vec<u32>>,
    party: Res<crate::character::Party>,
    mut se: EventWriter<PlaySe>,
) {
    let levels: Vec<u32> = party.members.iter().map(|m| m.character.stats.level).collect();
    if last.len() == levels.len() && levels.iter().zip(last.iter()).any(|(n, o)| n > o) {
        se.send(PlaySe(Se::LevelUp));
    }
    *last = levels;
}

// ------------------------------------------------------------------ BGM

/// What should be playing: the current level's track, unless a `ChangeBgm`
/// override is active (an override lasts until the next level move or the next
/// `ChangeBgm`). Track values are bare file names under `assets/audio/bgm/`
/// (empty = silence). Autotest asserts on this resource.
#[derive(Resource, Default, Debug)]
pub struct BgmState {
    pub level_track: String,
    pub override_track: Option<String>,
}

impl BgmState {
    /// The track that ought to be audible right now ("" = silence).
    pub fn desired(&self) -> &str {
        self.override_track.as_deref().unwrap_or(&self.level_track)
    }
}

/// One playing BGM entity. `gain` ramps 0→1 while `active`, 1→0 after another
/// track takes over; the entity despawns when fully faded out.
#[derive(Component)]
pub struct BgmChannel {
    pub track: String,
    pub active: bool,
    pub gain: f32,
}

/// Follow level changes: the level's own track becomes current and any
/// `ChangeBgm` override ends (plan10 spec: override lasts until the next level
/// move).
pub fn sync_level_bgm(
    mut last: Local<Option<usize>>,
    current: Res<CurrentLevel>,
    levels: Res<GameLevels>,
    mut state: ResMut<BgmState>,
) {
    if *last == Some(current.0) {
        return;
    }
    *last = Some(current.0);
    state.level_track = levels.levels.get(current.0).map(|l| l.bgm.clone()).unwrap_or_default();
    state.override_track = None;
}

/// Keep the playing channels in step with [`BgmState::desired`]: spawn the
/// desired track at gain 0, fade it in over [`CROSSFADE_SECS`] while everything
/// else fades out and despawns.
pub fn update_bgm(
    mut commands: Commands,
    settings: Res<UserSettings>,
    state: Res<BgmState>,
    asset_server: Res<AssetServer>,
    time: Res<Time>,
    mut channels: Query<(Entity, &mut BgmChannel, Option<&AudioSink>)>,
) {
    let desired = state.desired().to_string();
    let mut have_desired = false;
    let step = time.delta_secs() / CROSSFADE_SECS;
    for (e, mut ch, sink) in &mut channels {
        if ch.track == desired {
            ch.active = true;
            have_desired = true;
        } else {
            ch.active = false;
        }
        ch.gain = if ch.active { (ch.gain + step).min(1.0) } else { ch.gain - step };
        if ch.gain <= 0.0 && !ch.active {
            commands.entity(e).despawn_recursive();
            continue;
        }
        if let Some(sink) = sink {
            let master = if settings.mute { 0.0 } else { settings.bgm_volume };
            sink.set_volume(ch.gain.max(0.0) * master);
        }
    }
    if !desired.is_empty() && !have_desired {
        commands.spawn((
            AudioPlayer::new(asset_server.load(format!("audio/bgm/{desired}"))),
            PlaybackSettings {
                mode: PlaybackMode::Loop,
                volume: Volume::new(0.0),
                ..default()
            },
            BgmChannel { track: desired, active: true, gain: 0.0 },
        ));
    }
}
