#!/usr/bin/env bash
# Regenerates vectors/kzg/fixtures/kzg.json with arkworks 0.5.0 (TEST-ONLY insecure SRS).
set -euo pipefail
cd "$(dirname "$0")/../vectors/kzg"
cargo run --release --locked -q -- fixtures/kzg.json
echo "wrote vectors/kzg/fixtures/kzg.json"
