//! Independent minimal halo2-KZG verifier implementing `vectors/halo2/SPEC.md`.
//!
//! Operates only on `vk.json` (serde_json::Value), raw proof bytes and instance values.
//! No halo2 types are used; only halo2curves field/curve/pairing arithmetic and keccak.
//! Written to be a mechanical blueprint for the Fe port: one straight-line function
//! `verify`, everything structural (counts, query lists, point sets) comes from the vk.

use halo2_base::halo2_proofs::halo2curves::{
    bn256::{Bn256, Fq, Fq2, Fr, G1Affine, G2Affine, G2Prepared, G1},
    ff::{Field, FromUniformBytes, PrimeField},
    group::{Curve, Group},
    CurveAffine,
    pairing::{MillerLoopResult, MultiMillerLoop},
};
use serde_json::Value;
use sha3::{Digest, Keccak256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Gwc,
    Shplonk,
}

type R<T> = Result<T, String>;

// ---------------------------------------------------------------- json parsing helpers
fn hex32(s: &str) -> R<[u8; 32]> {
    let s = s.strip_prefix("0x").ok_or("missing 0x")?;
    let v = hex::decode(s).map_err(|e| e.to_string())?;
    v.try_into().map_err(|_| "need 32 bytes".to_string())
}
pub fn fr_from_be(b: &[u8; 32]) -> R<Fr> {
    let mut l = *b;
    l.reverse();
    Option::from(Fr::from_repr(l)).ok_or_else(|| "non-canonical Fr".to_string())
}
fn fq_from_be(b: &[u8; 32]) -> R<Fq> {
    let mut l = *b;
    l.reverse();
    Option::from(Fq::from_repr(l)).ok_or_else(|| "non-canonical Fq".to_string())
}
fn jfr(v: &Value) -> R<Fr> {
    fr_from_be(&hex32(v.as_str().ok_or("not str")?)?)
}
fn jfq(v: &Value) -> R<Fq> {
    fq_from_be(&hex32(v.as_str().ok_or("not str")?)?)
}
fn jg1(v: &Value) -> R<G1Affine> {
    let (x, y) = (jfq(&v["x"])?, jfq(&v["y"])?);
    Option::from(G1Affine::from_xy(x, y)).ok_or_else(|| "G1 not on curve".to_string())
}
fn jfq2(v: &Value) -> R<Fq2> {
    Ok(Fq2 { c0: jfq(&v[0])?, c1: jfq(&v[1])? })
}
fn jg2(v: &Value) -> R<G2Affine> {
    let (x, y) = (jfq2(&v["x"])?, jfq2(&v["y"])?);
    Option::from(G2Affine::from_xy(x, y)).ok_or_else(|| "G2 not on curve".to_string())
}
fn ju(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn ji(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn arr(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap()
}

// ---------------------------------------------------------------- transcript (SPEC section 2)
struct Transcript<'a> {
    buf: Vec<u8>,
    proof: &'a [u8],
    pos: usize,
}
impl<'a> Transcript<'a> {
    fn new(proof: &'a [u8]) -> Self {
        Self { buf: vec![], proof, pos: 0 }
    }
    fn absorb_scalar(&mut self, x: &Fr) {
        let mut b = x.to_repr();
        b.reverse();
        self.buf.extend_from_slice(&b);
    }
    fn absorb_point(&mut self, p: &G1Affine) {
        for c in [p.x, p.y] {
            let mut b = c.to_repr();
            b.reverse();
            self.buf.extend_from_slice(&b);
        }
    }
    fn take32(&mut self) -> R<[u8; 32]> {
        let s = self.proof.get(self.pos..self.pos + 32).ok_or("proof too short")?;
        self.pos += 32;
        Ok(s.try_into().unwrap())
    }
    fn read_scalar(&mut self) -> R<Fr> {
        let x = fr_from_be(&self.take32()?)?;
        self.absorb_scalar(&x);
        Ok(x)
    }
    fn read_point(&mut self) -> R<G1Affine> {
        let x = fq_from_be(&self.take32()?)?;
        let y = fq_from_be(&self.take32()?)?;
        // (0,0) is not on y^2 = x^3 + 3, so the point at infinity is rejected here.
        let p: G1Affine =
            Option::from(G1Affine::from_xy(x, y)).ok_or("point not on curve")?;
        self.absorb_point(&p);
        Ok(p)
    }
    fn squeeze(&mut self) -> Fr {
        let mut data = self.buf.clone();
        if data.len() == 32 {
            data.push(1);
        }
        let h: [u8; 32] = Keccak256::digest(&data).into();
        self.buf = h.to_vec();
        // 256-bit big-endian integer reduced mod r
        let mut wide = [0u8; 64];
        for i in 0..32 {
            wide[i] = h[31 - i];
        }
        Fr::from_uniform_bytes(&wide)
    }
}

// ---------------------------------------------------------------- misc math
fn powers(x: Fr, n: usize) -> Vec<Fr> {
    let mut v = Vec::with_capacity(n);
    let mut c = Fr::ONE;
    for _ in 0..n {
        v.push(c);
        c *= x;
    }
    v
}
fn pow_u(x: Fr, e: u64) -> Fr {
    x.pow([e, 0, 0, 0])
}
/// Lagrange interpolation of (points, evals) evaluated at u.
fn interp_eval(points: &[Fr], evals: &[Fr], u: Fr) -> R<Fr> {
    let mut acc = Fr::ZERO;
    for k in 0..points.len() {
        let mut num = Fr::ONE;
        let mut den = Fr::ONE;
        for m in 0..points.len() {
            if m != k {
                num *= u - points[m];
                den *= points[k] - points[m];
            }
        }
        acc += evals[k] * num * Option::<Fr>::from(den.invert()).ok_or("dup point")?;
    }
    Ok(acc)
}

#[derive(Clone)]
struct Query {
    /// index into the commitments table
    comm: usize,
    point: Fr,
    eval: Fr,
}

fn expr_eval(
    e: &Value,
    fixed: &[Fr],
    advice: &[Fr],
    instance: &[Fr],
) -> Fr {
    let rec = |x: &Value| expr_eval(x, fixed, advice, instance);
    match e["op"].as_str().unwrap() {
        "const" => jfr(&e["value"]).unwrap(),
        "fixed" => fixed[ju(&e["query"])],
        "advice" => advice[ju(&e["query"])],
        "instance" => instance[ju(&e["query"])],
        "neg" => -rec(&e["a"]),
        "add" => rec(&e["a"]) + rec(&e["b"]),
        "mul" => rec(&e["a"]) * rec(&e["b"]),
        "scale" => rec(&e["a"]) * jfr(&e["scalar"]).unwrap(),
        op => panic!("unsupported op {op} (challenges / multi-phase not covered)"),
    }
}

fn find_query(qs: &[Value], col: usize, rot: i32) -> usize {
    qs.iter()
        .position(|q| ju(&q["column"]) == col && ji(&q["rotation"]) == rot)
        .expect("query missing")
}

/// Verify a halo2-KZG proof. `instances[col][row]`. Returns Ok(()) iff accepted.
pub fn verify(vk: &Value, scheme: Scheme, proof: &[u8], instances: &[Vec<Fr>]) -> R<()> {
    // ---- vk constants
    let k = ju(&vk["k"]);
    let n = 1u64 << k;
    let omega = jfr(&vk["omega"])?;
    let omega_inv = Option::<Fr>::from(omega.invert()).unwrap();
    let n_inv = jfr(&vk["n_inv"])?;
    let delta = jfr(&vk["delta"])?;
    let bf = ju(&vk["blinding_factors"]) as i32;
    let n_adv = ju(&vk["num_advice_columns"]);
    let quotient_chunks = ju(&vk["quotient_chunks"]);
    if ju(&vk["num_challenges"]) != 0 {
        return Err("multi-phase / challenges not supported".into());
    }
    let advq = arr(&vk["advice_queries"]);
    let fixq = arr(&vk["fixed_queries"]);
    let insq = arr(&vk["instance_queries"]);
    let fixed_comms: Vec<G1Affine> = arr(&vk["fixed_commitments"]).iter().map(|v| jg1(v).unwrap()).collect();
    let perm_cols = arr(&vk["permutation"]["columns"]);
    let chunk_len = ju(&vk["permutation"]["chunk_len"]);
    let perm_comms: Vec<G1Affine> = arr(&vk["permutation"]["commitments"]).iter().map(|v| jg1(v).unwrap()).collect();
    let gates = arr(&vk["gates"]);
    let lookups = arr(&vk["lookups"]);
    let g1 = jg1(&vk["kzg"]["g1"])?;
    let g2 = jg2(&vk["kzg"]["g2"])?;
    let s_g2 = jg2(&vk["kzg"]["s_g2"])?;

    // ---- instance shape (rule: fixed count and lengths)
    let lens: Vec<usize> = arr(&vk["instance_lens"]).iter().map(ju).collect();
    if instances.len() != lens.len() || instances.iter().zip(&lens).any(|(i, l)| i.len() != *l) {
        return Err("wrong instance shape".into());
    }

    // ---- transcript
    let mut t = Transcript::new(proof);
    t.absorb_scalar(&jfr(&vk["transcript_repr"])?);
    for col in instances {
        for v in col {
            t.absorb_scalar(v); // canonical by type
        }
    }
    let advice_comms: Vec<G1Affine> = (0..n_adv).map(|_| t.read_point()).collect::<R<_>>()?;
    let theta = t.squeeze();
    let mut lk_perm: Vec<(G1Affine, G1Affine)> = vec![];
    for _ in lookups {
        let a = t.read_point()?;
        let b = t.read_point()?;
        lk_perm.push((a, b));
    }
    let beta = t.squeeze();
    let gamma = t.squeeze();
    let n_sets = (perm_cols.len() + chunk_len - 1) / chunk_len;
    let perm_z: Vec<G1Affine> = (0..n_sets).map(|_| t.read_point()).collect::<R<_>>()?;
    let lk_z: Vec<G1Affine> = lookups.iter().map(|_| t.read_point()).collect::<R<_>>()?;
    let random_comm = t.read_point()?;
    let y = t.squeeze();
    let h_comms: Vec<G1Affine> = (0..quotient_chunks).map(|_| t.read_point()).collect::<R<_>>()?;
    let x = t.squeeze();
    let trace = std::env::var("H2_TRACE").is_ok();
    let fh = |v: &Fr| { let mut b = v.to_repr(); b.reverse(); hex::encode(b) };
    if trace {
        eprintln!("H2T theta {}\nH2T beta {}\nH2T gamma {}\nH2T y {}\nH2T x {}", fh(&theta), fh(&beta), fh(&gamma), fh(&y), fh(&x));
    }

    // ---- evaluations from the proof
    let advice_evals: Vec<Fr> = (0..advq.len()).map(|_| t.read_scalar()).collect::<R<_>>()?;
    let fixed_evals: Vec<Fr> = (0..fixq.len()).map(|_| t.read_scalar()).collect::<R<_>>()?;
    let random_eval = t.read_scalar()?;
    let sigma_evals: Vec<Fr> = (0..perm_comms.len()).map(|_| t.read_scalar()).collect::<R<_>>()?;
    struct PSet { z: Fr, z_next: Fr, z_last: Option<Fr> }
    let mut psets = vec![];
    for i in 0..n_sets {
        let z = t.read_scalar()?;
        let z_next = t.read_scalar()?;
        let z_last = if i + 1 < n_sets { Some(t.read_scalar()?) } else { None };
        psets.push(PSet { z, z_next, z_last });
    }
    struct LEv { z: Fr, z_next: Fr, a: Fr, a_inv: Fr, s: Fr }
    let mut lev = vec![];
    for _ in lookups {
        lev.push(LEv {
            z: t.read_scalar()?,
            z_next: t.read_scalar()?,
            a: t.read_scalar()?,
            a_inv: t.read_scalar()?,
            s: t.read_scalar()?,
        });
    }

    // ---- Lagrange basis at x:  L_i(x) = w^i (x^n - 1) / (n (x - w^i))
    let xn = pow_u(x, n);
    let rot = |r: i32| -> Fr {
        if r >= 0 { pow_u(omega, r as u64) } else { pow_u(omega_inv, (-r) as u64) }
    };
    let lag = |i: i32| -> R<Fr> {
        let wi = rot(i);
        let d = Option::<Fr>::from((x - wi).invert()).ok_or("x is a domain point")?;
        Ok(wi * (xn - Fr::ONE) * n_inv * d)
    };
    // instance evals: sum_j inst[col][j] * L_{j - rot}(x)
    let mut instance_evals = vec![];
    for q in insq {
        let (c, r) = (ju(&q["column"]), ji(&q["rotation"]));
        let mut acc = Fr::ZERO;
        for (j, v) in instances[c].iter().enumerate() {
            acc += *v * lag(j as i32 - r)?;
        }
        instance_evals.push(acc);
    }
    let l_last = lag(-(bf + 1))?;
    let mut l_blind = Fr::ZERO;
    for i in 1..=bf {
        l_blind += lag(-i)?;
    }
    let l_0 = lag(0)?;
    let active = Fr::ONE - (l_last + l_blind);

    // ---- expected h(x): fold of all constraint values with y (Horner), divided by x^n - 1
    let mut cons: Vec<Fr> = vec![];
    for g in gates {
        for p in arr(&g["polys"]) {
            cons.push(expr_eval(p, &fixed_evals, &advice_evals, &instance_evals));
        }
    }
    // permutation
    let col_eval = |c: &Value| -> Fr {
        let idx = ju(&c["index"]);
        match c["type"].as_str().unwrap() {
            "advice" => advice_evals[find_query(advq, idx, 0)],
            "fixed" => fixed_evals[find_query(fixq, idx, 0)],
            _ => instance_evals[find_query(insq, idx, 0)],
        }
    };
    cons.push(l_0 * (Fr::ONE - psets[0].z));
    let last = psets.last().unwrap();
    cons.push((last.z.square() - last.z) * l_last);
    for i in 1..n_sets {
        cons.push((psets[i].z - psets[i - 1].z_last.unwrap()) * l_0);
    }
    for (ci, set) in psets.iter().enumerate() {
        let cols = &perm_cols[ci * chunk_len..((ci + 1) * chunk_len).min(perm_cols.len())];
        let sig = &sigma_evals[ci * chunk_len..ci * chunk_len + cols.len()];
        let mut left = set.z_next;
        for (c, s) in cols.iter().zip(sig) {
            left *= col_eval(c) + beta * s + gamma;
        }
        let mut right = set.z;
        let mut cur = beta * x * pow_u(delta, (ci * chunk_len) as u64);
        for c in cols {
            right *= col_eval(c) + cur + gamma;
            cur *= delta;
        }
        cons.push((left - right) * active);
    }
    // lookups
    for (lk, ev) in lookups.iter().zip(&lev) {
        let compress = |exprs: &Value| -> Fr {
            arr(exprs)
                .iter()
                .map(|e| expr_eval(e, &fixed_evals, &advice_evals, &instance_evals))
                .fold(Fr::ZERO, |acc, v| acc * theta + v)
        };
        let left = ev.z_next * (ev.a + beta) * (ev.s + gamma);
        let right = ev.z
            * (compress(&lk["input_expressions"]) + beta)
            * (compress(&lk["table_expressions"]) + gamma);
        cons.push(l_0 * (Fr::ONE - ev.z));
        cons.push(l_last * (ev.z.square() - ev.z));
        cons.push((left - right) * active);
        cons.push(l_0 * (ev.a - ev.s));
        cons.push((ev.a - ev.s) * (ev.a - ev.a_inv) * active);
    }
    let h_acc = cons.iter().fold(Fr::ZERO, |h, v| h * y + v);
    let expected_h = h_acc * Option::<Fr>::from((xn - Fr::ONE).invert()).ok_or("x^n = 1")?;
    if trace {
        eprintln!("H2T l_0 {}\nH2T l_last {}\nH2T l_blind {}\nH2T h_acc {}\nH2T expected_h {}", fh(&l_0), fh(&l_last), fh(&l_blind), fh(&h_acc), fh(&expected_h));
    }

    // ---- commitment table + query list (order = SPEC section 7)
    // table: advice | perm_z | lk_perm_in, lk_perm_tab, lk_z | fixed | sigma | H | random
    let mut comms: Vec<G1Affine> = vec![];
    let mut push = |c: G1Affine| {
        comms.push(c);
        comms.len() - 1
    };
    let advice_ids: Vec<usize> = advice_comms.iter().map(|c| push(*c)).collect();
    let pz_ids: Vec<usize> = perm_z.iter().map(|c| push(*c)).collect();
    let lk_ids: Vec<(usize, usize, usize)> = lk_perm
        .iter()
        .zip(&lk_z)
        .map(|((a, s), z)| (push(*a), push(*s), push(*z)))
        .collect();
    let fixed_ids: Vec<usize> = fixed_comms.iter().map(|c| push(*c)).collect();
    let sigma_ids: Vec<usize> = perm_comms.iter().map(|c| push(*c)).collect();
    // H(X) commitment: sum_i xn^i H_i, a derived point (fold from the last piece)
    let h_point: G1Affine = {
        let mut acc = G1::identity();
        for hc in h_comms.iter().rev() {
            acc = acc * xn + hc;
        }
        acc.to_affine()
    };
    let h_id = push(h_point);
    let rand_id = push(random_comm);

    let mut qs: Vec<Query> = vec![];
    for (i, q) in advq.iter().enumerate() {
        qs.push(Query {
            comm: advice_ids[ju(&q["column"])],
            point: x * rot(ji(&q["rotation"])),
            eval: advice_evals[i],
        });
    }
    let x_next = x * omega;
    let x_inv = x * omega_inv;
    let x_last = x * rot(-(bf + 1));
    for (i, s) in psets.iter().enumerate() {
        qs.push(Query { comm: pz_ids[i], point: x, eval: s.z });
        qs.push(Query { comm: pz_ids[i], point: x_next, eval: s.z_next });
    }
    for i in (0..n_sets - 1).rev() {
        qs.push(Query { comm: pz_ids[i], point: x_last, eval: psets[i].z_last.unwrap() });
    }
    for (ids, ev) in lk_ids.iter().zip(&lev) {
        qs.push(Query { comm: ids.2, point: x, eval: ev.z });
        qs.push(Query { comm: ids.0, point: x, eval: ev.a });
        qs.push(Query { comm: ids.1, point: x, eval: ev.s });
        qs.push(Query { comm: ids.0, point: x_inv, eval: ev.a_inv });
        qs.push(Query { comm: ids.2, point: x_next, eval: ev.z_next });
    }
    for (i, q) in fixq.iter().enumerate() {
        qs.push(Query {
            comm: fixed_ids[ju(&q["column"])],
            point: x * rot(ji(&q["rotation"])),
            eval: fixed_evals[i],
        });
    }
    for (i, e) in sigma_evals.iter().enumerate() {
        qs.push(Query { comm: sigma_ids[i], point: x, eval: *e });
    }
    qs.push(Query { comm: h_id, point: x, eval: expected_h });
    qs.push(Query { comm: rand_id, point: x, eval: random_eval });

    // ---- multi-open
    let (left, right): (G1, G1) = match scheme {
        Scheme::Gwc => {
            let v = t.squeeze();
            // group by point, order of first appearance
            let mut groups: Vec<(Fr, Vec<&Query>)> = vec![];
            for q in &qs {
                match groups.iter_mut().find(|(p, _)| *p == q.point) {
                    Some((_, g)) => g.push(q),
                    None => groups.push((q.point, vec![q])),
                }
            }
            let w: Vec<G1Affine> = (0..groups.len()).map(|_| t.read_point()).collect::<R<_>>()?;
            let u = t.squeeze();
            if trace { eprintln!("H2T v {}\nH2T u {}", fh(&v), fh(&u)); }
            let mut left = G1::identity();
            let mut right = G1::identity();
            let mut eval_multi = Fr::ZERO;
            let mut pu = Fr::ONE;
            for ((z, g), wi) in groups.iter().zip(&w) {
                let mut cb = G1::identity();
                let mut eb = Fr::ZERO;
                let mut pv = Fr::ONE;
                for q in g {
                    cb += comms[q.comm] * pv;
                    eb += pv * q.eval;
                    pv *= v;
                }
                right += cb * pu;
                eval_multi += pu * eb;
                right += *wi * (pu * *z);
                left += *wi * pu;
                pu *= u;
            }
            right -= g1 * eval_multi;
            (left, right)
        }
        Scheme::Shplonk => {
            let y2 = t.squeeze();
            let v = t.squeeze();
            let h1 = t.read_point()?;
            let u = t.squeeze();
            let h2 = t.read_point()?;
            if trace { eprintln!("H2T sh_y {}\nH2T sh_v {}\nH2T sh_u {}", fh(&y2), fh(&v), fh(&u)); }
            // super point set (distinct points, order irrelevant) and per-commitment point sets
            let mut super_points: Vec<Fr> = vec![];
            for q in &qs {
                if !super_points.contains(&q.point) {
                    super_points.push(q.point);
                }
            }
            // commitments in first-appearance order with their point lists
            let mut cps: Vec<(usize, Vec<Fr>)> = vec![];
            for q in &qs {
                match cps.iter_mut().find(|(c, _)| *c == q.comm) {
                    Some((_, ps)) => {
                        if !ps.contains(&q.point) {
                            ps.push(q.point)
                        }
                    }
                    None => cps.push((q.comm, vec![q.point])),
                }
            }
            // rotation sets: group commitments with equal point *sets*, first appearance order
            let same = |a: &[Fr], b: &[Fr]| a.len() == b.len() && a.iter().all(|p| b.contains(p));
            let mut rsets: Vec<(Vec<Fr>, Vec<usize>)> = vec![];
            for (c, ps) in &cps {
                match rsets.iter_mut().find(|(sp, _)| same(sp, ps)) {
                    Some((_, cs)) => cs.push(*c),
                    None => rsets.push((ps.clone(), vec![*c])),
                }
            }
            let eval_of = |c: usize, p: Fr| -> Fr {
                qs.iter().find(|q| q.comm == c && q.point == p).unwrap().eval
            };
            let zpoly = |pts: &[Fr]| pts.iter().fold(Fr::ONE, |a, p| a * (u - p));
            let pv = powers(v, rsets.len());
            let mut outer = G1::identity();
            let mut r_outer = Fr::ZERO;
            let (mut z0, mut z0_diff_inv) = (Fr::ZERO, Fr::ZERO);
            for (i, (pts, cs)) in rsets.iter().enumerate() {
                let diffs: Vec<Fr> =
                    super_points.iter().filter(|p| !pts.contains(p)).copied().collect();
                let mut zd = zpoly(&diffs);
                if i == 0 {
                    z0 = zpoly(pts);
                    z0_diff_inv = Option::<Fr>::from(zd.invert()).ok_or("zd=0")?;
                    zd = Fr::ONE;
                } else {
                    zd *= z0_diff_inv;
                }
                let py = powers(y2, cs.len());
                let mut inner = G1::identity();
                let mut r_inner = Fr::ZERO;
                for (c, pyj) in cs.iter().zip(&py) {
                    let evals: Vec<Fr> = pts.iter().map(|p| eval_of(*c, *p)).collect();
                    r_inner += *pyj * interp_eval(pts, &evals, u)?;
                    inner += comms[*c] * pyj;
                }
                outer += inner * (pv[i] * zd);
                r_outer += pv[i] * r_inner * zd;
            }
            outer -= g1 * r_outer;
            outer -= h1 * z0;
            outer += h2 * u;
            (G1::from(h2), outer)
        }
    };
    if t.pos != proof.len() {
        return Err("trailing bytes in proof".into());
    }

    // ---- final pairing: e(left, [s]G2) * e(right, -G2) == 1
    let (l, r) = (left.to_affine(), right.to_affine());
    let s_prep = G2Prepared::from(s_g2);
    let n_prep = G2Prepared::from(-g2);
    let ok = bool::from(
        Bn256::multi_miller_loop(&[(&l, &s_prep), (&r, &n_prep)])
            .final_exponentiation()
            .is_identity(),
    );
    if ok { Ok(()) } else { Err("pairing check failed".into()) }
}
