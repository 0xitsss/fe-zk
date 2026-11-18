#!/usr/bin/env python3
"""Measures Fe vs reference Solidity verifiers with gas-run (revm) and renders docs/GAS.md.

Called by scripts/gas-table.sh, which builds everything first. Inputs (all paths under --work):
  fe/<Contract>.bin                 wrapper contracts (vectors/gas-fe), creation bytecode
  g16/<fixture>/<Contract>.bin      generated Groth16 verifiers (fe-zk-gen)
  life/Life.bin                     examples/life contract
Every number written to the markdown is taken from gas-run output of this run.
"""
import argparse
import json
import os
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
EIP170 = 24576


def h32(x):
    return format(int(str(x), 0), "064x")


def read_json(*p):
    with open(os.path.join(ROOT, *p)) as f:
        return json.load(f)


class Ctx:
    def __init__(self, a):
        self.a = a
        self.tmp = os.path.join(a.work, "calls")
        os.makedirs(self.tmp, exist_ok=True)
        self.n = 0

    def write(self, text):
        self.n += 1
        p = os.path.join(self.tmp, f"{self.n}.txt")
        with open(p, "w") as f:
            f.write(text)
        return p

    def run(self, init_bin, calls):
        """calls: list of (name, 'call'|'iterate', spec). Returns the gas-run JSON."""
        cmd = [self.a.gas_run, "--init", init_bin]
        for name, kind, spec in calls:
            if kind == "call":
                cmd += ["--call", f"{name}={self.write(spec)}"]
            else:
                cmd += ["--iterate", f"{name}={spec}"]
        out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
        return json.loads(out)

    def solc(self, name, sol_path, contract, patch_pragma=False, optimize=True):
        d = os.path.join(self.a.work, "sol", name)
        os.makedirs(d, exist_ok=True)
        src = os.path.join(ROOT, sol_path)
        if patch_pragma:
            text = open(src).read()
            text = re.sub(r"pragma solidity 0\.8\.19;", "pragma solidity ^0.8.19;", text)
            src = os.path.join(d, "Patched.sol")
            open(src, "w").write(text)
        flags = ["--optimize", "--optimize-runs", "200"] if optimize else []
        r = subprocess.run(["solc", *flags, "--bin", "--overwrite", "-o", d, src], capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"solc failed for {sol_path}: {r.stderr}")
        return os.path.join(d, contract + ".bin")


def accepted(call):
    return call["ok"] and (call["ret"] == "" or call["ret"] == "0" * 63 + "1")


def measure(ctx, init_bin, sig, body_hex, tampered_hex, expect_bool=True):
    """Runs a valid call and a tampered call; returns a result dict. sig None = raw calldata."""
    pre = f"sig:{sig}\n" if sig else ""
    r = ctx.run(init_bin, [("valid", "call", pre + body_hex), ("tampered", "call", pre + tampered_hex)])
    v, t = r["calls"]
    assert accepted(v), f"valid proof not accepted: {init_bin} {v}"
    if expect_bool:
        assert v["ret"] == "0" * 63 + "1", f"valid proof did not return true: {init_bin} {v}"
    assert not (t["ok"] and (t["ret"] == "" or t["ret"] == "0" * 63 + "1")), f"tampered input accepted: {init_bin}"
    return {
        "exec": v["exec"], "tx": v["tx"], "calldata": v["calldata_bytes"],
        "code": r["deployed_code_bytes"], "deploy": r["deploy_gas"],
        "tampered": "reverts" if not t["ok"] else "returns false",
    }


def tamper(words, idx):
    w = list(words)
    w[idx] = h32(int(w[idx], 16) ^ 1)  # flip the lowest bit: stays a word, no longer the proven value
    return w


# ------------------------------------------------------------------ calldata per family

def groth16_words(fx):
    p = read_json("vectors", "groth16", fx, "proof.json")
    pub = read_json("vectors", "groth16", fx, "public.json")
    a, b, c = p["pi_a"], p["pi_b"], p["pi_c"]
    w = [a[0], a[1], b[0][1], b[0][0], b[1][1], b[1][0], c[0], c[1]] + pub
    return [h32(x) for x in w], len(pub)


def plonk_words(fx):
    pr = read_json("vectors", "plonk", fx, "proof.json")
    pub = read_json("vectors", "plonk", fx, "public.json")
    w = []
    for k in ["A", "B", "C", "Z", "T1", "T2", "T3", "Wxi", "Wxiw"]:
        w += [pr[k][0], pr[k][1]]
    for k in ["eval_a", "eval_b", "eval_c", "eval_s1", "eval_s2", "eval_zw"]:
        w.append(pr[k])
    return [h32(x) for x in w + pub], len(pub)


def gnark_fixture(fx):
    d = read_json("vectors", "gnark", fx, "fixture.json")
    return [h32(x) for x in d["proof_words"]], [h32(x) for x in d["public"]]


def gnark_solidity_calldata(proof_words, pubs):
    proof = "".join(proof_words)
    plen = len(proof) // 2
    pad = (-plen) % 32
    head = h32(0x40) + h32(0x40 + 32 + plen + pad)
    body = h32(plen) + proof + "00" * pad + h32(len(pubs)) + "".join(pubs)
    return head + body


def halo2_fixture(kind):
    d = read_json("vectors", "halo2", "standard_plonk_lookup", kind + ".json")
    proof = d["proof"][2:]
    inst = [h32(x) for col in d["instances"] for x in col]
    words = [proof[i:i + 64] for i in range(0, len(proof), 64)]
    return inst, words


def fri_words():
    text = open(os.path.join(ROOT, "src", "tests", "fri_fixture.fe")).read()
    body = text.split("pub fn fib_proof()")[1]
    body = re.sub(r"//.*", "", body)
    toks = re.findall(r"0x[0-9a-fA-F]+|(?<![\w])\d+", body)
    words = [h32(t) for t in toks]
    pub = re.search(r"pub const FIB_PUBLIC: \[u256; 3\] = \[([^\]]*)\]", text).group(1)
    return words, [h32(x.strip()) for x in pub.split(",")]


# ------------------------------------------------------------------ main

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--gas-run", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--meta", required=True, help="JSON with tool versions from gas-table.sh")
    ap.add_argument("--poseidon-ref", default="", help="dir with circomlibjs creation code files (poseidon_<n>.bin)")
    a = ap.parse_args()
    ctx = Ctx(a)
    meta = json.load(open(a.meta))
    fe = lambda n: os.path.join(a.work, "fe", n + ".bin")
    rows = []   # verifier table
    fe_only = []

    # ---- Groth16
    g16 = [
        ("multiplier", "G16MultiplierVerifier", "Groth16Verifier"),
        ("multiplier2", "G16Multiplier2Verifier", "Groth16Verifier"),
        ("membership", "G16MembershipVerifier", "Groth16Verifier"),
        ("life_512", "G16Life512Verifier", "Groth16Verifier"),
    ]
    g16_results = {}
    for fx, fe_name, sol_name in g16:
        w, n = groth16_words(fx)
        sig = f"verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[{n}])"
        body, tam = "".join(w), "".join(tamper(w, len(w) - 1))
        f = measure(ctx, os.path.join(a.work, "g16", fx, fe_name + ".bin"), sig, body, tam)
        s_bin = ctx.solc("g16_" + fx, f"vectors/groth16/{fx}/Verifier.sol", sol_name)
        s = measure(ctx, s_bin, sig, body, tam)
        g16_results[fx] = (f, s)
        rows.append(("Groth16 (snarkjs)", f"{fx}, N={n}", f, s))
    # life_64 is measured for the Life table only
    w, n = groth16_words("life_64")
    sig = f"verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[{n}])"
    body, tam = "".join(w), "".join(tamper(w, len(w) - 1))
    f = measure(ctx, os.path.join(a.work, "g16", "life_64", "G16Life64Verifier.bin"), sig, body, tam)
    s = measure(ctx, ctx.solc("g16_life_64", "vectors/groth16/life_64/Verifier.sol", "Groth16Verifier"), sig, body, tam)
    g16_results["life_64"] = (f, s)

    # ---- PLONK snarkjs
    for fx, fe_name in [("multiplier2", "PlonkMultiplier2"), ("membership", "PlonkMembership")]:
        w, n = plonk_words(fx)
        sig = f"verifyProof(uint256[24],uint256[{n}])"
        body, tam = "".join(w), "".join(tamper(w, len(w) - 1))
        f = measure(ctx, fe(fe_name), sig, body, tam)
        s = measure(ctx, ctx.solc("plonk_" + fx, f"vectors/plonk/{fx}/Verifier.sol", "PlonkVerifier"), sig, body, tam)
        rows.append(("PLONK (snarkjs)", f"{fx}, N={n}", f, s))

    # ---- PLONK gnark
    for fx, fe_name, fe_sig in [
        ("mul", "GnarkMul", "verify(uint256[24],uint256[2])"),
        ("mulcommit", "GnarkMulcommit", "verify(uint256[24],uint256[1],uint256[2][1],uint256[2])"),
    ]:
        pw, pubs = gnark_fixture(fx)
        fe_w = pw + pubs
        f = measure(ctx, fe(fe_name), fe_sig, "".join(fe_w), "".join(tamper(fe_w, len(fe_w) - 1)))
        sol_cd = gnark_solidity_calldata(pw, pubs)
        sol_tam = gnark_solidity_calldata(pw, tamper(pubs, len(pubs) - 1))
        s_bin = ctx.solc("gnark_" + fx, f"vectors/gnark/{fx}/Verifier.sol", "PlonkVerifier")
        s = measure(ctx, s_bin, "Verify(bytes,uint256[])", sol_cd, sol_tam, expect_bool=False)
        rows.append(("PLONK (gnark v0.12.0)", f"{fx}, N=2", f, s))

    # ---- halo2
    for kind, fe_name, nwords in [("shplonk", "Halo2Shplonk", 57), ("gwc", "Halo2Gwc", 61)]:
        inst, words = halo2_fixture(kind)
        assert len(words) == nwords
        fe_sig = f"verify(uint256[1],uint256[{nwords}])"
        raw = inst + words
        f = measure(ctx, fe(fe_name), fe_sig, "".join(raw), "".join(tamper(raw, 0)))
        s_bin = ctx.solc("halo2_" + kind, f"vectors/halo2/standard_plonk_lookup/snark_verifier_{kind}.sol",
                         "Halo2Verifier", patch_pragma=True, optimize=False)
        s = measure(ctx, s_bin, None, "".join(raw), "".join(tamper(raw, 0)), expect_bool=False)
        rows.append((f"halo2-KZG {kind.upper()} (snark-verifier)", "standard_plonk_lookup, N=1", f, s))

    # ---- halo2 SHPLONK with the library's own runtime-validated vk()
    inst, words = halo2_fixture("shplonk")
    raw = inst + words
    libvk = measure(ctx, fe("Halo2ShplonkLibVk"), "verify(uint256[1],uint256[57])", "".join(raw), "".join(tamper(raw, 0)))
    shplonk_row = [r for r in rows if r[0].startswith("halo2-KZG SHPLONK")][0][2]

    # ---- KZG (Fe only)
    k = read_json("vectors", "kzg", "fixtures", "kzg.json")
    sg, gw, sh = k["single"], k["gwc"], k["shplonk"]
    single = [*sg["c"], sg["z"], sg["y"], *sg["pi"]]
    kzg_cases = [
        ("KzgSingle", "verify(uint256[2],uint256,uint256,uint256[2])", single, "single opening"),
        ("KzgGwc", "verify(uint256[2][3],uint256,uint256[3],uint256[2])",
         [x for c in gw["cs"] for x in c] + [gw["z"], *gw["ys"], *gw["pi"]], "GWC, 3 polynomials at 1 point"),
        ("KzgShplonk", "verify(uint256[2][3],uint256,uint256,uint256[4],uint256[2],uint256[2])",
         [x for c in sh["cs"] for x in c] + [sh["z1"], sh["z2"], sh["e00"], sh["e01"], sh["e1"], sh["e2"], *sh["h"], *sh["pi"]],
         "SHPLONK, 3 polynomials at 2 points"),
    ]
    for name, sig, vals, label in kzg_cases:
        w = [h32(x) for x in vals]
        # tamper the claimed evaluation z/y words: use the last scalar word before the proof points
        idx = {"KzgSingle": 3, "KzgGwc": 7, "KzgShplonk": 9}[name]
        f = measure(ctx, fe(name), sig, "".join(w), "".join(tamper(w, idx)))
        fe_only.append(("KZG", label, f, "no Solidity reference"))

    # ---- FRI (Fe only)
    words, pub = fri_words()
    assert len(words) == 217, len(words)
    fw = words + pub
    f = measure(ctx, fe("FriFib"), "verify(uint256[217],uint256[3])", "".join(fw), "".join(tamper(fw, len(fw) - 1)))
    fe_only.append(("FRI/STARK", "Plonky3 Fibonacci, KoalaBear, 4 queries (log2 trace 4)", f, "no EVM reference verifier exists"))

    # ---- Poseidon: Fe vs circomlibjs gencontract (optional)
    pos = []
    for nin, fe_name in [(2, "Poseidon2In"), (3, "Poseidon3In")]:
        vals = [h32(i + 1) for i in range(nin)]
        sig = f"poseidon(uint256[{nin}])"
        rf = ctx.run(fe(fe_name), [("h", "call", f"sig:{sig}\n" + "".join(vals))])
        fc = rf["calls"][0]
        assert fc["ok"] and len(fc["ret"]) == 64
        fres = {"exec": fc["exec"], "tx": fc["tx"], "calldata": fc["calldata_bytes"],
                "code": rf["deployed_code_bytes"], "deploy": rf["deploy_gas"]}
        sres = None
        ref_bin = os.path.join(a.poseidon_ref, f"poseidon_{nin}.bin") if a.poseidon_ref else ""
        if ref_bin and os.path.exists(ref_bin):
            rs = ctx.run(ref_bin, [("h", "call", f"sig:{sig}\n" + "".join(vals))])
            sc = rs["calls"][0]
            assert sc["ok"] and sc["ret"] == fc["ret"], f"Poseidon mismatch Fe {fc['ret']} vs circomlibjs {sc['ret']}"
            sres = {"exec": sc["exec"], "tx": sc["tx"], "calldata": sc["calldata_bytes"],
                    "code": rs["deployed_code_bytes"], "deploy": rs["deploy_gas"]}
        pos.append((nin, fres, sres))

    # ---- Life
    start = "0000000000000000000000000100018003000000000000000000000000000000"
    life_bin = os.path.join(a.work, "life", "Life.bin")
    life = {}
    for ngen in (64, 512):
        fx = f"life_{ngen}"
        boards = read_json("vectors", "groth16", fx, "boards.json")
        r = ctx.run(life_bin, [
            ("steps", "iterate", f"step(uint256):{start}:{ngen}"),
            ("run", "call", f"sig:run(uint256,uint256)\n{start}{h32(ngen)}"),
        ])
        steps, run = r["calls"]
        end = boards["end"][2:].rjust(64, "0")
        assert steps["ret"] == end, f"N={ngen} step() chain does not reach the fixture end board"
        assert run["ret"] == end, f"N={ngen} run() does not reach the fixture end board"
        life[ngen] = {"steps": steps, "run": run, "code": r["deployed_code_bytes"], "deploy": r["deploy_gas"]}
    r1 = ctx.run(life_bin, [("one", "iterate", f"step(uint256):{start}:1")])
    life["step1"] = r1["calls"][0]

    # ------------------------------------------------------------------ render
    def fmt(x):
        return f"{x:,}".replace(",", "&#8239;") if False else f"{x:,}"

    def ratio(f, s):
        return f"{f / s:.2f}x"

    out = []
    P = out.append
    P("# Gas: fe-zk (Fe) vs reference Solidity verifiers\n")
    P("<!-- GENERATED by scripts/gas-table.sh (vectors/gas/gas_table.py). Do not edit by hand: rerun the script. -->\n")
    P("Every verifier below is deployed as a contract in the same in-memory revm and called through one transaction with the")
    P("same proof and the same words of calldata the reference verifier expects. Nothing in this file is hand-copied.\n")
    P("## Method\n")
    P(f"- EVM: revm {meta['revm']} (`gas-run`, `vectors/gas/`), spec `OSAKA` (Prague precompile gas schedule, EIP-7623 calldata floor, EIP-2929 access lists).")
    P("- One transaction per number: caller is an EOA with a funded balance, `gas_price = 0`, gas limit 10^9, block fresh. The target contract and the precompiles are warm from the start of the transaction, storage slots are cold. Contract code size limits are lifted so that over-sized contracts still deploy; sizes are reported and flagged.")
    P("- **exec**: gas spent by the top-level call frame (the contract's own execution including every precompile and sub-call; it excludes the 21,000 intrinsic cost and the calldata cost). Measured with an inspector on the frame, so it is exact even when the EIP-7623 floor decides the receipt.")
    P("- **tx**: `gas_used` of the receipt: 21,000 + calldata (4 gas per zero byte, 16 per non-zero byte) + exec, raised to the EIP-7623 floor when that is larger. This is what a user pays for the call (before any refund; there are none here).")
    P("- Every verifier is also run on a copy of the same calldata with one input word changed; the script asserts it is not accepted (revert or `false`) before it reports a number.")
    P(f"- Fe: `{meta['fe']}`, build `fe build -O {meta['fe_opt']}` (backend sonatina), `fe_zk` at commit `{meta['commit']}`. Groth16 verifiers are the output of `fe-zk-gen groth16 --snarkjs` ({meta['gen']}); other Fe rows are the tiny wrapper contracts in `vectors/gas-fe/` that call the `fe_zk` library verifier with the fixture's constants (they add one ABI decode, no other logic).")
    P(f"- Solidity: `{meta['solc']}`, `--optimize --optimize-runs 200`, default EVM version, the committed `Verifier.sol` of each fixture (snarkjs 0.7.6 `zkey export solidityverifier`, gnark v0.12.0 `ExportSolidity`, snark-verifier 0.2.7 for halo2). Exception: the snark-verifier (halo2) verifiers are compiled **without the optimizer**: with `--optimize` solc 0.8.24 fails with \"Stack too deep\", and with `--via-ir` the contract's own `mload(0x40) == 0x80` memory-layout check reverts. They pin `pragma solidity 0.8.19`; the script relaxes the pragma to `^0.8.19` in a scratch copy (no code change).")
    P("- Deployed code size is the size of the runtime code stored on chain. EIP-170 limits it to 24,576 bytes.\n")

    P("## Verifier gas, same proof\n")
    P("| Verifier | Fixture | Fe exec | Solidity exec | Fe / Sol exec | Fe tx | Solidity tx | Fe / Sol tx |")
    P("|---|---|---:|---:|---:|---:|---:|---:|")
    for fam, fx, f, s in rows:
        P(f"| {fam} | {fx} | {fmt(f['exec'])} | {fmt(s['exec'])} | {ratio(f['exec'], s['exec'])} | {fmt(f['tx'])} | {fmt(s['tx'])} | {ratio(f['tx'], s['tx'])} |")
    P("")
    P("Calldata sizes (bytes), Fe / Solidity: " + "; ".join(f"{fam} {fx.split(',')[0]}: {f['calldata']}/{s['calldata']}" for fam, fx, f, s in rows) + ".\n")

    P("## Fe only (no Solidity reference)\n")
    P("| Verifier | Case | exec | tx | Note |")
    P("|---|---|---:|---:|---|")
    for fam, label, f, note in fe_only:
        P(f"| {fam} | {label} | {fmt(f['exec'])} | {fmt(f['tx'])} | {note} |")
    P("")
    P("The KZG rows check opening proofs over the test-only SRS of `vectors/kzg` (arkworks 0.5.0 fixtures); there is no widely used Solidity KZG opening verifier with a fixed interface to compare against, so no reference is invented. The FRI row verifies a Plonky3 `p3-uni-stark` 0.4.2 proof of a Fibonacci AIR; no EVM verifier for Plonky3 proofs exists in any language to compare with.\n")

    P("## Poseidon hash (BN254, circomlib-compatible)\n")
    if all(s is not None for _, _, s in pos):
        P(f"Reference: circomlibjs {meta['circomlibjs']} `poseidon_gencontract` (hand-written EVM bytecode, the standard on-chain Poseidon for circom circuits). Both contracts return the same digest (asserted).\n")
        P("| Inputs | Fe exec | circomlibjs exec | Fe / ref exec | Fe tx | ref tx | Fe code B | ref code B |")
        P("|---|---:|---:|---:|---:|---:|---:|---:|")
        for nin, f, s in pos:
            P(f"| {nin} (t = {nin + 1}) | {fmt(f['exec'])} | {fmt(s['exec'])} | {ratio(f['exec'], s['exec'])} | {fmt(f['tx'])} | {fmt(s['tx'])} | {fmt(f['code'])} | {fmt(s['code'])} |")
    else:
        P("The reference column was skipped: circomlibjs is not installed (run `npm ci` in `vectors/js`, or set `NODE_MODULES_DIR`). Fe-only numbers:\n")
        P("| Inputs | Fe exec | Fe tx | Fe code B |")
        P("|---|---:|---:|---:|")
        for nin, f, _ in pos:
            P(f"| {nin} (t = {nin + 1}) | {fmt(f['exec'])} | {fmt(f['tx'])} | {fmt(f['code'])} |")
    P("")

    P("## Game of Life: on-chain stepping vs one proof\n")
    P("`examples/life` `Life` contract: `step(board)` is one transaction per generation (calldata 36 bytes); `run(board, N)` computes N generations in one call. The proof rows verify a Groth16 proof (2 public inputs: Poseidon commitments of the start and end board) that N generations were computed correctly; verifier is the `fe-zk-gen` output for the fixture `vectors/groth16/life_N`. The script asserts that the N-step chain and `run` both reach the end board committed in the fixture.\n")
    P("| N generations | N x `step()` exec | N x `step()` tx | `run(board, N)` exec | `run` tx | Groth16 verify, Fe exec | Fe tx | Groth16 verify, Solidity exec | Solidity tx |")
    P("|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for n in (64, 512):
        l = life[n]
        f, s = g16_results[f"life_{n}"]
        P(f"| {n} | {fmt(l['steps']['exec'])} | {fmt(l['steps']['tx'])} | {fmt(l['run']['exec'])} | {fmt(l['run']['tx'])} | {fmt(f['exec'])} | {fmt(f['tx'])} | {fmt(s['exec'])} | {fmt(s['tx'])} |")
    P("")
    P(f"One `step()` call: exec {fmt(life['step1']['exec'])}, tx {fmt(life['step1']['tx'])}. Deployed `Life` code: {fmt(life[64]['code'])} bytes.\n")

    P("## Deployed code size and deployment gas\n")
    P("| Contract | Fe bytes | Solidity bytes | Fe deploy gas | Solidity deploy gas |")
    P("|---|---:|---:|---:|---:|")
    for fam, fx, f, s in rows:
        flag = " (over EIP-170)" if f["code"] > EIP170 else ""
        flag2 = " (over EIP-170)" if s["code"] > EIP170 else ""
        P(f"| {fam}, {fx} | {fmt(f['code'])}{flag} | {fmt(s['code'])}{flag2} | {fmt(f['deploy'])} | {fmt(s['deploy'])} |")
    for fam, label, f, _ in fe_only:
        flag = " (over EIP-170)" if f["code"] > EIP170 else ""
        P(f"| {fam}, {label} | {fmt(f['code'])}{flag} | n/a | {fmt(f['deploy'])} | n/a |")
    P("")
    P("Fe bytes include the verifying-key constants and, for the generated Groth16 contracts, nothing else but the call into the library; Solidity bytes are the compiler output of the reference verifier (key constants included).\n")

    # ---- analysis (numbers computed from the data above)
    g_ratios = [f["exec"] / s["exec"] for fam, _, f, s in rows if fam.startswith("Groth16")]
    p_ratios = [f["exec"] / s["exec"] for fam, _, f, s in rows if fam.startswith("PLONK (snarkjs")]
    gn_ratios = [f["exec"] / s["exec"] for fam, _, f, s in rows if fam.startswith("PLONK (gnark")]
    h_ratios = [f["exec"] / s["exec"] for fam, _, f, s in rows if fam.startswith("halo2")]
    rng = lambda r: f"{min(r):.2f}x to {max(r):.2f}x"
    P("## Analysis\n")
    P(f"- Fe / Solidity execution gas: Groth16 {rng(g_ratios)}; snarkjs PLONK {rng(p_ratios)}; gnark PLONK {rng(gn_ratios)}; halo2-KZG {rng(h_ratios)}.")
    P("- Where the cost is: every one of these verifiers is dominated by the EC precompiles (ecAdd 0x06, ecMul 0x07, ecPairing 0x08) and modexp (0x05), priced identically for both languages. What differs is the work around each precompile call. Hand-optimised Solidity/Yul verifiers keep operands in scratch memory and fuse the field arithmetic. The Fe library calls the precompiles through typed wrappers (`G1`, `G2`, `Bn254Fr`): each call allocates a fixed buffer (a per-call cost measured in `docs/DESIGN.md`, \"Precompile wrapper gas\"), re-checks success and returned length (safety rule 6), and the scalar-field code works on checked canonical elements. Groth16 makes few calls (one `ecMul` and `ecAdd` per input and one 4-pair pairing), so it is closest to Solidity; the PLONK-family verifiers make dozens of calls and sit further above.")
    P(f"- Verifying keys are compile-time constants: the generated Groth16 contract validates its key (on curve, G2 subgroup) during compilation with `static_assert`, which costs no runtime gas. The wrappers of the other families do the same for every key point. This matters: the library's own `halo2::standard_plonk_lookup::vk()` builds its two G2 points with `G2::from_coords`, which runs the subgroup check at every call. A contract using it as is costs {fmt(libvk['exec'])} exec gas for the SHPLONK proof instead of {fmt(shplonk_row['exec'])}, an extra {fmt(libvk['exec'] - shplonk_row['exec'])} gas for the key check alone (the contract `Halo2ShplonkLibVk` exists only for this comparison). `PlonkVk::new` and `GnarkVk::new` also run their (cheap) root-of-unity checks at call time; they are included in the PLONK numbers.")
    P("- Poseidon: one generic implementation over the `Field` trait is unrolled into straight-line code at compile time; against hand-written bytecode it uses less execution gas but more code (see the size table).")
    P("- The checks that Fe adds by construction (canonical inputs, curve membership of every proof point, pairing output exactly 1, fixed input count) are part of the numbers above. A reference verifier may check less: for example snarkjs' Groth16 verifier range-checks the public inputs and leaves point validation to the precompiles. The gas gap is therefore not purely overhead.")
    P("- Where Fe is more expensive for a structural reason: `std` only exposes `Call.raw_staticcall` on a heap `MemBuffer`, so each precompile call pays for a buffer. A `raw_staticcall` taking a pointer and length would remove most of that per-call allocation (`docs/DESIGN.md`).")
    code_ratios = [f["code"] / s["code"] for _, _, f, s in rows]
    over = [f"{fam} {fx}" for fam, fx, f, s in rows if f["code"] > EIP170] + [f"{fam}" for fam, _, f, _ in fe_only if f["code"] > EIP170]
    P(f"- Code size: Fe runtime code is {min(code_ratios):.2f}x to {max(code_ratios):.2f}x the size of the Solidity verifier. Over the EIP-170 limit of {EIP170:,} bytes (not deployable on mainnet as is; revm limits were lifted for this table): {', '.join(over) if over else 'none'}. Contracts at or near the limit leave no room for application logic and would have to be split or shrunk before a real deployment.")
    l64, l512 = life[64], life[512]
    slope = (l512["run"]["tx"] - l64["run"]["tx"]) / (512 - 64)
    icept = l64["run"]["tx"] - slope * 64
    f64 = g16_results["life_512"][0]
    be_run = (f64["tx"] - icept) / slope
    be_step = f64["tx"] / life["step1"]["tx"]
    P(f"- Life: on-chain cost grows linearly with N, the proof verification does not. With one `step()` transaction per generation ({fmt(life['step1']['tx'])} tx gas each) the Fe-verified proof is cheaper from about N = {be_step:.0f}; with a single `run(board, N)` call (about {slope:,.0f} gas per generation plus {icept:,.0f} fixed, fitted on the two measured N) it is cheaper from about N = {be_run:.0f}, with the Fe verifier ({fmt(f64['tx'])} tx gas). Proof generation (off chain, minutes) is not included.")
    P("- Limits of this comparison: one fixture per row (gas varies by a few hundred with calldata zero bytes), a synthetic EOA caller (no `msg.value`, no access list), revm spec OSAKA rather than a live chain, and the Fe wrappers' ABI decoding of static arrays is part of Fe's number (the Solidity verifiers decode with `calldataload` directly).\n")

    P("## Reproduce\n")
    P("```sh")
    P("scripts/gas-table.sh        # builds Fe + Solidity, runs everything, rewrites docs/GAS.md")
    P("```")
    P("Needs: `fe` (see README for the pinned version), a Rust toolchain, `solc` 0.8.24, `python3`; optional: `node` and `npm ci` in `vectors/js` for the circomlibjs Poseidon reference (set `NODE_MODULES_DIR` to reuse an existing `node_modules`). Scratch and cargo target directories default to `$GAS_WORK` (a temp directory); set `FE` to choose the `fe` binary and `FE_OPT` the optimisation level. The measuring tool is `vectors/gas` (`gas-run`).")
    P("")
    with open(a.out, "w") as f:
        f.write("\n".join(out))


main()
