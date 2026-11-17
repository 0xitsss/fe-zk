//! Raw (unvalidated) verifying-key model. Coordinates are plain integers exactly as read from
//! the input; nothing is reduced. `Groth16Vk::validate` decides whether the key is acceptable.

use num_bigint::BigUint;
use num_traits::Zero;
use thiserror::Error;

/// BN254 G1 affine point. The point at infinity is encoded as (0, 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct G1 {
    pub x: BigUint,
    pub y: BigUint,
}

/// BN254 G2 affine point over Fq2 = Fq[u]/(u^2+1). Each Fq2 element is stored as `[c0, c1]`
/// meaning `c0 + c1*u` (snarkjs / arkworks order, real part first). Identity is all zeros.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct G2 {
    pub x: [BigUint; 2],
    pub y: [BigUint; 2],
}

impl G1 {
    pub fn new(x: BigUint, y: BigUint) -> Self {
        G1 { x, y }
    }
    pub fn identity() -> Self {
        G1 {
            x: BigUint::zero(),
            y: BigUint::zero(),
        }
    }
    pub fn is_identity(&self) -> bool {
        self.x.is_zero() && self.y.is_zero()
    }
}

impl G2 {
    pub fn identity() -> Self {
        G2 {
            x: [BigUint::zero(), BigUint::zero()],
            y: [BigUint::zero(), BigUint::zero()],
        }
    }
    pub fn is_identity(&self) -> bool {
        self.x.iter().chain(self.y.iter()).all(|c| c.is_zero())
    }

    /// Words in the order the EVM pairing precompile (0x08, EIP-197) expects for a G2 point:
    /// `[x.c1, x.c0, y.c1, y.c0]` (imaginary part FIRST). JSON / arkworks store `[c0, c1]`.
    pub fn evm_words(&self) -> [BigUint; 4] {
        [
            self.x[1].clone(),
            self.x[0].clone(),
            self.y[1].clone(),
            self.y[0].clone(),
        ]
    }
}

/// Groth16 verifying key over BN254. `ic[0]` is the constant term, `ic[1..]` pair with the
/// public inputs, so a well-formed key has `ic.len() == n_public + 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Groth16Vk {
    pub n_public: usize,
    pub alpha_g1: G1,
    pub beta_g2: G2,
    pub gamma_g2: G2,
    pub delta_g2: G2,
    pub ic: Vec<G1>,
}

#[derive(Debug, Error)]
pub enum VkError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("non-canonical field element (>= q) in {0}")]
    NonCanonical(String),
    #[error("point not on curve: {0}")]
    NotOnCurve(String),
    #[error("point not in the prime-order subgroup: {0}")]
    NotInSubgroup(String),
    #[error("{0} must not be the point at infinity")]
    Identity(String),
    #[error("wrong IC count: expected nPublic + 1 = {expected}, got {got}")]
    IcCount { expected: usize, got: usize },
    #[error("emission not implemented yet")]
    NotImplemented,
}
