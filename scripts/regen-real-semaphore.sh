#!/usr/bin/env bash
# Real-world Groth16 key: Semaphore v4 (MIT), depth 10, from the public Semaphore trusted-setup ceremony.
#   1. fetch pinned artifacts (hashes below) into vectors/cache/semaphore (never committed)
#   2. generate a valid proof against the REAL ceremony zkey with a throw-away test identity/group
#      (vectors/js/semaphore/prove.mjs: asserts published vkey == zkey-exported vkey, proof verifies)
#   3. write vectors/groth16/real_semaphore_d10/{verification_key,proof,public,inputs}.json
#   4. cross-check: Semaphore's own published Solidity verifier (SemaphoreVerifier.sol, unmodified) accepts the
#      same proof in revm and rejects a tampered public signal (needs solc >= 0.8.23 and a built gas-run:
#      set GAS_RUN, or it is built from vectors/gas)
#   5. optional: `fe-zk-gen groth16 --snarkjs` into examples/real_semaphore/verifier (see vectors/README.md)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE="${CACHE:-$ROOT/vectors/cache/semaphore}"
OUT="$ROOT/vectors/groth16/real_semaphore_d10"
D=10
COMMIT=dbab5553c486a09d5e1042e5271e3316224cc176   # semaphore-protocol/semaphore main, 2026-10-17
RAW=https://raw.githubusercontent.com/semaphore-protocol/semaphore/$COMMIT
ART=https://snark-artifacts.pse.dev/semaphore/4.13.0           # @zk-kit/artifacts, Project.SEMAPHORE, version 4.13.0

mkdir -p "$CACHE"
fetch() { # url file sha256
  [ -f "$CACHE/$2" ] || curl -sSfL -o "$CACHE/$2" "$1"
  echo "$3  $CACHE/$2" | sha256sum -c - >/dev/null || { echo "sha256 mismatch: $2" >&2; exit 1; }
}
fetch "$ART/semaphore-$D.zkey" semaphore-$D.zkey f644af6753be9d48454d8f9bb74bf2e9a416dd7d1a704c3cf1d50e535969a19a
fetch "$ART/semaphore-$D.wasm" semaphore-$D.wasm 1e9e1e3ad9917164bc96c751e32fae724400b92063f5a2cfd22ca64674bf4f96
fetch "$RAW/packages/proof/src/verification-keys.json" verification-keys.json d96ec60593b6fc2334f1f04e008e07425d5536d7777f6ac21d996fc80779b3c9
SOLB=$RAW/packages/contracts/contracts
fetch "$SOLB/base/SemaphoreVerifier.sol" SemaphoreVerifier.sol d0ad3ab6a72fa2b0cfbe6655792dffd3c02142ffeae68dca62c8b24528d98f19
fetch "$SOLB/base/SemaphoreVerifierKeyPts.sol" SemaphoreVerifierKeyPts.sol ccbd2fc316a07be40ca1fa1b3475ee704b7d92d4f6922810d295fba83e92732b
fetch "$SOLB/base/Constants.sol" Constants.sol ca401dd3031f73cc0005f6238ff22f09a35888cef91d2b03c1d1253415cc5991

(cd "$ROOT/vectors/js/semaphore" && npm ci --no-audit --no-fund >/dev/null &&
  node prove.mjs $D "$CACHE/semaphore-$D.wasm" "$CACHE/semaphore-$D.zkey" "$CACHE/verification-keys.json" "$OUT")

# --- Solidity cross-check in revm -------------------------------------------------------------
GAS_RUN="${GAS_RUN:-}"
if [ -z "$GAS_RUN" ]; then
  (cd "$ROOT/vectors/gas" && cargo build --release -q) && GAS_RUN="${CARGO_TARGET_DIR:-$ROOT/vectors/gas/target}/release/gas-run"
fi
SOL="$CACHE/sol"; mkdir -p "$SOL"
solc --optimize --optimize-runs 200 --bin --overwrite -o "$SOL" "$CACHE/SemaphoreVerifier.sol" >/dev/null
(cd "$ROOT/vectors/js/semaphore" &&
  node calldata.mjs $D "$OUT/proof.json" "$OUT/public.json" > "$SOL/valid.cd" &&
  node calldata.mjs $D "$OUT/proof.json" "$OUT/public.json" tamper > "$SOL/tampered.cd")
res="$("$GAS_RUN" --init "$SOL/SemaphoreVerifier.bin" --call valid="$SOL/valid.cd" --call tampered="$SOL/tampered.cd")"
echo "$res" | python3 -I -c '
import json, sys
r = json.load(sys.stdin)
c = {x["name"]: x for x in r["calls"]}
one = "0" * 63 + "1"
assert c["valid"]["ok"] and c["valid"]["ret"] == one, ("Semaphore Solidity verifier rejected the proof", c["valid"])
assert not (c["tampered"]["ok"] and c["tampered"]["ret"] == one), "Semaphore Solidity verifier accepted a tampered signal"
print("Semaphore SemaphoreVerifier.sol (revm): valid accepted (exec gas %d), tampered rejected" % c["valid"]["exec"])
'
echo "real_semaphore_d10 fixtures written to $OUT"
