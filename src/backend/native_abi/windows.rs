use crate::MirType;

use super::{
    NativeAbiClassifier, NativeAbiError, NativeAbiPassMode, NativeAbiPosition, NativeAbiRegister,
};

pub(super) fn classify_x64(
    classifier: &NativeAbiClassifier,
    type_node: &MirType,
    _position: NativeAbiPosition,
) -> Result<NativeAbiPassMode, NativeAbiError> {
    let layout = classifier.layout(type_node)?;
    if matches!(layout.size, 1 | 2 | 4 | 8) {
        return Ok(NativeAbiPassMode::Direct {
            registers: vec![NativeAbiRegister::integer((layout.size * 8) as u16)],
        });
    }
    Ok(NativeAbiPassMode::Indirect {
        by_value: false,
        alignment: layout.alignment,
    })
}

pub(super) fn classify_arm64(
    classifier: &NativeAbiClassifier,
    type_node: &MirType,
    position: NativeAbiPosition,
) -> Result<NativeAbiPassMode, NativeAbiError> {
    super::aapcs64::classify(classifier, type_node, position)
}
