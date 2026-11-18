#!/usr/bin/env bash
# Regenerate the Plonky3 uni-stark Fibonacci fixtures, the Fe test fixtures and the per-configuration
# modules of src/verifier/fri/. Two configurations (see vectors/plonky3-stark/README.md):
#   demo    2^4 rows,  log_blowup 2,  4 queries, query PoW  3 bits  (11 conjectured bits, NOT secure)
#   secure  2^8 rows,  log_blowup 3, 27 queries, query PoW 20 bits (101 conjectured bits)
# Tool versions: p3-* =0.4.2 (pinned in vectors/plonky3-stark/Cargo.lock), python3.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/vectors/plonky3-stark"
cargo run --release --locked -- fib
cargo run --release --locked -- fib_secure --log-trace 8 --log-blowup 3 --log-final 1 --queries 27 --commit-pow 0 --query-pow 20
python3 emit_fe.py fib "$root/src/tests/fri_fixture.fe" "$root/src/verifier/fri/config_demo.fe" demo --vectors
python3 emit_fe.py fib_secure "$root/src/tests/fri_fixture_secure.fe" "$root/src/verifier/fri/config_secure.fe" secure --min-bits 100
cd "$root"
for f in src/tests/fri_fixture.fe src/tests/fri_fixture_secure.fe src/verifier/fri/config_demo.fe src/verifier/fri/config_secure.fe; do
  fe fmt "$f"
done
echo "done: vectors/plonky3-stark/{fib,fib_secure}/*.json, src/tests/fri_fixture*.fe, src/verifier/fri/config_*.fe"
