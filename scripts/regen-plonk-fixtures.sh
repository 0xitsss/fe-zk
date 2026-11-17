#!/usr/bin/env bash
# Regenerates vectors/plonk/{multiplier2,membership}/ with circom + snarkjs `plonk`.
# Needs: scripts/setup-tools.sh run first. Env: CIRCOM (path to circom binary).
# Output per fixture: verification_key.json proof.json public.json verify.txt Verifier.sol r1cs-info.txt
# Test-only setup: the ptau is a public file and the PLONK setup is deterministic from it,
# so the keys are INSECURE by design. ptau/zkey/wasm are never committed (vectors/cache is ignored).
# NOTE: snarkjs plonk proofs are randomized (blinding), so re-running changes proof.json; the
# committed Fe test constants are copied from the committed files.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZK_TOOLS_DIR="${ZK_TOOLS_DIR:-$HOME/.cache/fe-zk-tools}"
CIRCOM="${CIRCOM:-$ZK_TOOLS_DIR/bin/circom}"
JS="$ROOT/vectors/js"
SNARKJS="$JS/node_modules/.bin/snarkjs"
CACHE="$ROOT/vectors/cache"
OUT="$ROOT/vectors/plonk"

# Same pinned PSE ppot_0080_14 file as scripts/regen-groth16-fixtures.sh (2^14 constraints).
PTAU_URL=https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_14.ptau
PTAU_SHA256=3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db
PTAU="$CACHE/ppot_0080_14.ptau"

mkdir -p "$CACHE" "$OUT"
[ -f "$PTAU" ] || curl -sSfL -o "$PTAU" "$PTAU_URL"
echo "$PTAU_SHA256  $PTAU" | sha256sum -c -

build() { # name circuit.circom input.json
  local name="$1" circuit="$2" input="$3" w="$CACHE/plonk-$1"
  rm -rf "$w"; mkdir -p "$w" "$OUT/$name"
  "$CIRCOM" "$circuit" --r1cs --wasm -l "$JS/node_modules" -o "$w"
  local base; base="$(basename "$circuit" .circom)"
  "$SNARKJS" plonk setup "$w/$base.r1cs" "$PTAU" "$w/final.zkey"
  "$SNARKJS" zkey export verificationkey "$w/final.zkey" "$OUT/$name/verification_key.json"
  node "$w/${base}_js/generate_witness.js" "$w/${base}_js/$base.wasm" "$input" "$w/witness.wtns"
  "$SNARKJS" plonk prove "$w/final.zkey" "$w/witness.wtns" "$OUT/$name/proof.json" "$OUT/$name/public.json"
  "$SNARKJS" plonk verify "$OUT/$name/verification_key.json" "$OUT/$name/public.json" "$OUT/$name/proof.json" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | tee "$OUT/$name/verify.txt"
  "$SNARKJS" zkey export solidityverifier "$w/final.zkey" "$OUT/$name/Verifier.sol"
  "$SNARKJS" r1cs info "$w/$base.r1cs" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' > "$OUT/$name/r1cs-info.txt"
}

IN="$CACHE/inputs"; mkdir -p "$IN"
echo '{"a":"3","b":"11"}' > "$IN/multiplier.json"
build multiplier2 "$ROOT/vectors/circuits/multiplier2.circom" "$IN/multiplier.json"
(cd "$JS" && node membership-input.mjs "$IN/membership.json")
build membership  "$ROOT/examples/membership/circuit/membership.circom" "$IN/membership.json"
echo "fixtures written to $OUT"
