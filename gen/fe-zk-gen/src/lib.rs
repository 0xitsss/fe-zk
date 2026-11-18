//! fe-zk-gen: parse, validate and fingerprint zk verifying keys; (later) emit Fe verifiers.
//!
//! Pipeline: `parse_*` (untrusted bytes -> raw `Groth16Vk`, no trust) ->
//! `Groth16Vk::validate` (curve/subgroup/canonical checks done with ark-bn254, an
//! independent library) -> `Groth16Vk::digest` -> `emit_fe`.

pub mod digest;
pub mod emit;
pub mod halo2;
pub mod model;
pub mod parse;
pub mod plonk;
pub mod validate;

pub use digest::{key_digest, DIGEST_DOMAIN};
pub use emit::{emit_fe, emit_ingot, EmitOptions, EmittedIngot, TestFixture};
pub use halo2::{
    emit_halo2_ingot, emit_halo2_module, halo2_key_digest, parse_halo2_proof, parse_halo2_vk,
    Halo2EmitOptions, Halo2Fixture, Halo2Layout, Halo2Vk,
};
pub use model::{Groth16Proof, Groth16Vk, VkError, G1, G2};
pub use parse::{
    parse_arkworks, parse_arkworks_proof, parse_public, parse_snarkjs, parse_snarkjs_proof,
};
pub use plonk::{
    emit_plonk_fe, emit_plonk_ingot, parse_plonk_snarkjs, parse_plonk_snarkjs_proof,
    plonk_key_digest, PlonkEmitOptions, PlonkFixture, PlonkProof, PlonkVk,
};

/// Tool version embedded in generated file headers.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
