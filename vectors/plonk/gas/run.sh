#!/usr/bin/env bash
# Gas of the snarkjs Solidity PLONK verifier (solc, optimizer 200 runs) on the committed
# fixtures, executed in revm 33.1.0 (Osaka). Needs: solc, node, cargo. One line per fixture.
# Env for the Rust toolchain on the dev box: PATH=/data/nightly-x86_64-unknown-linux-gnu/bin:$PATH CARGO_HOME=/data/cargo
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
TARGET="${CARGO_TARGET_DIR:-$HERE/target}"
cargo build -q --release --manifest-path "$HERE/Cargo.toml" --target-dir "$TARGET"
for name in multiplier2 membership; do
  d="$HERE/../$name"
  solc --optimize --optimize-runs 200 --bin -o "$TMP/$name" "$d/Verifier.sol" >/dev/null
  n=$(python3 -c "import json;print(len(json.load(open('$d/public.json'))))")
  sel=$(solc --hashes "$d/Verifier.sol" | grep -o '^[0-9a-f]\{8\}' | head -1)
  node "$HERE/calldata.mjs" "$d" "$sel" > "$TMP/$name.calldata"
  echo -n "$name (nPublic=$n): "
  "$TARGET/release/plonk-gas" "$TMP/$name/PlonkVerifier.bin" "$TMP/$name.calldata"
done
