//! gas-run: deploys a contract from creation bytecode into an in-memory revm (Osaka rules, as Fe's
//! test harness) and executes transactions against it. One JSON object is printed on stdout.
//!
//!   gas-run --init <creation.bin hex file>
//!           [--call NAME=<calldata hex file>]...
//!           [--iterate NAME=<signature>:<start word hex>:<N>]...
//!
//! `--call`: one transaction with the calldata from the file (hex, or `sig:<signature>` on the first
//! line and the hex of the arguments on the second: the selector is then prepended).
//! `--iterate`: N transactions `selector(signature) ++ word`, each fed the previous return word
//! (the first is fed the start word); gas is summed over the N transactions.
//!
//! Per transaction two numbers are reported:
//!   exec = gas spent by the top-level call frame (the contract's own execution, precompiles
//!          and sub-calls included; excludes the 21000 intrinsic and calldata cost), taken with an
//!          inspector so the EIP-7623 calldata floor cannot hide it;
//!   tx   = gas charged for the whole transaction (`gas_used` of the receipt: intrinsic 21000 +
//!          calldata + exec, raised to the EIP-7623 floor if that is larger).
//! Every transaction starts from the post-deploy state in a fresh block: the target account and the
//! precompiles are warm, storage slots are cold.
use revm::{
    bytecode::Bytecode,
    context::{
        result::{ExecutionResult, Output},
        CfgEnv, Context, TxEnv,
    },
    database::InMemoryDB,
    handler::{MainBuilder, MainContext},
    inspector::{InspectCommitEvm, Inspector},
    interpreter::{CallInputs, CallOutcome},
    primitives::{hardfork::SpecId, keccak256, Address, Bytes, TxKind, U256},
    state::AccountInfo,
};

fn parse_hex(s: &str) -> Vec<u8> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn cfg(c: &mut CfgEnv) {
    c.spec = SpecId::OSAKA;
    c.tx_gas_limit_cap = Some(u64::MAX);
    // Several Fe verifiers exceed the EIP-170 limit of 24576 bytes; deploy them anyway and report the size.
    c.limit_contract_code_size = Some(usize::MAX);
    c.limit_contract_initcode_size = Some(usize::MAX);
}

/// Records the gas spent by the outermost call frame.
#[derive(Default)]
struct TopFrame {
    depth: usize,
    spent: u64,
}

impl<CTX> Inspector<CTX> for TopFrame {
    fn call(&mut self, _c: &mut CTX, _i: &mut CallInputs) -> Option<CallOutcome> {
        self.depth += 1;
        None
    }
    fn call_end(&mut self, _c: &mut CTX, _i: &CallInputs, o: &mut CallOutcome) {
        self.depth -= 1;
        if self.depth == 0 {
            self.spent = o.result.gas.spent();
        }
    }
}

struct Run {
    name: String,
    ok: bool,
    exec: u64,
    tx: u64,
    calldata_bytes: usize,
    txs: u64,
    ret: Vec<u8>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut init: Option<Vec<u8>> = None;
    // (kind, name, spec)
    let mut calls: Vec<(char, String, String)> = vec![];
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).expect("missing value");
        match args[i].as_str() {
            "--init" => init = Some(parse_hex(&std::fs::read_to_string(v).expect("init file"))),
            "--call" | "--iterate" => {
                let (n, rest) = v.split_once('=').expect("NAME=...");
                calls.push((args[i].chars().nth(2).unwrap(), n.to_string(), rest.to_string()));
            }
            other => panic!("unknown arg {other}"),
        }
        i += 2;
    }
    let init = init.expect("--init");
    let caller = Address::ZERO;
    let mut db = InMemoryDB::default();
    db.insert_account_info(
        caller,
        AccountInfo::new(U256::from(1_000_000_000u64), 0, Default::default(), Bytecode::default()),
    );
    let mut evm = Context::mainnet()
        .with_db(db)
        .modify_cfg_chained(cfg)
        .build_mainnet_with_inspector(TopFrame::default());
    let tx = TxEnv::builder()
        .caller(caller)
        .gas_limit(1_000_000_000)
        .gas_price(0)
        .kind(TxKind::Create)
        .data(Bytes::from(init.clone()))
        .nonce(0)
        .build()
        .unwrap();
    let (address, deploy_gas) = match evm.inspect_tx_commit(tx).unwrap() {
        ExecutionResult::Success { output: Output::Create(_, Some(a)), gas_used, .. } => (a, gas_used),
        other => panic!("deploy failed: {other:?}"),
    };
    let code_size = {
        use revm::database_interface::Database;
        let db = &mut evm.ctx.journaled_state.database;
        let info = db.basic(address).unwrap().unwrap();
        let code = match info.code {
            Some(c) => c,
            None => db.code_by_hash(info.code_hash).unwrap(),
        };
        code.original_bytes().len()
    };

    let mut nonce = 1u64;
    let mut runs: Vec<Run> = vec![];
    for (kind, name, spec) in &calls {
        let mut inputs: Vec<Vec<u8>> = vec![];
        let mut feed: Option<(Vec<u8>, U256, u64)> = None;
        if *kind == 'c' {
            let text = std::fs::read_to_string(spec).expect("calldata file");
            // `sig:<signature>\n<hex body>`: the 4-byte selector of <signature> is prepended
            inputs.push(match text.strip_prefix("sig:") {
                Some(rest) => {
                    let (sig, body) = rest.split_once('\n').expect("sig line");
                    let mut d = keccak256(sig.trim().as_bytes())[..4].to_vec();
                    d.extend(parse_hex(body));
                    d
                }
                None => parse_hex(&text),
            });
        } else {
            let parts: Vec<&str> = spec.splitn(3, ':').collect();
            let sel = keccak256(parts[0].as_bytes());
            feed = Some((
                sel[..4].to_vec(),
                U256::from_be_slice(&parse_hex(parts[1])),
                parts[2].parse().unwrap(),
            ));
        }
        let n = feed.as_ref().map(|f| f.2).unwrap_or(1);
        let mut word = feed.as_ref().map(|f| f.1).unwrap_or(U256::ZERO);
        let (mut ok_all, mut exec, mut tx, mut cd) = (true, 0u64, 0u64, 0usize);
        let mut last = vec![];
        for _ in 0..n {
            let data = match &feed {
                Some((sel, _, _)) => {
                    let mut d = sel.clone();
                    d.extend_from_slice(&word.to_be_bytes::<32>());
                    d
                }
                None => inputs[0].clone(),
            };
            cd = data.len();
            evm.inspector.spent = 0;
            evm.inspector.depth = 0;
            let txenv = TxEnv::builder()
                .caller(caller)
                .gas_limit(1_000_000_000)
                .gas_price(0)
                .to(address)
                .data(Bytes::from(data))
                .nonce(nonce)
                .build()
                .unwrap();
            nonce += 1;
            let (ok, e, t, ret) = match evm.inspect_tx_commit(txenv).unwrap() {
                ExecutionResult::Success { output: Output::Call(ret), gas_used, .. } => {
                    (true, evm.inspector.spent, gas_used, ret.to_vec())
                }
                ExecutionResult::Revert { gas_used, output } => {
                    (false, evm.inspector.spent, gas_used, output.to_vec())
                }
                ExecutionResult::Halt { gas_used, .. } => (false, gas_used, gas_used, vec![]),
                other => panic!("call failed: {other:?}"),
            };
            ok_all &= ok;
            exec += e;
            tx += t;
            if feed.is_some() && ret.len() == 32 {
                word = U256::from_be_slice(&ret);
            }
            last = ret;
        }
        runs.push(Run { name: name.clone(), ok: ok_all, exec, tx, calldata_bytes: cd, txs: n, ret: last });
    }

    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let items: Vec<String> = runs
        .iter()
        .map(|r| {
            format!(
                "{{\"name\":\"{}\",\"ok\":{},\"exec\":{},\"tx\":{},\"calldata_bytes\":{},\"txs\":{},\"ret\":\"{}\"}}",
                r.name, r.ok, r.exec, r.tx, r.calldata_bytes, r.txs, hex(&r.ret)
            )
        })
        .collect();
    println!(
        "{{\"deployed_code_bytes\":{},\"initcode_bytes\":{},\"deploy_gas\":{},\"calls\":[{}]}}",
        code_size,
        init.len(),
        deploy_gas,
        items.join(",")
    );
}
