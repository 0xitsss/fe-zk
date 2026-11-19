//! snarkjs PLONK (BN254, KZG): key model, parser, validation (ark-bn254), digest and Fe emitter.
//!
//! Same pipeline as Groth16: `parse_plonk_snarkjs` (raw, untrusted) -> `PlonkVk::validate`
//! (ark-bn254, independent of the emitted code) -> `plonk_key_digest` -> `emit_plonk_ingot`.
//! The emitted contract only forwards calldata to `plonk::verify_calldata` in fe_zk; every check on
//! proofs lives in the library, and the key is validated a second time by the Fe compiler
//! (`static_assert`).

use crate::emit::{bad, check_name};
use crate::model::{VkError, G1, G2};
use crate::parse::{arr, dec, snarkjs_g1, snarkjs_g2};
use crate::validate::{check_g1, check_g2};
use crate::TOOL_VERSION;
use ark_bn254::Fr;
use ark_ff::{BigInteger, Field, One, PrimeField, Zero};
use num_bigint::BigUint;
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::fmt::Write;

/// ASCII domain tag that starts the digest preimage.
pub const PLONK_DIGEST_DOMAIN: &[u8] = b"fe-zk/plonk/bn254/vk/v1";

pub const R_DEC: &str =
    "21888242871839275222246405745257275088548364400416034343698204186575808495617";

/// Raw snarkjs PLONK verifying key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlonkVk {
    pub n_public: usize,
    pub power: u32,
    pub w: BigUint,
    pub k1: BigUint,
    pub k2: BigUint,
    /// Qm, Ql, Qr, Qo, Qc, S1, S2, S3 (this order, as hashed into beta)
    pub commitments: [G1; 8],
    pub x2: G2,
}

pub const COMMITMENT_NAMES: [&str; 8] = ["QM", "QL", "QR", "QO", "QC", "S1", "S2", "S3"];

/// Raw snarkjs PLONK proof: A, B, C, Z, T1, T2, T3, Wxi, Wxiw and eval_a, eval_b, eval_c,
/// eval_s1, eval_s2, eval_zw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlonkProof {
    pub points: [G1; 9],
    pub evals: [BigUint; 6],
}

impl PlonkProof {
    /// The `uint256[24]` calldata words of snarkjs' `Verifier.sol`.
    pub fn words(&self) -> Vec<BigUint> {
        let mut v = Vec::new();
        for p in &self.points {
            v.push(p.x.clone());
            v.push(p.y.clone());
        }
        v.extend(self.evals.iter().cloned());
        v
    }
}

#[derive(Clone, Debug)]
pub struct PlonkFixture {
    pub proof: PlonkProof,
    pub public: Vec<BigUint>,
}

pub fn parse_plonk_snarkjs(json: &str) -> Result<PlonkVk, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    if v["protocol"] != "plonk" {
        return Err(bad("protocol must be plonk"));
    }
    if v["curve"] != "bn128" {
        return Err(bad("curve must be bn128"));
    }
    let n_public = v["nPublic"]
        .as_u64()
        .ok_or_else(|| bad("nPublic missing"))? as usize;
    let power = v["power"].as_u64().ok_or_else(|| bad("power missing"))?;
    let power = u32::try_from(power).map_err(|_| bad("power too large"))?;
    let w = dec(&v["w"], "w")?;
    let k1 = dec(&v["k1"], "k1")?;
    let k2 = dec(&v["k2"], "k2")?;
    let mut cs: Vec<G1> = Vec::new();
    for n in ["Qm", "Ql", "Qr", "Qo", "Qc", "S1", "S2", "S3"] {
        cs.push(snarkjs_g1(&v[n], n)?);
    }
    let x2 = snarkjs_g2(&v["X_2"], "X_2")?;
    Ok(PlonkVk {
        n_public,
        power,
        w,
        k1,
        k2,
        commitments: cs.try_into().unwrap(),
        x2,
    })
}

pub fn parse_plonk_snarkjs_proof(json: &str) -> Result<PlonkProof, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    if v["protocol"] != "plonk" {
        return Err(bad("proof protocol must be plonk"));
    }
    let mut points = Vec::new();
    for n in ["A", "B", "C", "Z", "T1", "T2", "T3", "Wxi", "Wxiw"] {
        // proof points are on the wire as [x, y, 1]
        arr(&v[n], n, 3)?;
        points.push(snarkjs_g1(&v[n], n)?);
    }
    let mut evals = Vec::new();
    for n in [
        "eval_a", "eval_b", "eval_c", "eval_s1", "eval_s2", "eval_zw",
    ] {
        evals.push(dec(&v[n], n)?);
    }
    Ok(PlonkProof {
        points: points.try_into().unwrap(),
        evals: evals.try_into().unwrap(),
    })
}

fn fr_modulus() -> BigUint {
    BigUint::from_bytes_le(&Fr::MODULUS.to_bytes_le())
}

fn to_fr(c: &BigUint, what: &str) -> Result<Fr, VkError> {
    if *c >= fr_modulus() {
        return Err(VkError::NonCanonical(format!("{what} (scalar field)")));
    }
    Ok(Fr::from(c.clone()))
}

impl PlonkVk {
    /// Full validation. Every failure is an error; nothing is reduced or repaired.
    ///  * `n_public >= 1`, `1 <= power <= 28`
    ///  * `w`, `k1`, `k2` canonical (`< r`), `k1`, `k2` nonzero
    ///  * `w` has order exactly `2^power` (`w^(2^(power-1)) = -1`)
    ///  * `k1`, `k2` give three distinct cosets: `k1^n`, `k2^n`, `(k1/k2)^n` all differ from 1
    ///  * all eight commitments canonical, on curve, in the subgroup (identity allowed: a gate
    ///    selector may be the zero polynomial)
    ///  * `[x]_2` canonical, on the twist, in the order-r subgroup, not the identity
    pub fn validate(&self) -> Result<(), VkError> {
        if self.n_public == 0 {
            return Err(VkError::Domain("nPublic must be >= 1".into()));
        }
        if self.power == 0 || self.power > 28 {
            return Err(VkError::Domain(format!(
                "power {} not in 1..=28",
                self.power
            )));
        }
        let w = to_fr(&self.w, "w")?;
        let k1 = to_fr(&self.k1, "k1")?;
        let k2 = to_fr(&self.k2, "k2")?;
        if k1.is_zero() || k2.is_zero() {
            return Err(VkError::Domain("k1 and k2 must be nonzero".into()));
        }
        let n: u64 = 1u64 << self.power;
        if w.pow([n / 2]) != -Fr::one() {
            return Err(VkError::Domain(format!(
                "w is not a primitive 2^{}-th root of unity",
                self.power
            )));
        }
        let one = Fr::one();
        let k2_inv = k2.inverse().unwrap();
        if k1.pow([n]) == one || k2.pow([n]) == one || (k1 * k2_inv).pow([n]) == one {
            return Err(VkError::Domain(
                "k1, k2 do not give three distinct cosets of the domain".into(),
            ));
        }
        for (i, c) in self.commitments.iter().enumerate() {
            check_g1(c, COMMITMENT_NAMES[i])?;
        }
        if self.x2.is_identity() {
            return Err(VkError::Identity("X_2".into()));
        }
        check_g2(&self.x2, "X_2")?;
        Ok(())
    }
}

fn word(out: &mut Vec<u8>, v: &BigUint) {
    let b = v.to_bytes_be();
    assert!(b.len() <= 32, "digest is only defined for validated keys");
    out.extend(std::iter::repeat(0u8).take(32 - b.len()));
    out.extend(b);
}

/// Canonical preimage: domain tag, nPublic, power, w, k1, k2, then Qm Ql Qr Qo Qc S1 S2 S3
/// (x, y each; identity = 0, 0) and X_2 as x.c0, x.c1, y.c0, y.c1 (JSON order).
pub fn plonk_key_preimage(vk: &PlonkVk) -> Result<Vec<u8>, VkError> {
    vk.validate()?;
    let mut out = Vec::new();
    out.extend_from_slice(PLONK_DIGEST_DOMAIN);
    word(&mut out, &BigUint::from(vk.n_public));
    word(&mut out, &BigUint::from(vk.power));
    word(&mut out, &vk.w);
    word(&mut out, &vk.k1);
    word(&mut out, &vk.k2);
    for c in &vk.commitments {
        word(&mut out, &c.x);
        word(&mut out, &c.y);
    }
    for c in vk.x2.x.iter().chain(vk.x2.y.iter()) {
        word(&mut out, c);
    }
    Ok(out)
}

pub fn plonk_key_digest(vk: &PlonkVk) -> Result<[u8; 32], VkError> {
    let mut h = Keccak256::new();
    h.update(plonk_key_preimage(vk)?);
    Ok(h.finalize().into())
}

// ---------------------------------------------------------------- emitter

#[derive(Clone, Debug)]
pub struct PlonkEmitOptions {
    pub name: String,
    pub source: String,
    pub fe_zk_path: String,
    pub fixture: Option<PlonkFixture>,
}

impl Default for PlonkEmitOptions {
    fn default() -> Self {
        PlonkEmitOptions {
            name: "Plonk".into(),
            source: "verification_key.json".into(),
            fe_zk_path: "../..".into(),
            fixture: None,
        }
    }
}

pub use crate::emit::EmittedIngot;

/// Emit only the verifier module with default options.
pub fn emit_plonk_fe(vk: &PlonkVk) -> Result<String, VkError> {
    Ok(emit_plonk_ingot(vk, &PlonkEmitOptions::default())?.lib_fe)
}

pub fn emit_plonk_ingot(vk: &PlonkVk, opts: &PlonkEmitOptions) -> Result<EmittedIngot, VkError> {
    check_name(&opts.name)?;
    let digest = plonk_key_digest(vk)?; // validates
    let n = vk.n_public;
    let ingot_name = format!("{}_verifier", opts.name.to_lowercase());
    let contract = format!("{}Verifier", opts.name);
    let msg = format!("{contract}Msg");
    let fe_toml = format!(
        "[ingot]\nname = \"{ingot_name}\"\nversion = \"{TOOL_VERSION}\"\n\n[dependencies]\nfe_zk = {{ path = \"{}\", allow_unsafe = true }}\n",
        opts.fe_zk_path
    );
    let mut s = String::new();
    let w = &mut s;
    writeln!(
        w,
        "//! GENERATED by fe-zk-gen {TOOL_VERSION} -- do not edit."
    )
    .unwrap();
    writeln!(
        w,
        "//! PLONK (snarkjs, KZG) / BN254 verifier for {n} public signal(s), domain 2^{}.",
        vk.power
    )
    .unwrap();
    writeln!(w, "//! Source key : {}", opts.source).unwrap();
    writeln!(w, "//! Key digest : 0x{}", hex::encode(digest)).unwrap();
    writeln!(
        w,
        "//! (keccak256 over the canonical key encoding, see gen/README.md)"
    )
    .unwrap();
    writeln!(w, "//!").unwrap();
    writeln!(
        w,
        "//! The contract only forwards calldata to `plonk::verify_calldata` in fe_zk, which does all"
    )
    .unwrap();
    writeln!(w, "//! proof checks: canonical public inputs, evaluations and coordinates, commitments on curve").unwrap();
    writeln!(w, "//! and not infinity, Keccak transcript over the key, inputs and every commitment, pairing == 1.").unwrap();
    writeln!(w, "//! ABI-compatible with snarkjs' Verifier.sol: `verifyProof(uint256[24],uint256[{n}])`, proof words").unwrap();
    writeln!(w, "//! A, B, C, Z, T1, T2, T3, Wxi, Wxiw (x, y each) then eval_a, eval_b, eval_c, eval_s1, eval_s2,").unwrap();
    writeln!(
        w,
        "//! eval_zw, as printed by `snarkjs zkey export soliditycalldata`."
    )
    .unwrap();
    writeln!(w, "use fe_zk::ec::{{G1, G2, g1_is_valid, g2_is_valid}}").unwrap();
    writeln!(
        w,
        "use fe_zk::plonk"
    )
    .unwrap();
    writeln!(w).unwrap();
    writeln!(w, "pub const N_PUBLIC: u256 = {n}").unwrap();
    writeln!(w, "pub const VK_DIGEST: u256 = 0x{}", hex::encode(digest)).unwrap();
    writeln!(w).unwrap();
    writeln!(w, "// Verifying key, validated by fe-zk-gen (canonical, on curve, in subgroup, w of order 2^power, distinct").unwrap();
    writeln!(w, "// cosets) and re-validated at compile time below. G2 coordinates are (real, imaginary) parts.").unwrap();
    writeln!(w, "pub const POWER: u256 = {}", vk.power).unwrap();
    writeln!(w, "pub const OMEGA: u256 = {}", vk.w).unwrap();
    writeln!(w, "pub const K1: u256 = {}", vk.k1).unwrap();
    writeln!(w, "pub const K2: u256 = {}", vk.k2).unwrap();
    for (i, nm) in COMMITMENT_NAMES.iter().enumerate() {
        writeln!(w, "pub const {nm}_X: u256 = {}", vk.commitments[i].x).unwrap();
        writeln!(w, "pub const {nm}_Y: u256 = {}", vk.commitments[i].y).unwrap();
    }
    writeln!(w, "pub const X2_X0: u256 = {}", vk.x2.x[0]).unwrap();
    writeln!(w, "pub const X2_X1: u256 = {}", vk.x2.x[1]).unwrap();
    writeln!(w, "pub const X2_Y0: u256 = {}", vk.x2.y[0]).unwrap();
    writeln!(w, "pub const X2_Y1: u256 = {}", vk.x2.y[1]).unwrap();
    writeln!(w).unwrap();
    writeln!(w, "// Compile-time validation (Fe CTFE): a key with an invalid point or a bad evaluation domain does").unwrap();
    writeln!(w, "// not compile. Independent of the Rust-side validation fe-zk-gen did before emitting, and free of").unwrap();
    writeln!(w, "// gas: the constants are evaluated by the compiler (the G2 subgroup check alone is ~750k gas at").unwrap();
    writeln!(w, "// runtime). Commitments may be the point at infinity (zero selector polynomial); [x]_2 may not.").unwrap();
    writeln!(w, "static_assert(N_PUBLIC > 0)").unwrap();
    writeln!(
        w,
        "static_assert(plonk::domain_is_valid(POWER, OMEGA, K1, K2))"
    )
    .unwrap();
    for nm in COMMITMENT_NAMES {
        writeln!(w, "static_assert(g1_is_valid({nm}_X, {nm}_Y))").unwrap();
    }
    writeln!(w, "static_assert(g2_is_valid(X2_X0, X2_X1, X2_Y0, X2_Y1))").unwrap();
    writeln!(w, "static_assert(!VK_X2.is_infinity())").unwrap();
    writeln!(w).unwrap();
    writeln!(
        w,
        "// Typed key points. `from_coords_or_infinity` yields infinity for an invalid point, and"
    )
    .unwrap();
    writeln!(w, "// `plonk::Vk::from_consts` poisons a key with a bad domain, which `plonk::verify` rejects, so no").unwrap();
    writeln!(
        w,
        "// usable unchecked key can exist even if the assertions above were removed."
    )
    .unwrap();
    for nm in COMMITMENT_NAMES {
        writeln!(
            w,
            "const VK_{nm}: G1 = G1::from_coords_or_infinity({nm}_X, {nm}_Y)"
        )
        .unwrap();
    }
    writeln!(
        w,
        "const VK_X2: G2 = G2::from_coords_or_infinity(X2_X0, X2_X1, X2_Y0, X2_Y1)"
    )
    .unwrap();
    writeln!(w).unwrap();
    writeln!(w, "fn vk() -> plonk::Vk<{n}> {{").unwrap();
    writeln!(w, "    plonk::Vk<{n}>::from_consts(").unwrap();
    writeln!(w, "        POWER,").unwrap();
    writeln!(w, "        OMEGA,").unwrap();
    writeln!(w, "        K1,").unwrap();
    writeln!(w, "        K2,").unwrap();
    let cs: Vec<String> = COMMITMENT_NAMES.iter().map(|n| format!("VK_{n}")).collect();
    writeln!(w, "        [{}],", cs.join(", ")).unwrap();
    writeln!(w, "        VK_X2,").unwrap();
    writeln!(w, "    )").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    let sig = format!("verifyProof(uint256[24],uint256[{n}])");
    writeln!(w, "pub msg {msg} {{").unwrap();
    writeln!(w, "    #[selector = sol(\"{sig}\")]").unwrap();
    writeln!(
        w,
        "    VerifyProof {{ proof: [u256; 24], input: [u256; {n}] }} -> bool,"
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
        "        VerifyProof {{ proof, input }} -> bool uses (call: Call, ctx: Ctx) {{"
    )
    .unwrap();
    writeln!(w, "            plonk::verify_calldata(vk(), proof, input)").unwrap();
    writeln!(w, "        }}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "        VkDigest -> u256 {{").unwrap();
    writeln!(w, "            VK_DIGEST").unwrap();
    writeln!(w, "        }}").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w, "}}").unwrap();

    let tests_fe = match &opts.fixture {
        Some(f) => Some(emit_tests(vk, opts, f, &contract, &msg, &digest)?),
        None => None,
    };
    Ok(EmittedIngot {
        ingot_name,
        fe_toml,
        lib_fe: s,
        tests_fe,
    })
}

fn emit_tests(
    vk: &PlonkVk,
    opts: &PlonkEmitOptions,
    f: &PlonkFixture,
    contract: &str,
    msg: &str,
    digest: &[u8; 32],
) -> Result<String, VkError> {
    let n = vk.n_public;
    if f.public.len() != n {
        return Err(bad(format!(
            "public.json has {} inputs, key expects {n}",
            f.public.len()
        )));
    }
    for p in &f.proof.points {
        if p.is_identity() {
            return Err(bad("fixture proof has a point at infinity"));
        }
    }
    let words = f.proof.words();
    let mut s = String::new();
    let w = &mut s;
    writeln!(
        w,
        "//! GENERATED by fe-zk-gen {TOOL_VERSION} -- do not edit."
    )
    .unwrap();
    writeln!(w, "//! Key digest : 0x{}", hex::encode(digest)).unwrap();
    writeln!(
        w,
        "//! Fixture: one valid proof (from {}) and a battery of invalid ones;",
        opts.name
    )
    .unwrap();
    writeln!(
        w,
        "//! every safety check of the library has a failing case. All calls go through the"
    )
    .unwrap();
    writeln!(w, "//! deployed contract.").unwrap();
    writeln!(w, "use super::*").unwrap();
    writeln!(w, "use std::evm::{{Address, Evm}}").unwrap();
    writeln!(w, "use fe_zk::ec::{{G1, G2, g1_is_valid, g2_is_valid}}").unwrap();
    writeln!(w, "use fe_zk::plonk").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "const Q: u256 = 21888242871839275222246405745257275088696311157297823662689037894645226208583").unwrap();
    writeln!(w, "const R: u256 = {R_DEC}").unwrap();
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
    let ws: Vec<String> = words.iter().map(|x| x.to_string()).collect();
    writeln!(w, "fn proof_words() -> [u256; 24] {{").unwrap();
    writeln!(w, "    [").unwrap();
    for c in ws.chunks(2) {
        writeln!(w, "        {},", c.join(", ")).unwrap();
    }
    writeln!(w, "    ]").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    let xs: Vec<String> = f.public.iter().map(|x| x.to_string()).collect();
    writeln!(w, "fn inputs() -> [u256; {n}] {{").unwrap();
    writeln!(w, "    [{}]", xs.join(", ")).unwrap();
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
    writeln!(
        w,
        "fn vp(_ p: Address, _ w: [u256; 24], _ x: [u256; {n}]) -> bool"
    )
    .unwrap();
    writeln!(w, "uses (evm: mut Evm)").unwrap();
    writeln!(w, "{{").unwrap();
    writeln!(w, "    let r: bool = evm").unwrap();
    writeln!(w, "        .call(addr: p, gas: 10000000, value: Wei::zero(), message: {msg}::VerifyProof {{ proof: w, input: x }})").unwrap();
    writeln!(w, "    r").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();

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

    t(w, "valid_proof_accepted", &format!(
        "let g0 = evm.gas()\nlet ok = vp(p, proof_words(), inputs())\nevm.emit(GasUsed {{ what: {n}, gas: g0 - evm.gas() }})\nassert(ok)"));
    t(w, "vk_digest_exposed", &format!(
        "let d: u256 = evm.call(addr: p, gas: 1000000, value: Wei::zero(), message: {msg}::VkDigest {{}})\nassert(d == VK_DIGEST)\nassert(d == 0x{})",
        hex::encode(digest)));
    t(w, "tampered_commitment_rejected",
        "// every one of the 9 commitments replaced by the (valid) generator\nvar i: usize = 0\nwhile i < 9 {\n    var w = proof_words()\n    w[2 * i] = 1\n    w[2 * i + 1] = 2\n    assert(!vp(p, w, inputs()))\n    i += 1\n}");
    t(w, "tampered_evaluation_rejected",
        "var i: usize = 0\nwhile i < 6 {\n    var w = proof_words()\n    w[18 + i] = (w[18 + i] + 1) % R\n    assert(!vp(p, w, inputs()))\n    i += 1\n}");
    t(w, "tampered_public_input_rejected",
        &format!("var i: usize = 0\nwhile i < {n} {{\n    var x = inputs()\n    x[i] = x[i] + 1\n    assert(!vp(p, proof_words(), x))\n    i += 1\n}}"));
    t(w, "non_canonical_public_input_rejected",
        &format!("// pub + r is the same residue: a verifier that reduces silently would accept it\nvar i: usize = 0\nwhile i < {n} {{\n    var x = inputs()\n    x[i] = x[i] + R\n    assert(!vp(p, proof_words(), x))\n    i += 1\n}}\nvar y = inputs()\ny[0] = R\nassert(!vp(p, proof_words(), y))\nvar z = inputs()\nz[0] = 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\nassert(!vp(p, proof_words(), z))"));
    t(w, "non_canonical_evaluation_rejected",
        "var i: usize = 0\nwhile i < 6 {\n    var w = proof_words()\n    w[18 + i] = w[18 + i] + R\n    assert(!vp(p, w, inputs()))\n    i += 1\n}\nvar w = proof_words()\nw[18] = R\nassert(!vp(p, w, inputs()))\nw[18] = 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\nassert(!vp(p, w, inputs()))");
    t(w, "non_canonical_coordinate_rejected",
        "// same point, coordinates shifted by q\nvar i: usize = 0\nwhile i < 9 {\n    var w = proof_words()\n    w[2 * i] = w[2 * i] + Q\n    w[2 * i + 1] = w[2 * i + 1] + Q\n    assert(!vp(p, w, inputs()))\n    i += 1\n}");
    t(w, "point_not_on_curve_rejected",
        "var i: usize = 0\nwhile i < 9 {\n    var w = proof_words()\n    w[2 * i + 1] = w[2 * i + 1] + 1\n    assert(!vp(p, w, inputs()))\n    i += 1\n}");
    t(w, "infinity_commitment_rejected",
        "// like the snarkjs verifier, proof commitments may not be the point at infinity\nvar i: usize = 0\nwhile i < 9 {\n    var w = proof_words()\n    w[2 * i] = 0\n    w[2 * i + 1] = 0\n    assert(!vp(p, w, inputs()))\n    i += 1\n}");
    if n >= 2 && f.public[0] != f.public[1] {
        t(w, "swapped_public_inputs_rejected",
            "var x = inputs()\nlet t = x[0]\nx[0] = x[1]\nx[1] = t\nassert(!vp(p, proof_words(), x))");
    }
    t(
        w,
        "all_zero_public_inputs_rejected",
        &format!(
            "assert(!vp(p, proof_words(), [{}]))",
            vec!["0"; n].join(", ")
        ),
    );
    t(w, "all_zero_proof_rejected",
        "var w = proof_words()\nvar i: usize = 0\nwhile i < 24 {\n    w[i] = 0\n    i += 1\n}\nassert(!vp(p, w, inputs()))");

    writeln!(w, "/// The key constants pass the full checks of the library (canonical, on curve, G2 subgroup,").unwrap();
    writeln!(w, "/// evaluation domain), also at runtime.").unwrap();
    writeln!(w, "#[test]").unwrap();
    writeln!(w, "fn vk_constants_pass_full_checks() {{").unwrap();
    for nm in COMMITMENT_NAMES {
        writeln!(w, "    assert(G1::from_coords({nm}_X, {nm}_Y).is_some())").unwrap();
    }
    writeln!(
        w,
        "    assert(G2::from_coords(X2_X0, X2_X1, X2_Y0, X2_Y1).is_some())"
    )
    .unwrap();
    writeln!(w, "    assert(plonk::domain_is_valid(POWER, OMEGA, K1, K2))").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "/// What `static_assert` rejects at compile time, checked at runtime: each of these constants").unwrap();
    writeln!(w, "/// would make the generated verifier fail to compile.").unwrap();
    writeln!(w, "#[test]").unwrap();
    writeln!(w, "fn invalid_keys_fail_the_compile_time_checks() {{").unwrap();
    writeln!(
        w,
        "    // [x]_2 on the twist but outside the order-r subgroup"
    )
    .unwrap();
    writeln!(
        w,
        "    assert(!g2_is_valid(BAD_X0, BAD_X1, BAD_Y0, BAD_Y1))"
    )
    .unwrap();
    writeln!(w, "    // [x]_2 off the twist, non-canonical").unwrap();
    writeln!(
        w,
        "    assert(!g2_is_valid(X2_X0, X2_X1, X2_Y0, X2_Y1 + 1))"
    )
    .unwrap();
    writeln!(
        w,
        "    assert(!g2_is_valid(X2_X0 + Q, X2_X1, X2_Y0, X2_Y1))"
    )
    .unwrap();
    writeln!(w, "    // commitment off the curve / non-canonical").unwrap();
    writeln!(w, "    assert(!g1_is_valid(S1_X, S1_Y + 1))").unwrap();
    writeln!(w, "    assert(!g1_is_valid(S1_X + Q, S1_Y))").unwrap();
    writeln!(
        w,
        "    // domain: wrong power, w = 1, w not canonical, k1 = k2, k = 0, k in the domain"
    )
    .unwrap();
    writeln!(
        w,
        "    assert(!plonk::domain_is_valid(POWER + 1, OMEGA, K1, K2))"
    )
    .unwrap();
    writeln!(w, "    assert(!plonk::domain_is_valid(POWER, 1, K1, K2))").unwrap();
    writeln!(
        w,
        "    assert(!plonk::domain_is_valid(POWER, OMEGA + R, K1, K2))"
    )
    .unwrap();
    writeln!(
        w,
        "    assert(!plonk::domain_is_valid(POWER, OMEGA, K1, K1))"
    )
    .unwrap();
    writeln!(w, "    assert(!plonk::domain_is_valid(POWER, OMEGA, 0, K2))").unwrap();
    writeln!(w, "    assert(!plonk::domain_is_valid(POWER, OMEGA, 1, K2))").unwrap();
    writeln!(w, "}}").unwrap();
    Ok(s)
}
