#!/usr/bin/env bash
# Render the demo video (1920x1080 H.264, no audio) and a GIF teaser from REAL command output.
# Output: $VIDEO_DIR/fe-zk-demo.mp4 and $VIDEO_DIR/teaser.gif (default VIDEO_DIR=/data/fe-zk-video;
# keep it off the repository). Needs: fe (26.5.0), cargo, ffmpeg with libx264 + drawtext, python3.
# Optional env: FE, VIDEO_DIR, CARGO_TARGET_DIR_GEN, CARGO_TARGET_DIR_LIFE.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export VIDEO_DIR="${VIDEO_DIR:-/data/fe-zk-video}"
export FE="${FE:-fe}"
mkdir -p "$VIDEO_DIR"
GT="${CARGO_TARGET_DIR_GEN:-$VIDEO_DIR/gen-target}"
LT="${CARGO_TARGET_DIR_LIFE:-$VIDEO_DIR/life-cli-target}"

echo "== build fe-zk-gen, life-cli and the Life contracts"
(cd "$ROOT/gen" && CARGO_TARGET_DIR="$GT" cargo build -q -p fe-zk-gen)
(cd "$ROOT/examples/life/cli" && CARGO_TARGET_DIR="$LT" cargo build -q --release)
(cd "$ROOT/examples/life" && "$FE" build >/dev/null)
(cd "$ROOT/examples/life/verifier" && "$FE" build >/dev/null)

echo "== render"
GEN_BIN="$GT/debug/fe-zk-gen" LIFE_BIN="$LT/release/life-cli" python3 -I "$ROOT/scripts/video/build.py"
ffprobe -v error -show_entries format=duration,size -of default=nw=1 "$VIDEO_DIR/fe-zk-demo.mp4"
ls -l "$VIDEO_DIR/fe-zk-demo.mp4" "$VIDEO_DIR/teaser.gif"
