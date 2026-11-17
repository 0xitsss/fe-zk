#!/usr/bin/env bash
# Membership demo: key -> fe-zk-gen -> generated Fe verifier (tested) -> demo contract (tested).
#   1. fe-zk-gen validates vectors/groth16/membership/verification_key.json and emits
#      examples/membership/verifier (contract + tests with the real snarkjs proof)
#   2. fe test of the generated verifier (valid proof + invalid battery + compile-time key checks)
#   3. fe test of the demo contract (Poseidon Merkle tree, Groth16 membership proof, nullifiers)
# Needs: `fe` (26.5.0) and a Rust toolchain on PATH. Set FE=/path/to/fe to override the binary.
set -euo pipefail
cd "$(dirname "$0")/.."
FE="${FE:-fe}"
V=vectors/groth16/membership

echo "== 1. fe-zk-gen"
(cd gen && cargo run -q -p fe-zk-gen -- groth16 \
    --snarkjs "../$V/verification_key.json" \
    --proof "../$V/proof.json" --public "../$V/public.json" \
    --name Membership --out ../examples/membership/verifier --fe-zk-path ../../..)
(cd examples/membership/verifier && "$FE" fmt >/dev/null)

echo "== 2. generated verifier tests"
(cd examples/membership/verifier && "$FE" test)

echo "== 3. demo contract tests"
(cd examples/membership && "$FE" test)

echo "demo OK"
