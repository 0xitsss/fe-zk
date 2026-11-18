# fe-zk-gen

Rust CLI + library that reads a zk verifying key, **validates it**, fingerprints it and (later)
emits a safe Fe verifier. See CLAUDE.md sections 2 and 3 (rule 9).

Status: Groth16 (snarkjs, arkworks), PLONK (snarkjs) and halo2-KZG are implemented: parsing, validation,
digest, emission of a Fe ingot (contract plus tests with a real proof and an invalid battery) and the CLI. The
Groth16 sections below describe the shared model; `fe-zk-gen plonk --help` and `fe-zk-gen halo2 --help` list the
other options. `fe-zk-gen gnark` reads the key JSON exported by `vectors/gnark` (see "gnark" below).

## Usage

```
cd gen
cargo run -p fe-zk-gen -- groth16 --snarkjs ../vectors/groth16/multiplier/verification_key.json
cargo run -p fe-zk-gen -- groth16 --arkworks ../vectors/groth16/arkworks_multiplier/verification_key.json
cargo test
```

Exit codes: 0 valid, 1 key failed validation, 2 unreadable/unparseable input. There is no flag
to skip validation.

## Model

`Groth16Vk { n_public, alpha_g1: G1, beta_g2: G2, gamma_g2: G2, delta_g2: G2, ic: Vec<G1> }`.
Coordinates are raw `BigUint`s, never reduced. `G1` identity = `(0,0)`. `G2` stores each Fq2
element as `[c0, c1]` (`c0 + c1*u`, u^2 = -1), the snarkjs / arkworks order.

**EVM order:** the pairing precompile (0x08, EIP-197) wants a G2 point as
`x.c1, x.c0, y.c1, y.c0` (imaginary part first). `G2::evm_words()` produces that order; it is
pinned by tests against the EIP-197 generator and against snarkjs' own `Verifier.sol`.

## Validation (`Groth16Vk::validate`, done with ark-bn254 0.5.0)

1. `ic.len() == n_public + 1`
2. every coordinate canonical (`< q`), never reduced
3. alpha, beta, gamma, delta are not the identity (IC points may be, e.g. unconstrained input)
4. every point on its curve (G1: y^2 = x^3 + 3; G2: twist) and in the prime-order subgroup

## Input formats

* snarkjs `verification_key.json` (protocol groth16, curve bn128). G1 = `[x,y,"1"]` (infinity
  `["0","1","0"]`), G2 = `[[x0,x1],[y0,y1],["1","0"]]`, decimal strings.
* arkworks JSON written by `examples/arkworks_fixture.rs`
  (`cargo run -p fe-zk-gen --example arkworks_fixture -- <dir>`):
  `{"format":"arkworks-groth16-bn254-uncompressed-v1","n_public":N,"alpha_g1":hex,"beta_g2":hex,
  "gamma_g2":hex,"delta_g2":hex,"gamma_abc_g1":[hex,..]}`. Each hex string is ark-serialize
  `serialize_uncompressed` of the affine point: 32-byte little-endian coordinates (G1: x,y;
  G2: x.c0,x.c1,y.c0,y.c1); top two bits of the last byte are flags (0x40 infinity, 0x80
  y-negative, ignored for finite points). The fixture proof is `{"a","b","c"}` in the same
  encoding and `public.json` lists decimal public inputs.

## gnark (PLONK, gnark v0.12.0)

```
cargo run -p fe-zk-gen -- gnark --vk ../vectors/gnark/mul/fixture.json \
    --proof ../vectors/gnark/mul/fixture.json --public ../vectors/gnark/mul/fixture.json \
    --name GnarkMul --out ../examples/gnark/mul_verifier --fe-zk-path ../../..
```

`--vk` is the JSON written by `vectors/gnark` (`fixture.json`): `size`, `size_inv`, `generator`, `coset_shift`,
`nb_public`, `s` (3 points), `ql qr qm qo qk`, `qcp` (BSB22 selector commitments), `commit_constraint_ix`, `g1`, `g2`,
`g2_x`; points are `[x, y]` and G2 is `[x.real, x.imag, y.real, y.imag]`, all `0x` words. `--proof` reads
`proof_words` (gnark `MarshalSolidity()`), `--public` a JSON array or an object with `public`; both optional (without
them no tests are emitted). Validation (`GnarkVk::validate`, ark-bn254): `nb_public >= 1`, `size` a power of two
(`<= 2^28`), `size_inv * size = 1`, `generator` of order exactly `size`, `coset_shift` nonzero with `k^n != 1` and
`k^(2n) != 1`, NC (`qcp`) equal to the number of constraint indices, each `index < size` and `nb_public + index < size`,
every commitment canonical, on the curve, in the subgroup and not at infinity, `g1`/`g2` the BN254 generators,
`[x]_2` on the twist, in the subgroup, not the identity. Digest: `keccak256` of the tag `fe-zk/gnark-plonk/bn254/vk/v1`,
then 32-byte words `nb_public, power, NC, generator, coset_shift`, `Ql Qr Qm Qo Qk S1 S2 S3` and the `NC` `qcp` points
(x, y), the `NC` constraint indices, `[x]_2` (x.real, x.imag, y.real, y.imag).
The contract has the ABI of gnark's Solidity verifier, `Verify(bytes proof, uint256[] public_inputs) returns (bool)`
(selector checked against `solc --hashes` of `vectors/gnark/*/Verifier.sol` in `tests/gnark.rs`), plus `vkDigest()`. It
forwards to `gnark_plonk_verify_bytes`, which returns `false` unless the proof is exactly `24 + 3 NC` words and
`public_inputs` has exactly `nb_public` entries.

## Key digest

`digest = keccak256(preimage)`, preimage (all integers 32-byte big-endian words):

```
"fe-zk/groth16/bn254/vk/v1"          25 ASCII bytes, no terminator
n_public                             1 word
alpha_g1                             x, y                       2 words
beta_g2, gamma_g2, delta_g2          x.c0, x.c1, y.c0, y.c1     4 words each (JSON order, not EVM)
ic.len()                             1 word
ic[0..]                              x, y                       2 words each
```

Identity points encode as all-zero words. The digest is only defined for keys that pass
`validate()`; `key_digest` validates first. The format is source independent: the same key from
snarkjs or arkworks gives the same digest.
