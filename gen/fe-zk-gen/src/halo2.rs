//! halo2-KZG (halo2-axiom 0.5.3, BN254, EvmTranscript): key validation, digest and Fe emitter.
//!
//! This is a port of `vectors/halo2/emit_fe.py` (kept as a cross-check: in `Lib` mode the
//! output of both generators is byte-identical, see `tests/halo2.rs`). Algorithm and field order:
//! `vectors/halo2/SPEC.md`. Everything structural (gate and lookup expressions, permutation
//! sets, SHPLONK rotation sets, GWC point groups) is unrolled into straight-line Fe; the
//! circuit-independent code is `src/verifier/halo2/common.fe`.
//!
//! The key is validated here with ark-bn254 (every point canonical, on curve, in the subgroup,
//! not infinity; omega of order exactly `n`; `n_inv`, `delta`, shapes) and a second time by the
//! Fe compiler (`static_assert` on every constant). Unsupported circuits (challenges, several
//! phases, duplicate commitments) are rejected, never silently emitted.

use crate::emit::{bad, check_name, EmittedIngot};
use crate::model::{VkError, G1, G2};
use crate::validate::{check_g1, check_g2};
use crate::TOOL_VERSION;
use ark_bn254::Fr;
use ark_ff::{BigInteger, Field, One, PrimeField};
use num_bigint::BigUint;
use num_traits::{Num, Zero};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write;

fn r_mod() -> BigUint {
    BigUint::from_bytes_le(&Fr::MODULUS.to_bytes_le())
}

/// `0x` + 64 hex digits.
fn h(v: &BigUint) -> String {
    format!("0x{:0>64}", v.to_str_radix(16))
}

fn hx(v: &Value) -> Result<BigUint, VkError> {
    let s = v.as_str().ok_or_else(|| bad("expected 0x hex string"))?;
    let t = s
        .strip_prefix("0x")
        .ok_or_else(|| bad(format!("missing 0x in '{s}'")))?;
    BigUint::from_str_radix(t, 16).map_err(|_| bad(format!("bad hex '{s}'")))
}

fn inv(a: &BigUint) -> BigUint {
    let r = r_mod();
    (a % &r).modpow(&(&r - 2u8), &r)
}

/// Raw halo2-KZG verifying key as exported by `vectors/halo2` (`vk.json`).
#[derive(Clone, Debug)]
pub struct Halo2Vk {
    pub v: Value,
}

pub fn parse_halo2_vk(json: &str) -> Result<Halo2Vk, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    for k in [
        "k",
        "n",
        "omega",
        "n_inv",
        "delta",
        "cs_degree",
        "blinding_factors",
        "quotient_chunks",
        "transcript_repr",
        "num_advice_columns",
        "num_fixed_columns",
        "num_instance_columns",
        "instance_lens",
        "advice_queries",
        "fixed_queries",
        "instance_queries",
        "fixed_commitments",
        "permutation",
        "gates",
        "lookups",
        "kzg",
    ] {
        if v.get(k).is_none() {
            return Err(bad(format!("vk.json: missing '{k}'")));
        }
    }
    Ok(Halo2Vk { v })
}

fn g1_of(v: &Value, what: &str) -> Result<G1, VkError> {
    Ok(G1::new(
        hx(&v["x"]).map_err(|e| bad(format!("{what}.x: {e}")))?,
        hx(&v["y"]).map_err(|e| bad(format!("{what}.y: {e}")))?,
    ))
}

fn g2_of(v: &Value, what: &str) -> Result<G2, VkError> {
    let c = |p: &Value, i: usize| hx(&p[i]);
    Ok(G2 {
        x: [c(&v["x"], 0)?, c(&v["x"], 1)?],
        y: [c(&v["y"], 0)?, c(&v["y"], 1)?],
    })
    .map_err(|e: VkError| bad(format!("{what}: {e}")))
}

fn u(v: &Value, what: &str) -> Result<i64, VkError> {
    v.as_i64()
        .ok_or_else(|| bad(format!("{what}: expected integer")))
}

impl Halo2Vk {
    fn arr(&self, k: &str) -> &Vec<Value> {
        self.v[k].as_array().unwrap()
    }

    /// Fixed commitment columns that are actually queried (the only ones the verifier uses).
    pub fn used_fixed(&self) -> Vec<usize> {
        let mut s: Vec<usize> = self
            .arr("fixed_queries")
            .iter()
            .map(|q| q["column"].as_u64().unwrap() as usize)
            .collect();
        s.sort();
        s.dedup();
        s
    }

    /// Full validation. Every failure is an error; nothing is repaired.
    pub fn validate(&self) -> Result<(), VkError> {
        let vk = &self.v;
        if vk
            .get("num_challenges")
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
            != 0
        {
            return Err(bad("unsupported: challenges / multiple phases"));
        }
        for k in [
            "instance_lens",
            "advice_queries",
            "fixed_queries",
            "instance_queries",
            "fixed_commitments",
            "gates",
            "lookups",
        ] {
            if !vk[k].is_array() {
                return Err(bad(format!("{k}: expected array")));
            }
        }
        let k = u(&vk["k"], "k")?;
        if !(1..=28).contains(&k) {
            return Err(VkError::Domain(format!("k = {k} not in 1..=28")));
        }
        let n: u64 = 1 << k;
        if u(&vk["n"], "n")? as u64 != n {
            return Err(VkError::Domain("n != 2^k".into()));
        }
        let r = r_mod();
        let omega = hx(&vk["omega"])?;
        let n_inv = hx(&vk["n_inv"])?;
        let delta = hx(&vk["delta"])?;
        let repr = hx(&vk["transcript_repr"])?;
        for (c, w) in [
            (&omega, "omega"),
            (&n_inv, "n_inv"),
            (&delta, "delta"),
            (&repr, "transcript_repr"),
        ] {
            if *c >= r {
                return Err(VkError::NonCanonical(format!("{w} (scalar field)")));
            }
        }
        let om = Fr::from(omega.clone());
        if om.pow([n]) != Fr::one() || om.pow([n / 2]) == Fr::one() {
            return Err(VkError::Domain(
                "omega is not a primitive n-th root of unity".into(),
            ));
        }
        if (&n_inv * BigUint::from(n)) % &r != BigUint::from(1u8) {
            return Err(VkError::Domain("n_inv * n != 1".into()));
        }
        // delta = Fr::DELTA has odd order t = (r - 1) / 2^28: delta^t = 1, delta != 1
        let t = (&r - 1u8) >> 28u32;
        let d = Fr::from(delta);
        if d.is_zero() || d == Fr::one() || d.pow(t.to_u64_digits()) != Fr::one() {
            return Err(VkError::Domain(
                "delta is not Fr::DELTA-like (order dividing (r-1)/2^28)".into(),
            ));
        }
        for key in ["advice_queries", "fixed_queries", "instance_queries"] {
            for q in vk[key].as_array().unwrap() {
                let rot = u(&q["rotation"], "rotation")?;
                if !(-(n as i64) < rot && rot < n as i64) {
                    return Err(VkError::Domain("query rotation out of range".into()));
                }
            }
        }
        if u(&vk["cs_degree"], "cs_degree")? - 1 != u(&vk["quotient_chunks"], "quotient_chunks")? {
            return Err(bad("quotient_chunks != cs_degree - 1"));
        }
        if u(&vk["blinding_factors"], "blinding_factors")? < 1 {
            return Err(bad("blinding_factors must be >= 1"));
        }
        if vk["instance_lens"].as_array().unwrap().len() as i64
            != u(&vk["num_instance_columns"], "num_instance_columns")?
        {
            return Err(bad("instance_lens length != num_instance_columns"));
        }
        // curve points: canonical, on curve, subgroup, not infinity
        let kz = &vk["kzg"];
        let g1 = g1_of(&kz["g1"], "kzg.g1")?;
        check_g1(&g1, "kzg.g1")?;
        if g1.is_identity() {
            return Err(VkError::Identity("kzg.g1".into()));
        }
        for nm in ["g2", "s_g2"] {
            let p = g2_of(&kz[nm], nm)?;
            if p.is_identity() {
                return Err(VkError::Identity(format!("kzg.{nm}")));
            }
            check_g2(&p, &format!("kzg.{nm}"))?;
        }
        let mut used: Vec<(BigUint, BigUint)> = Vec::new();
        let fixed = self.arr("fixed_commitments");
        for c in self.used_fixed() {
            let p = g1_of(
                fixed
                    .get(c)
                    .ok_or_else(|| bad("fixed query column out of range"))?,
                &format!("fixed[{c}]"),
            )?;
            used.push((p.x.clone(), p.y.clone()));
            check_pt(&p, &format!("fixed[{c}]"))?;
        }
        let perm = &vk["permutation"];
        for (i, c) in perm["commitments"]
            .as_array()
            .ok_or_else(|| bad("permutation.commitments"))?
            .iter()
            .enumerate()
        {
            let p = g1_of(c, &format!("sigma[{i}]"))?;
            used.push((p.x.clone(), p.y.clone()));
            check_pt(&p, &format!("sigma[{i}]"))?;
        }
        // SPEC section 7: halo2 merges byte-identical commitments; not implemented
        let mut sorted = used.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != used.len() {
            return Err(bad(
                "duplicate fixed/sigma commitments: dedupe not implemented",
            ));
        }
        let cols = perm["columns"]
            .as_array()
            .ok_or_else(|| bad("permutation.columns"))?;
        if perm["commitments"].as_array().unwrap().len() != cols.len() {
            return Err(bad("permutation: commitments and columns differ in length"));
        }
        if perm["chunk_len"].as_u64().unwrap_or(0) == 0 {
            return Err(bad("permutation.chunk_len must be > 0"));
        }
        Ok(())
    }
}

fn check_pt(p: &G1, what: &str) -> Result<(), VkError> {
    if p.is_identity() {
        return Err(VkError::Identity(what.into()));
    }
    check_g1(p, what)
}

/// sha256 of the canonical vk JSON (sorted keys, compact): the digest embedded in the emitted
/// code (same value the Python emitter computes with `json.dumps(sort_keys, separators)`).
pub fn halo2_key_digest(vk: &Halo2Vk) -> Result<[u8; 32], VkError> {
    vk.validate()?;
    let text = serde_json::to_string(&vk.v).unwrap();
    if !text.is_ascii() {
        return Err(bad("vk.json must be ASCII (canonical digest)"));
    }
    Ok(Sha256::digest(text.as_bytes()).into())
}

// ---------------------------------------------------------------- emitter

/// A real proof of the circuit with its instances (words are 32-byte big-endian).
#[derive(Clone, Debug)]
pub struct Halo2Fixture {
    pub proof: Vec<BigUint>,
    pub instances: Vec<BigUint>,
}

pub fn parse_halo2_proof(json: &str) -> Result<Halo2Fixture, VkError> {
    let d: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    let hexs = d["proof"]
        .as_str()
        .ok_or_else(|| bad("proof: expected hex string"))?;
    let raw =
        hex::decode(hexs.trim_start_matches("0x")).map_err(|e| bad(format!("proof hex: {e}")))?;
    if raw.len() % 32 != 0 {
        return Err(bad("proof length is not a multiple of 32"));
    }
    let proof = raw.chunks(32).map(BigUint::from_bytes_be).collect();
    let mut instances = Vec::new();
    for col in d["instances"].as_array().ok_or_else(|| bad("instances"))? {
        for x in col.as_array().ok_or_else(|| bad("instances column"))? {
            instances.push(hx(x)?);
        }
    }
    Ok(Halo2Fixture { proof, instances })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halo2Mode {
    /// Module that lives inside the fe_zk ingot (`ingot::`, `super::common`).
    Lib,
    /// Standalone ingot depending on fe_zk (`fe_zk::...`) with a contract.
    Ingot,
}

#[derive(Clone, Debug)]
pub struct Halo2EmitOptions {
    pub name: String,
    pub source: String,
    pub fe_zk_path: String,
    pub shplonk: Option<Halo2Fixture>,
    pub gwc: Option<Halo2Fixture>,
}

impl Default for Halo2EmitOptions {
    fn default() -> Self {
        Halo2EmitOptions {
            name: "Halo2".into(),
            source: "vk.json".into(),
            fe_zk_path: "../..".into(),
            shplonk: None,
            gwc: None,
        }
    }
}

/// Sizes the emitted module exposes (needed by the contract and the tests).
#[derive(Clone, Debug)]
pub struct Halo2Layout {
    pub common_words: usize,
    pub n_points: usize,
    pub n_instances: usize,
    pub shplonk_words: usize,
    pub gwc_words: usize,
    pub gwc_points: usize,
}

struct Gen<'a> {
    vk: &'a Value,
    mode: Halo2Mode,
    lines: Vec<String>,
    omega: BigUint,
    omega_inv: BigUint,
    k: u32,
    n: u64,
    bf: usize,
    digest: String,
    a: usize,
    l: usize,
    s: usize,
    p: usize,
    hq: usize,
    aq: usize,
    fq: usize,
    ni: usize,
    chunk: usize,
    pt_names: Vec<String>,
    ev_names: Vec<String>,
    common_words: usize,
    shplonk_words: usize,
    gwc_words: usize,
    gwc_points: usize,
}

type Query = (String, i64, String);

/// `a.mul(b.mul(c))` style nesting used throughout the emitted code.
fn nest(op: &str, terms: &[String]) -> String {
    format!(
        "{}{}",
        terms.join(&format!(".{op}(")),
        ")".repeat(terms.len().saturating_sub(1))
    )
}

fn rname(r: i64) -> String {
    if r >= 0 {
        format!("{r}")
    } else {
        format!("m{}", -r)
    }
}

impl<'a> Gen<'a> {
    fn new(vk: &'a Halo2Vk, mode: Halo2Mode, digest: &[u8; 32]) -> Self {
        let v = &vk.v;
        let omega = hx(&v["omega"]).unwrap();
        let k = v["k"].as_u64().unwrap() as u32;
        let omega_inv = inv(&omega);
        Gen {
            vk: v,
            mode,
            lines: vec![],
            omega,
            omega_inv,
            k,
            n: 1u64 << k,
            bf: v["blinding_factors"].as_u64().unwrap() as usize,
            digest: hex::encode(digest),
            a: 0,
            l: 0,
            s: 0,
            p: 0,
            hq: 0,
            aq: 0,
            fq: 0,
            ni: 0,
            chunk: 0,
            pt_names: vec![],
            ev_names: vec![],
            common_words: 0,
            shplonk_words: 0,
            gwc_words: 0,
            gwc_points: 0,
        }
    }

    fn arr(&self, k: &str) -> &'a Vec<Value> {
        self.vk[k].as_array().unwrap()
    }

    fn w(&self, r: i64) -> BigUint {
        let m = r_mod();
        if r >= 0 {
            self.omega.modpow(&BigUint::from(r as u64), &m)
        } else {
            self.omega_inv.modpow(&BigUint::from((-r) as u64), &m)
        }
    }

    fn emit(&mut self, s: impl Into<String>) {
        self.lines.push(s.into());
    }

    fn ex(&self, e: &Value) -> String {
        let hv = |v: &Value| h(&hx(v).unwrap());
        match e["op"].as_str().unwrap() {
            "const" => format!("fr({})", hv(&e["value"])),
            "fixed" => format!("f{}", e["query"]),
            "advice" => format!("a{}", e["query"]),
            "instance" => format!("ie{}", e["query"]),
            "neg" => format!("{}.neg()", self.ex(&e["a"])),
            "add" => format!("{}.add({})", self.ex(&e["a"]), self.ex(&e["b"])),
            "mul" => format!("{}.mul({})", self.ex(&e["a"]), self.ex(&e["b"])),
            "scale" => format!("{}.mul(fr({}))", self.ex(&e["a"]), hv(&e["scalar"])),
            op => panic!("unsupported expression op {op:?}"),
        }
    }

    fn layout(&mut self) {
        let vk = self.vk;
        self.a = vk["num_advice_columns"].as_u64().unwrap() as usize;
        self.l = self.arr("lookups").len();
        let cols = vk["permutation"]["columns"].as_array().unwrap().len();
        self.chunk = vk["permutation"]["chunk_len"].as_u64().unwrap() as usize;
        self.s = (cols + self.chunk - 1) / self.chunk;
        self.p = cols;
        self.hq = vk["quotient_chunks"].as_u64().unwrap() as usize;
        self.aq = self.arr("advice_queries").len();
        self.fq = self.arr("fixed_queries").len();
        self.ni = self
            .arr("instance_lens")
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .sum();
        let mut pts: Vec<String> = (0..self.a).map(|c| format!("adv{c}")).collect();
        for j in 0..self.l {
            pts.push(format!("lpi{j}"));
            pts.push(format!("lpt{j}"));
        }
        pts.extend((0..self.s).map(|i| format!("zc{i}")));
        pts.extend((0..self.l).map(|j| format!("lzc{j}")));
        pts.push("rnd".into());
        pts.extend((0..self.hq).map(|i| format!("hq{i}")));
        self.pt_names = pts;
        let mut ev: Vec<String> = (0..self.aq).map(|q| format!("a{q}")).collect();
        ev.extend((0..self.fq).map(|q| format!("f{q}")));
        ev.push("rand_eval".into());
        ev.extend((0..self.p).map(|p| format!("sg{p}")));
        for i in 0..self.s {
            ev.push(format!("pz{i}"));
            ev.push(format!("pzn{i}"));
            if i < self.s - 1 {
                ev.push(format!("pzl{i}"));
            }
        }
        for j in 0..self.l {
            for nm in ["lz", "lzn", "la", "lai", "ls"] {
                ev.push(format!("{nm}{j}"));
            }
        }
        self.ev_names = ev;
        self.common_words = 2 * self.pt_names.len() + self.ev_names.len();
    }

    fn queries(&self) -> Vec<Query> {
        let vk = self.vk;
        let mut qs: Vec<Query> = vec![];
        for (q, aq) in self.arr("advice_queries").iter().enumerate() {
            qs.push((
                format!("adv{}", aq["column"]),
                aq["rotation"].as_i64().unwrap(),
                format!("a{q}"),
            ));
        }
        for i in 0..self.s {
            qs.push((format!("zc{i}"), 0, format!("pz{i}")));
            qs.push((format!("zc{i}"), 1, format!("pzn{i}")));
        }
        for i in (0..self.s.saturating_sub(1)).rev() {
            qs.push((format!("zc{i}"), -(self.bf as i64 + 1), format!("pzl{i}")));
        }
        for j in 0..self.l {
            qs.push((format!("lzc{j}"), 0, format!("lz{j}")));
            qs.push((format!("lpi{j}"), 0, format!("la{j}")));
            qs.push((format!("lpt{j}"), 0, format!("ls{j}")));
            qs.push((format!("lpi{j}"), -1, format!("lai{j}")));
            qs.push((format!("lzc{j}"), 1, format!("lzn{j}")));
        }
        for (q, fq) in self.arr("fixed_queries").iter().enumerate() {
            qs.push((
                format!("fx{}", fq["column"]),
                fq["rotation"].as_i64().unwrap(),
                format!("f{q}"),
            ));
        }
        for p in 0..self.p {
            qs.push((format!("sgc{p}"), 0, format!("sg{p}")));
        }
        let _ = vk;
        qs.push(("hc".into(), 0, "expected_h".into()));
        qs.push(("rnd".into(), 0, "rand_eval".into()));
        qs
    }

    fn unwrap_or(&mut self, name: &str, expr: &str, ret: &str) {
        self.emit(format!("let {name} = match {expr} {{"));
        self.emit("Option::Some(v) => v,");
        self.emit("Option::None => {");
        self.emit(format!("return {ret}"));
        self.emit("},");
        self.emit("}");
    }

    fn paths(&self) -> (&'static str, &'static str) {
        match self.mode {
            Halo2Mode::Lib => ("ingot", "super::common"),
            Halo2Mode::Ingot => ("fe_zk", "fe_zk::verifier::halo2::common"),
        }
    }

    fn header(&mut self) {
        let vk = self.vk;
        let t = |s: String| s;
        self.emit(t(format!(
            "//! GENERATED FILE, do not edit. Tool: fe-zk-gen {TOOL_VERSION}."
        )));
        self.emit(format!(
            "//! vk digest (sha256 of canonical vk.json): {}",
            self.digest
        ));
        self.emit(format!(
            "//! Circuit: k={}, {} advice / {} fixed / {} instance columns, {} lookup(s), {} permutation set(s),",
            self.k, self.a, vk["num_fixed_columns"], vk["num_instance_columns"], self.l, self.s
        ));
        self.emit(format!(
            "//! {} quotient chunks. halo2-KZG verifier for BN254 with the snark-verifier EvmTranscript.",
            self.hq
        ));
        self.emit(
            "//! Algorithm and field order: vectors/halo2/SPEC.md. Shared code: halo2/common.fe.",
        );
        self.emit("//!");
        self.emit("//! Safety rules: 1 (every proof scalar / instance `from_canonical`, every coordinate canonical),");
        self.emit("//! 2 (every proof point `read_g1`: on curve, no infinity), 3 (SRS and key points are compile-time");
        self.emit("//! constants checked by `static_assert`: canonical, on curve, G2 in the subgroup; no runtime gas),");
        self.emit("//! 4 (proof and instance lengths are array types),");
        self.emit("//! 5 (transcript absorbs the vk transcript_repr, the instances and every commitment and");
        self.emit("//! evaluation in reference order), 6 (precompile wrappers check success, length, result).");
        let (root, common) = self.paths();
        self.emit("use std::evm::Call");
        self.emit(format!(
            "use {root}::ec::{{G1, G2, g1_is_valid, g1_mul, g2_is_valid}}"
        ));
        self.emit(format!("use {root}::field::{{Bn254Fr, Field}}"));
        self.emit(format!("use {common}::{{"));
        self.emit("Halo2Transcript, Halo2Vk, batch_inv, final_pairing, fr, mul_add, read_g1,");
        self.emit("}");
        self.emit("");
        self.emit(
            "/// Number of 32-byte words of a proof for this circuit (fixed by the vk, rule 4).",
        );
        self.emit(format!(
            "pub const COMMON_WORDS: usize = {}",
            self.common_words
        ));
        self.emit(format!("pub const NUM_INSTANCES: usize = {}", self.ni));
        self.emit(format!(
            "pub const TRANSCRIPT_REPR: u256 = {}",
            h(&hx(&vk["transcript_repr"]).unwrap())
        ));
        self.emit(format!("pub const VK_DIGEST: u256 = 0x{}", self.digest));
        self.emit("");
        self.emit("const SCALAR_R: u256 = 21888242871839275222246405745257275088548364400416034343698204186575808495617");
        self.emit("// SRS and key constants, validated by fe-zk-gen and re-validated by the Fe compiler below.");
        self.emit("// G2 coordinates are (real, imaginary) parts: x0 + x1*u, y0 + y1*u.");
        let kz = &vk["kzg"];
        let hv = |v: &Value| h(&hx(v).unwrap());
        self.emit(format!("const SRS_G1_X: u256 = {}", hv(&kz["g1"]["x"])));
        self.emit(format!("const SRS_G1_Y: u256 = {}", hv(&kz["g1"]["y"])));
        for (nm, key) in [("SRS_G2", "g2"), ("SRS_S_G2", "s_g2")] {
            let vals = [
                &kz[key]["x"][0],
                &kz[key]["x"][1],
                &kz[key]["y"][0],
                &kz[key]["y"][1],
            ];
            for (part, v) in ["X0", "X1", "Y0", "Y1"].iter().zip(vals) {
                self.emit(format!("const {nm}_{part}: u256 = {}", hv(v)));
            }
        }
        let used_f: Vec<usize> = {
            let mut s: Vec<usize> = self
                .arr("fixed_queries")
                .iter()
                .map(|q| q["column"].as_u64().unwrap() as usize)
                .collect();
            s.sort();
            s.dedup();
            s
        };
        for &c in &used_f {
            let f = &vk["fixed_commitments"][c];
            self.emit(format!("const FX{c}_X: u256 = {}", hv(&f["x"])));
            self.emit(format!("const FX{c}_Y: u256 = {}", hv(&f["y"])));
        }
        let perm = vk["permutation"]["commitments"].as_array().unwrap();
        for (p, c) in perm.iter().enumerate() {
            self.emit(format!("const SG{p}_X: u256 = {}", hv(&c["x"])));
            self.emit(format!("const SG{p}_Y: u256 = {}", hv(&c["y"])));
        }
        self.emit("");
        self.emit("// Compile-time validation (Fe CTFE): a key with an invalid point does not compile. Costs no gas;");
        self.emit("// the G2 subgroup checks alone are ~750k gas each at runtime. Fixed and sigma commitments may");
        self.emit("// not be the point at infinity (halo2's EvmTranscript rejects it).");
        self.emit("static_assert(TRANSCRIPT_REPR < SCALAR_R)");
        self.emit("static_assert(g1_is_valid(SRS_G1_X, SRS_G1_Y))");
        self.emit("static_assert(!SRS_G1.is_infinity())");
        for nm in ["SRS_G2", "SRS_S_G2"] {
            self.emit(format!(
                "static_assert(g2_is_valid({nm}_X0, {nm}_X1, {nm}_Y0, {nm}_Y1))"
            ));
            self.emit(format!("static_assert(!{nm}.is_infinity())"));
        }
        for &c in &used_f {
            self.emit(format!("static_assert(g1_is_valid(FX{c}_X, FX{c}_Y))"));
            self.emit(format!("static_assert(!VK_FX{c}.is_infinity())"));
        }
        for p in 0..perm.len() {
            self.emit(format!("static_assert(g1_is_valid(SG{p}_X, SG{p}_Y))"));
            self.emit(format!("static_assert(!VK_SG{p}.is_infinity())"));
        }
        self.emit("");
        self.emit("// Typed points. `from_coords_or_infinity` yields infinity for an invalid point, so an unchecked");
        self.emit("// point cannot exist even if the assertions above were removed.");
        self.emit("const SRS_G1: G1 = G1::from_coords_or_infinity(SRS_G1_X, SRS_G1_Y)");
        for nm in ["SRS_G2", "SRS_S_G2"] {
            self.emit(format!(
                "const {nm}: G2 = G2::from_coords_or_infinity({nm}_X0, {nm}_X1, {nm}_Y0, {nm}_Y1)"
            ));
        }
        for &c in &used_f {
            self.emit(format!(
                "const VK_FX{c}: G1 = G1::from_coords_or_infinity(FX{c}_X, FX{c}_Y)"
            ));
        }
        for p in 0..perm.len() {
            self.emit(format!(
                "const VK_SG{p}: G1 = G1::from_coords_or_infinity(SG{p}_X, SG{p}_Y)"
            ));
        }
        self.emit("");
        self.emit(
            "/// SRS and transcript constants (compile-time validated, no gas for validation).",
        );
        self.emit("pub fn vk() -> Halo2Vk {");
        self.emit("Halo2Vk::from_consts(fr(TRANSCRIPT_REPR), SRS_G1, SRS_G2, SRS_S_G2)");
        self.emit("}");
        self.emit("");
    }

    fn phase1(&mut self) {
        let vk = self.vk;
        self.emit(
            "/// Everything the opening step needs, after the evaluation at `x` has been derived.",
        );
        self.emit("pub struct Phase1 {");
        for f in ["theta", "beta", "gamma", "y", "x", "xn", "expected_h"] {
            self.emit(format!("pub {f}: Bn254Fr,"));
        }
        self.emit(format!(
            "/// Prover commitments in proof order: {}.",
            self.pt_names.join(", ")
        ));
        self.emit(format!("pub pts: [G1; {}],", self.pt_names.len()));
        self.emit(format!(
            "/// Evaluations in proof order: {}.",
            self.ev_names.join(", ")
        ));
        self.emit(format!("pub ev: [Bn254Fr; {}],", self.ev_names.len()));
        self.emit("}");
        self.emit("");
        self.emit("/// Reads and validates the common part of the proof, derives theta, beta, gamma, y, x and the");
        self.emit("/// value `expected_h` of the quotient polynomial at `x` (SPEC sections 2-6). `None` = reject.");
        self.emit("pub fn phase1<const L: usize>(");
        self.emit("_ vk: Halo2Vk,");
        self.emit("_ proof: [u256; L],");
        self.emit(format!("_ instances: [u256; {}],", self.ni));
        self.emit("_ tr: mut Halo2Transcript,");
        self.emit(") -> Option<Phase1> uses (call: Call) {");
        self.emit("assert(L >= COMMON_WORDS)");
        self.emit("tr.absorb(vk.transcript_repr().to_canonical())");
        let mut inst: Vec<Vec<String>> = vec![];
        let mut idx = 0;
        for ln in self.arr("instance_lens") {
            let mut col = vec![];
            for j in 0..ln.as_u64().unwrap() as usize {
                let c = inst.len();
                self.unwrap_or(
                    &format!("inst_{c}_{j}"),
                    &format!("Bn254Fr::from_canonical(instances[{idx}])"),
                    "Option::None",
                );
                self.emit(format!("tr.absorb(instances[{idx}])"));
                col.push(format!("inst_{c}_{j}"));
                idx += 1;
            }
            inst.push(col);
        }
        let mut off = 0usize;
        macro_rules! rd_pt {
            ($name:expr) => {{
                let nm: String = $name;
                self.unwrap_or(
                    &nm,
                    &format!("read_g1(proof[{}], proof[{}])", off, off + 1),
                    "Option::None",
                );
                self.emit(format!(
                    "tr.absorb_point(proof[{}], proof[{}])",
                    off,
                    off + 1
                ));
                off += 2;
            }};
        }
        macro_rules! rd_sc {
            ($name:expr) => {{
                let nm: String = $name;
                self.unwrap_or(
                    &nm,
                    &format!("Bn254Fr::from_canonical(proof[{}])", off),
                    "Option::None",
                );
                self.emit(format!("tr.absorb(proof[{}])", off));
                off += 1;
            }};
        }
        for c in 0..self.a {
            rd_pt!(format!("adv{c}"));
        }
        self.emit("let theta = tr.squeeze()");
        for j in 0..self.l {
            rd_pt!(format!("lpi{j}"));
            rd_pt!(format!("lpt{j}"));
        }
        self.emit("let beta = tr.squeeze()");
        self.emit("let gamma = tr.squeeze()");
        for i in 0..self.s {
            rd_pt!(format!("zc{i}"));
        }
        for j in 0..self.l {
            rd_pt!(format!("lzc{j}"));
        }
        rd_pt!("rnd".to_string());
        self.emit("let y = tr.squeeze()");
        for i in 0..self.hq {
            rd_pt!(format!("hq{i}"));
        }
        self.emit("let x = tr.squeeze()");
        for q in 0..self.aq {
            rd_sc!(format!("a{q}"));
        }
        for q in 0..self.fq {
            rd_sc!(format!("f{q}"));
        }
        rd_sc!("rand_eval".to_string());
        for p in 0..self.p {
            rd_sc!(format!("sg{p}"));
        }
        for i in 0..self.s {
            rd_sc!(format!("pz{i}"));
            rd_sc!(format!("pzn{i}"));
            if i < self.s - 1 {
                rd_sc!(format!("pzl{i}"));
            }
        }
        for j in 0..self.l {
            for nm in ["lz", "lzn", "la", "lai", "ls"] {
                rd_sc!(format!("{nm}{j}"));
            }
        }
        assert_eq!(off, self.common_words);

        self.emit("// x^n and the Lagrange basis at x: L_i(x) = w^i (x^n - 1) n_inv / (x - w^i).");
        self.emit(format!("let xn = x.pow({})", self.n));
        self.emit("let xn_m1 = xn.sub(Bn254Fr::one())");
        let mut need: Vec<i64> = vec![0];
        need.extend((1..=self.bf as i64 + 1).map(|i| -i));
        for q in self.arr("instance_queries") {
            let ln = vk["instance_lens"][q["column"].as_u64().unwrap() as usize]
                .as_u64()
                .unwrap() as i64;
            for j in 0..ln {
                need.push(j - q["rotation"].as_i64().unwrap());
            }
        }
        need.sort();
        need.dedup();
        let mut args: Vec<String> = need
            .iter()
            .map(|&i| format!("x.sub(fr({}))", h(&self.w(i))))
            .collect();
        args.push("xn_m1".into());
        self.emit(format!(
            "let inv_d = match batch_inv([{}]) {{",
            args.join(", ")
        ));
        self.emit("Option::Some(v) => v,");
        self.emit("Option::None => {");
        self.emit("return Option::None");
        self.emit("},");
        self.emit("}");
        self.emit(format!(
            "let wn = xn_m1.mul(fr({}))",
            h(&hx(&vk["n_inv"]).unwrap())
        ));
        for (k, &i) in need.iter().enumerate() {
            self.emit(format!(
                "let l_{} = fr({}).mul(wn).mul(inv_d[{k}])",
                rname(i),
                h(&self.w(i))
            ));
        }
        self.emit(format!("let l_last = l_{}", rname(-(self.bf as i64 + 1))));
        let blind: Vec<String> = (1..=self.bf as i64)
            .map(|i| format!("l_{}", rname(-i)))
            .collect();
        self.emit(format!("let l_blind = {}", nest("add", &blind)));
        self.emit("let active = Bn254Fr::one().sub(l_last.add(l_blind))");
        for (qi, q) in self.arr("instance_queries").iter().enumerate() {
            let col = q["column"].as_u64().unwrap() as usize;
            let rot = q["rotation"].as_i64().unwrap();
            let terms: Vec<String> = inst[col]
                .iter()
                .enumerate()
                .map(|(j, v)| format!("{v}.mul(l_{})", rname(j as i64 - rot)))
                .collect();
            self.emit(format!("let ie{qi} = {}", nest("add", &terms)));
        }

        self.emit(
            "// Constraints, folded as h <- h*y + c in SPEC order: gates, permutation, lookups.",
        );
        self.emit("var hacc = Bn254Fr::zero()");
        self.emit("// gates");
        for g in self.arr("gates") {
            for p in g["polys"].as_array().unwrap() {
                let e = self.ex(p);
                self.emit(format!("hacc = hacc.mul(y).add({e})"));
            }
        }
        self.emit("// permutation argument");
        let cons = |s: &mut Self, e: String| s.emit(format!("hacc = hacc.mul(y).add({e})"));
        cons(self, "l_0.mul(Bn254Fr::one().sub(pz0))".into());
        let last = self.s - 1;
        cons(self, format!("pz{last}.square().sub(pz{last}).mul(l_last)"));
        for i in 1..self.s {
            cons(self, format!("pz{i}.sub(pzl{}).mul(l_0)", i - 1));
        }
        self.emit("let bx = beta.mul(x)");
        let perm_cols = vk["permutation"]["columns"].as_array().unwrap();
        let delta = hx(&vk["delta"]).unwrap();
        let cv = |c: &Value| -> String {
            let (list, pre) = match c["type"].as_str().unwrap() {
                "advice" => ("advice_queries", "a"),
                "fixed" => ("fixed_queries", "f"),
                _ => ("instance_queries", "ie"),
            };
            for (qi, q) in vk[list].as_array().unwrap().iter().enumerate() {
                if q["column"] == c["index"] && q["rotation"].as_i64().unwrap() == 0 {
                    return format!("{pre}{qi}");
                }
            }
            panic!("permutation column without rotation-0 query");
        };
        for i in 0..self.s {
            let cols = &perm_cols[i * self.chunk..((i + 1) * self.chunk).min(perm_cols.len())];
            let mut left = format!("pzn{i}");
            let mut right = format!("pz{i}");
            for (t, c) in cols.iter().enumerate() {
                let p = i * self.chunk + t;
                left += &format!(".mul({}.add(beta.mul(sg{p})).add(gamma))", cv(c));
                let dp = delta.modpow(&BigUint::from(p as u64), &r_mod());
                right += &format!(".mul({}.add(bx.mul(fr({}))).add(gamma))", cv(c), h(&dp));
            }
            cons(self, format!("{left}.sub({right}).mul(active)"));
        }
        self.emit("// lookup arguments");
        for (j, lk) in self.arr("lookups").iter().enumerate() {
            let compress = |s: &Self, exprs: &Vec<Value>| {
                let mut acc = "Bn254Fr::zero()".to_string();
                for e in exprs {
                    acc = format!("{acc}.mul(theta).add({})", s.ex(e));
                }
                acc
            };
            let inp = compress(self, lk["input_expressions"].as_array().unwrap());
            let tab = compress(self, lk["table_expressions"].as_array().unwrap());
            self.emit(format!("let lk_in{j} = {inp}"));
            self.emit(format!("let lk_tab{j} = {tab}"));
            cons(self, format!("l_0.mul(Bn254Fr::one().sub(lz{j}))"));
            cons(self, format!("l_last.mul(lz{j}.square().sub(lz{j}))"));
            cons(self, format!("lzn{j}.mul(la{j}.add(beta)).mul(ls{j}.add(gamma)).sub(lz{j}.mul(lk_in{j}.add(beta)).mul(lk_tab{j}.add(gamma))).mul(active)"));
            cons(self, format!("l_0.mul(la{j}.sub(ls{j}))"));
            cons(
                self,
                format!("la{j}.sub(ls{j}).mul(la{j}.sub(lai{j})).mul(active)"),
            );
        }
        self.emit(format!("let expected_h = hacc.mul(inv_d[{}])", need.len()));
        self.emit("Option::Some(Phase1 {");
        self.emit("theta, beta, gamma, y, x, xn, expected_h,");
        self.emit(format!("pts: [{}],", self.pt_names.join(", ")));
        self.emit(format!("ev: [{}],", self.ev_names.join(", ")));
        self.emit("})");
        self.emit("}");
        self.emit("");
    }

    fn prelude(&mut self, rots: &[i64]) {
        let pts = self.pt_names.clone();
        for (i, n) in pts.iter().enumerate() {
            self.emit(format!("let {n} = p1.pts[{i}]"));
        }
        let evs = self.ev_names.clone();
        for (i, n) in evs.iter().enumerate() {
            self.emit(format!("let {n} = p1.ev[{i}]"));
        }
        self.emit("let x = p1.x");
        self.emit("let expected_h = p1.expected_h");
        let mut used_f: Vec<usize> = self
            .arr("fixed_queries")
            .iter()
            .map(|q| q["column"].as_u64().unwrap() as usize)
            .collect();
        used_f.sort();
        used_f.dedup();
        for c in used_f {
            self.emit(format!("let fx{c} = VK_FX{c}"));
        }
        for p in 0..self.vk["permutation"]["commitments"]
            .as_array()
            .unwrap()
            .len()
        {
            self.emit(format!("let sgc{p} = VK_SG{p}"));
        }
        for &r in rots {
            if r == 0 {
                self.emit("let pt_0 = x");
            } else {
                self.emit(format!(
                    "let pt_{} = x.mul(fr({}))",
                    rname(r),
                    h(&self.w(r))
                ));
            }
        }
    }

    fn hc_terms(&self, scalar: &str) -> Vec<(String, String)> {
        (0..self.hq)
            .map(|i| {
                (
                    format!("hq{i}"),
                    if i == 0 {
                        scalar.to_string()
                    } else {
                        format!("{scalar}.mul(xn{i})")
                    },
                )
            })
            .collect()
    }

    fn xn_powers(&mut self) {
        self.emit("let xn = p1.xn");
        if self.hq > 1 {
            self.emit("let xn1 = xn");
        }
        for i in 2..self.hq {
            self.emit(format!("let xn{i} = xn{}.mul(xn)", i - 1));
        }
    }

    fn shplonk(&mut self) {
        let qs = self.queries();
        let total = self.common_words + 4;
        self.shplonk_words = total;
        self.emit("/// SHPLONK proof words: common part + h1 (2) + h2 (2).");
        self.emit(format!("pub const SHPLONK_PROOF_WORDS: usize = {total}"));
        self.emit("");
        self.emit("pub struct ShplonkOpen {");
        for f in ["y", "v", "u"] {
            self.emit(format!("pub {f}: Bn254Fr,"));
        }
        self.emit("pub h1: G1,");
        self.emit("pub h2: G1,");
        self.emit("}");
        self.emit("");
        self.emit("/// SPEC 9 transcript: y, v, h1, u, h2.");
        self.emit("pub fn shplonk_open<const L: usize>(");
        self.emit("_ proof: [u256; L],");
        self.emit("_ tr: mut Halo2Transcript,");
        self.emit(") -> Option<ShplonkOpen> {");
        self.emit(format!("assert(L >= {total})"));
        let o = self.common_words;
        self.emit("let y = tr.squeeze()");
        self.emit("let v = tr.squeeze()");
        self.unwrap_or(
            "h1",
            &format!("read_g1(proof[{}], proof[{}])", o, o + 1),
            "Option::None",
        );
        self.emit(format!("tr.absorb_point(proof[{}], proof[{}])", o, o + 1));
        self.emit("let u = tr.squeeze()");
        self.unwrap_or(
            "h2",
            &format!("read_g1(proof[{}], proof[{}])", o + 2, o + 3),
            "Option::None",
        );
        self.emit(format!(
            "tr.absorb_point(proof[{}], proof[{}])",
            o + 2,
            o + 3
        ));
        self.emit("Option::Some(ShplonkOpen { y, v, u, h1, h2 })");
        self.emit("}");
        self.emit("");

        let mut sup: Vec<i64> = vec![];
        for (_, r, _) in &qs {
            if !sup.contains(r) {
                sup.push(*r);
            }
        }
        let mut cps: Vec<(String, Vec<i64>)> = vec![];
        for (c, r, _) in &qs {
            match cps.iter_mut().find(|(n, _)| n == c) {
                Some((_, rs)) => {
                    if !rs.contains(r) {
                        rs.push(*r);
                    }
                }
                None => cps.push((c.clone(), vec![*r])),
            }
        }
        let mut rsets: Vec<(Vec<i64>, Vec<String>)> = vec![];
        for (c, rs_c) in &cps {
            let mut placed = false;
            for (rs, cs) in rsets.iter_mut() {
                let mut a = rs.clone();
                let mut b = rs_c.clone();
                a.sort();
                b.sort();
                if a == b {
                    cs.push(c.clone());
                    placed = true;
                    break;
                }
            }
            if !placed {
                rsets.push((rs_c.clone(), vec![c.clone()]));
            }
        }
        let evalof = |c: &str, r: i64| -> String {
            qs.iter()
                .find(|(cc, rr, _)| cc == c && *rr == r)
                .unwrap()
                .2
                .clone()
        };

        self.emit(
            "/// SPEC 9 final check. Rotation sets of this circuit (points as rotations of `x`):",
        );
        for (i, (rs, cs)) in rsets.iter().enumerate() {
            let rl: Vec<String> = rs.iter().map(|r| r.to_string()).collect();
            self.emit(format!(
                "///   S{i} = {{{}}}: {}",
                rl.join(", "),
                cs.join(", ")
            ));
        }
        self.emit("pub fn shplonk_check(_ vk: Halo2Vk, _ p1: Phase1, _ o: ShplonkOpen) -> bool uses (call: Call) {");
        self.prelude(&sup);
        self.xn_powers();
        self.emit("let y = o.y");
        self.emit("let v = o.v");
        self.emit("let u = o.u");
        for &r in &sup {
            self.emit(format!("let du_{} = u.sub(pt_{})", rname(r), rname(r)));
        }
        let prod = |rs: &[i64]| -> String {
            if rs.is_empty() {
                return "Bn254Fr::one()".into();
            }
            let names: Vec<String> = rs.iter().map(|&r| format!("du_{}", rname(r))).collect();
            nest("mul", &names)
        };
        let diff0: Vec<i64> = sup
            .iter()
            .copied()
            .filter(|r| !rsets[0].0.contains(r))
            .collect();
        self.emit(format!(
            "let invs = match batch_inv([x, {}]) {{",
            prod(&diff0)
        ));
        self.emit("Option::Some(v) => v,");
        self.emit("Option::None => {");
        self.emit("return false");
        self.emit("},");
        self.emit("}");
        self.emit("let xinv = invs[0]");
        self.emit("let z0_diff_inv = invs[1]");
        self.emit("let xinv2 = xinv.mul(xinv)");
        let maxm = rsets.iter().map(|(_, cs)| cs.len()).max().unwrap();
        self.emit("let y1 = y");
        for j in 2..maxm {
            self.emit(format!("let y{j} = y{}.mul(y)", j - 1));
        }
        self.emit("let v1 = v");
        for i in 2..rsets.len() {
            self.emit(format!("let v{i} = v{}.mul(v)", i - 1));
        }
        self.emit(format!("let z0 = {}", prod(&rsets[0].0)));
        let mut terms: Vec<(String, String)> = vec![];
        self.emit("var rsum = Bn254Fr::zero()");
        let m_r = r_mod();
        for (i, (rs, cs)) in rsets.iter().enumerate() {
            let ki = if i == 0 {
                None
            } else {
                let zd: Vec<i64> = sup.iter().copied().filter(|r| !rs.contains(r)).collect();
                self.emit(format!(
                    "let k{i} = v{i}.mul({}).mul(z0_diff_inv)",
                    prod(&zd)
                ));
                Some(format!("k{i}"))
            };
            let m = rs.len();
            let mut lg: Vec<String> = vec![];
            if m > 1 {
                for (kk, &rk) in rs.iter().enumerate() {
                    let num: Vec<i64> = rs.iter().copied().filter(|&r| r != rk).collect();
                    let mut den = BigUint::from(1u8);
                    for &rm in &num {
                        den = den * ((self.w(rk) + &m_r - self.w(rm)) % &m_r) % &m_r;
                    }
                    let xp = if m == 2 { "xinv" } else { "xinv2" };
                    assert!(m <= 3);
                    self.emit(format!(
                        "let lg{i}_{kk} = {}.mul({xp}).mul(fr({}))",
                        prod(&num),
                        h(&inv(&den))
                    ));
                    lg.push(format!("lg{i}_{kk}"));
                }
            }
            for (j, c) in cs.iter().enumerate() {
                let rj = if m == 1 {
                    evalof(c, rs[0])
                } else {
                    let mut s = "Bn254Fr::zero()".to_string();
                    for (kk, &rk) in rs.iter().enumerate() {
                        s = format!("{s}.add({}.mul({}))", evalof(c, rk), lg[kk]);
                    }
                    s
                };
                let yj = if j > 0 { Some(format!("y{j}")) } else { None };
                let sc = match (&ki, &yj) {
                    (Some(k), Some(y)) => format!("{k}.mul({y})"),
                    (Some(k), None) => k.clone(),
                    (None, Some(y)) => y.clone(),
                    (None, None) => "Bn254Fr::one()".into(),
                };
                let nm = format!("s_{c}");
                self.emit(format!("let {nm} = {sc}"));
                self.emit(format!("rsum = rsum.add({nm}.mul({rj}))"));
                if c == "hc" {
                    terms.extend(self.hc_terms(&nm));
                } else {
                    terms.push((c.clone(), nm));
                }
            }
        }
        for (idx, (c, sc)) in terms.iter().enumerate() {
            if idx == 0 {
                self.emit(format!("var acc = g1_mul({c}, {sc})"));
            } else {
                self.emit(format!("acc = mul_add(acc, {c}, {sc})"));
            }
        }
        self.emit("acc = mul_add(acc, vk.g1(), rsum.neg())");
        self.emit("acc = mul_add(acc, o.h1, z0.neg())");
        self.emit("acc = mul_add(acc, o.h2, u)");
        self.emit("final_pairing(vk, o.h2, acc)");
        self.emit("}");
        self.emit("");
        self.emit("/// Verifies a SHPLONK proof (`SHPLONK_PROOF_WORDS` big-endian words) for `instances`.");
        self.emit(
            "/// Returns false for every rejected input (non-canonical, off curve, wrong proof).",
        );
        self.emit("pub fn verify_shplonk(");
        self.emit("_ vk: Halo2Vk,");
        self.emit(format!("_ proof: [u256; {total}],"));
        self.emit(format!("_ instances: [u256; {}],", self.ni));
        self.emit(") -> bool uses (call: Call) {");
        self.emit("var tr = Halo2Transcript::new()");
        self.unwrap_or("p1", "phase1(vk, proof, instances, mut tr)", "false");
        self.unwrap_or("o", "shplonk_open(proof, mut tr)", "false");
        self.emit("shplonk_check(vk, p1, o)");
        self.emit("}");
        self.emit("");
    }

    fn gwc(&mut self) {
        let qs = self.queries();
        let mut groups: Vec<(i64, Vec<(String, String)>)> = vec![];
        for (c, r, e) in &qs {
            match groups.iter_mut().find(|g| g.0 == *r) {
                Some(g) => g.1.push((c.clone(), e.clone())),
                None => groups.push((*r, vec![(c.clone(), e.clone())])),
            }
        }
        let m = groups.len();
        let total = self.common_words + 2 * m;
        self.gwc_words = total;
        self.gwc_points = m;
        self.emit(format!(
            "/// GWC proof words: common part + one witness point per distinct opening point ({m})."
        ));
        self.emit(format!("pub const GWC_PROOF_WORDS: usize = {total}"));
        self.emit("");
        self.emit("pub struct GwcOpen {");
        self.emit("pub v: Bn254Fr,");
        self.emit("pub u: Bn254Fr,");
        self.emit(format!("pub w: [G1; {m}],"));
        self.emit("}");
        self.emit("");
        self.emit("/// SPEC 8 transcript: v, W_0..W_{m-1}, u.");
        self.emit("pub fn gwc_open<const L: usize>(_ proof: [u256; L], _ tr: mut Halo2Transcript) -> Option<GwcOpen> {");
        self.emit(format!("assert(L >= {total})"));
        self.emit("let v = tr.squeeze()");
        for g in 0..m {
            let o = self.common_words + 2 * g;
            self.unwrap_or(
                &format!("w{g}"),
                &format!("read_g1(proof[{}], proof[{}])", o, o + 1),
                "Option::None",
            );
            self.emit(format!("tr.absorb_point(proof[{}], proof[{}])", o, o + 1));
        }
        self.emit("let u = tr.squeeze()");
        let ws: Vec<String> = (0..m).map(|g| format!("w{g}")).collect();
        self.emit(format!(
            "Option::Some(GwcOpen {{ v, u, w: [{}] }})",
            ws.join(", ")
        ));
        self.emit("}");
        self.emit("");
        let desc: Vec<String> = groups
            .iter()
            .enumerate()
            .map(|(i, (r, q))| format!("G{i} = {r} ({} queries)", q.len()))
            .collect();
        self.emit(format!(
            "/// SPEC 8 final check. Point groups (rotation of `x`): {}.",
            desc.join(", ")
        ));
        self.emit("pub fn gwc_check(_ vk: Halo2Vk, _ p1: Phase1, _ o: GwcOpen) -> bool uses (call: Call) {");
        let rots: Vec<i64> = groups.iter().map(|g| g.0).collect();
        self.prelude(&rots);
        self.xn_powers();
        self.emit("let v = o.v");
        self.emit("let u = o.u");
        let maxq = groups.iter().map(|g| g.1.len()).max().unwrap();
        self.emit("let vp1 = v");
        for j in 2..maxq {
            self.emit(format!("let vp{j} = vp{}.mul(v)", j - 1));
        }
        self.emit("let up1 = u");
        for g in 2..m {
            self.emit(format!("let up{g} = up{}.mul(u)", g - 1));
        }
        let mut per: Vec<(String, Vec<String>)> = vec![];
        for (g, (_, q)) in groups.iter().enumerate() {
            for (j, (c, _)) in q.iter().enumerate() {
                let pu = if g > 0 { Some(format!("up{g}")) } else { None };
                let pv = if j > 0 { Some(format!("vp{j}")) } else { None };
                let t = match (pu, pv) {
                    (Some(a), Some(b)) => format!("{a}.mul({b})"),
                    (Some(a), None) => a,
                    (None, Some(b)) => b,
                    (None, None) => "Bn254Fr::one()".into(),
                };
                match per.iter_mut().find(|(n, _)| n == c) {
                    Some((_, v)) => v.push(t),
                    None => per.push((c.clone(), vec![t])),
                }
            }
        }
        let mut terms: Vec<(String, String)> = vec![];
        for (c, ts) in &per {
            let mut sc = ts[0].clone();
            for t in &ts[1..] {
                sc = format!("{sc}.add({t})");
            }
            self.emit(format!("let s_{c} = {sc}"));
            if c == "hc" {
                terms.extend(self.hc_terms("s_hc"));
            } else {
                terms.push((c.clone(), format!("s_{c}")));
            }
        }
        self.emit("var eval_multi = Bn254Fr::zero()");
        for (g, (_, q)) in groups.iter().enumerate() {
            let mut eb: Option<String> = None;
            for (j, (_, e)) in q.iter().enumerate() {
                let t = if j == 0 {
                    e.clone()
                } else {
                    format!("vp{j}.mul({e})")
                };
                eb = Some(match eb {
                    None => t,
                    Some(prev) => format!("{prev}.add({t})"),
                });
            }
            let mut eb = eb.unwrap();
            if g > 0 {
                eb = format!("up{g}.mul({eb})");
            }
            self.emit(format!("eval_multi = eval_multi.add({eb})"));
        }
        for (idx, (c, sc)) in terms.iter().enumerate() {
            if idx == 0 {
                self.emit(format!("var right = g1_mul({c}, {sc})"));
            } else {
                self.emit(format!("right = mul_add(right, {c}, {sc})"));
            }
        }
        self.emit("right = mul_add(right, vk.g1(), eval_multi.neg())");
        for (g, (r, _)) in groups.iter().enumerate() {
            let pz = format!("pt_{}", rname(*r));
            let sc = if g == 0 {
                pz
            } else {
                format!("up{g}.mul({pz})")
            };
            self.emit(format!("right = mul_add(right, o.w[{g}], {sc})"));
        }
        self.emit("var left = o.w[0]");
        for g in 1..m {
            self.emit(format!("left = mul_add(left, o.w[{g}], up{g})"));
        }
        self.emit("final_pairing(vk, left, right)");
        self.emit("}");
        self.emit("");
        self.emit("/// Verifies a GWC proof (`GWC_PROOF_WORDS` big-endian words) for `instances`.");
        self.emit("pub fn verify_gwc(");
        self.emit("_ vk: Halo2Vk,");
        self.emit(format!("_ proof: [u256; {total}],"));
        self.emit(format!("_ instances: [u256; {}],", self.ni));
        self.emit(") -> bool uses (call: Call) {");
        self.emit("var tr = Halo2Transcript::new()");
        self.unwrap_or("p1", "phase1(vk, proof, instances, mut tr)", "false");
        self.unwrap_or("o", "gwc_open(proof, mut tr)", "false");
        self.emit("gwc_check(vk, p1, o)");
        self.emit("}");
    }

    fn run(mut self) -> (String, Halo2Layout) {
        self.layout();
        self.header();
        self.phase1();
        self.shplonk();
        self.gwc();
        let lay = Halo2Layout {
            common_words: self.common_words,
            n_points: self.pt_names.len(),
            n_instances: self.ni,
            shplonk_words: self.shplonk_words,
            gwc_words: self.gwc_words,
            gwc_points: self.gwc_points,
        };
        (self.lines.join("\n") + "\n", lay)
    }
}

/// Emit the in-library module (`src/verifier/halo2/<module>.fe`, imports `ingot::` and
/// `super::common`). Byte-identical to `vectors/halo2/emit_fe.py` before `fe fmt`.
pub fn emit_halo2_module(vk: &Halo2Vk) -> Result<String, VkError> {
    let digest = halo2_key_digest(vk)?;
    Ok(Gen::new(vk, Halo2Mode::Lib, &digest).run().0)
}

/// Emit a standalone ingot: the same module with `fe_zk::` imports, plus a contract with
/// `verifyShplonk(uint256[W1],uint256[NI])`, `verifyGwc(uint256[W2],uint256[NI])` and `vkDigest()`,
/// and (with fixtures) a generated battery of tests.
pub fn emit_halo2_ingot(vk: &Halo2Vk, opts: &Halo2EmitOptions) -> Result<EmittedIngot, VkError> {
    check_name(&opts.name)?;
    let digest = halo2_key_digest(vk)?;
    let (mut lib, lay) = Gen::new(vk, Halo2Mode::Ingot, &digest).run();
    let ingot_name = format!("{}_verifier", opts.name.to_lowercase());
    let contract = format!("{}Verifier", opts.name);
    let msg = format!("{contract}Msg");
    let fe_toml = format!(
        "[ingot]\nname = \"{ingot_name}\"\nversion = \"0.1.0\"\n\n[dependencies]\nfe_zk = {{ path = \"{}\", allow_unsafe = true }}\n",
        opts.fe_zk_path
    );
    let (w1, w2, ni) = (lay.shplonk_words, lay.gwc_words, lay.n_instances);
    let w = &mut lib;
    writeln!(w).unwrap();
    writeln!(
        w,
        "// ---- contract ----------------------------------------------------------------"
    )
    .unwrap();
    writeln!(
        w,
        "// Source key: {}. ABI: `proof` is the raw halo2 proof as 32-byte words, `instances` the",
        opts.source
    )
    .unwrap();
    writeln!(w, "// flattened instance columns. All checks live in the verify functions above and in fe_zk.").unwrap();
    writeln!(w, "pub msg {msg} {{").unwrap();
    writeln!(
        w,
        "    #[selector = sol(\"verifyShplonk(uint256[{w1}],uint256[{ni}])\")]"
    )
    .unwrap();
    writeln!(
        w,
        "    VerifyShplonk {{ proof: [u256; {w1}], instances: [u256; {ni}] }} -> bool,"
    )
    .unwrap();
    writeln!(
        w,
        "    #[selector = sol(\"verifyGwc(uint256[{w2}],uint256[{ni}])\")]"
    )
    .unwrap();
    writeln!(
        w,
        "    VerifyGwc {{ proof: [u256; {w2}], instances: [u256; {ni}] }} -> bool,"
    )
    .unwrap();
    writeln!(w, "    #[selector = sol(\"vkDigest()\")]").unwrap();
    writeln!(w, "    VkDigest -> u256,").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "pub contract {contract} {{").unwrap();
    writeln!(w, "    recv {msg} {{").unwrap();
    writeln!(
        w,
        "        VerifyShplonk {{ proof, instances }} -> bool uses (call: Call) {{"
    )
    .unwrap();
    writeln!(w, "            verify_shplonk(vk(), proof, instances)").unwrap();
    writeln!(w, "        }}").unwrap();
    writeln!(w).unwrap();
    writeln!(
        w,
        "        VerifyGwc {{ proof, instances }} -> bool uses (call: Call) {{"
    )
    .unwrap();
    writeln!(w, "            verify_gwc(vk(), proof, instances)").unwrap();
    writeln!(w, "        }}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "        VkDigest -> u256 {{").unwrap();
    writeln!(w, "            VK_DIGEST").unwrap();
    writeln!(w, "        }}").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w, "}}").unwrap();

    let tests_fe = match (&opts.shplonk, &opts.gwc) {
        (Some(s), Some(g)) => Some(emit_tests(vk, opts, &lay, s, g, &contract, &msg, &digest)?),
        (None, None) => None,
        _ => return Err(bad("give both --shplonk and --gwc proofs, or neither")),
    };
    Ok(EmittedIngot {
        ingot_name,
        fe_toml,
        lib_fe: lib,
        tests_fe,
    })
}

fn words(name: &str, ws: &[BigUint]) -> String {
    let mut s = format!("const {name}: [u256; {}] = [\n", ws.len());
    for x in ws {
        s += &format!("    {},\n", h(x));
    }
    s + "]\n"
}

#[allow(clippy::too_many_arguments)]
fn emit_tests(
    vk: &Halo2Vk,
    opts: &Halo2EmitOptions,
    lay: &Halo2Layout,
    sh: &Halo2Fixture,
    gw: &Halo2Fixture,
    contract: &str,
    msg: &str,
    digest: &[u8; 32],
) -> Result<String, VkError> {
    let (w1, w2, ni) = (lay.shplonk_words, lay.gwc_words, lay.n_instances);
    for (n, f, ww) in [("shplonk", sh, w1), ("gwc", gw, w2)] {
        if f.proof.len() != ww {
            return Err(bad(format!(
                "{n} proof has {} words, key expects {ww}",
                f.proof.len()
            )));
        }
        if f.instances.len() != ni {
            return Err(bad(format!(
                "{n} fixture has {} instances, key expects {ni}",
                f.instances.len()
            )));
        }
    }
    let _ = vk;
    let mut s = String::new();
    let w = &mut s;
    writeln!(
        w,
        "//! GENERATED by fe-zk-gen {TOOL_VERSION} -- do not edit."
    )
    .unwrap();
    writeln!(
        w,
        "//! vk digest (sha256 of canonical vk.json): {}",
        hex::encode(digest)
    )
    .unwrap();
    writeln!(
        w,
        "//! Fixture: a real halo2-axiom proof per multiopen scheme (from {}) and a battery of",
        opts.name
    )
    .unwrap();
    writeln!(w, "//! invalid ones; every check of the verifier has a failing case. Calls go through the contract.").unwrap();
    writeln!(w, "use super::*").unwrap();
    writeln!(w, "use std::evm::{{Address, Evm}}").unwrap();
    writeln!(w, "use fe_zk::ec::{{G1, G2, g1_is_valid, g2_is_valid}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "const Q: u256 = 21888242871839275222246405745257275088696311157297823662689037894645226208583").unwrap();
    writeln!(w, "const R: u256 = 21888242871839275222246405745257275088548364400416034343698204186575808495617").unwrap();
    writeln!(
        w,
        "// On the twist but outside the order-r subgroup (py_ecc, scripts/ec-vectors.py)"
    )
    .unwrap();
    writeln!(w, "const BAD_X0: u256 = 2").unwrap();
    writeln!(w, "const BAD_X1: u256 = 1").unwrap();
    writeln!(w, "const BAD_Y0: u256 = 7292567877523311580221095596750716176434782432868683424513645834767876293070").unwrap();
    writeln!(w, "const BAD_Y1: u256 = 19659275751359636165940301690575149581329631496732780143538578556285923319774").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "#[event]").unwrap();
    writeln!(w, "struct GasUsed {{ what: u256, gas: u256 }}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "// Proof layout (SPEC section 3): the first {} words are {} commitments (x, y), then {} evaluations,", 2 * lay.n_points, lay.n_points, lay.common_words - 2 * lay.n_points).unwrap();
    writeln!(
        w,
        "// then the opening points: SHPLONK h1, h2; GWC witness points W_0..W_{}.",
        lay.gwc_points - 1
    )
    .unwrap();
    writeln!(w, "const EVAL_START: usize = {}", 2 * lay.n_points).unwrap();
    writeln!(w, "const OPEN_START: usize = {}", lay.common_words).unwrap();
    w.push_str(&words("SHPLONK_PROOF", &sh.proof));
    w.push_str(&words("GWC_PROOF", &gw.proof));
    for (n, f) in [("SHPLONK", sh), ("GWC", gw)] {
        let xs: Vec<String> = f.instances.iter().map(h).collect();
        writeln!(w, "const {n}_INSTANCES: [u256; {ni}] = [{}]", xs.join(", ")).unwrap();
    }
    writeln!(w).unwrap();
    writeln!(
        w,
        "fn is_point_word(_ i: usize) -> bool {{ i < EVAL_START || i >= OPEN_START }}"
    )
    .unwrap();
    writeln!(w, "fn is_x_word(_ i: usize) -> bool {{").unwrap();
    writeln!(w, "    if i < EVAL_START {{ i % 2 == 0 }} else {{ i >= OPEN_START && (i - OPEN_START) % 2 == 0 }}").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "fn deploy() -> Address uses (evm: mut Evm) {{").unwrap();
    writeln!(
        w,
        "    evm.create2<{contract}>(value: Wei::zero(), args: (), salt: 0)"
    )
    .unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    for (fnname, variant, ww) in [("vs", "VerifyShplonk", w1), ("vg", "VerifyGwc", w2)] {
        writeln!(
            w,
            "fn {fnname}(_ p: Address, _ pr: [u256; {ww}], _ x: [u256; {ni}]) -> bool"
        )
        .unwrap();
        writeln!(w, "uses (evm: mut Evm)").unwrap();
        writeln!(w, "{{").unwrap();
        writeln!(w, "    let r: bool = evm").unwrap();
        writeln!(w, "        .call(addr: p, gas: 20000000, value: Wei::zero(), message: {msg}::{variant} {{ proof: pr, instances: x }})").unwrap();
        writeln!(w, "    r").unwrap();
        writeln!(w, "}}").unwrap();
        writeln!(w).unwrap();
    }

    let t = |w: &mut String, name: &str, body: &str| {
        writeln!(w, "#[test]").unwrap();
        writeln!(w, "fn {name}() uses (evm: mut Evm) {{").unwrap();
        writeln!(w, "    let p = deploy()").unwrap();
        for l in body.lines() {
            writeln!(w, "    {l}").unwrap();
        }
        writeln!(w, "}}").unwrap();
        writeln!(w).unwrap();
    };

    t(w, "vk_digest_exposed", &format!(
        "let d: u256 = evm.call(addr: p, gas: 1000000, value: Wei::zero(), message: {msg}::VkDigest {{}})\nassert(d == VK_DIGEST)\nassert(d == 0x{})",
        hex::encode(digest)));
    for (sch, f, ww, big) in [("shplonk", "vs", w1, "SHPLONK"), ("gwc", "vg", w2, "GWC")] {
        let pr = format!("{big}_PROOF");
        let ins = format!("{big}_INSTANCES");
        t(w, &format!("{sch}_valid_proof_accepted"), &format!(
            "let g0 = evm.gas()\nlet ok = {f}(p, {pr}, {ins})\nevm.emit(GasUsed {{ what: {}, gas: g0 - evm.gas() }})\nassert(ok)",
            if sch == "shplonk" { 1 } else { 2 }));
        t(
            w,
            &format!("{sch}_wrong_instance_rejected"),
            &format!("assert(!{f}(p, {pr}, [{ins}[0] + 1]))\nassert(!{f}(p, {pr}, [0]))"),
        );
        t(w, &format!("{sch}_non_canonical_instance_rejected"), &format!(
            "// value + r is the same residue: rejected, never reduced\nassert(!{f}(p, {pr}, [{ins}[0] + R]))\nassert(!{f}(p, {pr}, [R]))\nassert(!{f}(p, {pr}, [0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff]))"));
        t(w, &format!("{sch}_every_word_plus_one_rejected"), &format!(
            "var i: usize = 0\nwhile i < {ww} {{\n    var pr = {pr}\n    pr[i] = pr[i] + 1\n    assert(!{f}(p, pr, {ins}))\n    i += 1\n}}"));
        t(w, &format!("{sch}_non_canonical_words_rejected"), &format!(
            "// every word shifted by its modulus (q for coordinates, r for scalars): same residue, not canonical\nvar i: usize = 0\nwhile i < {ww} {{\n    var pr = {pr}\n    if is_point_word(i) {{\n        pr[i] = pr[i] + Q\n    }} else {{\n        pr[i] = pr[i] + R\n    }}\n    assert(!{f}(p, pr, {ins}))\n    i += 1\n}}"));
        t(w, &format!("{sch}_off_curve_points_rejected"), &format!(
            "var i: usize = 0\nwhile i < {ww} {{\n    if is_x_word(i) {{\n        var pr = {pr}\n        pr[i + 1] = pr[i + 1] + 1\n        assert(!{f}(p, pr, {ins}))\n    }}\n    i += 1\n}}"));
        t(w, &format!("{sch}_infinity_points_rejected"), &format!(
            "// halo2's EvmTranscript rejects the point at infinity\nvar i: usize = 0\nwhile i < {ww} {{\n    if is_x_word(i) {{\n        var pr = {pr}\n        pr[i] = 0\n        pr[i + 1] = 0\n        assert(!{f}(p, pr, {ins}))\n    }}\n    i += 1\n}}"));
        t(w, &format!("{sch}_negated_points_rejected"), &format!(
            "var i: usize = 0\nwhile i < {ww} {{\n    if is_x_word(i) {{\n        var pr = {pr}\n        pr[i + 1] = Q - pr[i + 1]\n        assert(!{f}(p, pr, {ins}))\n    }}\n    i += 1\n}}"));
        t(w, &format!("{sch}_swapped_commitments_rejected"), &format!(
            "var pr = {pr}\nlet x0 = pr[0]\nlet y0 = pr[1]\npr[0] = pr[2]\npr[1] = pr[3]\npr[2] = x0\npr[3] = y0\nassert(!{f}(p, pr, {ins}))"));
        t(w, &format!("{sch}_proof_of_other_scheme_rejected"), &format!(
            "// the other scheme's proof words, resized to this scheme's length, never verify\nvar pr = {pr}\nvar i: usize = 0\nwhile i < {ww} {{\n    pr[i] = {}[i % {}]\n    i += 1\n}}\nassert(!{f}(p, pr, {ins}))",
            if sch == "shplonk" { "GWC_PROOF" } else { "SHPLONK_PROOF" },
            if sch == "shplonk" { w2 } else { w1 }));
    }

    writeln!(w, "/// The key constants pass the full checks of the library (canonical, on curve, G2 subgroup),").unwrap();
    writeln!(w, "/// also at runtime.").unwrap();
    writeln!(w, "#[test]").unwrap();
    writeln!(w, "fn vk_constants_pass_full_checks() {{").unwrap();
    writeln!(
        w,
        "    assert(G1::from_coords(SRS_G1_X, SRS_G1_Y).is_some())"
    )
    .unwrap();
    for nm in ["SRS_G2", "SRS_S_G2"] {
        writeln!(
            w,
            "    assert(G2::from_coords({nm}_X0, {nm}_X1, {nm}_Y0, {nm}_Y1).is_some())"
        )
        .unwrap();
    }
    let mut used_f: Vec<usize> = vk
        .arr("fixed_queries")
        .iter()
        .map(|q| q["column"].as_u64().unwrap() as usize)
        .collect();
    used_f.sort();
    used_f.dedup();
    for c in used_f {
        writeln!(w, "    assert(G1::from_coords(FX{c}_X, FX{c}_Y).is_some())").unwrap();
    }
    for p in 0..vk.arr_len_perm() {
        writeln!(w, "    assert(G1::from_coords(SG{p}_X, SG{p}_Y).is_some())").unwrap();
    }
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "/// What `static_assert` rejects at compile time, checked at runtime: each of these constants").unwrap();
    writeln!(w, "/// would make the generated verifier fail to compile.").unwrap();
    writeln!(w, "#[test]").unwrap();
    writeln!(w, "fn invalid_keys_fail_the_compile_time_checks() {{").unwrap();
    writeln!(
        w,
        "    assert(!g2_is_valid(BAD_X0, BAD_X1, BAD_Y0, BAD_Y1))"
    )
    .unwrap();
    writeln!(
        w,
        "    assert(!g2_is_valid(SRS_S_G2_X0, SRS_S_G2_X1, SRS_S_G2_Y0, SRS_S_G2_Y1 + 1))"
    )
    .unwrap();
    writeln!(
        w,
        "    assert(!g2_is_valid(SRS_G2_X0 + Q, SRS_G2_X1, SRS_G2_Y0, SRS_G2_Y1))"
    )
    .unwrap();
    writeln!(w, "    assert(!g1_is_valid(SG0_X, SG0_Y + 1))").unwrap();
    writeln!(w, "    assert(!g1_is_valid(SG0_X + Q, SG0_Y))").unwrap();
    writeln!(w, "    assert(!g1_is_valid(SRS_G1_X, SRS_G1_Y + 1))").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "#[test]").unwrap();
    writeln!(w, "fn vk_construction_gas() uses (evm: mut Evm) {{").unwrap();
    writeln!(w, "    let g0 = evm.gas()").unwrap();
    writeln!(w, "    let k = vk()").unwrap();
    writeln!(
        w,
        "    evm.emit(GasUsed {{ what: 0, gas: g0 - evm.gas() }})"
    )
    .unwrap();
    writeln!(w, "    assert(k.g1().x() == 1)").unwrap();
    writeln!(w, "}}").unwrap();
    Ok(s)
}

impl Halo2Vk {
    fn arr_len_perm(&self) -> usize {
        self.v["permutation"]["commitments"]
            .as_array()
            .unwrap()
            .len()
    }
}
