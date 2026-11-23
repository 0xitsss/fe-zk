# gnark PLONK verifier

`mul_verifier/` is emitted by `fe-zk-gen gnark` from a gnark v0.12.0 PLONK key (2 public inputs, real ceremony SRS).
It exposes the ABI of gnark's Solidity verifier, `Verify(bytes,uint256[])`. Do not edit it; regenerate it.

```sh
(cd gen && cargo run -q -p fe-zk-gen -- gnark \
    --vk ../vectors/gnark/mul/fixture.json --proof ../vectors/gnark/mul/fixture.json --public ../vectors/gnark/mul/fixture.json \
    --name GnarkMul --out ../examples/gnark/mul_verifier --fe-zk-path ../../..)
cd examples/gnark/mul_verifier && fe test
```

Recipe: [docs/USAGE.md](../../docs/USAGE.md#d-gnark-plonk).
