#!/usr/bin/env bash
# Reproduce the reference tooling used to generate test vectors.
#   circom  v2.2.2 (built from source at a pinned commit, outside the repo)
#   snarkjs 0.7.6, circomlib 2.0.5, circomlibjs 0.1.7 (pinned in vectors/js/package-lock.json)
# Env: ZK_TOOLS_DIR (default $HOME/.cache/fe-zk-tools) holds the circom checkout and binary.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZK_TOOLS_DIR="${ZK_TOOLS_DIR:-$HOME/.cache/fe-zk-tools}"
CIRCOM_TAG=v2.2.2
CIRCOM_COMMIT=e410b0d5cd2948a15931df0bc50d79ce56fa8c32
BIN="$ZK_TOOLS_DIR/bin/circom"

mkdir -p "$ZK_TOOLS_DIR/bin"
if [ -x "$BIN" ] && "$BIN" --version | grep -q "2.2.2"; then
  echo "circom already present: $("$BIN" --version)"
else
  SRC="$ZK_TOOLS_DIR/circom-src"
  [ -d "$SRC/.git" ] || git clone --branch "$CIRCOM_TAG" --depth 1 https://github.com/iden3/circom.git "$SRC"
  got="$(git -C "$SRC" rev-parse HEAD)"
  [ "$got" = "$CIRCOM_COMMIT" ] || { echo "circom commit mismatch: $got" >&2; exit 1; }
  # NOTE: with a very new nightly, rustix 0.37.x (pulled in via Cargo.lock) fails with
  # "attributes starting with rustc are reserved"; use a stable toolchain (see vectors/README.md).
  (cd "$SRC" && cargo build --release --target-dir "$ZK_TOOLS_DIR/circom-target")
  cp "$ZK_TOOLS_DIR/circom-target/release/circom" "$BIN"
fi
"$BIN" --version

(cd "$ROOT/vectors/js" && npm ci --ignore-scripts)
echo "tools ready; export CIRCOM=$BIN"
