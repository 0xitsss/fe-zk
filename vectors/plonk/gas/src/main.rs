//! Usage: plonk-gas <creation.bin hex file> <calldata hex file>
//! Deploys the contract in an in-memory revm (Osaka rules, as Fe's test harness) and calls it once.
//! Prints `gas=<execution gas without the 21000 intrinsic and calldata cost> ret=<hex>`.
use revm::{
    bytecode::Bytecode,
    context::{
        result::{ExecutionResult, Output},
        CfgEnv, Context, TxEnv,
    },
    database::InMemoryDB,
    handler::{ExecuteCommitEvm, MainBuilder, MainContext},
    primitives::{hardfork::SpecId, Address, Bytes, TxKind, U256},
    state::AccountInfo,
};

fn parse_hex(s: &str) -> Vec<u8> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn cfg(c: &mut CfgEnv) {
    c.spec = SpecId::OSAKA;
    c.tx_gas_limit_cap = Some(u64::MAX);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let init = parse_hex(&std::fs::read_to_string(&args[1]).expect("bin"));
    let calldata = parse_hex(&std::fs::read_to_string(&args[2]).expect("calldata"));
    let caller = Address::ZERO;
    let mut db = InMemoryDB::default();
    db.insert_account_info(
        caller,
        AccountInfo::new(U256::from(1_000_000_000u64), 0, Default::default(), Bytecode::default()),
    );
    let mut evm = Context::mainnet().with_db(db).modify_cfg_chained(cfg).build_mainnet();
    let tx = TxEnv::builder()
        .caller(caller)
        .gas_limit(1_000_000_000)
        .gas_price(0)
        .kind(TxKind::Create)
        .data(Bytes::from(init))
        .nonce(0)
        .build()
        .unwrap();
    let address = match evm.transact_commit(tx).unwrap() {
        ExecutionResult::Success { output: Output::Create(_, Some(a)), .. } => a,
        other => panic!("deploy failed: {other:?}"),
    };
    let intr: u64 = 21_000 + calldata.iter().map(|b| if *b == 0 { 4 } else { 16 }).sum::<u64>();
    let tx = TxEnv::builder()
        .caller(caller)
        .gas_limit(1_000_000_000)
        .gas_price(0)
        .to(address)
        .data(Bytes::from(calldata))
        .nonce(1)
        .build()
        .unwrap();
    match evm.transact_commit(tx).unwrap() {
        ExecutionResult::Success { output: Output::Call(ret), gas_used, .. } => {
            let hex: String = ret.iter().map(|b| format!("{b:02x}")).collect();
            println!("gas={} ret={}", gas_used - intr, hex);
        }
        other => panic!("call failed: {other:?}"),
    }
}
