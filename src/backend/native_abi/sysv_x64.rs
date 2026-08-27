use crate::MirType;

use super::{
    AggregateChunk, NativeAbiClassifier, NativeAbiPassMode, NativeAbiPosition, NativeAbiRegister,
    NativeAbiRegisterClass,
};

pub(super) fn classify(
    classifier: &NativeAbiClassifier,
    type_node: &MirType,
    position: NativeAbiPosition,
) -> Result<NativeAbiPassMode, super::NativeAbiError> {
    let layout = classifier.layout(type_node)?;
    if layout.size > 16 {
        return Ok(NativeAbiPassMode::Indirect {
            by_value: position == NativeAbiPosition::Parameter,
            alignment: layout.alignment,
        });
    }
    let mut chunks = vec![AggregateChunk::default(); layout.size.div_ceil(8) as usize];
    classifier.classify_sysv_chunks(type_node, 0, &mut chunks)?;
    Ok(NativeAbiPassMode::Direct {
        registers: chunks
            .into_iter()
            .map(|chunk| NativeAbiRegister {
                class: chunk.class.unwrap_or(NativeAbiRegisterClass::Integer),
                bits: normalized_integer_width(chunk.used_bits),
            })
            .collect(),
    })
}

fn normalized_integer_width(used_bits: u16) -> u16 {
    match used_bits {
        0..=8 => 8,
        9..=16 => 16,
        17..=32 => 32,
        _ => 64,
    }
}
