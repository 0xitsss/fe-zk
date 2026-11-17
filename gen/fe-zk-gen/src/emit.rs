use crate::model::{Groth16Vk, VkError};

/// Emit a self-contained Fe verifier module for `vk`.
///
/// TODO(emission task): validate `vk`, embed the key digest and TOOL_VERSION in a header,
/// emit constants in EVM G2 order (`G2::evm_words`), all section-3 safety checks by
/// construction, plus a Fe test with a valid proof and a battery of invalid ones.
pub fn emit_fe(vk: &Groth16Vk) -> Result<String, VkError> {
    let _ = vk;
    Err(VkError::NotImplemented)
}
