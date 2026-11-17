# fe-zk design

## Fe capabilities

Findings from probes under `scripts/probe/`, run with `fe 26.5.0 (3dab95f)`.
Each probe runs with `fe test <file>`. The `encap` probe is a separate ingot run with
`fe test` inside `scripts/probe/encap`, and it is expected to fail to compile.

| Feature | Works? | Probe | Notes |
|---|---|---|---|
| Trait with associated `const` (`const MODULUS: u256`) | yes | `field_trait.fe` | `F::MODULUS` usable in generic code |
| Const generics over `u256` (`struct Fp<const P: u256>`) | yes | `field_trait.fe` | One impl serves KoalaBear and BabyBear |
| Generic functions over a trait bound (`fn f<F: Field + Copy>`) | yes | `field_trait.fe` | |
| Generic array length (`fn sum<F, const N: usize>(xs: [F; N])`) | yes | `field_trait.fe` | `for x in xs` works |
| `addmod` / `mulmod` | yes | `field_trait.fe` | `std::evm::crypto::{addmod, mulmod}`, also `const fn` |
| Precompile call with success and returndata length check | yes | `pairing.fe` | `Call.raw_staticcall` returns `RawCallOutcome { success, returndata_len }` |
| Message with a fixed array argument (`[u256; 12]`) | yes | `pairing.fe` | ABI `uint256[12]` |
| Gas measurement in tests | yes | `pairing.fe` | `evm.gas()` before and after `evm.call`, reported via an event and `fe test --show-logs` |
| Private struct fields across modules | enforced | `encap/` | `Fr { v: 100 }` outside the defining module fails: `` `v` is not visible `` |

### Notes on the standard library

`std::evm::crypto` already wraps the BN254 precompiles (`ec_add`, `ec_mul`, `ec_pairing`).
fe-zk does not use these wrappers in verifiers, for three reasons:

- `ec_pairing` returns `word != 0`. Safety rule 6 requires the result to be exactly `1`.
- The fixed-output wrappers do not check the returndata length. The module comment says so on purpose:
  "intentionally do not check returndata length on the hot path".
- The wrappers take raw `u256` coordinates. They do not check that a coordinate is
  canonical or that a point is on the curve.

fe-zk calls the precompiles through `Call.raw_staticcall` instead. It checks the success flag, the
returndata length and the exact value. Verifiers declare `uses (call: Call)`: the
signature shows that verification can make static calls but cannot write storage or send value.

### Measurements

- Pairing check with 2 pairs via `raw_staticcall`, called through a contract message: 119 376 gas
  (precompile cost 113 000 plus call and ABI overhead).

### Local build

The prebuilt `fe` from `feup` stops with `Illegal instruction` on CPUs without AVX. Building the
same tag (`v26.5.0`) from source with `cargo build --release -p fe` works. The build needs the
`tree-sitter` CLI on `PATH` (`npm ci` in `crates/tree-sitter-fe`). CI uses the prebuilt
binary.
