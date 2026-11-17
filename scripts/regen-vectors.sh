#!/usr/bin/env bash
# Regenerates every committed test vector and the generated Fe sources that embed them.
# Vectors come from independent reference implementations, never from fe-zk code.
# Sections are independent; other components add their own section below.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ---------------------------------------------------------------------------------------
# hash: Poseidon (circomlibjs), Poseidon2 (HorizenLabs BN254, Plonky3 KoalaBear), transcript
# ---------------------------------------------------------------------------------------
# Tools: node >= 18, circomlibjs 0.1.7 + ethers 5.8.0 (vectors/js/package-lock.json),
#        HorizenLabs/poseidon2 at HL_COMMIT, p3-koala-bear 0.9.0-rc.2 (vectors/plonky3/Cargo.lock).
HL_COMMIT=055bde3f4782731ba5f5ce5888a440a94327eaf3
HL_DIR="$ROOT/vectors/cache/poseidon2-horizenlabs"

(cd vectors/js && npm ci --silent)

node scripts/gen-poseidon-constants.js

if [ ! -d "$HL_DIR/.git" ]; then
  mkdir -p "$ROOT/vectors/cache"
  git clone --quiet https://github.com/HorizenLabs/poseidon2.git "$HL_DIR"
fi
git -C "$HL_DIR" checkout --quiet "$HL_COMMIT"

P3_JSON="$ROOT/vectors/cache/plonky3-koalabear-t16.json"
mkdir -p "$ROOT/vectors/cache"
(cd vectors/plonky3 && cargo run --release --quiet --locked > "$P3_JSON")

HL_SRC="$HL_DIR/plain_implementations/src/poseidon2"
node scripts/gen-poseidon2-constants.js \
  "$HL_SRC/poseidon2_instance_bn256.rs" "$HL_SRC/poseidon2.rs" "$P3_JSON"

# Fe tests with the expected values inlined (needs all vector files above)
node scripts/gen-hash-tests.js

command -v fe >/dev/null && fe fmt src
echo "hash vectors regenerated"
