//! Fixture generator for the fe-zk FRI/STARK verifier.
//!
//! Proves a Fibonacci AIR with Plonky3 `p3-uni-stark` 0.4.2 over KoalaBear, degree-4 binomial
//! extension challenge field, FRI (arity 2), Keccak-256 Merkle commitments over serialized
//! field elements, and `SerializingChallenger32<HashChallenger<u8, Keccak256Hash, 32>>`.
//! Writes `fib/proof.json`, `fib/config.json` and `fib/vectors.json` (extension field and
//! challenger test vectors computed by Plonky3 itself). See `README.md`.

use std::borrow::Borrow;
use std::fs;
use std::path::PathBuf;

use p3_air::{Air, AirBuilder, AirBuilderWithPublicValues, BaseAir};
use p3_challenger::{
    CanObserve, CanSample, CanSampleBits, FieldChallenger, GrindingChallenger, HashChallenger,
    SerializingChallenger32,
};
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::{
    BasedVectorSpace, Field, PrimeCharacteristicRing, PrimeField32, PrimeField64, TwoAdicField,
};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_keccak::Keccak256Hash;
use p3_koala_bear::KoalaBear;
use p3_matrix::dense::RowMajorMatrix;
use p3_matrix::Matrix;
use p3_merkle_tree::MerkleTreeMmcs;
use p3_symmetric::{CompressionFunctionFromHasher, SerializingHasher};
use p3_uni_stark::{prove, verify, StarkConfig};
use serde_json::{json, Value};

const LOG_TRACE_ROWS: usize = 4;
const LOG_BLOWUP: usize = 2;
const LOG_FINAL_POLY_LEN: usize = 1;
const NUM_QUERIES: usize = 4;
const COMMIT_POW_BITS: usize = 0;
const QUERY_POW_BITS: usize = 3;

pub struct FibonacciAir {}

impl<F> BaseAir<F> for FibonacciAir {
    fn width(&self) -> usize {
        2
    }
}

impl<AB: AirBuilderWithPublicValues> Air<AB> for FibonacciAir {
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let pis = builder.public_values();
        let a = pis[0];
        let b = pis[1];
        let x = pis[2];
        let (local, next) = (
            main.row_slice(0).expect("empty"),
            main.row_slice(1).expect("one row"),
        );
        let local: &FibonacciRow<AB::Var> = (*local).borrow();
        let next: &FibonacciRow<AB::Var> = (*next).borrow();

        let mut when_first_row = builder.when_first_row();
        when_first_row.assert_eq(local.left.clone(), a);
        when_first_row.assert_eq(local.right.clone(), b);

        let mut when_transition = builder.when_transition();
        when_transition.assert_eq(local.right.clone(), next.left.clone());
        when_transition.assert_eq(local.left.clone() + local.right.clone(), next.right.clone());

        builder.when_last_row().assert_eq(local.right.clone(), x);
    }
}

pub struct FibonacciRow<F> {
    pub left: F,
    pub right: F,
}

impl<F> Borrow<FibonacciRow<F>> for [F] {
    fn borrow(&self) -> &FibonacciRow<F> {
        let (prefix, shorts, suffix) = unsafe { self.align_to::<FibonacciRow<F>>() };
        assert!(prefix.is_empty() && suffix.is_empty() && shorts.len() == 1);
        &shorts[0]
    }
}

fn generate_trace(a: u64, b: u64, n: usize) -> RowMajorMatrix<Val> {
    let mut values = Vec::with_capacity(2 * n);
    let (mut l, mut r) = (Val::from_u64(a), Val::from_u64(b));
    for _ in 0..n {
        values.push(l);
        values.push(r);
        let t = l + r;
        l = r;
        r = t;
    }
    RowMajorMatrix::new(values, 2)
}

type Val = KoalaBear;
type Challenge = BinomialExtensionField<Val, 4>;
type ByteHash = Keccak256Hash;
type FieldHash = SerializingHasher<ByteHash>;
type MyCompress = CompressionFunctionFromHasher<ByteHash, 2, 32>;
type ValMmcs = MerkleTreeMmcs<Val, u8, FieldHash, MyCompress, 32>;
type ChallengeMmcs = ExtensionMmcs<Val, Challenge, ValMmcs>;
type Challenger = SerializingChallenger32<Val, HashChallenger<u8, ByteHash, 32>>;
type Dft = Radix2DitParallel<Val>;
type Pcs = TwoAdicFriPcs<Val, Dft, ValMmcs, ChallengeMmcs>;
type MyConfig = StarkConfig<Pcs, Challenge, Challenger>;

fn make_config() -> MyConfig {
    let byte_hash = ByteHash {};
    let field_hash = FieldHash::new(byte_hash);
    let compress = MyCompress::new(byte_hash);
    let val_mmcs = ValMmcs::new(field_hash, compress);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let fri_params = FriParameters {
        log_blowup: LOG_BLOWUP,
        log_final_poly_len: LOG_FINAL_POLY_LEN,
        num_queries: NUM_QUERIES,
        commit_proof_of_work_bits: COMMIT_POW_BITS,
        query_proof_of_work_bits: QUERY_POW_BITS,
        mmcs: challenge_mmcs,
    };
    let pcs = Pcs::new(Dft::default(), val_mmcs, fri_params);
    MyConfig::new(pcs, Challenger::from_hasher(vec![], byte_hash))
}

fn u(v: &Value) -> u64 {
    v.as_u64().expect("u64")
}

/// `Challenge` as its four canonical base-field coefficients.
fn ext(e: &Challenge) -> Value {
    let c: &[Val] = e.as_basis_coefficients_slice();
    json!(c.iter().map(|x| x.as_canonical_u64()).collect::<Vec<_>>())
}

fn ext_list(v: &[Challenge]) -> Value {
    Value::Array(v.iter().map(ext).collect())
}

/// Serialize the proof through serde (shape is whatever Plonky3 serializes) and then
/// normalize it: hashes become 0x-hex strings, extension elements become [u64; 4].
fn normalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            // BinomialExtensionField serializes as {"value": [..4..], ..}
            if let Some(Value::Array(a)) = m.get("value") {
                if a.len() == 4 && m.len() <= 2 {
                    return Value::Array(a.clone());
                }
            }
            Value::Object(m.iter().map(|(k, x)| (k.clone(), normalize(x))).collect())
        }
        Value::Array(a) => {
            if a.len() == 32 && a.iter().all(|x| x.as_u64().is_some_and(|n| n < 256)) {
                let hex: String = a.iter().map(|x| format!("{:02x}", u(x))).collect();
                return Value::String(format!("0x{hex}"));
            }
            Value::Array(a.iter().map(normalize).collect())
        }
        _ => v.clone(),
    }
}

fn challenger_vectors() -> Value {
    // Scripted use of the real Plonky3 challenger. The Fe test replays `ops` and compares.
    let mut c = Challenger::from_hasher(vec![], ByteHash {});
    let mut ops = vec![];
    let mut out = vec![];
    let f = |x: u64| Val::from_u64(x);
    // observe base elements, sample ext, observe hash-like bytes, sample bits, grind.
    for x in [1u64, 2, 3, 0x7eff_ffff] {
        c.observe(f(x));
    }
    ops.push(json!({"op": "observe_felts", "vals": [1, 2, 3, 0x7eff_ffffu64]}));
    let e: Challenge = c.sample_algebra_element();
    ops.push(json!({"op": "sample_ext"}));
    out.push(ext(&e));
    let h: [u8; 32] = core::array::from_fn(|i| (i * 7 + 1) as u8);
    c.observe(p3_symmetric::Hash::<Val, u8, 32>::from(h));
    ops.push(json!({"op": "observe_hash", "hash": format!("0x{}", h.iter().map(|b| format!("{b:02x}")).collect::<String>())}));
    let b = c.sample_bits(5);
    ops.push(json!({"op": "sample_bits", "bits": 5}));
    out.push(json!(b));
    // many samples to cross the 32-byte output buffer boundary and force rejection loops
    for _ in 0..6 {
        let e: Challenge = c.sample_algebra_element();
        ops.push(json!({"op": "sample_ext"}));
        out.push(ext(&e));
    }
    let w = c.grind(QUERY_POW_BITS);
    ops.push(json!({"op": "grind_check", "bits": QUERY_POW_BITS, "witness": w.as_canonical_u64()}));
    let e: Challenge = c.sample_algebra_element();
    ops.push(json!({"op": "sample_ext"}));
    out.push(ext(&e));
    json!({"ops": ops, "outputs": out})
}

fn ext_vectors() -> Value {
    let mut xs: Vec<Challenge> = vec![];
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % 0x7f00_0001
    };
    for _ in 0..6 {
        let c: [Val; 4] = core::array::from_fn(|_| Val::from_u64(next()));
        xs.push(Challenge::from_basis_coefficients_fn(|i| c[i]));
    }
    let mut cases = vec![];
    for w in xs.windows(2) {
        let (a, b) = (w[0], w[1]);
        cases.push(json!({
            "a": ext(&a), "b": ext(&b),
            "add": ext(&(a + b)), "sub": ext(&(a - b)), "mul": ext(&(a * b)),
            "sq": ext(&(a * a)), "inv": ext(&a.inverse()),
            "pow_1000003": ext(&a.exp_u64(1000003)),
        }));
    }
    json!({
        "cases": cases,
        "two_adic_generators": (0..=8).map(|i| Val::two_adic_generator(i).as_canonical_u64()).collect::<Vec<_>>(),
        "generator": Val::GENERATOR.as_canonical_u64(),
        "two_adic_generator_24": Val::two_adic_generator(24).as_canonical_u64(),
        "w": 3,
    })
}

/// Replay the verifier's Fiat-Shamir transcript with the real Plonky3 challenger and report
/// every derived challenge (independent expected values for the Fe transcript test).
fn transcript_dump(
    config: &MyConfig,
    proof: &p3_uni_stark::Proof<MyConfig>,
    pis: &[Val],
) -> Value {
    use p3_uni_stark::StarkGenericConfig;
    let mut c = config.initialise_challenger();
    c.observe(Val::from_usize(proof.degree_bits));
    c.observe(Val::from_usize(proof.degree_bits));
    c.observe(Val::from_usize(0));
    c.observe(proof.commitments.trace.clone());
    c.observe_slice(pis);
    let alpha: Challenge = c.sample_algebra_element();
    c.observe(proof.commitments.quotient_chunks.clone());
    let zeta: Challenge = c.sample_algebra_element();
    let ov = &proof.opened_values;
    c.observe_algebra_slice(&ov.trace_local);
    c.observe_algebra_slice(&ov.trace_next);
    c.observe_algebra_slice(&ov.quotient_chunks[0]);
    let fri_alpha: Challenge = c.sample_algebra_element();
    let op = &proof.opening_proof;
    let mut betas = vec![];
    for (comm, w) in op.commit_phase_commits.iter().zip(&op.commit_pow_witnesses) {
        c.observe(comm.clone());
        assert!(c.check_witness(COMMIT_POW_BITS, *w));
        let b: Challenge = c.sample_algebra_element();
        betas.push(ext(&b));
    }
    c.observe_algebra_slice(&op.final_poly);
    let pow_ok = c.check_witness(QUERY_POW_BITS, op.query_pow_witness);
    let log_h = op.commit_phase_commits.len() + LOG_BLOWUP + LOG_FINAL_POLY_LEN;
    let indices: Vec<usize> = (0..NUM_QUERIES).map(|_| c.sample_bits(log_h)).collect();
    json!({"alpha": ext(&alpha), "zeta": ext(&zeta), "fri_alpha": ext(&fri_alpha),
           "betas": betas, "pow_ok": pow_ok, "indices": indices})
}

fn main() {
    let out_dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "fib".into()));
    fs::create_dir_all(&out_dir).unwrap();

    let n = 1usize << LOG_TRACE_ROWS;
    let trace = generate_trace(0, 1, n);
    let x = trace.get(n - 1, 1).unwrap();
    let pis = vec![Val::ZERO, Val::ONE, x];

    let config = make_config();
    let proof = prove(&config, &FibonacciAir {}, trace, &pis);
    verify(&config, &FibonacciAir {}, &proof, &pis).expect("plonky3 rejects its own proof");

    let raw = serde_json::to_value(&proof).unwrap();
    let proof_json = normalize(&raw);

    let config_json = json!({
        "plonky3": "p3-uni-stark / p3-fri / p3-challenger / p3-merkle-tree / p3-keccak / p3-koala-bear =0.4.2 (crates.io)",
        "air": "Fibonacci: columns (left,right); first row left=pv0,right=pv1; transition left'=right, right'=left+right; last row right=pv2",
        "field": "KoalaBear p = 2^31 - 2^24 + 1",
        "challenge_field": "BinomialExtensionField<KoalaBear,4>, x^4 = 3",
        "hash": "Keccak256Hash over SerializingHasher (little-endian u32 bytes); compress = keccak256(left || right)",
        "challenger": "SerializingChallenger32<KoalaBear, HashChallenger<u8, Keccak256Hash, 32>>, initial state empty",
        "log_trace_rows": LOG_TRACE_ROWS,
        "log_blowup": LOG_BLOWUP,
        "log_final_poly_len": LOG_FINAL_POLY_LEN,
        "num_queries": NUM_QUERIES,
        "commit_proof_of_work_bits": COMMIT_POW_BITS,
        "query_proof_of_work_bits": QUERY_POW_BITS,
        "public_values": pis.iter().map(|v| v.as_canonical_u64()).collect::<Vec<_>>(),
        "degree_bits": proof.degree_bits,
        "num_quotient_chunks": proof.opened_values.quotient_chunks.len(),
        "opened_trace_local": ext_list(&proof.opened_values.trace_local),
    });

    fs::write(out_dir.join("proof.json"), serde_json::to_string_pretty(&proof_json).unwrap()).unwrap();
    fs::write(out_dir.join("config.json"), serde_json::to_string_pretty(&config_json).unwrap()).unwrap();
    let vectors = json!({"ext": ext_vectors(), "challenger": challenger_vectors(),
        "transcript": transcript_dump(&config, &proof, &pis)});
    fs::write(out_dir.join("vectors.json"), serde_json::to_string_pretty(&vectors).unwrap()).unwrap();
    eprintln!("wrote {} (proof valid in Plonky3)", out_dir.display());
}
