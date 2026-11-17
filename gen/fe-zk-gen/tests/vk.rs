use ark_bn254::{Fq, Fq2, G2Affine};
use ark_ec::AffineRepr;
use fe_zk_gen::digest::key_preimage;
use fe_zk_gen::validate::{fq_modulus, g2_from_ark};
use fe_zk_gen::{
    emit_fe, key_digest, parse_arkworks, parse_snarkjs, Groth16Vk, VkError, DIGEST_DOMAIN, G1, G2,
};
use num_bigint::BigUint;
use serde_json::Value;

fn read(rel: &str) -> String {
    let p = format!("{}/../../vectors/groth16/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn snarkjs(name: &str) -> Groth16Vk {
    parse_snarkjs(&read(&format!("{name}/verification_key.json"))).unwrap()
}

fn ark_vk() -> Groth16Vk {
    parse_arkworks(&read("arkworks_multiplier/verification_key.json")).unwrap()
}

fn big(s: &str) -> BigUint {
    s.parse().unwrap()
}

// ------------------------------------------------------------------ accept

#[test]
fn valid_snarkjs_keys_accepted() {
    for (name, n) in [("multiplier", 1), ("multiplier2", 2), ("membership", 3)] {
        let vk = snarkjs(name);
        assert_eq!(vk.n_public, n, "{name}");
        assert_eq!(vk.ic.len(), n + 1, "{name}");
        vk.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn valid_arkworks_key_accepted() {
    let vk = ark_vk();
    assert_eq!(vk.n_public, 2);
    assert_eq!(vk.ic.len(), 3);
    vk.validate().unwrap();
}

// -------------------------------------------------------------- G2 order

#[test]
fn g2_evm_order_is_imaginary_first() {
    // BN254 G2 generator (EIP-197 / go-ethereum / arkworks), snarkjs order [c0, c1].
    let g = G2Affine::generator();
    let g2 = g2_from_ark(&g);
    assert_eq!(
        g2.x[0],
        big("10857046999023057135944570762232829481370756359578518086990519993285655852781")
    );
    assert_eq!(
        g2.x[1],
        big("11559732032986387107991004021392285783925812861821192530917403151452391805634")
    );
    let w = g2.evm_words();
    // EIP-197 test vector: x_imag (c1) first, then x_real (c0), then y_imag, y_real.
    assert_eq!(
        w[0],
        big("11559732032986387107991004021392285783925812861821192530917403151452391805634")
    );
    assert_eq!(
        w[1],
        big("10857046999023057135944570762232829481370756359578518086990519993285655852781")
    );
    assert_eq!(
        w[2],
        big("4082367875863433681332203403145435568316851327593401208105741076214120093531")
    );
    assert_eq!(
        w[3],
        big("8495653923123431417604973247489272438418190587263600148770280649306958101930")
    );
}

#[test]
fn g2_evm_order_matches_snarkjs_solidity_verifier() {
    // The snarkjs-generated Verifier.sol writes (deltax1, deltax2, deltay1, deltay2) to the
    // pairing input in that order (offsets 640..768), so deltax1 is the precompile's FIRST word,
    // i.e. the imaginary part c1 of the JSON pair [c0, c1].
    let sol = read("multiplier/Verifier.sol");
    let konst = |name: &str| -> BigUint {
        let pat = format!("uint256 constant {name} = ");
        let i = sol.find(&pat).unwrap() + pat.len();
        big(sol[i..].split(';').next().unwrap())
    };
    let vk = snarkjs("multiplier");
    let w = vk.delta_g2.evm_words();
    assert_eq!(w[0], konst("deltax1"));
    assert_eq!(w[1], konst("deltax2"));
    assert_eq!(w[2], konst("deltay1"));
    assert_eq!(w[3], konst("deltay2"));
    let at = |n: &str, off: u32| sol.contains(&format!("mstore(add(_pPairing, {off}), {n})"));
    assert!(at("deltax1", 640) && at("deltax2", 672) && at("deltay1", 704) && at("deltay2", 736));
}

// ------------------------------------------------------------------ reject

fn snarkjs_mut(f: impl FnOnce(&mut Value)) -> Result<Groth16Vk, VkError> {
    let mut v: Value = serde_json::from_str(&read("multiplier/verification_key.json")).unwrap();
    f(&mut v);
    let vk = parse_snarkjs(&v.to_string())?;
    vk.validate()?;
    Ok(vk)
}

#[test]
fn rejects_g1_off_curve() {
    let e = snarkjs_mut(|v| {
        let y = big(v["vk_alpha_1"][1].as_str().unwrap()) + 1u8;
        v["vk_alpha_1"][1] = Value::String(y.to_string());
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::NotOnCurve(ref w) if w == "alpha_g1"),
        "{e}"
    );
}

#[test]
fn rejects_ic_off_curve() {
    let e = snarkjs_mut(|v| {
        let x = big(v["IC"][1][0].as_str().unwrap()) + 1u8;
        v["IC"][1][0] = Value::String(x.to_string());
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::NotOnCurve(ref w) if w == "IC[1]"),
        "{e}"
    );
}

#[test]
fn rejects_g2_off_curve() {
    let e = snarkjs_mut(|v| {
        let c = big(v["vk_beta_2"][1][0].as_str().unwrap()) + 1u8;
        v["vk_beta_2"][1][0] = Value::String(c.to_string());
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::NotOnCurve(ref w) if w == "beta_g2"),
        "{e}"
    );
}

#[test]
fn rejects_non_canonical_coordinate() {
    // x + q is the same residue (so the "point" would be on the curve if reduced) but is not
    // a canonical encoding: it must be refused, never silently reduced.
    let e = snarkjs_mut(|v| {
        let x = big(v["vk_alpha_1"][0].as_str().unwrap()) + fq_modulus();
        v["vk_alpha_1"][0] = Value::String(x.to_string());
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::NonCanonical(ref w) if w == "alpha_g1"),
        "{e}"
    );

    let e = snarkjs_mut(|v| {
        let x = big(v["vk_delta_2"][0][1].as_str().unwrap()) + fq_modulus();
        v["vk_delta_2"][0][1] = Value::String(x.to_string());
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::NonCanonical(ref w) if w == "delta_g2"),
        "{e}"
    );
}

#[test]
fn rejects_gamma_identity() {
    let e = snarkjs_mut(|v| {
        v["vk_gamma_2"] = serde_json::json!([["0", "0"], ["1", "0"], ["0", "0"]]);
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::Identity(ref w) if w == "gamma_g2"),
        "{e}"
    );
}

#[test]
fn rejects_delta_identity() {
    let e = snarkjs_mut(|v| {
        v["vk_delta_2"] = serde_json::json!([["0", "0"], ["1", "0"], ["0", "0"]]);
    })
    .unwrap_err();
    assert!(
        matches!(e, VkError::Identity(ref w) if w == "delta_g2"),
        "{e}"
    );
}

#[test]
fn rejects_alpha_and_beta_identity() {
    let e = snarkjs_mut(|v| v["vk_alpha_1"] = serde_json::json!(["0", "1", "0"])).unwrap_err();
    assert!(
        matches!(e, VkError::Identity(ref w) if w == "alpha_g1"),
        "{e}"
    );
    let e =
        snarkjs_mut(|v| v["vk_beta_2"] = serde_json::json!([["0", "0"], ["1", "0"], ["0", "0"]]))
            .unwrap_err();
    assert!(
        matches!(e, VkError::Identity(ref w) if w == "beta_g2"),
        "{e}"
    );
}

#[test]
fn rejects_wrong_ic_count() {
    // drop one IC point
    let e = snarkjs_mut(|v| {
        v["IC"].as_array_mut().unwrap().pop();
    })
    .unwrap_err();
    assert!(
        matches!(
            e,
            VkError::IcCount {
                expected: 2,
                got: 1
            }
        ),
        "{e}"
    );
    // claim an extra public input
    let e = snarkjs_mut(|v| v["nPublic"] = Value::from(2)).unwrap_err();
    assert!(
        matches!(
            e,
            VkError::IcCount {
                expected: 3,
                got: 2
            }
        ),
        "{e}"
    );
    // arkworks key with an appended IC point
    let mut vk = ark_vk();
    vk.ic.push(vk.ic[0].clone());
    assert!(matches!(
        vk.validate(),
        Err(VkError::IcCount {
            expected: 3,
            got: 4
        })
    ));
}

/// Find a point on the twist curve E'(Fq2) that is NOT in the order-r subgroup.
fn non_subgroup_g2() -> G2Affine {
    for i in 1u64..1000 {
        let x = Fq2::new(Fq::from(i), Fq::from(1u64));
        if let Some(p) = G2Affine::get_point_from_x_unchecked(x, true) {
            assert!(p.is_on_curve());
            if !p.is_in_correct_subgroup_assuming_on_curve() {
                return p;
            }
        }
    }
    panic!("no twist point found");
}

#[test]
fn rejects_g2_not_in_subgroup() {
    let bad: G2 = g2_from_ark(&non_subgroup_g2());
    for which in ["beta", "gamma", "delta"] {
        let mut vk = snarkjs("multiplier");
        match which {
            "beta" => vk.beta_g2 = bad.clone(),
            "gamma" => vk.gamma_g2 = bad.clone(),
            _ => vk.delta_g2 = bad.clone(),
        }
        let e = vk.validate().unwrap_err();
        assert!(
            matches!(e, VkError::NotInSubgroup(ref w) if w == &format!("{which}_g2")),
            "{e}"
        );
    }
}

#[test]
fn rejects_malformed_inputs() {
    assert!(parse_snarkjs("{}").is_err());
    assert!(snarkjs_mut(|v| v["curve"] = Value::from("bls12381")).is_err());
    assert!(snarkjs_mut(|v| v["vk_alpha_1"][2] = Value::from("2")).is_err());
    assert!(parse_arkworks("{}").is_err());
    // arkworks: bad length
    let mut v: Value =
        serde_json::from_str(&read("arkworks_multiplier/verification_key.json")).unwrap();
    v["alpha_g1"] = Value::from("00");
    assert!(parse_arkworks(&v.to_string()).is_err());
}

#[test]
fn arkworks_rejects_off_curve_and_non_canonical() {
    let mut vk = ark_vk();
    vk.alpha_g1.y += 1u8;
    assert!(matches!(vk.validate(), Err(VkError::NotOnCurve(_))));
    let mut vk = ark_vk();
    vk.alpha_g1 = G1::new(vk.alpha_g1.x.clone() + fq_modulus(), vk.alpha_g1.y.clone());
    assert!(matches!(vk.validate(), Err(VkError::NonCanonical(_))));
}

// ------------------------------------------------------------------ digest

#[test]
fn digest_layout_and_sensitivity() {
    let vk = snarkjs("multiplier");
    let pre = key_preimage(&vk).unwrap();
    assert!(pre.starts_with(DIGEST_DOMAIN));
    // domain || n_public || alpha(2w) || beta,gamma,delta(4w each) || ic_len || ic(2w each)
    assert_eq!(
        pre.len(),
        DIGEST_DOMAIN.len() + 32 + 64 + 3 * 128 + 32 + 64 * vk.ic.len()
    );
    let d = key_digest(&vk).unwrap();
    assert_eq!(d, key_digest(&vk).unwrap());
    // every component changes the digest
    let mut m = vk.clone();
    m.ic.swap(0, 1);
    assert_ne!(key_digest(&m).unwrap(), d);
    let mut m = vk.clone();
    std::mem::swap(&mut m.gamma_g2, &mut m.delta_g2);
    assert_ne!(key_digest(&m).unwrap(), d);
    assert_ne!(key_digest(&snarkjs("multiplier2")).unwrap(), d);
}

#[test]
fn digest_refuses_invalid_key() {
    let mut vk = snarkjs("multiplier");
    vk.delta_g2 = G2::identity();
    assert!(key_digest(&vk).is_err());
}

#[test]
fn emit_is_not_implemented_yet() {
    assert!(matches!(
        emit_fe(&snarkjs("multiplier")),
        Err(VkError::NotImplemented)
    ));
}
