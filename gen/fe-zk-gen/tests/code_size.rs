//! Code size guard (ignored, needs the `fe` binary): every verifier the generator can emit must fit
//! the EIP-170 limit of 24,576 bytes of runtime code, otherwise it cannot be deployed on Ethereum.
//! The check is `scripts/code-size.sh` (the same script `scripts/demo.sh` ends with).
//! Run: `FE=/path/to/fe cargo test --test code_size -- --ignored --nocapture`.

use fe_zk_gen::{
    emit_halo2_ingot, emit_ingot, emit_plonk_ingot, parse_arkworks, parse_halo2_vk,
    parse_plonk_snarkjs, parse_snarkjs, EmitOptions, EmittedIngot, Halo2EmitOptions, Halo2Scheme,
    PlonkEmitOptions,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let p = root().join("vectors").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn write_ingot(dir: &Path, e: &EmittedIngot) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("fe.toml"), &e.fe_toml).unwrap();
    std::fs::write(dir.join("src/lib.fe"), &e.lib_fe).unwrap();
}

#[test]
#[ignore = "needs the fe binary (set FE=/path/to/fe); builds every emitted verifier, several minutes"]
fn every_emitted_verifier_fits_eip170() {
    // fe.toml rejects absolute dependency paths, so emit inside the repo (removed afterwards)
    let base = root().join(".emit-size-tmp");
    let mut dirs: Vec<PathBuf> = vec![];

    for n in ["multiplier2", "membership", "life_512"] {
        let vk = parse_snarkjs(&read(&format!("groth16/{n}/verification_key.json"))).unwrap();
        let e = emit_ingot(
            &vk,
            &EmitOptions {
                fe_zk_path: "../..".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let d = base.join(format!("groth16_{n}"));
        write_ingot(&d, &e);
        dirs.push(d);
    }
    let vk = parse_arkworks(&read("groth16/arkworks_multiplier/verification_key.json")).unwrap();
    let e = emit_ingot(
        &vk,
        &EmitOptions {
            fe_zk_path: "../..".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let d = base.join("groth16_arkworks");
    write_ingot(&d, &e);
    dirs.push(d);

    for n in ["multiplier2", "membership"] {
        let vk = parse_plonk_snarkjs(&read(&format!("plonk/{n}/verification_key.json"))).unwrap();
        let e = emit_plonk_ingot(
            &vk,
            &PlonkEmitOptions {
                fe_zk_path: "../..".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let d = base.join(format!("plonk_{n}"));
        write_ingot(&d, &e);
        dirs.push(d);
    }

    let hvk = parse_halo2_vk(&read("halo2/standard_plonk_lookup/vk.json")).unwrap();
    for (n, scheme) in [
        ("shplonk", Halo2Scheme::Shplonk),
        ("gwc", Halo2Scheme::Gwc),
        ("both", Halo2Scheme::Both),
    ] {
        let e = emit_halo2_ingot(
            &hvk,
            &Halo2EmitOptions {
                scheme,
                fe_zk_path: "../..".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let d = base.join(format!("halo2_{n}"));
        write_ingot(&d, &e);
        dirs.push(d);
    }

    let rel: Vec<String> = dirs
        .iter()
        .map(|d| {
            d.strip_prefix(root())
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let out = std::process::Command::new(root().join("scripts/code-size.sh"))
        .args(&rel)
        .current_dir(root())
        .output()
        .expect("run scripts/code-size.sh");
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    println!("{text}");
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        out.status.success(),
        "a verifier is over the EIP-170 limit or did not build\n{text}"
    );
}
