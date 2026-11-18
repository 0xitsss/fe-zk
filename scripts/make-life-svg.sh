#!/usr/bin/env bash
# Regenerates docs/media/life.svg from the real contract execution (needs fe build in
# examples/life and examples/life/verifier, plus cargo). Boards and gas come from life-cli.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
(cd "$ROOT/examples/life" && fe build >/dev/null)
(cd "$ROOT/examples/life/verifier" && fe build >/dev/null)
cd "$ROOT/examples/life/cli"
cargo run --release --quiet -- --pattern rpentomino --gens 64 --no-anim --check --frames-json "$TMP/play.json" >/dev/null
cargo run --release --quiet -- --prove --frames-json "$TMP/prove.json" >/dev/null
python3 -I "$ROOT/scripts/make-life-svg.py" "$TMP/play.json" "$TMP/prove.json" "$ROOT/docs/media/life.svg"
