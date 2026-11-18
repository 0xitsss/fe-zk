#!/usr/bin/env bash
# Regenerates vectors/gnark/{mul,mulcommit}/{Verifier.sol,fixture.json} with gnark v0.12.0
# (gnark-crypto v0.15.0, pinned in go.mod/go.sum). Needs Go >= 1.22, curl.
# The KZG SRS is the real Perpetual Powers of Tau ceremony (PSE ppot_0080_05, 2^5 powers, challenge 0080,
# sha256 pinned here and again in vectors/gnark/ptau.go); nobody knows tau. vectors/gnark/ptau.go converts it to
# gnark's canonical + Lagrange SRS and checks it (generators, pairing ratio of every consecutive G1 power, sum
# of Lagrange points = generator). The ptau is downloaded to vectors/cache (never committed) or $FE_ZK_PTAU.
# Proofs are randomised (blinding), so regenerating changes every value; the Fe tests in
# src/tests/gnark_plonk_fixtures.fe must be regenerated too:
#   python3 -I scripts/gen-gnark-fe-fixtures.py > src/tests/gnark_plonk_fixtures.fe && fe fmt src/tests/gnark_plonk_fixtures.fe
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HERE="$ROOT/vectors/gnark"
PTAU_URL=https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_05.ptau
PTAU_SHA256=9c1bfc4a2895b9beb1df6e95c96785dc9382bee84d84a0d8c56b1061e09d89d1
PTAU="${FE_ZK_PTAU:-$ROOT/vectors/cache/ppot_0080_05.ptau}"
mkdir -p "$(dirname "$PTAU")"
[ -f "$PTAU" ] || curl -sSfL -o "$PTAU" "$PTAU_URL"
echo "$PTAU_SHA256  $PTAU" | sha256sum -c -
cd "$HERE"
export GOTOOLCHAIN="${GOTOOLCHAIN:-local}"
go run . "$HERE" "$PTAU"
