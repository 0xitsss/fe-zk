//! halo2 generator tests: Rust port vs the Python emitter (byte-identical), validation,
//! determinism, and (ignored, needs `fe`) emit + `fe test`, corrupted key must not compile.

use fe_zk_gen::{
    emit_halo2_ingot, emit_halo2_module, halo2_key_digest, parse_halo2_proof, parse_halo2_vk,
    Halo2EmitOptions, Halo2Scheme, Halo2Vk,
};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn dir() -> PathBuf {
    root().join("vectors/halo2/standard_plonk_lookup")
}

fn read(p: PathBuf) -> String {
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn vk() -> Halo2Vk {
    parse_halo2_vk(&read(dir().join("vk.json"))).unwrap()
}

fn opts() -> Halo2EmitOptions {
    Halo2EmitOptions {
        name: "Fixture".into(),
        scheme: Halo2Scheme::Both,
        source: "standard_plonk_lookup".into(),
        fe_zk_path: "../..".into(),
        shplonk: Some(parse_halo2_proof(&read(dir().join("shplonk.json"))).unwrap()),
        gwc: Some(parse_halo2_proof(&read(dir().join("gwc.json"))).unwrap()),
    }
}

#[test]
fn rust_port_is_byte_identical_to_python_emitter() {
    let py = std::process::Command::new("python3")
        .arg("-c")
        .arg("import sys,json; sys.path.insert(0, sys.argv[1]); import emit_fe; sys.stdout.write(emit_fe.Gen(json.load(open(sys.argv[2])), 'm').run())")
        .arg(root().join("vectors/halo2"))
        .arg(dir().join("vk.json"))
        .output();
    let Ok(py) = py else {
        eprintln!("python3 not found, skipping cross-check");
        return;
    };
    assert!(
        py.status.success(),
        "{}",
        String::from_utf8_lossy(&py.stderr)
    );
    let rs = emit_halo2_module(&vk()).unwrap();
    assert_eq!(String::from_utf8(py.stdout).unwrap(), rs);
}

#[test]
fn digest_matches_embedded_constant_and_is_stable() {
    let d = hex::encode(halo2_key_digest(&vk()).unwrap());
    let m = emit_halo2_module(&vk()).unwrap();
    assert!(m.contains(&format!("pub const VK_DIGEST: u256 = 0x{d}")));
    assert!(m.contains("GENERATED FILE, do not edit"));
}

#[test]
fn emission_is_deterministic() {
    let a = emit_halo2_ingot(&vk(), &opts()).unwrap();
    let b = emit_halo2_ingot(&vk(), &opts()).unwrap();
    assert_eq!(a.lib_fe, b.lib_fe);
    assert_eq!(a.tests_fe, b.tests_fe);
    assert!(a.lib_fe.contains("use fe_zk::verifier::halo2::common::{"));
    assert!(a
        .lib_fe
        .contains("sol(\"verifyShplonk(uint256[57],uint256[1])\")"));
    assert!(a
        .lib_fe
        .contains("sol(\"verifyGwc(uint256[61],uint256[1])\")"));
    assert!(a.lib_fe.contains("static_assert(g2_is_valid(SRS_S_G2_X0"));
    assert!(a
        .lib_fe
        .contains("static_assert(g1_is_valid(SG0_X, SG0_Y))"));
    assert!(a
        .lib_fe
        .contains("static_assert(g1_is_valid(FX0_X, FX0_Y))"));
}

const ONE: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";

#[test]
fn invalid_keys_rejected() {
    let good = read(dir().join("vk.json"));
    let mutate = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
        f(&mut v);
        let vk = Halo2Vk { v };
        assert!(vk.validate().is_err());
        assert!(emit_halo2_module(&vk).is_err());
    };
    mutate(&|v| v["kzg"]["g1"]["y"] = "0x03".into());
    mutate(&|v| v["kzg"]["s_g2"]["y"][1] = ONE.into());
    mutate(&|v| v["fixed_commitments"][0]["y"] = "0x05".into());
    mutate(&|v| v["permutation"]["commitments"][0] = v["permutation"]["commitments"][1].clone());
    mutate(&|v| v["omega"] = ONE.into());
    mutate(&|v| v["n_inv"] = "0x02".into());
    mutate(&|v| {
        v["transcript_repr"] =
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into()
    });
    mutate(&|v| v["num_challenges"] = 1.into());
    mutate(&|v| v["quotient_chunks"] = 9.into());
    mutate(&|v| v["delta"] = ONE.into());
    // infinity SRS point
    mutate(&|v| {
        v["kzg"]["g1"]["x"] = "0x00".into();
        v["kzg"]["g1"]["y"] = "0x00".into();
    });
}

#[test]
fn fixture_size_mismatch_rejected() {
    let mut o = opts();
    o.shplonk.as_mut().unwrap().proof.pop();
    assert!(emit_halo2_ingot(&vk(), &o).is_err());
    let mut o = opts();
    o.gwc = None;
    // `Both` without the GWC proof: refused, never half-tested
    assert!(emit_halo2_ingot(&vk(), &o).is_err());
}

#[test]
fn every_check_has_a_failing_case_in_generated_tests() {
    let t = emit_halo2_ingot(&vk(), &opts()).unwrap().tests_fe.unwrap();
    for sch in ["shplonk", "gwc"] {
        for n in [
            "valid_proof_accepted",
            "wrong_instance_rejected",
            "non_canonical_instance_rejected",
            "every_word_plus_one_rejected",
            "non_canonical_words_rejected",
            "off_curve_points_rejected",
            "infinity_points_rejected",
            "negated_points_rejected",
            "swapped_commitments_rejected",
            "proof_of_other_scheme_rejected",
            "vk_digest_exposed",
        ] {
            assert!(t.contains(&format!("fn {sch}_{n}()")), "{sch}_{n}");
        }
    }
    for n in [
        "vk_constants_pass_full_checks",
        "invalid_keys_fail_the_compile_time_checks",
    ] {
        assert!(t.contains(&format!("fn {n}()")), "{n}");
    }
}

#[test]
fn one_contract_per_scheme() {
    // both: two contracts in one ingot, each reaching only its own verify function
    let both = emit_halo2_ingot(&vk(), &opts()).unwrap();
    assert!(both
        .lib_fe
        .contains("pub contract FixtureShplonkVerifier {"));
    assert!(both.lib_fe.contains("pub contract FixtureGwcVerifier {"));
    assert!(!both.lib_fe.contains("pub contract FixtureVerifier {"));
    // single scheme: only that verifier and contract, tests need only its fixture
    for (scheme, has, lacks) in [
        (Halo2Scheme::Shplonk, "verify_shplonk", "verify_gwc"),
        (Halo2Scheme::Gwc, "verify_gwc", "verify_shplonk"),
    ] {
        let mut o = opts();
        o.scheme = scheme;
        if scheme == Halo2Scheme::Shplonk {
            o.gwc = None;
        } else {
            o.shplonk = None;
        }
        let e = emit_halo2_ingot(&vk(), &o).unwrap();
        assert!(e.lib_fe.contains("pub contract FixtureVerifier {"));
        assert!(e.lib_fe.contains(&format!("pub fn {has}(")));
        assert!(!e.lib_fe.contains(&format!("pub fn {lacks}(")));
        let t = e.tests_fe.unwrap();
        assert!(t.contains("_valid_proof_accepted()"));
        assert!(!t.contains("proof_of_other_scheme"));
        // a proof for a scheme that is not emitted, or a missing one for an emitted scheme: error
        let mut bad = o.clone();
        bad.scheme = if scheme == Halo2Scheme::Shplonk {
            Halo2Scheme::Gwc
        } else {
            Halo2Scheme::Shplonk
        };
        assert!(emit_halo2_ingot(&vk(), &bad).is_err());
    }
    // default scheme is SHPLONK
    assert_eq!(Halo2Scheme::default(), Halo2Scheme::Shplonk);
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

fn write_ingot(dir: &std::path::Path, toml: &str, lib: &str, tests: &str) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("fe.toml"), toml).unwrap();
    std::fs::write(dir.join("src/lib.fe"), lib).unwrap();
    std::fs::write(dir.join("src/tests.fe"), tests).unwrap();
}

#[test]
#[ignore = "needs the fe binary (set FE=/path/to/fe) and takes a few minutes"]
fn emitted_halo2_ingot_passes_fe_test() {
    let e = emit_halo2_ingot(&vk(), &opts()).unwrap();
    let base = root().join(".emit-halo2-test-tmp");
    let d = base.join("ok");
    write_ingot(&d, &e.fe_toml, &e.lib_fe, e.tests_fe.as_ref().unwrap());
    let (ok, text) = fe_test(&d);
    assert!(ok, "fe test failed\n{text}");
    assert!(text.contains("0 failed"), "{text}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
#[ignore = "needs the fe binary (set FE=/path/to/fe)"]
fn corrupted_halo2_key_fails_to_compile() {
    let e = emit_halo2_ingot(&vk(), &opts()).unwrap();
    let base = root().join(".emit-halo2-corrupt-tmp");
    let v = vk().v;
    let sg0y = v["permutation"]["commitments"][0]["y"]
        .as_str()
        .unwrap()
        .to_string();
    let n = num_bigint::BigUint::parse_bytes(sg0y.trim_start_matches("0x").as_bytes(), 16).unwrap();
    let from = format!("const SG0_Y: u256 = 0x{:0>64}", n.to_str_radix(16));
    let to = format!("const SG0_Y: u256 = 0x{:0>64}", (n + 1u8).to_str_radix(16));
    assert!(e.lib_fe.contains(&from));
    let sx = v["kzg"]["s_g2"]["x"][0].as_str().unwrap().to_string();
    let from2 = format!("const SRS_S_G2_X0: u256 = {sx}");
    assert!(e.lib_fe.contains(&from2));
    let cases = [
        ("off_curve_sigma", e.lib_fe.replace(&from, &to)),
        (
            "bad_s_g2",
            e.lib_fe
                .replace(&from2, &format!("const SRS_S_G2_X0: u256 = {ONE}")),
        ),
    ];
    for (n, lib) in cases {
        let d = base.join(n);
        write_ingot(&d, &e.fe_toml, &lib, e.tests_fe.as_ref().unwrap());
        let (ok, text) = fe_test(&d);
        assert!(!ok, "{n} compiled");
        assert!(text.contains("static assertion failed"), "{n}: {text}");
    }
    let _ = std::fs::remove_dir_all(&base);
}
