//! gnark PLONK generator tests: key validation (rule 9), digest, emitter determinism, snapshot, ABI
//! agreement with gnark's Solidity verifier, and (ignored, needs `fe`) emit + `fe test` and
//! corrupted-key-fails-to-compile.

use fe_zk_gen::{
    emit_gnark_fe, emit_gnark_ingot, gnark_key_digest, parse_gnark_proof_words, parse_gnark_public,
    parse_gnark_vk, GnarkEmitOptions, GnarkFixture, GnarkVk, VkError, G1, G2,
};
use sha3::{Digest, Keccak256};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let p = root().join("vectors/gnark").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

const NAMES: [&str; 2] = ["mul", "mulcommit"];

fn load(name: &str) -> (GnarkVk, GnarkFixture) {
    let j = read(&format!("{name}/fixture.json"));
    let vk = parse_gnark_vk(&j).unwrap();
    let fx = GnarkFixture {
        proof_words: parse_gnark_proof_words(&j).unwrap(),
        public: parse_gnark_public(&j).unwrap(),
    };
    (vk, fx)
}

fn opts(name: &str, fx: Option<GnarkFixture>) -> GnarkEmitOptions {
    GnarkEmitOptions {
        name: "Fixture".into(),
        source: format!("{name}/fixture.json"),
        fe_zk_path: "../..".into(),
        fixture: fx,
    }
}

fn big(s: &str) -> num_bigint::BigUint {
    s.parse().unwrap()
}

const Q: &str = "21888242871839275222246405745257275088696311157297823662689037894645226208583";
const R: &str = "21888242871839275222246405745257275088548364400416034343698204186575808495617";

#[test]
fn valid_keys_accepted() {
    for (n, np, nc) in [("mul", 2, 0), ("mulcommit", 2, 1)] {
        let (vk, _) = load(n);
        assert_eq!((vk.n_public, vk.n_commit()), (np, nc));
        vk.validate().unwrap_or_else(|e| panic!("{n}: {e}"));
    }
}

#[test]
fn invalid_keys_rejected() {
    let (good, _) = load("mulcommit");
    let check = |mutate: &dyn Fn(&mut GnarkVk), what: &str| {
        let mut vk = good.clone();
        mutate(&mut vk);
        assert!(vk.validate().is_err(), "{what} accepted");
        assert!(gnark_key_digest(&vk).is_err(), "{what}: digest of invalid key");
        assert!(emit_gnark_fe(&vk).is_err(), "{what}: emitted");
    };
    check(&|v| v.n_public = 0, "nb_public = 0");
    check(&|v| v.n_public = 17, "nb_public > size");
    check(&|v| v.size = 0, "size 0");
    check(&|v| v.size = 1, "size 1");
    check(&|v| v.size = 24, "size not a power of two");
    check(&|v| v.size *= 2, "wrong size (w not primitive)");
    check(&|v| v.generator = big("1"), "w = 1");
    check(&|v| v.generator = &v.generator + big(R), "non-canonical w");
    check(&|v| v.size_inv = big("1"), "wrong size_inv");
    check(&|v| v.coset_shift = big("0"), "k = 0");
    check(&|v| v.coset_shift = big("1"), "k in the domain");
    check(&|v| v.coset_shift = &v.coset_shift + big(R), "non-canonical k");
    check(&|v| v.coset_shift = v.generator.clone(), "k in the domain (k = w)");
    check(&|v| v.cci.clear(), "constraint indices missing (NC mismatch)");
    check(&|v| v.cci[0] = 16, "constraint index = n");
    check(&|v| v.cci[0] = 14, "nb_public + index >= n");
    check(&|v| v.qcp.push(v.qcp[0].clone()), "NC mismatch (extra commitment)");
    check(&|v| v.qcp[0] = G1::identity(), "Qcp at infinity");
    check(&|v| v.commitments[3] = G1::new(big("1"), big("3")), "off-curve commitment");
    check(&|v| v.commitments[4] = G1::identity(), "Qk at infinity");
    check(
        &|v| v.commitments[0] = G1::new(&v.commitments[0].x + big(Q), v.commitments[0].y.clone()),
        "non-canonical commitment",
    );
    check(&|v| v.x2 = G2::identity(), "[x]_2 = identity");
    check(&|v| v.g1 = good.commitments[0].clone(), "g1 not the generator");
    check(&|v| v.g2 = good.x2.clone(), "g2 not the generator");
    // on the twist, outside the order-r subgroup
    check(
        &|v| {
            v.x2 = G2 {
                x: [big("2"), big("1")],
                y: [
                    big("7292567877523311580221095596750716176434782432868683424513645834767876293070"),
                    big("19659275751359636165940301690575149581329631496732780143538578556285923319774"),
                ],
            }
        },
        "[x]_2 outside the subgroup",
    );
    let mut vk = good.clone();
    vk.x2.y[1] += 1u8;
    assert!(matches!(vk.validate(), Err(VkError::NotOnCurve(_))));
}

#[test]
fn digest_is_deterministic_and_binds_the_key() {
    let (vk, _) = load("mulcommit");
    let d = gnark_key_digest(&vk).unwrap();
    assert_eq!(d, gnark_key_digest(&vk).unwrap());
    let (other, _) = load("mul");
    assert_ne!(d, gnark_key_digest(&other).unwrap());
    let mut v2 = vk.clone();
    v2.commitments[7] = vk.commitments[0].clone();
    assert_ne!(d, gnark_key_digest(&v2).unwrap());
    let mut v3 = vk.clone();
    v3.cci[0] = 6;
    assert_ne!(d, gnark_key_digest(&v3).unwrap());
}

#[test]
fn emission_is_deterministic_and_carries_digest() {
    for n in NAMES {
        let (vk, fx) = load(n);
        let a = emit_gnark_ingot(&vk, &opts(n, Some(fx.clone()))).unwrap();
        let b = emit_gnark_ingot(&vk, &opts(n, Some(fx))).unwrap();
        assert_eq!(a.lib_fe, b.lib_fe);
        assert_eq!(a.tests_fe, b.tests_fe);
        let d = hex::encode(gnark_key_digest(&vk).unwrap());
        assert!(a.lib_fe.contains(&format!("pub const VK_DIGEST: u256 = 0x{d}")));
        assert!(a.lib_fe.contains(&format!("Key digest : 0x{d}")));
        assert!(a.lib_fe.contains("GENERATED by fe-zk-gen 0.2.0"));
        assert!(a.tests_fe.unwrap().contains(&d));
        assert!(a.lib_fe.contains("gnark::verify_calldata(vk(), proof, public_inputs)"));
        assert!(a
            .lib_fe
            .contains("static_assert(gnark::domain_is_valid(POWER, OMEGA, COSET_SHIFT))"));
        assert!(a
            .lib_fe
            .contains("static_assert(g2_is_valid(X2_X0, X2_X1, X2_Y0, X2_Y1))"));
        assert!(a.lib_fe.contains("gnark::Vk<2,"));
        for nm in ["QL", "QR", "QM", "QO", "QK", "S1", "S2", "S3"] {
            assert!(a
                .lib_fe
                .contains(&format!("static_assert(g1_is_valid({nm}_X, {nm}_Y))")));
        }
        assert!(a.fe_toml.contains("allow_unsafe = true"));
    }
    let (vk, _) = load("mulcommit");
    let l = emit_gnark_fe(&vk).unwrap();
    assert!(l.contains("static_assert(g1_is_valid(QCP0_X, QCP0_Y))"));
    assert!(l.contains("static_assert(N_PUBLIC + CCI0 < (1 << POWER))"));
}

fn selector(sig: &str) -> String {
    hex::encode(&Keccak256::digest(sig.as_bytes())[..4])
}

#[test]
fn selector_signature_matches_gnark_solidity_verifier() {
    for n in NAMES {
        let (vk, _) = load(n);
        let sol = read(&format!("{n}/Verifier.sol"));
        assert!(sol.contains("function Verify(bytes calldata proof, uint256[] calldata public_inputs)"));
        let l = emit_gnark_fe(&vk).unwrap();
        assert!(l.contains("sol(\"Verify(bytes,uint256[])\")"));
    }
    // the selector solc computes for gnark's contract equals keccak("Verify(bytes,uint256[])")[..4]
    if let Ok(out) = std::process::Command::new("solc")
        .arg("--hashes")
        .arg(root().join("vectors/gnark/mul/Verifier.sol"))
        .output()
    {
        let t = String::from_utf8_lossy(&out.stdout).to_string();
        if t.contains("Verify(bytes,uint256[])") {
            let want = selector("Verify(bytes,uint256[])");
            assert!(
                t.contains(&format!("{want}: Verify(bytes,uint256[])")),
                "solc selector differs from {want}:\n{t}"
            );
        }
    }
}

#[test]
fn fixture_words_follow_the_marshal_solidity_layout() {
    let (_, fx) = load("mul");
    assert_eq!(fx.proof_words.len(), 24);
    let (_, fx) = load("mulcommit");
    assert_eq!(fx.proof_words.len(), 27);
}

#[test]
fn refuses_bad_names_and_mismatched_fixtures() {
    let (vk, fx) = load("mul");
    for bad in ["", "1x", "a-b", "a b"] {
        let mut o = opts("mul", Some(fx.clone()));
        o.name = bad.into();
        assert!(emit_gnark_ingot(&vk, &o).is_err(), "{bad}");
    }
    let mut fx2 = fx.clone();
    fx2.public.push(1u8.into());
    assert!(emit_gnark_ingot(&vk, &opts("mul", Some(fx2))).is_err());
    let mut fx3 = fx.clone();
    fx3.proof_words.pop();
    assert!(emit_gnark_ingot(&vk, &opts("mul", Some(fx3))).is_err());
}

#[test]
fn every_check_has_a_failing_case_in_generated_tests() {
    let (vk, fx) = load("mulcommit");
    let t = emit_gnark_ingot(&vk, &opts("mulcommit", Some(fx)))
        .unwrap()
        .tests_fe
        .unwrap();
    for name in [
        "valid_proof_accepted",
        "vk_digest_exposed",
        "tampered_commitment_rejected",
        "tampered_evaluation_rejected",
        "tampered_public_input_rejected",
        "non_canonical_public_input_rejected",
        "non_canonical_evaluation_rejected",
        "non_canonical_coordinate_rejected",
        "point_not_on_curve_rejected",
        "infinity_commitment_rejected",
        "wrong_proof_length_rejected",
        "wrong_public_input_count_rejected",
        "swapped_public_inputs_rejected",
        "all_zero_public_inputs_rejected",
        "all_zero_proof_rejected",
        "vk_constants_pass_full_checks",
        "invalid_keys_fail_the_compile_time_checks",
        "poisoned_keys_reject_every_proof",
    ] {
        assert!(t.contains(&format!("fn {name}()")), "{name}");
    }
}

#[test]
fn snapshot_mulcommit() {
    let (vk, fx) = load("mulcommit");
    let e = emit_gnark_ingot(&vk, &opts("mulcommit", Some(fx))).unwrap();
    let dir = root().join("gen/fe-zk-gen/tests/snapshots");
    for (file, got) in [
        ("gnark_mulcommit_lib.fe", e.lib_fe.as_str()),
        ("gnark_mulcommit_tests.fe", e.tests_fe.as_deref().unwrap()),
        ("gnark_mulcommit_fe.toml", e.fe_toml.as_str()),
    ] {
        let p = dir.join(file);
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
            std::fs::write(&p, got).unwrap();
        }
        let want = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            want, got,
            "{file} changed; rerun with UPDATE_SNAPSHOTS=1 if intended"
        );
    }
}

fn fe_test(dir: &std::path::Path) -> (bool, String) {
    let fe = std::env::var("FE").unwrap_or_else(|_| "fe".into());
    let out = std::process::Command::new(&fe)
        .arg("test")
        .current_dir(dir)
        .output()
        .expect("run fe");
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), text)
}

fn write_ingot(dir: &std::path::Path, e: &fe_zk_gen::EmittedIngot, lib: &str) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("fe.toml"), &e.fe_toml).unwrap();
    std::fs::write(dir.join("src/lib.fe"), lib).unwrap();
    if let Some(t) = &e.tests_fe {
        std::fs::write(dir.join("src/tests.fe"), t).unwrap();
    }
}

#[test]
#[ignore = "needs the fe binary (set FE=/path/to/fe) and takes a few minutes"]
fn emitted_gnark_ingots_pass_fe_test() {
    // fe.toml rejects absolute dependency paths, so emit inside the repo (removed afterwards)
    let base = root().join(".emit-gnark-test-tmp");
    for n in NAMES {
        let (vk, fx) = load(n);
        let e = emit_gnark_ingot(&vk, &opts(n, Some(fx))).unwrap();
        let dir = base.join(n);
        write_ingot(&dir, &e, &e.lib_fe);
        let (ok, text) = fe_test(&dir);
        assert!(ok, "{n}: fe test failed\n{text}");
        assert!(text.contains("0 failed"), "{n}: {text}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// Corrupts one constant of an emitted verifier by hand: `fe test` must fail at COMPILE time with
/// a static assertion (compile-time key validation, independent of the Rust-side validation).
#[test]
#[ignore = "needs the fe binary (set FE=/path/to/fe)"]
fn corrupted_gnark_key_fails_to_compile() {
    let base = root().join(".emit-gnark-corrupt-tmp");
    let (vk, fx) = load("mulcommit");
    let e = emit_gnark_ingot(&vk, &opts("mulcommit", Some(fx))).unwrap();
    let rep = |lib: &str, from: String, to: String| {
        assert!(lib.contains(&from), "{from}");
        lib.replace(&from, &to)
    };
    let bad_y0 = "7292567877523311580221095596750716176434782432868683424513645834767876293070";
    let bad_y1 = "19659275751359636165940301690575149581329631496732780143538578556285923319774";
    let non_subgroup = {
        let l = rep(
            &e.lib_fe,
            format!("pub const X2_X0: u256 = {}", vk.x2.x[0]),
            "pub const X2_X0: u256 = 2".into(),
        );
        let l = rep(
            &l,
            format!("pub const X2_X1: u256 = {}", vk.x2.x[1]),
            "pub const X2_X1: u256 = 1".into(),
        );
        let l = rep(
            &l,
            format!("pub const X2_Y0: u256 = {}", vk.x2.y[0]),
            format!("pub const X2_Y0: u256 = {bad_y0}"),
        );
        rep(
            &l,
            format!("pub const X2_Y1: u256 = {}", vk.x2.y[1]),
            format!("pub const X2_Y1: u256 = {bad_y1}"),
        )
    };
    let cases: Vec<(&str, String)> = vec![
        (
            "off_curve_commitment",
            rep(
                &e.lib_fe,
                format!("pub const S1_Y: u256 = {}", vk.commitments[5].y),
                format!("pub const S1_Y: u256 = {}", &vk.commitments[5].y + 1u8),
            ),
        ),
        (
            "off_curve_qcp",
            rep(
                &e.lib_fe,
                format!("pub const QCP0_Y: u256 = {}", vk.qcp[0].y),
                format!("pub const QCP0_Y: u256 = {}", &vk.qcp[0].y + 1u8),
            ),
        ),
        ("non_subgroup_x2", non_subgroup),
        (
            "bad_omega",
            rep(
                &e.lib_fe,
                format!("pub const OMEGA: u256 = {}", vk.generator),
                "pub const OMEGA: u256 = 1".into(),
            ),
        ),
        (
            "coset_shift_in_domain",
            rep(
                &e.lib_fe,
                format!("pub const COSET_SHIFT: u256 = {}", vk.coset_shift),
                "pub const COSET_SHIFT: u256 = 1".into(),
            ),
        ),
        (
            "constraint_index_outside_domain",
            rep(
                &e.lib_fe,
                format!("pub const CCI0: u256 = {}", vk.cci[0]),
                "pub const CCI0: u256 = 16".into(),
            ),
        ),
    ];
    for (name, lib) in cases {
        let dir = base.join(name);
        write_ingot(&dir, &e, &lib);
        let (ok, text) = fe_test(&dir);
        assert!(!ok, "{name}: corrupted key compiled\n{text}");
        assert!(text.contains("static assertion failed"), "{name}: {text}");
    }
    let _ = std::fs::remove_dir_all(&base);
}
