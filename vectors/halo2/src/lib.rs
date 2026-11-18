//! fe-zk halo2-KZG vectors: circuit, fixture export, and an independent mini verifier.
//!
//! Flavor: halo2-axiom 0.5.3 (via halo2-base 0.5.5) + snark-verifier 0.2.7 (axiom) EVM transcript.
pub mod circuit;
pub mod export;
pub mod mini_verifier;
pub mod ptau;
