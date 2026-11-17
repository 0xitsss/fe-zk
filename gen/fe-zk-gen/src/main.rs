use clap::{Args, Parser, Subcommand};
use fe_zk_gen::{
    emit_ingot, key_digest, parse_arkworks, parse_arkworks_proof, parse_public, parse_snarkjs,
    parse_snarkjs_proof, EmitOptions, Groth16Proof, Groth16Vk, TestFixture, TOOL_VERSION,
};
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "fe-zk-gen",
    version,
    about = "Validate zk verifying keys and emit safe Fe verifiers"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Groth16 over BN254
    Groth16(Groth16Args),
}

#[derive(Args)]
struct Groth16Args {
    /// snarkjs verification_key.json
    #[arg(long, value_name = "FILE", conflicts_with = "arkworks")]
    snarkjs: Option<String>,
    /// arkworks verifying key JSON (see gen/README.md)
    #[arg(long, value_name = "FILE", conflicts_with = "snarkjs")]
    arkworks: Option<String>,
    /// proof.json (snarkjs) or arkworks proof JSON; emits a test with a real proof
    #[arg(long, value_name = "FILE", requires = "public")]
    proof: Option<String>,
    /// public.json: decimal public inputs of the proof
    #[arg(long, value_name = "FILE", requires = "proof")]
    public: Option<String>,
    /// Name of the verifier (alphanumeric); contract is `<NAME>Verifier`
    #[arg(long, default_value = "Groth16")]
    name: String,
    /// Output directory for the Fe ingot; without it the key is only validated
    #[arg(long, value_name = "DIR")]
    out: Option<String>,
    /// Path to the fe_zk ingot as written into fe.toml (relative to the output dir)
    #[arg(long, value_name = "PATH", default_value = "../..")]
    fe_zk_path: String,
}

fn read(p: &str) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))
}

fn load(a: &Groth16Args) -> Result<(Groth16Vk, &'static str, String), String> {
    if let Some(p) = &a.snarkjs {
        return parse_snarkjs(&read(p)?)
            .map(|v| (v, "snarkjs", p.clone()))
            .map_err(|e| e.to_string());
    }
    let p = a.arkworks.as_ref().ok_or("one of --snarkjs / --arkworks is required")?;
    parse_arkworks(&read(p)?)
        .map(|v| (v, "arkworks", p.clone()))
        .map_err(|e| e.to_string())
}

fn load_fixture(a: &Groth16Args, ark: bool) -> Result<Option<TestFixture>, String> {
    let (Some(pp), Some(pu)) = (&a.proof, &a.public) else {
        return Ok(None);
    };
    let proof: Groth16Proof = if ark {
        parse_arkworks_proof(&read(pp)?)
    } else {
        parse_snarkjs_proof(&read(pp)?)
    }
    .map_err(|e| e.to_string())?;
    let public = parse_public(&read(pu)?).map_err(|e| e.to_string())?;
    Ok(Some(TestFixture { proof, public }))
}

fn main() -> ExitCode {
    let Cmd::Groth16(args) = Cli::parse().cmd;
    let (vk, src, src_path) = match load(&args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let digest = match key_digest(&vk) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: invalid verifying key: {e}");
            return ExitCode::from(1);
        }
    };
    println!("fe-zk-gen {TOOL_VERSION}: groth16/bn254 key from {src}: VALID");
    println!("  public inputs : {}", vk.n_public);
    println!("  IC points     : {}", vk.ic.len());
    println!("  key digest    : 0x{}", hex::encode(digest));
    let Some(out) = &args.out else {
        return ExitCode::SUCCESS;
    };
    let fixture = match load_fixture(&args, src == "arkworks") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let file_name = Path::new(&src_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(src_path.clone());
    let opts = EmitOptions {
        name: args.name.clone(),
        source: file_name,
        fe_zk_path: args.fe_zk_path.clone(),
        fixture,
    };
    let ing = match emit_ingot(&vk, &opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let dir = Path::new(out);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir.join("src"))?;
        std::fs::write(dir.join("fe.toml"), &ing.fe_toml)?;
        std::fs::write(dir.join("src/lib.fe"), &ing.lib_fe)?;
        let t = dir.join("src/tests.fe");
        match &ing.tests_fe {
            Some(s) => std::fs::write(t, s)?,
            None => {
                let _ = std::fs::remove_file(t);
            }
        }
        Ok(())
    };
    if let Err(e) = write() {
        eprintln!("error: {out}: {e}");
        return ExitCode::from(2);
    }
    println!("  emitted       : {out} (ingot {})", ing.ingot_name);
    ExitCode::SUCCESS
}
