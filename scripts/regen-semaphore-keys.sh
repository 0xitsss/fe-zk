#!/usr/bin/env bash
# Regenerate semaphore/src/keys_d{1..10}.fe from Semaphore v4's published verification keys.
#   1. fetch packages/proof/src/verification-keys.json at the pinned commit (sha256 checked)
#   2. split it into one snarkjs vk per depth (alpha/beta/gamma shared; delta + IC per depth)
#   3. fe-zk-gen groth16 --snarkjs ... --lib-module semaphore/src/keys_d<k>.fe, then scripts/gen-semaphore-lib.py
#      (dispatch: shared alpha/beta/gamma + per-depth delta/IC) and fe fmt
# Env: GEN (fe-zk-gen binary; default: cargo run from gen/), CACHE, MAXD (default 10).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE="${CACHE:-$ROOT/vectors/cache/semaphore}"
COMMIT=dbab5553c486a09d5e1042e5271e3316224cc176
URL=https://raw.githubusercontent.com/semaphore-protocol/semaphore/$COMMIT/packages/proof/src/verification-keys.json
SHA=d96ec60593b6fc2334f1f04e008e07425d5536d7777f6ac21d996fc80779b3c9
MAXD="${MAXD:-10}"
mkdir -p "$CACHE"
[ -f "$CACHE/verification-keys.json" ] || curl -sSfL -o "$CACHE/verification-keys.json" "$URL"
echo "$SHA  $CACHE/verification-keys.json" | sha256sum -c - >/dev/null || { echo "sha256 mismatch" >&2; exit 1; }
python3 -I - "$CACHE" "$MAXD" <<'PY'
import json, sys
cache, maxd = sys.argv[1], int(sys.argv[2])
a = json.load(open(f"{cache}/verification-keys.json"))
for d in range(1, maxd + 1):
    vk = dict(a, vk_delta_2=a["vk_delta_2"][d - 1], IC=a["IC"][d - 1])
    json.dump(vk, open(f"{cache}/vk_d{d}.json", "w"), indent=1)
PY
if [ -n "${GEN:-}" ]; then run=("$GEN"); else run=(cargo run -q --manifest-path "$ROOT/gen/Cargo.toml" -p fe-zk-gen --); fi
mkdir -p "$ROOT/semaphore/src"
for d in $(seq 1 "$MAXD"); do
  "${run[@]}" groth16 --snarkjs "$CACHE/vk_d$d.json" --lib-module "$ROOT/semaphore/src/keys_d$d.fe" --module-name "semaphore_d$d"
done
python3 -I "$ROOT/scripts/gen-semaphore-lib.py"
(cd "$ROOT/semaphore" && "${FE:-fe}" fmt >/dev/null)
echo "wrote semaphore/src/keys_d{1..$MAXD}.fe and semaphore/src/lib.fe"
