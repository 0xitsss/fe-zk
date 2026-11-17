#!/usr/bin/env bash
# zk Game of Life demo: on-chain path (works now) + zk path (TODO).
# Needs: fe (see README for the pinned version), cargo.
# Optional env: GENS (default 64), PATTERN (default glider), SEED.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIFE="$ROOT/examples/life"
GENS="${GENS:-64}"
PATTERN="${PATTERN:-glider}"
SEED="${SEED:-1}"

echo "== Fe tests (step vectors, contract calls, gas events) =="
(cd "$LIFE" && fe fmt --check && fe test --show-logs 2>&1 | tee "${TMPDIR:-/tmp}/life-test.log" | grep -E "^(PASS|FAIL)  .*(gas|glider_via|stateful)|test result")

echo
echo "== Gas measured inside fe test (GasUsed events: what, gas) =="
python3 -I - "${TMPDIR:-/tmp}/life-test.log" <<'PY'
import re, sys
names = {1: "step() via call", 64: "run(board, 64)", 256: "run(board, 256)",
         1001: "stateful advance(1) (storage)"}
print("%-32s %10s" % ("call", "gas"))
for m in re.finditer(r"data: 0x([0-9a-f]{64})([0-9a-f]{64})", open(sys.argv[1]).read()):
    what, gas = int(m.group(1), 16), int(m.group(2), 16)
    if what in names:
        print("%-32s %10d" % (names[what], gas))
PY

echo
echo "== On-chain play in revm: ${GENS} generations of '${PATTERN}' (per-cell reference check) =="
(cd "$LIFE" && fe build >/dev/null)
(cd "$LIFE/cli" && cargo run --release --quiet -- \
    --pattern "$PATTERN" --gens "$GENS" --seed "$SEED" --no-anim --check)

echo
echo "Interactive: cd examples/life/cli && cargo run --release -- --pattern rpentomino --gens 200"

# ---------------------------------------------------------------------------
# zk path. Circuit: examples/life/circuit/life.circom (7 constraints per cell, 1792 per generation).
# Public signals: [startCommit, endCommit] = Poseidon(lo128, hi128) of the packed boards. The start
# board is private; N is fixed per circuit. Set ZK=1 to run it (needs circom + snarkjs, see
# scripts/setup-tools.sh; the first run downloads a PSE ptau of up to 1.2 GB into $CACHE and takes
# minutes for N=512). Default: reuse the committed fixture vectors/groth16/life_64 (N=64).
if [ "${ZK:-0}" = 1 ]; then
  echo
  echo "== zk path: Groth16 proof of ${ZKGENS:-512} generations (rpentomino) =="
  "$ROOT/scripts/life-prove.sh" rpentomino "${ZKGENS:-512}"
fi
# TODO(zk-integration): emit the Fe verifier with `fe-zk-gen groth16 --snarkjs
#   vectors/groth16/life_<N>/verification_key.json`, verify vectors/groth16/life_<N>/proof.json
#   against public.json, and add `--prove` to life-cli to print: gas of N on-chain steps
#   vs one proof verification (break-even about 450 generations).
# ---------------------------------------------------------------------------
