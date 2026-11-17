//! Writes an arkworks (ark-groth16 0.5.0, ark-bn254 0.5.0) Groth16 fixture.
//!
//! Circuit: witness a, b; public c = a * b and public a. (2 public inputs.)
//! The setup RNG is seeded with a constant: the keys are INSECURE test fixtures.
//!
//! usage: cargo run --example arkworks_fixture -- <out_dir>
//! Output (all points are ark-serialize `serialize_uncompressed`, lowercase hex, 0x-less):
//!   verification_key.json  {"format","n_public","alpha_g1","beta_g2","gamma_g2","delta_g2","gamma_abc_g1":[..]}
//!   proof.json             {"format","a","b","c"}                (G1, G2, G1)
//!   public.json            ["<decimal c>", "<decimal a>"]        (public inputs, in circuit order)

use ark_bn254::{Bn254, Fr};
use ark_groth16::Groth16;
use ark_relations::lc;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_serialize::CanonicalSerialize;
use ark_snark::SNARK;
use rand::{rngs::StdRng, SeedableRng};
use std::fs;

#[derive(Clone)]
struct Mul {
    a: Option<Fr>,
    b: Option<Fr>,
}

impl ConstraintSynthesizer<Fr> for Mul {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let a_val = self.a;
        let b_val = self.b;
        let c = cs.new_input_variable(|| {
            Ok(a_val.ok_or(SynthesisError::AssignmentMissing)?
                * b_val.ok_or(SynthesisError::AssignmentMissing)?)
        })?;
        let a_pub = cs.new_input_variable(|| a_val.ok_or(SynthesisError::AssignmentMissing))?;
        let a = cs.new_witness_variable(|| a_val.ok_or(SynthesisError::AssignmentMissing))?;
        let b = cs.new_witness_variable(|| b_val.ok_or(SynthesisError::AssignmentMissing))?;
        cs.enforce_constraint(lc!() + a, lc!() + b, lc!() + c)?;
        cs.enforce_constraint(
            lc!() + a,
            lc!() + ark_relations::r1cs::Variable::One,
            lc!() + a_pub,
        )?;
        Ok(())
    }
}

fn hx<T: CanonicalSerialize>(t: &T) -> String {
    let mut v = Vec::new();
    t.serialize_uncompressed(&mut v).unwrap();
    v.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .expect("usage: arkworks_fixture <out_dir>");
    fs::create_dir_all(&out).unwrap();
    let mut rng = StdRng::seed_from_u64(0xfe_2c);
    let (a, b) = (Fr::from(7u64), Fr::from(13u64));
    let (pk, vk) =
        Groth16::<Bn254>::circuit_specific_setup(Mul { a: None, b: None }, &mut rng).unwrap();
    let proof = Groth16::<Bn254>::prove(
        &pk,
        Mul {
            a: Some(a),
            b: Some(b),
        },
        &mut rng,
    )
    .unwrap();
    let public = vec![a * b, a];
    assert!(
        Groth16::<Bn254>::verify(&vk, &public, &proof).unwrap(),
        "ark-groth16 rejects own proof"
    );

    const FMT: &str = "arkworks-groth16-bn254-uncompressed-v1";
    let abc: Vec<String> = vk
        .gamma_abc_g1
        .iter()
        .map(|p| format!("\"{}\"", hx(p)))
        .collect();
    let vk_json = format!(
        "{{\n  \"format\": \"{FMT}\",\n  \"n_public\": {},\n  \"alpha_g1\": \"{}\",\n  \"beta_g2\": \"{}\",\n  \"gamma_g2\": \"{}\",\n  \"delta_g2\": \"{}\",\n  \"gamma_abc_g1\": [\n    {}\n  ]\n}}\n",
        public.len(), hx(&vk.alpha_g1), hx(&vk.beta_g2), hx(&vk.gamma_g2), hx(&vk.delta_g2), abc.join(",\n    ")
    );
    fs::write(format!("{out}/verification_key.json"), vk_json).unwrap();
    fs::write(
        format!("{out}/proof.json"),
        format!("{{\n  \"format\": \"{FMT}\",\n  \"a\": \"{}\",\n  \"b\": \"{}\",\n  \"c\": \"{}\"\n}}\n", hx(&proof.a), hx(&proof.b), hx(&proof.c)),
    )
    .unwrap();
    fs::write(
        format!("{out}/public.json"),
        format!("[\"{}\", \"{}\"]\n", a * b, a),
    )
    .unwrap();
    println!("wrote arkworks fixture to {out}; ark-groth16 verify: OK");
}
