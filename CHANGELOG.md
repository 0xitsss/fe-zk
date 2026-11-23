# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses semantic versioning.

## [0.4.0] - 2026-11-23

Plonky3 0.8.0 for the FRI/STARK path, `fe-zk-gen plonky3`. Crate and ingot manifests and generated file headers
now carry 0.4.0 (`fe-zk-gen 0.4.0`).

### Changed
- FRI/STARK verifier ported from Plonky3 0.4.2 to **0.8.0** (latest stable): domain-separated transcript (three seeds
  rebuilt from the configuration), squeezing proof of work, zero witnesses at zero-bit PoW sites, pruned Merkle
  multi-openings (`fri/merkle.fe` `verify_pruned`), `MerkleCap` commitments. Fixtures (`fib`, `fib_secure`, `mul`)
  re-proved with 0.8.0. The proof length now depends on the pruning: configuration modules export
  `FIXED_PROOF_WORDS` / `MAX_PROOF_WORDS` instead of `PROOF_WORDS`, `proof_words(params, proof)` takes the proof, and
  `examples/fri_stark` takes `verify(uint256[],uint256[3])`. Secure Fibonacci proof 2,721 to 1,844 words; execution gas
  in the test runner 1,366,516 to 1,782,844 (seeds and the multi-opening walk; see `docs/DESIGN.md`).

### Added
- `fe-zk-gen plonky3`: Rust port of `vectors/plonky3-stark/emit_fe.py` (configuration module + fixture), snapshot
  tests, and a byte-identical cross-check against the Python emitter (`gen/fe-zk-gen/tests/plonky3.rs`,
  `scripts/regen-plonky3-stark.sh`).

## [0.3.0] - 2026-11-22

Fe 26.5.0. Cheaper verifiers and a FRI/STARK verifier generic over the AIR. Crate and ingot manifests and generated
file headers now carry 0.3.0 (`fe-zk-gen 0.3.0`).

### Changed
- Gas tuning of the BN254 verifiers: shared `inv_modexp` (modexp precompile) instead of Fermat inversion in PLONK, gnark
  and KZG; closed-form `1 / 2^k` in the gnark key; contiguous `MemBuffer` halo2 transcript. Groth16 is now 1.02-1.03x,
  snarkjs PLONK 1.05x, gnark PLONK 1.10-1.11x and halo2 1.17x of the Solidity verifier ([docs/GAS.md](docs/GAS.md)).
- FRI/STARK verifier 70 % cheaper: 4.67 M down to 1.41 M execution gas (Plonky3 Fibonacci, 2^10 rows, 100 conjectured bits).

### Added
- `Air` trait (`src/verifier/fri/air.fe`) and `verify<A: Air>`: the FRI verifier is generic over the AIR. Fibonacci is ported
  and a second AIR, `MulAir` (6 columns, proved by Plonky3 0.4.2, `vectors/plonky3-stark/mul`), shares the test suite.

## [0.2.0] - 2026-11-19

Fe 26.5.0. A cleaner verifier API, a building kit for anonymous actions, and a Tornado Cash verifier demo.

### Added
- `fe_zk::anon`: `LeanImt` (insert-only LeanIMT, roots equal to `@semaphore-protocol/group` 4.x), `Nullifiers`
  (spent nullifiers per scope) and `hash_to_field` (Semaphore's `_hash`).
- `semaphore/` ingot (`fe_zk_semaphore`): `verify_proof`, `verify_proof_low` (depth 1..5), `verify_proof_high`
  (depth 6..10) and `key_digest`, with the keys of the public Semaphore ceremony (depth 1..10) emitted by
  `fe-zk-gen`; tested with real proofs of the official SDK and cross-checked against Semaphore's own
  `SemaphoreVerifier.sol` (`scripts/check-semaphore-sol.sh`).
- Examples `examples/anon_vote`, `examples/anon_claim`, `examples/anon_signal` (groups up to 32 members), and
  `examples/tornado`: the Tornado Cash classic `withdraw` verifier, accepting real mainnet proofs (verifier only, not a mixer).
- `fe-zk-gen groth16 --lib-module FILE [--module-name IDENT]`: emits only the verifying key (constants, compile-time
  checks, `pub fn vk()`), no contract.
- Failure rule: verifiers revert when the remaining gas is below the cap of the next precompile call plus
  `GAS_MARGIN` (5,000), so a starved call can no longer look like an invalid proof (`src/ec/precompile.fe`
  `require_gas`, `src/tests/failure_rule.fe`).
- `docs/USAGE.md` starts with "Anonymous membership in one evening"; `docs/SAFETY.md` has sections for the failure
  rule, `anon`, Semaphore and Tornado; `docs/API.md` lists `anon` and the Semaphore ingot.
- CI split into four jobs (`test`, `crosscheck`, `demos`, `life`).

### Changed (breaking)
One module per proof system, `use fe_zk::groth16` then `groth16::verify(..)`. Old name to new name:

| Old | New |
|---|---|
| `Groth16Vk<N>` (struct literal) | `groth16::Vk<N>` via `Vk::from_coords` (`Option`) or `Vk::from_consts` |
| `groth16_verify` | `groth16::verify` |
| `groth16_verify_raw` | `groth16::verify_calldata` |
| `PlonkVk`, `PlonkVk::new` | `plonk::Vk`, `plonk::Vk::from_coords` (`from_consts` unchanged) |
| `PlonkProof`, `plonk_verify` | `plonk::Proof`, `plonk::verify` |
| `plonk_verify_words` | `plonk::verify_calldata` |
| `GnarkVk`, `GnarkProof` | `gnark::Vk`, `gnark::Proof` |
| `gnark_plonk_verify`, `gnark_plonk_verify_bytes` | `gnark::verify`, `gnark::verify_calldata` |
| `src/verifier/gnark_plonk.fe` | `src/verifier/gnark.fe` |
| `KzgVk`, `KzgVk::new` | `kzg::Vk`, `kzg::Vk::from_coords` |
| `kzg_verify`, `kzg_batch_verify_gwc`, `kzg_verify_shplonk` | `kzg::verify`, `kzg::verify_gwc`, `kzg::verify_shplonk` |
| `Halo2Vk`, `Halo2Vk::new` | `halo2::Vk`, `halo2::Vk::from_coords` |
| `Groth16Proof` | `groth16::Proof` (`b` is a `G2Proof`) |

- Every function that reaches a precompile (verifiers, `g1_add`, `pairing_check`, ...) now declares
  `uses (call: Call, ctx: Ctx)`; callers add `ctx`.
- A degenerate key (`alpha`, `beta`, `gamma` or `delta` at infinity): `Vk::from_coords` returns `None`,
  `Vk::from_consts` yields a key whose `verify` returns `false`.
- Generated verifiers call the new entry points; ABIs, selectors and key digests are unchanged. Generated headers
  carry `fe-zk-gen 0.2.0`.

## [0.1.0] - 2026-11-24

First release. Fe 26.5.0.

### Added
- `src/field`: a `Field` trait and implementations for BN254 `Fr` and `Fq`, KoalaBear, BabyBear and Goldilocks
  (`Fp<P>`), plus the degree 4 extension `Ext4`. Hash, Merkle and transcript code is written once against the trait.
- `src/ec`: BN254 `G1` and `G2` with on-curve and subgroup checks, checked wrappers for the precompiles `ecAdd`,
  `ecMul` and `ecPairing`, a typed `pairing_check`, BLS12-381 wrappers for EIP-2537 and the EIP-4844 point evaluation
  precompile.
- `src/hash`: Poseidon (circomlib compatible, BN254), Poseidon2 (BN254 and KoalaBear), a Merkle path and a Keccak
  transcript, with test vectors from circomlibjs and Plonky3.
- `src/verifier`: Groth16, PLONK (snarkjs), PLONK (gnark v0.12.0, optional BSB22 commitments), halo2-KZG (SHPLONK and
  GWC), KZG single, GWC and SHPLONK openings, and a FRI/STARK verifier for a Plonky3 `uni-stark` Fibonacci proof.
- `gen/fe-zk-gen`: validates verifying keys and emits Fe verifier contracts with a key digest, compile-time key checks
  and a generated test battery, for Groth16 (snarkjs, arkworks), PLONK (snarkjs), halo2-KZG and gnark PLONK
  (`fe-zk-gen gnark`, with the ABI `Verify(bytes,uint256[])` of gnark's Solidity verifier).
- `gnark_plonk_verify_bytes`: the dynamic-ABI adapter, which rejects any proof or input length other than the key's.
- Examples: Poseidon Merkle membership with a Groth16 proof, zk Game of Life (on-chain stepping and a Groth16 proof
  of 512 generations), a quickstart, and generated verifiers for a real Semaphore v4 key, PLONK, halo2 and gnark.
- Fixtures from independent reference tools on real ceremony material (PSE Perpetual Powers of Tau) with the scripts
  that regenerate them; differential testing against the reference verifiers (`scripts/difftest.sh`).
- Docs: `docs/SAFETY.md` (rules, enforcement, tests), `docs/DESIGN.md`, `docs/GAS.md`, `docs/TESTING.md`,
  `docs/AI_LOG.md`, `docs/API.md` (generated by `scripts/gen-api-docs.py`, checked in CI), `SECURITY.md`.

### Security
- Every verifier rejects non-canonical field elements, off-curve points, wrong input counts and unchecked G2 points;
  the generator refuses to emit code that skips a check. Not audited.
