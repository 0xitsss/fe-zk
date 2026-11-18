//! zk path: loads a snarkjs Groth16 fixture (proof.json, public.json, boards.json) and builds the
//! calldata for `advanceProven` / `setCommitted` / `proveRun` of the Fe `Life` contract.
use crate::abi;
use revm::primitives::U256;
use serde_json::Value;
use std::path::Path;

pub struct Fixture {
    pub start: U256,
    pub end: U256,
    pub start_commit: U256,
    pub end_commit: U256,
    /// a (2 words), b (4 words, calldata order [[x_imag, x_real], [y_imag, y_real]]), c (2 words).
    pub proof: [U256; 8],
}

fn dec(v: &Value) -> Result<U256, String> {
    let s = v.as_str().ok_or("expected decimal string")?;
    U256::from_str_radix(s, 10).map_err(|e| format!("{s}: {e}"))
}

fn read(dir: &Path, name: &str) -> Result<Value, String> {
    let p = dir.join(name);
    let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn load(dir: &Path) -> Result<Fixture, String> {
    let proof = read(dir, "proof.json")?;
    let public = read(dir, "public.json")?;
    let boards = read(dir, "boards.json")?;
    let hex = |k: &str| -> Result<U256, String> {
        let s = boards[k].as_str().ok_or(format!("boards.json: {k}"))?;
        U256::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
    };
    let (a, b, c) = (&proof["pi_a"], &proof["pi_b"], &proof["pi_c"]);
    // snarkjs pi_b is [[x_real, x_imag], [y_real, y_imag]]; the EVM ABI wants imag first
    let p = [
        dec(&a[0])?,
        dec(&a[1])?,
        dec(&b[0][1])?,
        dec(&b[0][0])?,
        dec(&b[1][1])?,
        dec(&b[1][0])?,
        dec(&c[0])?,
        dec(&c[1])?,
    ];
    let f = Fixture {
        start: hex("start")?,
        end: hex("end")?,
        start_commit: dec(&public[0])?,
        end_commit: dec(&public[1])?,
        proof: p,
    };
    if f.start_commit != dec(&boards["startCommit"])? || f.end_commit != dec(&boards["endCommit"])? {
        return Err("public.json and boards.json disagree".into());
    }
    Ok(f)
}

pub fn halves(b: U256) -> (U256, U256) {
    let mask = (U256::from(1u8) << 128usize) - U256::from(1u8);
    (b & mask, b >> 128usize)
}

pub fn encode_set_committed(b: U256) -> Vec<u8> {
    let (lo, hi) = halves(b);
    abi::encode_words("setCommitted(uint256,uint256)", &[lo, hi])
}

pub fn encode_advance_proven(end: U256, proof: &[U256; 8]) -> Vec<u8> {
    let (lo, hi) = halves(end);
    let mut w = vec![lo, hi];
    w.extend_from_slice(proof);
    abi::encode_words("advanceProven(uint256,uint256,uint256[2],uint256[2][2],uint256[2])", &w)
}

pub fn encode_prove_run(f: &Fixture) -> Vec<u8> {
    let mut w = vec![f.start_commit, f.end_commit];
    w.extend_from_slice(&f.proof);
    abi::encode_words("proveRun(uint256,uint256,uint256[2],uint256[2][2],uint256[2])", &w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_split_at_128_bits() {
        let b = (U256::from(5u8) << 128usize) | U256::from(7u8);
        assert_eq!(halves(b), (U256::from(7u8), U256::from(5u8)));
    }

    #[test]
    fn advance_proven_layout() {
        let d = encode_advance_proven(U256::from(3u8), &[U256::from(9u8); 8]);
        assert_eq!(d.len(), 4 + 32 * 10);
        assert_eq!(
            &d[..4],
            &abi::selector("advanceProven(uint256,uint256,uint256[2],uint256[2][2],uint256[2])")
        );
        assert_eq!(d[35], 3); // end_lo
        assert!(d[36..68].iter().all(|b| *b == 0)); // end_hi
    }
}
