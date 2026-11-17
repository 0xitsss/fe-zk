//! life-cli: play Game of Life by executing the compiled Fe `Life` contract.
//!
//! Every generation is one `step(board)` call into the contract (in revm). `--check` compares
//! each generation with a per-cell Rust reference. A later `--prove` mode (Groth16 proof of N
//! generations verified by a generated Fe verifier) plugs in next to `Mode::Play`; see the TODO.
mod abi;
mod board;
mod chain;

use std::{io::Write, process::ExitCode, thread, time::Duration};

struct Args {
    pattern: String,
    gens: u64,
    delay_ms: u64,
    seed: u64,
    no_anim: bool,
    check: bool,
    bin: String,
}

const USAGE: &str = "usage: life-cli --pattern glider|blinker|rpentomino|random --gens N \
[--delay ms] [--seed S] [--no-anim] [--check] [--bin path/to/Life.bin]";

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        pattern: "glider".into(),
        gens: 16,
        delay_ms: 80,
        seed: 1,
        no_anim: false,
        check: false,
        bin: concat!(env!("CARGO_MANIFEST_DIR"), "/../out/Life.bin").into(),
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
            // TODO(zk): "--prove" => prove N generations off chain and verify the Groth16 proof.
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(a)
}

fn run() -> Result<(), String> {
    let a = parse_args()?;
    let start = board::pattern(&a.pattern, a.seed).ok_or(format!("unknown pattern {}", a.pattern))?;
    let init = std::fs::read_to_string(&a.bin).map_err(|e| format!("{}: {e} (run `fe build` in examples/life)", a.bin))?;
    let mut chain = chain::Chain::deploy(&init)?;

    let mut b = start;
    let mut total_exec = 0u64;
    let mut total_tx = 0u64;
    let mut expect = start;
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
    if a.check {
        println!("check: OK, {} generations match the per-cell reference", a.gens);
    }
    println!("pattern {}  generations {}", a.pattern, a.gens);
    println!("gas, {} separate step() calls: {} execution, {} with tx overhead", a.gens, total_exec, total_tx);
    println!("gas, one run(board, {}) call:  {} execution, {} with tx overhead", a.gens, run_call.exec_gas, run_call.tx_gas);
    // TODO(zk): print gas of one Groth16 proof verification next to the numbers above.
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
