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

Gas (revm, Osaka, execution gas without the 21 000 intrinsic; `docs/GAS.md`): `step` = 657; `run(board, 64)` = 34 246;
`run(board, 512)` = 271 686. On-chain stepping costs about 530 gas per generation, so a Groth16
proof of N generations (one pairing check with 4 pairs, about 200k gas) is cheaper than one `run` call only for
N above roughly 385 generations (about 10 against one `step()` transaction per generation), and the verifier's cost is
constant in N.

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

### Compiler issue found

`scripts/probe/issue_block_after_ident.fe`: when a line ends with an identifier and the next line starts a block
statement `{ ... }`, the parser reads `ident { ... }` as a struct literal and reports "`a` is not found". A literal
before the block (`var s: u256 = 1`) compiles. Workaround: `var s: u256 = (a)`. To be reported upstream.

### Compile-time key validation (CTFE)

`scripts/probe/ctfe_validation.fe`: a `const fn` with a 254-iteration `while` loop over `mulmod` runs inside
`static_assert(...)`; an invalid point fails compilation with `error[8-0081]: static assertion failed`, and the
resulting constant costs 2 gas at runtime. `assert`, `Option::is_some` and `Option::unwrap` are not allowed in a
const context, so validity functions return `bool` and constant constructors map invalid input to the point at
infinity (never to an unchecked point). Generated verifiers use this to validate their verifying key in the Fe
compiler, on top of the generator's own arkworks validation.

## halo2-KZG (`src/verifier/halo2/`)

A circuit-independent core (`common.fe`: EvmTranscript, point/scalar reading with validation, batched inversion via
modexp, final pairing) plus a per-circuit module emitted by `fe-zk-gen halo2` (Rust; `vectors/halo2/emit_fe.py` is the byte-identical cross-check) from `vk.json`
(gate, lookup and permutation expressions, SHPLONK rotation sets and GWC point groups unrolled; the proof commitments and
evaluations are validated by two loops, `read_points` and `read_scalars`, and absorbed by `absorb_words`).

**Code size (EIP-170).** The first generator unrolled every `match read_g1(..)`/`from_canonical(..)` (about 40 copies of ~15 IR
instructions) and put both schemes in one contract: 43,102 bytes. Loops for the reads: 31,251; one contract per scheme
(`--scheme`): SHPLONK 22,918, GWC 21,162 (23,136 / 21,598 after the cheaper-`is_infinity` change below, whose purpose is
the KZG wrappers). Trade-off: execution gas rose about 1.9% (SHPLONK 384.7k to 391.9k) from loop overhead. `fe build -O 2`
shrinks only one scheme noticeably (21.7k for SHPLONK alone) and does nothing for the two-scheme contract (42.6k), so the
split is what makes it deployable. The size regression of the KZG wrapper (1,647 to 2,283 bytes) came from
`kzg::Vk::is_degenerate`: `G2::is_infinity` was a short-circuit `&&` chain (branches per word); `(x0|x1|y0|y1) == 0` restores 1,647
bytes with the check intact. Gas (`docs/GAS.md`, execution, same proof in revm): SHPLONK 376.0k,
GWC 408.7k; the snark-verifier Solidity verifier costs 322.6k / 349.0k, so Fe is about 1.17x (wrapper overhead per
precompile call). The transcript is one contiguous `MemBuffer` (an absorb is one `MSTORE`, a squeeze one `KECCAK256`):
the first version used `std::evm::packed::Packed`, whose byte-wise `u256` append cost about 250 gas per absorbed word
(about 18k of the 397k) and about 1.1 kB of code. The Fe numbers use the compile-time key path (`halo2::Vk::from_consts`). Limits: one circuit shape
(single phase, one instance column, uncommitted instances); duplicate commitments are rejected by the generator.

## Groth16 and the generator

- `G1`, `G2`, `G2Proof` hold canonical `u256` words. Validity is `const fn` (`g1_is_valid`, `g2_is_valid` with the
  psi subgroup test, `from_coords_or_infinity`), shared by the runtime constructors and by compile-time constants.
- `fe-zk-gen groth16` emits an ingot whose key is checked twice: by ark-bn254 in the generator, and by the Fe compiler
  through `static_assert(g2_is_valid(..))` for every point plus `static_assert(!VK_X.is_infinity())` for alpha, beta,
  gamma, delta. An invalid key does not compile; a valid key costs no runtime gas. CTFE of three G2 subgroup checks
  adds about 20 s of compile time.
- The generated contract has the snarkjs `Verifier.sol` selector `verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[N])`
  and calldata layout (B as `[[x_imag, x_real], [y_imag, y_real]]`), and forwards to `groth16::verify_calldata`; all checks
  live in the library, so the generator cannot emit a verifier that skips one.
- Failure policy: invalid input returns `false`; a precompile violating its output contract reverts, and so does a
  call with too little gas (the failure rule, "Verifier API" below).
- Gas per library `verify`, measured in-test with `evm.gas()` around the call (a different measurement from the revm
  table in `docs/GAS.md`, which runs the deployed generated contract: N=1 191 869, N=2 200 045, N=3 207 964, Solidity
  1.02x to 1.03x lower): N=1 190 982, N=2 198 851, N=3 206 158; generated membership contract call 211 433.
- Fe findings: a dependency that reaches `unsafe` needs `allow_unsafe = true` in the dependent's `[dependencies]`
  entry; absolute dependency paths are rejected ("invalid path"); `assert` and `Option` methods are not const.

### Precompile wrapper gas

Wrappers use fixed-size uninitialised buffers and `pairing_check` writes G2 coordinates straight into the pairing
input; every rule 6 check stays. Savings are 1–3% per verifier. The floor is the STATICCALL (100 warm + precompile
price) plus about 190 gas of buffer allocation: std only exposes `Call.raw_staticcall`, which takes a heap
`MemBuffer`; converting a static `FixedMemBuffer` with `into_buffer()` makes it escape to the heap. A public
`raw_staticcall` taking a pointer and length (or a `FixedMemBuffer`) would save about 200 gas per precompile call —
worth an upstream issue. The same wrapper measures 474–740 gas depending on surrounding code (inlining/arena
decisions).

## gnark PLONK (`src/verifier/gnark.fe`)

`gnark::Vk<NP, NC>` fixes the public-input count and the BSB22 commitment count in the type. The sha256 Fiat–Shamir
chain (gamma, beta, alpha, zeta, each on the previous raw digest) runs on precompile 0x02 with exact-length checks.
Gas (`docs/GAS.md`, execution, revm): `mul` 333.6k vs gnark Solidity 261.9k (1.27x); `mulcommit` 379.3k vs 283.3k (1.34x).
The gnark verifier is the furthest above Solidity; its transcript runs on the sha256 precompile, so it pays the
per-call wrapper overhead (see "Precompile wrapper gas") on every challenge as well.

`fe-zk-gen gnark` (`gen/fe-zk-gen/src/gnark.rs`) emits a contract with gnark's own Solidity ABI,
`Verify(bytes,uint256[])`. Fe's `Bytes` and `Vec<u256>` message fields decode the dynamic arguments;
`gnark::verify_calldata` rejects any length other than the key's fixed `24 + 3 NC` proof words and `NP` inputs before
reading a word (safety rule 4: the dynamic ABI must not let a caller drop or append an input). The key is compile-time
constants checked by `static_assert` and `gnark::Vk::from_consts`. The gnark fixtures use the real PSE `ppot_0080_05`
SRS (`vectors/gnark/ptau.go` converts ptau to gnark's canonical and Lagrange SRS, checking the generators, the pairing
ratio of every consecutive power and that the Lagrange points sum to the generator).

## Setup provenance: real ceremony material where the protocol allows it

A universal SRS needs no per-circuit ceremony, so the halo2-KZG fixtures (`ppot_0080_05` converted to halo2
`ParamsKZG` by `vectors/halo2/src/ptau.rs`: monomial powers verbatim, Lagrange basis by halo2's own `g_to_lagrange`) and
the KZG fixtures (commit and quotient proof as MSMs over the ceremony points) use the PSE Perpetual Powers of Tau, as do
the snarkjs PLONK fixtures. The verifiers are unchanged by this: a verifier only sees `g`, `g2`, `s_g2` / `[tau]G2` as key
constants. Groth16 is the exception: phase 2 is circuit-specific, so our own demo circuits (single contributor, public
entropy) are still test keys, while `vectors/groth16/real_semaphore_d10` is a production key (Semaphore v4 ceremony)
that exercises the generator on keys it did not make. Detail, hashes and checks: `vectors/README.md`. The conversion
test (`ptau_params_consistent`) checks a common tau along the whole power chain and the Lagrange basis, because a
wrong Montgomery decode or ordering would otherwise only show up as failing pairings. One practical finding:
`halo2-axiom` 0.5.3 `ParamsKZG::from_parts` is an instance method (it needs an existing `ParamsKZG` as receiver), so
the converter builds a throw-away receiver and overwrites every field.

## Compile-time keys for every verifier family

Emitted key points are `pub const` words checked by `static_assert(g1_is_valid(..))` / `static_assert(g2_is_valid(..))`;
domains by `const fn` (`plonk::domain_is_valid`, `gnark::domain_is_valid`). Typed constants come from
`from_coords_or_infinity`, keys from `from_consts`. If a constant is invalid, `from_consts` poisons the key (`[x]_2` or the
SRS points become infinity) and every verify returns false, so no usable unchecked key exists even without the
assertions. PLONK key construction: 1.05M gas at runtime before, 673 gas now; halo2 `vk()`: 1.51M before, 1.3k now.

## zk Game of Life

`examples/life/circuit/life.circom`: 7 constraints per cell (4-bit decomposition of the neighbour sum, then
`p = b1 (1 - b2 - b3)`, `q = p b0`, `next = q + alive (p - q)`), 1792 per generation; public signals
`[startCommit, endCommit]`, each `Poseidon(lo128, hi128)` of the packed board; the start board is private.

| N | constraints | ptau | setup | prove | peak RSS |
|---|---|---|---|---|---|
| 64 | 115 424 | 2^17 | 105 s | 22 s | 2.0 GB |
| 512 | 918 240 | 2^20 | 1066 s | 90 s | 8.8 GB |
| 1024 | 1 835 744 | 2^21 | 1705 s | 144 s | 15.5 GB |

`advanceProven(endLo, endHi, proof)` recomputes both commitments with `poseidon_2`, calls the generated
`Life512Verifier` and stores the end board: 512 generations in one transaction. Gas (revm, execution): 512 `step()` calls
336 393; one `run(board, 512)` 271 686; one proof verification 207 758; `advanceProven` 263 675. Break-even near 392
generations. Limitations: the stored board is public (the demo shows succinctness, not privacy); N is fixed by the key;
the ptau contribution and phase-2 entropy are public test values.

## FRI/STARK verifier (`src/verifier/fri.fe`)

Verifies a Plonky3 0.8.0 `uni-stark` proof (KoalaBear, degree-4 extension, arity 2, Keccak Merkle trees and challenger)
of any AIR that implements the `Air` trait: `verify<A: Air>`. The verifier logic is one piece of code; a configuration is
the constants of a small module emitted by `fe-zk-gen plonky3` (Rust; `vectors/plonky3-stark/emit_fe.py` is the
byte-identical Python cross-check) from the proof's own `config.json`
(`fri/config_secure.fe`, `fri/config_demo.fe`, `fri/config_mul.fe`):
`FriParams { width, log_n, log_blowup, log_final, num_queries, commit_pow_bits, query_pow_bits, stark_pattern,
pcs_pattern, fri_pattern }` (the last three are the Keccak hashes of Plonky3's transcript interaction patterns, see
"Plonky3 0.8 port" below).

### Plonky3 0.8 port (from 0.4.2)

Diff of the 0.4.2 and 0.8.0 sources of `p3-uni-stark`, `p3-fri`, `p3-challenger`, `p3-merkle-tree`, `p3-commit`; what
changed for a verifier of this shape (no zk, no preprocessed trace, arity 2, cap height 0):

- **Transcript domain separators** (`p3_challenger::fs`): uni-stark, the FRI PCS and FRI each open a
  `VerifierState` that first absorbs a seed: `protocol_id (version 1, name, length) || pattern_hash || u32be(len) ||
  instance label || 0x80`, packed 3 bytes per field element after a length element. The instance label carries the
  shape (uni-stark: degree bits, width, preprocessed width, periodic columns, quotient chunks, next-row flags; PCS:
  commitments / matrices / points; FRI: `log_blowup`, `max_log_arity`, each round's arity). The pattern hash is Keccak-256
  of the rendered interaction pattern and depends on the counts (public values, opened widths, final polynomial length,
  queries, PoW bits); the Fe verifier rebuilds the whole seed from `FriParams` and takes only the three pattern hashes as
  configuration constants (the generator checks the seed against the elements Plonky3 itself absorbed). The old
  explicit observations of `degree_bits` and the preprocessed width are gone (they are now in the label).
- **Proof of work**: `check_witness` first squeezes one challenger byte (a flush when the output is empty), and a
  zero-bit site requires the witness to be zero (`check_canonical_pow_witnesses`; also new: batch PoW and out-of-domain
  PoW sites, both 0 bits here, their witnesses must be 0).
- **Merkle openings**: per-query authentication paths are replaced by one pruned multi-opening per commitment
  (`PrunedMerklePaths`: the siblings the verifier cannot compute, in `walk_frontier` order) for the input batches and
  for every FRI round (`CommitPhaseMultiStep`, `BatchMultiOpening`); duplicate query rows must open identically.
  Commitments are `MerkleCap`s (cap height 0 here: one root). The tree, leaf hash and compression are unchanged.
  `fri/merkle.fe` `verify_pruned` implements the walk (sort and deduplicate the query rows, then per level compress
  known pairs or take the next supplied sibling).
- **Proof format**: `Proof` gains `ood_pow_witness`, `Commitments.random`, an optional `trace_next`; `FriProof` gains
  `batch_pow_witness`; `FriParameters` gains `max_log_arity` (higher-arity folding; we use 1) and
  `batch_proof_of_work_bits`. Folding, the reduced openings (alpha powers per height), the final-polynomial check and
  `SerializingChallenger32` sampling are unchanged.

The proof is shorter (secure configuration: 2,721 words with per-query paths, 1,844 pruned) but verification costs more
in the test runner (Fibonacci secure: 1,366,516 to 1,782,844 gas; MulAir 1,294,137 to 1,564,584; demo 258,320 to
387,106): three seeds (about 33 k gas each), the sort of the query rows per multi-opening, and the leaf digests kept in
memory between the queries and the multi-openings, against fewer Keccak hashes. As a deployed contract (`docs/GAS.md`, revm): execution 1,407,935 to 1,812,924 gas,
transaction 2,864,620 to 2,520,732 (the EIP-7623 calldata floor falls with the shorter proof), runtime code 12,943 to
16,287 bytes. 0.9.0 was a release candidate and was
not used.

### Adding an AIR (`fri/air.fe`)

An AIR is a unit struct that implements `Air` (`fri/air_fib.fe`: Fibonacci, 2 columns, 3 public values; `fri/air_mul.fe`:
6 columns, 1 public value):

```fe
pub trait Air {
    const WIDTH: u256        // trace columns
    const NUM_PUBLIC: u256   // public values (verify rejects any other count)
    const MAX_DEGREE: u256   // constraint degree incl. selector, <= 2 (one quotient chunk)
    fn eval(_ ctx: AirCtx) -> E   // folded constraints at zeta, Plonky3 assertion order
}
```

`eval` reads `ctx.local[i]`, `ctx.next[i]` (columns at `zeta` and `zeta * g`) and `ctx.public_value(i)` and folds with
`acc = ctx.assert_all|assert_first|assert_last|assert_transition(acc, constraint)`, which is
`acc * alpha + selector * constraint`, the same as the Rust `AirBuilder` calls in the same order. Steps: (1) write the
Rust AIR in `vectors/plonky3-stark/src/main.rs` and prove it (`--air`); (2) write the Fe `impl Air`; (3) run
`fe-zk-gen plonky3 ... --air <name>` (add the module and type to `air_info` in `gen/fe-zk-gen/src/plonky3.rs` and to the `AIRS` table of `emit_fe.py`, its byte-identical twin); (4) run `fri_suite` on the fixture (copy
`src/tests/fri_mul.fe`). Constraints are the only AIR-specific code: width enters the proof layout, the transcript,
the Merkle leaf hash and the opening sums through `FriParams.width`, and `verify<A>` rejects a configuration whose width is
not `A::WIDTH`. Fe monomorphises `verify<A>`, so `A::WIDTH` and `A::eval` are compile-time and there is no dispatch:
the secure Fibonacci verification costs 1,366,516 gas in the test runner after the generalisation, 1,349,487 before
(+1.3 %, the loops over columns that replaced unrolled width-2 code, not the trait). Not supported: constraint degree
above 2 (several quotient chunks; `verify` rejects `MAX_DEGREE > 2`), preprocessed or permutation traces, zero-knowledge
mode, other fields (the base-field arithmetic of the hot paths is KoalaBear-specific).
The proof is one flat `MemArray<u256>` whose layout (offsets are functions of the parameters) is documented at the top
of `fri.fe`. Queries, fold rounds and Merkle levels are loops, so the code size does not depend on the configuration.
`params_valid` bounds every shift and table index; `verify` rejects a proof whose length is not `proof_words(params, proof)`
(the fixed part plus the Merkle sibling counts the proof declares).
The secure module carries `static_assert(CONJECTURED_BITS >= 100)`.

### Security parameters

Plonky3 (0.4.2 and 0.8.0 alike) states `FriParameters::conjectured_soundness_bits = log_blowup * num_queries + query_proof_of_work_bits`
(the ethSTARK conjecture: capacity-bound proximity gaps for Reed-Solomon codes). It says itself that proven soundness
"isn't currently supported by this crate". We therefore also compute our own estimates (`Cfg::soundness` in
`vectors/plonky3-stark/src/main.rs`, written to `config.json`): per-query error `(1 + rho)/2` (unique decoding) or
`(1 + 1/(2m)) sqrt(rho)` (Johnson bound, `m = 10`), `rho = 2^-log_blowup`, plus the grinding bits, combined with the
BCIKS'20 commit-phase error `(m + 1/2)^7 / (3 rho^1.5) * n^2 / |F|` with `|F| = 2^124`, `n` the largest committed
domain. These are our reading of the literature, not an audit; the proven figures sit far below the conjectured one.

Trade-off, Fibonacci AIR, 2^10 rows, `log_final = 3`, 20 PoW bits (execution gas of `verify` in the Fe test
framework, measured before the optimisation below, so the absolute numbers are about 3x too high, the ratios between rows still hold; proof words are calldata words, 32 bytes each; the code size, 12,875 bytes for the deployed contract (18,009 before the optimisation), is the
same for all configurations because only constants differ; deployed contract: see `docs/GAS.md` for the current
execution gas of the secure configuration):

| `log_blowup` | queries | conjectured | proven (Johnson / unique) | proof words | Fe gas |
|---:|---:|---:|---:|---:|---:|
| 1 | 80 | 100 | 54 / 53 | 8,481 | 15,681,656 |
| 2 | 40 | 100 | 57 / 47 | 4,641 | 8,082,989 |
| 3 | 27 | 101 | 59 / 42 | 3,402 | 5,625,016 |
| **4** | **20** | **100** | **59 / 38** | **2,721** | **4,293,638** |
| 5 | 17 | 105 | 61 / 36 | 2,478 | 3,748,713 |

Other axes at `log_blowup = 4`, 20 queries: `log_final = 1` instead of 3: 4,671,837 gas (3,081 words); 2^8 rows:
3,514,576 (1,977 words); 2^12 rows: 5,108,141 (3,545 words). Chosen: 2^10 rows, `log_blowup = 4`, 20 queries,
20 PoW bits: 100 conjectured bits, about 59 (Johnson) / 38 (unique decoding) proven, 4.29 M gas at the time of that measurement (about 1.38 M after the optimisation), 87 kB of calldata.
A higher blowup costs the prover (LDE of 2^14 points here, negligible) and buys fewer queries; the proven figure
barely moves, so the proven security of every row is about 55-60 bits and only more queries would raise it. The demo
configuration (4 queries, `log_blowup` 2, 3 PoW bits: 11 conjectured, about 6 proven bits) stays as a second test
configuration and is not secure.

### Gas profile and optimisation (2026-11-22)

The first version cost 4,674,110 gas (deployed contract, execution only, `scripts/gas-table.sh`); the optimised one costs
about 1.38 M with the same proof format and the same verdicts (the Plonky3 fixture verifies, every tamper,
non-canonical and transcript test still rejects). The profile came from a per-PC gas histogram of the deployed code
(a revm inspector, the throwaway `gas-run` variant is not committed) plus variants that return early at a chosen point.
Measured steps, each on top of the previous (execution gas of `verify(uint256[2721],uint256[3])`):

| Step | Gas |
|---|---:|
| baseline | 4,674,110 |
| `Ext4::mul` with lazy reduction (one `% p` per coefficient, bound in `mul_lazy`); KoalaBear inversion as a straight-line addition chain (`Fp::inv_koalabear`); one reduced `1/(zeta - x)(zeta*g - x)` inversion per query instead of two; one `1/y0` per query and `y_{j+1} = y_j^2` for the fold points instead of a `pow`, a bit reversal and an inversion per round (`s_j = (-1)^(idx_j & 1) y_j`, see the comment in `check_query`); the query-independent part of the reduced opening (`alpha^k`, the claimed evaluations) computed once in `replay_transcript` | 2,839,632 |
| proof copied with one `calldatacopy` instead of 2,721 `calldataload` + checked stores (`examples/fri_stark`) | 2,583,930 |
| `betas`, `indices`, `alpha_pows` in `MemArray`s: a by-value struct with inline arrays is copied at every call (`check_query`, `open_input`) | 2,459,567 |
| `climb_path`: the Merkle loop reads siblings through one pointer, unchecked counters | 2,367,941 |
| `core::ptr::offset` multiplies by the element size with an overflow check; `offset_bytes(ptr, i * 32)` after a single range check does not | 1,975,142 |
| fold rounds in raw `u256` lanes (`leaf8`: the 8 felts of a fold leaf are one word, one byte swap for all lanes; lane arithmetic with stated bounds; Horner of the final polynomial per lane) | 1,556,213 |
| `verify` range-checks only the query-independent words up front; each query range-checks the words it reads (same verdict) | 1,442,214 |
| unchecked reads inside `check_query`, y0 loop on pointers, `#[inline(always)]` on the small helpers | 1,384,751 |

The transaction gas in `docs/GAS.md` (2,864,620) is no longer exec + intrinsic: with 87 kB of calldata it is the EIP-7623
calldata floor (21,000 + 10 per calldata token), so only the execution column moved below it; a shorter proof (fewer queries,
higher blowup) is what lowers it.

Where the rest goes (opcode histogram): about 26 % `DUP`/`SWAP` (stack shuffling of many live scalars), 20 % `PUSH`
(function-call addresses), 18 % `JUMP`/`JUMPI`, 6 % `KECCAK256` itself; the Merkle path loops (about 1,960 Keccak
hashes per proof, 150 gas per level) are about 21 % of the total and cannot shrink below the hash cost. Per query:
about 61 k gas, of which the 7 fold rounds are about half, `open_input` about 13 k (one extension inversion about 3 k,
two Merkle paths about 4 k). Findings that may help others: loops cost 100-400 gas per iteration in Fe 26.5.0 at `-O1`
(`-O2` was within 1 % here), so straight-line code on scalar locals wins; indexing a `MemArray` or calling
`core::ptr::offset` costs about 100 gas per access because of the bounds and overflow checks; passing a struct with array
fields by value copies the arrays.

### Fe findings from this work

- A static array `[u256; N]` as a contract message field or local makes `fe build` time grow much faster than N
  (probe contract: N = 300 4 s, 600 15 s, 1200 over 3 min, 2721 over 4 min); the first FRI contract built that way
  took 2 min 15 s and was 106 kB. `MemArray<u256>` (runtime length, heap) compiles in under a second at N = 2721.
  The contract therefore reads calldata with `CallData::word_at` into a `MemArray`, and uses the raw selector
  `0x12852f9a` (`sol("...")` selectors need typed fields). Worth an upstream issue.
- Many 2.7k-word array copies in one function overflow the test frame ("StackOverflow"); heap `MemArray`
  copies do not. `var r = p` on a `MemArray` aliases (struct copy), tests use an explicit `clone_words`.
- `mut` array parameters cannot be assigned through in 26.5.0 ("immutable assignment"), so tamper helpers copy.

## Verifier API (v0.2.0)

One module per proof system: `groth16`, `plonk` (snarkjs), `gnark`, `halo2`, `kzg`, `fri`, re-exported from
`src/lib.fe` (`pub use verifier::{..}`), so `groth16::verify(..)` and `plonk::verify(..)` do not collide and the generated
contracts read the same for every system. Each module has `Vk` (with `from_coords`, an `Option` for runtime keys, and
`from_consts`, a `const fn` for compile-time keys), `verify` over typed arguments, and `verify_calldata` over raw words.
The old loose names are listed in `CHANGELOG.md`. `scripts/gen-api-docs.py` generates `docs/API.md` from the source and
CI checks it.

**Degenerate keys.** A key with `alpha`, `beta`, `gamma` or `delta` at infinity is refused by `from_coords` (`None`);
`from_consts` cannot return an `Option` (it is `const`), so it builds a poisoned key whose `verify` returns `false`
before any precompile call. The generator never emits such a key, and `static_assert(!VK_X.is_infinity())` stops a
hand-edited one.

**Failure rule.** An invalid proof returns `false`; too little gas reverts. A precompile that runs out of gas fails like
one that rejected its input, and `groth16::verify` maps a failed pairing call to `false` (so that a `B` outside the
subgroup is a clean `false`), so without a guard a caller could starve a call and read "rejected" for a valid proof.
`require_gas(cap)` (`src/ec/precompile.fe`) asserts `gas >= cap + GAS_MARGIN` (5,000) before every precompile call.
Fe offers the remaining gas only through the `Ctx` effect (`ctx.gas()`; the `gas` op is `pub(ingot)` and an `extern`
declaration fails codegen on the EVM target), so every function that reaches a precompile declares
`uses (call: Call, ctx: Ctx)`. Cost: 0.1 to 1.3 % execution gas (the guard plus the effect plumbing).
PLONK, gnark and halo2 use the strict `pairing_check`, which reverts on a failed call anyway; only the Groth16 tests
are sensitive to the guard (`src/tests/failure_rule.fe`).

## Anonymous kit (`src/anon/`, `semaphore/`)

Goal: a developer writes an anonymous vote, claim or signal in about 50 lines. The pieces:

- `LeanImt` is a port of zk-kit's `InternalLeanIMT._insert` (insert-only): one side node per level, an odd node is
  promoted unhashed, the depth grows with the size, Poseidon t = 3. Roots equal `@semaphore-protocol/group` 4.x
  (`vectors/semaphore/lean_imt.json`, checked in `src/tests/anon.fe`). Every historical root stays known. The depth is
  capped below 32, Semaphore's maximum. It is a storage struct (`StorageMap` fields), declared as a contract field.
- `Nullifiers` keys the spent set by `keccak(scope, nullifier)`, so a member is limited once per scope and entries of two
  scopes cannot alias. `hash_to_field` is `keccak(x) >> 8`, below `2^248 < r`, so no reduction occurs.
- The `semaphore/` ingot verifies Semaphore v4 proofs with the ceremony keys for depth 1..10. The keys are emitted by
  `fe-zk-gen groth16 --lib-module` (one `keys_dN.fe` each, `static_assert`ed), `src/lib.fe` dispatches on the depth, and
  `verify_proof_low` / `verify_proof_high` split the keys over two ranges. The split is a size decision: all ten keys are
  about 11 KB, a `LeanImt` about 14 KB, so one contract holds the tree plus only `verify_proof_low` (21 KB for the vote
  example); this is why the examples stop at 32 members. Beyond that, the verifier lives in a second contract
  (`docs/USAGE.md`, "Beyond 32 members").
- Compile cost: ten keys with compile-time subgroup checks take about 3 minutes to compile.
- The proof generator is the official JS SDK; the Fe side never sees secrets. Cross-check: every committed proof is also
  accepted by Semaphore's unmodified `SemaphoreVerifier.sol` in revm (`scripts/check-semaphore-sol.sh`).

## Tornado Cash verifier (`examples/tornado`)

`fe-zk-gen groth16` applied to the `withdraw` verifying key of Tornado Cash classic (6 public inputs). The key numbers
come from the deployed verifier contract (all 28 coordinates found in its bytecode); the real proofs are calldata of
five mainnet withdrawals of the 0.1 ETH pool. Verdicts equal Tornado's `Verifier.sol` on 236 cases (5 valid, 231
mutations) in revm (`vectors/tornado/crosscheck.py`). Gas: 231,117 in Fe against 236,365 for `Verifier.sol` (solc 0.7.6).
A verifier demo only: no pool, no deposits, no funds. It shows the generator working on a production key that
nobody here made, and the public-input binding that stops a front-runner from swapping the recipient.

### Gas tuning log (2026-11-22)

Method: `gas-run` with a temporary inspector that split top-frame gas into precompile gas and "own" gas, plus a
per-program-counter profile (not committed; the numbers below are from `docs/GAS.md`). The own gas was the gap to the
Solidity verifiers. What was found and fixed:

- **Fermat inversion in verifiers.** `Field::inv` is square-and-multiply (about 25k gas for `r - 2`). The snarkjs and gnark
  PLONK verifiers and the SHPLONK KZG check used it for the batched Lagrange inversion; they now call
  `ec::precompile::inv_modexp` (modexp 0x05, about 1.4k gas, fixed buffers, success and length checked, result through
  `from_canonical`). Saves about 21k per verification (snarkjs PLONK 293.1k to 272.6k).
- **`1 / n` in the gnark key.** `gnark::Vk::from_consts` ran another Fermat inversion per call for `1 / 2^power`. It is
  closed form: `r - 1 = k 2^power` (2-adicity 28), so `1 / 2^power = r - k` (`inv_pow2`, a shift and a subtraction).
  gnark PLONK 333.6k to 287.3k (mul), 379.3k to 313.1k (mulcommit) together with the modexp change.
- **halo2 transcript.** `Packed` replaced by one `MemBuffer` (SHPLONK 396.6k to 376.0k, code 23.2 kB to 22.1 kB).
- **Tried, no gain:** `#[inline(always)]` on every `Fp` method (the backend already inlines them); `-O 2` (under 1%);
  `#[arithmetic(unchecked)]` in `pairing_raw` (no change in Groth16 gas, reverted).
- **What remains.** Per precompile call the wrapper costs about 300 gas (GAS-guard, struct and buffer allocation, the
  returndata length check, the `G1` result): 25k to 30k of the 82k "own" gas of halo2 and 20k of gnark. It cannot go
  lower without a std API taking a pointer and length (compiler issue 2 in the README).
