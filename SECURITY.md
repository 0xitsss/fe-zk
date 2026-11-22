# Security policy

## Not audited

fe-zk has **not been audited**. It is a research and contest project. Do not secure real funds with it before an
independent review. All demo keys in this repository are test-only (see README, "Setup provenance").

## Supported versions

| Version | Supported |
|---|---|
| 0.3.x (latest `main`) | yes, fixes land on `main` |
| anything older | no |

Fe 26.5.0 is the only supported compiler version.

## Reporting a vulnerability

Report privately through GitHub security advisories: <https://github.com/0xitsss/fe-zk/security/advisories/new>
(repository `0xitsss/fe-zk`, "Security" tab, "Report a vulnerability"). Please do not open a public issue or pull
request for a suspected vulnerability. Include the verifier or generator affected, a proof or key that triggers it
(a failing `fe test` is ideal) and the impact you see. You can expect an acknowledgement within a week.

## Scope

In scope: the `fe_zk` library (`src/`: field, curve, hash, verifiers), the generator `gen/fe-zk-gen` and the
contracts it emits, and the example contracts. Examples of bugs we want to hear about: a verifier that accepts an
invalid proof, a missing safety check from [docs/SAFETY.md](docs/SAFETY.md), a transcript that omits an input, a
generator that emits code without a required check, or a non-canonical input that is silently reduced.

Out of scope: the test-only demo keys and SRS (known toxic waste in some demo setups, documented in the README),
the reference tools (snarkjs, gnark, halo2, Plonky3), the Fe compiler (report to the Fe project), and
availability of the demo scripts.

## Threat model in short

The attacker controls everything a caller passes to a verifier: proof bytes, public inputs, their lengths and
encodings. The verifying key is trusted only if it came through `fe-zk-gen`, which validates it and re-validates it at
compile time. Verifiers are read-only (no storage). Precompile failures fail closed. Groth16 proofs are malleable, so
they must not be used as nullifiers or ids. Soundness of the underlying proof system and the trusted setup are
assumed. The full list of rules, where each is enforced (file and line) and the test that exercises it is
[docs/SAFETY.md](docs/SAFETY.md).
