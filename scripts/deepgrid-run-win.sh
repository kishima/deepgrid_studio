#!/usr/bin/env bash
# Build + run the Windows-native GPU build from WSL (plan11 / project.md
# 「Windows ネイティブ GPU 実行」). Cross-compiles with mingw, copies the exe to
# the repo root (gitignored) and starts it through Windows interop so it runs
# on the real GPU instead of docker's lavapipe.
#
# One-time setup (mingw needs sudo — ask a human):
#   ~/.cargo/bin/rustup target add x86_64-pc-windows-gnu
#   sudo apt install gcc-mingw-w64-x86-64
#
# Usage:
#   ./scripts/deepgrid-run-win.sh [args passed to the binary]
#   DEEPGRID_DEBUG_SHOT=title ./scripts/deepgrid-run-win.sh
#
# Check the startup log's AdapterInfo: it must name the NVIDIA GPU with the
# Vulkan backend (not lavapipe / Gl).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/deepgrid-target-win}"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "x86_64-w64-mingw32-gcc not found." >&2
  echo "One-time setup (needs sudo):" >&2
  echo "  ~/.cargo/bin/rustup target add x86_64-pc-windows-gnu" >&2
  echo "  sudo apt install gcc-mingw-w64-x86-64" >&2
  exit 1
fi

echo ">>> Cross-building for x86_64-pc-windows-gnu (target dir: $TARGET_DIR)"
(cd "$PROJECT" && CARGO_TARGET_DIR="$TARGET_DIR" \
  "$CARGO" build --release --target x86_64-pc-windows-gnu)

EXE="$TARGET_DIR/x86_64-pc-windows-gnu/release/deepgrid_studio.exe"
cp "$EXE" "$PROJECT/deepgrid_studio.exe"
echo ">>> Copied $(du -h "$PROJECT/deepgrid_studio.exe" | cut -f1) exe to repo root"

# DEEPGRID_* / WGPU_* don't cross the WSL→Windows boundary on their own; WSLENV
# whitelists them. Add every new env var here AND in docker/deepgrid-run.sh.
export WSLENV="${WSLENV:+$WSLENV:}DEEPGRID_DEBUG_SHOT:DEEPGRID_AUTOTEST:DEEPGRID_PERF:DEEPGRID_WINDOW:WGPU_BACKEND:RUST_LOG"

echo ">>> Launching on Windows (watch the AdapterInfo line: expect NVIDIA + Vulkan)"
cd "$PROJECT"
exec ./deepgrid_studio.exe "$@"
