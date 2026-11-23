# Differential testing: Fe verifiers vs reference verifiers

Claim under test: for every verifier family, the Fe verifier gives the **same accept/reject verdict** as the
reference verifier on the same calldata, for valid proofs and for tens of thousands of adversarial inputs.
The only allowed exceptions are listed (with reasons) in the allowlist at the end; everything else is a bug.

Run: `scripts/difftest.sh --quick` (committed fixtures, < 3 min, runs in CI) or `scripts/difftest.sh` (full).
Code: `vectors/difftest/` (`difftest.py` driver, `prove.cjs` snarkjs prover, `src/evm_batch.rs`, `src/ark_ref.rs`).

## Method

* **Same EVM, side by side.** `evm-batch` (revm 33.1.0, Osaka rules, as `vectors/gas`) deploys the Fe verifier and
  the reference verifier into one in-memory chain and sends them byte-identical argument words (each side with its own
  ABI wrapper where the reference ABI differs). Verdict: *accept* = the call succeeded and returned `true` (or returned
  nothing, as the gnark and snark-verifier contracts do); *reject* = returned `false` or reverted or halted.
* **References.**

  | family | Fe side | reference |
  |---|---|---|
  | Groth16 snarkjs (multiplier, multiplier2, membership) | `fe-zk-gen groth16` output | snarkjs 0.7.6 `Groth16Verifier` (solc 0.8.24, optimizer 200) |
  | Groth16 arkworks | `fe-zk-gen groth16 --arkworks` output | native ark-groth16 0.5.0 (`ark-ref verify`), see "Native reference" |
  | PLONK snarkjs (multiplier2, membership) | `fe-zk-gen plonk` output | snarkjs 0.7.6 `PlonkVerifier` (solc 0.8.24, optimizer 200) |
  | PLONK gnark (mul, mulcommit) | `vectors/gas-fe` wrapper | gnark v0.12.0 `PlonkVerifier` |
  | halo2-KZG SHPLONK and GWC | `vectors/gas-fe` wrapper | snark-verifier 0.2.7 Solidity (solc, **no optimizer**, see GAS.md) |

* **Valid inputs.** Full mode runs a fresh circom compile and snarkjs setup per circuit (Groth16 setup + one
  contribution with a fixed public entropy string, PLONK setup), regenerates the Fe verifier from that fresh key with
  `fe-zk-gen`, and proves random witnesses (seeded; multiplier inputs of 8, 64, 128 and 253 bits; membership: random
  secret, leaf index and external nullifier in a depth-10 Poseidon tree). The arkworks family uses seeded ark-groth16
  proofs of random `(a, b)`. Groth16 families additionally get **re-randomised** variants of valid proofs
  (`A' = r*A`, `B' = r^-1*B`, `C` unchanged: valid, so both sides must accept: malleability is preserved, not
  accidentally removed). gnark and halo2 use the committed fixtures (see Limitations). Every valid input must be
  accepted by both sides; a valid proof rejected by Fe would be a COMPLETENESS bug.
* **Mutations** (seeded RNG, seed printed; 1 to 4 valid proofs per family are used as bases, `--random` extra random
  cases per family). Per calldata word: `+1`, `-1`, `0`, `1`, `q`, `r`, `r-1`, `q-1`, `q+1`, `r+1`, `2^256-1`, `+r`,
  `+q`, `2^255`. Per G1 point: negate `y`, infinity `(0,0)`, the generator `(1,2)`, swap `x`/`y`. Per G2 point: negate,
  infinity, swap real/imaginary parts of `x` or `y`. Plus: swap two points, swap two public inputs, copy one word over
  another, swap two words, 1 to 3 random words replaced by random values (full 256 bit, `< q`, `< r`, 64 bit, boundary
  constants), and calldata length changes (drop 32 bytes / 1 byte, append 32 zero bytes / 1 byte / 32 `0xff` bytes,
  empty calldata, body repeated twice). Cases identical to the valid input or to an earlier case are dropped.
* **Classification.** Both accept or both reject: *agree*. Otherwise the case must match an entry of the allowlist in
  `vectors/difftest/difftest.py` (`ALLOW`: family, mutation kind, verdicts, reason), else it is a BUG:
  SOUNDNESS if Fe accepts and the reference rejects, COMPLETENESS if Fe rejects and the reference accepts. The script
  exits 1 on any BUG and prints the case.

### Native reference (arkworks)

There is no Solidity verifier for the arkworks key, so the reference is ark-groth16 itself, behind a *strict* decoder
written in `ark_ref.rs`: every coordinate `< q`, every input `< r`, G1 on curve, G2 on curve **and in the subgroup**,
infinity only as `(0,0)`, at least the expected number of words (trailing calldata ignored, as ABI decoding does);
then `Groth16::verify_with_processed_vk`. Strict by construction, so any Fe accept that it rejects is a real soundness
finding, and any Fe reject of a proof it accepts is a completeness finding.

## Result of the full run

Command: `PTAU=/data/fe-zk-cache/ppot_0080_17.ptau CIRCOM=... scripts/difftest.sh --json out.json` (PSE perpetual
powers of tau 2^17, test-only keys). Seed `20261124`. Fe 26.5.0 (`-O 1`), solc 0.8.24, revm 33.1.0, snarkjs 0.7.6,
circom 2.2.2. Machine: 16 cores. Runtime **595 s** (two earlier runs with the same seed gave identical counts) of which ~367 s preparation (circom, snarkjs setups and 300 proofs),
~190 s building the one combined Fe ingot (`fe build`, single-threaded), ~37 s running all cases (the EVM part is small).

| family | valid tested (fresh) | mutations | agree | allowed diffs | bugs |
|---|---|---|---|---|---|
| groth16/multiplier-fresh | 70 (60) | 2211 | 2281 | 0 | 0 |
| groth16/multiplier2-fresh | 70 (60) | 2289 | 2359 | 0 | 0 |
| groth16/membership-fresh | 30 (20) | 2386 | 2416 | 0 | 0 |
| groth16/multiplier-fixture | 11 (0) | 1597 | 1608 | 0 | 0 |
| groth16/arkworks-vs-ark-groth16 | 70 (60) | 2306 | 2376 | 0 | 0 |
| plonk-snarkjs/multiplier2-fresh | 60 (60) | 3497 | 3557 | 0 | 0 |
| plonk-snarkjs/membership-fresh | 20 (20) | 3566 | 3586 | 0 | 0 |
| plonk-snarkjs/multiplier2-fixture | 1 (0) | 1943 | 1944 | 0 | 0 |
| plonk-gnark/mul | 1 (0) | 1959 | 1960 | 0 | 0 |
| plonk-gnark/mulcommit | 1 (0) | 2015 | 2016 | 0 | 0 |
| halo2-shplonk/standard_plonk_lookup | 1 (0) | 2472 | 2445 | 28 | 0 |
| halo2-gwc/standard_plonk_lookup | 1 (0) | 2535 | 2508 | 28 | 0 |
| **total** | **336** | **28776** | **29056** | **56** | **0** |

("valid tested" of the Groth16 families includes 10 re-randomised variants; "agree" counts valid and mutated cases.)

**No BUG.** In particular no case where Fe accepts something the reference rejects, and no valid or re-randomised
proof rejected by Fe. Mutated inputs accepted by *both* sides: only the calldata length cases that append bytes
(`len/pad1`, `len/pad32-zero`, `len/pad32-ff`, `len/dup-body`): Solidity and Fe ABI decoders ignore trailing calldata
and the proof statement does not change, so this is the same behaviour on both sides. Every point, scalar, input and
word mutation listed above was rejected by both sides, except the allowlisted ones.

## Allowlisted differences

| family | mutation | Fe | reference | reason |
|---|---|---|---|---|
| halo2-shplonk, halo2-gwc | `word/*/+r` at the scalar positions (instance word 0 and the 27 proof evaluation words, 28 cases per family) | reject | accept | Fe is stricter by design (SAFETY rule 1: never reduce silently). The word is a scalar shifted by `r`, the same field element non-canonically encoded. The Solidity verifier generated by snark-verifier reduces it mod `r` and accepts; Fe rejects non-canonical scalars. Same class as the snarkjs-reduces-public-inputs note in the README. No acceptance of an invalid statement: the reduced value is the original proven one. |

The snarkjs Groth16 and PLONK Solidity verifiers also reject non-canonical inputs (they check `< r`), so there is no
such entry for those families.

## Limitations

* gnark and halo2 have one valid proof each (the committed fixtures): a fresh gnark proof needs a fresh KZG setup (the
  test SRS in `vectors/gnark` samples a random toxic value, so a new vk and a new generated Fe wrapper) and the Fe halo2
  verifier is a fixed-circuit verifier built from `vectors/halo2`. Their mutation sets are as large as the others.
* Mutations are structured and random, not coverage-guided. Verdict equality on these sets is evidence, not a proof.
* The snark-verifier Solidity was compiled without optimizer (the contract exceeds the size limit otherwise); verdicts do
  not depend on it.
* Fresh setups use a public ptau and fixed public entropy: insecure by design, for testing only.
* The quick mode in CI uses the committed fixtures only (no circom, no snarkjs) plus 12 fresh seeded arkworks proofs and
  ~230-380 mutations per family (2870 in total), ~155 s on 16 cores, dominated by the single `fe build`. CI downloads the static `solc` 0.8.24 release binary from the Solidity GitHub release page
  (pinned by version tag, hash not pinned).

## Toolchain and regenerating fixtures

CPUs without AVX: the prebuilt binary stops with `Illegal instruction`. Build the tag from source with
`cargo build --release -p fe`; the build needs the `tree-sitter` CLI on `PATH` (`npm ci` in
`crates/tree-sitter-fe`). The result is the same version (`docs/DESIGN.md`, "Local build"). CI uses the prebuilt binary.

Other tools (only needed to regenerate vectors, not to run tests):

| Tool | Version | Pinned in |
|---|---|---|
| circom | 2.2.2 (commit e410b0d5) | `scripts/setup-tools.sh` |
| snarkjs | 0.7.6 | `vectors/js/package-lock.json` |
| circomlib | 2.0.5 | `vectors/js/package-lock.json` |
| circomlibjs | 0.1.7 | `vectors/js/package-lock.json` |
| ethers | 5.8.0 (transcript vectors) | `vectors/js/package-lock.json` |
| gnark | v0.12.0 (gnark-crypto v0.15.0) | `vectors/gnark/go.mod` |
| halo2-axiom | 0.5.3 (snark-verifier 0.2.7) | `vectors/halo2/Cargo.lock` |
| Plonky3 | 0.8.0 (STARK fixture), 0.9.0-rc.2 (Poseidon2 KoalaBear vectors) | `vectors/plonky3-stark/Cargo.lock`, `vectors/plonky3/Cargo.lock` |
| HorizenLabs/poseidon2 | commit 055bde3f4782731ba5f5ce5888a440a94327eaf3 | `scripts/regen-vectors.sh` |
| arkworks | 0.5.0 | `gen/Cargo.lock`, `vectors/kzg/Cargo.lock` |
| revm | 33.1.0 (Life CLI, gas runners); 27.1.0 inside the halo2 vector crate | `Cargo.lock` files |
| solc | 0.8.24 (reference Solidity gas baselines only) | `docs/GAS.md` |
| py_ecc | 8.0.0 (EC vectors) | `docs/AI_LOG.md` |

Regenerate: `scripts/setup-tools.sh` (circom, npm packages), `scripts/regen-vectors.sh` (hash vectors),
`scripts/regen-groth16-fixtures.sh`, `scripts/regen-plonk-fixtures.sh`, `scripts/regen-gnark-fixtures.sh` (Go),
`scripts/regen-halo2.sh` (needs `solc`; real PSE ptau SRS), `scripts/regen-kzg.sh` + `scripts/gen-kzg-fe-fixtures.py`,
`scripts/regen-real-semaphore.sh` (real Semaphore v4 key and ceremony zkey), `scripts/regen-plonky3-stark.sh`,
`scripts/life-prove.sh <pattern> <N>` (about 30 min and 1.2 GB of ptau for N = 512). Fixtures from circom/snarkjs are
randomised, so a regeneration changes every value.
