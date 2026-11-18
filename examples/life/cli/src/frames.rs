//! `--frames-json`: dump what the Fe contract returned, for `scripts/make-life-svg.py`.
//! Boards are the u256 returned by `step()` (hex, bit index = row * 16 + col); gas is execution
//! gas of the call as measured in revm.
use revm::primitives::U256;

pub struct Frame {
    pub generation: u64,
    pub board: U256,
    pub step_gas: Option<u64>,
}

pub fn play_json(pattern: &str, gens: u64, frames: &[Frame], run_gas: u64) -> String {
    let fr: Vec<String> = frames
        .iter()
        .map(|f| {
            let gas = f.step_gas.map_or("null".to_string(), |g| g.to_string());
            format!("{{\"generation\":{},\"board\":\"{:#x}\",\"step_gas\":{}}}", f.generation, f.board, gas)
        })
        .collect();
    format!(
        "{{\"mode\":\"play\",\"pattern\":\"{pattern}\",\"generations\":{gens},\"run_gas\":{run_gas},\"frames\":[{}]}}\n",
        fr.join(",")
    )
}

pub fn prove_json(n: u64, start: U256, end: U256, steps_gas: u64, run_gas: u64, verify_gas: u64, advance_gas: u64) -> String {
    format!(
        "{{\"mode\":\"prove\",\"generations\":{n},\"start_board\":\"{start:#x}\",\"end_board\":\"{end:#x}\",\
\"steps_gas\":{steps_gas},\"run_gas\":{run_gas},\"verify_gas\":{verify_gas},\"advance_proven_gas\":{advance_gas}}}\n"
    )
}

pub fn write_play(path: &str, pattern: &str, gens: u64, frames: &[Frame], run_gas: u64) -> Result<(), String> {
    std::fs::write(path, play_json(pattern, gens, frames, run_gas)).map_err(|e| format!("{path}: {e}"))
}

pub fn write_prove(path: &str, n: u64, start: U256, end: U256, steps_gas: u64, run_gas: u64, verify_gas: u64, advance_gas: u64) -> Result<(), String> {
    std::fs::write(path, prove_json(n, start, end, steps_gas, run_gas, verify_gas, advance_gas)).map_err(|e| format!("{path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_json_parses_and_keeps_boards() {
        let frames = vec![
            Frame { generation: 0, board: U256::from(5u8), step_gas: None },
            Frame { generation: 1, board: U256::from(0x1234u32) << 200usize, step_gas: Some(657) },
        ];
        let v: serde_json::Value = serde_json::from_str(&play_json("glider", 1, &frames, 900)).unwrap();
        assert_eq!(v["mode"], "play");
        assert_eq!(v["frames"].as_array().unwrap().len(), 2);
        assert!(v["frames"][0]["step_gas"].is_null());
        assert_eq!(v["frames"][1]["step_gas"], 657);
        let b = v["frames"][1]["board"].as_str().unwrap();
        assert_eq!(U256::from_str_radix(&b[2..], 16).unwrap(), frames[1].board);
        assert_eq!(v["run_gas"], 900);
    }

    #[test]
    fn prove_json_parses() {
        let v: serde_json::Value =
            serde_json::from_str(&prove_json(512, U256::from(1u8), U256::from(2u8), 10, 20, 30, 40)).unwrap();
        assert_eq!(v["generations"], 512);
        assert_eq!(v["verify_gas"], 30);
        assert_eq!(v["end_board"], "0x2");
    }
}
