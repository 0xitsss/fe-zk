# fe-zk safety rules: where each is enforced and tested

This file maps every safety rule from `CLAUDE.md` section 3 to the code that enforces it and the
test that would fail if the check were removed. Line numbers refer to the current `main`.
Rows marked *pending* are filled in when that verifier lands.

## 1. Non-canonical field elements are rejected, never reduced

| Where | How | Test |
|---|---|---|
| `src/field/fp.fe:41` `Fp::from_canonical` | returns `None` for `v >= P`; the value field of `Fp` is private, so code outside the module cannot build an element any other way except the explicit `reduce` | `src/tests/field.fe` `rejects_non_canonical` |
| `src/ec/bn254.fe:99` `G1::from_coords`, `:238` `G2::from_coords` | coordinates `>= q` rejected | `src/tests/ec.fe` `g1_rejects_invalid`, `g2_rejects_invalid` |
| `src/ec/bls12_381.fe:84`, `:147` `from_words` | non-canonical Fp words rejected before the precompile | `src/tests/ec_bls.fe` `g1_validation` |
| `src/verifier/kzg.fe` | all inputs typed `G1` / `G2` / `Bn254Fr`; raw words cannot be passed | `src/tests/kzg.fe` `raw_inputs_need_checked_constructors` |
| `src/verifier/fri.fe:130` `proof_is_canonical` | runs first in `verify`; every KoalaBear element of proof and public values `< p` | `src/tests/fri.fe` `rejects_non_canonical_elements` |
| `src/hash/poseidon.fe` | inputs typed `Bn254Fr` | `src/tests/hash.fe` `poseidon_inputs_must_be_canonical` |
| Groth16, PLONK, halo2 | *pending* | |

`Field::reduce` exists for constants and for Fiat–Shamir challenges (`keccak mod r`); it is never applied to
untrusted input.

## 2. Curve membership of every G1 point; explicit point at infinity

| Where | How | Test |
|---|---|---|
| `src/ec/bn254.fe:99` `G1::from_coords` | `y^2 = x^3 + 3` over Fq; `(0, 0)` is the only accepted encoding of infinity | `src/tests/ec.fe` `g1_accepts_valid`, `g1_rejects_invalid` |
| `src/ec/bls12_381.fe:84` | curve + subgroup checked by the EIP-2537 MSM precompile with scalar 1 | `src/tests/ec_bls.fe` `g1_validation` |
| `src/verifier/kzg.fe:40` `KzgVk::new` | rejects infinity in the key | `src/tests/kzg.fe` `vk_rejects_infinity` |

## 3. G2 points: on curve and in the subgroup

| Where | How | Test |
|---|---|---|
| `src/ec/bn254.fe:238` `G2::from_coords` | canonical, on the twist, `psi(P) == [6x^2]P` (about 755k gas); used for key constants | `src/tests/ec.fe` `g2_accepts_valid`, `g2_rejects_invalid`, `g2_rejects_non_subgroup` |
| pairing precompile 0x08 | EIP-197 requires the subgroup check ("the order of the element has to be checked"); proof G2 elements rely on it (see `docs/DESIGN.md`, "Decision: G2 points from proofs") | `src/tests/ec.fe` `precompile_vs_non_subgroup` |
| `fe-zk-gen` (`gen/fe-zk-gen/src`) | verifying-key G2 points checked with ark-bn254 at generation time | `gen/fe-zk-gen/tests/vk.rs` (non-subgroup beta/gamma/delta) |

## 4. Public-input count and layout fixed by the key

| Where | How | Test |
|---|---|---|
| `src/verifier/kzg.fe` | batch size is a const generic `N`; `assert(N != 0)` at `:121` | `src/tests/kzg.fe` `gwc_rejects_swapped_or_resized` |
| `src/verifier/fri.fe` | public values typed `[u256; 3]` | `src/tests/fri.fe` `rejects_wrong_public_values` |
| `fe-zk-gen` | `IC.len() == nPublic + 1` | `gen/fe-zk-gen/tests/vk.rs` (wrong IC count, three ways) |
| Groth16, PLONK, halo2 | *pending* | |

## 5. Fiat–Shamir transcripts bind everything

| Where | How | Test |
|---|---|---|
| `src/hash/transcript.fe:41`, `:62` | tagged keccak per operation, label and counter in every squeeze | `src/tests/hash.fe` `transcript_changing_any_input_changes_the_challenge`, `transcript_domain_separation` |
| `src/verifier/kzg.fe:88`, `:137`, `:160` | GWC `v` and SHPLONK `y`, `u` bind the whole key, points, commitments and values | `src/tests/kzg.fe` `gwc_challenge_binds_every_input`, `shplonk_challenges_bind_every_input`, `*_matches_reference` |
| `src/verifier/fri.fe:181` `replay_transcript` | Plonky3 observation order, checked challenge by challenge against Plonky3 | `src/tests/fri.fe` `transcript_matches_plonky3`, `every_transcript_input_changes_the_challenges` |
| Groth16 (no transcript), PLONK, halo2 | *pending* | |

## 6. Precompile calls: success flag, returned length, exact result

| Where | How | Test |
|---|---|---|
| `src/ec/precompile.fe:38` `call_try`, `:59` | success flag checked; returndata length must equal the expected length | `src/tests/ec.fe` `pairing_accepts_and_rejects`; `src/tests/ec_bls.fe` `precompile_rejects_invalid_point` |
| `src/ec/bn254.fe:380`, `src/ec/bls12_381.fe:368` | pairing word must be 0 or 1; `true` only for exactly 1 | `src/tests/ec.fe` `pairing_bilinearity`, `src/tests/ec_bls.fe` `pairing` |
| `src/ec/precompile.fe` | explicit gas cap per call: a failing precompile consumes all gas forwarded to it | `src/tests/ec.fe` `precompile_vs_non_subgroup` |
| `src/ec/eip4844.fe` | output must be exactly `(4096, BLS_MODULUS)` | `src/tests/ec_bls.fe` `point_evaluation_c_kzg_vector` |

`std::evm::crypto::ec_pairing` is not used: it returns `word != 0` and does not check the returned length.

## 7. No unchecked arithmetic on untrusted values

All field arithmetic goes through `addmod` / `mulmod` on canonical values (`src/field/fp.fe`). `Fp::sub`
relies on `other.v < P` (comment at the line). Any internal fast path in `src/hash/` states its bound next
to the code.

## 8. Malleability

Groth16 proofs are malleable: anyone can re-randomize `(A, B, C)` into another valid proof for the same
public inputs. The membership example records the `nullifierHash` public input, never the proof or its hash.
*Pending: link to the code and the replay test when the example lands.*

## 9. Generator hygiene

| Check | Where | Test |
|---|---|---|
| Points on curve, in subgroup, coordinates canonical | `gen/fe-zk-gen/src` `validate()` | `gen/fe-zk-gen/tests/vk.rs` |
| alpha, beta, gamma, delta not the identity | same | same |
| IC count = nPublic + 1 | same | same |
| Key digest (keccak over a documented encoding, `gen/README.md`) | `key_digest` (refuses invalid keys) | digest layout and sensitivity tests |
| Emitted verifier with header, digest, valid + invalid proof tests | *pending* | |
