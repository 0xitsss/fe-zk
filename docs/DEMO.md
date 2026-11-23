# Demo

Reproducible CLI examples; the commands are in the [README](../README.md#try-it).

### Membership and generated verifiers (`scripts/demo.sh`)

1. `fe-zk-gen groth16 --snarkjs vectors/groth16/membership/verification_key.json --proof ... --public ...` validates
   the key and emits `examples/membership/verifier` (contract, key constants, tests with the real snarkjs proof).
2. `fe test` there: valid proof accepted; tampered `A`/`B`/`C`, tampered / non-canonical / swapped public inputs,
   off-curve and non-subgroup points all rejected.
3. `fe test` in `examples/membership`: a Poseidon incremental Merkle tree (depth 10, roots checked against circomlibjs)
   and `prove(root, nullifierHash, ...)`, which checks the root, the nullifier, then calls the generated verifier.
4. The same pipeline for a snarkjs PLONK key (`fe-zk-gen plonk`) and a halo2-KZG key (`fe-zk-gen halo2`).

CI runs it in its own job (`demos`) and checks that no generated file changed (`git diff --exit-code`); the anonymous
examples and the Tornado verifier run there too.

### zk Game of Life (`scripts/life-demo.sh`, about 5 minutes)

`examples/life/cli` (`life-cli`, Rust, revm) executes the compiled Fe `Life` contract: each generation is one
`step(board)` call, compared with a per-cell Rust reference. The circuit (`examples/life/circuit/life.circom`, 7
constraints per cell) proves that 512 generations were computed from a private start board whose Poseidon commitment
is public, ending at a board with the second public commitment. The verifier is the `fe-zk-gen` output
`Life512Verifier`; `advanceProven(endLo, endHi, proof)` recomputes both commitments on chain and stores the end board.
`life-cli --frames-json PATH` dumps the boards and gas that drive the animation at the top of the README.

Output of `life-cli --pattern glider --gens 64 --check` (excerpt) and `life-cli --prove`:

```
gen    0  alive   5  start
gen   16  alive   5  step gas 657 (tx 21885)
...
check: OK, 64 generations match the per-cell reference
gas, 64 separate step() calls: 42048 execution, 1400880 with tx overhead
gas, one run(board, 64) call:  34246 execution, 55614 with tx overhead

fixture vectors/groth16/life_512  proven generations 512
tampered proof:    REJECTED
wrong end board:   REJECTED
wrong start board: REJECTED

gas (revm, Osaka)                                 execution      with tx
512 separate step() calls                            336393     11205525
one run(board, 512) call                             271686       293066
one proveRun (verify proof of the same run)          207758       234394
one advanceProven (verify + 2 Poseidon + store)       264127       289579

break-even: a proof pays off above about 392 generations (531 gas per on-chain generation)
```

Three measurements of Life gas exist and differ slightly: `life-cli` calls the `Life` contract, which forwards to the
verifier (so `proveRun` includes the extra call); `docs/GAS.md` calls the verifier directly (199,593 Fe, 195,613
Solidity; break-even about 385 generations against `run`); and `fe test --show-logs` prints `GasUsed` events
(`evm.gas()` around the call), a few percent off. The method of each is stated where it is reported.
