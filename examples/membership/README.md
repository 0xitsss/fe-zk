# Membership: Poseidon Merkle tree + Groth16

A group contract: `insert(leaf)` appends `leaf = Poseidon(secret)` to an incremental Merkle tree of depth 10, and a
Groth16 proof shows that the caller knows a leaf in the tree without saying which. `verifier/` is emitted by
`fe-zk-gen groth16` from the circuit's snarkjs key; `circuit/` holds the circom circuit.

```sh
scripts/demo.sh                          # from the repo root: emit the verifier, test it, test the demo contract
cd examples/membership && fe test        # the demo contract only
```

Step by step: [docs/USAGE.md](../../docs/USAGE.md#a-circom--snarkjs-groth16).
