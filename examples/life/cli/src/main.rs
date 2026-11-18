//! life-cli: play Game of Life by executing the compiled Fe `Life` contract.
//!
//! Every generation is one `step(board)` call into the contract (in revm). `--check` compares
//! each generation with a per-cell Rust reference. `--prove` deploys the generated Fe Groth16
//! verifier plus `Life`, advances the stored board by one proof and prints the gas comparison.
mod abi;
mod board;
mod chain;
mod frames;
mod zk;

use std::{io::Write, process::ExitCode, thread, time::Duration};

struct Args {
    pattern: String,
    gens: u64,
    delay_ms: u64,
    seed: u64,
    no_anim: bool,
    check: bool,
    bin: String,
    prove: bool,
    prove_live: bool,
    fixture: String,
    verifier_bin: String,
    frames_json: Option<String>,
}

const USAGE: &str = "usage: life-cli --pattern glider|blinker|rpentomino|random --gens N \
[--delay ms] [--seed S] [--no-anim] [--check] [--bin path/to/Life.bin] [--frames-json PATH]\n\
       life-cli --prove [--fixture DIR] [--bin Life.bin] [--verifier-bin Life512Verifier.bin] [--frames-json PATH]\n\
       life-cli --prove-live [--pattern P] [--seed S]   (runs scripts/life-prove.sh, takes minutes)";

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        pattern: "glider".into(),
        gens: 16,
        delay_ms: 80,
        seed: 1,
        no_anim: false,
        check: false,
        bin: concat!(env!("CARGO_MANIFEST_DIR"), "/../out/Life.bin").into(),
        prove: false,
        prove_live: false,
        fixture: concat!(env!("CARGO_MANIFEST_DIR"), "/../../../vectors/groth16/life_512").into(),
        verifier_bin: concat!(env!("CARGO_MANIFEST_DIR"), "/../verifier/out/Life512Verifier.bin").into(),
        frames_json: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match k.as_str() {
            "--pattern" => a.pattern = val("--pattern")?,
            "--gens" => a.gens = val("--gens")?.parse().map_err(|e| format!("--gens: {e}"))?,
            "--delay" => a.delay_ms = val("--delay")?.parse().map_err(|e| format!("--delay: {e}"))?,
            "--seed" => a.seed = val("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--bin" => a.bin = val("--bin")?,
            "--no-anim" => a.no_anim = true,
            "--check" => a.check = true,
            "--prove" => a.prove = true,
            "--prove-live" => {
                a.prove = true;
                a.prove_live = true;
            }
            "--fixture" => a.fixture = val("--fixture")?,
            "--verifier-bin" => a.verifier_bin = val("--verifier-bin")?,
            "--frames-json" => a.frames_json = Some(val("--frames-json")?),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(a)
}

fn run() -> Result<(), String> {
    let a = parse_args()?;
    if a.prove {
        return prove_mode(&a);
    }
    let start = board::pattern(&a.pattern, a.seed).ok_or(format!("unknown pattern {}", a.pattern))?;
    let init = std::fs::read_to_string(&a.bin).map_err(|e| format!("{}: {e} (run `fe build` in examples/life)", a.bin))?;
    let mut chain = chain::Chain::deploy(&init, &[0u8; 32])?; // verifier address: unused by the pure messages

    let mut b = start;
    let mut total_exec = 0u64;
    let mut total_tx = 0u64;
    let mut expect = start;
    let mut frames = vec![frames::Frame { generation: 0, board: start, step_gas: None }];
    let animate = !a.no_anim;
    let mut out = std::io::stdout();
    if animate {
        write!(out, "\x1b[2J").ok();
    }
    for g in 0..=a.gens {
        let mut gas_line = String::from("start");
        if g > 0 {
            let (next, call) = chain.step(b)?;
            b = next;
            total_exec += call.exec_gas;
            total_tx += call.tx_gas;
            gas_line = format!("step gas {} (tx {})", call.exec_gas, call.tx_gas);
            frames.push(frames::Frame { generation: g, board: b, step_gas: Some(call.exec_gas) });
            if a.check {
                expect = board::ref_step_packed(expect);
                if b != expect {
                    return Err(format!(
                        "MISMATCH at generation {g}: contract {b:#x}, reference {expect:#x}"
                    ));
                }
            }
        }
        let header = format!("gen {g:>4}  alive {:>3}  {gas_line}", board::alive(b));
        let last = g == a.gens;
        if animate {
            write!(out, "\x1b[H{header}\x1b[K\n{}", board::render(b, true)).ok();
            out.flush().ok();
            thread::sleep(Duration::from_millis(a.delay_ms));
        } else if last || g == 0 || g % 16 == 0 {
            println!("{header}");
            if last {
                print!("{}", board::render(b, false));
            }
        }
    }

    let (run_b, run_call) = chain.run(start, a.gens)?;
    if run_b != b {
        return Err(format!("run({}) disagrees with {} step calls", a.gens, a.gens));
    }
    if let Some(path) = &a.frames_json {
        frames::write_play(path, &a.pattern, a.gens, &frames, run_call.exec_gas)?;
        println!("frames written to {path}");
    }
    if a.check {
        println!("check: OK, {} generations match the per-cell reference", a.gens);
    }
    println!("pattern {}  generations {}", a.pattern, a.gens);
    println!("gas, {} separate step() calls: {} execution, {} with tx overhead", a.gens, total_exec, total_tx);
    println!("gas, one run(board, {}) call:  {} execution, {} with tx overhead", a.gens, run_call.exec_gas, run_call.tx_gas);
    println!("zk path: life-cli --prove (Groth16 proof of 512 generations, one verification)");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read_bin(path: &str, what: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e} (run `fe build` for the {what})"))
}

fn show(label: &str, b: revm::primitives::U256) {
    println!("{label}  alive {}", board::alive(b));
    print!("{}", board::render(b, false));
}

/// Deploys verifier + Life, then compares: N separate step() calls, one run(board, N), and one
/// Groth16 proof of the same N generations (`advanceProven`).
fn prove_mode(a: &Args) -> Result<(), String> {
    use revm::primitives::U256;
    let mut fixture_dir = std::path::PathBuf::from(&a.fixture);
    if a.prove_live {
        // Proves N = 512 generations of the chosen pattern with snarkjs into a scratch dir. The
        // emitted verifier is bound to the committed verifying key; a proof from a different
        // setup run is rejected by it (the script uses fixed entropy, so keys normally match).
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let out = std::env::temp_dir().join("life-prove-live");
        let status = std::process::Command::new(root.join("scripts/life-prove.sh"))
            .args([a.pattern.as_str(), "512", &a.seed.to_string()])
            .env("OUT", &out)
            .env("NO_NEG", "1")
            .status()
            .map_err(|e| format!("scripts/life-prove.sh: {e}"))?;
        if !status.success() {
            return Err("scripts/life-prove.sh failed".into());
        }
        fixture_dir = out;
    }
    let f = zk::load(&fixture_dir)?;
    let verifier_init = read_bin(&a.verifier_bin, "generated verifier in examples/life/verifier")?;
    let life_init = read_bin(&a.bin, "Life contract in examples/life")?;
    let mut chain = chain::Chain::deploy(&verifier_init, &[])?;
    let mut ctor = [0u8; 32];
    ctor[12..].copy_from_slice(chain.address.as_slice());
    chain.create(&life_init, &ctor)?;

    let n_out = chain.call(abi::selector("provenGenerations()").to_vec())?;
    let n = abi::decode_word(&n_out.ret)?.to::<u64>();
    println!("fixture {}  proven generations {n}", fixture_dir.display());
    show("start board", f.start);

    // 1. the same n generations on chain
    let (mut b, mut steps_exec, mut steps_tx) = (f.start, 0u64, 0u64);
    for _ in 0..n {
        let (next, call) = chain.step(b)?;
        b = next;
        steps_exec += call.exec_gas;
        steps_tx += call.tx_gas;
    }
    let (run_b, run_call) = chain.run(f.start, n)?;
    if b != f.end || run_b != f.end {
        return Err("on-chain end board differs from the circuit's end board".into());
    }
    // per-cell reference, independent of both
    let mut r = f.start;
    for _ in 0..n {
        r = board::ref_step_packed(r);
    }
    if r != f.end {
        return Err("per-cell reference differs from the fixture end board".into());
    }

    // 2. one proof: store the start board, then advance by n generations
    chain.call(zk::encode_set_committed(f.start))?;
    let prove_run = chain.call(zk::encode_prove_run(&f))?;
    let adv = chain.call(zk::encode_advance_proven(f.end, &f.proof))?;
    let stored: U256 = abi::decode_word(&chain.call(abi::selector("board()").to_vec())?.ret)?;
    if stored != f.end {
        return Err("advanceProven did not store the end board".into());
    }
    show(&format!("end board after {n} generations (one proof)"), stored);

    // 3. rejections
    let mut bad = f.proof;
    bad[0] += U256::from(1u8);
    chain.call(zk::encode_set_committed(f.start))?;
    let tampered = chain.try_call(zk::encode_advance_proven(f.end, &bad))?.is_none();
    let wrong_end = chain.try_call(zk::encode_advance_proven(f.end ^ U256::from(1u8), &f.proof))?.is_none();
    chain.call(zk::encode_set_committed(f.start ^ U256::from(1u8)))?;
    let wrong_start = chain.try_call(zk::encode_advance_proven(f.end, &f.proof))?.is_none();
    let verdict = |rejected: bool| if rejected { "REJECTED" } else { "ACCEPTED (bug!)" };
    println!("tampered proof:    {}", verdict(tampered));
    println!("wrong end board:   {}", verdict(wrong_end));
    println!("wrong start board: {}", verdict(wrong_start));
    if !(tampered && wrong_end && wrong_start) {
        return Err("an invalid proof was accepted".into());
    }

    println!();
    println!("{:<50} {:>12} {:>12}", "gas (revm, Osaka)", "execution", "with tx");
    let row = |name: String, e: u64, t: u64| println!("{name:<50} {e:>12} {t:>12}");
    row(format!("{n} separate step() calls"), steps_exec, steps_tx);
    row(format!("one run(board, {n}) call"), run_call.exec_gas, run_call.tx_gas);
    row("one proveRun (verify proof of the same run)".into(), prove_run.exec_gas, prove_run.tx_gas);
    row("one advanceProven (verify + 2 Poseidon + store)".into(), adv.exec_gas, adv.tx_gas);
    let per_gen = run_call.exec_gas as f64 / n as f64;
    println!(
        "\nbreak-even: a proof pays off above about {:.0} generations ({per_gen:.0} gas per on-chain generation)",
        prove_run.exec_gas as f64 / per_gen
    );
    if let Some(path) = &a.frames_json {
        frames::write_prove(path, n, f.start, f.end, steps_exec, run_call.exec_gas, prove_run.exec_gas, adv.exec_gas)?;
        println!("frames written to {path}");
    }
    Ok(())
}
