# anon_claim

Anonymous claim from an allow-list: a member claims once, to any recipient, without revealing which
member they are.

* Deploy with `(scope = campaign id)`; the deployer registers the allow-list (identity commitments).
* `claim(recipient, depth, root, nullifier, points)`: the proof's message is the recipient address,
  so a front-runner cannot redirect a claim they copy from the mempool. A second claim by the same
  member (any recipient) reverts.
* The contract records `claimed(recipient)` and emits `Claimed`; the payout goes where `// PAYOUT`
  is marked in `src/lib.fe`.
* Group limit: 32 holders (`verify_proof_low`).

`fe test` runs real SDK proofs (`vectors/semaphore/examples/claim.json`). Gas of one claim: about 286k.

## Why 32 members, and how to go beyond

The cap is a code-size limit, not a cryptographic one. A `LeanImt` is about 14 KB and `verify_proof_low` (tree depth
1 to 5, the keys of groups up to 2^5 = 32) about 7 KB, which fits in one contract (EIP-170: 24,576 bytes); adding all ten
keys (`verify_proof`, about 11 KB) does not. For bigger groups (depth 6 to 10, up to 1,024 members) put the verifier in
its own contract using `verify_proof_high` and call it from the group contract with a static call. A tested example
(`HighVerifier` and a caller) is in [docs/USAGE.md](../../docs/USAGE.md), "Beyond 32 members". `register` stops at
`MAX_MEMBERS` here so that no member is registered who could never prove.
