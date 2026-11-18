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
(cd "$LIFE" && fe fmt --check && fe test --show-logs 2>&1 | tee "${TMPDIR:-/tmp}/life-test.log" | grep -E "^(PASS|FAIL)  .*(gas|glider_via|stateful|zk_)|test result")

echo
echo "== Gas measured inside fe test (GasUsed events: what, gas) =="
python3 -I - "${TMPDIR:-/tmp}/life-test.log" <<'PY'
import re, sys
names = {1: "step() via call", 64: "run(board, 64)", 256: "run(board, 256)",
         512: "run(board, 512)", 1001: "stateful advance(1) (storage)",
         2001: "advanceProven (proof of 512 gens)", 2002: "proveRun (proof of 512 gens)"}
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
# zk path, from the committed fixture vectors/groth16/life_512 (fast, no circom/snarkjs needed).
# Circuit: examples/life/circuit/life.circom (7 constraints per cell, 1792 per generation), public
# signals [startCommit, endCommit] = Poseidon(lo128, hi128) of the packed boards, N = 512 fixed by
# the key. The verifier ingot examples/life/verifier is emitted by fe-zk-gen (see below).
echo
echo "== zk path: one Groth16 proof of 512 generations vs 512 on-chain steps =="
(cd "$LIFE/verifier" && fe test 2>&1 | grep -E "test result")
(cd "$LIFE/verifier" && fe build >/dev/null)
(cd "$LIFE/cli" && cargo run --release --quiet -- --prove)
echo
echo "Regenerate the fixture (needs circom + snarkjs, scripts/setup-tools.sh; ~30 min, 1.2 GB ptau):"
echo "  scripts/life-prove.sh rpentomino 512"
echo "  (cd gen && cargo run -q -p fe-zk-gen -- groth16 --snarkjs ../vectors/groth16/life_512/verification_key.json \\"
echo "     --proof ../vectors/groth16/life_512/proof.json --public ../vectors/groth16/life_512/public.json \\"
echo "     --name Life512 --out ../examples/life/verifier --fe-zk-path ../../..)"
echo "Prove live from the CLI: cd examples/life/cli && cargo run --release -- --prove-live --pattern rpentomino"
