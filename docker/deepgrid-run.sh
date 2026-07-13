#!/usr/bin/env bash
# Run DeepGrid Studio on the Windows desktop through WSLg.
# Default = software rendering (Vulkan/lavapipe), proven reliable on this WSL2 box.
#
# Usage:
#   ./deepgrid-run.sh                 # release build, software rendering
#   ./deepgrid-run.sh debug           # run the debug build instead
#   ./deepgrid-run.sh --edit          # pass args to the binary (release build)
#   ./deepgrid-run.sh debug --edit    # debug build + args
#
# The first argument is the build mode only when it is exactly `release` or
# `debug`; anything else (and all remaining args) is forwarded to the binary.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)"
IMAGE="gaia-maker-build"
MODE="release"
if [[ "${1:-}" == "release" || "${1:-}" == "debug" ]]; then
  MODE="$1"
  shift
fi
BIN="./target/$MODE/deepgrid_studio"

if [[ ! -x "$PROJECT/target/$MODE/deepgrid_studio" ]]; then
  echo "Binary not found ($MODE). Run ./deepgrid-build.sh $MODE first." >&2
  exit 1
fi

# Allocate a TTY only when we actually have one, so non-interactive callers
# (CI, AI-driven verification) don't fail with "stdin is not a terminal".
TTY_FLAGS=()
[[ -t 0 ]] && TTY_FLAGS=(-it)

exec docker run --rm "${TTY_FLAGS[@]}" \
  -v "$PROJECT":/app \
  -w /app \
  -e DISPLAY="${DISPLAY:-:0}" \
  -e WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}" \
  -e XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir \
  -e PULSE_SERVER=/mnt/wslg/PulseServer \
  -e WGPU_BACKEND="${WGPU_BACKEND:-vulkan}" \
  -e BEVY_ASSET_ROOT=/app \
  -e DEEPGRID_DEBUG_SHOT="${DEEPGRID_DEBUG_SHOT:-}" \
  -e DEEPGRID_AUTOTEST="${DEEPGRID_AUTOTEST:-}" \
  -v /tmp/.X11-unix:/tmp/.X11-unix \
  -v /mnt/wslg:/mnt/wslg \
  "$IMAGE" "$BIN" "$@"
