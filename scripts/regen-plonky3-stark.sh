#!/usr/bin/env bash
# Regenerate the Plonky3 uni-stark Fibonacci fixture and the Fe test fixture.
# Tool versions: p3-* =0.4.2 (pinned in vectors/plonky3-stark/Cargo.lock), python3.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/vectors/plonky3-stark"
cargo run --release --locked -- fib
python3 emit_fe.py fib "$root/src/tests/fri_fixture.fe"
cd "$root"
fe fmt src/tests/fri_fixture.fe
echo "done: vectors/plonky3-stark/fib/*.json and src/tests/fri_fixture.fe"
