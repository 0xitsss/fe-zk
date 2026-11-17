//! Key digest. keccak256 over the canonical encoding documented in gen/README.md.

use crate::model::{Groth16Vk, VkError, G1, G2};
use num_bigint::BigUint;
use sha3::{Digest, Keccak256};

/// ASCII domain tag (no terminator) that starts the preimage.
pub const DIGEST_DOMAIN: &[u8] = b"fe-zk/groth16/bn254/vk/v1";

fn word(out: &mut Vec<u8>, v: &BigUint) {
    let b = v.to_bytes_be();
    assert!(b.len() <= 32, "digest is only defined for validated keys");
    out.extend(std::iter::repeat(0u8).take(32 - b.len()));
    out.extend(b);
}

fn g1(out: &mut Vec<u8>, p: &G1) {
    word(out, &p.x);
    word(out, &p.y);
}

fn g2(out: &mut Vec<u8>, p: &G2) {
    // [c0, c1] order, i.e. the JSON order, NOT the EVM precompile order.
    word(out, &p.x[0]);
    word(out, &p.x[1]);
    word(out, &p.y[0]);
    word(out, &p.y[1]);
}

/// The canonical preimage. Only call on a validated key (the function validates first).
pub fn key_preimage(vk: &Groth16Vk) -> Result<Vec<u8>, VkError> {
    vk.validate()?;
    let mut out = Vec::new();
    out.extend_from_slice(DIGEST_DOMAIN);
    word(&mut out, &BigUint::from(vk.n_public));
    g1(&mut out, &vk.alpha_g1);
    g2(&mut out, &vk.beta_g2);
    g2(&mut out, &vk.gamma_g2);
    g2(&mut out, &vk.delta_g2);
    word(&mut out, &BigUint::from(vk.ic.len()));
    for p in &vk.ic {
        g1(&mut out, p);
    }
    Ok(out)
}

/// keccak256 of the canonical preimage. Fails if the key does not validate.
pub fn key_digest(vk: &Groth16Vk) -> Result<[u8; 32], VkError> {
    let pre = key_preimage(vk)?;
    let mut h = Keccak256::new();
    h.update(&pre);
    Ok(h.finalize().into())
}
