#!/usr/bin/env bash
# Differential test: every Fe verifier vs its reference verifier on valid and mutated proofs, run side by
# side in revm. See docs/TESTING.md and vectors/difftest/.
#
#   scripts/difftest.sh --quick     committed fixtures only, small mutation sets (CI smoke, < 3 min)
#   scripts/difftest.sh             full run: fresh circom/snarkjs setups + proofs, thousands of mutations
#   extra flags go to vectors/difftest/difftest.py (--seed N, --fresh N, --random N, --only REGEX, --json FILE)
#
# Needs: fe (FE), cargo, solc 0.8.24, python3. Full mode also: circom (CIRCOM), node + snarkjs
# (NODE_MODULES_DIR, default vectors/js/node_modules; `npm ci --ignore-scripts` there), a ptau file (PTAU, e.g. the
# PSE ppot_0080_14.ptau that scripts/regen-groth16-fixtures.sh downloads to vectors/cache/).
# Env: FE_OPT (default 1), DIFF_WORK (scratch dir, default mktemp), CARGO_TARGET_DIR (default $DIFF_WORK/target), KEEP=1.
# Exit status 1 if any BUG is found (an unexplained verdict difference).
# Dev box: PATH=/data/nightly-x86_64-unknown-linux-gnu/bin:$PATH CARGO_HOME=/data/cargo
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
FE="${FE:-fe}"
if [ -z "${DIFF_WORK:-}" ]; then
  DIFF_WORK="$(mktemp -d)"
  [ "${KEEP:-0}" = 1 ] || trap 'rm -rf "$DIFF_WORK"' EXIT
fi
mkdir -p "$DIFF_WORK"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$DIFF_WORK/target}"
NM="${NODE_MODULES_DIR:-$ROOT/vectors/js/node_modules}"
QUICK=0
for x in "$@"; do [ "$x" = "--quick" ] && QUICK=1; done
EXTRA=()
if [ "$QUICK" = 0 ]; then
  PTAU="${PTAU:-$ROOT/vectors/cache/ppot_0080_14.ptau}"
  [ -f "$PTAU" ] || { echo "full mode needs a ptau file: set PTAU=... (see header)" >&2; exit 2; }
  EXTRA+=(--ptau "$PTAU" --circom "${CIRCOM:-circom}")
fi

echo "== build evm-batch, ark-ref, fe-zk-gen"
cargo build -q --release --manifest-path vectors/difftest/Cargo.toml
cargo build -q --release --manifest-path gen/Cargo.toml -p fe-zk-gen
python3 -I vectors/difftest/difftest.py --work "$DIFF_WORK" \
  --evm-batch "$CARGO_TARGET_DIR/release/evm-batch" --ark-ref "$CARGO_TARGET_DIR/release/ark-ref" \
  --gen "$CARGO_TARGET_DIR/release/fe-zk-gen" --fe "$FE" --fe-opt "${FE_OPT:-1}" --node-modules "$NM" \
  ${EXTRA[@]+"${EXTRA[@]}"} "$@"
