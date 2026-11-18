#!/usr/bin/env bash
# Regenerates vectors/gnark/{mul,mulcommit}/{Verifier.sol,fixture.json} with gnark v0.12.0
# (gnark-crypto v0.15.0, pinned in go.mod/go.sum). Needs Go >= 1.22. The KZG SRS comes from gnark's
# test/unsafekzg (known toxic waste): TEST ONLY. Proofs are randomised (blinding), so regenerating
# changes every value; the Fe tests in src/tests/gnark_plonk_fixtures.fe must be regenerated too:
#   python3 scripts/gen-gnark-fe-fixtures.py
set -euo pipefail
HERE="$(cd "$(dirname "$0")/../vectors/gnark" && pwd)"
cd "$HERE"
export GOTOOLCHAIN="${GOTOOLCHAIN:-local}"
go run . "$HERE"
