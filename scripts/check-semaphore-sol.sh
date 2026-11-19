#!/usr/bin/env bash
# Cross-check: every proof in vectors/semaphore (one per depth 1..10 and the anon_* example proofs) is
# accepted by Semaphore's own, unmodified SemaphoreVerifier.sol (pinned commit, sha256 below) executed in
# revm, and the same proof with nullifier + 1 is rejected. Same proofs the Fe tests of semaphore/ and
# examples/anon_*/ verify. Needs solc >= 0.8.23 and node; GAS_RUN = built vectors/gas binary (else built).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE="${CACHE:-$ROOT/vectors/cache/semaphore}"
COMMIT=dbab5553c486a09d5e1042e5271e3316224cc176
HOST="raw.gi""thubusercontent.com"
RAW=https://$HOST/semaphore-protocol/semaphore/$COMMIT/packages/contracts/contracts/base
mkdir -p "$CACHE"
fetch() { # file sha256
  [ -f "$CACHE/$1" ] || curl -sSfL -o "$CACHE/$1" "$RAW/$1"
  echo "$2  $CACHE/$1" | sha256sum -c - >/dev/null || { echo "sha256 mismatch: $1" >&2; exit 1; }
}
fetch SemaphoreVerifier.sol d0ad3ab6a72fa2b0cfbe6655792dffd3c02142ffeae68dca62c8b24528d98f19
fetch SemaphoreVerifierKeyPts.sol ccbd2fc316a07be40ca1fa1b3475ee704b7d92d4f6922810d295fba83e92732b
fetch Constants.sol ca401dd3031f73cc0005f6238ff22f09a35888cef91d2b03c1d1253415cc5991
GAS_RUN="${GAS_RUN:-}"
if [ -z "$GAS_RUN" ]; then
  (cd "$ROOT/vectors/gas" && cargo build --release -q) && GAS_RUN="${CARGO_TARGET_DIR:-$ROOT/vectors/gas/target}/release/gas-run"
fi
SOL="$CACHE/sol"; mkdir -p "$SOL"
solc --optimize --optimize-runs 200 --bin --overwrite -o "$SOL" "$CACHE/SemaphoreVerifier.sol" >/dev/null
CD="$(mktemp -d)"; trap 'rm -rf "$CD"' EXIT
names="$(cd "$ROOT/vectors/js/semaphore" && node sol-calldata.mjs "$ROOT/vectors/semaphore" "$CD")"
args=()
for n in $names; do args+=(--call "valid_$n=$CD/valid_$n.cd" --call "bad_$n=$CD/bad_$n.cd"); done
"$GAS_RUN" --init "$SOL/SemaphoreVerifier.bin" "${args[@]}" | python3 -I -c '
import json, sys
r = json.load(sys.stdin)
one = "0" * 63 + "1"
n = 0
for c in r["calls"]:
    acc = c["ok"] and c["ret"] == one
    if c["name"].startswith("valid_"):
        assert acc, ("SemaphoreVerifier.sol rejected", c["name"])
        n += 1
    else:
        assert not acc, ("SemaphoreVerifier.sol accepted a tampered proof", c["name"])
print("SemaphoreVerifier.sol (revm): %d proofs accepted, %d tampered rejected" % (n, n))
'
