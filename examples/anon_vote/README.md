# anon_vote

Anonymous voting: one vote per member, nobody learns who voted for what. About 100 lines of Fe on
top of `fe_zk::anon` and `fe_zk_semaphore`.

* Deploy with `(scope = poll id, options = N)`. The deployer registers voters' Semaphore identity
  commitments with `register`.
* A voter makes a proof with `@semaphore-protocol/proof` `generateProof(identity, group, option, scope)`
  and sends `vote(option, depth, root, nullifier, points)`.
* The contract checks option range, known root, unspent nullifier, the proof; then spends the
  nullifier and counts the vote. The scope is fixed at deploy, the option is the proof's message:
  copying a pending proof and changing the option fails; the same member voting twice fails.
* Group limit: 32 voters (`verify_proof_low`, see `semaphore/README.md`).

`fe test` here runs real SDK proofs (`vectors/semaphore/examples/vote.json`). Gas of one vote:
about 266k. Groth16 proofs are malleable, so the proof is never used as an id; the nullifier is.
