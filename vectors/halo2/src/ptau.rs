//! snarkjs `.ptau` (powers of tau) -> halo2 `ParamsKZG<Bn256>` converter.
//!
//! Real ceremony SRS: the Perpetual Powers of Tau (PSE `ppot_0080_NN.ptau`, challenge 0080). Nobody knows tau.
//! ptau layout (snarkjs `binfileutils`): magic "ptau", version u32, nSections u32, then per section
//! `id u32 | size u64 | data` (little endian). Section 1 = header (n8 u32, q[n8], power u32, ceremonyPower u32),
//! section 2 = tauG1 (2^(power+1)-1 points [tau^i]G1, i = 0..), section 3 = tauG2 (2^power points [tau^i]G2).
//! Points are uncompressed affine, every Fq coordinate is 32 bytes little endian in **Montgomery form**
//! (value * 2^256 mod q); G2 coordinates are (c0, c1) of Fq2.
//!
//! ParamsKZG needs: g = [tau^i]G1 for i < 2^k (taken verbatim), g_lagrange = inverse FFT of g in the group
//! (computed with halo2's own `g_to_lagrange`), g2 = G2 generator (= tauG2[0]) and s_g2 = [tau]G2 (= tauG2[1]).
use halo2_base::halo2_proofs::{
    arithmetic::g_to_lagrange,
    halo2curves::{
        bn256::{Bn256, Fq, Fq2, Fr, G1Affine, G2Affine},
        ff::{Field, PrimeField},
        group::{cofactor::CofactorCurveAffine, Group},
        CurveAffine,
    },
    poly::kzg::commitment::ParamsKZG,
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::path::{Path, PathBuf};

/// PSE perpetual powers of tau, challenge 0080, power 5 (>= 2^5 G1 powers), fetched by `ensure_ptau`.
pub const PPOT_NAME: &str = "ppot_0080_05.ptau";
pub const PPOT_URL: &str =
    "https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_05.ptau";
/// sha256 recorded at first download (trust on first use; prefix cross-checked against ppot_0080_17, see vectors/README.md).
pub const PPOT_SHA256: &str = "9c1bfc4a2895b9beb1df6e95c96785dc9382bee84d84a0d8c56b1061e09d89d1";

pub struct Ptau {
    pub power: u32,
    /// [tau^i]G1, i = 0..min(max_g1, 2^(power+1)-1)
    pub tau_g1: Vec<G1Affine>,
    /// [tau^i]G2, i = 0..min(max_g2, 2^power)
    pub tau_g2: Vec<G2Affine>,
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Montgomery LE 32 bytes -> Fq. Rejects non-canonical limbs.
fn fq_from_mont(b: &[u8], r_inv: &Fq) -> Fq {
    let mut repr = [0u8; 32];
    repr.copy_from_slice(b);
    // `from_repr` takes the canonical LE integer; here that integer is x*R mod q, so multiply by R^-1.
    let xm = Option::<Fq>::from(Fq::from_repr(repr)).expect("non-canonical ptau coordinate");
    xm * r_inv
}

/// Parse a ptau file, decoding the first `max_g1` G1 and `max_g2` G2 powers (all on-curve checked).
pub fn read_ptau(path: &Path, max_g1: usize, max_g2: usize) -> Ptau {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(&b[0..4], b"ptau", "not a ptau file");
    let nsec = rd32(&b, 8) as usize;
    let mut off = 12;
    let mut secs = std::collections::HashMap::new();
    for _ in 0..nsec {
        let id = rd32(&b, off);
        let sz = rd64(&b, off + 4) as usize;
        secs.insert(id, (off + 12, sz));
        off += 12 + sz;
    }
    assert_eq!(off, b.len(), "trailing bytes / truncated ptau");
    let (h, _) = secs[&1];
    let n8 = rd32(&b, h) as usize;
    assert_eq!(n8, 32);
    let power = rd32(&b, h + 4 + n8);
    let (s2, sz2) = secs[&2];
    let (s3, sz3) = secs[&3];
    assert_eq!(sz2, ((1usize << (power + 1)) - 1) * 64);
    assert_eq!(sz3, (1usize << power) * 128);
    // R^-1 = 2^-256 mod q
    let r_inv = Fq::from(2u64).pow([256, 0, 0, 0]).invert().unwrap();
    let n1 = max_g1.min(sz2 / 64);
    let n2 = max_g2.min(sz3 / 128);
    let tau_g1 = (0..n1)
        .map(|i| {
            let o = s2 + 64 * i;
            let (x, y) = (fq_from_mont(&b[o..o + 32], &r_inv), fq_from_mont(&b[o + 32..o + 64], &r_inv));
            if bool::from(x.is_zero()) && bool::from(y.is_zero()) {
                return G1Affine::identity();
            }
            Option::<G1Affine>::from(G1Affine::from_xy(x, y)).expect("tauG1 point not on curve")
        })
        .collect();
    let tau_g2 = (0..n2)
        .map(|i| {
            let o = s3 + 128 * i;
            let f = |j: usize| fq_from_mont(&b[o + 32 * j..o + 32 * j + 32], &r_inv);
            let (x, y) = (Fq2 { c0: f(0), c1: f(1) }, Fq2 { c0: f(2), c1: f(3) });
            let p = Option::<G2Affine>::from(G2Affine::from_xy(x, y)).expect("tauG2 point not on curve");
            // subgroup check: [r-1]P + P == O (G2 cofactor != 1)
            assert!(bool::from((p * (-Fr::ONE) + p).is_identity()), "tauG2 point not in the r-torsion subgroup");
            p
        })
        .collect();
    Ptau { power, tau_g1, tau_g2 }
}

/// Convert to halo2 params of size 2^k.
pub fn params_from_ptau(path: &Path, k: u32) -> ParamsKZG<Bn256> {
    let n = 1usize << k;
    let p = read_ptau(path, n, 2);
    assert!(p.power >= k, "ptau power {} < k {k}", p.power);
    assert_eq!(p.tau_g1.len(), n);
    let g = p.tau_g1;
    assert_eq!(g[0], G1Affine::generator(), "tauG1[0] must be the BN254 G1 generator");
    assert_eq!(p.tau_g2[0], G2Affine::generator(), "tauG2[0] must be the BN254 G2 generator");
    // `from_parts` is an instance method in halo2-axiom 0.5.3; the dummy params are used only as the receiver
    // and every field is overwritten (nothing of the dummy's secret survives).
    let dummy = ParamsKZG::<Bn256>::setup(1, ChaCha20Rng::seed_from_u64(0));
    let g_lagrange = g_to_lagrange::<G1Affine>(g.iter().map(|p| p.to_curve()).collect(), k);
    dummy.from_parts(k, g, Some(g_lagrange), p.tau_g2[0], p.tau_g2[1])
}

/// Path of the pinned ptau in `vectors/cache` (or `$FE_ZK_PTAU`); downloaded with curl and sha256-checked if missing.
pub fn ensure_ptau() -> PathBuf {
    use sha2::{Digest, Sha256};
    let path: PathBuf = std::env::var_os("FE_ZK_PTAU")
        .map(Into::into)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../cache").join(PPOT_NAME));
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let st = std::process::Command::new("curl")
            .args(["-sSfL", "-o"])
            .arg(&path)
            .arg(PPOT_URL)
            .status()
            .expect("curl");
        assert!(st.success(), "download {PPOT_URL} failed");
    }
    let h = hex::encode(Sha256::digest(std::fs::read(&path).unwrap()));
    assert_eq!(h, PPOT_SHA256, "ptau sha256 mismatch for {}", path.display());
    path
}
