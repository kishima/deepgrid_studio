#!/usr/bin/env bash
# Build DeepGrid Studio (the project at the repo root) in the container.
# Reuses the gaia-maker-build image (Rust + Bevy deps) and the shared cargo cache.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)"
IMAGE="gaia-maker-build"
MODE="${1:-release}" # release (default) | debug

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo ">>> Image $IMAGE missing; building it"
  docker build -t "$IMAGE" "$HERE"
fi

FLAG=()
[[ "$MODE" == "release" ]] && FLAG=(--release)

echo ">>> Building deepgrid_studio ($MODE)"
docker run --rm \
  -v "$PROJECT":/app \
  -v gaia-cargo-registry:/usr/local/cargo/registry \
  -w /app \
  "$IMAGE" \
  cargo build "${FLAG[@]}"

echo ">>> Done: $PROJECT/target/$MODE/deepgrid_studio"
