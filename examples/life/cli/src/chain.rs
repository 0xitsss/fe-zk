//! Deploys `Life.bin` into an in-memory revm and calls it. Mirrors the setup used by Fe's own
//! test harness (Osaka rules, since the Fe backend emits Osaka opcodes).
use crate::abi;
use revm::{
    bytecode::Bytecode,
    context::{
        result::{ExecutionResult, Output},
        CfgEnv, Context, TxEnv,
    },
    database::InMemoryDB,
    handler::{ExecuteCommitEvm, MainBuilder, MainContext, MainnetContext, MainnetEvm},
    primitives::{hardfork::SpecId, Address, Bytes, TxKind, U256},
    state::AccountInfo,
};

pub struct Chain {
    evm: MainnetEvm<MainnetContext<InMemoryDB>>,
    caller: Address,
    nonce: u64,
    pub address: Address,
}

pub struct CallOut {
    pub ret: Vec<u8>,
    /// Gas charged for the whole transaction (includes the 21000 intrinsic + calldata cost).
    pub tx_gas: u64,
    /// `tx_gas` minus intrinsic gas: what the contract execution itself used.
    pub exec_gas: u64,
}

fn intrinsic(calldata: &[u8]) -> u64 {
    21_000
        + calldata
            .iter()
            .map(|b| if *b == 0 { 4 } else { 16 })
            .sum::<u64>()
}

fn cfg(c: &mut CfgEnv) {
    c.spec = SpecId::OSAKA;
    c.tx_gas_limit_cap = Some(u64::MAX);
}

pub fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() % 2 != 0 {
        return Err("odd hex length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

impl Chain {
    pub fn deploy(init_hex: &str) -> Result<Chain, String> {
        let caller = Address::ZERO;
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            caller,
            AccountInfo::new(
                U256::from(1_000_000_000u64),
                0,
                Default::default(),
                Bytecode::default(),
            ),
        );
        let mut evm = Context::mainnet()
            .with_db(db)
            .modify_cfg_chained(cfg)
            .build_mainnet();
        let tx = TxEnv::builder()
            .caller(caller)
            .gas_limit(1_000_000_000)
            .gas_price(0)
            .kind(TxKind::Create)
            .data(Bytes::from(parse_hex(init_hex)?))
            .nonce(0)
            .build()
            .map_err(|e| format!("{e:?}"))?;
        match evm.transact_commit(tx).map_err(|e| e.to_string())? {
            ExecutionResult::Success {
                output: Output::Create(_, Some(address)),
                ..
            } => Ok(Chain {
                evm,
                caller,
                nonce: 1,
                address,
            }),
            other => Err(format!("deploy failed: {other:?}")),
        }
    }

    pub fn call(&mut self, calldata: Vec<u8>) -> Result<CallOut, String> {
        let intr = intrinsic(&calldata);
        let tx = TxEnv::builder()
            .caller(self.caller)
            .gas_limit(1_000_000_000)
            .gas_price(0)
            .to(self.address)
            .data(Bytes::from(calldata))
            .nonce(self.nonce)
            .build()
            .map_err(|e| format!("{e:?}"))?;
        self.nonce += 1;
        match self.evm.transact_commit(tx).map_err(|e| e.to_string())? {
            ExecutionResult::Success {
                output: Output::Call(ret),
                gas_used,
                ..
            } => Ok(CallOut {
                ret: ret.to_vec(),
                tx_gas: gas_used,
                exec_gas: gas_used.saturating_sub(intr),
            }),
            other => Err(format!("call failed: {other:?}")),
        }
    }

    pub fn step(&mut self, board: U256) -> Result<(U256, CallOut), String> {
        let out = self.call(abi::encode_step(board))?;
        Ok((abi::decode_word(&out.ret)?, out))
    }

    pub fn run(&mut self, board: U256, n: u64) -> Result<(U256, CallOut), String> {
        let out = self.call(abi::encode_run(board, n))?;
        Ok((abi::decode_word(&out.ret)?, out))
    }
}
