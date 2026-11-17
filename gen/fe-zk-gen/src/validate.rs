//! Verifying-key validation (CLAUDE.md safety rule 9), done with ark-bn254 so the checks do not
//! depend on any code that will be emitted into Fe.

use crate::model::{Groth16Vk, VkError, G1, G2};
use ark_bn254::{Fq, Fq2, G1Affine, G2Affine};
use ark_ec::AffineRepr;
use ark_ff::{BigInteger, PrimeField};
use num_bigint::BigUint;

/// The BN254 base field modulus q.
pub fn fq_modulus() -> BigUint {
    BigUint::from_bytes_le(&Fq::MODULUS.to_bytes_le())
}

fn canonical(c: &BigUint, what: &str) -> Result<Fq, VkError> {
    if *c >= fq_modulus() {
        return Err(VkError::NonCanonical(what.to_string()));
    }
    Ok(Fq::from(c.clone()))
}

fn big(f: &Fq) -> BigUint {
    BigUint::from_bytes_le(&f.into_bigint().to_bytes_le())
}

/// Convert to an arkworks affine point without any curve check (caller validates).
pub fn g1_to_ark(p: &G1, what: &str) -> Result<G1Affine, VkError> {
    if p.is_identity() {
        return Ok(G1Affine::zero());
    }
    Ok(G1Affine::new_unchecked(
        canonical(&p.x, what)?,
        canonical(&p.y, what)?,
    ))
}

pub fn g2_to_ark(p: &G2, what: &str) -> Result<G2Affine, VkError> {
    if p.is_identity() {
        return Ok(G2Affine::zero());
    }
    let x = Fq2::new(canonical(&p.x[0], what)?, canonical(&p.x[1], what)?);
    let y = Fq2::new(canonical(&p.y[0], what)?, canonical(&p.y[1], what)?);
    Ok(G2Affine::new_unchecked(x, y))
}

pub fn g1_from_ark(p: &G1Affine) -> G1 {
    match p.xy() {
        Some((x, y)) => G1::new(big(&x), big(&y)),
        None => G1::identity(),
    }
}

pub fn g2_from_ark(p: &G2Affine) -> G2 {
    match p.xy() {
        Some((x, y)) => G2 {
            x: [big(&x.c0), big(&x.c1)],
            y: [big(&y.c0), big(&y.c1)],
        },
        None => G2::identity(),
    }
}

/// Check one G1 point: canonical coordinates, on curve y^2 = x^3 + 3, in the subgroup
/// (G1 has cofactor 1, the check is kept for uniformity). Identity passes (callers decide).
pub fn check_g1(p: &G1, what: &str) -> Result<(), VkError> {
    let a = g1_to_ark(p, what)?;
    if !a.is_on_curve() {
        return Err(VkError::NotOnCurve(what.to_string()));
    }
    if !a.is_in_correct_subgroup_assuming_on_curve() {
        return Err(VkError::NotInSubgroup(what.to_string()));
    }
    Ok(())
}

/// Check one G2 point: canonical coordinates, on the twist curve, in the order-r subgroup.
pub fn check_g2(p: &G2, what: &str) -> Result<(), VkError> {
    let a = g2_to_ark(p, what)?;
    if !a.is_on_curve() {
        return Err(VkError::NotOnCurve(what.to_string()));
    }
    if !a.is_in_correct_subgroup_assuming_on_curve() {
        return Err(VkError::NotInSubgroup(what.to_string()));
    }
    Ok(())
}

impl Groth16Vk {
    /// Full validation. Every failure is an error; nothing is reduced or repaired.
    ///  * IC count == n_public + 1
    ///  * all coordinates canonical (< q)
    ///  * alpha, beta, gamma, delta are not the identity
    ///  * every point is on its curve and in the prime-order subgroup
    ///
    /// IC points may be the identity (a public input that is unconstrained by the circuit);
    /// they are still checked for curve/subgroup membership.
    pub fn validate(&self) -> Result<(), VkError> {
        if self.ic.len() != self.n_public + 1 {
            return Err(VkError::IcCount {
                expected: self.n_public + 1,
                got: self.ic.len(),
            });
        }
        // Canonical coordinates first, so every later message is about a real field element.
        check_g1_canonical(&self.alpha_g1, "alpha_g1")?;
        check_g2_canonical(&self.beta_g2, "beta_g2")?;
        check_g2_canonical(&self.gamma_g2, "gamma_g2")?;
        check_g2_canonical(&self.delta_g2, "delta_g2")?;
        for (i, p) in self.ic.iter().enumerate() {
            check_g1_canonical(p, &format!("IC[{i}]"))?;
        }
        if self.alpha_g1.is_identity() {
            return Err(VkError::Identity("alpha_g1".into()));
        }
        if self.beta_g2.is_identity() {
            return Err(VkError::Identity("beta_g2".into()));
        }
        if self.gamma_g2.is_identity() {
            return Err(VkError::Identity("gamma_g2".into()));
        }
        if self.delta_g2.is_identity() {
            return Err(VkError::Identity("delta_g2".into()));
        }
        check_g1(&self.alpha_g1, "alpha_g1")?;
        check_g2(&self.beta_g2, "beta_g2")?;
        check_g2(&self.gamma_g2, "gamma_g2")?;
        check_g2(&self.delta_g2, "delta_g2")?;
        for (i, p) in self.ic.iter().enumerate() {
            check_g1(p, &format!("IC[{i}]"))?;
        }
        Ok(())
    }
}

fn check_g1_canonical(p: &G1, what: &str) -> Result<(), VkError> {
    canonical(&p.x, what)?;
    canonical(&p.y, what)?;
    Ok(())
}

fn check_g2_canonical(p: &G2, what: &str) -> Result<(), VkError> {
    for c in p.x.iter().chain(p.y.iter()) {
        canonical(c, what)?;
    }
    Ok(())
}
