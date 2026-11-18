use clap::{Args, Parser, Subcommand};
use fe_zk_gen::{
    emit_halo2_ingot, emit_halo2_module, emit_ingot, emit_plonk_ingot, halo2_key_digest,
    key_digest, parse_arkworks, parse_arkworks_proof, parse_halo2_proof, parse_halo2_vk,
    parse_plonk_snarkjs, parse_plonk_snarkjs_proof, parse_public, parse_snarkjs,
    parse_snarkjs_proof, plonk_key_digest, EmitOptions, EmittedIngot, Groth16Proof, Groth16Vk,
    Halo2EmitOptions, PlonkEmitOptions, PlonkFixture, TestFixture, TOOL_VERSION,
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
    /// PLONK (snarkjs) over BN254 with KZG
    Plonk(PlonkArgs),
    /// halo2-KZG (halo2-axiom, EvmTranscript) over BN254, SHPLONK and GWC
    Halo2(Halo2Args),
}

#[derive(Args)]
struct Halo2Args {
    /// vk.json exported by vectors/halo2 (see vectors/halo2/SPEC.md)
    #[arg(long, value_name = "FILE")]
    vk: String,
    /// SHPLONK proof JSON (proof hex + instances); with --gwc emits tests with real proofs
    #[arg(long, value_name = "FILE", requires = "gwc")]
    shplonk: Option<String>,
    /// GWC proof JSON
    #[arg(long, value_name = "FILE", requires = "shplonk")]
    gwc: Option<String>,
    /// Name of the verifier (alphanumeric); contract is `<NAME>Verifier`
    #[arg(long, default_value = "Halo2")]
    name: String,
    /// Output directory for a standalone Fe ingot (contract + tests)
    #[arg(long, value_name = "DIR", conflicts_with = "lib_module")]
    out: Option<String>,
    /// Write only the circuit module for src/verifier/halo2/ of fe_zk itself (ingot:: imports)
    #[arg(long, value_name = "FILE")]
    lib_module: Option<String>,
    /// Path to the fe_zk ingot as written into fe.toml (relative to the output dir)
    #[arg(long, value_name = "PATH", default_value = "../..")]
    fe_zk_path: String,
}

fn run_halo2(a: Halo2Args) -> ExitCode {
    let vk = match read(&a.vk).and_then(|j| parse_halo2_vk(&j).map_err(|e| e.to_string())) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let digest = match halo2_key_digest(&vk) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: invalid verifying key: {e}");
            return ExitCode::from(1);
        }
    };
    println!("fe-zk-gen {TOOL_VERSION}: halo2-kzg/bn254 key: VALID");
    println!(
        "  key digest    : 0x{} (sha256 of canonical vk.json)",
        hex::encode(digest)
    );
    if let Some(path) = &a.lib_module {
        let text = match emit_halo2_module(&vk) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("error: {path}: {e}");
            return ExitCode::from(2);
        }
        println!("  emitted       : {path} (library module)");
        return ExitCode::SUCCESS;
    }
    let Some(out) = &a.out else {
        return ExitCode::SUCCESS;
    };
    let load_fx = |p: &Option<String>| -> Result<Option<_>, String> {
        match p {
            Some(p) => read(p)
                .and_then(|j| parse_halo2_proof(&j).map_err(|e| e.to_string()))
                .map(Some),
            None => Ok(None),
        }
    };
    let (shplonk, gwc) = match (load_fx(&a.shplonk), load_fx(&a.gwc)) {
        (Ok(s), Ok(g)) => (s, g),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let source = Path::new(&a.vk)
        .parent()
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| a.vk.clone());
    let opts = Halo2EmitOptions {
        name: a.name.clone(),
        source,
        fe_zk_path: a.fe_zk_path.clone(),
        shplonk,
        gwc,
    };
    let ing = match emit_halo2_ingot(&vk, &opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = write_ingot(out, &ing) {
        eprintln!("error: {out}: {e}");
        return ExitCode::from(2);
    }
    println!("  emitted       : {out} (ingot {})", ing.ingot_name);
    ExitCode::SUCCESS
}

#[derive(Args)]
struct PlonkArgs {
    /// snarkjs verification_key.json (protocol plonk)
    #[arg(long, value_name = "FILE")]
    snarkjs: String,
    /// snarkjs plonk proof.json; emits a test with a real proof
    #[arg(long, value_name = "FILE", requires = "public")]
    proof: Option<String>,
    /// public.json: decimal public signals of the proof
    #[arg(long, value_name = "FILE", requires = "proof")]
    public: Option<String>,
    /// Name of the verifier (alphanumeric); contract is `<NAME>Verifier`
    #[arg(long, default_value = "Plonk")]
    name: String,
    /// Output directory for the Fe ingot; without it the key is only validated
    #[arg(long, value_name = "DIR")]
    out: Option<String>,
    /// Path to the fe_zk ingot as written into fe.toml (relative to the output dir)
    #[arg(long, value_name = "PATH", default_value = "../..")]
    fe_zk_path: String,
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
    let p = a
        .arkworks
        .as_ref()
        .ok_or("one of --snarkjs / --arkworks is required")?;
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

fn write_ingot(out: &str, ing: &EmittedIngot) -> std::io::Result<()> {
    let dir = Path::new(out);
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
}

fn run_plonk(a: PlonkArgs) -> ExitCode {
    let vk = match read(&a.snarkjs).and_then(|j| parse_plonk_snarkjs(&j).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let digest = match plonk_key_digest(&vk) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: invalid verifying key: {e}");
            return ExitCode::from(1);
        }
    };
    println!("fe-zk-gen {TOOL_VERSION}: plonk/bn254 key from snarkjs: VALID");
    println!("  public inputs : {}", vk.n_public);
    println!("  domain        : 2^{}", vk.power);
    println!("  key digest    : 0x{}", hex::encode(digest));
    let Some(out) = &a.out else {
        return ExitCode::SUCCESS;
    };
    let fixture = match (&a.proof, &a.public) {
        (Some(pp), Some(pu)) => {
            let f = read(pp)
                .and_then(|j| parse_plonk_snarkjs_proof(&j).map_err(|e| e.to_string()))
                .and_then(|proof| {
                    read(pu)
                        .and_then(|j| parse_public(&j).map_err(|e| e.to_string()))
                        .map(|public| PlonkFixture { proof, public })
                });
            match f {
                Ok(f) => Some(f),
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(2);
                }
            }
        }
        _ => None,
    };
    let source = Path::new(&a.snarkjs)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(a.snarkjs.clone());
    let opts = PlonkEmitOptions {
        name: a.name.clone(),
        source,
        fe_zk_path: a.fe_zk_path.clone(),
        fixture,
    };
    let ing = match emit_plonk_ingot(&vk, &opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = write_ingot(out, &ing) {
        eprintln!("error: {out}: {e}");
        return ExitCode::from(2);
    }
    println!("  emitted       : {out} (ingot {})", ing.ingot_name);
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args = match Cli::parse().cmd {
        Cmd::Groth16(a) => a,
        Cmd::Plonk(a) => return run_plonk(a),
        Cmd::Halo2(a) => return run_halo2(a),
    };
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
    if let Err(e) = write_ingot(out, &ing) {
        eprintln!("error: {out}: {e}");
        return ExitCode::from(2);
    }
    println!("  emitted       : {out} (ingot {})", ing.ingot_name);
    ExitCode::SUCCESS
}
