//! Plonky3 FRI/STARK generator tests: Rust port vs the Python emitter (byte-identical) for the
//! three committed fixtures, snapshots of the emitted modules (`UPDATE_SNAPSHOTS=1` rewrites
//! them), the committed Fe files match a fresh emission (modulo `fe fmt`), and rejections.

use fe_zk_gen::{emit_plonky3, emit_plonky3_dir, Plonky3EmitOptions, Plonky3Emitted};
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn p3() -> PathBuf {
    root().join("vectors/plonky3-stark")
}

/// `(dir, name, air, vectors, min_bits)` exactly as in scripts/regen-plonky3-stark.sh.
fn cases() -> Vec<(&'static str, &'static str, &'static str, bool, Option<u64>)> {
    vec![
        ("fib", "demo", "fib", true, None),
        ("fib_secure", "secure", "fib", false, Some(100)),
        ("mul", "mul", "mul", false, Some(100)),
    ]
}

fn opts(c: &(&str, &str, &str, bool, Option<u64>)) -> Plonky3EmitOptions {
    Plonky3EmitOptions {
        dir: c.0.into(),
        name: c.1.into(),
        air: c.2.into(),
        vectors: c.3,
        min_bits: c.4,
    }
}

fn emit(c: &(&str, &str, &str, bool, Option<u64>)) -> Plonky3Emitted {
    emit_plonky3_dir(&p3().join(c.0), &opts(c)).unwrap()
}

#[test]
fn rust_port_is_byte_identical_to_python_emitter() {
    let tmp = std::env::temp_dir().join(format!("fe-zk-p3-xcheck-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    for c in cases() {
        let (fx, cf) = (tmp.join(format!("{}_fx.fe", c.0)), tmp.join(format!("{}_cf.fe", c.0)));
        let mut cmd = std::process::Command::new("python3");
        cmd.current_dir(p3())
            .arg("emit_fe.py")
            .arg(c.0)
            .arg(&fx)
            .arg(&cf)
            .arg(c.1)
            .args(["--air", c.2]);
        if c.3 {
            cmd.arg("--vectors");
        }
        if let Some(m) = c.4 {
            cmd.args(["--min-bits", &m.to_string()]);
        }
        let Ok(out) = cmd.output() else {
            eprintln!("python3 not found, skipping cross-check");
            return;
        };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let rs = emit(&c);
        assert_eq!(std::fs::read_to_string(&fx).unwrap(), rs.fixture_fe, "{} fixture", c.0);
        assert_eq!(std::fs::read_to_string(&cf).unwrap(), rs.config_fe, "{} config", c.0);
    }
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn config_modules_match_snapshots() {
    for c in cases() {
        let got = emit(&c).config_fe;
        let path = root().join(format!("gen/fe-zk-gen/tests/snapshots/plonky3_{}_config.fe", c.1));
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (run with UPDATE_SNAPSHOTS=1)", path.display()));
        assert_eq!(want, got, "snapshot {} differs (UPDATE_SNAPSHOTS=1 to accept)", path.display());
    }
}

/// The committed `src/verifier/fri/config_*.fe` carry the parameters and pattern hashes of a fresh
/// emission (the committed files are `fe fmt`-ed, so compare the `FriParams` fields line by line).
#[test]
fn committed_config_modules_are_current() {
    for c in cases() {
        let got = emit(&c).config_fe;
        let committed =
            std::fs::read_to_string(root().join(format!("src/verifier/fri/config_{}.fe", c.1))).unwrap();
        for line in got.lines().filter(|l| l.starts_with("    ") && l.trim_end().ends_with(',')) {
            assert!(committed.contains(line.trim()), "config_{}: missing `{}`", c.1, line.trim());
        }
        for line in got.lines().filter(|l| l.starts_with("pub const") || l.starts_with("static_assert")) {
            assert!(committed.contains(line), "config_{}: missing `{line}`", c.1);
        }
    }
}

fn load(dir: &str) -> (Value, Value, Value) {
    let r = |n: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(p3().join(dir).join(n)).unwrap()).unwrap()
    };
    (r("proof.json"), r("config.json"), r("vectors.json"))
}

#[test]
fn rejects_unsupported_or_inconsistent_inputs() {
    let c = cases()[1];
    let o = opts(&c);
    let (proof, cfg, vec) = load(c.0);
    assert!(emit_plonky3(&proof, &cfg, &vec, &o).is_ok());

    // soundness below the requested minimum: refused at generation time
    let mut hi = o.clone();
    hi.min_bits = Some(101);
    assert!(emit_plonky3(&proof, &cfg, &vec, &hi).unwrap_err().contains("< required 101"));

    // a Merkle cap of height 1 (two roots)
    let mut p = proof.clone();
    let cap = p["commitments"]["trace"]["cap"].as_array_mut().unwrap();
    cap.push(cap[0].clone());
    assert!(emit_plonky3(&p, &cfg, &vec, &o).unwrap_err().contains("cap height 0"));

    // nonzero batch PoW witness at a zero-bit site
    let mut p = proof.clone();
    p["opening_proof"]["batch_pow_witness"] = 1.into();
    assert!(emit_plonky3(&p, &cfg, &vec, &o).is_err());

    // a field element out of range
    let mut p = proof.clone();
    p["opening_proof"]["query_pow_witness"] = 0x7f00_0001u64.into();
    assert!(emit_plonky3(&p, &cfg, &vec, &o).unwrap_err().contains("out of range"));

    // a domain-separator pattern hash that does not match Plonky3's seed elements
    let mut v = vec.clone();
    v["transcript"]["seeds"]["fri"]["pattern_hash"] = format!("0x{}", "00".repeat(32)).into();
    assert!(emit_plonky3(&proof, &cfg, &v, &o).unwrap_err().contains("fri domain separator"));

    // a truncated multi-opening changes nothing the emitter can check but the count bound;
    // a configuration that disagrees with the proof is refused
    let mut cf = cfg.clone();
    cf["num_queries"] = 21.into();
    assert!(emit_plonky3(&proof, &cf, &vec, &o).is_err());

    // unknown AIR
    let mut bad = o.clone();
    bad.air = "keccak".into();
    assert!(emit_plonky3(&proof, &cfg, &vec, &bad).is_err());
}

#[test]
fn emission_is_deterministic() {
    for c in cases() {
        assert_eq!(emit(&c), emit(&c));
    }
}
