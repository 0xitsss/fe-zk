# Fe compiler and std issues found

Found while building fe-zk with fe 26.5.0. Upstream PRs are listed in the [README](../README.md#compiler-issues-found--upstream-suggestions).

Filed upstream (from the account of this repository's author):
- [#2061](https://github.com/argotorg/fe/pull/2061) fixes the fixed-output precompile wrappers of `std::evm::crypto` (item 4: they returned `Ok` with zeroed output when the precompile is absent)
- [#2062](https://github.com/argotorg/fe/pull/2062) fixes item 1, [#2063](https://github.com/argotorg/fe/pull/2063) fixes item 9
- [#2068](https://github.com/argotorg/fe/pull/2068) adds item 2 (issue [#2064](https://github.com/argotorg/fe/issues/2064)), [#2069](https://github.com/argotorg/fe/pull/2069) fixes item 8 (issue [#2065](https://github.com/argotorg/fe/issues/2065))
- [#2070](https://github.com/argotorg/fe/pull/2070): `[u256; N - 1]` at `N = 0` became an invalid type that matched any value; now reported

Each PR has a regression test and passes the Fe test suite.

Found with fe 26.5.0 (3dab95f):

1. **Parse error: identifier at the end of a line, block on the next.** `var s: u256 = a` followed by a line `{ ... }`
   is parsed as a struct literal `a { ... }` and reports "`a` is not found". A literal before the block compiles.
   Workaround: `var s: u256 = (a)`. Repro: `scripts/probe/issue_block_after_ident.fe`.
2. **No public raw call with a static buffer.** `Call.raw_staticcall` takes a heap `MemBuffer`; converting a static
   `FixedMemBuffer` with `into_buffer()` makes it escape to the heap. In a small isolated call the optimizer removes
   the heap buffer at `-O1` (both paths 292 gas); the saving shows at `-O0` (742 vs 1085 gas) and when the buffer escapes.
3. **Loops are not unrolled and arrays in loops are expensive.** `for i in 0..N` with constant `N` is not unrolled;
   loops over mutable memory arrays cost 5-7x more than scalar locals; checked arithmetic on counters is a large share
   (`#[arithmetic(unchecked)]` is the lever). We generate unrolled scalar code for Poseidon as a workaround.
4. **`std::evm::crypto::ec_pairing` returns `word != 0` and does not check the returndata length.** The module comment
   says the fixed-output wrappers intentionally skip the length check. fe-zk does not use them for that reason.
5. **Prebuilt binary crashes with `Illegal instruction` on CPUs without AVX.**
6. **`allow_unsafe = true` is required in a dependent's `[dependencies]` entry** when a dependency reaches `unsafe`
   (our precompile wrappers set buffer lengths in `unsafe` blocks).
7. **Absolute dependency paths are rejected** ("invalid path"); only relative paths work.
8. **Const contexts reject `assert` and `Option` methods.** Validity functions must return `bool`; constant
   constructors map invalid input to infinity.
9. **`fe fmt` moves a trailing `//` comment inside an argument list.** Put comments above the call.
10. **Optimizer drops unused test results.** Gas measurements must emit the result in an event.
11. **Hidden per-call and per-access costs in hot code** (found while taking the FRI verifier from 4.67 M to 1.38 M gas, `docs/DESIGN.md`):
    a struct with inline array fields passed by value is copied at every call (use `MemArray` fields); `MemArray` indexing and
    `core::ptr::offset` carry a bounds check and a checked multiplication (about 100 gas per access; `offset_bytes(ptr, i * 32)`
    in `#[arithmetic(unchecked)]` after one range assert avoids it); loops cost 100-400 gas per iteration at `-O1`.

Works well (and was a pleasant surprise): associated consts in traits, const generics over `u256`, generics over
trait bounds, `const fn` evaluation of 254-iteration loops, private-field enforcement, `uses` effect checking.
