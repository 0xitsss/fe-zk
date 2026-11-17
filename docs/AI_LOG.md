# AI assistance log

| Date | What was produced | How it was verified |
|---|---|---|
| 2026-11-17 | Language probes in `scripts/probe/` (Field trait with associated const and const generics, BN254 pairing via `raw_staticcall`, private-field encapsulation) and `docs/DESIGN.md` → "Fe capabilities" (Claude Opus 5.5) | Each probe run with `fe test` on fe 26.5.0; the pairing probe checks that a valid pair `e(G1,G2)·e(-G1,G2)` is accepted and `e(G1,G2)·e(G1,G2)` is rejected; the `encap` probe fails to compile as expected |
