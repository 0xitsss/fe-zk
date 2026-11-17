#!/usr/bin/env bash
# Regenerates vectors/groth16/{multiplier,multiplier2,membership}/ with circom + snarkjs, and
# vectors/groth16/arkworks_multiplier/ with ark-groth16 (needs cargo).
# Needs: scripts/setup-tools.sh run first. Env: CIRCOM (path to circom binary).
# Output per fixture: verification_key.json proof.json public.json verify.txt Verifier.sol
# Test-only setup: phase-2 entropy is a fixed public string, so the keys are INSECURE by design.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZK_TOOLS_DIR="${ZK_TOOLS_DIR:-$HOME/.cache/fe-zk-tools}"
CIRCOM="${CIRCOM:-$ZK_TOOLS_DIR/bin/circom}"
JS="$ROOT/vectors/js"
SNARKJS="$JS/node_modules/.bin/snarkjs"
CACHE="$ROOT/vectors/cache"
OUT="$ROOT/vectors/groth16"

# Perpetual Powers of Tau (PSE ppot_0080_14, 2^14, fits 8191 constraints). The Hermez bucket is no longer
# anonymously readable, so this mirror is used. Hash pinned; verified before every use.
PTAU_URL=https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_14.ptau
PTAU_SHA256=3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db
PTAU="$CACHE/ppot_0080_14.ptau"

mkdir -p "$CACHE" "$OUT"
[ -f "$PTAU" ] || curl -sSfL -o "$PTAU" "$PTAU_URL"
echo "$PTAU_SHA256  $PTAU" | sha256sum -c -

build() { # name circuit.circom input.json
  local name="$1" circuit="$2" input="$3" w="$CACHE/$1"
  rm -rf "$w"; mkdir -p "$w" "$OUT/$name"
  "$CIRCOM" "$circuit" --r1cs --wasm -l "$JS/node_modules" -o "$w"
  local base; base="$(basename "$circuit" .circom)"
  "$SNARKJS" groth16 setup "$w/$base.r1cs" "$PTAU" "$w/0.zkey"
  "$SNARKJS" zkey contribute "$w/0.zkey" "$w/final.zkey" --name="fe-zk-test" -e="fe-zk public test entropy, insecure"
  "$SNARKJS" zkey export verificationkey "$w/final.zkey" "$OUT/$name/verification_key.json"
  node "$w/${base}_js/generate_witness.js" "$w/${base}_js/$base.wasm" "$input" "$w/witness.wtns"
  "$SNARKJS" groth16 prove "$w/final.zkey" "$w/witness.wtns" "$OUT/$name/proof.json" "$OUT/$name/public.json"
  "$SNARKJS" groth16 verify "$OUT/$name/verification_key.json" "$OUT/$name/public.json" "$OUT/$name/proof.json" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | tee "$OUT/$name/verify.txt"
  "$SNARKJS" zkey export solidityverifier "$w/final.zkey" "$OUT/$name/Verifier.sol"
  "$SNARKJS" r1cs info "$w/$base.r1cs" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' > "$OUT/$name/r1cs-info.txt"
}

IN="$CACHE/inputs"; mkdir -p "$IN"
echo '{"a":"3","b":"11"}' > "$IN/multiplier.json"
build multiplier  "$ROOT/vectors/circuits/multiplier.circom"  "$IN/multiplier.json"
build multiplier2 "$ROOT/vectors/circuits/multiplier2.circom" "$IN/multiplier.json"
(cd "$JS" && node membership-input.mjs "$IN/membership.json")
build membership  "$ROOT/examples/membership/circuit/membership.circom" "$IN/membership.json"
# arkworks fixture (ark-groth16 0.5.0 / ark-bn254 0.5.0, seeded insecure setup)
(cd "$ROOT/gen" && cargo run -q -p fe-zk-gen --example arkworks_fixture -- "$OUT/arkworks_multiplier")
echo "fixtures written to $OUT"
