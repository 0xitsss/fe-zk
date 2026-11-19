# Tornado Cash classic: withdraw verifier in Fe

A **verifier demo only**. It is not a mixer: no pool, no deposits, no Merkle tree, no MiMC, no funds.

`fe-zk-gen groth16` turned the real `withdraw` verifying key of Tornado Cash classic (6 public inputs
`[root, nullifierHash, recipient, relayer, fee, refund]`) into `verifier/`, and the generated Fe contract accepts
real withdraw proofs taken from Ethereum mainnet transactions of the 0.1 ETH pool, and rejects tampered ones, with the
same verdicts as Tornado's own `Verifier.sol` (checked side by side in revm: `vectors/tornado/crosscheck.py`).

```
scripts/regen-tornado.sh          # key conversion, snarkjs check, generation, fe test, Solidity cross-check
cd examples/tornado/verifier && fe test
```

Tests: the generated battery (`src/tests.fe`) plus `src/mainnet_tests.fe`:
`real_mainnet_withdraw_proofs_accepted` (all fetched mainnet proofs) and `recipient_swap_rejected`
(valid proof, other recipient: the front-running attack the public inputs bind against),
`relayer_and_fee_mutation_rejected` and `refund_mutation_rejected` (every one of the six public inputs is bound).
Gas of one proof: ~231k execution in Fe vs ~236k for Verifier.sol (solc 0.7.6, optimizer 200), revm.

Sources and licences: see `vectors/tornado/README.md`. tornado-core is GPL-3.0; its `Verifier.sol` is MIT (Reitwiessner / OKIMS).
Nothing from it is copied: only the verifying-key numbers and public chain data. Context: the US Treasury (OFAC)
sanctions on the Tornado Cash contracts were lifted in March 2025.
