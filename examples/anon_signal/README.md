# anon_signal

Anonymous signals (Semaphore's broadcast pattern): any member of an open group publishes a 256-bit
value (text hash, feedback code) once per topic without revealing which member sent it.

* `join(commitment)` is open to anyone (demo; gate it for a real group). `signal(topic, text_hash,
  depth, root, nullifier, points)` emits `Signal(topic, text_hash, nullifier)`.
* scope = topic, message = text hash. Once per member per topic; the same member can signal on other
  topics, unlinkably. Swapping the text or the topic of a copied proof fails.
* Group limit: 32 members (`verify_proof_low`).

`fe test` runs real SDK proofs (`vectors/semaphore/examples/signal.json`). Gas of one signal: about 242k.

## Why 32 members, and how to go beyond

The cap is a code-size limit, not a cryptographic one. A `LeanImt` is about 14 KB and `verify_proof_low` (tree depth
1 to 5, the keys of groups up to 2^5 = 32) about 7 KB, which fits in one contract (EIP-170: 24,576 bytes); adding all ten
keys (`verify_proof`, about 11 KB) does not. For bigger groups (depth 6 to 10, up to 1,024 members) put the verifier in
its own contract using `verify_proof_high` and call it from the group contract with a static call. A tested example
(`HighVerifier` and a caller) is in [docs/USAGE.md](../../docs/USAGE.md), "Beyond 32 members". `register` stops at
`MAX_MEMBERS` here so that no member is registered who could never prove.
