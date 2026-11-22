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

/// Proof configuration. Defaults are the small demo fixture (`fib/`); `--log-trace`, `--log-blowup`,
/// `--log-final`, `--queries`, `--commit-pow`, `--query-pow` override them (see `README.md`).
#[derive(Clone, Copy)]
struct Cfg {
    log_trace_rows: usize,
    log_blowup: usize,
    log_final_poly_len: usize,
    num_queries: usize,
    commit_pow_bits: usize,
    query_pow_bits: usize,
    /// `fib` (default) or `mul` (see `MulAir`).
    mul_air: bool,
}

/// Grinding bits of the scripted challenger session (independent of the proof configuration).
const CHALLENGER_SCRIPT_POW_BITS: usize = 3;

impl Cfg {
    fn parse() -> (PathBuf, Cfg) {
        let mut c = Cfg {
            log_trace_rows: 4,
            log_blowup: 2,
            log_final_poly_len: 1,
            num_queries: 4,
            commit_pow_bits: 0,
            query_pow_bits: 3,
            mul_air: false,
        };
        let mut out = PathBuf::from("fib");
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut num = || -> usize { args.next().expect("value").parse().expect("number") };
            match a.as_str() {
                "--log-trace" => c.log_trace_rows = num(),
                "--log-blowup" => c.log_blowup = num(),
                "--log-final" => c.log_final_poly_len = num(),
                "--queries" => c.num_queries = num(),
                "--commit-pow" => c.commit_pow_bits = num(),
                "--query-pow" => c.query_pow_bits = num(),
                "--air" => match args.next().expect("air").as_str() {
                    "fib" => c.mul_air = false,
                    "mul" => c.mul_air = true,
                    other => panic!("unknown air {other}"),
                },
                other => out = PathBuf::from(other),
            }
        }
        (out, c)
    }

    /// Plonky3's `FriParameters::conjectured_soundness_bits` (ethSTARK conjecture).
    fn conjectured_bits(&self) -> usize {
        self.log_blowup * self.num_queries + self.query_pow_bits
    }

    /// Soundness estimates in bits (floating point), see `docs/DESIGN.md`. The `proven_*` figures
    /// take the query-phase error of the decoding regime plus the grinding bits and combine it with
    /// the commit-phase / field error (extension field of size about 2^124).
    fn soundness(&self) -> Value {
        let rho = 0.5f64.powi(self.log_blowup as i32);
        let q = self.num_queries as f64;
        let pow = (self.query_pow_bits + self.commit_pow_bits) as f64;
        // unique decoding: per-query error (1 + rho) / 2
        let uniq_q = -((1.0 + rho) / 2.0).log2() * q + pow;
        // Johnson bound with multiplicity m: per-query error (1 + 1/(2m)) sqrt(rho)
        let m = 10.0f64;
        let john_q = -((1.0 + 1.0 / (2.0 * m)) * rho.sqrt()).log2() * q + pow;
        // BCIKS'20 round-by-round FRI error: (m + 1/2)^7 / (3 rho^1.5) * n^2 / |F|, n = LDE size
        let log_n = (self.log_trace_rows + self.log_blowup) as f64;
        let log_f = 4.0 * 31.0; // |F| ~ (2^31)^4
        let eps_commit =
            (m + 0.5).log2() * 7.0 - (3.0f64).log2() - 1.5 * rho.log2() + 2.0 * log_n - log_f;
        let comb = |bits: f64| -((-bits).exp2() + eps_commit.exp2()).log2();
        json!({
            "conjectured_bits": self.conjectured_bits(),
            "conjectured_formula": "log_blowup * num_queries + query_proof_of_work_bits (p3_fri FriParameters::conjectured_soundness_bits)",
            "proven_unique_decoding_bits": comb(uniq_q),
            "proven_johnson_m10_bits": comb(john_q),
            "log2_eps_commit_phase": eps_commit,
        })
    }
}

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

/// Plonky3's `MulAir` (p3-uni-stark `tests/mul_air.rs`) with `REPS` repetitions of the triple
/// `(a, b, c)`, degree 2 (`a * b = c`) and the transition constraints, plus one public value
/// (`a` of the first repetition in the first row). Width `3 * REPS`. Constraint order, per
/// repetition `i`: `a_i * b_i - c_i = 0` (every row), `a_i + 1 - next a_i = 0` (transition);
/// then first row: `a_0 - pv_0 = 0`.
pub const MUL_REPS: usize = 2;
pub struct MulAir {}

impl<F> BaseAir<F> for MulAir {
    fn width(&self) -> usize {
        3 * MUL_REPS
    }
}

impl<AB: AirBuilderWithPublicValues> Air<AB> for MulAir {
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let pis = builder.public_values();
        let pv0 = pis[0];
        let (local, next) = (
            main.row_slice(0).expect("empty"),
            main.row_slice(1).expect("one row"),
        );
        for i in 0..MUL_REPS {
            let s = 3 * i;
            let a = local[s].clone();
            let b = local[s + 1].clone();
            let c = local[s + 2].clone();
            builder.assert_zero(a.clone() * b - c);
            let next_a = next[s].clone();
            builder.when_transition().assert_eq(a + AB::Expr::ONE, next_a);
        }
        builder.when_first_row().assert_eq(local[0].clone(), pv0);
    }
}

/// Mul trace: repetition `r` has `a = 5 r + row + 1` (so `a + 1 = next a`), `b` a fixed
/// pseudo-random value, `c = a * b`.
fn generate_mul_trace(n: usize) -> RowMajorMatrix<Val> {
    let mut values = Vec::with_capacity(3 * MUL_REPS * n);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for row in 0..n {
        for r in 0..MUL_REPS {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let a = Val::from_u64((5 * r + row + 1) as u64);
            let b = Val::from_u64((seed >> 33) % 0x7f00_0001);
            values.push(a);
            values.push(b);
            values.push(a * b);
        }
    }
    RowMajorMatrix::new(values, 3 * MUL_REPS)
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

fn make_config(c: &Cfg) -> MyConfig {
    let byte_hash = ByteHash {};
    let field_hash = FieldHash::new(byte_hash);
    let compress = MyCompress::new(byte_hash);
    let val_mmcs = ValMmcs::new(field_hash, compress);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let fri_params = FriParameters {
        log_blowup: c.log_blowup,
        log_final_poly_len: c.log_final_poly_len,
        num_queries: c.num_queries,
        commit_proof_of_work_bits: c.commit_pow_bits,
        query_proof_of_work_bits: c.query_pow_bits,
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
    let w = c.grind(CHALLENGER_SCRIPT_POW_BITS);
    ops.push(json!({"op": "grind_check", "bits": CHALLENGER_SCRIPT_POW_BITS, "witness": w.as_canonical_u64()}));
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
    cfg: &Cfg,
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
        assert!(c.check_witness(cfg.commit_pow_bits, *w));
        let b: Challenge = c.sample_algebra_element();
        betas.push(ext(&b));
    }
    c.observe_algebra_slice(&op.final_poly);
    let pow_ok = c.check_witness(cfg.query_pow_bits, op.query_pow_witness);
    let log_h = op.commit_phase_commits.len() + cfg.log_blowup + cfg.log_final_poly_len;
    let indices: Vec<usize> = (0..cfg.num_queries).map(|_| c.sample_bits(log_h)).collect();
    json!({"alpha": ext(&alpha), "zeta": ext(&zeta), "fri_alpha": ext(&fri_alpha),
           "betas": betas, "pow_ok": pow_ok, "indices": indices})
}

fn main() {
    let (out_dir, cfg) = Cfg::parse();
    fs::create_dir_all(&out_dir).unwrap();

    let n = 1usize << cfg.log_trace_rows;
    let config = make_config(&cfg);
    let (proof, pis, air_name, width) = if cfg.mul_air {
        let trace = generate_mul_trace(n);
        let pis = vec![trace.get(0, 0).unwrap()];
        let proof = prove(&config, &MulAir {}, trace, &pis);
        verify(&config, &MulAir {}, &proof, &pis).expect("plonky3 rejects its own proof");
        (
            proof,
            pis,
            "Mul: REPS=2 triples (a,b,c) per row, width 6; every row a*b=c; transition a'=a+1; first row a_0=pv0 (degree 2, 1 public value)",
            3 * MUL_REPS,
        )
    } else {
        let trace = generate_trace(0, 1, n);
        let x = trace.get(n - 1, 1).unwrap();
        let pis = vec![Val::ZERO, Val::ONE, x];
        let proof = prove(&config, &FibonacciAir {}, trace, &pis);
        verify(&config, &FibonacciAir {}, &proof, &pis).expect("plonky3 rejects its own proof");
        (
            proof,
            pis,
            "Fibonacci: columns (left,right); first row left=pv0,right=pv1; transition left'=right, right'=left+right; last row right=pv2",
            2,
        )
    };

    let raw = serde_json::to_value(&proof).unwrap();
    let proof_json = normalize(&raw);

    let config_json = json!({
        "plonky3": "p3-uni-stark / p3-fri / p3-challenger / p3-merkle-tree / p3-keccak / p3-koala-bear =0.4.2 (crates.io)",
        "air": air_name,
        "trace_width": width,
        "field": "KoalaBear p = 2^31 - 2^24 + 1",
        "challenge_field": "BinomialExtensionField<KoalaBear,4>, x^4 = 3",
        "hash": "Keccak256Hash over SerializingHasher (little-endian u32 bytes); compress = keccak256(left || right)",
        "challenger": "SerializingChallenger32<KoalaBear, HashChallenger<u8, Keccak256Hash, 32>>, initial state empty",
        "log_trace_rows": cfg.log_trace_rows,
        "log_blowup": cfg.log_blowup,
        "log_final_poly_len": cfg.log_final_poly_len,
        "num_queries": cfg.num_queries,
        "commit_proof_of_work_bits": cfg.commit_pow_bits,
        "query_proof_of_work_bits": cfg.query_pow_bits,
        "soundness": cfg.soundness(),
        "public_values": pis.iter().map(|v| v.as_canonical_u64()).collect::<Vec<_>>(),
        "degree_bits": proof.degree_bits,
        "num_quotient_chunks": proof.opened_values.quotient_chunks.len(),
        "opened_trace_local": ext_list(&proof.opened_values.trace_local),
    });

    fs::write(out_dir.join("proof.json"), serde_json::to_string_pretty(&proof_json).unwrap()).unwrap();
    fs::write(out_dir.join("config.json"), serde_json::to_string_pretty(&config_json).unwrap()).unwrap();
    let vectors = json!({"ext": ext_vectors(), "challenger": challenger_vectors(),
        "transcript": transcript_dump(&cfg, &config, &proof, &pis)});
    fs::write(out_dir.join("vectors.json"), serde_json::to_string_pretty(&vectors).unwrap()).unwrap();
    eprintln!("wrote {} (proof valid in Plonky3)", out_dir.display());
}
