//! evm-batch: deploys several contracts (creation bytecode) into one in-memory revm (Osaka, as
//! Fe's test harness) and runs many calls against them.
//!
//!   evm-batch --init NAME=<creation.bin hex file> [--init ...] < cases
//!
//! stdin: one case per line, `NAME <sig|-> <body hex>`: calldata = keccak(sig)[..4] ++ body, or just
//! body when sig is `-`; hex without 0x, the body may be empty.
//! stdout: one line per case: `a` accepted (call succeeded and returned `true` or nothing),
//! `r` rejected (returned false / any other value), `v` reverted or halted. Contracts are
//! stateless verifiers, so state carried over between cases is harmless.
use revm::{
    bytecode::Bytecode,
    context::{
        result::{ExecutionResult, Output},
        CfgEnv, Context, TxEnv,
    },
    database::InMemoryDB,
    handler::{ExecuteCommitEvm, MainBuilder, MainContext},
    primitives::{hardfork::SpecId, keccak256, Address, Bytes, TxKind, U256},
    state::AccountInfo,
};
use std::collections::HashMap;
use std::io::{BufRead, Write};

fn parse_hex(s: &str) -> Vec<u8> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

fn cfg(c: &mut CfgEnv) {
    c.spec = SpecId::OSAKA;
    c.tx_gas_limit_cap = Some(u64::MAX);
    c.limit_contract_code_size = Some(usize::MAX);
    c.limit_contract_initcode_size = Some(usize::MAX);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut inits: Vec<(String, Vec<u8>)> = vec![];
    let mut i = 0;
    while i < args.len() {
        assert_eq!(args[i], "--init", "unknown arg {}", args[i]);
        let (n, p) = args[i + 1].split_once('=').expect("NAME=file");
        inits.push((n.to_string(), parse_hex(&std::fs::read_to_string(p).expect("init file"))));
        i += 2;
    }
    let caller = Address::ZERO;
    let mut db = InMemoryDB::default();
    db.insert_account_info(
        caller,
        AccountInfo::new(U256::from(1_000_000_000u64), 0, Default::default(), Bytecode::default()),
    );
    let mut evm = Context::mainnet().with_db(db).modify_cfg_chained(cfg).build_mainnet();
    let mut nonce = 0u64;
    let mut addrs: HashMap<String, Address> = HashMap::new();
    for (name, code) in inits {
        let tx = TxEnv::builder()
            .caller(caller)
            .gas_limit(1_000_000_000)
            .gas_price(0)
            .kind(TxKind::Create)
            .data(Bytes::from(code))
            .nonce(nonce)
            .build()
            .unwrap();
        nonce += 1;
        match evm.transact_commit(tx).unwrap() {
            ExecutionResult::Success { output: Output::Create(_, Some(a)), .. } => {
                addrs.insert(name, a);
            }
            other => panic!("deploy {name} failed: {other:?}"),
        }
    }
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(3, ' ');
        let name = it.next().unwrap();
        let sig = it.next().unwrap_or("-");
        let body = it.next().unwrap_or("");
        let mut data = if sig == "-" { vec![] } else { keccak256(sig.as_bytes())[..4].to_vec() };
        data.extend(parse_hex(body));
        let to = *addrs.get(name).unwrap_or_else(|| panic!("unknown contract {name}"));
        let tx = TxEnv::builder()
            .caller(caller)
            .gas_limit(200_000_000)
            .gas_price(0)
            .to(to)
            .data(Bytes::from(data))
            .nonce(nonce)
            .build()
            .unwrap();
        nonce += 1;
        let c = match evm.transact_commit(tx).unwrap() {
            ExecutionResult::Success { output: Output::Call(ret), .. } => {
                let one = ret.len() == 32 && ret[..31].iter().all(|b| *b == 0) && ret[31] == 1;
                if ret.is_empty() || one {
                    'a'
                } else {
                    'r'
                }
            }
            ExecutionResult::Success { .. } => 'r',
            ExecutionResult::Revert { .. } | ExecutionResult::Halt { .. } => 'v',
        };
        writeln!(out, "{c}").unwrap();
    }
}
