//! gnark PLONK (BN254, KZG, gnark v0.12.0) : key model, parser, validation (ark-bn254), digest and
//! Fe emitter.
//!
//! Same pipeline as the other emitters: `parse_gnark_vk` (raw, untrusted) -> `GnarkVk::validate`
//! (ark-bn254, independent of the emitted code) -> `gnark_key_digest` -> `emit_gnark_ingot`.
//! The emitted contract has the ABI of gnark's own Solidity verifier,
//! `Verify(bytes proof, uint256[] public_inputs) returns (bool)`, and forwards both dynamic
//! arguments to `gnark_plonk_verify_bytes` in fe_zk, which rejects every length other than the
//! key's fixed sizes (safety rule 4) and does all proof checks. The key is validated a second time
//! by the Fe compiler (`static_assert`).
//!
//! Input: the JSON exported by `vectors/gnark` (`fixture.json`, words as `0x` hex): the gnark
//! `VerifyingKey` fields `size`, `size_inv`, `generator`, `coset_shift`, `nb_public`, `s`, `ql`,
//! `qr`, `qm`, `qo`, `qk`, `qcp`, `commit_constraint_ix`, `g1`, `g2`, `g2_x` (G2 coordinates are
//! `x.real, x.imag, y.real, y.imag`), and optionally `proof_words` (gnark `MarshalSolidity()`
//! words) and `public` for the generated tests.

use crate::emit::{bad, check_name, EmittedIngot};
use crate::model::{VkError, G1, G2};
use crate::validate::{check_g1, check_g2, g1_from_ark, g2_from_ark};
use crate::TOOL_VERSION;
use ark_bn254::{Fr, G1Affine, G2Affine};
use ark_ec::AffineRepr;
use ark_ff::{BigInteger, Field, One, PrimeField, Zero};
use num_bigint::BigUint;
use num_traits::Num;
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::fmt::Write;

/// ASCII domain tag that starts the digest preimage.
pub const GNARK_DIGEST_DOMAIN: &[u8] = b"fe-zk/gnark-plonk/bn254/vk/v1";

pub const R_DEC: &str =
    "21888242871839275222246405745257275088548364400416034343698204186575808495617";

/// Names of the eight fixed key commitments, in the order of `GnarkVk::from_consts`.
pub const GNARK_COMMITMENT_NAMES: [&str; 8] = ["QL", "QR", "QM", "QO", "QK", "S1", "S2", "S3"];

/// Raw gnark PLONK verifying key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GnarkVk {
    pub n_public: usize,
    /// Domain size `n` (a power of two).
    pub size: u64,
    pub size_inv: BigUint,
    pub generator: BigUint,
    pub coset_shift: BigUint,
    /// Ql, Qr, Qm, Qo, Qk, S1, S2, S3
    pub commitments: [G1; 8],
    pub qcp: Vec<G1>,
    pub cci: Vec<u64>,
    /// KZG G1 generator (must be the BN254 generator)
    pub g1: G1,
    /// KZG G2 generator (must be the BN254 generator)
    pub g2: G2,
    /// `[x]_2`
    pub x2: G2,
}

impl GnarkVk {
    pub fn power(&self) -> u32 {
        self.size.trailing_zeros()
    }
    pub fn n_commit(&self) -> usize {
        self.qcp.len()
    }
}

/// A gnark proof (`MarshalSolidity()` words) and its public inputs, for the generated tests.
#[derive(Clone, Debug)]
pub struct GnarkFixture {
    pub proof_words: Vec<BigUint>,
    pub public: Vec<BigUint>,
}

fn hex_word(v: &Value, what: &str) -> Result<BigUint, VkError> {
    let s = v
        .as_str()
        .ok_or_else(|| bad(format!("{what}: expected hex string")))?;
    let h = s
        .strip_prefix("0x")
        .ok_or_else(|| bad(format!("{what}: expected 0x-prefixed hex, got '{s}'")))?;
    BigUint::from_str_radix(h, 16).map_err(|_| bad(format!("{what}: bad hex '{s}'")))
}

fn words(v: &Value, what: &str, n: usize) -> Result<Vec<BigUint>, VkError> {
    let a = v
        .as_array()
        .filter(|a| a.len() == n)
        .ok_or_else(|| bad(format!("{what}: expected array of {n} words")))?;
    a.iter().map(|x| hex_word(x, what)).collect()
}

fn g1_of(v: &Value, what: &str) -> Result<G1, VkError> {
    let w = words(v, what, 2)?;
    Ok(G1::new(w[0].clone(), w[1].clone()))
}

fn g2_of(v: &Value, what: &str) -> Result<G2, VkError> {
    let w = words(v, what, 4)?;
    Ok(G2 {
        x: [w[0].clone(), w[1].clone()],
        y: [w[2].clone(), w[3].clone()],
    })
}

pub fn parse_gnark_vk(json: &str) -> Result<GnarkVk, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    let n_public = v["nb_public"]
        .as_u64()
        .ok_or_else(|| bad("nb_public missing"))? as usize;
    let size = v["size"].as_u64().ok_or_else(|| bad("size missing"))?;
    let one = |k: &str| hex_word(&v[k], k);
    let mut cs = Vec::new();
    for k in ["ql", "qr", "qm", "qo", "qk"] {
        cs.push(g1_of(&v[k], k)?);
    }
    let s = v["s"]
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| bad("s: expected 3 points"))?;
    for (i, p) in s.iter().enumerate() {
        cs.push(g1_of(p, &format!("s[{i}]"))?);
    }
    let qcp = v["qcp"]
        .as_array()
        .ok_or_else(|| bad("qcp: expected array"))?
        .iter()
        .enumerate()
        .map(|(i, p)| g1_of(p, &format!("qcp[{i}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let cci = v["commit_constraint_ix"]
        .as_array()
        .ok_or_else(|| bad("commit_constraint_ix: expected array"))?
        .iter()
        .map(|x| {
            x.as_u64()
                .ok_or_else(|| bad("commit_constraint_ix: expected non-negative integers"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GnarkVk {
        n_public,
        size,
        size_inv: one("size_inv")?,
        generator: one("generator")?,
        coset_shift: one("coset_shift")?,
        commitments: cs.try_into().unwrap(),
        qcp,
        cci,
        g1: g1_of(&v["g1"], "g1")?,
        g2: g2_of(&v["g2"], "g2")?,
        x2: g2_of(&v["g2_x"], "g2_x")?,
    })
}

/// `proof_words` of a fixture JSON (gnark `MarshalSolidity()` as 32-byte words).
pub fn parse_gnark_proof_words(json: &str) -> Result<Vec<BigUint>, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    v["proof_words"]
        .as_array()
        .ok_or_else(|| bad("proof_words missing"))?
        .iter()
        .map(|x| hex_word(x, "proof_words"))
        .collect()
}

/// Public inputs: a JSON array, or an object with a `public` array; each entry a `0x` hex or
/// decimal string.
pub fn parse_gnark_public(json: &str) -> Result<Vec<BigUint>, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    let a = if v.is_array() { &v } else { &v["public"] };
    a.as_array()
        .ok_or_else(|| bad("public: expected an array"))?
        .iter()
        .map(|x| {
            let s = x.as_str().ok_or_else(|| bad("public: expected strings"))?;
            match s.strip_prefix("0x") {
                Some(h) => BigUint::from_str_radix(h, 16),
                None => BigUint::from_str_radix(s, 10),
            }
            .map_err(|_| bad(format!("public: bad number '{s}'")))
        })
        .collect()
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

impl GnarkVk {
    /// Full validation. Every failure is an error; nothing is reduced or repaired.
    ///  * `n_public >= 1`, `n_public <= size`, `size` a power of two with `2 <= size <= 2^28`
    ///  * `generator`, `coset_shift`, `size_inv` canonical (`< r`); `size_inv * size = 1`
    ///  * `generator` has order exactly `size` (`w^(size/2) = -1`)
    ///  * `coset_shift` nonzero with `k^n != 1` and `k^(2n) != 1` (three distinct cosets)
    ///  * `qcp.len() == cci.len()` (NC), every `cci[j] < size` and `n_public + cci[j] < size`
    ///  * the eight selector/permutation commitments and every `qcp` commitment canonical, on
    ///    curve, in the subgroup and NOT the point at infinity (the Fe verifier refuses them: a
    ///    zero selector would make gnark's byte encoding of the key ambiguous)
    ///  * `g1` and `g2` are the BN254 generators (the Fe verifier hard-wires them)
    ///  * `[x]_2` canonical, on the twist, in the order-r subgroup, not the identity
    pub fn validate(&self) -> Result<(), VkError> {
        if self.n_public == 0 {
            return Err(VkError::Domain("nb_public must be >= 1".into()));
        }
        if self.size < 2 || !self.size.is_power_of_two() || self.size > 1 << 28 {
            return Err(VkError::Domain(format!(
                "size {} is not a power of two in 2..=2^28",
                self.size
            )));
        }
        let n = self.size;
        if self.n_public as u64 > n {
            return Err(VkError::Domain(format!(
                "nb_public {} exceeds the domain size {n}",
                self.n_public
            )));
        }
        let w = to_fr(&self.generator, "generator")?;
        let k = to_fr(&self.coset_shift, "coset_shift")?;
        let size_inv = to_fr(&self.size_inv, "size_inv")?;
        if size_inv * Fr::from(n) != Fr::one() {
            return Err(VkError::Domain("size_inv is not the inverse of size".into()));
        }
        if w.pow([n / 2]) != -Fr::one() {
            return Err(VkError::Domain(format!(
                "generator is not a primitive {n}-th root of unity"
            )));
        }
        let kn = k.pow([n]);
        if k.is_zero() || kn == Fr::one() || kn * kn == Fr::one() {
            return Err(VkError::Domain(
                "coset_shift does not give three distinct cosets of the domain".into(),
            ));
        }
        if self.qcp.len() != self.cci.len() {
            return Err(VkError::Domain(format!(
                "{} BSB22 selector commitments but {} constraint indices",
                self.qcp.len(),
                self.cci.len()
            )));
        }
        if self.qcp.len() > 1000 {
            return Err(VkError::Domain("more than 1000 BSB22 commitments".into()));
        }
        for (j, &c) in self.cci.iter().enumerate() {
            if c >= n || self.n_public as u64 + c >= n {
                return Err(VkError::Domain(format!(
                    "commit_constraint_ix[{j}] = {c} is outside the domain (nb_public + index must be < {n})"
                )));
            }
        }
        const NAMES: [&str; 8] = ["Ql", "Qr", "Qm", "Qo", "Qk", "S1", "S2", "S3"];
        for (i, c) in self.commitments.iter().enumerate() {
            check_g1(c, NAMES[i])?;
            if c.is_identity() {
                return Err(VkError::Identity(NAMES[i].into()));
            }
        }
        for (j, c) in self.qcp.iter().enumerate() {
            check_g1(c, &format!("Qcp[{j}]"))?;
            if c.is_identity() {
                return Err(VkError::Identity(format!("Qcp[{j}]")));
            }
        }
        check_g1(&self.g1, "g1")?;
        if self.g1 != g1_from_ark(&G1Affine::generator()) {
            return Err(VkError::Domain("g1 is not the BN254 G1 generator".into()));
        }
        check_g2(&self.g2, "g2")?;
        if self.g2 != g2_from_ark(&G2Affine::generator()) {
            return Err(VkError::Domain("g2 is not the BN254 G2 generator".into()));
        }
        if self.x2.is_identity() {
            return Err(VkError::Identity("g2_x".into()));
        }
        check_g2(&self.x2, "g2_x")?;
        Ok(())
    }
}

fn word(out: &mut Vec<u8>, v: &BigUint) {
    let b = v.to_bytes_be();
    assert!(b.len() <= 32, "digest is only defined for validated keys");
    out.extend(std::iter::repeat(0u8).take(32 - b.len()));
    out.extend(b);
}

/// Canonical preimage: domain tag, nb_public, power, NC, generator, coset_shift, then Ql Qr Qm Qo
/// Qk S1 S2 S3 (x, y each), the NC Qcp points, the NC constraint indices and `[x]_2` as
/// x.real, x.imag, y.real, y.imag (JSON order), all as 32-byte big-endian words.
pub fn gnark_key_preimage(vk: &GnarkVk) -> Result<Vec<u8>, VkError> {
    vk.validate()?;
    let mut out = Vec::new();
    out.extend_from_slice(GNARK_DIGEST_DOMAIN);
    word(&mut out, &BigUint::from(vk.n_public));
    word(&mut out, &BigUint::from(vk.power()));
    word(&mut out, &BigUint::from(vk.n_commit()));
    word(&mut out, &vk.generator);
    word(&mut out, &vk.coset_shift);
    for c in vk.commitments.iter().chain(vk.qcp.iter()) {
        word(&mut out, &c.x);
        word(&mut out, &c.y);
    }
    for c in &vk.cci {
        word(&mut out, &BigUint::from(*c));
    }
    for c in vk.x2.x.iter().chain(vk.x2.y.iter()) {
        word(&mut out, c);
    }
    Ok(out)
}

pub fn gnark_key_digest(vk: &GnarkVk) -> Result<[u8; 32], VkError> {
    let mut h = Keccak256::new();
    h.update(gnark_key_preimage(vk)?);
    Ok(h.finalize().into())
}

// ---------------------------------------------------------------- emitter

#[derive(Clone, Debug)]
pub struct GnarkEmitOptions {
    pub name: String,
    pub source: String,
    pub fe_zk_path: String,
    pub fixture: Option<GnarkFixture>,
}

impl Default for GnarkEmitOptions {
    fn default() -> Self {
        GnarkEmitOptions {
            name: "Gnark".into(),
            source: "fixture.json".into(),
            fe_zk_path: "../..".into(),
            fixture: None,
        }
    }
}

/// Emit only the verifier module with default options.
pub fn emit_gnark_fe(vk: &GnarkVk) -> Result<String, VkError> {
    Ok(emit_gnark_ingot(vk, &GnarkEmitOptions::default())?.lib_fe)
}

pub fn emit_gnark_ingot(vk: &GnarkVk, opts: &GnarkEmitOptions) -> Result<EmittedIngot, VkError> {
    check_name(&opts.name)?;
    let digest = gnark_key_digest(vk)?; // validates
    let np = vk.n_public;
    let nc = vk.n_commit();
    let ingot_name = format!("{}_verifier", opts.name.to_lowercase());
    let contract = format!("{}Verifier", opts.name);
    let msg = format!("{contract}Msg");
    let fe_toml = format!(
        "[ingot]\nname = \"{ingot_name}\"\nversion = \"0.1.0\"\n\n[dependencies]\nfe_zk = {{ path = \"{}\", allow_unsafe = true }}\n",
        opts.fe_zk_path
    );
    let mut s = String::new();
    let w = &mut s;
    let l = |w: &mut String, t: &str| writeln!(w, "{t}").unwrap();
    l(w, &format!("//! GENERATED by fe-zk-gen {TOOL_VERSION} -- do not edit."));
    l(
        w,
        &format!(
            "//! PLONK (gnark v0.12.0, KZG) / BN254 verifier for {np} public input(s), {nc} BSB22 commitment(s), domain 2^{}.",
            vk.power()
        ),
    );
    l(w, &format!("//! Source key : {}", opts.source));
    l(w, &format!("//! Key digest : 0x{}", hex::encode(digest)));
    l(w, "//! (keccak256 over the canonical key encoding, see gen/README.md)");
    l(w, "//!");
    l(w, "//! The contract only forwards calldata to `gnark_plonk_verify_bytes` in fe_zk, which does all");
    l(w, "//! proof checks: exact proof and input lengths, canonical public inputs and evaluations,");
    l(w, "//! commitments on curve and not infinity, the sha256 transcript over the key, inputs and every");
    l(w, "//! commitment, pairing == 1. ABI-compatible with gnark's Solidity verifier (`vk.ExportSolidity`):");
    l(w, &format!("//! `Verify(bytes proof, uint256[] public_inputs) returns (bool)`, `proof` is `MarshalSolidity()` ({} words),", 24 + 3 * nc));
    l(w, &format!("//! `public_inputs` has exactly {np} entries. Any other length returns false (gnark's contract reverts)."));
    l(w, "use fe_zk::ec::{G1, G2, g1_is_valid, g2_is_valid}");
    l(w, "use fe_zk::verifier::{GnarkVk, gnark_domain_is_valid, gnark_plonk_verify_bytes}");
    l(w, "");
    l(w, &format!("pub const N_PUBLIC: u256 = {np}"));
    l(w, &format!("pub const N_COMMIT: u256 = {nc}"));
    l(w, &format!("pub const VK_DIGEST: u256 = 0x{}", hex::encode(digest)));
    l(w, "");
    l(w, "// Verifying key, validated by fe-zk-gen (canonical, on curve, in subgroup, generator of order 2^power,");
    l(w, "// distinct cosets, constraint indices inside the domain) and re-validated at compile time below.");
    l(w, "// G2 coordinates are (real, imaginary) parts.");
    l(w, &format!("pub const POWER: u256 = {}", vk.power()));
    l(w, &format!("pub const OMEGA: u256 = {}", vk.generator));
    l(w, &format!("pub const COSET_SHIFT: u256 = {}", vk.coset_shift));
    for (i, nm) in GNARK_COMMITMENT_NAMES.iter().enumerate() {
        l(w, &format!("pub const {nm}_X: u256 = {}", vk.commitments[i].x));
        l(w, &format!("pub const {nm}_Y: u256 = {}", vk.commitments[i].y));
    }
    for (j, p) in vk.qcp.iter().enumerate() {
        l(w, &format!("pub const QCP{j}_X: u256 = {}", p.x));
        l(w, &format!("pub const QCP{j}_Y: u256 = {}", p.y));
        l(w, &format!("pub const CCI{j}: u256 = {}", vk.cci[j]));
    }
    l(w, &format!("pub const X2_X0: u256 = {}", vk.x2.x[0]));
    l(w, &format!("pub const X2_X1: u256 = {}", vk.x2.x[1]));
    l(w, &format!("pub const X2_Y0: u256 = {}", vk.x2.y[0]));
    l(w, &format!("pub const X2_Y1: u256 = {}", vk.x2.y[1]));
    l(w, "");
    l(w, "// Compile-time validation (Fe CTFE): a key with an invalid point, a bad evaluation domain or a");
    l(w, "// constraint index outside the domain does not compile. Independent of the Rust-side validation");
    l(w, "// fe-zk-gen did before emitting, and free of gas (the G2 subgroup check alone is ~750k gas at runtime).");
    l(w, "// gnark keys never contain a commitment at infinity, so none is accepted.");
    l(w, "static_assert(N_PUBLIC > 0)");
    l(w, "static_assert(gnark_domain_is_valid(POWER, OMEGA, COSET_SHIFT))");
    l(w, "static_assert(N_PUBLIC <= (1 << POWER))");
    for nm in GNARK_COMMITMENT_NAMES {
        l(w, &format!("static_assert(g1_is_valid({nm}_X, {nm}_Y))"));
        l(w, &format!("static_assert(!VK_{nm}.is_infinity())"));
    }
    for j in 0..nc {
        l(w, &format!("static_assert(g1_is_valid(QCP{j}_X, QCP{j}_Y))"));
        l(w, &format!("static_assert(!VK_QCP{j}.is_infinity())"));
        l(w, &format!("static_assert(N_PUBLIC + CCI{j} < (1 << POWER))"));
    }
    l(w, "static_assert(g2_is_valid(X2_X0, X2_X1, X2_Y0, X2_Y1))");
    l(w, "static_assert(!VK_X2.is_infinity())");
    l(w, "");
    l(w, "// Typed key points. `from_coords_or_infinity` yields infinity for an invalid point, and");
    l(w, "// `GnarkVk::from_consts` poisons a key with a bad domain, a commitment at infinity or a bad");
    l(w, "// constraint index, which `gnark_plonk_verify` rejects, so no usable unchecked key can exist even");
    l(w, "// if the assertions above were removed.");
    for nm in GNARK_COMMITMENT_NAMES {
        l(w, &format!("const VK_{nm}: G1 = G1::from_coords_or_infinity({nm}_X, {nm}_Y)"));
    }
    for j in 0..nc {
        l(w, &format!("const VK_QCP{j}: G1 = G1::from_coords_or_infinity(QCP{j}_X, QCP{j}_Y)"));
    }
    l(w, "const VK_X2: G2 = G2::from_coords_or_infinity(X2_X0, X2_X1, X2_Y0, X2_Y1)");
    l(w, "");
    l(w, &format!("fn vk() -> GnarkVk<{np}, {nc}> {{"));
    let cs: Vec<String> = GNARK_COMMITMENT_NAMES.iter().map(|n| format!("VK_{n}")).collect();
    let qcp: Vec<String> = (0..nc).map(|j| format!("VK_QCP{j}")).collect();
    let cci: Vec<String> = (0..nc).map(|j| format!("CCI{j}")).collect();
    l(w, &format!("    let qcp: [G1; {nc}] = [{}]", qcp.join(", ")));
    l(w, &format!("    let cci: [u256; {nc}] = [{}]", cci.join(", ")));
    l(w, &format!("    GnarkVk<{np}, {nc}>::from_consts("));
    l(w, "        POWER,");
    l(w, "        OMEGA,");
    l(w, "        COSET_SHIFT,");
    l(w, &format!("        [{}],", cs.join(", ")));
    l(w, "        qcp,");
    l(w, "        cci,");
    l(w, "        VK_X2,");
    l(w, "    )");
    l(w, "}");
    l(w, "");
    l(w, &format!("pub msg {msg} {{"));
    l(w, "    #[selector = sol(\"Verify(bytes,uint256[])\")]");
    l(w, "    Verify { proof: Bytes, public_inputs: Vec<u256> } -> bool,");
    l(w, "    #[selector = sol(\"vkDigest()\")]");
    l(w, "    VkDigest -> u256,");
    l(w, "}");
    l(w, "");
    l(w, &format!("pub contract {contract} {{"));
    l(w, &format!("    recv {msg} {{"));
    l(w, "        Verify { proof, public_inputs } -> bool uses (call: Call) {");
    l(w, "            gnark_plonk_verify_bytes(vk(), proof, public_inputs)");
    l(w, "        }");
    l(w, "");
    l(w, "        VkDigest -> u256 {");
    l(w, "            VK_DIGEST");
    l(w, "        }");
    l(w, "    }");
    l(w, "}");

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
    vk: &GnarkVk,
    opts: &GnarkEmitOptions,
    f: &GnarkFixture,
    contract: &str,
    msg: &str,
    digest: &[u8; 32],
) -> Result<String, VkError> {
    let np = vk.n_public;
    let nc = vk.n_commit();
    let m = 24 + 3 * nc;
    if f.public.len() != np {
        return Err(bad(format!(
            "fixture has {} public inputs, key expects {np}",
            f.public.len()
        )));
    }
    if f.proof_words.len() != m {
        return Err(bad(format!(
            "fixture proof has {} words, key expects {m}",
            f.proof_words.len()
        )));
    }
    // point word pairs: L R O H0 H1 H2 (0..12), Z (17), W_zeta, W_zeta_omega (20, 22), BSB22
    let mut pts: Vec<usize> = vec![0, 2, 4, 6, 8, 10, 17, 20, 22];
    for j in 0..nc {
        pts.push(24 + nc + 2 * j);
    }
    // evaluations: l r o s1 s2 (12..17), Z(w zeta) (19), qcp openings (24..24+nc)
    let mut evs: Vec<usize> = vec![12, 13, 14, 15, 16, 19];
    for j in 0..nc {
        evs.push(24 + j);
    }
    for &i in &pts {
        let (x, y) = (&f.proof_words[i], &f.proof_words[i + 1]);
        if x.bits() == 0 && y.bits() == 0 {
            return Err(bad("fixture proof has a point at infinity"));
        }
    }
    let list = |v: &[usize]| {
        v.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut s = String::new();
    let w = &mut s;
    let l = |w: &mut String, t: &str| writeln!(w, "{t}").unwrap();
    l(w, &format!("//! GENERATED by fe-zk-gen {TOOL_VERSION} -- do not edit."));
    l(w, &format!("//! Key digest : 0x{}", hex::encode(digest)));
    l(w, &format!("//! Fixture: one valid proof (from {}) and a battery of invalid ones;", opts.name));
    l(w, "//! every safety check of the library has a failing case. All calls go through the");
    l(w, "//! deployed contract, with gnark's `Verify(bytes,uint256[])` ABI.");
    l(w, "use super::*");
    l(w, "use std::abi::{bytes_from_words, bytes_from_words_prefix, dyn_array_from}");
    l(w, "use std::evm::{Address, Evm}");
    l(w, "use fe_zk::ec::{G1, G2, g1_is_valid, g2_is_valid}");
    l(w, "use fe_zk::verifier::{GnarkVk, gnark_domain_is_valid, gnark_plonk_verify_bytes}");
    l(w, "use std::evm::Call");
    l(w, "");
    l(w, "const Q: u256 = 21888242871839275222246405745257275088696311157297823662689037894645226208583");
    l(w, &format!("const R: u256 = {R_DEC}"));
    l(w, "// On the twist but outside the order-r subgroup (py_ecc, scripts/ec-vectors.py)");
    l(w, "const BAD_X0: u256 = 2");
    l(w, "const BAD_X1: u256 = 1");
    l(w, "const BAD_Y0: u256 = 7292567877523311580221095596750716176434782432868683424513645834767876293070");
    l(w, "const BAD_Y1: u256 = 19659275751359636165940301690575149581329631496732780143538578556285923319774");
    l(w, "");
    l(w, "#[event]");
    l(w, "struct GasUsed { what: u256, gas: u256 }");
    l(w, "");
    let arr = |w: &mut String, name: &str, ws: &[BigUint]| {
        writeln!(w, "fn {name}() -> [u256; {}] {{", ws.len()).unwrap();
        writeln!(w, "    [").unwrap();
        for c in ws.chunks(2) {
            let v: Vec<String> = c.iter().map(|x| x.to_string()).collect();
            writeln!(w, "        {},", v.join(", ")).unwrap();
        }
        writeln!(w, "    ]").unwrap();
        writeln!(w, "}}").unwrap();
        writeln!(w).unwrap();
    };
    arr(w, "proof_words", &f.proof_words);
    let mut longer = f.proof_words.clone();
    longer.push(BigUint::from(0u8));
    arr(w, "proof_one_word_longer", &longer);
    arr(w, "proof_one_word_shorter", &f.proof_words[..m - 1]);
    let xs: Vec<String> = f.public.iter().map(|x| x.to_string()).collect();
    l(w, &format!("fn inputs() -> [u256; {np}] {{"));
    l(w, &format!("    [{}]", xs.join(", ")));
    l(w, "}");
    l(w, "");
    l(w, "fn deploy() -> Address uses (evm: mut Evm) {");
    l(w, &format!("    evm.create2<{contract}>(value: Wei::zero(), args: (), salt: 0)"));
    l(w, "}");
    l(w, "");
    l(w, &format!("fn vp(_ p: Address, _ w: [u256; {m}], _ x: [u256; {np}]) -> bool"));
    l(w, "uses (evm: mut Evm)");
    l(w, "{");
    l(w, "    vb(p, bytes_from_words(w), dyn_array_from(x))");
    l(w, "}");
    l(w, "");
    l(w, "fn vb(_ p: Address, _ proof: Bytes, _ x: Vec<u256>) -> bool");
    l(w, "uses (evm: mut Evm)");
    l(w, "{");
    l(w, "    let r: bool = evm");
    l(w, &format!("        .call(addr: p, gas: 10000000, value: Wei::zero(), message: {msg}::Verify {{ proof, public_inputs: x }})"));
    l(w, "    r");
    l(w, "}");
    l(w, "");

    let t = |w: &mut String, name: &str, body: &str| {
        writeln!(w, "#[test]").unwrap();
        writeln!(w, "fn {name}() uses (evm: mut Evm) {{").unwrap();
        writeln!(w, "    let p = deploy()").unwrap();
        for ln in body.lines() {
            writeln!(w, "    {ln}").unwrap();
        }
        writeln!(w, "}}").unwrap();
        writeln!(w).unwrap();
    };
    // loop over fixed index lists: `idx` is an array literal in the generated test
    let each = |idx: &[usize], body: &str| {
        format!(
            "let idx: [usize; {}] = [{}]\nvar k: usize = 0\nwhile k < {} {{\n    let i = idx[k]\n{}\n    k += 1\n}}",
            idx.len(),
            list(idx),
            idx.len(),
            body.lines().map(|x| format!("    {x}")).collect::<Vec<_>>().join("\n")
        )
    };
    let pt_pairs: Vec<usize> = pts.clone();

    t(w, "valid_proof_accepted", &format!(
        "let g0 = evm.gas()\nlet ok = vp(p, proof_words(), inputs())\nevm.emit(GasUsed {{ what: {np}, gas: g0 - evm.gas() }})\nassert(ok)"));
    t(w, "vk_digest_exposed", &format!(
        "let d: u256 = evm.call(addr: p, gas: 1000000, value: Wei::zero(), message: {msg}::VkDigest {{}})\nassert(d == VK_DIGEST)\nassert(d == 0x{})",
        hex::encode(digest)));
    t(w, "tampered_commitment_rejected", &format!(
        "// every proof commitment replaced by the (valid) generator\n{}",
        each(&pt_pairs, "var w = proof_words()\nw[i] = 1\nw[i + 1] = 2\nassert(!vp(p, w, inputs()))")));
    t(w, "tampered_evaluation_rejected", &each(&evs,
        "var w = proof_words()\nw[i] = (w[i] + 1) % R\nassert(!vp(p, w, inputs()))"));
    t(w, "tampered_public_input_rejected", &format!(
        "var i: usize = 0\nwhile i < {np} {{\n    var x = inputs()\n    x[i] = x[i] + 1\n    assert(!vp(p, proof_words(), x))\n    i += 1\n}}"));
    t(w, "non_canonical_public_input_rejected", &format!(
        "// pub + r is the same residue: a verifier that reduces silently would accept it\nvar i: usize = 0\nwhile i < {np} {{\n    var x = inputs()\n    x[i] = x[i] + R\n    assert(!vp(p, proof_words(), x))\n    i += 1\n}}\nvar y = inputs()\ny[0] = R\nassert(!vp(p, proof_words(), y))\nvar z = inputs()\nz[0] = 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\nassert(!vp(p, proof_words(), z))"));
    t(w, "non_canonical_evaluation_rejected", &format!(
        "{}\nvar w = proof_words()\nw[12] = R\nassert(!vp(p, w, inputs()))\nw[12] = 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\nassert(!vp(p, w, inputs()))",
        each(&evs, "var w = proof_words()\nw[i] = w[i] + R\nassert(!vp(p, w, inputs()))")));
    t(w, "non_canonical_coordinate_rejected", &format!(
        "// same point, coordinates shifted by q\n{}",
        each(&pt_pairs, "var w = proof_words()\nw[i] = w[i] + Q\nw[i + 1] = w[i + 1] + Q\nassert(!vp(p, w, inputs()))")));
    t(w, "point_not_on_curve_rejected", &each(&pt_pairs,
        "var w = proof_words()\nw[i + 1] = w[i + 1] + 1\nassert(!vp(p, w, inputs()))"));
    t(w, "infinity_commitment_rejected", &format!(
        "// gnark's byte encoding of infinity is not representable in the Solidity layout: refused\n{}",
        each(&pt_pairs, "var w = proof_words()\nw[i] = 0\nw[i + 1] = 0\nassert(!vp(p, w, inputs()))")));
    t(w, "wrong_proof_length_rejected", &format!(
        "// exactly {m} words ({} bytes) are required: never truncated, never padded\nassert(!vb(p, bytes_from_words(proof_one_word_longer()), dyn_array_from(inputs())))\nassert(!vb(p, bytes_from_words(proof_one_word_shorter()), dyn_array_from(inputs())))\nassert(!vb(p, bytes_from_words_prefix<{}, {m}>(proof_words()), dyn_array_from(inputs())))\nassert(!vb(p, bytes_from_words_prefix<{}, {}>(proof_one_word_longer()), dyn_array_from(inputs())))\nlet nothing: [u256; 0] = []\nassert(!vb(p, bytes_from_words(nothing), dyn_array_from(inputs())))",
        32 * m, 32 * m - 1, 32 * m + 1, m + 1));
    let mut more = f.public.clone();
    more.push(BigUint::from(0u8));
    let more_s: Vec<String> = more.iter().map(|x| x.to_string()).collect();
    let fewer_s: Vec<String> = f.public[..np - 1].iter().map(|x| x.to_string()).collect();
    t(w, "wrong_public_input_count_rejected", &format!(
        "// exactly {np} inputs are required: a caller can neither drop nor append one\nlet more: [u256; {}] = [{}]\nassert(!vb(p, bytes_from_words(proof_words()), dyn_array_from(more)))\nlet fewer: [u256; {}] = [{}]\nassert(!vb(p, bytes_from_words(proof_words()), dyn_array_from(fewer)))",
        np + 1, more_s.join(", "), np - 1, fewer_s.join(", ")));
    if np >= 2 && f.public[0] != f.public[1] {
        t(w, "swapped_public_inputs_rejected",
            "var x = inputs()\nlet t = x[0]\nx[0] = x[1]\nx[1] = t\nassert(!vp(p, proof_words(), x))");
    }
    t(w, "all_zero_public_inputs_rejected", &format!(
        "assert(!vp(p, proof_words(), [{}]))", vec!["0"; np].join(", ")));
    t(w, "all_zero_proof_rejected", &format!(
        "var w = proof_words()\nvar i: usize = 0\nwhile i < {m} {{\n    w[i] = 0\n    i += 1\n}}\nassert(!vp(p, w, inputs()))"));

    l(w, "/// The key constants pass the full checks of the library (canonical, on curve, G2 subgroup,");
    l(w, "/// evaluation domain), also at runtime.");
    l(w, "#[test]");
    l(w, "fn vk_constants_pass_full_checks() {");
    for nm in GNARK_COMMITMENT_NAMES {
        l(w, &format!("    assert(G1::from_coords({nm}_X, {nm}_Y).is_some())"));
    }
    for j in 0..nc {
        l(w, &format!("    assert(G1::from_coords(QCP{j}_X, QCP{j}_Y).is_some())"));
        l(w, &format!("    assert(N_PUBLIC + CCI{j} < (1 << POWER))"));
    }
    l(w, "    assert(G2::from_coords(X2_X0, X2_X1, X2_Y0, X2_Y1).is_some())");
    l(w, "    assert(gnark_domain_is_valid(POWER, OMEGA, COSET_SHIFT))");
    l(w, "}");
    l(w, "");
    l(w, "/// What `static_assert` rejects at compile time, checked at runtime: each of these constants");
    l(w, "/// would make the generated verifier fail to compile.");
    l(w, "#[test]");
    l(w, "fn invalid_keys_fail_the_compile_time_checks() {");
    l(w, "    // [x]_2 on the twist but outside the order-r subgroup");
    l(w, "    assert(!g2_is_valid(BAD_X0, BAD_X1, BAD_Y0, BAD_Y1))");
    l(w, "    // [x]_2 off the twist, non-canonical");
    l(w, "    assert(!g2_is_valid(X2_X0, X2_X1, X2_Y0, X2_Y1 + 1))");
    l(w, "    assert(!g2_is_valid(X2_X0 + Q, X2_X1, X2_Y0, X2_Y1))");
    l(w, "    // commitment off the curve / non-canonical");
    l(w, "    assert(!g1_is_valid(S1_X, S1_Y + 1))");
    l(w, "    assert(!g1_is_valid(S1_X + Q, S1_Y))");
    l(w, "    // domain: wrong power, w = 1, w not canonical, k = 0, k in the domain, k^2 in the domain");
    l(w, "    assert(!gnark_domain_is_valid(POWER + 1, OMEGA, COSET_SHIFT))");
    l(w, "    assert(!gnark_domain_is_valid(0, OMEGA, COSET_SHIFT))");
    l(w, "    assert(!gnark_domain_is_valid(POWER, 1, COSET_SHIFT))");
    l(w, "    assert(!gnark_domain_is_valid(POWER, OMEGA + R, COSET_SHIFT))");
    l(w, "    assert(!gnark_domain_is_valid(POWER, OMEGA, 0))");
    l(w, "    assert(!gnark_domain_is_valid(POWER, OMEGA, 1))");
    l(w, "}");
    l(w, "");
    l(w, "/// A key that fails the library checks is poisoned by `GnarkVk::from_consts`: it verifies nothing,");
    l(w, "/// not even the valid proof, even if the compile-time assertions were removed.");
    l(w, "fn key_with(_ omega: u256, _ cci0: u256) -> GnarkVk<%NP%, %NC%> {");
    l(w, "    let qcp: [G1; %NC%] = [%QCP%]");
    l(w, "    let cci: [u256; %NC%] = [%CCI%]");
    l(w, "    GnarkVk<%NP%, %NC%>::from_consts(POWER, omega, COSET_SHIFT, [%CS%], qcp, cci, VK_X2)");
    l(w, "}");
    l(w, "");
    l(w, "#[test]");
    l(w, "fn poisoned_keys_reject_every_proof() uses (call: Call) {");
    l(w, "    let ok = key_with(OMEGA, %CCI0%)");
    l(w, "    assert(gnark_plonk_verify_bytes(ok, bytes_from_words(proof_words()), dyn_array_from(inputs())))");
    l(w, "    let bad_domain = key_with(1, %CCI0%)");
    l(w, "    assert(!gnark_plonk_verify_bytes(bad_domain, bytes_from_words(proof_words()), dyn_array_from(inputs())))");
    if nc > 0 {
        l(w, "    // constraint index outside the domain");
        l(w, "    let bad_index = key_with(OMEGA, 1 << POWER)");
        l(w, "    assert(!gnark_plonk_verify_bytes(bad_index, bytes_from_words(proof_words()), dyn_array_from(inputs())))");
    }
    l(w, "}");
    let cs: Vec<String> = GNARK_COMMITMENT_NAMES.iter().map(|n| format!("VK_{n}")).collect();
    let qcp: Vec<String> = (0..nc).map(|j| format!("VK_QCP{j}")).collect();
    let cci: Vec<String> = (0..nc)
        .map(|j| if j == 0 { "cci0".to_string() } else { format!("CCI{j}") })
        .collect();
    let s = s
        .replace("%NP%", &np.to_string())
        .replace("%NC%", &nc.to_string())
        .replace("%QCP%", &qcp.join(", "))
        .replace("%CCI0%", if nc > 0 { "CCI0" } else { "0" })
        .replace("%CCI%", &cci.join(", "))
        .replace("%CS%", &cs.join(", "));
    Ok(s)
}
