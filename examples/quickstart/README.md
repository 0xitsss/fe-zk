# quickstart: use fe-zk in your own Fe project

> The full guide, with one recipe per proof system, deployment and a safety checklist, is
> [docs/USAGE.md](../../docs/USAGE.md). This page is the shortest path.

A 50-line contract (`src/lib.fe`) that uses a generated Groth16 verifier, `poseidon_2` and a
`Field`-generic helper. Tests: `fe test` in this directory (4 tests).

1. **Add the dependency** in your `fe.toml`:
   ```toml
   [dependencies]
   fe_zk = { path = "../fe-zk", allow_unsafe = true }
   # or, pinned remote dependency (Fe 26.5 keys: source, rev, optional path):
   # fe_zk = { source = "https://github.com/0xitsss/fe-zk.git", rev = "<full-commit-sha>", allow_unsafe = true }
   ```
   `allow_unsafe = true` lets fe_zk use the unsafe EVM primitives (precompile calls) it is built on.
2. **Export your verifying key** from your prover (snarkjs `verification_key.json`, arkworks, or
   a PLONK / halo2-KZG key).
3. **Generate the verifier** (never hand-write one):
   ```
   cd fe-zk/gen
   cargo run -p fe-zk-gen -- groth16 --snarkjs vk.json --proof proof.json --public public.json \
       --name MyCircuit --out ../../my-project/verifier --fe-zk-path ../../fe-zk
   ```
   Also `fe-zk-gen plonk ...` and `fe-zk-gen halo2 ...`. The generator validates the key (curve,
   subgroup, non-degenerate) and refuses to emit otherwise; the emitted ingot carries the key
   digest and a test with your proof plus a battery of invalid ones.
4. **Depend on the generated ingot**: `my_circuit_verifier = { path = "verifier" }`, then
   `fe fmt && fe test` inside `verifier/`.
5. **Call it** from your contract with a static call
   (`store.verifier.static(MyCircuitVerifierMsg::VerifyProof { a, b, c, input })`), so the verifier
   cannot touch your storage.
6. **Use the library**: `Bn254Fr::from_canonical(x)` for untrusted values, `poseidon_2(a, b)`,
   `merkle_root`, `Transcript`, all behind the `Field` trait.
7. **Write generic code once**: `fn horner<F: Field, const N: usize>(...)` in `src/lib.fe` runs over
   `Bn254Fr` and `KoalaBear` (test `horner_is_generic_over_fields`).
8. **Test**: `fe fmt --check && fe test && fe build`.

This directory was produced exactly like that, from `vectors/groth16/multiplier2`:
```
cd gen && cargo run -q -p fe-zk-gen -- groth16 --snarkjs ../vectors/groth16/multiplier2/verification_key.json \
    --proof ../vectors/groth16/multiplier2/proof.json --public ../vectors/groth16/multiplier2/public.json \
    --name Multiplier2 --out ../examples/quickstart/verifier --fe-zk-path ../../..
```
