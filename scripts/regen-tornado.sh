#!/usr/bin/env bash
# Tornado Cash classic (0.1 ETH pool) withdraw verifier demo. Verifier only: no pool, deposits or MiMC.
#   1. fetch pinned tornado-core sources (sha256 below) into vectors/cache/tornado (never committed)
#   2. convert the key hard-coded in Verifier.sol to snarkjs format (vectors/tornado/verification_key.json)
#   3. FETCH=1: re-fetch the real mainnet withdraw calldata (read-only JSON-RPC) into vectors/tornado/proofs
#   4. snarkjs `groth16 verify` of every proof against the converted key
#   5. fe-zk-gen -> examples/tornado/verifier, mainnet tests, fe test
#   6. revm cross-check against Verifier.sol (solc 0.7.6, static binary pinned by sha256)
# Needs: fe, cargo, curl, python3, snarkjs (NODE_MODULES_DIR, default vectors/js/node_modules).
# Env: FE, CACHE, CARGO_TARGET_DIR, FETCH=1, RPC_URL.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
FE="${FE:-fe}"
CACHE="${CACHE:-$ROOT/vectors/cache/tornado}"
NM="${NODE_MODULES_DIR:-$ROOT/vectors/js/node_modules}"
T=vectors/tornado
COMMIT=1ef6a263ac6a0e476d063fcb269a9df65a1bd56a   # tornadocash/tornado-core master, 2022-03-24
RAW=https://raw.githubusercontent.com/tornadocash/tornado-core/$COMMIT
mkdir -p "$CACHE"
fetch() { # url file sha256
  [ -f "$CACHE/$2" ] || curl -sSfL -o "$CACHE/$2" "$1"
  echo "$3  $CACHE/$2" | sha256sum -c - >/dev/null || { echo "sha256 mismatch: $2" >&2; exit 1; }
}
fetch "$RAW/contracts/Verifier.sol" Verifier.sol a2fd9a12dc008ae95e1f9da2968e7d017617e804f22aa0491d2e8a91d965a824
fetch "$RAW/contracts/Tornado.sol" Tornado.sol 64e0d454d6c077c0ae14f3e826378ad09bae57f1642e8dc1b0824a33a19c2014
fetch "$RAW/LICENSE" LICENSE 3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986
SOLC="$CACHE/solc-0.7.6"
fetch "https://binaries.soliditylang.org/linux-amd64/solc-linux-amd64-v0.7.6+commit.7338295f" solc-0.7.6 \
  bd69ea85427bf2f4da74cb426ad951dd78db9dfdd01d791208eccc2d4958a6bb
chmod +x "$SOLC"

echo "== 2. convert key"
python3 -I $T/convert_vk.py "$CACHE/Verifier.sol" $T/verification_key.json

if [ "${FETCH:-0}" = 1 ]; then
  echo "== 3. fetch mainnet calldata"
  python3 -I $T/fetch.py ${RPC_URL:-}
fi

echo "== 4. snarkjs verify of every mainnet proof"
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
for f in $T/proofs/*.json; do
  b="$(basename "$f" .json)"
  python3 -I $T/to_snarkjs.py "$f" "$W/$b"
  "$NM/.bin/snarkjs" groth16 verify $T/verification_key.json "$W/$b/public.json" "$W/$b/proof.json" 2>&1 | grep -q "OK!" \
    || { echo "snarkjs rejected $b" >&2; exit 1; }
  echo "   $b OK"
done

echo "== 5. fe-zk-gen + fe test"
first="$(ls $T/proofs/*.json | head -1)"; b="$(basename "$first" .json)"
(cd gen && cargo build -q --release -p fe-zk-gen)
GEN="${CARGO_TARGET_DIR:-gen/target}/release/fe-zk-gen"
"$GEN" groth16 --snarkjs $T/verification_key.json --proof "$W/$b/proof.json" --public "$W/$b/public.json" \
  --name Tornado --out examples/tornado/verifier --fe-zk-path ../../..
python3 -I $T/gen_fe_tests.py examples/tornado/verifier/src/mainnet_tests.fe
(cd examples/tornado/verifier && "$FE" fmt >/dev/null && "$FE" test)

echo "== 6. revm cross-check vs Verifier.sol (solc 0.7.6)"
(cd vectors/gas && cargo build --release -q)
GAS_RUN="${CARGO_TARGET_DIR:-$ROOT/vectors/gas/target}/release/gas-run"
"$FE" build examples/tornado/verifier -O 1 --out-dir "$W/fe" >/dev/null
"$SOLC" --optimize --optimize-runs 200 --bin --overwrite -o "$W/sol" "$CACHE/Verifier.sol" >/dev/null
python3 -I $T/crosscheck.py "$GAS_RUN" "$W/fe/TornadoVerifier.bin" "$W/sol/Verifier.bin"
