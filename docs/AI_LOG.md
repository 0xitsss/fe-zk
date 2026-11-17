# AI assistance log

| Date | What was produced | How it was verified |
|---|---|---|
| 2026-11-17 | Language probes in `scripts/probe/` (Field trait with associated const and const generics, BN254 pairing via `raw_staticcall`, private-field encapsulation) and `docs/DESIGN.md` → "Fe capabilities" (Claude Opus 5.5) | Each probe run with `fe test` on fe 26.5.0; the pairing probe checks that a valid pair `e(G1,G2)·e(-G1,G2)` is accepted and `e(G1,G2)·e(G1,G2)` is rejected; the `encap` probe fails to compile as expected |
| 2026-11-17 | Field trait and generic `Fp<P>` (`src/field*`) (Claude Opus 5.5) | `fe test`: expected values computed with Python big integers (`pow(a, -1, p)`, `a * b % p`), non-canonical inputs rejected |
| 2026-11-17 | `examples/life`: word-parallel Game of Life step in Fe, `Life` contract, Python per-cell reference + vector generator, Fe tests, revm CLI `life-cli`, `scripts/life-demo.sh` (Claude Sonnet subagent) | All 22 step vectors come from the independent per-cell Python reference `ref/life.py`; `fe test` (26) and `cargo test` (10) green; `life-cli --check` compares every generation with a per-cell Rust reference |
