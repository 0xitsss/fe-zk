#!/usr/bin/env bash
# Proves N generations of the zk Game of Life (examples/life/circuit/life.circom) with Groth16 and
# writes vectors/groth16/life_<N>/ : verification_key.json proof.json public.json verify.txt
# Verifier.sol r1cs-info.txt input.json boards.json stats.txt
# usage: scripts/life-prove.sh <pattern: glider|blinker|rpentomino|random> <N> [seed]
# Needs: scripts/setup-tools.sh run first (circom, node, snarkjs in vectors/js). Python 3.
# Env: CIRCOM (circom binary), CACHE (default vectors/cache; ptau/zkey/wasm live here and are never
#      committed; ptau files are 0.15 - 2.4 GB), NO_NEG=1 skips the negative tests.
# Test-only setup: phase-2 entropy is a fixed public string, so the keys are INSECURE by design.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PATTERN="${1:?usage: life-prove.sh <pattern> <N> [seed]}"
N="${2:?usage: life-prove.sh <pattern> <N> [seed]}"
SEED="${3:-1}"
ZK_TOOLS_DIR="${ZK_TOOLS_DIR:-$HOME/.cache/fe-zk-tools}"
CIRCOM="${CIRCOM:-$ZK_TOOLS_DIR/bin/circom}"
JS="$ROOT/vectors/js"
SNARKJS="$JS/node_modules/.bin/snarkjs"
CACHE="${CACHE:-$ROOT/vectors/cache}"
OUT="$ROOT/vectors/groth16/life_$N"
W="$CACHE/life_$N"
export NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=14000}"

# Perpetual Powers of Tau, PSE mirror (ppot_0080_XX = 2^XX constraints). Hashes pinned; verified before use.
PTAU_BASE=https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080
declare -A PTAU_SHA256=(
  [17]=f807e065fde53f72f4bf4d57140fab85b26daa6cc95bdfec7cce93622b3a367c
  [19]=d0cadfb7d5bad6c5e0b012fb1c4bbaa3cc98fe249988f9f99d1818175e4c2015
  [20]=560412532a1205145d5f21585274fb2cef61273496ddc7186aec855aab01a8cd
  [21]=b4131202088141088d83fc05d654a142b51117df0960568613069e0719e7645a
)

strip() { sed 's/\x1b\[[0-9;]*m//g'; }
timed() { # label cmd... ; appends "label seconds peak_rss" to $W/stats.raw (python3 resource, no GNU time needed)
  python3 -I - "$W/stats.raw" "$@" <<'PY'
import resource, subprocess, sys, time
out, label, cmd = sys.argv[1], sys.argv[2], sys.argv[3:]
t = time.time(); rc = subprocess.call(cmd); dt = time.time() - t
rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss  # KB, max over children so far
open(out, "a").write("%s %.1f s  peak_rss_so_far %d MB\n" % (label, dt, rss // 1024))
sys.exit(rc)
PY
}

[ -x "$SNARKJS" ] || (cd "$JS" && npm ci --ignore-scripts)
rm -rf "$W"; mkdir -p "$W" "$OUT" "$CACHE"

# --- boards (per-cell Python reference) and circuit input (circomlibjs Poseidon) ---
read -r START END < <(python3 -I "$ROOT/examples/life/circuit/make_boards.py" "$PATTERN" "$N" "$SEED")
(cd "$JS" && node life-input.mjs "$START" "$END" "$OUT/input.json" "$OUT/boards.json")
echo "pattern $PATTERN  N=$N  start=$START  end=$END"

# --- compile (main wrapper fixes N and the public signals) ---
cat > "$W/life_main.circom" <<EOF
pragma circom 2.0.0;
include "$ROOT/examples/life/circuit/life.circom";
component main {public [startCommit, endCommit]} = Life($N);
EOF
timed compile "$CIRCOM" "$W/life_main.circom" --r1cs --wasm --O2 -l "$JS/node_modules" -o "$W"
"$SNARKJS" r1cs info "$W/life_main.r1cs" 2>&1 | strip | tee "$OUT/r1cs-info.txt"
CONSTRAINTS="$(sed -n 's/.*# of Constraints: *//p' "$OUT/r1cs-info.txt")"
POWER=1; while [ $((1 << POWER)) -lt $((CONSTRAINTS + 4)) ]; do POWER=$((POWER + 1)); done
echo "constraints $CONSTRAINTS -> ptau power $POWER"

# --- ptau (pinned) ---
PTAU="$CACHE/ppot_0080_$POWER.ptau"
[ -n "${PTAU_SHA256[$POWER]:-}" ] || { echo "no pinned ptau for power $POWER"; exit 1; }
[ -f "$PTAU" ] || curl -sSfL -o "$PTAU" "$PTAU_BASE/ppot_0080_$POWER.ptau"
echo "${PTAU_SHA256[$POWER]}  $PTAU" | sha256sum -c -

# --- setup, witness, prove, verify ---
timed setup "$SNARKJS" groth16 setup "$W/life_main.r1cs" "$PTAU" "$W/0.zkey"
timed contribute "$SNARKJS" zkey contribute "$W/0.zkey" "$W/final.zkey" --name="fe-zk-test" -e="fe-zk public test entropy, insecure"
rm -f "$W/0.zkey"
"$SNARKJS" zkey export verificationkey "$W/final.zkey" "$OUT/verification_key.json"
timed witness node "$W/life_main_js/generate_witness.js" "$W/life_main_js/life_main.wasm" "$OUT/input.json" "$W/witness.wtns"
timed prove "$SNARKJS" groth16 prove "$W/final.zkey" "$W/witness.wtns" "$OUT/proof.json" "$OUT/public.json"
"$SNARKJS" groth16 verify "$OUT/verification_key.json" "$OUT/public.json" "$OUT/proof.json" 2>&1 | strip | tee "$OUT/verify.txt"
grep -q "OK!" "$OUT/verify.txt"
"$SNARKJS" zkey export solidityverifier "$W/final.zkey" "$OUT/Verifier.sol"
{ echo "# $(uname -sm), $(nproc) cores; timings via python resource; peak_rss is the max child RSS so far, not per step"; cat "$W/stats.raw"; } > "$OUT/stats.txt"

# --- public signals must be [startCommit, endCommit] of input.json ---
node -e '
const p=require(process.argv[1]), i=require(process.argv[2]);
if (p.length!==2 || p[0]!==i.startCommit || p[1]!==i.endCommit) { console.error("public.json layout mismatch"); process.exit(1); }
console.log("public.json = [startCommit, endCommit]  ok");' "$OUT/public.json" "$OUT/input.json"

# --- negative tests: witness generation must fail ---
if [ -z "${NO_NEG:-}" ]; then
  neg() { # name start_hex end_hex
    (cd "$JS" && node life-input.mjs "$2" "$3" "$W/neg_$1.json" "$W/neg_boards.json")
    if node "$W/life_main_js/generate_witness.js" "$W/life_main_js/life_main.wasm" "$W/neg_$1.json" "$W/neg.wtns" >"$W/neg_$1.log" 2>&1; then
      echo "NEGATIVE TEST FAILED: $1 produced a witness"; exit 1
    fi
    echo "negative ok: $1 rejected ($(head -c 80 "$W/neg_$1.log" | tr '\n' ' '))"
  }
  neg wrong_end "$START" "$(python3 -I -c "print(hex(int('$END',16) ^ 1))")"            # one end cell flipped
  neg start_is_end "$START" "$START"                                                     # N generations skipped
  neg one_gen_short "$START" "$(python3 -I "$ROOT/examples/life/circuit/make_boards.py" "$PATTERN" "$((N - 1))" "$SEED" | cut -d' ' -f2)"
  # right end board, wrong start commitment: commit of start^1 against the real board
  python3 -I - "$OUT/input.json" "$W/neg_start_commit.json" "$W/neg_boards.json" <<'PY'
import json, sys
i = json.load(open(sys.argv[1])); b = json.load(open(sys.argv[3]))
i["startCommit"] = b["endCommit"]  # a valid commitment, but not of this start board
json.dump(i, open(sys.argv[2], "w"))
PY
  if node "$W/life_main_js/generate_witness.js" "$W/life_main_js/life_main.wasm" "$W/neg_start_commit.json" "$W/neg.wtns" >/dev/null 2>&1; then
    echo "NEGATIVE TEST FAILED: wrong start commit produced a witness"; exit 1
  fi
  echo "negative ok: wrong_start_commit rejected"
fi
echo "fixtures written to $OUT"
