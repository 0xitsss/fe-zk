# Limitations

What fe-zk does not do, or does with caveats. Short version in the [README](../README.md#limitations).

What works: everything listed under Approach is implemented and tested against real reference proofs. What does not,
and the assumptions:

- **Gas.** Fe verifiers cost 1.02-1.03x the Solidity ones for Groth16, 1.04-1.05x for snarkjs PLONK, 1.10-1.11x for
  gnark PLONK and 1.17x for halo2 (`docs/GAS.md`, revm, same proofs). Both sides are dominated by the same precompiles;
  the difference is per-call overhead: each precompile call costs about 300 gas of wrapper code in Fe (a heap
  `MemBuffer` from `Call.raw_staticcall`, a checked returndata length and the typed `G1` result, all of which keep the
  safety checks) against about 100 in hand-written Solidity assembly, and the typed field code is not unrolled
  (`docs/DESIGN.md`). Groth16 makes few calls, the PLONK family dozens. Deployed code is 1.31-1.98x the size of the
  Solidity versions (`docs/GAS.md`). Every generated
  verifier and example contract fits the EIP-170 limit of 24,576 bytes (guard: `scripts/code-size.sh`, run by
  `scripts/demo.sh`; ignored test `gen/fe-zk-gen/tests/code_size.rs`). `fe-zk-gen halo2` emits one contract per multiopen
  scheme (`--scheme shplonk|gwc|both`, default shplonk): SHPLONK 22.1 kB, GWC 21.4 kB, both together would be over. The FRI
  contract (`examples/fri_stark`, about 16 kB) fits as well.
- **FRI/STARK.** Generic over the AIR (`verify<A: Air>`; Fibonacci and a 6-column multiplication AIR are proved by Plonky3 and
  verified), constraint degree at most 2 (one quotient chunk), Plonky3 0.8.0 `uni-stark` (pruned Merkle multi-openings and domain-separated transcript; Merkle cap height 0, no zk mode, arity 2 only), KoalaBear with a degree-4 extension, Keccak commitments,
  arity 2. Query count, blowup, PoW bits and trace height are constants of a per-configuration module
  (`src/verifier/fri/config_secure.fe`, emitted from the proof). The secure configuration (2^10 rows, `log_blowup` 4,
  20 queries, 20 query-PoW bits) has 100 bits under Plonky3's conjecture (`log_blowup * queries + pow bits`); our estimate
  of what is *proven* is about 59 bits (Johnson bound) or 38 (unique decoding), see `docs/DESIGN.md`. The 4-query
  `config_demo` (about 11 bits) is only a second test configuration. Research-grade, not audited. A new AIR needs one `impl Air`
  (no verifier changes); higher-degree AIRs (several quotient chunks), other fields and preprocessed traces are not supported.
- **halo2.** One circuit shape: single phase, one instance column, uncommitted instances, no challenges. The emitter
  rejects duplicate commitments. The generated module is circuit-specific.
- **G2 points in proofs.** The subgroup check for a G2 point taken from a proof (`G2Proof`, e.g. Groth16 `B`) relies on
  the EIP-197 rule enforced by the pairing precompile, not on Fe code. It is tested against the test EVM (revm), not
  against every client. Key G2 points are fully checked (in Fe, at compile time).
- **`false` is not a verdict if your contract swallows reverts.** The verifiers return `false` for an invalid proof and
  revert when the forwarded gas is below what the next precompile call needs (the failure rule, `docs/SAFETY.md`
  section 10). A contract that turns any failed inner call into "invalid" (for example in a slashing rule) turns a
  starved call into a verdict; forward ample gas.
- **Example groups are capped at 32 members.** `anon_vote`, `anon_claim` and `anon_signal` use `verify_proof_low`
  (tree depth 1..5) so that the tree and the verifier fit one contract under EIP-170. Bigger groups need a separate
  verifier contract using `verify_proof_high` (`docs/USAGE.md`, "Beyond 32 members").
- **Semaphore: depth 1..10 only.** The ingot ships the ceremony keys for tree depth 1 to 10 (up to 1,024 members), not
  up to 32; a deeper group has no verifier here. The ingot verifies proofs only: the caller checks the root and spends
  the nullifier. `LeanImt` is insert-only (no update, no remove).
- **Tornado is verifier-only.** `examples/tornado` is not a mixer: no pool, no deposits, no Merkle tree, no funds.
- **Gas caps.** Precompile calls carry hard-coded gas caps set just above the current prices (EIP-1108, EIP-2565). A
  repricing makes the calls fail closed (revert or `false`), never open.
- **Setup provenance (what is real and what is not).** Real ceremony material: the halo2-KZG fixtures and the KZG
  fixtures use the PSE Perpetual Powers of Tau (`ppot_0080_05`, nobody knows tau; commitments are MSMs over the ceremony
  points); the snarkjs PLONK fixtures are universal and use `ppot_0080_14`; and `vectors/groth16/real_semaphore_d10` is
  the production Semaphore v4 key (public multi-party ceremony) with a proof we generated against its real zkey, run
  through `fe-zk-gen` into `examples/real_semaphore/verifier` (Semaphore's own Solidity verifier accepts the same proof
  in revm). Still test-only: **the Groth16 phase 2 of our own demo circuits** (`multiplier*`, `membership`, `life_*`)
  is a single contribution with a public entropy string, so the delta toxic waste is known and those keys are insecure
  even though phase 1 is the real ceremony; the arkworks Groth16 setup uses a fixed RNG seed. The gnark PLONK fixtures are no longer on a
  known-tau SRS: `vectors/gnark` converts the real PSE `ppot_0080_05` (sha256 pinned) to gnark's canonical and Lagrange
  SRS instead of using `unsafekzg`. Do not deploy any of the demo keys; deploy your own ceremony's key.
- **Malleability.** Groth16 proofs are malleable (rule 8); the examples never use a proof or its hash as an id.
- **Membership demo.** The tree is permissionless (anyone can insert) and has 1024 leaves; a deployment can bootstrap
  a root of an earlier tree (`bootstrap_root`) that the contract cannot verify. Proofs are not bound to `msg.sender`,
  so a front-runner can submit someone's proof first and burn the nullifier (the proof still verifies for the
  intended statement). The external nullifier is fixed per contract (the scope).
- **gnark keys.** gnark keys with an all-zero (infinity) selector commitment are refused (the generator refuses them
  too). This is a liveness limitation, not a soundness one. The generated contract returns `false` for a wrong proof or
  input length where gnark's Solidity verifier reverts.
- **Game of Life.** The stored board is public (the demo shows succinctness, not privacy); `N` = 512 is fixed by the
  key; the circuit setup needs a 2^20 ptau and 9 GB of RAM to prove.
- **Generated code style.** The emitters write valid but not `fe fmt`-clean code; `scripts/demo.sh` runs `fe fmt` on
  the output. The gnark generator reads the JSON that `vectors/gnark` exports (key fields as 0x words), not gnark's
  binary `vk` serialisation.
- **BLS12-381.** The EIP-2537 wrappers and the point-evaluation wrapper are tested on the test EVM (Prague/Osaka); no
  verifier is built on them yet beyond the KZG tests.
- **Not audited.** No external review. See `docs/SAFETY.md` for what is enforced and tested.
