//! Minimal Solidity ABI encoding for the `Life` contract messages.
use revm::primitives::{keccak256, U256};

pub fn selector(signature: &str) -> [u8; 4] {
    let h = keccak256(signature.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn word(v: U256) -> [u8; 32] {
    v.to_be_bytes::<32>()
}

/// `step(uint256)` calldata.
pub fn encode_step(board: U256) -> Vec<u8> {
    let mut d = selector("step(uint256)").to_vec();
    d.extend_from_slice(&word(board));
    d
}

/// `run(uint256,uint256)` calldata.
pub fn encode_run(board: U256, n: u64) -> Vec<u8> {
    let mut d = selector("run(uint256,uint256)").to_vec();
    d.extend_from_slice(&word(board));
    d.extend_from_slice(&word(U256::from(n)));
    d
}

/// Decode a single `uint256` return value.
pub fn decode_word(data: &[u8]) -> Result<U256, String> {
    if data.len() != 32 {
        return Err(format!("expected 32 return bytes, got {}", data.len()));
    }
    Ok(U256::from_be_slice(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_match_known_values() {
        // keccak256("transfer(address,uint256)")[..4] = a9059cbb (well-known)
        assert_eq!(selector("transfer(address,uint256)"), [0xa9, 0x05, 0x9c, 0xbb]);
    }

    #[test]
    fn step_calldata_layout() {
        let d = encode_step(U256::from(0x0102u64));
        assert_eq!(d.len(), 36);
        assert_eq!(&d[..4], &selector("step(uint256)"));
        assert_eq!(d[34], 0x01);
        assert_eq!(d[35], 0x02);
        assert!(d[4..34].iter().all(|b| *b == 0));
    }

    #[test]
    fn run_calldata_layout() {
        let d = encode_run(U256::MAX, 64);
        assert_eq!(d.len(), 68);
        assert_eq!(&d[..4], &selector("run(uint256,uint256)"));
        assert!(d[4..36].iter().all(|b| *b == 0xff));
        assert_eq!(d[67], 64);
        assert!(d[36..67].iter().all(|b| *b == 0));
    }

    #[test]
    fn decode_rejects_bad_length() {
        assert!(decode_word(&[0u8; 31]).is_err());
        assert_eq!(decode_word(&U256::from(7u64).to_be_bytes::<32>()).unwrap(), U256::from(7u64));
    }
}
