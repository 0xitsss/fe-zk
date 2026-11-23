# halo2-KZG verifier

`circuit_verifier/` is emitted by `fe-zk-gen halo2` from a halo2-axiom circuit (k = 5, one lookup), with one
contract per multiopen scheme (SHPLONK and GWC). Do not edit it; regenerate it.

```sh
(cd gen && cargo run -q -p fe-zk-gen -- halo2 \
    --vk ../vectors/halo2/standard_plonk_lookup/vk.json \
    --shplonk ../vectors/halo2/standard_plonk_lookup/shplonk.json --gwc ../vectors/halo2/standard_plonk_lookup/gwc.json \
    --scheme both --name Halo2Circuit --out ../examples/halo2/circuit_verifier --fe-zk-path ../../..)
cd examples/halo2/circuit_verifier && fe test
```

Supported shape and limits: [docs/USAGE.md](../../docs/USAGE.md#e-halo2-kzg).
