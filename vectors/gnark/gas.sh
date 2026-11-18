#!/usr/bin/env bash
# Gas of gnark's own exported Solidity PLONK verifier (v0.12.0, solc optimizer 200 runs) on the
# committed fixtures, executed in revm (Osaka). Needs: solc, python3, cargo (via vectors/plonk/gas).
# Rust toolchain on the dev box: PATH=/data/nightly-x86_64-unknown-linux-gnu/bin:$PATH CARGO_HOME=/data/cargo
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
GAS="$HERE/../plonk/gas"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
TARGET="${CARGO_TARGET_DIR:-$GAS/target}"
cargo build -q --release --manifest-path "$GAS/Cargo.toml" --target-dir "$TARGET"
for name in mul mulcommit; do
  d="$HERE/$name"
  solc --optimize --optimize-runs 200 --bin -o "$TMP/$name" "$d/Verifier.sol" >/dev/null
  sel=$(solc --hashes "$d/Verifier.sol" | grep -o '^[0-9a-f]\{8\}' | head -1)
  python3 "$HERE/calldata.py" "$d/fixture.json" "$sel" > "$TMP/$name.calldata"
  echo -n "$name: "
  "$TARGET/release/plonk-gas" "$TMP/$name/PlonkVerifier.bin" "$TMP/$name.calldata"
done
