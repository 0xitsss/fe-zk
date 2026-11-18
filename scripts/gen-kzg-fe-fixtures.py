#!/usr/bin/env python3
"""Rewrites the fixture constants of src/tests/kzg.fe from vectors/kzg/fixtures/kzg.json (arkworks, real SRS).
Run after scripts/regen-kzg.sh. Only `const NAME: u256 = ...` lines listed below are touched."""
import json, re, sys
root = __file__.rsplit("/scripts/", 1)[0]
d = json.load(open(f"{root}/vectors/kzg/fixtures/kzg.json"))
m = {}
t = d["vk"]["tau_g2"]
for n, v in zip(["TAU_X0", "TAU_X1", "TAU_Y0", "TAU_Y1"], t):
    m[n] = v
s = d["single"]
m.update(S_CX=s["c"][0], S_CY=s["c"][1], S_Z=s["z"], S_Y=s["y"], S_PIX=s["pi"][0], S_PIY=s["pi"][1])
g = d["gwc"]
m["B_Z"] = g["z"]
for i in range(3):
    m[f"B_C{i}X"], m[f"B_C{i}Y"], m[f"B_Y{i}"] = g["cs"][i][0], g["cs"][i][1], g["ys"][i]
m.update(B_V=g["v"], B_PIX=g["pi"][0], B_PIY=g["pi"][1])
h = d["shplonk"]
m.update(H_Z1=h["z1"], H_Z2=h["z2"], H_E00=h["e00"], H_E01=h["e01"], H_E1=h["e1"], H_E2=h["e2"],
         H_Y=h["y"], H_U=h["u"], H_HX=h["h"][0], H_HY=h["h"][1], H_PIX=h["pi"][0], H_PIY=h["pi"][1])
for i in range(3):
    m[f"H_C{i}X"], m[f"H_C{i}Y"] = h["cs"][i]
p = f"{root}/src/tests/kzg.fe"
src = open(p).read()
done = set()
def rep(mo):
    done.add(mo.group(1))
    return f"const {mo.group(1)}: u256 = {m[mo.group(1)]}"
src = re.sub(r"const ([A-Z0-9_]+): u256 = \d+", lambda mo: rep(mo) if mo.group(1) in m else mo.group(0), src)
missing = set(m) - done
if missing:
    sys.exit(f"constants not found in kzg.fe: {sorted(missing)}")
open(p, "w").write(src)
print(f"updated {len(done)} constants in src/tests/kzg.fe")
