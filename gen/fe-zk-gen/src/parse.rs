//! Parsers for untrusted key files. They only structure the data; `Groth16Vk::validate` judges it.

use crate::model::{Groth16Vk, VkError, G1, G2};
use num_bigint::BigUint;
use num_traits::{Num, Zero};
use serde_json::Value;

fn perr<T>(m: impl Into<String>) -> Result<T, VkError> {
    Err(VkError::Parse(m.into()))
}

// ---------------------------------------------------------------- snarkjs

fn dec(v: &Value, what: &str) -> Result<BigUint, VkError> {
    let s = v
        .as_str()
        .ok_or_else(|| VkError::Parse(format!("{what}: expected decimal string")))?;
    BigUint::from_str_radix(s, 10).map_err(|_| VkError::Parse(format!("{what}: bad decimal '{s}'")))
}

fn arr<'a>(v: &'a Value, what: &str, n: usize) -> Result<&'a Vec<Value>, VkError> {
    match v.as_array() {
        Some(a) if a.len() == n => Ok(a),
        _ => perr(format!("{what}: expected array of {n}")),
    }
}

fn snarkjs_g1(v: &Value, what: &str) -> Result<G1, VkError> {
    let a = arr(v, what, 3)?;
    let (x, y, z) = (dec(&a[0], what)?, dec(&a[1], what)?, dec(&a[2], what)?);
    if z.is_zero() {
        // snarkjs encodes infinity as ["0","1","0"]; anything else with z = 0 is garbage.
        if !x.is_zero() || y != BigUint::from(1u8) {
            return perr(format!("{what}: malformed infinity encoding"));
        }
        return Ok(G1::identity());
    }
    if z != BigUint::from(1u8) {
        return perr(format!("{what}: projective z must be 1 (got {z})"));
    }
    Ok(G1::new(x, y))
}

fn snarkjs_fq2(v: &Value, what: &str) -> Result<[BigUint; 2], VkError> {
    let a = arr(v, what, 2)?;
    Ok([dec(&a[0], what)?, dec(&a[1], what)?])
}

fn snarkjs_g2(v: &Value, what: &str) -> Result<G2, VkError> {
    let a = arr(v, what, 3)?;
    let x = snarkjs_fq2(&a[0], what)?;
    let y = snarkjs_fq2(&a[1], what)?;
    let z = snarkjs_fq2(&a[2], what)?;
    if z[0].is_zero() && z[1].is_zero() {
        if !x.iter().all(|c| c.is_zero()) || y != [BigUint::from(1u8), BigUint::zero()] {
            return perr(format!("{what}: malformed infinity encoding"));
        }
        return Ok(G2::identity());
    }
    if z != [BigUint::from(1u8), BigUint::zero()] {
        return perr(format!("{what}: projective z must be [1,0]"));
    }
    Ok(G2 { x, y })
}

/// Parse a snarkjs `verification_key.json` (protocol groth16, curve bn128). Fq2 elements are
/// `[c0, c1]`. Extra fields (e.g. `vk_alphabeta_12`) are ignored.
pub fn parse_snarkjs(json: &str) -> Result<Groth16Vk, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    if v["protocol"] != "groth16" {
        return perr("protocol must be groth16");
    }
    if v["curve"] != "bn128" {
        return perr("curve must be bn128");
    }
    let n_public = v["nPublic"]
        .as_u64()
        .ok_or_else(|| VkError::Parse("nPublic missing".into()))? as usize;
    let ic_json = v["IC"]
        .as_array()
        .ok_or_else(|| VkError::Parse("IC missing".into()))?;
    let mut ic = Vec::with_capacity(ic_json.len());
    for (i, p) in ic_json.iter().enumerate() {
        ic.push(snarkjs_g1(p, &format!("IC[{i}]"))?);
    }
    Ok(Groth16Vk {
        n_public,
        alpha_g1: snarkjs_g1(&v["vk_alpha_1"], "vk_alpha_1")?,
        beta_g2: snarkjs_g2(&v["vk_beta_2"], "vk_beta_2")?,
        gamma_g2: snarkjs_g2(&v["vk_gamma_2"], "vk_gamma_2")?,
        delta_g2: snarkjs_g2(&v["vk_delta_2"], "vk_delta_2")?,
        ic,
    })
}

// -------------------------------------------------------------- arkworks

/// Format tag of the arkworks JSON written by `examples/arkworks_fixture.rs`.
pub const ARKWORKS_FORMAT: &str = "arkworks-groth16-bn254-uncompressed-v1";

fn hex_bytes(v: &Value, what: &str, len: usize) -> Result<Vec<u8>, VkError> {
    let s = v
        .as_str()
        .ok_or_else(|| VkError::Parse(format!("{what}: expected hex string")))?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    let b = hex::decode(s).map_err(|e| VkError::Parse(format!("{what}: {e}")))?;
    if b.len() != len {
        return perr(format!("{what}: expected {len} bytes, got {}", b.len()));
    }
    Ok(b)
}

/// Split off the arkworks serialization flags (top two bits of the last byte).
/// 0x00 / 0x80 = finite point (0x80 is ark's redundant "y is negative" flag), 0x40 = infinity. Returns the cleared bytes and whether infinite.
fn take_flags(mut b: Vec<u8>, what: &str) -> Result<(Vec<u8>, bool), VkError> {
    let last = b.len() - 1;
    let flags = b[last] & 0xC0;
    b[last] &= 0x3F;
    match flags {
        0x00 | 0x80 => Ok((b, false)),
        0x40 => {
            if b.iter().any(|&x| x != 0) {
                return perr(format!("{what}: infinity flag with non-zero coordinates"));
            }
            Ok((b, true))
        }
        f => perr(format!("{what}: unexpected flag bits 0x{f:02x}")),
    }
}

/// Raw little-endian 32-byte integer (NOT reduced: canonicity is checked by validation).
fn le(b: &[u8]) -> BigUint {
    BigUint::from_bytes_le(b)
}

/// arkworks `serialize_uncompressed` of a G1Affine: x (32 LE) || y (32 LE, flags in last byte).
fn ark_g1(v: &Value, what: &str) -> Result<G1, VkError> {
    let (b, inf) = take_flags(hex_bytes(v, what, 64)?, what)?;
    if inf {
        return Ok(G1::identity());
    }
    Ok(G1::new(le(&b[0..32]), le(&b[32..64])))
}

/// G2Affine uncompressed: x.c0 || x.c1 || y.c0 || y.c1, each 32 LE, flags in the last byte.
fn ark_g2(v: &Value, what: &str) -> Result<G2, VkError> {
    let (b, inf) = take_flags(hex_bytes(v, what, 128)?, what)?;
    if inf {
        return Ok(G2::identity());
    }
    Ok(G2 {
        x: [le(&b[0..32]), le(&b[32..64])],
        y: [le(&b[64..96]), le(&b[96..128])],
    })
}

/// Parse the arkworks verifying-key JSON:
/// `{"format": ARKWORKS_FORMAT, "n_public": N, "alpha_g1": hex, "beta_g2": hex, "gamma_g2": hex,
///   "delta_g2": hex, "gamma_abc_g1": [hex, ...]}` where each hex string is ark-serialize
/// `serialize_uncompressed` output of the point.
pub fn parse_arkworks(json: &str) -> Result<Groth16Vk, VkError> {
    let v: Value = serde_json::from_str(json).map_err(|e| VkError::Parse(e.to_string()))?;
    if v["format"] != ARKWORKS_FORMAT {
        return perr(format!("format must be {ARKWORKS_FORMAT}"));
    }
    let n_public = v["n_public"]
        .as_u64()
        .ok_or_else(|| VkError::Parse("n_public missing".into()))? as usize;
    let abc = v["gamma_abc_g1"]
        .as_array()
        .ok_or_else(|| VkError::Parse("gamma_abc_g1 missing".into()))?;
    let mut ic = Vec::with_capacity(abc.len());
    for (i, p) in abc.iter().enumerate() {
        ic.push(ark_g1(p, &format!("IC[{i}]"))?);
    }
    Ok(Groth16Vk {
        n_public,
        alpha_g1: ark_g1(&v["alpha_g1"], "alpha_g1")?,
        beta_g2: ark_g2(&v["beta_g2"], "beta_g2")?,
        gamma_g2: ark_g2(&v["gamma_g2"], "gamma_g2")?,
        delta_g2: ark_g2(&v["delta_g2"], "delta_g2")?,
        ic,
    })
}
