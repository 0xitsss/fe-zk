use clap::{Args, Parser, Subcommand};
use fe_zk_gen::{key_digest, parse_arkworks, parse_snarkjs, Groth16Vk, TOOL_VERSION};
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
#[group(required = true, multiple = false)]
struct Groth16Args {
    /// snarkjs verification_key.json
    #[arg(long, value_name = "FILE")]
    snarkjs: Option<String>,
    /// arkworks verifying key JSON (see gen/README.md)
    #[arg(long, value_name = "FILE")]
    arkworks: Option<String>,
}

fn load(a: &Groth16Args) -> Result<(Groth16Vk, &'static str), String> {
    if let Some(p) = &a.snarkjs {
        let s = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        return parse_snarkjs(&s)
            .map(|v| (v, "snarkjs"))
            .map_err(|e| e.to_string());
    }
    let p = a
        .arkworks
        .as_ref()
        .expect("clap group guarantees one source");
    let s = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
    parse_arkworks(&s)
        .map(|v| (v, "arkworks"))
        .map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let Cmd::Groth16(args) = Cli::parse().cmd;
    let (vk, src) = match load(&args) {
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
    // TODO(emission task): call fe_zk_gen::emit_fe(&vk) and write the Fe verifier + test.
    println!("  emission      : not implemented yet");
    ExitCode::SUCCESS
}
