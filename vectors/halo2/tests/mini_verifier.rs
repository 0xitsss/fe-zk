use fe_zk_halo2_vectors::circuit::Demo;
use fe_zk_halo2_vectors::export::*;
use fe_zk_halo2_vectors::mini_verifier::{fr_from_be, verify, Scheme};
use halo2_base::halo2_proofs::halo2curves::bn256::Fr;
use serde_json::Value;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/standard_plonk_lookup");

fn load(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{DIR}/{name}")).unwrap()).unwrap()
}
fn hex32(s: &str) -> [u8; 32] {
    hex::decode(s.trim_start_matches("0x")).unwrap().try_into().unwrap()
}
struct Fixture {
    vk: Value,
    proof: Vec<u8>,
    inst: Vec<Vec<Fr>>,
    scheme: Scheme,
}
fn fixture(name: &str, scheme: Scheme) -> Fixture {
    let p = load(&format!("{name}.json"));
    Fixture {
        vk: load("vk.json"),
        proof: hex::decode(p["proof"].as_str().unwrap().trim_start_matches("0x")).unwrap(),
        inst: p["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_array().unwrap().iter().map(|v| fr_from_be(&hex32(v.as_str().unwrap())).unwrap()).collect())
            .collect(),
        scheme,
    }
}
fn fixtures() -> [Fixture; 2] {
    [fixture("shplonk", Scheme::Shplonk), fixture("gwc", Scheme::Gwc)]
}

#[test]
fn accepts_valid_proofs() {
    for f in fixtures() {
        verify(&f.vk, f.scheme, &f.proof, &f.inst).unwrap_or_else(|e| panic!("{:?}: {e}", f.scheme));
    }
}

#[test]
fn rejects_wrong_scheme() {
    for f in fixtures() {
        let other = if f.scheme == Scheme::Gwc { Scheme::Shplonk } else { Scheme::Gwc };
        assert!(verify(&f.vk, other, &f.proof, &f.inst).is_err());
    }
}

#[test]
fn rejects_every_single_byte_flip() {
    for f in fixtures() {
        for i in 0..f.proof.len() {
            let mut p = f.proof.clone();
            p[i] ^= 0x01;
            assert!(verify(&f.vk, f.scheme, &p, &f.inst).is_err(), "{:?} byte {i} flip accepted", f.scheme);
        }
    }
}

#[test]
fn rejects_truncated_and_extended_proof() {
    for f in fixtures() {
        assert!(verify(&f.vk, f.scheme, &f.proof[..f.proof.len() - 32], &f.inst).is_err());
        let mut p = f.proof.clone();
        p.extend_from_slice(&[0u8; 32]);
        assert!(verify(&f.vk, f.scheme, &p, &f.inst).is_err());
    }
}

#[test]
fn rejects_tampered_and_malformed_instances() {
    for f in fixtures() {
        let mut bad = f.inst.clone();
        bad[0][0] += Fr::one();
        assert!(verify(&f.vk, f.scheme, &f.proof, &bad).is_err());
        // wrong count: extra input, missing input, missing column
        let mut extra = f.inst.clone();
        extra[0].push(Fr::zero());
        assert!(verify(&f.vk, f.scheme, &f.proof, &extra).is_err());
        assert!(verify(&f.vk, f.scheme, &f.proof, &[vec![]]).is_err());
        assert!(verify(&f.vk, f.scheme, &f.proof, &[]).is_err());
    }
}

#[test]
fn rejects_tampered_vk_constants() {
    for f in fixtures() {
        // transcript_repr binds the vk: changing it changes every challenge
        let mut vk = f.vk.clone();
        vk["transcript_repr"] = Value::String(format!("0x{:064x}", 1));
        assert!(verify(&vk, f.scheme, &f.proof, &f.inst).is_err());
        // changing a fixed commitment must fail
        let mut vk = f.vk.clone();
        let c = vk["fixed_commitments"].as_array().unwrap().clone();
        vk["fixed_commitments"][0] = c[1].clone();
        assert!(verify(&vk, f.scheme, &f.proof, &f.inst).is_err());
    }
}

/// Other witnesses (same circuit; a, a*b must be < 16 for the lookup) proven here with halo2 itself must also verify,
/// and the exported vk must equal the committed fixture (determinism).
#[test]
fn fresh_witnesses_and_vk_stability() {
    let params = setup_params();
    let pk = keygen(&params);
    let vk = vk_json(&params, &pk);
    assert_eq!(vk, load("vk.json"), "vk.json is stale: run scripts/regen-halo2.sh");
    for (a, b) in [(2u64, 7u64), (0, 0), (15, 1), (1, 15)] {
        let c = Demo::new(a, b);
        mock_check(&c);
        let inst = c.instances();
        for (scheme, proof) in [
            (Scheme::Shplonk, prove_shplonk(&params, &pk, c.clone(), &inst, 99)),
            (Scheme::Gwc, prove_gwc(&params, &pk, c.clone(), &inst, 98)),
        ] {
            verify(&vk, scheme, &proof, &inst).unwrap_or_else(|e| panic!("{scheme:?} ({a},{b}): {e}"));
            let mut bad = inst.clone();
            bad[0][0] += Fr::one();
            assert!(verify(&vk, scheme, &proof, &bad).is_err());
        }
    }
}

/// Converted ptau params are consistent: g[0] = G, the tauG1 chain has one common tau
/// (e([tau^{i+1}]G, H) = e([tau^i]G, [tau]H)), e([tau]G1, G2) == e(G1, [tau]G2), and g_lagrange is the group iFFT of g
/// (commit_lagrange of the evaluations of X equals g[1]; of the constant 1 equals G).
#[test]
fn ptau_params_consistent() {
    use halo2_base::halo2_proofs::{
        arithmetic::Field,
        halo2curves::{
            bn256::{Bn256, G1Affine, G2Affine},
            group::Curve,
            pairing::Engine,
        },
        poly::{
            commitment::{Blind, Params, ParamsProver},
            EvaluationDomain,
        },
    };
    let params = fe_zk_halo2_vectors::ptau::params_from_ptau(&fe_zk_halo2_vectors::ptau::ensure_ptau(), K);
    let g = params.get_g();
    assert_eq!(g.len(), 1 << K);
    assert_eq!(g[0], G1Affine::generator());
    assert_eq!(params.g2(), G2Affine::generator());
    assert_ne!(params.s_g2(), params.g2());
    assert_eq!(
        Bn256::pairing(&g[1], &G2Affine::generator()),
        Bn256::pairing(&G1Affine::generator(), &params.s_g2())
    );
    for i in 0..g.len() - 1 {
        assert_eq!(Bn256::pairing(&g[i + 1], &params.g2()), Bn256::pairing(&g[i], &params.s_g2()), "power {i}");
    }
    let domain = EvaluationDomain::<Fr>::new(2, K);
    let mut ones = domain.empty_lagrange();
    for v in ones.iter_mut() {
        *v = Fr::ONE;
    }
    assert_eq!(params.commit_lagrange(&ones, Blind::default()).to_affine(), g[0]);
    let mut x = domain.empty_lagrange();
    let mut w = Fr::ONE;
    for v in x.iter_mut() {
        *v = w;
        w *= domain.get_omega();
    }
    assert_eq!(params.commit_lagrange(&x, Blind::default()).to_affine(), g[1]);
}

/// Provenance cross-check (ignored by default; needs a second, independently downloaded ppot file):
/// `FE_ZK_PTAU_CROSS=/path/ppot_0080_17.ptau cargo test --release -- --ignored ptau_prefix`.
/// The first 2^6 G1 powers and 2^5 G2 powers of ppot_0080_05 must equal the prefix of the larger file.
#[test]
#[ignore]
fn ptau_prefix_matches_larger_file() {
    use fe_zk_halo2_vectors::ptau::{ensure_ptau, read_ptau};
    let other = std::env::var("FE_ZK_PTAU_CROSS").expect("set FE_ZK_PTAU_CROSS");
    let a = read_ptau(&ensure_ptau(), 1 << 6, 1 << 5);
    let b = read_ptau(std::path::Path::new(&other), a.tau_g1.len(), a.tau_g2.len());
    assert!(b.power > a.power);
    assert_eq!(a.tau_g1, b.tau_g1);
    assert_eq!(a.tau_g2, b.tau_g2);
}
