# FRI/STARK verifier contract

`FriFibVerifier` checks a Plonky3 0.8.0 `p3-uni-stark` proof of the Fibonacci AIR (2^10 rows, public values
`(a, b, x)`) with the secure configuration: KoalaBear, degree-4 extension, `log_blowup` 4, 20 queries, 20 bits of
query proof of work, 100 conjectured bits.

- Call: `verify(uint256[],uint256[3])`, selector `0xc969d25e`: the proof as flat words, then the 3 public values.
- Pure: reads no storage.
- Gas and code size: [docs/GAS.md](../../docs/GAS.md), measured in revm by `scripts/gas-table.sh`.

```sh
fe build                 # from this directory
cd ../.. && fe test -f secure_   # the tests of this configuration in the root ingot
```

Your own configuration or AIR: [docs/USAGE.md](../../docs/USAGE.md#g-fristark-plonky3) and
[(h) Your own STARK AIR](../../docs/USAGE.md#h-your-own-stark-air).
