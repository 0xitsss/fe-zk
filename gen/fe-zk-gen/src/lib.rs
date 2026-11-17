//! fe-zk-gen: parse, validate and fingerprint zk verifying keys; (later) emit Fe verifiers.
//!
//! Pipeline: `parse_*` (untrusted bytes -> raw `Groth16Vk`, no trust) ->
//! `Groth16Vk::validate` (curve/subgroup/canonical checks done with ark-bn254, an
//! independent library) -> `Groth16Vk::digest` -> `emit_fe`.

pub mod digest;
pub mod emit;
pub mod model;
pub mod parse;
pub mod validate;

pub use digest::{key_digest, DIGEST_DOMAIN};
pub use emit::emit_fe;
pub use model::{Groth16Vk, VkError, G1, G2};
pub use parse::{parse_arkworks, parse_snarkjs};

/// Tool version embedded in generated file headers.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
