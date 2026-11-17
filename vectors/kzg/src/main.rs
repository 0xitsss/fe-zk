//! Generates KZG opening fixtures for fe-zk with arkworks (ark-bn254 / ark-ec / ark-poly 0.5.0).
//! TEST-ONLY and INSECURE: the SRS secret tau is a public constant. See ../README.md for the spec.
use ark_bn254::{Bn254, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::{pairing::Pairing, AffineRepr, CurveGroup};
use ark_ff::{BigInteger, One, PrimeField, Zero};
use ark_poly::{
    univariate::{DenseOrSparsePolynomial, DensePolynomial},
    DenseUVPolynomial, Polynomial,
};
use serde_json::{json, Value};
use tiny_keccak::{Hasher, Keccak};

type Poly = DensePolynomial<Fr>;

// Domain tags: ASCII right-aligned in a 32-byte word (same u256 constants as src/verifier/kzg.fe).
const DOM_GWC: &[u8] = b"FEZK-KZG-GWC-V1";
const DOM_SHP_Y: &[u8] = b"FEZK-SHPLONK-Y1";
const DOM_SHP_U: &[u8] = b"FEZK-SHPLONK-U1";

fn word<F: PrimeField>(x: F) -> [u8; 32] {
    let b = x.into_bigint().to_bytes_be();
    let mut o = [0u8; 32];
    o[32 - b.len()..].copy_from_slice(&b);
    o
}
fn dec<F: PrimeField>(x: F) -> String {
    x.into_bigint().to_string()
}
fn dom(d: &[u8]) -> [u8; 32] {
    let mut o = [0u8; 32];
    o[32 - d.len()..].copy_from_slice(d);
    o
}
fn words_hash(words: &[[u8; 32]]) -> Fr {
    let mut k = Keccak::v256();
    for w in words {
        k.update(w);
    }
    let mut out = [0u8; 32];
    k.finalize(&mut out);
    Fr::from_be_bytes_mod_order(&out)
}
fn g1j(p: &G1Affine) -> Value {
    if p.is_zero() {
        return json!(["0", "0"]);
    }
    json!([dec(p.x), dec(p.y)])
}
fn g1w(p: &G1Affine) -> [[u8; 32]; 2] {
    if p.is_zero() {
        return [[0; 32]; 2];
    }
    [word(p.x), word(p.y)]
}
fn g2j(p: &G2Affine) -> Value {
    json!([dec(p.x.c0), dec(p.x.c1), dec(p.y.c0), dec(p.y.c1)])
}
fn g2w(p: &G2Affine) -> [[u8; 32]; 4] {
    [word(p.x.c0), word(p.x.c1), word(p.y.c0), word(p.y.c1)]
}

struct Srs {
    g: G1Affine,
    h: G2Affine,
    tau_h: G2Affine,
    tau: Fr,
}
impl Srs {
    // TEST-ONLY secret. Anyone can forge proofs for this SRS.
    fn new() -> Srs {
        let tau = Fr::from(0x1234_5678_9abc_def0_u64) * Fr::from(1_000_003u64) + Fr::from(7u64);
        let g = G1Affine::generator();
        let h = G2Affine::generator();
        Srs { g, h, tau_h: (G2Projective::from(h) * tau).into_affine(), tau }
    }
    /// Words of the verifying key bound into every transcript: g1 (2), g2 (4), tau_g2 (4).
    fn vk_words(&self) -> Vec<[u8; 32]> {
        let mut v: Vec<[u8; 32]> = g1w(&self.g).to_vec();
        v.extend(g2w(&self.h));
        v.extend(g2w(&self.tau_h));
        v
    }
    /// [p(tau)] G. Possible because the toy SRS has a known tau; a real SRS gives the same
    /// point as the MSM sum_i p_i [tau^i] G.
    fn commit(&self, p: &Poly) -> G1Affine {
        (G1Projective::from(self.g) * p.evaluate(&self.tau)).into_affine()
    }
}
fn rand_poly(seed: u64, deg: usize) -> Poly {
    let mut s = seed;
    let mut cs = vec![];
    for _ in 0..=deg {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        cs.push(Fr::from(s >> 3) * Fr::from(s | 1) + Fr::from(s & 0xff));
    }
    Poly::from_coefficients_vec(cs)
}
fn x_minus(a: Fr) -> Poly {
    Poly::from_coefficients_vec(vec![-a, Fr::one()])
}
fn div_exact(n: &Poly, d: &Poly) -> Poly {
    let (q, r) = DenseOrSparsePolynomial::from(n)
        .divide_with_q_and_r(&DenseOrSparsePolynomial::from(d))
        .unwrap();
    assert!(r.is_zero(), "division not exact");
    q
}
fn open(srs: &Srs, f: &Poly, z: Fr) -> (Fr, G1Affine) {
    let y = f.evaluate(&z);
    let q = div_exact(&(f.clone() - Poly::from_coefficients_vec(vec![y])), &x_minus(z));
    (y, srs.commit(&q))
}
fn cpoly(c: Fr) -> Poly {
    Poly::from_coefficients_vec(vec![c])
}

fn main() {
    let srs = Srs::new();
    let vk = json!({
        "g1": g1j(&srs.g),
        "g2": g2j(&srs.h),
        "tau_g2": g2j(&srs.tau_h),
    });

    // ---- single opening ----
    let f = rand_poly(1, 7);
    let z = Fr::from(123456789u64);
    let c = srs.commit(&f);
    let (y, pi) = open(&srs, &f, z);
    let single = json!({ "c": g1j(&c), "z": dec(z), "y": dec(y), "pi": g1j(&pi) });

    // ---- GWC batch: N = 3 polynomials at one point ----
    let n = 3usize;
    let fs: Vec<Poly> = (0..n).map(|i| rand_poly(10 + i as u64, 5 + i)).collect();
    let zb = Fr::from(987654321u64);
    let cs: Vec<G1Affine> = fs.iter().map(|p| srs.commit(p)).collect();
    let ys: Vec<Fr> = fs.iter().map(|p| p.evaluate(&zb)).collect();
    let mut ws: Vec<[u8; 32]> = vec![dom(DOM_GWC), word(Fr::from(n as u64))];
    ws.extend(srs.vk_words());
    ws.push(word(zb));
    for c in &cs {
        ws.extend(g1w(c));
    }
    for y in &ys {
        ws.push(word(*y));
    }
    let v = words_hash(&ws);
    let mut comb = Poly::zero();
    let mut pw = Fr::one();
    for p in &fs {
        comb = comb + p.clone() * pw;
        pw *= v;
    }
    let (ycomb, pib) = open(&srs, &comb, zb);
    let gwc = json!({
        "z": dec(zb),
        "cs": cs.iter().map(g1j).collect::<Vec<_>>(),
        "ys": ys.iter().map(|y| dec(*y)).collect::<Vec<_>>(),
        "v": dec(v), "y_combined": dec(ycomb), "pi": g1j(&pib),
    });

    // ---- SHPLONK, fixed shape: T = {z1, z2}; f0 @ {z1,z2}, f1 @ {z1}, f2 @ {z2} ----
    let z1 = Fr::from(31415926u64);
    let z2 = Fr::from(27182818u64);
    let sf: Vec<Poly> = vec![rand_poly(20, 6), rand_poly(21, 4), rand_poly(22, 5)];
    let sc: Vec<G1Affine> = sf.iter().map(|p| srs.commit(p)).collect();
    let e00 = sf[0].evaluate(&z1);
    let e01 = sf[0].evaluate(&z2);
    let e1 = sf[1].evaluate(&z1);
    let e2 = sf[2].evaluate(&z2);
    let mut ws: Vec<[u8; 32]> = vec![dom(DOM_SHP_Y)];
    ws.extend(srs.vk_words());
    ws.push(word(z1));
    ws.push(word(z2));
    for c in &sc {
        ws.extend(g1w(c));
    }
    for e in [e00, e01, e1, e2] {
        ws.push(word(e));
    }
    let ych = words_hash(&ws);
    let slope = (e01 - e00) / (z2 - z1);
    let r0 = Poly::from_coefficients_vec(vec![e00 - slope * z1, slope]);
    let rs = [r0, cpoly(e1), cpoly(e2)];
    let zs = [x_minus(z1) * x_minus(z2), x_minus(z1), x_minus(z2)]; // Z_{S_i}
    let mut h = Poly::zero();
    let mut pw = Fr::one();
    for i in 0..3 {
        h = h + div_exact(&(sf[i].clone() - rs[i].clone()), &zs[i]) * pw;
        pw *= ych;
    }
    let hc = srs.commit(&h);
    let mut ws: Vec<[u8; 32]> = vec![dom(DOM_SHP_U), word(ych)];
    ws.extend(g1w(&hc));
    let u = words_hash(&ws);
    let zt_u = (u - z1) * (u - z2);
    let zts = [Fr::one(), u - z2, u - z1]; // Z_{T\S_i}(u)
    let mut lpoly = h.clone() * (-zt_u);
    let mut pw = Fr::one();
    for i in 0..3 {
        let a = pw * zts[i];
        lpoly = lpoly + (sf[i].clone() - cpoly(rs[i].evaluate(&u))) * a;
        pw *= ych;
    }
    assert!(lpoly.evaluate(&u).is_zero());
    let (_, pis) = open(&srs, &lpoly, u);
    let shp = json!({
        "z1": dec(z1), "z2": dec(z2),
        "cs": sc.iter().map(g1j).collect::<Vec<_>>(),
        "e00": dec(e00), "e01": dec(e01), "e1": dec(e1), "e2": dec(e2),
        "y": dec(ych), "u": dec(u), "h": g1j(&hc), "pi": g1j(&pis),
    });

    // sanity: the single-opening pairing identity holds
    let lhs = Bn254::multi_pairing(
        [
            (G1Projective::from(c) - G1Projective::from(srs.g) * y + G1Projective::from(pi) * z)
                .into_affine(),
            (-G1Projective::from(pi)).into_affine(),
        ],
        [srs.h, srs.tau_h],
    );
    assert!(lhs.0.is_one());

    let out = json!({
        "_comment": "TEST-ONLY insecure SRS (public tau). Generated by vectors/kzg (arkworks 0.5.0). See vectors/kzg/README.md.",
        "vk": vk, "single": single, "gwc": gwc, "shplonk": shp,
    });
    let path = std::env::args().nth(1).expect("usage: kzg-fixtures <out.json>");
    std::fs::write(path, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
}
