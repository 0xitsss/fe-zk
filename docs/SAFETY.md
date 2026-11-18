# fe-zk safety rules: where each is enforced and tested

This file maps every safety rule from `CLAUDE.md` section 3 to the code that enforces it and the
test that would fail if the check were removed. Line numbers refer to the current `main`.
Paths are relative to the repository root. "Generated" tests are the tests emitted by `fe-zk-gen` next to the
generated verifier (see section 9), e.g. `examples/membership/verifier/src/tests.fe`.

## 1. Non-canonical field elements are rejected, never reduced

| Where | How | Test |
|---|---|---|
| `src/field/fp.fe:41` `Fp::from_canonical` | returns `None` for `v >= P`; the value field of `Fp` is private, so code outside the module cannot build an element any other way except the explicit `reduce` | `src/tests/field.fe` `rejects_non_canonical` |
| `src/ec/bn254.fe:147` `G1::from_coords`, `:319` `G2::from_coords`, `:398` `G2Proof::from_coords` | coordinates `>= q` rejected | `src/tests/ec.fe` `g1_rejects_invalid`, `g2_rejects_invalid` |
| `src/ec/bls12_381.fe:84`, `:147` `from_words` | non-canonical Fp words rejected before the precompile | `src/tests/ec_bls.fe` `g1_validation` |
| `src/verifier/kzg.fe` | all inputs typed `G1` / `G2` / `Bn254Fr`; raw words cannot be passed | `src/tests/kzg.fe` `raw_inputs_need_checked_constructors` |
| `src/verifier/fri.fe:270` `proof_is_canonical` | runs first in `verify`; every KoalaBear element of proof and public values `< p` | `src/tests/fri_suite.fe` `rejects_non_canonical_elements` (run for both configurations from `fri.fe`, `fri_secure.fe`) |
| `src/hash/poseidon.fe` | inputs typed `Bn254Fr` | `src/tests/hash.fe` `poseidon_inputs_must_be_canonical` |
| `src/verifier/groth16.fe:77` `verify_raw` | public inputs through `Bn254Fr::from_canonical` (`:89`), `A`/`C` through `G1::from_coords` (`:99`, `:105`), `B` through `G2Proof::from_coords` (`:111`); any failure returns `false`; the typed `verify` (`:49`) cannot receive a raw word | `src/tests/groth16.fe` `groth16_multiplier_battery`, `groth16_membership_battery`, `groth16_arkworks_multiplier_battery` (inputs `r` and `x + r`, coordinates `+ q`); generated `non_canonical_public_input_rejected`, `non_canonical_coordinate_rejected` |
| `src/verifier/plonk.fe:241` `PlonkProof::from_words`, `:222` `proof_point`, `:379` `plonk_verify` | six evaluations through `from_canonical` (`:251`-`:256`), commitments through `G1::from_coords`, public inputs through `from_canonical` (`:394`) | `src/tests/plonk.fe` `rejects_non_canonical_public_input`, `rejects_non_canonical_evaluation`, `rejects_off_curve_and_non_canonical_points` |
| `src/verifier/gnark_plonk.fe:290` `GnarkProof::from_words`, `:748` `gnark_plonk_verify` | evaluations, BSB22 openings (`:330`) and public inputs (`:762`) through `from_canonical`; every point through `G1::from_coords` (`:272`, `:307`-`:310`, `:338`) | `src/tests/gnark_plonk.fe` `gnark_rejects_non_canonical_public_input`, `gnark_rejects_non_canonical_evaluation`, `gnark_rejects_off_curve_and_non_canonical_points` |
| `src/verifier/halo2/common.fe:82` `read_g1`; `src/verifier/halo2/standard_plonk_lookup.fe:137` `phase1` | every instance and proof scalar through `from_canonical` (`:147`, `:250` onwards), every proof point through `read_g1` | `src/tests/halo2.fe` `shplonk_rejects_non_canonical_and_off_curve`, `gwc_rejects_non_canonical_and_off_curve`; generated `shplonk_non_canonical_instance_rejected`, `gwc_non_canonical_words_rejected` |
| `fe-zk-gen` | key coordinates `< q` checked before any other check (`validate.rs:15` `canonical`, `:135`, `:141`); the parsers keep raw big integers and never reduce | `gen/fe-zk-gen/tests/vk.rs` `rejects_non_canonical_coordinate`, `arkworks_rejects_off_curve_and_non_canonical` |

`Field::reduce` exists for constants and for Fiat–Shamir challenges (`keccak mod r`); it is never applied to
untrusted input.

## 2. Curve membership of every G1 point; explicit point at infinity

| Where | How | Test |
|---|---|---|
| `src/ec/bn254.fe:129` `g1_is_valid`, `:147` `G1::from_coords` | `y^2 = x^3 + 3` over Fq; `(0, 0)` is the only accepted encoding of infinity | `src/tests/ec.fe` `g1_accepts_valid`, `g1_rejects_invalid` |
| `src/ec/bls12_381.fe:84` | curve + subgroup checked by the EIP-2537 MSM precompile with scalar 1 | `src/tests/ec_bls.fe` `g1_validation` |
| `src/verifier/kzg.fe:40` `KzgVk::new` | rejects infinity in the key | `src/tests/kzg.fe` `vk_rejects_infinity` |
| `src/verifier/groth16.fe:99`, `:105`, `:111` | `A`, `C` from calldata only through `G1::from_coords` (curve check; `(0, 0)` is the explicit infinity and then fails the pairing); `B` through `G2Proof::from_coords` (twist check) | `src/tests/groth16.fe` `groth16_multiplier_battery` (`A + 1`, `C + 1`, `B` off the twist, `C = infinity`); generated `point_not_on_curve_rejected` |
| `src/verifier/plonk.fe:222` `proof_point` | proof commitments through `G1::from_coords` | `src/tests/plonk.fe` `rejects_off_curve_and_non_canonical_points`; generated `point_not_on_curve_rejected`, `infinity_commitment_rejected` |
| `src/verifier/gnark_plonk.fe:272` | proof commitments on curve; `Z`, `W_z`, `W_zw` and the BSB22 commitments must not be infinity (`:317`, `:340`) | `src/tests/gnark_plonk.fe` `gnark_rejects_off_curve_and_non_canonical_points` |
| `src/verifier/halo2/common.fe:82` `read_g1` | on curve, infinity rejected | `src/tests/halo2.fe` `shplonk_rejects_non_canonical_and_off_curve`; generated `shplonk_off_curve_points_rejected`, `shplonk_infinity_points_rejected`, `gwc_infinity_points_rejected` |
| `const fn g1_is_valid` in generated keys | every key point `static_assert`ed (`examples/membership/verifier/src/lib.fe:47`, `:55`-`:58`); an invalid constant never becomes a usable point (`from_coords_or_infinity`, `src/ec/bn254.fe:158`) | `gen/fe-zk-gen/tests/emit.rs` `corrupted_key_constant_fails_to_compile`; generated `vk_constants_pass_full_checks` |

## 3. G2 points: on curve and in the subgroup

| Where | How | Test |
|---|---|---|
| `src/ec/bn254.fe:278` `g2_is_valid`, `:319` `G2::from_coords` | canonical, on the twist, `psi(P) == [6x^2]P` (about 755k gas at runtime, free as a `const fn` in `static_assert`); used for key constants | `src/tests/ec.fe` `g2_accepts_valid`, `g2_rejects_invalid`, `g2_rejects_non_subgroup` |
| `src/ec/bn254.fe:392` `G2Proof` | proof G2 points (Groth16 `B`): canonical and on the twist in Fe; the type has no conversion to `G2` and is consumed only by `pairing_check_proof` / `pairing_try_proof` (`:525`, `:542`), where the precompile completes the check | `src/tests/groth16.fe` `proof_g2_non_subgroup_passes_cheap_check_then_pairing_rejects`; generated `b_not_in_subgroup_rejected` |
| pairing precompile 0x08 | EIP-197 requires the subgroup check ("the order of the element has to be checked"); proof G2 elements rely on it (see `docs/DESIGN.md`, "Decision: G2 points from proofs") | `src/tests/ec.fe` `precompile_vs_non_subgroup` |
| `fe-zk-gen` (`gen/fe-zk-gen/src`) | verifying-key G2 points checked with ark-bn254 at generation time (`validate.rs:77` `check_g2`) | `gen/fe-zk-gen/tests/vk.rs` `rejects_g2_not_in_subgroup` |
| generated keys, all families | every G2 key point `static_assert(g2_is_valid(..))` (`examples/membership/verifier/src/lib.fe:49`, `:51`, `:53`); PLONK `[x]_2`, gnark `[x]_2`, halo2 `g2` / `s_g2` and KZG `tau_g2` are typed `G2`, so only checked points can be built (`PlonkVk::new` `src/verifier/plonk.fe:111`, `from_consts` `:171`) | `src/tests/plonk.fe` `vk_x2_must_be_in_subgroup`; `src/tests/gnark_plonk.fe` `gnark_vk_x2_must_be_a_checked_g2`; generated `invalid_keys_fail_the_compile_time_checks` (PLONK, halo2); `gen/fe-zk-gen/tests/plonk.rs` `corrupted_plonk_key_fails_to_compile`; `gen/fe-zk-gen/tests/halo2.rs` `corrupted_halo2_key_fails_to_compile` |

## 4. Public-input count and layout fixed by the key

| Where | How | Test |
|---|---|---|
| `src/verifier/kzg.fe` | batch size is a const generic `N`; `assert(N != 0)` at `:141` | `src/tests/kzg.fe` `gwc_rejects_swapped_or_resized` |
| `src/verifier/fri.fe:655` `verify` | public values typed `[u256; 3]`; proof length must equal `proof_words(params)`, `params_valid` bounds the configuration | `src/tests/fri_suite.fe` `rejects_wrong_public_values`, `rejects_wrong_configuration` |
| `fe-zk-gen` | `IC.len() == nPublic + 1` (`validate.rs:98`) | `gen/fe-zk-gen/tests/vk.rs` (wrong IC count, three ways) |
| `src/verifier/groth16.fe:36` `Groth16Vk<N>`, `:49`, `:77` | `N` is a const generic of the key; inputs are `[Bn254Fr; N]` / `[u256; N]`; the generated contract fixes `N_PUBLIC` and the ABI `uint256[N]` | `src/tests/groth16.fe` `groth16_multiplier2_battery` (swapped inputs); generated `swapped_public_inputs_rejected`, `all_zero_public_inputs_rejected` |
| `src/verifier/plonk.fe:111` `PlonkVk::new` (`NP == 0` rejected at `:119`, `NP > 2^power` at `:123`), `:171` `from_consts` (same conditions, poisons the key), `:379` `plonk_verify`, `:528` `plonk_verify_words` | `NP` const generic; the proof is exactly `[u256; 24]`; the number of public inputs cannot exceed the evaluation domain size (`NP <= 2^power`) | `src/tests/plonk.fe` `rejects_swapped_public_inputs`, `rejects_proof_of_other_circuit`, `vk_rejects_bad_domain_parameters` |
| `src/verifier/gnark_plonk.fe:126` `GnarkVk::new` (`NP == 0`, `NC > 1000` rejected at `:135`), `:748` | `NP` and the BSB22 commitment count `NC` are const generics | `src/tests/gnark_plonk.fe` `gnark_rejects_swapped_public_inputs`, `gnark_rejects_proof_of_other_circuit`, `gnark_vk_rejects_bad_parameters` |
| `src/verifier/halo2/standard_plonk_lookup.fe:137` `phase1` | `instances: [u256; 1]`, `proof: [u256; L]` fixed by the emitted circuit | `src/tests/halo2.fe` `shplonk_rejects_wrong_instance`, `gwc_rejects_wrong_instance` |
| `fe-zk-gen` PLONK / halo2 | `n_public >= 1`; fixture instance count must equal the key (`plonk.rs:145`, `:419`; `halo2.rs:125` `validate`) | `gen/fe-zk-gen/tests/plonk.rs` `invalid_keys_rejected`, `refuses_bad_names_and_mismatched_fixtures`; `gen/fe-zk-gen/tests/halo2.rs` `invalid_keys_rejected`, `fixture_size_mismatch_rejected` |

## 5. Fiat–Shamir transcripts bind everything

| Where | How | Test |
|---|---|---|
| `src/hash/transcript.fe:41`, `:62` | tagged keccak per operation, label and counter in every squeeze | `src/tests/hash.fe` `transcript_changing_any_input_changes_the_challenge`, `transcript_domain_separation` |
| `src/verifier/kzg.fe:108` `gwc_challenge`, `:157` `shplonk_challenge_y`, `:180` `shplonk_challenge_u` | GWC `v` and SHPLONK `y`, `u` bind the whole key, points, commitments and values | `src/tests/kzg.fe` `gwc_challenge_binds_every_input`, `shplonk_challenges_bind_every_input`, `*_matches_reference` |
| `src/verifier/fri.fe:306` `replay_transcript` | Plonky3 observation order, checked challenge by challenge against Plonky3 | `src/tests/fri.fe` `transcript_matches_plonky3`, `transcript_binds_public_values_and_trace_commitment`, `transcript_binds_claimed_evaluations`, `transcript_binds_fri_commitments`, `transcript_binds_final_poly_and_query_pow` (`src/tests/fri_suite.fe`) |
| Groth16 | no transcript. The verifying key is fixed in the generated contract; its keccak digest is exposed as `VK_DIGEST` | generated `vk_digest_exposed`; `gen/fe-zk-gen/tests/emit.rs` `emission_is_deterministic_and_carries_digest` |
| `src/verifier/plonk.fe:317` `plonk_challenges` | `beta, gamma, alpha, xi, v, u` in the snarkjs 0.7.6 order, binding the 8 key commitments, public inputs, every proof commitment and evaluation | `src/tests/plonk.fe` `transcript_binds_every_vk_commitment`, `transcript_binds_every_public_input`, `transcript_binds_every_commitment`, `transcript_binds_every_evaluation`, `challenges_match_snarkjs_multiplier2`, `challenges_match_snarkjs_membership` |
| `src/verifier/gnark_plonk.fe:460` `gnark_challenges`, `:538` `gnark_trace`, `:404` `hash_to_fr` | gnark's sha256 chain (gamma, beta, alpha, zeta, folding, random challenge) over key, public inputs, commitments, evaluations | `src/tests/gnark_plonk.fe` `gnark_transcript_binds_every_vk_commitment`, `gnark_transcript_binds_every_public_input`, `gnark_transcript_binds_every_commitment`, `gnark_folding_and_random_challenges_bind_everything`, `challenges_match_gnark_mul`, `challenges_match_gnark_mulcommit`, `bsb22_hash_to_field_matches_gnark` |
| `src/verifier/halo2/common.fe:101` `Halo2Transcript`; `standard_plonk_lookup.fe:146` | key `transcript_repr`, instances and every point and scalar absorbed in snark-verifier `EvmTranscript` order; one-word squeezes domain-separated | `src/tests/halo2.fe` `shplonk_challenges_bind_every_input`, `gwc_challenges_bind_every_input`, `transcript_absorbs_coordinates`, `transcript_one_word_squeeze_is_domain_separated`, `shplonk_challenges_match_rust`, `gwc_challenges_match_rust` |

## 6. Precompile calls: success flag, returned length, exact result

| Where | How | Test |
|---|---|---|
| `src/ec/precompile.fe:38` `call_try`, `:59`; `:98` `call_try_fixed`, `:115`; `:122`, `:143` | success flag checked; returndata length must equal the expected length | `src/tests/ec.fe` `pairing_accepts_and_rejects`; `src/tests/ec_bls.fe` `precompile_rejects_invalid_point` |
| `src/ec/bn254.fe:466`, `:519`-`:520`, `:529`-`:530`, `src/ec/bls12_381.fe:368`-`:369` | pairing word must be 0 or 1 (`assert(w <= 1)`); `true` only for exactly 1 | `src/tests/ec.fe` `pairing_bilinearity`, `src/tests/ec_bls.fe` `pairing` |
| `src/ec/precompile.fe` | explicit gas cap per call: a failing precompile consumes all gas forwarded to it | `src/tests/ec.fe` `precompile_vs_non_subgroup` |
| `src/ec/eip4844.fe:62`-`:63` | output must be exactly `(4096, BLS_MODULUS)` | `src/tests/ec_bls.fe` `point_evaluation_c_kzg_vector` |
| `src/verifier/halo2/common.fe:129` `inv_modexp` (precompile 0x05) and `src/verifier/gnark_plonk.fe:377` `sha256` (precompile 0x02) | `call_checked`: success flag and exact returndata length (32); the modexp result goes through `from_canonical` | `src/tests/halo2.fe` `shplonk_accepts`; `src/tests/gnark_plonk.fe` `bsb22_hash_to_field_matches_gnark` (a wrong result changes every challenge) |
| Groth16 / PLONK / gnark / halo2 final check | `pairing_try_proof` (Groth16) returns `false` for a failed precompile call and for any word other than 1; `pairing_check` (PLONK, gnark, halo2) reverts on a failed call and is true only for 1 | `src/tests/groth16.fe` `groth16_multiplier_battery`; `src/tests/plonk.fe` `rejects_tampered_commitment`; `src/tests/gnark_plonk.fe` `gnark_rejects_tampered_commitment`; `src/tests/halo2.fe` `shplonk_rejects_every_word_plus_one` |

`std::evm::crypto::ec_pairing` is not used: it returns `word != 0` and does not check the returned length.

## 7. No unchecked arithmetic on untrusted values

All field arithmetic goes through `addmod` / `mulmod` on canonical values (`src/field/fp.fe`). `Fp::sub`
relies on `other.v < P` (`src/field/fp.fe:69`-`:82`, bound stated in the comments there; `Fp` values are
canonical by construction). Every `#[arithmetic(unchecked)]` in `src/hash/` (`poseidon2.fe:48`, `:92`, `:111`;
the generated `poseidon_unrolled.fe` and `poseidon2_unrolled.fe`) states its bound next to the code (lazy
reduction bounds, `docs/DESIGN.md` "Hash gas"). Tests: the fast paths equal the checked references at maximal
inputs (`src/tests/hash.fe` `poseidon_fast_equals_reference`, `poseidon2_fast_equals_generic`) and the constant
tables are canonical (`poseidon_tables_are_canonical`, `poseidon2_tables_are_canonical`). Verifier code has no
`unchecked` arithmetic; calldata words enter only through the checked constructors of rules 1 to 3.

## 8. Malleability

Groth16 proofs are malleable: anyone can re-randomize `(A, B, C)` into another valid proof for the same
public inputs. The membership example records the `nullifierHash` public input, never the proof or its hash.
Code: `examples/membership/src/lib.fe:11`-`:15` (rule stated), `:140` (`assert(external_nullifier == store.scope)`:
the external nullifier is fixed per contract, so one member cannot mint a fresh nullifier per call), `:142` and
`:154` (`nullifiers[nullifier_hash]` checked before the proof and set only after it verified; only the nullifier
hash is stored). Tests: `examples/membership/src/tests.fe` `proof_flow_unknown_root_tampering_and_replay` (a
failed attempt does not burn the nullifier; a replay with the same nullifier is rejected) and
`proof_for_another_scope_rejected` (a valid proof for another external nullifier is rejected). The generated verifiers state the malleability in their header
(`gen/fe-zk-gen/src/emit.rs`). `examples/life` stores the end board, not a proof or proof hash.

## 9. Generator hygiene

| Check | Where | Test |
|---|---|---|
| Points on curve, in subgroup, coordinates canonical | `gen/fe-zk-gen/src/validate.rs:97` `Groth16Vk::validate`, `:65` `check_g1`, `:77` `check_g2`; `plonk.rs:144`; `halo2.rs:125` | `gen/fe-zk-gen/tests/vk.rs` `rejects_g1_off_curve`, `rejects_ic_off_curve`, `rejects_g2_off_curve`, `rejects_non_canonical_coordinate`, `rejects_g2_not_in_subgroup`; `tests/plonk.rs` and `tests/halo2.rs` `invalid_keys_rejected` |
| alpha, beta, gamma, delta not the identity | same (`validate.rs:97`) | `gen/fe-zk-gen/tests/vk.rs` `rejects_gamma_identity`, `rejects_delta_identity`, `rejects_alpha_and_beta_identity` |
| IC count = nPublic + 1 | `validate.rs:98` | `gen/fe-zk-gen/tests/vk.rs` `rejects_wrong_ic_count` |
| Key digest (keccak over a documented encoding, `gen/README.md`) | `digest.rs:48` `key_digest` (refuses invalid keys); `plonk.rs:213` `plonk_key_digest`; `halo2.rs:275` `halo2_key_digest` | `gen/fe-zk-gen/tests/vk.rs` `digest_layout_and_sensitivity`, `digest_refuses_invalid_key`; `tests/plonk.rs` `digest_is_deterministic_and_binds_the_key`; `tests/halo2.rs` `digest_matches_embedded_constant_and_is_stable` |
| Emitters refuse invalid keys; there is no flag to skip checks | `emit.rs:77` (`emit_ingot` calls `key_digest`, which validates), `plonk.rs:249`, `halo2.rs:275`; the CLI (`main.rs`) has no skip option | `gen/fe-zk-gen/tests/emit.rs` `refuses_invalid_keys`; `tests/plonk.rs` `invalid_keys_rejected` |
| Header with tool version and key digest; digest embedded as a constant | `emit.rs:92`-`:97`, `plonk.rs:262`-`:272`, `halo2.rs:567` `header` | `tests/emit.rs` `emission_is_deterministic_and_carries_digest`, `tests/halo2.rs` `digest_matches_embedded_constant_and_is_stable`; generated `vk_digest_exposed` |
| Key re-validated by the Fe compiler | `static_assert(g1_is_valid(..))`, `static_assert(g2_is_valid(..))`, `static_assert(!VK_*.is_infinity())` emitted at `emit.rs:164`-`:175`; domain checks for PLONK (`static_assert(plonk_domain_is_valid(..))`) | `gen/fe-zk-gen/tests/emit.rs` `corrupted_key_constant_fails_to_compile`; `tests/plonk.rs` `corrupted_plonk_key_fails_to_compile`; `tests/halo2.rs` `corrupted_halo2_key_fails_to_compile` |
| Emitted tests: one valid proof plus an invalid battery, one failing case per check | `emit.rs:266` `emit_tests`; PLONK and halo2 equivalents | `gen/fe-zk-gen/tests/emit.rs` `every_check_has_a_failing_case_in_generated_tests`, `emitted_ingots_pass_fe_test` (runs `fe`, ignored by default); `tests/plonk.rs` and `tests/halo2.rs` same names; generated `valid_proof_accepted` and the `*_rejected` tests |
| Emitted code is byte-stable | `gen/fe-zk-gen/tests/snapshots` | `tests/emit.rs` `snapshot_membership`, `tests/plonk.rs` `snapshot_membership`, `tests/halo2.rs` `rust_port_is_byte_identical_to_python_emitter` |
| Generator on a production key | `fe-zk-gen groth16 --snarkjs` on the Semaphore v4 depth-10 key (public ceremony), emitted to `examples/real_semaphore/verifier`; the same checks, header and battery as every other key | `examples/real_semaphore/verifier/src/tests.fe` (15 tests, run by `scripts/demo.sh` step 5b); `scripts/regen-real-semaphore.sh` also runs Semaphore's own Solidity verifier on the same proof (`vectors/README.md`, "Real-world keys") |

| gnark generator (`fe-zk-gen gnark`): key validation, digest, compile-time checks, dynamic ABI lengths | `gen/fe-zk-gen/src/gnark.rs` `GnarkVk::validate`, `gnark_key_preimage`, `emit_gnark_ingot` (`static_assert(gnark_domain_is_valid(..))`, `g1_is_valid`, `g2_is_valid`, constraint-index bounds); `src/verifier/gnark_plonk.fe` `gnark_plonk_verify_bytes` (exact proof and input length, rule 4) | `gen/fe-zk-gen/tests/gnark.rs` `invalid_keys_rejected`, `digest_is_deterministic_and_binds_the_key`, `selector_signature_matches_gnark_solidity_verifier`, `snapshot_mulcommit`, `corrupted_gnark_key_fails_to_compile`, `emitted_gnark_ingots_pass_fe_test`; `src/tests/gnark_plonk.fe` `verify_bytes_rejects_wrong_lengths`; generated `wrong_proof_length_rejected`, `wrong_public_input_count_rejected`, `poisoned_keys_reject_every_proof` (`examples/gnark/mul_verifier/src/tests.fe`, `scripts/demo.sh` step 5c) |

The emitted contracts only forward calldata to library entry points (`groth16_verify_raw`, `plonk_verify_words`,
`gnark_plonk_verify_words`, `gnark_plonk_verify_bytes`) or to the per-circuit halo2 module, so the generator cannot drop a checked rule:
the checks live in `src/`, not in templates.
