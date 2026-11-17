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

More findings (from building `examples/life`):

- Path dependencies work: `[dependencies] fe_zk = { path = "../.." }` in `fe.toml`.
- `~x` (bitwise not) and `u256` shifts by a runtime `u256` amount work.
- Source files under `src/` are modules without a `mod` declaration.
- A message handler without a `uses` clause cannot touch storage: pure handlers are checked by the compiler.
- `fe build` needs at least one contract in the ingot; the root library ingot has none, so CI builds the example ingots.

## Game of Life (`examples/life`)

One 16×16 toroidal board is one `u256`. Bit `row * 16 + col`; each row is a 16-bit lane, col 0 is
the least significant bit of the lane. Horizontal neighbours come from a masked lane rotation,
vertical neighbours from a 16-bit rotation of the whole word. A bitwise adder network computes the
neighbour count for all 256 cells at once, and B3/S23 becomes one boolean expression:
`twos & ~big & (ones | board)`. A generation is about 60 word operations and no per-cell loop.

Gas (revm, Osaka, execution gas without the 21 000 intrinsic): `step` ≈ 624; `run(board, 64)` = 34 187;
`run(board, 100)` = 53 267. On-chain stepping costs about 530 gas per generation, so a Groth16
proof of N generations (one pairing check with 4 pairs, roughly 200–250k gas) is cheaper only for
N above roughly 450 generations, and the verifier's cost is constant in N.

## Curve layer (`src/ec/`)

- Point types have private fields; `G1::from_coords` / `G2::from_coords` are the only way in from raw words.
- All precompile calls go through `src/ec/precompile.fe`: success flag, exact returndata length, pairing word
  exactly 1 (any other word reverts), and an explicit gas cap per call. A failing precompile consumes all gas
  forwarded to it, so forwarding "all gas" would let one invalid point burn 63/64 of the caller's gas.
- G2 subgroup: the Fe check `psi(P) == [6x^2]P` costs about 755k gas. The pairing precompile of the test EVM
  (revm) also rejects an on-curve non-subgroup point (test `precompile_vs_non_subgroup`).
- The test EVM supports Prague: EIP-2537 (BLS12-381) and EIP-4844 point evaluation work in `fe test`.
- More Fe findings: `use super::x::*` globs work; const-generic functions over `[Struct; N]` work; `Evm`
  implements `Call`, so tests call `uses (call: Call)` functions directly; `fe fmt` moves trailing `//`
  comments inside argument lists (put comments above the call).

### Decision: G2 points from proofs (safety rule 3)

EIP-197 makes the subgroup check part of the pairing precompile's consensus rules:
"For `G_2`, in addition to that, the order of the element has to be checked to be equal to the group order"
and "If ... any of the inputs are not elements of the respective group ... the call fails."
(https://eips.ethereum.org/EIPS/eip-197). The test `precompile_vs_non_subgroup` shows the test EVM rejects an
on-curve point outside the subgroup.

So fe-zk uses two G2 types:

- `G2` — fully checked in Fe (canonical, on curve, `psi(P) == [6x^2]P`). Used for verifying-key constants
  (the generator also checks them in Rust with arkworks) and by anyone who needs a G2 point outside a pairing.
- `G2Proof` (proof elements such as Groth16 `B`) — canonical and on-curve checked in Fe (cheap); the subgroup
  check is completed by the pairing precompile, which is the only operation this type allows. A `G2Proof`
  cannot be turned into a `G2` or passed to anything except `pairing_check`, so the type system keeps the
  guarantee: an unchecked point can only reach the one place that checks it.

## Hash layer (`src/hash/`)

- **One Poseidon2 for every field:** `permute_generic<F: Field + Copy, const T: usize, const NE: usize, const NI: usize>`
  instantiated as BN254 t=3 (x^5) and KoalaBear t=16 (x^3, Plonky3 constants). The external layer is chosen by `T`
  (circ(2,1,..) for T < 4, Plonky3 M4 blocks for T % 4 == 0); the internal layer is `s_i = sum + diag_i * s_i`.
- Generic Merkle path: `merkle_root<H: Compression<D>, D: Copy, const N: usize>`, with `D = Bn254Fr` or `D = [KoalaBear; 8]`.
- Circomlib Poseidon uses the optimized sparse-matrix form (identical outputs to the plain form).
- Keccak transcript: every operation is one `keccak_words` over 32-byte words — `new`: `keccak(1, label)`,
  `absorb`: `keccak(2, state, x)`, `squeeze`: `h = keccak(3, state, label, counter)`, challenge `h mod r`.
- Fe findings: user traits with type parameters, marker types and turbofish calls work in generic code; `[F::zero(); T]`
  with indexed mutation works; constant tables of ~1300 `u256` compile in about 2 s; results not used in a test are
  removed by the optimizer, so gas measurements must emit the result in an event.

### Hash gas

The readable implementations stay in the library as references: `reference_poseidon_n` and the generic
`permute_generic<F: Field>` (the one-implementation-for-every-field highlight). Production paths are generated,
scalar-local, unrolled permutations on raw `u256` with lazy reduction, tested equal to the references:

| function | array / generic | fast path |
|---|---|---|
| `poseidon_1` (t=2) | 33.3k | 19.7k |
| `poseidon_2` (t=3) | 93.9k | 25.6k |
| `poseidon_3` (t=4) | 131k | 31.5k |
| `poseidon_4` (t=5) | 180k | 39.9k |
| `poseidon_5` (t=6) | 227k | 47.2k |
| Poseidon2 BN254 t=3 | 56.7k | 19.5k |
| Poseidon2 KoalaBear t=16 | 149.5k | 42.2k |

Bounds behind the lazy reduction: `R < 0.19 * 2^256`, so up to 5 values below R sum without overflow; KoalaBear lanes
stay below `36p < 2^37` before the S-box, and the cube stays below `2^111`. Each bound is restated in a comment next
to the code that relies on it (safety rule 7).

Fe optimizer findings (worth an upstream issue): loops over mutable memory arrays cost 5–7x more than scalar locals,
and `for i in 0..N` with a constant `N` is not unrolled; checked arithmetic on loop counters and indices is a large
share of loop cost (`#[arithmetic(unchecked)]` is the main lever); a block statement `{` directly after a `var`
statement fails to parse with "`let` conditions are only allowed in `if` and `while`".
