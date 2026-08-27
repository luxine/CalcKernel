use crate::MirType;

use super::{
    NativeAbiClassifier, NativeAbiError, NativeAbiPassMode, NativeAbiPosition, NativeAbiRegister,
};

pub(super) fn classify(
    classifier: &NativeAbiClassifier,
    type_node: &MirType,
    _position: NativeAbiPosition,
) -> Result<NativeAbiPassMode, NativeAbiError> {
    let layout = classifier.layout(type_node)?;
    if let Some(members @ 1..=4) = classifier.homogeneous_f64_members(type_node)? {
        return Ok(NativeAbiPassMode::Direct {
            registers: vec![NativeAbiRegister::floating(64); members as usize],
        });
    }
    if layout.size <= 16 {
        return Ok(NativeAbiPassMode::Direct {
            registers: vec![NativeAbiRegister::integer(64); layout.size.div_ceil(8) as usize],
        });
    }
    Ok(NativeAbiPassMode::Indirect {
        by_value: false,
        alignment: layout.alignment,
    })
}
