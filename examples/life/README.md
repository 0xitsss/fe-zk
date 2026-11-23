# zk Game of Life

A 16x16 toroidal board packed into one `u256`. Two ways to advance it:

- **On chain:** the `Life` contract steps the board with word-parallel bit operations, 657 gas per generation.
- **By proof:** one Groth16 proof that 512 generations were computed correctly from a committed start board to a
  committed end board (Poseidon commitments), checked by a verifier that `fe-zk-gen` emitted (`verifier/`).
  Break-even is about 392 generations.

```sh
scripts/life-demo.sh                                    # from the repo root: both paths, about 5 minutes
cd examples/life/cli && cargo run --release -- --prove  # one proof vs 512 on-chain steps
cd examples/life/cli && cargo run --release -- --pattern glider --gens 200 --delay 80
```

The CLI needs `fe build` run once in `examples/life` and `examples/life/verifier`; `scripts/life-demo.sh` does it.

`circuit/` holds the circom circuit, `cli/` the Rust runner (revm), `ref/` the per-cell reference used to check the
on-chain stepping.
