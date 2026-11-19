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
