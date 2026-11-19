//! PLONK generator tests: key validation (rule 9), digest, emitter determinism, snapshot, ABI
//! agreement with snarkjs' Verifier.sol, and (ignored, needs `fe`) emit + `fe test`.

use fe_zk_gen::{
    emit_plonk_fe, emit_plonk_ingot, parse_plonk_snarkjs, parse_plonk_snarkjs_proof, parse_public,
    plonk_key_digest, PlonkEmitOptions, PlonkFixture, PlonkVk, VkError, G1, G2,
};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let p = root().join("vectors/plonk").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn load(name: &str) -> (PlonkVk, PlonkFixture) {
    let vk = parse_plonk_snarkjs(&read(&format!("{name}/verification_key.json"))).unwrap();
    let proof = parse_plonk_snarkjs_proof(&read(&format!("{name}/proof.json"))).unwrap();
    let public = parse_public(&read(&format!("{name}/public.json"))).unwrap();
    (vk, PlonkFixture { proof, public })
}

const NAMES: [&str; 2] = ["multiplier2", "membership"];

fn opts(name: &str, fx: Option<PlonkFixture>) -> PlonkEmitOptions {
    PlonkEmitOptions {
        name: "Fixture".into(),
        source: format!("{name}/verification_key.json"),
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
    for (n, np) in [("multiplier2", 2), ("membership", 3)] {
        let (vk, _) = load(n);
        assert_eq!(vk.n_public, np);
        vk.validate().unwrap_or_else(|e| panic!("{n}: {e}"));
    }
}

#[test]
fn invalid_keys_rejected() {
    let (good, _) = load("multiplier2");
    let check = |mutate: &dyn Fn(&mut PlonkVk), what: &str| {
        let mut vk = good.clone();
        mutate(&mut vk);
        assert!(vk.validate().is_err(), "{what} accepted");
        assert!(
            plonk_key_digest(&vk).is_err(),
            "{what}: digest of invalid key"
        );
        assert!(emit_plonk_fe(&vk).is_err(), "{what}: emitted");
    };
    check(&|v| v.n_public = 0, "nPublic = 0");
    check(&|v| v.power = 0, "power 0");
    check(&|v| v.power = 29, "power 29");
    check(&|v| v.power = 4, "wrong power (w not primitive)");
    check(&|v| v.w = big("1"), "w = 1");
    check(&|v| v.w = &v.w + big(R), "non-canonical w");
    check(&|v| v.k1 = big("0"), "k1 = 0");
    check(&|v| v.k1 = big("1"), "k1 in the domain");
    check(&|v| v.k2 = v.k1.clone(), "k1 = k2");
    check(&|v| v.k2 = &v.k2 + big(R), "non-canonical k2");
    // k2 = k1 * w: same coset as k1
    check(&|v| v.k2 = (&v.k1 * &v.w) % big(R), "k2 in the coset of k1");
    check(
        &|v| v.commitments[3] = G1::new(big("1"), big("3")),
        "off-curve commitment",
    );
    check(
        &|v| v.commitments[0] = G1::new(&v.commitments[0].x + big(Q), v.commitments[0].y.clone()),
        "non-canonical commitment",
    );
    check(&|v| v.x2 = G2::identity(), "X_2 = identity");
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
        "X_2 outside the subgroup",
    );
    let mut vk = good.clone();
    vk.x2.y[1] += 1u8;
    assert!(matches!(vk.validate(), Err(VkError::NotOnCurve(_))));
}

#[test]
fn identity_commitments_are_allowed() {
    // multiplier2 has Qr = Qc = infinity (snarkjs encodes ["0","1","0"])
    let (vk, _) = load("multiplier2");
    assert!(vk.commitments[2].is_identity() && vk.commitments[4].is_identity());
    vk.validate().unwrap();
}

#[test]
fn digest_is_deterministic_and_binds_the_key() {
    let (vk, _) = load("membership");
    let d = plonk_key_digest(&vk).unwrap();
    assert_eq!(d, plonk_key_digest(&vk).unwrap());
    let (other, _) = load("multiplier2");
    assert_ne!(d, plonk_key_digest(&other).unwrap());
    let mut v2 = vk.clone();
    v2.commitments[7] = vk.commitments[0].clone();
    assert_ne!(d, plonk_key_digest(&v2).unwrap());
}

#[test]
fn emission_is_deterministic_and_carries_digest() {
    for n in NAMES {
        let (vk, fx) = load(n);
        let a = emit_plonk_ingot(&vk, &opts(n, Some(fx.clone()))).unwrap();
        let b = emit_plonk_ingot(&vk, &opts(n, Some(fx))).unwrap();
        assert_eq!(a.lib_fe, b.lib_fe);
        assert_eq!(a.tests_fe, b.tests_fe);
        let d = hex::encode(plonk_key_digest(&vk).unwrap());
        assert!(a
            .lib_fe
            .contains(&format!("pub const VK_DIGEST: u256 = 0x{d}")));
        assert!(a.lib_fe.contains(&format!("Key digest : 0x{d}")));
        assert!(a.lib_fe.contains("GENERATED by fe-zk-gen"));
        assert!(a.tests_fe.unwrap().contains(&d));
        // all logic is forwarded to the library; the key is validated at compile time
        assert!(a.lib_fe.contains("plonk::verify_calldata(vk(), proof, input)"));
        assert!(a
            .lib_fe
            .contains("static_assert(plonk::domain_is_valid(POWER, OMEGA, K1, K2))"));
        assert!(a
            .lib_fe
            .contains("static_assert(g2_is_valid(X2_X0, X2_X1, X2_Y0, X2_Y1))"));
        for nm in ["QM", "QL", "QR", "QO", "QC", "S1", "S2", "S3"] {
            assert!(a
                .lib_fe
                .contains(&format!("static_assert(g1_is_valid({nm}_X, {nm}_Y))")));
        }
        assert!(a.fe_toml.contains("allow_unsafe = true"));
    }
}

#[test]
fn selector_signature_matches_snarkjs_solidity_verifier() {
    for (n, k) in [("multiplier2", 2), ("membership", 3)] {
        let (vk, _) = load(n);
        let sol = read(&format!("{n}/Verifier.sol"));
        assert!(sol.contains(&format!(
            "verifyProof(uint256[24] calldata _proof, uint256[{k}] calldata _pubSignals)"
        )));
        let l = emit_plonk_fe(&vk).unwrap();
        assert!(l.contains(&format!("sol(\"verifyProof(uint256[24],uint256[{k}])\")")));
    }
}

#[test]
fn fixture_words_follow_the_solidity_calldata_layout() {
    let (_, fx) = load("multiplier2");
    let w = fx.proof.words();
    assert_eq!(w.len(), 24);
    // A.x first; eval_a at 18, eval_zw last (snarkjs `soliditycalldata` order)
    assert_eq!(
        w[0],
        big("20176710108600221042848346415794603723722508574617474600888447681216842016017")
    );
    assert_eq!(
        w[18],
        big("12321076811494032936008298570553759319536267382116972865952619283132540806280")
    );
    assert_eq!(
        w[23],
        big("18621058752167940229920058385358981561345027125537068658239163685310620128554")
    );
}

#[test]
fn refuses_bad_names_and_mismatched_fixtures() {
    let (vk, fx) = load("multiplier2");
    for bad in ["", "1x", "a-b", "a b"] {
        let mut o = opts("multiplier2", Some(fx.clone()));
        o.name = bad.into();
        assert!(emit_plonk_ingot(&vk, &o).is_err(), "{bad}");
    }
    let mut fx2 = fx.clone();
    fx2.public.push(1u8.into());
    assert!(emit_plonk_ingot(&vk, &opts("multiplier2", Some(fx2))).is_err());
}

#[test]
fn every_check_has_a_failing_case_in_generated_tests() {
    let (vk, fx) = load("membership");
    let t = emit_plonk_ingot(&vk, &opts("membership", Some(fx)))
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
        "swapped_public_inputs_rejected",
        "all_zero_public_inputs_rejected",
        "vk_constants_pass_full_checks",
        "invalid_keys_fail_the_compile_time_checks",
    ] {
        assert!(t.contains(&format!("fn {name}()")), "{name}");
    }
}

#[test]
fn snapshot_membership() {
    let (vk, fx) = load("membership");
    let e = emit_plonk_ingot(&vk, &opts("membership", Some(fx))).unwrap();
    let dir = root().join("gen/fe-zk-gen/tests/snapshots");
    for (file, got) in [
        ("plonk_membership_lib.fe", e.lib_fe.as_str()),
        ("plonk_membership_tests.fe", e.tests_fe.as_deref().unwrap()),
        ("plonk_membership_fe.toml", e.fe_toml.as_str()),
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

fn write_ingot(dir: &std::path::Path, e: &fe_zk_gen::plonk::EmittedIngot, lib: &str) {
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
fn emitted_plonk_ingots_pass_fe_test() {
    // fe.toml rejects absolute dependency paths, so emit inside the repo (removed afterwards)
    let base = root().join(".emit-plonk-test-tmp");
    for n in NAMES {
        let (vk, fx) = load(n);
        let e = emit_plonk_ingot(&vk, &opts(n, Some(fx))).unwrap();
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
fn corrupted_plonk_key_fails_to_compile() {
    let base = root().join(".emit-plonk-corrupt-tmp");
    let (vk, fx) = load("multiplier2");
    let e = emit_plonk_ingot(&vk, &opts("multiplier2", Some(fx))).unwrap();
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
        ("non_subgroup_x2", non_subgroup),
        (
            "bad_omega",
            rep(
                &e.lib_fe,
                format!("pub const OMEGA: u256 = {}", vk.w),
                "pub const OMEGA: u256 = 1".into(),
            ),
        ),
        (
            "k1_equals_k2",
            rep(
                &e.lib_fe,
                format!("pub const K2: u256 = {}", vk.k2),
                format!("pub const K2: u256 = {}", vk.k1),
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
