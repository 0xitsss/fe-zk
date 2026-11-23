#!/usr/bin/env bash
# Regenerate the Plonky3 uni-stark Fibonacci fixtures, the Fe test fixtures and the per-configuration
# modules of src/verifier/fri/. Two configurations (see vectors/plonky3-stark/README.md):
#   demo    2^4 rows,  log_blowup 2,  4 queries, query PoW  3 bits  (11 conjectured bits, NOT secure)
#   secure  2^10 rows, log_blowup 4, 20 queries, query PoW 20 bits (100 conjectured bits)
#   mul     MulAir (6 columns, 1 public value), 2^8 rows, log_blowup 4, 20 queries, query PoW 20 bits (100 bits)
# Tool versions: p3-* =0.8.0 (pinned in vectors/plonky3-stark/Cargo.lock), fe-zk-gen (gen/), python3.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/vectors/plonky3-stark"
cargo run --release --locked -- fib
cargo run --release --locked -- fib_secure --log-trace 10 --log-blowup 4 --log-final 3 --queries 20 --commit-pow 0 --query-pow 20
cargo run --release --locked -- mul --air mul --log-trace 8 --log-blowup 4 --log-final 3 --queries 20 --commit-pow 0 --query-pow 20
# Fe files: fe-zk-gen plonky3 (Rust); emit_fe.py is the byte-identical Python cross-check
# (also run by `cargo test --test plonky3` in gen/).
gen() { cargo run -q --release --manifest-path "$root/gen/Cargo.toml" -p fe-zk-gen -- plonky3 "$@"; }
gen fib --fixture "$root/src/tests/fri_fixture.fe" --config "$root/src/verifier/fri/config_demo.fe" --name demo --vectors
gen fib_secure --fixture "$root/src/tests/fri_fixture_secure.fe" --config "$root/src/verifier/fri/config_secure.fe" --name secure --min-bits 100
gen mul --fixture "$root/src/tests/fri_fixture_mul.fe" --config "$root/src/verifier/fri/config_mul.fe" --name mul --air mul --min-bits 100
xc="$(mktemp -d)"
trap 'rm -rf "$xc"' EXIT
python3 emit_fe.py fib "$xc/fx_demo.fe" "$xc/cf_demo.fe" demo --vectors
python3 emit_fe.py fib_secure "$xc/fx_secure.fe" "$xc/cf_secure.fe" secure --min-bits 100
python3 emit_fe.py mul "$xc/fx_mul.fe" "$xc/cf_mul.fe" mul --air mul --min-bits 100
cmp "$xc/fx_demo.fe" "$root/src/tests/fri_fixture.fe"
cmp "$xc/cf_demo.fe" "$root/src/verifier/fri/config_demo.fe"
cmp "$xc/fx_secure.fe" "$root/src/tests/fri_fixture_secure.fe"
cmp "$xc/cf_secure.fe" "$root/src/verifier/fri/config_secure.fe"
cmp "$xc/fx_mul.fe" "$root/src/tests/fri_fixture_mul.fe"
cmp "$xc/cf_mul.fe" "$root/src/verifier/fri/config_mul.fe"
echo "cross-check: fe-zk-gen plonky3 == emit_fe.py (byte-identical)"
cd "$root"
for f in src/tests/fri_fixture.fe src/tests/fri_fixture_secure.fe src/verifier/fri/config_demo.fe src/verifier/fri/config_secure.fe src/tests/fri_fixture_mul.fe src/verifier/fri/config_mul.fe; do
  fe fmt "$f"
done
echo "done: vectors/plonky3-stark/{fib,fib_secure}/*.json, src/tests/fri_fixture*.fe, src/verifier/fri/config_*.fe, mul/*.json"
