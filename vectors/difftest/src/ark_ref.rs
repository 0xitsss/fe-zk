//! ark-ref: native reference for the Groth16 differential test (ark-groth16 0.5.0 / ark-bn254 0.5.0).
//!
//!   ark-ref gen <out_dir> <seed> <n>   seeded setup of the circuit `c = a*b, public [c, a]` (same as
//!                                      gen/fe-zk-gen/examples/arkworks_fixture.rs), writes
//!                                      verification_key.json (fe-zk-gen --arkworks format),
//!                                      proof.json + public.json (first proof) and valid.txt:
//!                                      n valid proofs for random (a, b), one line each of hex
//!                                      calldata words (no selector): a.x a.y | b.x1 b.x0 b.y1 b.y0 | c.x c.y | c a
//!   ark-ref verify <seed>              stdin: hex word lines; stdout per line `a` or `r`. The native verdict is
//!                                      STRICT: at least the expected length (trailing bytes ignored, as ABI decoding does), every coordinate < q, every input < r, points on
//!                                      curve, G2 in the subgroup, infinity only as (0,0); then ark-groth16 verify.
//!   ark-ref rerand                     stdin: valid hex word lines; stdout: A' = r*A, B' = r^-1 * B, C, inputs
//!                                      (still valid: e(rA, B/r) = e(A, B)); r from a fixed seed.
use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{BigInt, BigInteger, Field, PrimeField, UniformRand, Zero};
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_relations::lc;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_serialize::CanonicalSerialize;
use ark_snark::SNARK;
use rand::{rngs::StdRng, SeedableRng};
use std::io::BufRead;

#[derive(Clone)]
struct Mul {
    a: Option<Fr>,
    b: Option<Fr>,
}

impl ConstraintSynthesizer<Fr> for Mul {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let (a_val, b_val) = (self.a, self.b);
        let c = cs.new_input_variable(|| {
            Ok(a_val.ok_or(SynthesisError::AssignmentMissing)? * b_val.ok_or(SynthesisError::AssignmentMissing)?)
        })?;
        let a_pub = cs.new_input_variable(|| a_val.ok_or(SynthesisError::AssignmentMissing))?;
        let a = cs.new_witness_variable(|| a_val.ok_or(SynthesisError::AssignmentMissing))?;
        let b = cs.new_witness_variable(|| b_val.ok_or(SynthesisError::AssignmentMissing))?;
        cs.enforce_constraint(lc!() + a, lc!() + b, lc!() + c)?;
        cs.enforce_constraint(lc!() + a, lc!() + ark_relations::r1cs::Variable::One, lc!() + a_pub)?;
        Ok(())
    }
}

fn hx<T: CanonicalSerialize>(t: &T) -> String {
    let mut v = Vec::new();
    t.serialize_uncompressed(&mut v).unwrap();
    v.iter().map(|b| format!("{b:02x}")).collect()
}

fn w(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fq_word(x: &Fq) -> String {
    w(&x.into_bigint().to_bytes_be())
}
fn fr_word(x: &Fr) -> String {
    w(&x.into_bigint().to_bytes_be())
}

fn g1_words(p: &G1Affine) -> Vec<String> {
    match p.xy() {
        Some((x, y)) => vec![fq_word(&x), fq_word(&y)],
        None => vec![fq_word(&Fq::zero()); 2],
    }
}
fn g2_words(p: &G2Affine) -> Vec<String> {
    let (x, y) = p.xy().unwrap();
    // calldata order: imaginary part first
    vec![fq_word(&x.c1), fq_word(&x.c0), fq_word(&y.c1), fq_word(&y.c0)]
}

fn proof_line(p: &Proof<Bn254>, public: &[Fr]) -> String {
    let mut v = g1_words(&p.a);
    v.extend(g2_words(&p.b));
    v.extend(g1_words(&p.c));
    v.extend(public.iter().map(fr_word));
    v.concat()
}

fn setup(seed: u64) -> (ark_groth16::ProvingKey<Bn254>, VerifyingKey<Bn254>, StdRng) {
    let mut rng = StdRng::seed_from_u64(seed);
    let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(Mul { a: None, b: None }, &mut rng).unwrap();
    (pk, vk, rng)
}

fn be_limbs(b: &[u8]) -> BigInt<4> {
    let mut l = [0u64; 4];
    for i in 0..4 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[24 - 8 * i..32 - 8 * i]);
        l[i] = u64::from_be_bytes(a);
    }
    BigInt::new(l)
}

fn parse_fq(b: &[u8]) -> Option<Fq> {
    Fq::from_bigint(be_limbs(b))
}

fn parse_g1(x: &[u8], y: &[u8]) -> Option<G1Affine> {
    let (x, y) = (parse_fq(x)?, parse_fq(y)?);
    if x.is_zero() && y.is_zero() {
        return Some(G1Affine::zero());
    }
    let p = G1Affine::new_unchecked(x, y);
    (p.is_on_curve() && p.is_in_correct_subgroup_assuming_on_curve()).then_some(p)
}

fn parse_g2(x1: &[u8], x0: &[u8], y1: &[u8], y0: &[u8]) -> Option<G2Affine> {
    let (x, y) = (Fq2::new(parse_fq(x0)?, parse_fq(x1)?), Fq2::new(parse_fq(y0)?, parse_fq(y1)?));
    if x.is_zero() && y.is_zero() {
        return Some(G2Affine::zero());
    }
    let p = G2Affine::new_unchecked(x, y);
    (p.is_on_curve() && p.is_in_correct_subgroup_assuming_on_curve()).then_some(p)
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn parse_proof(bytes: &[u8], npub: usize) -> Option<(Proof<Bn254>, Vec<Fr>)> {
    // ABI decoders (Solidity and Fe) ignore trailing calldata: only a shorter input is a reject
    if bytes.len() < 32 * (8 + npub) {
        return None;
    }
    let c = |i: usize| &bytes[32 * i..32 * i + 32];
    let a = parse_g1(c(0), c(1))?;
    let b = parse_g2(c(2), c(3), c(4), c(5))?;
    let cc = parse_g1(c(6), c(7))?;
    let mut pubs = vec![];
    for i in 0..npub {
        pubs.push(Fr::from_bigint(be_limbs(c(8 + i)))?);
    }
    Some((Proof { a, b, c: cc }, pubs))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "gen" => {
            let out = &args[1];
            let seed: u64 = args[2].parse().unwrap();
            let n: usize = args[3].parse().unwrap();
            std::fs::create_dir_all(out).unwrap();
            let (pk, vk, mut rng) = setup(seed);
            let mut lines = vec![];
            for k in 0..n {
                let (a, b) = (Fr::rand(&mut rng), Fr::rand(&mut rng));
                let proof = Groth16::<Bn254>::prove(&pk, Mul { a: Some(a), b: Some(b) }, &mut rng).unwrap();
                let public = vec![a * b, a];
                assert!(Groth16::<Bn254>::verify(&vk, &public, &proof).unwrap());
                lines.push(proof_line(&proof, &public));
                if k == 0 {
                    const FMT: &str = "arkworks-groth16-bn254-uncompressed-v1";
                    let abc: Vec<String> = vk.gamma_abc_g1.iter().map(|p| format!("\"{}\"", hx(p))).collect();
                    std::fs::write(
                        format!("{out}/verification_key.json"),
                        format!(
                            "{{\n  \"format\": \"{FMT}\",\n  \"n_public\": 2,\n  \"alpha_g1\": \"{}\",\n  \"beta_g2\": \"{}\",\n  \"gamma_g2\": \"{}\",\n  \"delta_g2\": \"{}\",\n  \"gamma_abc_g1\": [\n    {}\n  ]\n}}\n",
                            hx(&vk.alpha_g1), hx(&vk.beta_g2), hx(&vk.gamma_g2), hx(&vk.delta_g2), abc.join(",\n    ")
                        ),
                    )
                    .unwrap();
                    std::fs::write(
                        format!("{out}/proof.json"),
                        format!("{{\n  \"format\": \"{FMT}\",\n  \"a\": \"{}\",\n  \"b\": \"{}\",\n  \"c\": \"{}\"\n}}\n", hx(&proof.a), hx(&proof.b), hx(&proof.c)),
                    )
                    .unwrap();
                    std::fs::write(format!("{out}/public.json"), format!("[\"{}\", \"{}\"]\n", a * b, a)).unwrap();
                }
            }
            std::fs::write(format!("{out}/valid.txt"), lines.join("\n") + "\n").unwrap();
        }
        "verify" => {
            let seed: u64 = args[1].parse().unwrap();
            let (_, vk, _) = setup(seed);
            let pvk = Groth16::<Bn254>::process_vk(&vk).unwrap();
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap();
                let ok = match parse_proof(&unhex(line.trim()), 2) {
                    Some((p, pubs)) => Groth16::<Bn254>::verify_with_processed_vk(&pvk, &pubs, &p).unwrap_or(false),
                    None => false,
                };
                println!("{}", if ok { 'a' } else { 'r' });
            }
        }
        "rerand" => {
            let mut rng = StdRng::seed_from_u64(0x7e7a);
            let npub: usize = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(2);
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap();
                let (p, pubs) = parse_proof(&unhex(line.trim()), npub).expect("valid proof line");
                let r = Fr::rand(&mut rng);
                let ri = r.inverse().unwrap();
                let a2 = (p.a.into_group() * r).into_affine();
                let b2 = (p.b.into_group() * ri).into_affine();
                println!("{}", proof_line(&Proof { a: a2, b: b2, c: p.c }, &pubs));
            }
        }
        _ => panic!("usage"),
    }
}
