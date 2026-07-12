#!/usr/bin/env bash
# Run DeepGrid Studio on the Windows desktop through WSLg.
# Default = software rendering (Vulkan/lavapipe), proven reliable on this WSL2 box.
#
# Usage:
#   ./deepgrid-run.sh            # release build, software rendering
#   ./deepgrid-run.sh debug      # run the debug build instead
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)"
IMAGE="gaia-maker-build"
MODE="${1:-release}"
BIN="./target/$MODE/deepgrid_studio"

if [[ ! -x "$PROJECT/target/$MODE/deepgrid_studio" ]]; then
  echo "Binary not found ($MODE). Run ./deepgrid-build.sh $MODE first." >&2
  exit 1
fi

exec docker run --rm -it \
  -v "$PROJECT":/app \
  -w /app \
  -e DISPLAY="${DISPLAY:-:0}" \
  -e WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}" \
  -e XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir \
  -e PULSE_SERVER=/mnt/wslg/PulseServer \
  -e WGPU_BACKEND="${WGPU_BACKEND:-vulkan}" \
  -e BEVY_ASSET_ROOT=/app \
  -e DEEPGRID_DEBUG_SHOT="${DEEPGRID_DEBUG_SHOT:-}" \
  -v /tmp/.X11-unix:/tmp/.X11-unix \
  -v /mnt/wslg:/mnt/wslg \
  "$IMAGE" "$BIN"
