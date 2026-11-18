#!/usr/bin/env bash
# Regenerates vectors/halo2/standard_plonk_lookup/* (halo2-KZG proofs, vk, snark-verifier baseline), the
# Fe fixtures/verifiers derived from them, and runs the reference mini verifier tests.
#
# SRS: REAL ceremony material. halo2 ParamsKZG (k = 5) is converted from the PSE Perpetual Powers of Tau
# `ppot_0080_05.ptau` (downloaded to vectors/cache, sha256 pinned in vectors/halo2/src/ptau.rs; nobody knows
# tau). Circuit and vk are deterministic. Proof bytes are NOT byte-reproducible (halo2-axiom blinding is not
# fully driven by the seeded RNG): regenerated proofs are equally valid but differ; commit only when intended.
# Needs: a Rust toolchain (build from source; no AVX needed), `solc` on PATH for the snark-verifier Solidity
# baseline/gas, `fe` and `python3`. Crates are pinned in vectors/halo2/Cargo.lock.
# Steps after the Rust fixtures: Fe proof words (emit_fe.py --fixtures), expected challenges
# (gen-halo2-fe-challenges.py), the library module and the example ingot (fe-zk-gen halo2).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FE="${FE:-fe}"
H=vectors/halo2/standard_plonk_lookup
cd "$ROOT/vectors/halo2"
cargo run --release --locked --bin gen_fixtures -- standard_plonk_lookup
cargo test --release --locked
cd "$ROOT"
python3 vectors/halo2/emit_fe.py --fixtures "$H" --out src/tests/halo2_fixture.fe
python3 scripts/gen-halo2-fe-challenges.py
(cd gen && cargo run -q -p fe-zk-gen -- halo2 --vk "../$H/vk.json" --shplonk "../$H/shplonk.json" --gwc "../$H/gwc.json" \
    --scheme both --name Halo2Circuit --out ../examples/halo2/circuit_verifier --fe-zk-path ../../..
 cargo run -q -p fe-zk-gen -- halo2 --vk "../$H/vk.json" --lib-module ../src/verifier/halo2/standard_plonk_lookup.fe)
(cd examples/halo2/circuit_verifier && "$FE" fmt >/dev/null)
"$FE" fmt >/dev/null
