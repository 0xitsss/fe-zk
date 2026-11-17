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

The phase-2 contribution uses a fixed public entropy string and the arkworks setup uses a
fixed RNG seed: all keys are INSECURE test fixtures. Circom setups and proofs are randomised, so every
regeneration changes verification_key.json and proof.json (and any digest that pins them).
