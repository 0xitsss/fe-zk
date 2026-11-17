#!/usr/bin/env bash
# Regenerates vectors/halo2/standard_plonk_lookup/* (halo2-KZG proofs, vk, snark-verifier baseline)
# and runs the reference mini verifier tests. Deterministic: fixed RNG seeds, test-only insecure SRS.
# Needs: a Rust toolchain (build from source; no AVX needed) and `solc` on PATH for the
# snark-verifier Solidity baseline/gas. Crates are pinned in vectors/halo2/Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/../vectors/halo2"
cargo run --release --locked --bin gen_fixtures -- standard_plonk_lookup
cargo test --release --locked
