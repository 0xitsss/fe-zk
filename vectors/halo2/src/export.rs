//! Proving (GWC + SHPLONK, EVM Keccak transcript), halo2-native verification, JSON export.

use crate::circuit::Demo;
use halo2_base::halo2_proofs::{
    dev::MockProver,
    halo2curves::{
        bn256::{Bn256, Fq, Fq2, Fr, G1Affine, G2Affine},
        ff::PrimeField,
        CurveAffine,
    },
    plonk::{
        create_proof, keygen_pk, keygen_vk, verify_proof, Any, Expression, ProvingKey,
        VerifyingKey,
    },
    poly::{
        commitment::ParamsProver,
        kzg::{
            commitment::{KZGCommitmentScheme, ParamsKZG},
            multiopen::{ProverGWC, ProverSHPLONK, VerifierGWC, VerifierSHPLONK},
            strategy::AccumulatorStrategy,
        },
        VerificationStrategy,
    },
    transcript::{TranscriptReadBuffer, TranscriptWriterBuffer},
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use serde_json::{json, Value};
use snark_verifier::system::halo2::transcript::evm::EvmTranscript;

pub const K: u32 = 5;

pub fn hex_fr(x: &Fr) -> String {
    let mut b = x.to_repr();
    b.as_mut().reverse();
    format!("0x{}", hex::encode(b.as_ref()))
}
pub fn hex_fq(x: &Fq) -> String {
    let mut b = x.to_repr();
    b.as_mut().reverse();
    format!("0x{}", hex::encode(b.as_ref()))
}
pub fn g1_json(p: &G1Affine) -> Value {
    let c = p.coordinates().unwrap();
    json!({"x": hex_fq(c.x()), "y": hex_fq(c.y())})
}
fn fq2_json(v: &Fq2) -> Value {
    json!([hex_fq(&v.c0), hex_fq(&v.c1)])
}
pub fn g2_json(p: &G2Affine) -> Value {
    let c = p.coordinates().unwrap();
    json!({"x": fq2_json(c.x()), "y": fq2_json(c.y())})
}

/// REAL SRS: halo2 params of size 2^K converted from the PSE perpetual powers of tau (see `ptau.rs`).
pub fn setup_params() -> ParamsKZG<Bn256> {
    crate::ptau::params_from_ptau(&crate::ptau::ensure_ptau(), K)
}

pub fn keygen(params: &ParamsKZG<Bn256>) -> ProvingKey<G1Affine> {
    let c = Demo::default();
    let vk = keygen_vk(params, &c).unwrap();
    keygen_pk(params, vk, &c).unwrap()
}

macro_rules! prove_with {
    ($prover:ident, $params:expr, $pk:expr, $circuit:expr, $inst:expr, $seed:expr) => {{
        let instances: Vec<&[Fr]> = $inst.iter().map(|v: &Vec<Fr>| v.as_slice()).collect();
        let mut transcript = TranscriptWriterBuffer::<_, G1Affine, _>::init(Vec::new());
        create_proof::<KZGCommitmentScheme<Bn256>, $prover<_>, _, _, EvmTranscript<_, _, _, _>, _>(
            $params,
            $pk,
            &[$circuit],
            &[instances.as_slice()],
            ChaCha20Rng::seed_from_u64($seed),
            &mut transcript,
        )
        .unwrap();
        transcript.finalize()
    }};
}

macro_rules! verify_with {
    ($verifier:ident, $params:expr, $vk:expr, $inst:expr, $proof:expr) => {{
        let instances: Vec<&[Fr]> = $inst.iter().map(|v: &Vec<Fr>| v.as_slice()).collect();
        let mut transcript = TranscriptReadBuffer::<_, G1Affine, _>::init($proof);
        match verify_proof::<_, $verifier<_>, _, EvmTranscript<_, _, _, _>, _>(
            $params.verifier_params(),
            $vk,
            AccumulatorStrategy::new($params.verifier_params()),
            &[instances.as_slice()],
            &mut transcript,
        ) {
            Ok(s) => VerificationStrategy::<_, $verifier<_>>::finalize(s),
            Err(_) => false,
        }
    }};
}

pub fn prove_shplonk(
    params: &ParamsKZG<Bn256>,
    pk: &ProvingKey<G1Affine>,
    c: Demo,
    inst: &[Vec<Fr>],
    seed: u64,
) -> Vec<u8> {
    prove_with!(ProverSHPLONK, params, pk, c, inst, seed)
}
pub fn prove_gwc(
    params: &ParamsKZG<Bn256>,
    pk: &ProvingKey<G1Affine>,
    c: Demo,
    inst: &[Vec<Fr>],
    seed: u64,
) -> Vec<u8> {
    prove_with!(ProverGWC, params, pk, c, inst, seed)
}
pub fn verify_shplonk(
    params: &ParamsKZG<Bn256>,
    vk: &VerifyingKey<G1Affine>,
    inst: &[Vec<Fr>],
    proof: &[u8],
) -> bool {
    verify_with!(VerifierSHPLONK, params, vk, inst, proof)
}
pub fn verify_gwc(
    params: &ParamsKZG<Bn256>,
    vk: &VerifyingKey<G1Affine>,
    inst: &[Vec<Fr>],
    proof: &[u8],
) -> bool {
    verify_with!(VerifierGWC, params, vk, inst, proof)
}

fn expr_json(e: &Expression<Fr>, fixed_q: &[(usize, i32)], adv_q: &[(usize, i32)], inst_q: &[(usize, i32)]) -> Value {
    let rec = |x: &Expression<Fr>| expr_json(x, fixed_q, adv_q, inst_q);
    let find = |qs: &[(usize, i32)], col: usize, rot: i32| {
        qs.iter().position(|&(c, r)| c == col && r == rot).expect("query missing")
    };
    match e {
        Expression::Constant(c) => json!({"op":"const","value":hex_fr(c)}),
        Expression::Selector(_) => panic!("virtual selector must have been compressed away"),
        Expression::Fixed(q) => {
            let (c, r) = (q.column_index(), q.rotation().0);
            json!({"op":"fixed","query":find(fixed_q,c,r),"column":c,"rotation":r})
        }
        Expression::Advice(q) => {
            let (c, r) = (q.column_index(), q.rotation().0);
            json!({"op":"advice","query":find(adv_q,c,r),"column":c,"rotation":r})
        }
        Expression::Instance(q) => {
            let (c, r) = (q.column_index(), q.rotation().0);
            json!({"op":"instance","query":find(inst_q,c,r),"column":c,"rotation":r})
        }
        Expression::Challenge(c) => json!({"op":"challenge","index":c.index()}),
        Expression::Negated(a) => json!({"op":"neg","a":rec(a)}),
        Expression::Sum(a, b) => json!({"op":"add","a":rec(a),"b":rec(b)}),
        Expression::Product(a, b) => json!({"op":"mul","a":rec(a),"b":rec(b)}),
        Expression::Scaled(a, s) => json!({"op":"scale","a":rec(a),"scalar":hex_fr(s)}),
    }
}

/// Everything a verifier needs besides proof + instances.
pub fn vk_json(params: &ParamsKZG<Bn256>, pk: &ProvingKey<G1Affine>) -> Value {
    let vk = pk.get_vk();
    let cs = vk.cs();
    let dom = vk.get_domain();
    let n = 1u64 << K;
    let omega = dom.get_omega();
    let n_inv = Fr::from(n).invert().unwrap();
    let advq: Vec<(usize, i32)> =
        cs.advice_queries().iter().map(|(c, r)| (c.index(), r.0)).collect();
    let fixq: Vec<(usize, i32)> =
        cs.fixed_queries().iter().map(|(c, r)| (c.index(), r.0)).collect();
    let insq: Vec<(usize, i32)> =
        cs.instance_queries().iter().map(|(c, r)| (c.index(), r.0)).collect();
    let q_json = |qs: &[(usize, i32)]| -> Value {
        Value::Array(qs.iter().map(|&(c, r)| json!({"column":c,"rotation":r})).collect())
    };
    let perm_cols: Vec<Value> = cs
        .permutation()
        .get_columns()
        .iter()
        .map(|col| {
            let t = match col.column_type() {
                Any::Advice(_) => "advice",
                Any::Fixed => "fixed",
                Any::Instance => "instance",
            };
            json!({"type":t,"index":col.index()})
        })
        .collect();
    let gates: Vec<Value> = cs
        .gates()
        .iter()
        .map(|g| {
            json!({
                "name": g.name(),
                "polys": g.polynomials().iter().map(|p| expr_json(p,&fixq,&advq,&insq)).collect::<Vec<_>>()
            })
        })
        .collect();
    let lookups: Vec<Value> = cs
        .lookups()
        .iter()
        .map(|l| {
            json!({
                "name": l.name(),
                "input_expressions": l.input_expressions().iter().map(|p| expr_json(p,&fixq,&advq,&insq)).collect::<Vec<_>>(),
                "table_expressions": l.table_expressions().iter().map(|p| expr_json(p,&fixq,&advq,&insq)).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "_doc": "halo2-KZG verifying key + circuit metadata for fe-zk. Field elements: 0x + 64 hex, big-endian. G1: {x,y} in Fq. G2: {x:[c0,c1],y:[c0,c1]} (Fq2 = c0 + c1*u). See vectors/halo2/SPEC.md.",
        "flavor": "halo2-axiom 0.5.3 (halo2-base 0.5.5), snark-verifier 0.2.7, EvmTranscript (keccak256, big-endian uncompressed)",
        "k": K,
        "n": n,
        "omega": hex_fr(&omega),
        "n_inv": hex_fr(&n_inv),
        "delta": hex_fr(&Fr::DELTA),
        "cs_degree": cs.degree(),
        "blinding_factors": cs.blinding_factors(),
        "quotient_chunks": dom.get_quotient_poly_degree(),
        "transcript_repr": hex_fr(&vk.transcript_repr()),
        "num_advice_columns": cs.num_advice_columns(),
        "num_fixed_columns": cs.num_fixed_columns(),
        "num_instance_columns": cs.num_instance_columns(),
        "num_challenges": cs.num_challenges(),
        "instance_lens": Demo::num_instance(),
        "advice_queries": q_json(&advq),
        "fixed_queries": q_json(&fixq),
        "instance_queries": q_json(&insq),
        "fixed_commitments": vk.fixed_commitments().iter().map(g1_json).collect::<Vec<_>>(),
        "permutation": {
            "columns": perm_cols,
            "chunk_len": cs.degree() - 2,
            "commitments": vk.permutation().commitments().iter().map(g1_json).collect::<Vec<_>>(),
        },
        "gates": gates,
        "lookups": lookups,
        "kzg": {
            "_doc": "TEST-ONLY INSECURE SRS: s derived from a fixed ChaCha20 seed, trapdoor is public. Never use outside tests.",
            "g1": g1_json(&params.get_g()[0]),
            "g2": g2_json(&params.g2()),
            "s_g2": g2_json(&params.s_g2()),
        },
    })
}

pub fn mock_check(c: &Demo) {
    MockProver::run(K, c, c.instances()).unwrap().assert_satisfied();
}
