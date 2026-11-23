# Plonky3 uni-stark Fibonacci fixtures

Standalone crate (not part of `gen/`). `scripts/regen-plonky3-stark.sh` runs it for three configurations and
converts the output with `fe-zk-gen plonky3` (checked byte for byte against `emit_fe.py`) to the Fe fixtures (`src/tests/fri_fixture*.fe`) and the per-configuration modules
(`src/verifier/fri/config_*.fe`).

- Plonky3 `p3-uni-stark`, `p3-fri`, `p3-challenger`, `p3-merkle-tree`, `p3-keccak`,
  `p3-koala-bear`, all `=0.8.0` from crates.io (versions fixed by `Cargo.lock`).
- AIR: Fibonacci (default, `--air fib`), 2 columns, 3 public values `(a, b, x)`, `(a, b) = (0, 1)`, `x` = the last row's right column.
- Field KoalaBear, challenge field `BinomialExtensionField<KoalaBear, 4>` (`x^4 = 3`).
- FRI: arity 2 (`max_log_arity = 1`), one commit-phase proof of work per round (0 bits here), query proof of work;
  batch and out-of-domain proof of work 0 bits (witnesses 0).
- Merkle trees: cap height 0; openings are 0.8 pruned multi-openings (`PrunedMerklePaths`).
- Commitments: Keccak-256 over `SerializingHasher` rows, compression `keccak256(l || r)`;
  challenger `SerializingChallenger32<KoalaBear, HashChallenger<u8, Keccak256Hash, 32>>`.

| directory | trace rows | `log_blowup` | `log_final_poly_len` | queries | query PoW | conjectured bits | use |
|---|---:|---:|---:|---:|---:|---:|---|
| `fib/` | 2^4 | 2 | 1 | 4 | 3 | 11 | demo configuration (`config_demo`), carries the extension-field and challenger vectors; NOT secure |
| `fib_secure/` | 2^10 | 4 | 3 | 20 | 20 | 100 | secure configuration (`config_secure`), the deployable contract `examples/fri_stark` |
| `mul/` | 2^8 | 4 | 3 | 20 | 20 | 100 | second AIR (`--air mul`, `MulAir`: 6 columns, 1 public value, degree 2, modelled on Plonky3's `MulAir` test AIR), configuration `config_mul` |

The crate takes `--log-trace`, `--log-blowup`, `--log-final`, `--queries`, `--commit-pow`, `--query-pow` and an output
directory (defaults: the `fib/` row). `cargo run --release --locked -- <dir> --log-trace 10 ...`.

Soundness, as written into `config.json` (`soundness`):

- `conjectured_bits` is Plonky3's own `FriParameters::conjectured_soundness_bits`,
  `log_blowup * num_queries + query_proof_of_work_bits` (the ethSTARK conjecture on the proximity gap of
  Reed-Solomon codes up to capacity).
- `proven_unique_decoding_bits` and `proven_johnson_m10_bits` are our own estimates of what is proven without the
  conjecture (per-query error `(1 + rho) / 2` and `(1 + 1/(2m)) sqrt(rho)` with `m = 10`, plus the grinding bits,
  combined with the BCIKS'20 commit-phase error `(m + 1/2)^7 / (3 rho^1.5) * n^2 / |F|` for `|F| = 2^124`).
  The formulas are in `src/main.rs` (`Cfg::soundness`); they are not an audit.

Files in `fib/` and `fib_secure/`:

- `proof.json`: serde_json of the Plonky3 `Proof`. Field elements are in Montgomery form
  (`value * 2^32 mod p`), as serde emits them; `emit_fe.py` converts them to canonical values.
  Hashes are 0x-hex.
- `config.json`: parameters, public values (canonical) and the soundness estimates.
- `vectors.json`: values printed by Plonky3 itself (all canonical): extension-field
  operations, a scripted challenger session, two-adic generators, and every Fiat-Shamir
  challenge of this proof (`transcript`), and the three domain-separator seeds (`transcript.seeds`: protocol id,
  pattern hash, instance label and the field elements Plonky3 absorbed). Only `fib/vectors.json` is used for the extension-field and challenger
  tests; `fib_secure/vectors.json` supplies the transcript.

`fe-zk-gen plonky3 <dir> --fixture F --config C --name N [--air fib|mul] [--vectors] [--min-bits N]` (and
`emit_fe.py <dir> <fixture.fe> <config.fe> <name> [--air fib|mul] [--vectors] [--min-bits N]`, same output) flattens the proof into the word layout
documented in `src/verifier/fri.fe`, asserts the length against the layout formula, and (with `--min-bits`) emits a
compile-time `static_assert` that the configuration's conjectured soundness is at least `N`.

The query PoW witness is found by Plonky3's `grind`, which may run on several threads; the
witness (not its validity) can differ between machines.
