#!/usr/bin/env bash
# Regenerates vectors/halo2/standard_plonk_lookup/* (halo2-KZG proofs, vk, snark-verifier baseline)
# and runs the reference mini verifier tests. SRS, vk, circuit: deterministic (fixed seed, test-only
# INSECURE SRS). Proof bytes are NOT byte-reproducible (halo2-axiom blinding is not fully driven by the
# seeded RNG): regenerated proofs are equally valid but differ; commit them only when intended.
# Needs: a Rust toolchain (build from source; no AVX needed) and `solc` on PATH for the
# snark-verifier Solidity baseline/gas. Crates are pinned in vectors/halo2/Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/../vectors/halo2"
cargo run --release --locked --bin gen_fixtures -- standard_plonk_lookup
cargo test --release --locked
