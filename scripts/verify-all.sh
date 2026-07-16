#!/usr/bin/env bash
# The one verification entry point (plan11). Runs, in order:
#   1. clippy (host cargo, zero warnings)      2. cargo test (host)
#   3. docker release build                    4. autotest (all steps)
#   5. every debug-shot scene (mtime-checked)  6. export smoke test
# and prints a PASS/FAIL table. Exit code 0 only when everything passed.
# Plan completion checks run this single script from now on.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
cd "$REPO"

CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
CHECK_DIR="${DEEPGRID_CHECK_DIR:-/tmp/deepgrid-check}"

# The machine-readable scene list (README「検証用スクリーンショット」mirrors this).
PLAY_SCENES=(1 fall ladder door monster magic light potion plate warp stairs hole
             combat items pickup data liquid demo override title)
EDITOR_SCENES=(editor editor-chars editor-items editor-monsters editor-magics
               editor-events editor-demos editor-settings editor-3d editor-testplay)

declare -a RESULTS=()
FAILED=0

note() { printf '\n\033[1m>>> %s\033[0m\n' "$*"; }

record() { # record <name> <status(0=pass)>
  if [[ "$2" -eq 0 ]]; then
    RESULTS+=("PASS  $1")
  else
    RESULTS+=("FAIL  $1")
    FAILED=1
  fi
}

run_step() { # run_step <name> <cmd...>
  note "$1"
  "${@:2}"
  record "$1" $?
}

shot_scene() { # shot_scene <scene>
  local scene="$1"
  rm -f debug-shot.png
  if ! DEEPGRID_DEBUG_SHOT="$scene" ./docker/deepgrid-run.sh >/dev/null 2>&1; then
    record "scene $scene (run)" 1
    return
  fi
  # The shot must exist, be fresh (this run), and be non-trivial.
  if [[ -f debug-shot.png ]] && [[ -n "$(find debug-shot.png -newermt "-120 seconds")" ]] \
     && [[ "$(stat -c%s debug-shot.png)" -gt 10000 ]]; then
    record "scene $scene" 0
  else
    record "scene $scene (stale/missing shot)" 1
  fi
}

# 1-2. host static checks.
run_step "clippy" env CARGO_TARGET_DIR="$CHECK_DIR" "$CARGO" clippy --all-targets -- -D warnings
run_step "cargo test" env CARGO_TARGET_DIR="$CHECK_DIR" "$CARGO" test --quiet

# 3. docker release build.
run_step "docker build" ./docker/deepgrid-build.sh

# 4. autotest.
note "autotest"
DEEPGRID_AUTOTEST=1 ./docker/deepgrid-run.sh
record "autotest" $?

# 5. every scene.
for scene in "${PLAY_SCENES[@]}" "${EDITOR_SCENES[@]}"; do
  note "scene $scene"
  shot_scene "$scene"
done

# 6. export smoke test: the artifact excludes saves/*.bak, pins play_only, and
#    its --edit is politely refused (checked against the runtime's message).
note "export"
EXPORT_DIR="$(mktemp -d /tmp/deepgrid-export-XXXX)"
if ./scripts/export-game.sh assets/projects/sample "$EXPORT_DIR" >/dev/null; then
  ok=0
  [[ -e "$EXPORT_DIR/assets/projects/sample/saves" ]] && { echo "export: saves/ leaked"; ok=1; }
  [[ -n "$(find "$EXPORT_DIR" -name '*.ron.bak' -print -quit)" ]] && { echo "export: .ron.bak leaked"; ok=1; }
  [[ -f "$EXPORT_DIR/deepgrid.ron" && -f "$EXPORT_DIR/CREDITS.md" && -f "$EXPORT_DIR/README.md" ]] || { echo "export: missing metadata files"; ok=1; }
  ls "$EXPORT_DIR"/assets/projects | grep -qv '^sample$' && { echo "export: extra projects leaked"; ok=1; }
  record "export layout" $ok

  # Run the exported build in docker (same runtime env), asking for --edit:
  # play_only must refuse it and still produce a play-mode title shot.
  rm -f "$EXPORT_DIR/debug-shot.png"
  OUT="$(docker run --rm -v "$EXPORT_DIR":/app -w /app \
      -e DISPLAY="${DISPLAY:-:0}" -e WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}" \
      -e XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir -e PULSE_SERVER=/mnt/wslg/PulseServer \
      -e WGPU_BACKEND="${WGPU_BACKEND:-vulkan}" -e BEVY_ASSET_ROOT=/app \
      -e DEEPGRID_DEBUG_SHOT=title \
      -v /tmp/.X11-unix:/tmp/.X11-unix -v /mnt/wslg:/mnt/wslg \
      gaia-maker-build ./deepgrid_studio --edit 2>&1)"
  status=$?
  ok=0
  [[ $status -eq 0 ]] || { echo "export run: exit $status"; ok=1; }
  grep -q "play_only" <<<"$OUT" || { echo "export run: no polite --edit refusal in output"; ok=1; }
  [[ -s "$EXPORT_DIR/debug-shot.png" ]] || { echo "export run: no title shot"; ok=1; }
  record "export play_only run" $ok
else
  record "export-game.sh" 1
fi
rm -rf "$EXPORT_DIR"

# ---- summary ----
printf '\n================ verify-all ================\n'
printf '%s\n' "${RESULTS[@]}"
printf '============================================\n'
if [[ $FAILED -ne 0 ]]; then
  echo "RESULT: FAIL"
  exit 1
fi
echo "RESULT: PASS"
