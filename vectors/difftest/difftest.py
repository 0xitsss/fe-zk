#!/usr/bin/env python3
"""Differential test: every Fe verifier vs its reference verifier, same inputs, same EVM (revm).

Called by scripts/difftest.sh, which builds the tools first. For each family it
  1. prepares a Fe verifier (fe-zk-gen or the vectors/gas-fe wrapper) and the reference verifier
     (snarkjs / gnark / snark-verifier Solidity compiled with solc, or ark-groth16 natively),
  2. collects valid proofs (fresh ones from the reference prover where cheap, plus committed fixtures),
  3. derives mutations of them (seeded RNG) and runs Fe and the reference on identical inputs,
  4. compares verdicts: accept = call succeeded and returned true (or nothing), reject = false / revert.
A disagreement must be on the ALLOW list (with a reason) or it is a BUG. Exit status 1 if any BUG.
See docs/TESTING.md.
"""
import argparse
import concurrent.futures as cf
import itertools
import json
import os
import random
import re
import shutil
import subprocess
import sys
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
Q = 21888242871839275222246405745257275088696311157297823662689037894645226208583
R = 21888242871839275222246405745257275088548364400416034343698204186575808495617
M256 = 1 << 256

# --------------------------------------------------------------------------- allowlist
# (family regex, mutation-kind regex, Fe verdict 'a'|'n', reference verdict 'a'|'n', reason)
# 'n' = not accepted (false or revert). Anything that disagrees and is not listed here is a BUG.
ALLOW = [
    (r"halo2-(shplonk|gwc)/.*", r"word/(pt|pub)/\+r", "n", "a",
     "Fe stricter by design (SAFETY rule 1): the word is a scalar (instance or proof evaluation) shifted by +r, i.e. the same "
     "field element non-canonically encoded; snark-verifier's generated Solidity reduces it mod r and accepts, Fe rejects "
     "non-canonical scalars. Only positions that hold scalars appear here (instance word 0 and proof evaluations)."),
]


def h32(x):
    return format(x % M256, "064x")


def read_json(*p):
    with open(os.path.join(ROOT, *p)) as f:
        return json.load(f)


def sh(cmd, **kw):
    r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, **kw)
    if r.returncode != 0:
        sys.exit(f"command failed: {' '.join(map(str, cmd))}\n{r.stdout}\n{r.stderr}")
    return r.stdout


# --------------------------------------------------------------------------- families

class Fam:
    """One Fe verifier vs one reference. Words are ints; a case is (kind, fe_body_hex, ref_body_hex)."""

    def __init__(self, name, fe_sig, ref_sig, fe_body, ref_body, valid, pt_pairs, g2, pubs, ref_kind="evm",
                 rerand_npub=None):
        self.name, self.fe_sig, self.ref_sig = name, fe_sig, ref_sig
        self.fe_body, self.ref_body = fe_body, ref_body        # functions: words -> hex string
        self.valid = valid                                      # list of word lists
        self.pt_pairs = pt_pairs                                # (i, i+1) = G1 point words
        self.g2 = g2                                            # start index of 4-word G2 points (imag first)
        self.pubs = pubs                                        # indices of public inputs
        self.ref_kind = ref_kind                                # 'evm' or 'ark'
        self.rerand_npub = rerand_npub                          # Groth16: also test re-randomised proofs
        self.fe_bin = self.ref_bin = None
        self.ark_seed = None
        self.n_fresh = 0
        pt = {j for p in pt_pairs for j in p} | {g + k for g in g2 for k in range(4)}
        self.ptset, self.pubset = pt, set(pubs)

    def cls(self, i):
        return "pub" if i in self.pubset else "pt" if i in self.ptset else "sc"


OPS = [
    ("+1", lambda v: (v + 1) % M256), ("-1", lambda v: (v - 1) % M256), ("0", lambda v: 0), ("1", lambda v: 1),
    ("q", lambda v: Q), ("r", lambda v: R), ("r-1", lambda v: R - 1), ("q-1", lambda v: Q - 1),
    ("q+1", lambda v: Q + 1), ("r+1", lambda v: R + 1), ("max", lambda v: M256 - 1),
    ("+r", lambda v: v + R if v + R < M256 else v), ("+q", lambda v: v + Q if v + Q < M256 else v),
    ("2^255", lambda v: 1 << 255),
]


def rand_value(rng):
    t = rng.randrange(6)
    return [rng.randrange(M256), rng.randrange(Q), rng.randrange(R), rng.randrange(1 << 64), rng.choice([0, 1, 2, Q - 1, R - 1, Q, R, M256 - 1]),
            rng.randrange(M256) | (1 << 255)][t]


def mutations(f, base, rng, nrand, quick):
    """Yield (kind, words). Deterministic given rng."""
    n = len(base)
    combos = [(i, name) for i in range(n) for name, _ in OPS]
    if quick:
        combos = rng.sample(combos, min(len(combos), 70))
    ops = dict(OPS)
    for i, name in combos:
        w = list(base)
        w[i] = ops[name](base[i])
        yield f"word/{f.cls(i)}/{name}", w, f"word {i}"
    for (i, j) in f.pt_pairs:
        w = list(base); w[j] = (Q - base[j]) % Q if base[j] < Q else base[j]
        yield "pt/negate", w, None
        w = list(base); w[i], w[j] = 0, 0
        yield "pt/infinity", w, None
        w = list(base); w[i], w[j] = 1, 2
        yield "pt/generator", w, None
        w = list(base); w[i], w[j] = base[j], base[i]
        yield "pt/swap-xy", w, None
    for g in f.g2:
        w = list(base)
        for k in (2, 3):
            w[g + k] = (Q - base[g + k]) % Q if base[g + k] < Q else base[g + k]
        yield "g2/negate", w, None
        w = list(base)
        for k in range(4):
            w[g + k] = 0
        yield "g2/infinity", w, None
        w = list(base); w[g], w[g + 1] = base[g + 1], base[g]
        yield "g2/swap-x-parts", w, None
        w = list(base); w[g + 2], w[g + 3] = base[g + 3], base[g + 2]
        yield "g2/swap-y-parts", w, None
    pairs = list(itertools.combinations(f.pt_pairs, 2))
    if len(pairs) > 40:
        pairs = rng.sample(pairs, 40)
    for (a, b) in pairs:
        w = list(base)
        w[a[0]], w[a[1]], w[b[0]], w[b[1]] = base[b[0]], base[b[1]], base[a[0]], base[a[1]]
        yield "pt/swap-points", w, None
    for a, b in itertools.combinations(f.pubs, 2):
        if base[a] != base[b]:
            w = list(base); w[a], w[b] = base[b], base[a]
            yield "pub/swap", w, None
    for _ in range(12 if quick else 40):
        i, j = rng.sample(range(n), 2)
        w = list(base); w[i] = base[j]
        yield "word/dup", w, None
        w = list(base); w[i], w[j] = base[j], base[i]
        yield "word/swap", w, None
    for _ in range(nrand):
        w = list(base)
        for i in rng.sample(range(n), rng.randint(1, min(3, n))):
            w[i] = rand_value(rng)
        yield "random", w, None


def length_cases(f, base):
    """Length mutations on the final calldata bodies; Fe and the reference each get their own."""
    fe, rf = f.fe_body(base), f.ref_body(base)
    out = []
    for kind, fn in [
        ("len/drop32", lambda b: b[:-64]), ("len/drop1", lambda b: b[:-2]), ("len/pad32-zero", lambda b: b + "00" * 32),
        ("len/pad1", lambda b: b + "01"), ("len/pad32-ff", lambda b: b + "ff" * 32), ("len/empty", lambda b: ""),
        ("len/dup-body", lambda b: b + b),
    ]:
        out.append((kind, fn(fe), fn(rf)))
    return out


# --------------------------------------------------------------------------- running

def run_evm(a, fe_bin, ref_bin, fe_sig, ref_sig, cases, chunk=4000):
    """Returns [(fe_verdict, ref_verdict)] for cases = [(kind, fe_body, ref_body)]."""
    lines = []
    for _, fb, rb in cases:
        lines.append(f"F {fe_sig or '-'} {fb}")
        lines.append(f"R {ref_sig or '-'} {rb}")
    r = subprocess.run([a.evm_batch, "--init", f"F={fe_bin}", "--init", f"R={ref_bin}"],
                       input="\n".join(lines) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"evm-batch failed: {r.stderr[-2000:]}")
    v = r.stdout.split()
    assert len(v) == 2 * len(cases), (len(v), len(cases))
    return [(v[2 * i], v[2 * i + 1]) for i in range(len(cases))]


def run_ark(a, fe_bin, fe_sig, seed, cases):
    lines = [f"F {fe_sig} {fb}" for _, fb, _ in cases]
    r = subprocess.run([a.evm_batch, "--init", f"F={fe_bin}"], input="\n".join(lines) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"evm-batch failed: {r.stderr[-2000:]}")
    fv = r.stdout.split()
    r = subprocess.run([a.ark_ref, "verify", str(seed)], input="\n".join(rb for _, _, rb in cases) + "\n",
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"ark-ref failed: {r.stderr[-2000:]}")
    rv = r.stdout.split()
    assert len(fv) == len(rv) == len(cases)
    return list(zip(fv, rv))


def allowed(fam, kind, fv, rv):
    for fre, kre, f, r, reason in ALLOW:
        if re.fullmatch(fre, fam) and re.fullmatch(kre, kind) and (fv == "a") == (f == "a") and (rv == "a") == (r == "a"):
            return reason
    return None


def test_family(a, f, rng):
    t0 = time.time()
    res = {"family": f.name, "valid": 0, "mut": 0, "agree": 0, "allowed": 0, "bugs": [], "allowed_kinds": {}, "both_accept_mut": {},
           "mut_reject_both": 0, "allowed_pos": {}}

    def run(cases):
        if f.ref_kind == "ark":
            return run_ark(a, f.fe_bin, f.fe_sig, f.ark_seed, cases)
        return run_evm(a, f.fe_bin, f.ref_bin, f.fe_sig, f.ref_sig, cases)

    # ---- valid proofs (+ re-randomised variants for Groth16)
    valid = list(f.valid)
    if f.rerand_npub:
        src = [valid[i % len(valid)] for i in range(a.rerand_n)]
        lines = "\n".join("".join(h32(x) for x in w) for w in src) + "\n"
        out = sh([a.ark_ref, "rerand", str(f.rerand_npub)], input=lines).split()
        rer = [[int(o[i:i + 64], 16) for i in range(0, len(o), 64)] for o in out]
        res["rerand"] = len(rer)
        valid_cases = [("valid", f.fe_body(w), f.ref_body(w)) for w in valid] + [("valid/rerandomised", f.fe_body(w), f.ref_body(w)) for w in rer]
    else:
        valid_cases = [("valid", f.fe_body(w), f.ref_body(w)) for w in valid]
    for (kind, fb, rb), (fv, rv) in zip(valid_cases, run(valid_cases)):
        res["valid"] += 1
        if fv == "a" and rv == "a":
            res["agree"] += 1
        else:
            res["bugs"].append({"kind": kind, "fe": fv, "ref": rv, "fe_body": fb, "ref_body": rb,
                                "class": "COMPLETENESS (Fe rejects a valid proof)" if rv == "a" else "HARNESS (reference rejects a valid proof)"})
    # ---- mutations
    nbase = 1 if a.quick else min(4, len(valid))
    nrand = a.quick_random if a.quick else a.random
    cases, seen, notes = [], set(), {}
    for bi, base in enumerate(rng.sample(valid, nbase) if len(valid) > nbase else valid):
        gens = list(mutations(f, base, rng, nrand // nbase, a.quick))
        for kind, w, note in gens:
            if w == base:
                continue
            c = (kind, f.fe_body(w), f.ref_body(w))
            if (c[1], c[2]) not in seen:
                seen.add((c[1], c[2])); cases.append(c); notes[(c[1], c[2])] = note
        for c in length_cases(f, base):
            if (c[1], c[2]) not in seen:
                seen.add((c[1], c[2])); cases.append(c)
    verdicts = []
    for i in range(0, len(cases), 6000):
        verdicts += run(cases[i:i + 6000])
    kinds_both = {}
    for (kind, fb, rb), (fv, rv) in zip(cases, verdicts):
        res["mut"] += 1
        top = kind.split("/")[0] + "/" + kind.split("/")[1] if kind.count("/") >= 1 else kind
        if (fv == "a") == (rv == "a"):
            res["agree"] += 1
            if fv == "a":
                kinds_both[kind] = kinds_both.get(kind, 0) + 1
            else:
                res["mut_reject_both"] += 1
            continue
        why = allowed(f.name, kind, fv, rv)
        if why:
            res["allowed"] += 1
            k = f"{kind} fe={fv} ref={rv}: {why}"
            res["allowed_pos"].setdefault(k, set()).add(notes.get((fb, rb)) or "")
            res["allowed_kinds"][k] = res["allowed_kinds"].get(k, 0) + 1
        else:
            res["bugs"].append({"kind": kind, "note": notes.get((fb, rb)), "fe": fv, "ref": rv, "fe_body": fb, "ref_body": rb,
                                "class": "SOUNDNESS (Fe accepts, reference rejects)" if fv == "a" else "COMPLETENESS (Fe rejects, reference accepts)"})
    res["both_accept_mut"] = kinds_both
    res["allowed_pos"] = {k: sorted(v) for k, v in res["allowed_pos"].items()}
    res["secs"] = round(time.time() - t0, 1)
    res["fresh"] = f.n_fresh
    return res


# --------------------------------------------------------------------------- preparation

class Prep:
    def __init__(self, a):
        self.a = a
        self.w = a.work
        self.builds = []      # (ingot dir, out dir)
        self.fams = []

    def d(self, *p):
        d = os.path.join(self.w, *p)
        os.makedirs(d, exist_ok=True)
        return d

    def merge(self, name, ingot):
        """Adds a generated verifier to the combined ingot (one `fe build` compiles fe_zk once, not once per verifier)."""
        shutil.copy(os.path.join(ingot, "src", "lib.fe"), os.path.join(self.w, "gas-fe", "src", name.lower() + "_gen.fe"))
        return os.path.join(self.w, "bin", "all")

    def solc(self, name, sol, contract, patch=False, optimize=True):
        d = self.d("sol", name)
        src = sol
        if patch:
            text = re.sub(r"pragma solidity 0\.8\.19;", "pragma solidity ^0.8.19;", open(sol).read())
            src = os.path.join(d, "Patched.sol")
            open(src, "w").write(text)
        flags = ["--optimize", "--optimize-runs", "200"] if optimize else []
        sh(["solc", *flags, "--bin", "--overwrite", "-o", d, src])
        return os.path.join(d, contract + ".bin")

    def snarkjs(self, *args):
        return sh([os.path.join(self.a.node_modules, ".bin", "snarkjs"), *map(str, args)])

    def fresh_setup(self, proto, circom_file, name):
        """Compile + setup with snarkjs; returns dict of paths. PLONK setup is deterministic from the ptau."""
        d = self.d("fresh", f"{proto}-{name}")
        a = self.a
        sh([a.circom, circom_file, "--r1cs", "--wasm", "-l", a.node_modules, "-o", d])
        base = os.path.basename(circom_file)[:-7]
        r1cs, wasm = f"{d}/{base}.r1cs", f"{d}/{base}_js/{base}.wasm"
        zkey = f"{d}/final.zkey"
        if proto == "groth16":
            self.snarkjs("groth16", "setup", r1cs, a.ptau, f"{d}/0.zkey")
            self.snarkjs("zkey", "contribute", f"{d}/0.zkey", zkey, "--name=difftest", "-e=fe-zk difftest public entropy, insecure")
        else:
            self.snarkjs("plonk", "setup", r1cs, a.ptau, zkey)
        self.snarkjs("zkey", "export", "verificationkey", zkey, f"{d}/vk.json")
        self.snarkjs("zkey", "export", "solidityverifier", zkey, f"{d}/Verifier.sol")
        return {"dir": d, "wasm": wasm, "zkey": zkey, "vk": f"{d}/vk.json", "sol": f"{d}/Verifier.sol"}

    def fresh_proofs(self, proto, circuit, setup, n, seed):
        out = sh(["node", os.path.join(ROOT, "vectors", "difftest", "prove.cjs"), proto, circuit, setup["wasm"], setup["zkey"], str(n), str(seed),
                  self.a.node_modules])
        return [json.loads(l) for l in out.splitlines() if l.strip()]

    # ---- Groth16 (snarkjs layout)
    @staticmethod
    def g16_words(p, pub):
        a, b, c = p["pi_a"], p["pi_b"], p["pi_c"]
        return [int(x) for x in [a[0], a[1], b[0][1], b[0][0], b[1][1], b[1][0], c[0], c[1]] + list(pub)]

    def g16_family(self, label, vk, sol, valids, first, sig_n, fresh_n):
        """valids: list of word lists; first = (proof.json path, public.json path) for the gen test."""
        name = "G16" + re.sub(r"[^A-Za-z0-9]", "", label)
        ing = self.d("ingots", name)
        sh([self.a.gen, "groth16", "--snarkjs", vk, "--proof", first[0], "--public", first[1], "--name", name, "--out", ing,
            "--fe-zk-path", os.path.relpath(ROOT, ing)])
        out = self.merge(name, ing)
        sig = f"verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[{sig_n}])"
        body = lambda w: "".join(h32(x) for x in w)
        f = Fam(f"groth16/{label}", sig, sig, body, body, valids, [(0, 1), (6, 7)], [2], list(range(8, 8 + sig_n)), rerand_npub=sig_n)
        f.n_fresh = fresh_n
        f.fe_bin = os.path.join(out, name + "Verifier.bin")
        f.ref_bin = self.solc("g16_" + label, sol, "Groth16Verifier")
        return f

    def groth16_fixture(self, fx, n):
        d = os.path.join(ROOT, "vectors", "groth16", fx)
        p, pub = read_json("vectors", "groth16", fx, "proof.json"), read_json("vectors", "groth16", fx, "public.json")
        return self.g16_family(fx + "-fixture", f"{d}/verification_key.json", f"{d}/Verifier.sol", [self.g16_words(p, pub)],
                               (f"{d}/proof.json", f"{d}/public.json"), n, 0)

    def groth16_fresh(self, circuit, circom_file, count, seed):
        s = self.fresh_setup("groth16", circom_file, circuit)
        pr = self.fresh_proofs("groth16", "membership" if circuit == "membership" else "multiplier", s, count, seed)
        d = s["dir"]
        json.dump(pr[0]["proof"], open(f"{d}/proof0.json", "w"))
        json.dump(pr[0]["pub"], open(f"{d}/public0.json", "w"))
        valids = [self.g16_words(x["proof"], x["pub"]) for x in pr]
        return self.g16_family(circuit + "-fresh", s["vk"], s["sol"], valids, (f"{d}/proof0.json", f"{d}/public0.json"), len(pr[0]["pub"]), count)

    # ---- PLONK snarkjs
    @staticmethod
    def plonk_words(pr, pub):
        w = []
        for k in ["A", "B", "C", "Z", "T1", "T2", "T3", "Wxi", "Wxiw"]:
            w += [pr[k][0], pr[k][1]]
        for k in ["eval_a", "eval_b", "eval_c", "eval_s1", "eval_s2", "eval_zw"]:
            w.append(pr[k])
        return [int(x) for x in w + list(pub)]

    def plonk_family(self, label, vk, sol, valids, first, n, fresh_n):
        name = "Plonk" + re.sub(r"[^A-Za-z0-9]", "", label)
        ing = self.d("ingots", name)
        sh([self.a.gen, "plonk", "--snarkjs", vk, "--proof", first[0], "--public", first[1], "--name", name, "--out", ing,
            "--fe-zk-path", os.path.relpath(ROOT, ing)])
        out = self.merge(name, ing)
        sig = f"verifyProof(uint256[24],uint256[{n}])"
        body = lambda w: "".join(h32(x) for x in w)
        f = Fam(f"plonk-snarkjs/{label}", sig, sig, body, body, valids, [(2 * k, 2 * k + 1) for k in range(9)], [], list(range(24, 24 + n)))
        f.n_fresh = fresh_n
        f.fe_bin = os.path.join(out, name + "Verifier.bin")
        f.ref_bin = self.solc("plonk_" + label, sol, "PlonkVerifier")
        return f

    def plonk_fixture(self, fx):
        d = os.path.join(ROOT, "vectors", "plonk", fx)
        p, pub = read_json("vectors", "plonk", fx, "proof.json"), read_json("vectors", "plonk", fx, "public.json")
        return self.plonk_family(fx + "-fixture", f"{d}/verification_key.json", f"{d}/Verifier.sol", [self.plonk_words(p, pub)],
                                 (f"{d}/proof.json", f"{d}/public.json"), len(pub), 0)

    def plonk_fresh(self, circuit, circom_file, count, seed):
        s = self.fresh_setup("plonk", circom_file, circuit)
        pr = self.fresh_proofs("plonk", "membership" if circuit == "membership" else "multiplier", s, count, seed)
        d = s["dir"]
        json.dump(pr[0]["proof"], open(f"{d}/proof0.json", "w"))
        json.dump(pr[0]["pub"], open(f"{d}/public0.json", "w"))
        valids = [self.plonk_words(x["proof"], x["pub"]) for x in pr]
        return self.plonk_family(circuit + "-fresh", s["vk"], s["sol"], valids, (f"{d}/proof0.json", f"{d}/public0.json"), len(pr[0]["pub"]), count)

    # ---- arkworks: Fe vs native ark-groth16
    def arkworks(self, count, seed):
        d = self.d("ark")
        sh([self.a.ark_ref, "gen", d, str(seed), str(count)])
        valids = [[int(l[i:i + 64], 16) for i in range(0, len(l), 64)] for l in open(f"{d}/valid.txt").read().split()]
        name = "Ark"
        ing = self.d("ingots", name)
        sh([self.a.gen, "groth16", "--arkworks", f"{d}/verification_key.json", "--proof", f"{d}/proof.json", "--public", f"{d}/public.json",
            "--name", name, "--out", ing, "--fe-zk-path", os.path.relpath(ROOT, ing)])
        out = self.merge(name, ing)
        sig = "verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[2])"
        body = lambda w: "".join(h32(x) for x in w)
        f = Fam("groth16/arkworks-vs-ark-groth16", sig, None, body, body, valids, [(0, 1), (6, 7)], [2], [8, 9], ref_kind="ark", rerand_npub=2)
        f.ark_seed = seed
        f.n_fresh = count
        f.fe_bin = os.path.join(out, name + "Verifier.bin")
        return f

    # ---- gnark + halo2 through the gas-fe wrapper ingot (scratch copy with only the needed wrappers)
    def wrapper_ingot(self):
        d = os.path.join(self.w, "gas-fe")
        if os.path.isdir(d):
            shutil.rmtree(d)
        shutil.copytree(os.path.join(ROOT, "vectors", "gas-fe"), d)
        # Keep only the gnark and halo2 wrappers; other wrappers may come and go in gas-fe.
        for fn in ["fri_wrap.fe", "kzg_wrap.fe", "plonk_wrap.fe", "poseidon_wrap.fe"]:
            path = os.path.join(d, "src", fn)
            if os.path.exists(path):
                os.remove(path)
        tom = open(os.path.join(d, "fe.toml")).read().replace('path = "../.."', f'path = "{os.path.relpath(ROOT, d)}"')
        open(os.path.join(d, "fe.toml"), "w").write(tom)
        shutil.copy(os.path.join(ROOT, "src/tests/gnark_fixtures.fe"), os.path.join(d, "src/fx_gnark.fe"))
        t = open(os.path.join(ROOT, "src/verifier/halo2/standard_plonk_lookup.fe")).read()
        src = ["SRS_G2_X0", "SRS_G2_X1", "SRS_G2_Y0", "SRS_G2_Y1", "SRS_S_G2_X0", "SRS_S_G2_X1", "SRS_S_G2_Y0", "SRS_S_G2_Y1"]
        hx = [re.search(r"const " + n + r": u256 = (0x[0-9a-f]{64})", t).group(1) for n in src]
        names = ["H_G2_X0", "H_G2_X1", "H_G2_Y0", "H_G2_Y1", "H_S_X0", "H_S_X1", "H_S_Y0", "H_S_Y1"]
        with open(os.path.join(d, "src/fx_halo2.fe"), "w") as fo:
            for n, v in zip(names, hx):
                fo.write(f"pub const {n}: u256 = {v}\n")
        out = self.d("bin", "all")
        self.builds.append((d, out))
        return out

    @staticmethod
    def gnark_calldata(words, npub):
        pw, pubs = words[:-npub], words[-npub:]
        proof = "".join(h32(x) for x in pw)
        plen = len(proof) // 2
        pad = (-plen) % 32
        head = h32(0x40) + h32(0x40 + 32 + plen + pad)
        return head + h32(plen) + proof + "00" * pad + h32(len(pubs)) + "".join(h32(x) for x in pubs)

    def gnark(self, wrappers, fx, fe_name, fe_sig):
        d = read_json("vectors", "gnark", fx, "fixture.json")
        words = [int(x, 0) for x in d["proof_words"] + d["public"]]
        n = len(words)
        pairs = [(i, i + 1) for i in range(0, n - 2 - 1, 2)]
        f = Fam(f"plonk-gnark/{fx}", fe_sig, "Verify(bytes,uint256[])", lambda w: "".join(h32(x) for x in w),
                lambda w: self.gnark_calldata(w, 2), [words], pairs, [], [n - 2, n - 1])
        f.fe_bin = os.path.join(wrappers, fe_name + ".bin")
        f.ref_bin = self.solc("gnark_" + fx, os.path.join(ROOT, "vectors", "gnark", fx, "Verifier.sol"), "PlonkVerifier")
        return f

    def halo2(self, wrappers, kind, fe_name, nwords):
        d = read_json("vectors", "halo2", "standard_plonk_lookup", kind + ".json")
        proof = d["proof"][2:]
        words = [int(x, 0) for col in d["instances"] for x in col] + [int(proof[i:i + 64], 16) for i in range(0, len(proof), 64)]
        assert len(words) == nwords + 1
        pairs = [(i, i + 1) for i in range(1, nwords, 2)]
        body = lambda w: "".join(h32(x) for x in w)
        f = Fam(f"halo2-{kind}/standard_plonk_lookup", f"verify(uint256[1],uint256[{nwords}])", None, body, body, [words], pairs, [], [0])
        f.fe_bin = os.path.join(wrappers, fe_name + ".bin")
        f.ref_bin = self.solc("halo2_" + kind, os.path.join(ROOT, "vectors", "halo2", "standard_plonk_lookup", f"snark_verifier_{kind}.sol"),
                              "Halo2Verifier", patch=True, optimize=False)
        return f


def fe_build_all(a, builds):
    def one(b):
        ing, out = b
        r = subprocess.run([a.fe, "build", ing, "-O", a.fe_opt, "--out-dir", out], capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"fe build failed for {ing}:\n{r.stdout[-3000:]}\n{r.stderr[-3000:]}")
    with cf.ThreadPoolExecutor(max_workers=min(len(builds), os.cpu_count() or 4)) as ex:
        list(ex.map(one, builds))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--evm-batch", required=True)
    ap.add_argument("--ark-ref", required=True)
    ap.add_argument("--gen", required=True)
    ap.add_argument("--fe", default="fe")
    ap.add_argument("--fe-opt", default="1")
    ap.add_argument("--circom", default="circom")
    ap.add_argument("--node-modules", default=os.path.join(ROOT, "vectors", "js", "node_modules"))
    ap.add_argument("--ptau", default="")
    ap.add_argument("--quick", action="store_true", help="committed fixtures only, small mutation sets (CI smoke, < 3 min)")
    ap.add_argument("--seed", type=int, default=20261124)
    ap.add_argument("--fresh", type=int, default=60, help="fresh valid proofs per Groth16/PLONK circuit (membership: a third)")
    ap.add_argument("--random", type=int, default=1500, help="random-word mutations per family (full mode)")
    ap.add_argument("--rerand-n", type=int, default=10, help="re-randomised variants per Groth16 family")
    ap.add_argument("--quick-random", type=int, default=120)
    ap.add_argument("--only", default="", help="regex: run only families whose name matches")
    ap.add_argument("--json", default="")
    a = ap.parse_args()
    t0 = time.time()
    print(f"seed {a.seed}  mode {'quick' if a.quick else 'full'}", flush=True)
    p = Prep(a)
    fams = []
    wrappers = p.wrapper_ingot()
    jobs = []
    if a.quick:
        jobs += [lambda: p.groth16_fixture("multiplier", 1), lambda: p.groth16_fixture("multiplier2", 2),
                 lambda: p.groth16_fixture("membership", 3), lambda: p.plonk_fixture("multiplier2"),
                 lambda: p.plonk_fixture("membership"), lambda: p.arkworks(12, a.seed)]
    else:
        if not a.ptau:
            sys.exit("--ptau required in full mode")
        c = os.path.join(ROOT, "vectors", "circuits")
        mem = os.path.join(ROOT, "examples", "membership", "circuit", "membership.circom")
        n, m = a.fresh, max(a.fresh // 3, 8)
        jobs += [lambda: p.groth16_fresh("multiplier", f"{c}/multiplier.circom", n, a.seed),
                 lambda: p.groth16_fresh("multiplier2", f"{c}/multiplier2.circom", n, a.seed + 1),
                 lambda: p.groth16_fresh("membership", mem, m, a.seed + 2),
                 lambda: p.plonk_fresh("multiplier2", f"{c}/multiplier2.circom", n, a.seed + 3),
                 lambda: p.plonk_fresh("membership", mem, m, a.seed + 4),
                 lambda: p.groth16_fixture("multiplier", 1), lambda: p.plonk_fixture("multiplier2"),
                 lambda: p.arkworks(n, a.seed)]
    jobs += [lambda: p.gnark(wrappers, "mul", "GnarkMul", "verify(uint256[24],uint256[2])"),
             lambda: p.gnark(wrappers, "mulcommit", "GnarkMulcommit", "verify(uint256[24],uint256[1],uint256[2][1],uint256[2])"),
             lambda: p.halo2(wrappers, "shplonk", "Halo2Shplonk", 57), lambda: p.halo2(wrappers, "gwc", "Halo2Gwc", 61)]
    # prepare in parallel (provers, gen, solc), then build all Fe ingots in parallel
    with cf.ThreadPoolExecutor(max_workers=min(len(jobs), os.cpu_count() or 4)) as ex:
        fams = [f.result() for f in [ex.submit(j) for j in jobs]]
    if a.only:
        fams = [f for f in fams if re.search(a.only, f.name)]
    print(f"prepared {len(fams)} families in {time.time() - t0:.0f}s; building Fe ingots", flush=True)
    fe_build_all(a, p.builds)
    print(f"built in {time.time() - t0:.0f}s; running", flush=True)

    def one(i_f):
        i, f = i_f
        return test_family(a, f, random.Random(a.seed * 1000 + i))
    with cf.ThreadPoolExecutor(max_workers=min(len(fams), os.cpu_count() or 4)) as ex:
        results = list(ex.map(one, enumerate(fams)))

    # ---- report
    print()
    hdr = f"{'family':44} {'valid':>6} {'fresh':>6} {'mutations':>9} {'agree':>9} {'allowed':>8} {'bugs':>5} {'secs':>6}"
    print(hdr)
    print("-" * len(hdr))
    tot = [0] * 5
    for r in results:
        print(f"{r['family']:44} {r['valid']:>6} {r['fresh']:>6} {r['mut']:>9} {r['agree']:>9} {r['allowed']:>8} {len(r['bugs']):>5} {r['secs']:>6}")
        for k, v in zip(range(5), [r["valid"], r["mut"], r["agree"], r["allowed"], len(r["bugs"])]):
            tot[k] += v
    print("-" * len(hdr))
    print(f"{'total':44} {tot[0]:>6} {'':>6} {tot[1]:>9} {tot[2]:>9} {tot[3]:>8} {tot[4]:>5}")
    for r in results:
        for k, v in r["allowed_kinds"].items():
            print(f"  ALLOWED {r['family']}: {v} x {k}  [{', '.join(r['allowed_pos'].get(k, []))[:120]}]")
        for k, v in sorted(r["both_accept_mut"].items()):
            print(f"  note {r['family']}: both accept mutated input ({k}) x{v}")
        for b in r["bugs"][:10]:
            print(f"  BUG {r['family']}: {b['class']} kind={b['kind']} {b.get('note') or ''} fe={b['fe']} ref={b['ref']}")
            print(f"      fe_body={b['fe_body'][:200]}{'...' if len(b['fe_body']) > 200 else ''}")
        if len(r["bugs"]) > 10:
            print(f"  BUG {r['family']}: ... {len(r['bugs']) - 10} more")
    print(f"runtime {time.time() - t0:.0f}s")
    if a.json:
        json.dump({"seed": a.seed, "quick": a.quick, "runtime_s": round(time.time() - t0), "results": results}, open(a.json, "w"), indent=1)
    sys.exit(1 if tot[4] else 0)


if __name__ == "__main__":
    main()
