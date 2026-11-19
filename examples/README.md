# examples

| Directory | What it is |
|---|---|
| `quickstart/` | the shortest path: a generated Groth16 verifier, a gate contract, Poseidon and a `Field`-generic helper |
| `membership/` | Poseidon Merkle tree plus a Groth16 membership proof (generated verifier) |
| `anon_vote/`, `anon_claim/`, `anon_signal/` | anonymous vote, claim and signal on `fe_zk::anon` and the Semaphore ingot (groups up to 32 members, see each README) |
| `tornado/` | the Tornado Cash classic `withdraw` verifier, real mainnet proofs (verifier only, not a mixer) |
| `life/` | zk Game of Life: on-chain stepping and a Groth16 proof of 512 generations |
| `plonk/`, `gnark/`, `halo2/`, `real_semaphore/` | generated verifiers (`fe-zk-gen`) with their tests |
| `fri_stark/` | FRI/STARK verifier contract for a Plonky3 Fibonacci proof |

Start with [docs/USAGE.md](../docs/USAGE.md). Each directory is an ingot with its own `fe test`.
