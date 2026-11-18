#!/usr/bin/env bash
# Membership demo: key -> fe-zk-gen -> generated Fe verifier (tested) -> demo contract (tested).
#   1. fe-zk-gen validates vectors/groth16/membership/verification_key.json and emits
#      examples/membership/verifier (contract + tests with the real snarkjs proof)
#   2. fe test of the generated verifier (valid proof + invalid battery + compile-time key checks)
#   3. fe test of the demo contract (Poseidon Merkle tree, Groth16 membership proof, nullifiers)
#   4. PLONK (snarkjs) verifier emitted from vectors/plonk/membership and tested
#   5. halo2-KZG verifiers (one contract per multiopen scheme) emitted from
#      vectors/halo2/standard_plonk_lookup and tested
#   5b. real-world Semaphore v4 key (public ceremony) emitted from vectors/groth16/real_semaphore_d10 and tested
#   5c. gnark PLONK verifier (ABI of gnark's Solidity verifier) emitted from vectors/gnark/mul and tested
#   6. code size table: every generated verifier and example contract must fit EIP-170 (24,576 bytes)
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

echo "== 4. PLONK: fe-zk-gen plonk -> generated verifier -> fe test"
P=vectors/plonk/membership
(cd gen && cargo run -q -p fe-zk-gen -- plonk \
    --snarkjs "../$P/verification_key.json" \
    --proof "../$P/proof.json" --public "../$P/public.json" \
    --name PlonkMembership --out ../examples/plonk/membership_verifier --fe-zk-path ../../..)
(cd examples/plonk/membership_verifier && "$FE" fmt >/dev/null && "$FE" test)

echo "== 5. halo2-KZG: fe-zk-gen halo2 -> generated verifier -> fe test"
H=vectors/halo2/standard_plonk_lookup
(cd gen && cargo run -q -p fe-zk-gen -- halo2 \
    --vk "../$H/vk.json" --shplonk "../$H/shplonk.json" --gwc "../$H/gwc.json" \
    --scheme both --name Halo2Circuit --out ../examples/halo2/circuit_verifier --fe-zk-path ../../..)
(cd examples/halo2/circuit_verifier && "$FE" fmt >/dev/null && "$FE" test)

echo "== 5b. real-world key: Semaphore v4 (public ceremony), fe-zk-gen groth16 -> generated verifier -> fe test"
R=vectors/groth16/real_semaphore_d10
(cd gen && cargo run -q -p fe-zk-gen -- groth16 \
    --snarkjs "../$R/verification_key.json" \
    --proof "../$R/proof.json" --public "../$R/public.json" \
    --name RealSemaphore --out ../examples/real_semaphore/verifier --fe-zk-path ../../..)
(cd examples/real_semaphore/verifier && "$FE" fmt >/dev/null && "$FE" test)

echo "== 5c. gnark PLONK (real ceremony SRS): fe-zk-gen gnark -> generated verifier -> fe test"
GN=vectors/gnark/mul/fixture.json
(cd gen && cargo run -q -p fe-zk-gen -- gnark \
    --vk "../$GN" --proof "../$GN" --public "../$GN" \
    --name GnarkMul --out ../examples/gnark/mul_verifier --fe-zk-path ../../..)
(cd examples/gnark/mul_verifier && "$FE" fmt >/dev/null && "$FE" test)

echo "== 6. deployed code size (EIP-170 limit 24,576 bytes; fails if any contract is over)"
scripts/code-size.sh examples/membership/verifier examples/membership \
    examples/plonk/membership_verifier examples/halo2/circuit_verifier \
    examples/real_semaphore/verifier examples/gnark/mul_verifier examples/life/verifier examples/life \
    examples/quickstart/verifier examples/quickstart examples/fri_stark

echo "demo OK"
