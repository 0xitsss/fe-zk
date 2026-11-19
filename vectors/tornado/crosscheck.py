#!/usr/bin/env python3
"""Fe verifier vs tornado-core's Verifier.sol (solc 0.7.6) in revm: same verdicts on valid and mutated mainnet proofs.

usage: python3 -I crosscheck.py GAS_RUN FE_BIN SOL_BIN
Fe:  verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[6])   (flat words: A, B, C, inputs)
Sol: verifyProof(bytes,uint256[6])  with `_proof` = the 8 words, as Tornado.withdraw passes it
accept = call succeeded and returned 1. Exits 1 on any disagreement (printed), prints a table + gas.
"""
import glob
import json
import os
import subprocess
import sys
import tempfile

GAS_RUN, FE_BIN, SOL_BIN = sys.argv[1:4]
HERE = os.path.dirname(os.path.abspath(__file__))
Q = 21888242871839275222246405745257275088696311157297823662689037894645226208583
R = 21888242871839275222246405745257275088548364400416034343698204186575808495617
M = 1 << 256
ONE = "0" * 63 + "1"
h = lambda x: format(x % M, "064x")

proofs = [json.load(open(f)) for f in sorted(glob.glob(os.path.join(HERE, "proofs", "*.json")))]


def fe_cd(w, x):
    return "sig:verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[6])\n" + "".join(h(v) for v in w + x)


def sol_cd(w, x):
    # head: offset of bytes (7 words), inputs[6]; tail: length 256, 8 words
    return (
        "sig:verifyProof(bytes,uint256[6])\n"
        + h(7 * 32)
        + "".join(h(v) for v in x)
        + h(256)
        + "".join(h(v) for v in w)
    )


cases = []  # (name, words, inputs)
for p in proofs:
    w = [int(v) for v in p["proof_words"]]
    x = [int(v) for v in p["public"]]
    t = p["tx"][:10]
    cases.append((t + "/valid", w, x))
    for i in range(8):
        v = list(w)
        v[i] = (v[i] + 1) % M
        cases.append((t + "/proof_word%d+1" % i, v, x))
        v = list(w)
        v[i] = v[i] + Q  # non-canonical coordinate (also may exceed 2^256 for large words, wrapped below)
        cases.append((t + "/proof_word%d+q" % i, v, x))
        v = list(w)
        v[i] = 0
        cases.append((t + "/proof_word%d=0" % i, v, x))
    for i in range(6):
        y = list(x)
        y[i] = (y[i] + 1) % M
        cases.append((t + "/input%d+1" % i, w, y))
        y = list(x)
        y[i] = y[i] + R
        cases.append((t + "/input%d+r" % i, w, y))
        y = list(x)
        y[i] = 0
        cases.append((t + "/input%d=0" % i, w, y))
    cases.append((t + "/B_real_imag_swapped", w[0:2] + [w[3], w[2], w[5], w[4]] + w[6:8], x))
    cases.append((t + "/A_C_swapped", w[6:8] + w[2:6] + w[0:2], x))
    cases.append((t + "/inputs_swapped_0_1", w, [x[1], x[0]] + x[2:]))
# cross-proof mixes: valid proof of tx i with public inputs of tx j (incl. recipient swap)
for i in range(len(proofs)):
    j = (i + 1) % len(proofs)
    wi = [int(v) for v in proofs[i]["proof_words"]]
    xi = [int(v) for v in proofs[i]["public"]]
    xj = [int(v) for v in proofs[j]["public"]]
    cases.append((proofs[i]["tx"][:10] + "/inputs_of_other_tx", wi, xj))
    y = list(xi)
    y[2] = xj[2]
    cases.append((proofs[i]["tx"][:10] + "/recipient_swap", wi, y))
    y = list(xi)
    y[3] = xj[3] if xj[3] != xi[3] else xi[3] + 1
    cases.append((proofs[i]["tx"][:10] + "/relayer_swap", wi, y))

# drop "mutations" that leave the case unchanged (e.g. a word that is already 0)
orig = {c[0][:10]: (c[1], c[2]) for c in cases if c[0].endswith("/valid")}
cases = [c for c in cases if c[0].endswith("/valid") or (c[1], c[2]) != orig[c[0][:10]]]


def run(init, mk):
    d = tempfile.mkdtemp()
    cmd = [GAS_RUN, "--init", init]
    for n, (name, w, x) in enumerate(cases):
        f = os.path.join(d, "c%d" % n)
        open(f, "w").write(mk(w, x))
        cmd += ["--call", "c%d=%s" % (n, f)]
    r = json.loads(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout)
    return r["calls"]


fe = run(FE_BIN, fe_cd)
so = run(SOL_BIN, sol_cd)
bad = 0
acc = 0
for (name, _, _), a, b in zip(cases, fe, so):
    fa = a["ok"] and a["ret"].endswith(ONE) and a["ret"] == ONE
    sa = b["ok"] and b["ret"] == ONE
    acc += fa
    if fa and not name.endswith("/valid"):
        print("both accept (mutation):", name)
    if fa != sa:
        bad += 1
        print("DISAGREE", name, "fe", fa, "sol", sa)
valid = [i for i, c in enumerate(cases) if c[0].endswith("/valid")]
print("cases %d (accepted by Fe: %d, valid: %d), disagreements: %d" % (len(cases), acc, len(valid), bad))
for i in valid:
    print("valid %s: Fe exec %d gas, Solidity exec %d gas (tx %d vs %d)" % (cases[i][0], fe[i]["exec"], so[i]["exec"], fe[i]["tx"], so[i]["tx"]))
sys.exit(1 if bad else 0)
