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
# TODO(zk): second path, not implemented yet.
#   1. circom circuit under examples/life/circuit/: N generations from a Poseidon-committed
#      start board to a committed end board.
#   2. prove with snarkjs, emit the Fe verifier with `fe-zk-gen groth16 --snarkjs vk.json`.
#   3. add `--prove` to life-cli and print: gas of N on-chain steps vs one proof verification.
# ---------------------------------------------------------------------------
