# vectors

See CLAUDE.md, section 2. `scripts/regen-vectors.sh` regenerates everything here (one section
per component). Vectors come from independent reference implementations.

## hash

| Directory | Reference | Version |
|---|---|---|
| `poseidon/` | circomlibjs `poseidon` (`vectors/js`) | circomlibjs 0.1.7 |
| `poseidon2/bn254_t3.json` | HorizenLabs/poseidon2 `plain_implementations` known-answer test | commit `055bde3f4782731ba5f5ce5888a440a94327eaf3` |
| `poseidon2/koalabear_t16.json` | Plonky3 `default_koalabear_poseidon2_16` and `TruncatedPermutation<_, 2, 8, 16>` (`vectors/plonky3`) | p3-koala-bear 0.9.0-rc.2 |
| `transcript/` | Keccak-256 via ethers over the layout in `src/hash/transcript.fe` | ethers 5.8.0 |

Generated Fe: `src/hash/poseidon_constants.fe`, `src/hash/poseidon2_constants.fe`,
`src/tests/hash_vectors.fe`.

Limits: the HorizenLabs reference ships a single known-answer test for BN254 t = 3 (input
`[0, 1, 2]`), so that is the only independent BN254 Poseidon2 vector. The BN254 Merkle
compression has no independent vector; the KoalaBear Merkle path is checked against a tree built
by Plonky3's `TruncatedPermutation`.

## Groth16 (`vectors/groth16/<name>/`)

Regenerate: `scripts/setup-tools.sh` then `scripts/regen-groth16-fixtures.sh` (~2-3 min).

| name | source | public inputs |
|------|--------|---------------|
| `multiplier` | circom, `vectors/circuits/multiplier.circom` (c = a*b) | 1: `[c]` |
| `multiplier2` | circom, `vectors/circuits/multiplier2.circom` | 2: `[c, a]` |
| `membership` | circom, `examples/membership/circuit/membership.circom` (depth 10) | 3: `[nullifierHash, root, externalNullifier]` |
| `arkworks_multiplier` | ark-groth16 0.5.0, `gen/fe-zk-gen/examples/arkworks_fixture.rs` | 2: `[c, a]` |

Files: `verification_key.json`, `proof.json`, `public.json`, plus for the snarkjs ones
`verify.txt` (output of `snarkjs groth16 verify`), `Verifier.sol` (snarkjs Solidity verifier,
for the gas comparison) and `r1cs-info.txt`.

Tools: circom 2.2.2 (commit e410b0d5), snarkjs 0.7.6, circomlib 2.0.5, circomlibjs 0.1.7
(`vectors/js/package-lock.json`). Powers of tau: PSE perpetual `ppot_0080_14.ptau`, sha256
`3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db` (checked by the script;
cached in `vectors/cache/`, never committed). The Hermez S3/GCS buckets return 403 for anonymous
users, hence the PSE mirror. The pinned hash was recorded at first download (trust on first use).

Phase 1 is the real perpetual ceremony. The phase-2 contribution of our own circuits uses a fixed public entropy string
(a single contributor, delta toxic waste is known) and the arkworks setup uses a fixed RNG seed: those keys are INSECURE
test fixtures. The production Semaphore key is under "Real-world keys". Circom setups and proofs are randomised, so every
regeneration changes verification_key.json and proof.json (and any digest that pins them).

## Real ceremony SRS (halo2-KZG and KZG)

`vectors/halo2/standard_plonk_lookup/` and `vectors/kzg/fixtures/kzg.json` are built on the **PSE Perpetual Powers of
Tau** (the same ppot used above), not on a seeded or known-tau SRS.

| item | value |
|---|---|
| file | `ppot_0080_05.ptau` (challenge 0080, power 5: 2^6-1 G1 powers, 2^5 G2 powers; we use 2^5 G1 and `[tau]G2`) |
| URL | `https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_05.ptau` |
| sha256 | `9c1bfc4a2895b9beb1df6e95c96785dc9382bee84d84a0d8c56b1061e09d89d1` (130,194 bytes), trust on first use |
| ceremony | Perpetual Powers of Tau (Hermez/PSE, BN254; `0080` is the state after the 80th contribution, soundness holds if any one contributor was honest); the Hermez buckets are no longer anonymously readable, hence the PSE mirror |
| cross-check | the 2^6 G1 and 2^5 G2 powers equal the prefix of the independently cached `ppot_0080_17.ptau` (sha256 `f807e065...a367c`): ignored test `ptau_prefix_matches_larger_file` |

Checks on the converted material (`cargo test --release` in `vectors/halo2`, `ptau_params_consistent`): every point is
on the curve (G2: also in the r-torsion), `g[0]` and `tauG2[0]` are the BN254 generators, one common tau along the whole
chain (`e([tau^(i+1)]G, H) = e([tau^i]G, [tau]H)` for all i, which includes `e([tau]G1, G2) == e(G1, [tau]G2)`), and the
Lagrange basis computed by halo2's `g_to_lagrange` is the group inverse FFT of the monomial powers (`commit_lagrange` of
the evaluations of `1` and of `X` equals `g[0]` and `g[1]`).

* **halo2** (`vectors/halo2/src/ptau.rs`): snarkjs ptau layout (section 2 `tauG1`, section 3 `tauG2`, little-endian
  Montgomery coordinates) -> `ParamsKZG<Bn256>`: `g` = first 2^k powers verbatim, `g_lagrange` computed, `g2` =
  generator, `s_g2` = `tauG2[1]`. `scripts/regen-halo2.sh` regenerates SHPLONK/GWC proofs (circuit unchanged), `vk.json`,
  the snark-verifier Solidity baseline and gas, the Fe verifiers and the expected challenges. The Python emitter
  cross-check (`gen/fe-zk-gen/tests/halo2.rs`) and the mini verifier tests pass on the new vk.
* **KZG** (`vectors/kzg`): commitments are `sum_i p_i [tau^i]G1` (arkworks MSM over the ceremony points), the opening
  proof is the same MSM of the quotient; tau never appears. `scripts/regen-kzg.sh` then
  `scripts/gen-kzg-fe-fixtures.py` rewrite `src/tests/kzg.fe`.

## Real-world keys

| name | source | public inputs |
|---|---|---|
| `groth16/real_semaphore_d10` | Semaphore v4, tree depth 10, public Semaphore trusted-setup ceremony | 4: `[merkleRoot, nullifier, hash(message), hash(scope)]` |

Semaphore v4 (`semaphore-protocol/semaphore`, license **MIT**) is the most widely deployed circom/snarkjs Groth16 system in
the Ethereum privacy stack. Its keys come from the multi-party Semaphore ceremony, not from a single contributor.

Pinned sources (`scripts/regen-real-semaphore.sh` checks every hash; nothing large is committed):

| artifact | source | sha256 |
|---|---|---|
| per-depth verification keys | `semaphore-protocol/semaphore` commit `dbab5553c486a09d5e1042e5271e3316224cc176`, `packages/proof/src/verification-keys.json` | `d96ec60593b6fc2334f1f04e008e07425d5536d7777f6ac21d996fc80779b3c9` |
| ceremony zkey, depth 10 | `https://snark-artifacts.pse.dev/semaphore/4.13.0/semaphore-10.zkey` (`@zk-kit/artifacts`, Project.SEMAPHORE 4.13.0) | `f644af6753be9d48454d8f9bb74bf2e9a416dd7d1a704c3cf1d50e535969a19a` |
| circuit wasm, depth 10 | `https://snark-artifacts.pse.dev/semaphore/4.13.0/semaphore-10.wasm` | `1e9e1e3ad9917164bc96c751e32fae724400b92063f5a2cfd22ca64674bf4f96` |
| `SemaphoreVerifier.sol` | same commit, `packages/contracts/contracts/base/` | `d0ad3ab6a72fa2b0cfbe6655792dffd3c02142ffeae68dca62c8b24528d98f19` |
| `SemaphoreVerifierKeyPts.sol` | same | `ccbd2fc316a07be40ca1fa1b3475ee704b7d92d4f6922810d295fba83e92732b` |
| `Constants.sol` | same | `ca401dd3031f73cc0005f6238ff22f09a35888cef91d2b03c1d1253415cc5991` |

Semaphore publishes no ready-made proof, so the proof is ours but made with the **real zkey**: `vectors/js/semaphore/prove.mjs`
(`@semaphore-protocol/{identity,group}` 4.14.5, snarkjs 0.7.6, input layout and `hash()` copied from
`@semaphore-protocol/proof` `generateProof`) proves membership of a throw-away test identity in a 5-member group with
`message = 1`, `scope = 42`. The script asserts that Semaphore's published vkey for depth 10 (shared alpha/beta/gamma, per
depth delta and IC) **equals** `snarkjs zkey export verificationkey` of the ceremony zkey, that `snarkjs groth16 verify`
accepts the proof, and that a changed nullifier is rejected. Committed: `verification_key.json` (that slice of the
published key), `proof.json`, `public.json`, `inputs.json` (public part only). The proof is randomised, a regeneration changes it.

Checks: (1) `fe-zk-gen groth16 --snarkjs` validates the key (curve, subgroup, non-degenerate gamma/delta, 5 IC points for 4
public inputs) and emits `examples/real_semaphore/verifier` (key digest `0xcc185f74...7211`), whose generated battery of
15 tests passes (valid proof, tampered A/B/C, tampered/swapped/non-canonical/all-zero public inputs, point off curve,
B not in subgroup, swapped real/imag, A/C swapped, key constants re-validated). (2) Cross-check: Semaphore's own
unmodified `SemaphoreVerifier.sol` (solc 0.8.24, optimizer 200 runs) accepts the same proof in revm (execution gas
214,120) and rejects a tampered signal.

Not covered: the other 31 depths (same generator, only delta/IC differ) and Semaphore's per-depth key table for an
on-chain verifier shared across depths (our emitted verifier is per key, as for any generated Groth16 verifier).
