#!/usr/bin/env python3
"""Rewrites the expected challenge constants (SH_*/GW_*) of src/tests/halo2.fe from the independent Rust mini
verifier trace (`H2_TRACE=1 cargo test --test mini_verifier accepts_valid -- --nocapture` in vectors/halo2).
Run after scripts/regen-halo2.sh (proof bytes change on every regeneration, so do the challenges)."""
import os, re, subprocess, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
env = dict(os.environ, H2_TRACE="1")
r = subprocess.run(["cargo", "test", "--release", "--locked", "--test", "mini_verifier", "accepts_valid", "--", "--nocapture"],
                   cwd=f"{root}/vectors/halo2", env=env, capture_output=True, text=True)
if r.returncode != 0:
    sys.exit(r.stdout + r.stderr)
blocks, cur = [], None
for line in r.stderr.splitlines():
    m = re.match(r"H2T (\w+) ([0-9a-f]{64})$", line)
    if not m:
        continue
    k, v = m.groups()
    if k == "theta":
        cur = {}
        blocks.append(cur)
    cur[k] = v
assert len(blocks) == 2, blocks
sh, gw = blocks  # accepts_valid_proofs verifies SHPLONK first, then GWC
vals = {}
for pre, b in (("SH", sh), ("GW", gw)):
    for k in ("theta", "beta", "gamma", "y", "x", "expected_h"):
        vals[f"{pre}_{k.upper()}"] = b[k]
vals.update(SH_OPEN_Y=sh["sh_y"], SH_OPEN_V=sh["sh_v"], SH_OPEN_U=sh["sh_u"], GW_OPEN_V=gw["v"], GW_OPEN_U=gw["u"])
p = f"{root}/src/tests/halo2.fe"
src = open(p).read()
done = set()
def rep(m):
    done.add(m.group(1))
    return f"const {m.group(1)}: u256 = 0x{vals[m.group(1)]}"
src = re.sub(r"const ((?:SH|GW)_[A-Z_]+): u256 = 0x[0-9a-f]{64}", lambda m: rep(m) if m.group(1) in vals else m.group(0), src)
if set(vals) - done:
    sys.exit(f"constants not found: {sorted(set(vals) - done)}")
open(p, "w").write(src)
print(f"updated {len(done)} challenge constants in src/tests/halo2.fe")
