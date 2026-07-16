//! Frame-time measurement mode (plan11, `DEEPGRID_PERF`).
//!
//! With `DEEPGRID_PERF` set, play mode skips the title, warms up for a couple of
//! seconds, measures every frame for the configured duration, prints an
//! average / worst summary to stdout and exits. `DEEPGRID_PERF=1` (or any
//! non-numeric value) measures for the default 10 s; a value ≥ 2 is the
//! duration in seconds.

use bevy::app::AppExit;
use bevy::prelude::*;

/// Seconds ignored before measurement starts (asset loads, shader warmup).
const WARMUP_SECS: f64 = 2.0;
/// Default measurement window.
const DEFAULT_SECS: f64 = 10.0;

/// Whether perf mode is on (any non-empty `DEEPGRID_PERF`).
pub fn enabled() -> bool {
    std::env::var("DEEPGRID_PERF").is_ok_and(|v| !v.is_empty())
}

/// The measurement window in seconds. `1` means "enabled, default duration".
fn duration_secs() -> f64 {
    std::env::var("DEEPGRID_PERF")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|&v| v >= 2.0)
        .unwrap_or(DEFAULT_SECS)
}

#[derive(Default)]
pub struct PerfAccum {
    elapsed: f64,
    frames: u64,
    total: f64,
    worst: f64,
    done: bool,
}

/// Accumulate frame times after warmup; print the summary and quit when the
/// window closes.
pub fn measure(mut acc: Local<PerfAccum>, time: Res<Time>, mut exit: EventWriter<AppExit>) {
    if acc.done {
        return;
    }
    let dt = time.delta_secs_f64();
    acc.elapsed += dt;
    if acc.elapsed < WARMUP_SECS {
        return;
    }
    acc.frames += 1;
    acc.total += dt;
    acc.worst = acc.worst.max(dt);
    if acc.total >= duration_secs() {
        let avg = acc.total / acc.frames as f64;
        println!(
            "[perf] frames={} window={:.1}s avg={:.2}ms worst={:.2}ms avg_fps={:.1}",
            acc.frames,
            acc.total,
            avg * 1000.0,
            acc.worst * 1000.0,
            1.0 / avg
        );
        acc.done = true;
        exit.send(AppExit::Success);
    }
}
