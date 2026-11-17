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
