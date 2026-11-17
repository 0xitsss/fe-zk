# fe-zk — instructions for the agent working on this repository

You are building **fe-zk**, an entry for the **Fe Builders Challenge 2026**
(https://fe-lang.org/contest/). Read this whole file before writing code, and re-read the
"Safety rules" section before every verifier you write.

## 1. Hard facts about the contest

- Build window: Nov 3 – **Nov 24, 2026, 23:59 UTC** (final submission). Results ~Dec 1.
  Today is in the last week: plan for a working, tested core by **Nov 22**, polish and docs
  on Nov 23–24. Never be in a state where `main` does not build.
- Fe version: any release **≥ 26.5.0**. Install:
  `curl -fsSL https://raw.githubusercontent.com/argotorg/fe/master/feup/feup.sh | bash -s -- --version v26.5.0`
  Use one exact version for the whole project and record it (`fe --version`) in README.
- **The core technical contribution must be written in Fe.** Frontends, scripts and the
  generator CLI may be in other languages (we use Rust for `gen/`), but the verifiers,
  field/curve/hash code and examples that the jury evaluates must be Fe.
- Submission needs: public GitHub repo with an OSI license (Apache-2.0, already in
  `LICENSE`), full commit SHA, `fe.toml`, exact Fe version, reproducible build + tests,
  README covering problem/approach/architecture/limitations, a one-sentence pitch, **one
  Fe-specific highlight with a code excerpt**, a demo (reproducible CLI example is fine),
  and a declaration of pre-existing work, third-party code and AI assistance.
  `SUBMISSION.md` is the README template — the final README must follow it.
- Judging: technical quality 30 %, **Fe-specific value 25 %**, originality 20 %, usefulness
  15 %, docs/demo 10 %. Social reach does not count. Mainnet deployment does not count.
- AI assistance is allowed but must be disclosed; every line must be tested and
  explainable. Keep a running log in `docs/AI_LOG.md` (what was generated, how verified).

## 2. What we are building

**Pitch:** *fe-zk lets Fe contracts verify zero-knowledge proofs: a generic field, curve and
hash layer behind Fe traits, on-chain verifiers for Groth16, PLONK, halo2-KZG, raw KZG and
FRI/STARK proofs, and a generator that turns verifying keys into safe Fe verifier contracts.*

Today Fe has no proof verifier at all; every zk project must leave Fe for Solidity.
fe-zk closes that gap and makes Fe's type system (traits, generics, effects) the reason the
code is safer than the Solidity equivalents.

### Components (priority order — do them in this order, each fully tested before the next)

**P0 — must ship**
1. `src/field/` — a `Field` trait (add, sub, mul, neg, inv, pow, square, from/to canonical,
   `is_canonical`, constants `ZERO`, `ONE`, `MODULUS`) with implementations:
   - `Bn254Fr` (scalar field) and `Bn254Fq` (base field) over u256 with `mulmod`/`addmod`,
     inversion via `modexp` precompile (0x05) or Fermat;
   - `KoalaBear` (p = 2^31 − 2^24 + 1) and `BabyBear` (p = 2^31 − 2^27 + 1) and `Goldilocks`
     (p = 2^64 − 2^32 + 1) with plain modular arithmetic on small integers (no Montgomery needed
     on the EVM; document the choice);
   - generic code (hash, Merkle, transcript) written once against the trait.
2. `src/ec/` — BN254 G1/G2 types with on-curve and (for G2) subgroup checks, wrappers for the
   precompiles `ecAdd` (0x06), `ecMul` (0x07), `ecPairing` (0x08); a typed `pairing_check`
   over a list of (G1, G2) pairs. BLS12-381 wrappers for EIP-2537 (0x0b–0x11) if the target
   EVM/test runner supports them (Prague); otherwise a clearly marked stub + limitation.
3. `src/hash/` — Poseidon (circomlib-compatible, BN254, t = 2..6) and Poseidon2 (BN254 t = 3;
   KoalaBear t = 16, Plonky3 default constants), Keccak transcript helper. Test vectors from the
   reference implementations (circomlibjs / Plonky3 / HorizenLabs poseidon2) committed under
   `vectors/` with a script that regenerates them.
4. `src/verifier/groth16.fe` — Groth16 over BN254 (snarkjs / arkworks / gnark key formats via
   the generator). One pairing check `e(-A,B)·e(α,β)·e(vk_x,γ)·e(C,δ) = 1`.
5. `gen/` (Rust CLI `fe-zk-gen`) — reads a verifying key and emits a self-contained Fe
   verifier module + a Fe test that verifies a real proof:
   - `fe-zk-gen groth16 --snarkjs vk.json`, `--arkworks`, `--gnark`;
   - every emitted verifier includes **all** safety checks of section 3 by construction
     (the generator refuses to emit code that skips them; there is no flag to disable them).
6. `examples/membership/` — a small demo: an incremental Poseidon Merkle tree contract plus a
   Groth16 membership proof (circom circuit from Semaphore-style membership, or a trivial
   circuit compiled with circom/snarkjs). Reproducible from `scripts/demo.sh`.

7a. `examples/life/` — **zk Game of Life** (second demo, high priority for the jury): a 16×16
   (or 32×32) toroidal board packed bit-wise into `u256` words, one Fe contract that
   (a) steps the board on chain with word-parallel bit tricks (neighbour counts with shifts and
   bitwise adders, no per-cell loops) and reports gas per generation, and (b) accepts a Groth16
   proof that `N` generations (e.g. 64 or 256) were computed correctly off chain from a committed
   start board to a committed end board (Poseidon commitments), verified by a verifier emitted by
   `fe-zk-gen`. The circuit (circom) and a script that proves a run are under
   `examples/life/circuit/`; `scripts/life-demo.sh` runs both paths and prints the gas of N
   on-chain steps vs one proof verification. Pure on-chain stepping is P0-level simple and should
   come first; the zk path depends on the Groth16 verifier and generator.

**P1 — should ship**
7. `src/verifier/plonk.fe` — PLONK (snarkjs `plonk` and gnark PLONK) over BN254 with KZG,
   Keccak Fiat–Shamir exactly as the reference verifier.
8. `src/verifier/kzg.fe` — single and batched KZG opening checks (GWC and SHPLONK), plus the
   EIP-4844 point-evaluation precompile (0x0a) wrapper with its exact input layout.
9. `gen` support for **halo2-KZG** (PSE halo2 / halo2-axiom with SHPLONK and GWC): read the
   verifying key + circuit metadata the way `snark-verifier` does and emit a Fe verifier.
   Validate against a real halo2 proof generated by a tiny halo2 circuit in `gen/tests`.
10. Gas table: each verifier vs the Solidity verifier produced by the reference tool
    (snarkjs `zkey export solidityverifier`, gnark, snark-verifier) on the same proof.

**P2 — stretch (only if P0/P1 are done and green)**
11. `src/verifier/fri.fe` — a FRI / STARK verifier for small fields (KoalaBear/BabyBear,
    extension degree 4, Keccak or Poseidon2 Merkle commitments, arity 2) compatible with a
    Plonky3 `uni-stark` proof of a toy AIR (e.g. Fibonacci). Even verifying a small proof is a
    first for any EVM language. Gas will be high: report it honestly.
12. Recursion-friendly helpers (transcript challengers shared between Rust and Fe).

### Repository layout

```
fe.toml                 ingot "fe_zk"
src/lib.fe              re-exports
src/field/              Field trait + bn254, koalabear, babybear, goldilocks
src/ec/                 bn254 g1/g2, precompile wrappers, bls12_381 (EIP-2537)
src/hash/               poseidon, poseidon2, keccak transcript
src/verifier/           groth16, plonk, kzg, (fri)
src/tests/ or tests.fe  Fe tests (fe test)
gen/                    Rust workspace: fe-zk-gen CLI + vector tooling
examples/membership/    demo contract + generated verifier + proof fixtures
vectors/                committed test vectors (small JSON) + README how they were made
scripts/                demo.sh, regen-vectors.sh, gas-table.sh
docs/                   DESIGN.md, SAFETY.md, AI_LOG.md, GAS.md
```

## 3. Safety rules (non-negotiable — "safe generators" is the point of the project)

Every verifier, hand-written or generated, must:

1. **Reject non-canonical field elements**: every public input and every proof scalar must be
   `< r` (scalar field modulus); every coordinate `< q`. Never reduce silently.
2. **Check curve membership** of every G1 point from calldata (y² = x³ + 3 over Fq) and treat
   the point at infinity explicitly (encoding (0,0)); reject anything else.
3. **G2 points from untrusted input**: check on-curve *and* subgroup membership (the pairing
   precompile checks on-curve but implementations differ; do the subgroup check or document why
   the precompile's check suffices for BN254 G2 — and prove it with a test of a non-subgroup
   point).
4. **Public-input count and layout fixed** by the verifying key; reject wrong lengths; no
   dynamic arrays that let a caller drop or append inputs.
5. **Fiat–Shamir transcripts** bind everything: the verifying key digest, all public inputs and
   every commitment, in the reference order; domain-separate every challenge. A transcript that
   omits an input is a soundness bug (Frozen Heart class) — add a test that changing any
   transcript input changes the challenge.
6. **Precompile calls**: check the success flag and the returned length; never ignore failures;
   pairing result must be exactly 1.
7. **No `unchecked` arithmetic on untrusted values** unless a bound is proven in a comment next
   to it; prefer the Field trait's checked constructors.
8. **Malleability**: document which proofs are malleable (Groth16 is) and never use a proof
   (or its hash) as a nullifier or unique id in examples.
9. **Generator hygiene**: the generator validates the verifying key itself (points on curve, in
   subgroup, non-degenerate γ/δ ≠ identity, correct number of IC points), embeds a digest of the
   key in the emitted code, and emits a test with one valid proof and a battery of invalid ones
   (each check above must have a failing case). Generated files carry a header with the tool
   version and the key digest.
10. Write `docs/SAFETY.md` listing each rule, where it is enforced (file:line), and the test
    that exercises it. The jury values this; keep it current.

## 4. Fe-specific value (25 % of the score — design for it)

- Make the **trait-based generic field layer** the highlight: one Poseidon2 / Merkle / transcript
  implementation for every field via `Field`, with zero runtime cost. Show the excerpt in README.
- Use Fe's effect/storage system to make the verifier contracts *pure* where possible (no storage
  access in verification), and use typed wrappers (`G1`, `G2`, `Fr`) so that an unchecked raw
  u256 cannot be passed where a checked element is expected — the type system enforces rule 1–3.
- Before designing, **write small probes** (`scripts/probe/*.fe`) to find out what Fe 26.5 really
  supports (traits with associated constants, generics over traits, arrays of generic length,
  `const` evaluation, inline/asm-like access to precompiles via call with raw data, test
  framework features). Record findings in `docs/DESIGN.md` → "Fe capabilities". If a feature
  is missing or buggy, work around it and **file/mention the compiler issue** in README
  (judges like useful bug reports).

## 5. Workflow rules

- Small commits, each building and testing green (`fe fmt --check && fe test && fe build`, and
  `cargo test` in `gen/`). CI (`.github/workflows/ci.yml`) must stay green; extend it to build and
  test `gen/` and to run `scripts/demo.sh` if it is fast.
- Test vectors must come from **independent reference implementations** (snarkjs, arkworks,
  gnark, halo2, Plonky3, circomlibjs), never from our own code; commit the script that produced
  them and the tool versions.
- Every verifier test includes: a valid proof accepted; and invalid cases rejected — tampered
  proof element, tampered public input, non-canonical input, point not on curve, wrong number of
  inputs, swapped inputs.
- Gas: measure with Fe's test framework or an EVM runner; record in `docs/GAS.md` with the
  command used.
- Keep `docs/AI_LOG.md` updated (date, what the AI produced, how it was verified).
- Do not mention or reuse code, names or fixtures from any other private project of the author.
  Everything in this repo must be original to the contest window or properly attributed
  third-party code (record license and source in README → Declaration).
- Never commit secrets, private keys, SRS/ptau files or large binaries (see `.gitignore`);
  fetch them in scripts with pinned hashes.
- Commit author for this repo: `0xItsss`. Push to `origin` (`https://github.com/0xitsss/fe-zk.git`).

## 6. Definition of done (for the submission)

- [ ] `fe test` green with Groth16 (+ PLONK, KZG if P1) verifying real reference proofs and
      rejecting every invalid case listed above.
- [ ] `fe-zk-gen` emits a working verifier from snarkjs (and at least one more format) keys.
- [ ] Demo reproducible from a clean clone with `scripts/demo.sh`.
- [ ] `docs/SAFETY.md`, `docs/DESIGN.md`, `docs/GAS.md`, `docs/AI_LOG.md` complete.
- [ ] README follows `SUBMISSION.md`, with exact Fe version, Fe highlight excerpt, limitations
      and the declaration.
- [ ] Final commit SHA recorded for the submission form.
