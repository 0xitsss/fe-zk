# Real Semaphore v4 key

`verifier/` is a Groth16 verifier emitted by `fe-zk-gen groth16` from the public Semaphore v4 ceremony key
(depth 10, 4 public inputs), tested with a proof made by the official SDK. For all depths 1..10 in one ingot,
see [`semaphore/`](../../semaphore).

```sh
(cd gen && cargo run -q -p fe-zk-gen -- groth16 \
    --snarkjs ../vectors/groth16/real_semaphore_d10/verification_key.json \
    --proof ../vectors/groth16/real_semaphore_d10/proof.json --public ../vectors/groth16/real_semaphore_d10/public.json \
    --name RealSemaphore --out ../examples/real_semaphore/verifier --fe-zk-path ../../..)
cd examples/real_semaphore/verifier && fe test
```
