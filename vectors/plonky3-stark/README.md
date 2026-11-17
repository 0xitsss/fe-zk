# Plonky3 uni-stark Fibonacci fixture

Standalone crate (not part of `gen/`). `scripts/regen-plonky3-stark.sh` runs it and converts
the output to `src/tests/fri_fixture.fe`.

- Plonky3 `p3-uni-stark`, `p3-fri`, `p3-challenger`, `p3-merkle-tree`, `p3-keccak`,
  `p3-koala-bear`, all `=0.4.2` from crates.io (versions fixed by `Cargo.lock`).
- AIR: Fibonacci, 2 columns, 3 public values `(a, b, x)`, 16 rows, `(a, b) = (0, 1)`, `x = 987`.
- Field KoalaBear, challenge field `BinomialExtensionField<KoalaBear, 4>` (`x^4 = 3`).
- FRI: arity 2, `log_blowup = 2`, `log_final_poly_len = 1`, 4 queries, commit PoW 0 bits,
  query PoW 3 bits.
- Commitments: Keccak-256 over `SerializingHasher` rows, compression `keccak256(l || r)`;
  challenger `SerializingChallenger32<KoalaBear, HashChallenger<u8, Keccak256Hash, 32>>`.

Files in `fib/`:

- `proof.json`: serde_json of the Plonky3 `Proof`. Field elements are in Montgomery form
  (`value * 2^32 mod p`), as serde emits them; `emit_fe.py` converts them to canonical values.
  Hashes are 0x-hex.
- `config.json`: parameters and public values (canonical).
- `vectors.json`: values printed by Plonky3 itself (all canonical): extension-field
  operations, a scripted challenger session, two-adic generators, and every Fiat-Shamir
  challenge of this proof (`transcript`).

The query PoW witness is found by Plonky3's `grind`, which may run on several threads; the
witness (not its validity) can differ between machines.
