# PLONK (snarkjs) verifier

`membership_verifier/` is emitted by `fe-zk-gen plonk` from the snarkjs PLONK key of the membership circuit
(3 public signals, domain 2^13). Do not edit it; regenerate it.

```sh
(cd gen && cargo run -q -p fe-zk-gen -- plonk \
    --snarkjs ../vectors/plonk/membership/verification_key.json \
    --proof ../vectors/plonk/membership/proof.json --public ../vectors/plonk/membership/public.json \
    --name PlonkMembership --out ../examples/plonk/membership_verifier --fe-zk-path ../../..)
cd examples/plonk/membership_verifier && fe test
```

The tests accept the real snarkjs proof and reject tampered, non-canonical and off-curve inputs. Recipe:
[docs/USAGE.md](../../docs/USAGE.md#c-snarkjs-plonk).
