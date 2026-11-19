# anon_signal

Anonymous signals (Semaphore's broadcast pattern): any member of an open group publishes a 256-bit
value (text hash, feedback code) once per topic without revealing which member sent it.

* `join(commitment)` is open to anyone (demo; gate it for a real group). `signal(topic, text_hash,
  depth, root, nullifier, points)` emits `Signal(topic, text_hash, nullifier)`.
* scope = topic, message = text hash. Once per member per topic; the same member can signal on other
  topics, unlinkably. Swapping the text or the topic of a copied proof fails.
* Group limit: 32 members (`verify_proof_low`).

`fe test` runs real SDK proofs (`vectors/semaphore/examples/signal.json`). Gas of one signal: about 242k.
