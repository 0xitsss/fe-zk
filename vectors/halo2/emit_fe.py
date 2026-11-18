#!/usr/bin/env python3
"""fe-zk halo2-KZG verifier generator: `vk.json` -> circuit-specific Fe verifier module.

    python3 vectors/halo2/emit_fe.py vectors/halo2/standard_plonk_lookup/vk.json \
        --module standard_plonk_lookup --out src/verifier/halo2/standard_plonk_lookup.fe
    python3 vectors/halo2/emit_fe.py --fixtures vectors/halo2/standard_plonk_lookup \
        --out src/tests/halo2_fixture.fe

The output is deterministic (no timestamps), carries the tool version and the SHA-256 of the
canonical vk JSON, and is passed through `fe fmt` when `fe` is on PATH (the committed file is
the formatted one). Everything structural is unrolled exactly as SPEC.md section 13 prescribes:
gate / lookup expressions, permutation sets, the SHPLONK rotation sets (`zdiff` products,
Lagrange coefficients) and the GWC point groups. The shared, circuit-independent code is
`src/verifier/halo2/common.fe`.

Scope (same as SPEC.md): BN254 KZG, one phase and no challenges, instances not committed.
Unsupported inputs are rejected here, never silently emitted. Re-check of the safety rules is
by construction: every point read is `read_g1` (canonical, on curve, no infinity), every scalar
`Bn254Fr::from_canonical`, all instances canonical, proof length fixed by the array type.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys

TOOL = "fe-zk-gen 0.1.0"
R = 21888242871839275222246405745257275088548364400416034343698204186575808495617
Q = 21888242871839275222246405745257275088696311157297823662689037894645226208583


def h(v):
    return "0x%064x" % v


def hx(s):
    return int(s, 16)


def inv(a):
    return pow(a % R, -1, R)


class Gen:
    def __init__(self, vk, module):
        self.vk = vk
        self.module = module
        self.lines = []
        self.omega = hx(vk["omega"])
        self.omega_inv = inv(self.omega)
        self.k = vk["k"]
        self.n = 1 << self.k
        self.bf = vk["blinding_factors"]
        self.check_vk()

    # ------------------------------------------------------------------ vk validation
    def check_vk(self):
        vk = self.vk
        if vk.get("num_challenges", 0) != 0:
            sys.exit("unsupported: challenges / multiple phases")
        assert pow(self.omega, self.n, R) == 1 and pow(self.omega, self.n // 2, R) != 1, "omega"
        assert hx(vk["n_inv"]) * self.n % R == 1, "n_inv"
        for q in vk["advice_queries"] + vk["fixed_queries"] + vk["instance_queries"]:
            assert -self.n < q["rotation"] < self.n
        assert vk["cs_degree"] - 1 == vk["quotient_chunks"], "quotient_chunks"
        pts = []
        for c in vk["fixed_commitments"] + vk["permutation"]["commitments"]:
            x, y = hx(c["x"]), hx(c["y"])
            assert x < Q and y < Q and (x, y) != (0, 0)
            assert (y * y - x * x * x - 3) % Q == 0, "vk G1 point not on curve"
            pts.append((x, y))
        used = {q["column"] for q in vk["fixed_queries"]}
        used_pts = [(hx(vk["fixed_commitments"][c]["x"]), hx(vk["fixed_commitments"][c]["y"])) for c in sorted(used)]
        used_pts += [(hx(c["x"]), hx(c["y"])) for c in vk["permutation"]["commitments"]]
        # SPEC section 7: halo2 merges byte-identical commitments; not implemented here.
        assert len(set(used_pts)) == len(used_pts), "duplicate fixed/sigma commitments: dedupe not implemented"
        assert len(vk["instance_lens"]) == vk["num_instance_columns"]

    def w(self, r):
        return pow(self.omega, r, R) if r >= 0 else pow(self.omega_inv, -r, R)

    def emit(self, s=""):
        self.lines.append(s)

    # ------------------------------------------------------------------ expressions
    def ex(self, e):
        op = e["op"]
        if op == "const":
            return "fr(%s)" % h(hx(e["value"]))
        if op == "fixed":
            return "f%d" % e["query"]
        if op == "advice":
            return "a%d" % e["query"]
        if op == "instance":
            return "ie%d" % e["query"]
        if op == "neg":
            return "%s.neg()" % self.ex(e["a"])
        if op == "add":
            return "%s.add(%s)" % (self.ex(e["a"]), self.ex(e["b"]))
        if op == "mul":
            return "%s.mul(%s)" % (self.ex(e["a"]), self.ex(e["b"]))
        if op == "scale":
            return "%s.mul(fr(%s))" % (self.ex(e["a"]), h(hx(e["scalar"])))
        sys.exit("unsupported expression op %r" % op)

    # ------------------------------------------------------------------ layout
    def layout(self):
        vk = self.vk
        A = vk["num_advice_columns"]
        L = len(vk["lookups"])
        perm_cols = vk["permutation"]["columns"]
        self.chunk = vk["permutation"]["chunk_len"]
        self.S = (len(perm_cols) + self.chunk - 1) // self.chunk
        self.P = len(perm_cols)
        self.H = vk["quotient_chunks"]
        self.A, self.L = A, L
        self.Aq, self.Fq = len(vk["advice_queries"]), len(vk["fixed_queries"])
        self.NI = sum(vk["instance_lens"])
        # prover commitments in proof order
        pts = ["adv%d" % c for c in range(A)]
        for j in range(L):
            pts += ["lpi%d" % j, "lpt%d" % j]
        pts += ["zc%d" % i for i in range(self.S)]
        pts += ["lzc%d" % j for j in range(L)]
        pts += ["rnd"]
        pts += ["hq%d" % i for i in range(self.H)]
        self.pt_names = pts
        # evaluations in proof order
        ev = ["a%d" % q for q in range(self.Aq)] + ["f%d" % q for q in range(self.Fq)] + ["rand_eval"]
        ev += ["sg%d" % p for p in range(self.P)]
        for i in range(self.S):
            ev += ["pz%d" % i, "pzn%d" % i]
            if i < self.S - 1:
                ev += ["pzl%d" % i]
        for j in range(L):
            ev += ["lz%d" % j, "lzn%d" % j, "la%d" % j, "lai%d" % j, "ls%d" % j]
        self.ev_names = ev
        self.common_words = 2 * len(pts) + len(ev)

    # ------------------------------------------------------------------ query list (SPEC 7)
    def queries(self):
        vk = self.vk
        qs = []  # (commitment name, rotation, eval name)
        for q, aq in enumerate(vk["advice_queries"]):
            qs.append(("adv%d" % aq["column"], aq["rotation"], "a%d" % q))
        for i in range(self.S):
            qs.append(("zc%d" % i, 0, "pz%d" % i))
            qs.append(("zc%d" % i, 1, "pzn%d" % i))
        for i in range(self.S - 2, -1, -1):
            qs.append(("zc%d" % i, -(self.bf + 1), "pzl%d" % i))
        for j in range(self.L):
            qs.append(("lzc%d" % j, 0, "lz%d" % j))
            qs.append(("lpi%d" % j, 0, "la%d" % j))
            qs.append(("lpt%d" % j, 0, "ls%d" % j))
            qs.append(("lpi%d" % j, -1, "lai%d" % j))
            qs.append(("lzc%d" % j, 1, "lzn%d" % j))
        for q, fq in enumerate(vk["fixed_queries"]):
            qs.append(("fx%d" % fq["column"], fq["rotation"], "f%d" % q))
        for p in range(self.P):
            qs.append(("sgc%d" % p, 0, "sg%d" % p))
        qs.append(("hc", 0, "expected_h"))
        qs.append(("rnd", 0, "rand_eval"))
        return qs

    @staticmethod
    def rname(r):
        return "%d" % r if r >= 0 else "m%d" % -r

    def unwrap(self, name, expr, ret):
        self.emit("let %s = match %s {" % (name, expr))
        self.emit("Option::Some(v) => v,")
        self.emit("Option::None => {")
        self.emit("return %s" % ret)
        self.emit("},")
        self.emit("}")

    # ------------------------------------------------------------------ module header
    def header(self):
        vk = self.vk
        digest = hashlib.sha256(json.dumps(vk, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        self.digest = digest
        self.emit("//! GENERATED FILE, do not edit. Tool: %s." % TOOL)
        self.emit("//! vk digest (sha256 of canonical vk.json): %s" % digest)
        self.emit("//! Circuit: k=%d, %d advice / %d fixed / %d instance columns, %d lookup(s), %d permutation set(s)," % (
            self.k, self.A, vk["num_fixed_columns"], vk["num_instance_columns"], self.L, self.S))
        self.emit("//! %d quotient chunks. halo2-KZG verifier for BN254 with the snark-verifier EvmTranscript." % self.H)
        self.emit("//! Algorithm and field order: vectors/halo2/SPEC.md. Shared code: halo2/common.fe.")
        self.emit("//!")
        self.emit("//! Safety rules: 1 (every proof scalar / instance `from_canonical`, every coordinate canonical),")
        self.emit("//! 2 (every proof point `read_g1`: on curve, no infinity), 3 (SRS and key points are compile-time")
        self.emit("//! constants checked by `static_assert`: canonical, on curve, G2 in the subgroup; no runtime gas),")
        self.emit("//! 4 (proof and instance lengths are array types),")
        self.emit("//! 5 (transcript absorbs the vk transcript_repr, the instances and every commitment and")
        self.emit("//! evaluation in reference order), 6 (precompile wrappers check success, length, result).")
        self.emit("use std::evm::Call")
        self.emit("use ingot::ec::{G1, G2, g1_is_valid, g1_mul, g2_is_valid}")
        self.emit("use ingot::field::{Bn254Fr, Field}")
        self.emit("use super::common::{")
        self.emit("Halo2Transcript, Halo2Vk, batch_inv, final_pairing, fr, mul_add, read_g1, read_points,\nread_scalars,")
        self.emit("}")
        self.emit()
        self.emit("/// Number of 32-byte words of a proof for this circuit (fixed by the vk, rule 4).")
        self.emit("pub const COMMON_WORDS: usize = %d" % self.common_words)
        self.emit("pub const NUM_INSTANCES: usize = %d" % self.NI)
        self.emit("pub const TRANSCRIPT_REPR: u256 = %s" % h(hx(vk["transcript_repr"])))
        self.emit("pub const VK_DIGEST: u256 = 0x%s" % digest)
        self.emit()
        self.emit("const SCALAR_R: u256 = 21888242871839275222246405745257275088548364400416034343698204186575808495617")
        self.emit("// SRS and key constants, validated by fe-zk-gen and re-validated by the Fe compiler below.")
        self.emit("// G2 coordinates are (real, imaginary) parts: x0 + x1*u, y0 + y1*u.")
        kz = vk["kzg"]
        self.emit("const SRS_G1_X: u256 = %s" % h(hx(kz["g1"]["x"])))
        self.emit("const SRS_G1_Y: u256 = %s" % h(hx(kz["g1"]["y"])))
        for nm, key in (("SRS_G2", "g2"), ("SRS_S_G2", "s_g2")):
            for part, v in zip(("X0", "X1", "Y0", "Y1"), kz[key]["x"] + kz[key]["y"]):
                self.emit("const %s_%s: u256 = %s" % (nm, part, h(hx(v))))
        used_f = sorted({q["column"] for q in vk["fixed_queries"]})
        for c in used_f:
            f = vk["fixed_commitments"][c]
            self.emit("const FX%d_X: u256 = %s" % (c, h(hx(f["x"]))))
            self.emit("const FX%d_Y: u256 = %s" % (c, h(hx(f["y"]))))
        for p, c in enumerate(vk["permutation"]["commitments"]):
            self.emit("const SG%d_X: u256 = %s" % (p, h(hx(c["x"]))))
            self.emit("const SG%d_Y: u256 = %s" % (p, h(hx(c["y"]))))
        self.emit()
        self.emit("// Compile-time validation (Fe CTFE): a key with an invalid point does not compile. Costs no gas;")
        self.emit("// the G2 subgroup checks alone are ~750k gas each at runtime. Fixed and sigma commitments may")
        self.emit("// not be the point at infinity (halo2's EvmTranscript rejects it).")
        self.emit("static_assert(TRANSCRIPT_REPR < SCALAR_R)")
        self.emit("static_assert(g1_is_valid(SRS_G1_X, SRS_G1_Y))")
        self.emit("static_assert(!SRS_G1.is_infinity())")
        for nm in ("SRS_G2", "SRS_S_G2"):
            self.emit("static_assert(g2_is_valid(%s_X0, %s_X1, %s_Y0, %s_Y1))" % ((nm,) * 4))
            self.emit("static_assert(!%s.is_infinity())" % nm)
        for c in used_f:
            self.emit("static_assert(g1_is_valid(FX%d_X, FX%d_Y))" % (c, c))
            self.emit("static_assert(!VK_FX%d.is_infinity())" % c)
        for p in range(len(vk["permutation"]["commitments"])):
            self.emit("static_assert(g1_is_valid(SG%d_X, SG%d_Y))" % (p, p))
            self.emit("static_assert(!VK_SG%d.is_infinity())" % p)
        self.emit()
        self.emit("// Typed points. `from_coords_or_infinity` yields infinity for an invalid point, so an unchecked")
        self.emit("// point cannot exist even if the assertions above were removed.")
        self.emit("const SRS_G1: G1 = G1::from_coords_or_infinity(SRS_G1_X, SRS_G1_Y)")
        for nm in ("SRS_G2", "SRS_S_G2"):
            self.emit("const %s: G2 = G2::from_coords_or_infinity(%s_X0, %s_X1, %s_Y0, %s_Y1)" % ((nm,) * 5))
        for c in used_f:
            self.emit("const VK_FX%d: G1 = G1::from_coords_or_infinity(FX%d_X, FX%d_Y)" % (c, c, c))
        for p in range(len(vk["permutation"]["commitments"])):
            self.emit("const VK_SG%d: G1 = G1::from_coords_or_infinity(SG%d_X, SG%d_Y)" % (p, p, p))
        self.emit()
        self.emit("/// SRS and transcript constants (compile-time validated, no gas for validation).")
        self.emit("pub fn vk() -> Halo2Vk {")
        self.emit("Halo2Vk::from_consts(fr(TRANSCRIPT_REPR), SRS_G1, SRS_G2, SRS_S_G2)")
        self.emit("}")
        self.emit()

    # ------------------------------------------------------------------ phase 1
    def phase1(self):
        vk = self.vk
        self.emit("/// Everything the opening step needs, after the evaluation at `x` has been derived.")
        self.emit("pub struct Phase1 {")
        self.emit("pub theta: Bn254Fr,")
        self.emit("pub beta: Bn254Fr,")
        self.emit("pub gamma: Bn254Fr,")
        self.emit("pub y: Bn254Fr,")
        self.emit("pub x: Bn254Fr,")
        self.emit("pub xn: Bn254Fr,")
        self.emit("pub expected_h: Bn254Fr,")
        self.emit("/// Prover commitments in proof order: %s." % ", ".join(self.pt_names))
        self.emit("pub pts: [G1; %d]," % len(self.pt_names))
        self.emit("/// Evaluations in proof order: %s." % ", ".join(self.ev_names))
        self.emit("pub ev: [Bn254Fr; %d]," % len(self.ev_names))
        self.emit("}")
        self.emit()
        self.emit("/// Reads and validates the common part of the proof, derives theta, beta, gamma, y, x and the")
        self.emit("/// value `expected_h` of the quotient polynomial at `x` (SPEC sections 2-6). `None` = reject.")
        self.emit("pub fn phase1<const L: usize>(")
        self.emit("_ vk: Halo2Vk,")
        self.emit("_ proof: [u256; L],")
        self.emit("_ instances: [u256; %d]," % self.NI)
        self.emit("_ tr: mut Halo2Transcript,")
        self.emit(") -> Option<Phase1> uses (call: Call) {")
        self.emit("assert(L >= COMMON_WORDS)")
        self.emit("tr.absorb(vk.transcript_repr().to_canonical())")
        # instances (flat; column-major)
        inst = []
        idx = 0
        for c, ln in enumerate(vk["instance_lens"]):
            col = []
            for j in range(ln):
                self.unwrap("inst_%d_%d" % (c, j), "Bn254Fr::from_canonical(instances[%d])" % idx, "Option::None")
                self.emit("tr.absorb(instances[%d])" % idx)
                col.append("inst_%d_%d" % (c, j))
                idx += 1
            inst.append(col)
        # Proof layout (SPEC section 3): np commitments, then the evaluations. Each run is validated
        # by one loop (read_points, read_scalars: rules 1 and 2) and absorbed word by word, so
        # the emitted code does not grow with the number of commitments (EIP-170 code size).
        np_ = len(self.pt_names)
        nev = len(self.ev_names)
        self.unwrap("pts: [G1; %d]" % np_, "read_points(proof, 0)", "Option::None")
        self.unwrap("ev: [Bn254Fr; %d]" % nev, "read_scalars(proof, %d)" % (2 * np_), "Option::None")
        off = 0
        self.emit("tr.absorb_words(proof, 0, %d)" % (2 * self.A))
        off += 2 * self.A
        self.emit("let theta = tr.squeeze()")
        self.emit("tr.absorb_words(proof, %d, %d)" % (off, off + 4 * self.L))
        off += 4 * self.L
        self.emit("let beta = tr.squeeze()")
        self.emit("let gamma = tr.squeeze()")
        n = 2 * (self.S + self.L + 1)
        self.emit("tr.absorb_words(proof, %d, %d)" % (off, off + n))
        off += n
        self.emit("let y = tr.squeeze()")
        self.emit("tr.absorb_words(proof, %d, %d)" % (off, off + 2 * self.H))
        off += 2 * self.H
        self.emit("let x = tr.squeeze()")
        self.emit("tr.absorb_words(proof, %d, %d)" % (off, self.common_words))
        assert off == 2 * np_
        for i, nm in enumerate(self.ev_names):
            self.emit("let %s = ev[%d]" % (nm, i))

        # ---- domain: x^n, Lagrange basis (SPEC 4, 5) with one batched inversion
        self.emit("// x^n and the Lagrange basis at x: L_i(x) = w^i (x^n - 1) n_inv / (x - w^i).")
        self.emit("let xn = x.pow(%d)" % self.n)
        self.emit("let xn_m1 = xn.sub(Bn254Fr::one())")
        need = {0} | {-i for i in range(1, self.bf + 2)}
        for q in vk["instance_queries"]:
            for j in range(vk["instance_lens"][q["column"]]):
                need.add(j - q["rotation"])
        need = sorted(need)
        self.emit("let inv_d = match batch_inv([%s]) {" % ", ".join(
            ["x.sub(fr(%s))" % h(self.w(i)) for i in need] + ["xn_m1"]))
        self.emit("Option::Some(v) => v,")
        self.emit("Option::None => {")
        self.emit("return Option::None")
        self.emit("},")
        self.emit("}")
        self.emit("let wn = xn_m1.mul(fr(%s))" % h(hx(vk["n_inv"])))
        for k, i in enumerate(need):
            self.emit("let l_%s = fr(%s).mul(wn).mul(inv_d[%d])" % (self.rname(i), h(self.w(i)), k))
        self.emit("let l_last = l_%s" % self.rname(-(self.bf + 1)))
        self.emit("let l_blind = %s" % ".add(".join(
            ["l_%s" % self.rname(-i) for i in range(1, self.bf + 1)]) + ")" * (self.bf - 1))
        self.emit("let active = Bn254Fr::one().sub(l_last.add(l_blind))")
        for qi, q in enumerate(vk["instance_queries"]):
            terms = []
            for j, v in enumerate(inst[q["column"]]):
                terms.append("%s.mul(l_%s)" % (v, self.rname(j - q["rotation"])))
            self.emit("let ie%d = %s" % (qi, ".add(".join(terms) + ")" * (len(terms) - 1)))

        # ---- constraints (SPEC 6)
        self.emit("// Constraints, folded as h <- h*y + c in SPEC order: gates, permutation, lookups.")
        self.emit("var hacc = Bn254Fr::zero()")

        def cons(expr):
            self.emit("hacc = hacc.mul(y).add(%s)" % expr)

        self.emit("// gates")
        for g in vk["gates"]:
            for p in g["polys"]:
                cons(self.ex(p))
        self.emit("// permutation argument")
        cons("l_0.mul(Bn254Fr::one().sub(pz0))")
        last = self.S - 1
        cons("pz%d.square().sub(pz%d).mul(l_last)" % (last, last))
        for i in range(1, self.S):
            cons("pz%d.sub(pzl%d).mul(l_0)" % (i, i - 1))
        self.emit("let bx = beta.mul(x)")
        perm_cols = vk["permutation"]["columns"]
        qlist = {"advice": vk["advice_queries"], "fixed": vk["fixed_queries"], "instance": vk["instance_queries"]}
        pre = {"advice": "a", "fixed": "f", "instance": "ie"}

        def cv(c):
            for qi, q in enumerate(qlist[c["type"]]):
                if q["column"] == c["index"] and q["rotation"] == 0:
                    return "%s%d" % (pre[c["type"]], qi)
            sys.exit("permutation column without rotation-0 query")

        for i in range(self.S):
            cols = perm_cols[i * self.chunk: (i + 1) * self.chunk]
            left = "pzn%d" % i
            right = "pz%d" % i
            for t, c in enumerate(cols):
                p = i * self.chunk + t
                left += ".mul(%s.add(beta.mul(sg%d)).add(gamma))" % (cv(c), p)
                dp = pow(hx(vk["delta"]), p, R)
                right += ".mul(%s.add(bx.mul(fr(%s))).add(gamma))" % (cv(c), h(dp))
            cons("%s.sub(%s).mul(active)" % (left, right))
        self.emit("// lookup arguments")
        for j, lk in enumerate(vk["lookups"]):

            def compress(exprs):
                s = "Bn254Fr::zero()"
                for e in exprs:
                    s = "%s.mul(theta).add(%s)" % (s, self.ex(e))
                return s

            self.emit("let lk_in%d = %s" % (j, compress(lk["input_expressions"])))
            self.emit("let lk_tab%d = %s" % (j, compress(lk["table_expressions"])))
            cons("l_0.mul(Bn254Fr::one().sub(lz%d))" % j)
            cons("l_last.mul(lz%d.square().sub(lz%d))" % (j, j))
            cons("lzn{j}.mul(la{j}.add(beta)).mul(ls{j}.add(gamma)).sub(lz{j}.mul(lk_in{j}.add(beta)).mul(lk_tab{j}.add(gamma))).mul(active)".format(j=j))
            cons("l_0.mul(la{j}.sub(ls{j}))".format(j=j))
            cons("la{j}.sub(ls{j}).mul(la{j}.sub(lai{j})).mul(active)".format(j=j))
        self.emit("let expected_h = hacc.mul(inv_d[%d])" % len(need))
        self.emit("Option::Some(Phase1 {")
        self.emit("theta, beta, gamma, y, x, xn, expected_h,")
        self.emit("pts, ev,")
        self.emit("})")
        self.emit("}")
        self.emit()

    # ------------------------------------------------------------------ shared tail helpers
    def prelude(self, rots):
        """Loads commitments, evaluations and opening points into locals."""
        for i, n in enumerate(self.pt_names):
            if n.startswith("hq") or True:
                self.emit("let %s = p1.pts[%d]" % (n, i))
        for i, n in enumerate(self.ev_names):
            self.emit("let %s = p1.ev[%d]" % (n, i))
        self.emit("let x = p1.x")
        self.emit("let expected_h = p1.expected_h")
        used_f = sorted({q["column"] for q in self.vk["fixed_queries"]})
        for c in used_f:
            self.emit("let fx%d = VK_FX%d" % (c, c))
        for p in range(len(self.vk["permutation"]["commitments"])):
            self.emit("let sgc%d = VK_SG%d" % (p, p))
        for r in rots:
            if r == 0:
                self.emit("let pt_0 = x")
            else:
                self.emit("let pt_%s = x.mul(fr(%s))" % (self.rname(r), h(self.w(r))))

    def hc_terms(self, scalar_expr):
        """Terms (G1 name, scalar expression) replacing the derived commitment HC = sum xn^i H_i."""
        out = []
        for i in range(self.H):
            out.append(("hq%d" % i, scalar_expr if i == 0 else "%s.mul(xn%d)" % (scalar_expr, i)))
        return out

    def xn_powers(self):
        self.emit("let xn = p1.xn")
        for i in range(2, self.H):
            pass
        if self.H > 1:
            self.emit("let xn1 = xn")
        for i in range(2, self.H):
            self.emit("let xn%d = xn%d.mul(xn)" % (i, i - 1))

    # ------------------------------------------------------------------ SHPLONK
    def shplonk(self):
        qs = self.queries()
        total = self.common_words + 4
        self.shplonk_words = total
        self.emit("/// SHPLONK proof words: common part + h1 (2) + h2 (2).")
        self.emit("pub const SHPLONK_PROOF_WORDS: usize = %d" % total)
        self.emit()
        self.emit("pub struct ShplonkOpen {")
        self.emit("pub y: Bn254Fr,")
        self.emit("pub v: Bn254Fr,")
        self.emit("pub u: Bn254Fr,")
        self.emit("pub h1: G1,")
        self.emit("pub h2: G1,")
        self.emit("}")
        self.emit()
        self.emit("/// SPEC 9 transcript: y, v, h1, u, h2.")
        self.emit("pub fn shplonk_open<const L: usize>(")
        self.emit("_ proof: [u256; L],")
        self.emit("_ tr: mut Halo2Transcript,")
        self.emit(") -> Option<ShplonkOpen> {")
        self.emit("assert(L >= %d)" % total)
        o = self.common_words
        self.emit("let y = tr.squeeze()")
        self.emit("let v = tr.squeeze()")
        self.unwrap("h1", "read_g1(proof[%d], proof[%d])" % (o, o + 1), "Option::None")
        self.emit("tr.absorb_point(proof[%d], proof[%d])" % (o, o + 1))
        self.emit("let u = tr.squeeze()")
        self.unwrap("h2", "read_g1(proof[%d], proof[%d])" % (o + 2, o + 3), "Option::None")
        self.emit("tr.absorb_point(proof[%d], proof[%d])" % (o + 2, o + 3))
        self.emit("Option::Some(ShplonkOpen { y, v, u, h1, h2 })")
        self.emit("}")
        self.emit()

        # static structure
        sup = []
        for _, r, _ in qs:
            if r not in sup:
                sup.append(r)
        cps = {}
        order = []
        for c, r, _ in qs:
            if c not in cps:
                cps[c] = []
                order.append(c)
            if r not in cps[c]:
                cps[c].append(r)
        rsets = []  # (rots, [comm...])
        for c in order:
            for rs in rsets:
                if set(rs[0]) == set(cps[c]):
                    rs[1].append(c)
                    break
            else:
                rsets.append((list(cps[c]), [c]))
        evalof = {(c, r): e for c, r, e in qs}

        self.emit("/// SPEC 9 final check. Rotation sets of this circuit (points as rotations of `x`):")
        for i, (rs, cs) in enumerate(rsets):
            self.emit("///   S%d = {%s}: %s" % (i, ", ".join(map(str, rs)), ", ".join(cs)))
        self.emit("pub fn shplonk_check(_ vk: Halo2Vk, _ p1: Phase1, _ o: ShplonkOpen) -> bool uses (call: Call) {")
        self.prelude(sup)
        self.xn_powers()
        self.emit("let y = o.y")
        self.emit("let v = o.v")
        self.emit("let u = o.u")
        for r in sup:
            self.emit("let du_%s = u.sub(pt_%s)" % (self.rname(r), self.rname(r)))

        def prod(rs):
            if not rs:
                return "Bn254Fr::one()"
            return ".mul(".join("du_%s" % self.rname(r) for r in rs) + ")" * (len(rs) - 1)

        diff0 = [r for r in sup if r not in rsets[0][0]]
        self.emit("let invs = match batch_inv([x, %s]) {" % prod(diff0))
        self.emit("Option::Some(v) => v,")
        self.emit("Option::None => {")
        self.emit("return false")
        self.emit("},")
        self.emit("}")
        self.emit("let xinv = invs[0]")
        self.emit("let z0_diff_inv = invs[1]")
        self.emit("let xinv2 = xinv.mul(xinv)")
        maxm = max(len(cs) for _, cs in rsets)
        self.emit("let y1 = y")
        for j in range(2, maxm):
            self.emit("let y%d = y%d.mul(y)" % (j, j - 1))
        self.emit("let v1 = v")
        for i in range(2, len(rsets)):
            self.emit("let v%d = v%d.mul(v)" % (i, i - 1))
        self.emit("let z0 = %s" % prod(rsets[0][0]))
        terms = []  # (comm, scalar name)
        self.emit("var rsum = Bn254Fr::zero()")
        for i, (rs, cs) in enumerate(rsets):
            # coefficient k_i = v^i * zdiff_i * z0_diff_inv  (i == 0: 1)
            if i == 0:
                ki = None
            else:
                zd = [r for r in sup if r not in rs]
                self.emit("let k%d = v%d.mul(%s).mul(z0_diff_inv)" % (i, i, prod(zd)))
                ki = "k%d" % i
            m = len(rs)
            lg = []
            if m > 1:
                for kk, rk in enumerate(rs):
                    num = [r for r in rs if r != rk]
                    den = 1
                    for rm in num:
                        den = den * (self.w(rk) - self.w(rm)) % R
                    xp = "xinv" if m == 2 else "xinv2"
                    assert m <= 3
                    self.emit("let lg%d_%d = %s.mul(%s).mul(fr(%s))" % (i, kk, prod(num), xp, h(inv(den))))
                    lg.append("lg%d_%d" % (i, kk))
            for j, c in enumerate(cs):
                if m == 1:
                    rj = evalof[(c, rs[0])]
                else:
                    rj = "Bn254Fr::zero()"
                    for kk, rk in enumerate(rs):
                        rj = "%s.add(%s.mul(%s))" % (rj, evalof[(c, rk)], lg[kk])
                yj = "y%d" % j if j > 0 else None
                if ki and yj:
                    sc = "%s.mul(%s)" % (ki, yj)
                elif ki:
                    sc = ki
                elif yj:
                    sc = yj
                else:
                    sc = "Bn254Fr::one()"
                nm = "s_%s" % c
                self.emit("let %s = %s" % (nm, sc))
                self.emit("rsum = rsum.add(%s.mul(%s))" % (nm, rj))
                if c == "hc":
                    terms.extend(self.hc_terms(nm))
                else:
                    terms.append((c, nm))
        # MSM
        first = True
        for c, sc in terms:
            if first:
                self.emit("var acc = g1_mul(%s, %s)" % (c, sc))
                first = False
            else:
                self.emit("acc = mul_add(acc, %s, %s)" % (c, sc))
        self.emit("acc = mul_add(acc, vk.g1(), rsum.neg())")
        self.emit("acc = mul_add(acc, o.h1, z0.neg())")
        self.emit("acc = mul_add(acc, o.h2, u)")
        self.emit("final_pairing(vk, o.h2, acc)")
        self.emit("}")
        self.emit()
        n = total
        self.emit("/// Verifies a SHPLONK proof (`SHPLONK_PROOF_WORDS` big-endian words) for `instances`.")
        self.emit("/// Returns false for every rejected input (non-canonical, off curve, wrong proof).")
        self.emit("pub fn verify_shplonk(")
        self.emit("_ vk: Halo2Vk,")
        self.emit("_ proof: [u256; %d]," % n)
        self.emit("_ instances: [u256; %d]," % self.NI)
        self.emit(") -> bool uses (call: Call) {")
        self.emit("var tr = Halo2Transcript::new()")
        self.unwrap("p1", "phase1(vk, proof, instances, mut tr)", "false")
        self.unwrap("o", "shplonk_open(proof, mut tr)", "false")
        self.emit("shplonk_check(vk, p1, o)")
        self.emit("}")
        self.emit()

    # ------------------------------------------------------------------ GWC
    def gwc(self):
        qs = self.queries()
        groups = []  # (rot, [(comm, eval)])
        for c, r, e in qs:
            for g in groups:
                if g[0] == r:
                    g[1].append((c, e))
                    break
            else:
                groups.append((r, [(c, e)]))
        M = len(groups)
        total = self.common_words + 2 * M
        self.emit("/// GWC proof words: common part + one witness point per distinct opening point (%d)." % M)
        self.emit("pub const GWC_PROOF_WORDS: usize = %d" % total)
        self.emit()
        self.emit("pub struct GwcOpen {")
        self.emit("pub v: Bn254Fr,")
        self.emit("pub u: Bn254Fr,")
        self.emit("pub w: [G1; %d]," % M)
        self.emit("}")
        self.emit()
        self.emit("/// SPEC 8 transcript: v, W_0..W_{m-1}, u.")
        self.emit("pub fn gwc_open<const L: usize>(_ proof: [u256; L], _ tr: mut Halo2Transcript) -> Option<GwcOpen> {")
        self.emit("assert(L >= %d)" % total)
        self.emit("let v = tr.squeeze()")
        for g in range(M):
            o = self.common_words + 2 * g
            self.unwrap("w%d" % g, "read_g1(proof[%d], proof[%d])" % (o, o + 1), "Option::None")
            self.emit("tr.absorb_point(proof[%d], proof[%d])" % (o, o + 1))
        self.emit("let u = tr.squeeze()")
        self.emit("Option::Some(GwcOpen { v, u, w: [%s] })" % ", ".join("w%d" % g for g in range(M)))
        self.emit("}")
        self.emit()
        self.emit("/// SPEC 8 final check. Point groups (rotation of `x`): %s." % ", ".join(
            "G%d = %d (%d queries)" % (i, r, len(q)) for i, (r, q) in enumerate(groups)))
        self.emit("pub fn gwc_check(_ vk: Halo2Vk, _ p1: Phase1, _ o: GwcOpen) -> bool uses (call: Call) {")
        self.prelude([r for r, _ in groups])
        self.xn_powers()
        self.emit("let v = o.v")
        self.emit("let u = o.u")
        maxq = max(len(q) for _, q in groups)
        self.emit("let vp1 = v")
        for j in range(2, maxq):
            self.emit("let vp%d = vp%d.mul(v)" % (j, j - 1))
        self.emit("let up1 = u")
        for g in range(2, M):
            self.emit("let up%d = up%d.mul(u)" % (g, g - 1))
        # per-commitment scalars
        per = {}
        corder = []
        for g, (r, q) in enumerate(groups):
            for j, (c, e) in enumerate(q):
                if c not in per:
                    per[c] = []
                    corder.append(c)
                pu = "up%d" % g if g > 0 else None
                pv = "vp%d" % j if j > 0 else None
                if pu and pv:
                    per[c].append("%s.mul(%s)" % (pu, pv))
                else:
                    per[c].append(pu or pv or "Bn254Fr::one()")
        terms = []
        for c in corder:
            sc = per[c][0]
            for t in per[c][1:]:
                sc = "%s.add(%s)" % (sc, t)
            self.emit("let s_%s = %s" % (c, sc))
            if c == "hc":
                terms.extend(self.hc_terms("s_hc"))
            else:
                terms.append((c, "s_%s" % c))
        # eval_multi
        self.emit("var eval_multi = Bn254Fr::zero()")
        for g, (r, q) in enumerate(groups):
            eb = None
            for j, (c, e) in enumerate(q):
                t = e if j == 0 else "vp%d.mul(%s)" % (j, e)
                eb = t if eb is None else "%s.add(%s)" % (eb, t)
            if g > 0:
                eb = "up%d.mul(%s)" % (g, eb)
            self.emit("eval_multi = eval_multi.add(%s)" % eb)
        first = True
        for c, sc in terms:
            if first:
                self.emit("var right = g1_mul(%s, %s)" % (c, sc))
                first = False
            else:
                self.emit("right = mul_add(right, %s, %s)" % (c, sc))
        self.emit("right = mul_add(right, vk.g1(), eval_multi.neg())")
        for g, (r, q) in enumerate(groups):
            pz = "pt_%s" % self.rname(r)
            sc = pz if g == 0 else "up%d.mul(%s)" % (g, pz)
            self.emit("right = mul_add(right, o.w[%d], %s)" % (g, sc))
        self.emit("var left = o.w[0]")
        for g in range(1, M):
            self.emit("left = mul_add(left, o.w[%d], up%d)" % (g, g))
        self.emit("final_pairing(vk, left, right)")
        self.emit("}")
        self.emit()
        self.emit("/// Verifies a GWC proof (`GWC_PROOF_WORDS` big-endian words) for `instances`.")
        self.emit("pub fn verify_gwc(")
        self.emit("_ vk: Halo2Vk,")
        self.emit("_ proof: [u256; %d]," % total)
        self.emit("_ instances: [u256; %d]," % self.NI)
        self.emit(") -> bool uses (call: Call) {")
        self.emit("var tr = Halo2Transcript::new()")
        self.unwrap("p1", "phase1(vk, proof, instances, mut tr)", "false")
        self.unwrap("o", "gwc_open(proof, mut tr)", "false")
        self.emit("gwc_check(vk, p1, o)")
        self.emit("}")

    def run(self):
        self.layout()
        self.header()
        self.phase1()
        self.shplonk()
        self.gwc()
        return "\n".join(self.lines) + "\n"


def fixtures(dirp, out):
    lines = [
        "//! GENERATED FILE (vectors/halo2/emit_fe.py --fixtures), do not edit.",
        "//! halo2-axiom 0.5.3 proofs of the `standard_plonk_lookup` circuit (EvmTranscript), as 32-byte",
        "//! big-endian words. Source: vectors/halo2/standard_plonk_lookup/{shplonk,gwc}.json.",
    ]
    for name in ("shplonk", "gwc"):
        d = json.load(open("%s/%s.json" % (dirp, name)))
        raw = bytes.fromhex(d["proof"][2:])
        assert len(raw) % 32 == 0
        words = [int.from_bytes(raw[i:i + 32], "big") for i in range(0, len(raw), 32)]
        lines.append("pub const %s_PROOF: [u256; %d] = [" % (name.upper(), len(words)))
        lines += ["%s," % h(wd) for wd in words]
        lines.append("]")
        inst = [int(v, 16) for col in d["instances"] for v in col]
        lines.append("pub const %s_INSTANCES: [u256; %d] = [%s]" % (name.upper(), len(inst), ", ".join(h(v) for v in inst)))
    write(out, "\n".join(lines) + "\n")


def write(out, text):
    with open(out, "w") as f:
        f.write(text)
    if shutil.which("fe"):
        subprocess.run(["fe", "fmt", out], check=False, stdout=subprocess.DEVNULL)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("vk", help="vk.json (or, with --fixtures, the fixture directory)")
    ap.add_argument("--module", default="circuit")
    ap.add_argument("--out", required=True)
    ap.add_argument("--fixtures", action="store_true")
    a = ap.parse_args()
    if a.fixtures:
        fixtures(a.vk, a.out)
        return
    vk = json.load(open(a.vk))
    write(a.out, Gen(vk, a.module).run())


if __name__ == "__main__":
    main()
