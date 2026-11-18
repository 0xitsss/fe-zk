#!/usr/bin/env bash
# Regenerates vectors/kzg/fixtures/kzg.json with arkworks 0.5.0 on a REAL SRS: the PSE perpetual
# powers of tau (challenge 0080, ppot_0080_05, 2^5 G1 powers). tau is unknown; commitments and opening
# proofs are MSMs over the ceremony points. The ptau is downloaded to vectors/cache (never committed)
# and its sha256 is checked. Output is deterministic.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PTAU_URL=https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_05.ptau
PTAU_SHA256=9c1bfc4a2895b9beb1df6e95c96785dc9382bee84d84a0d8c56b1061e09d89d1
PTAU="${FE_ZK_PTAU:-$ROOT/vectors/cache/ppot_0080_05.ptau}"
mkdir -p "$(dirname "$PTAU")"
[ -f "$PTAU" ] || curl -sSfL -o "$PTAU" "$PTAU_URL"
echo "$PTAU_SHA256  $PTAU" | sha256sum -c -
cd "$ROOT/vectors/kzg"
cargo run --release --locked -q -- "$PTAU" fixtures/kzg.json
echo "wrote vectors/kzg/fixtures/kzg.json"
