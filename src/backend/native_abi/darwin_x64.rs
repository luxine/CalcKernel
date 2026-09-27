use crate::MirType;

use super::{NativeAbiClassifier, NativeAbiError, NativeAbiPassMode, NativeAbiPosition};

pub(super) fn classify(
    classifier: &NativeAbiClassifier,
    type_node: &MirType,
    position: NativeAbiPosition,
) -> Result<NativeAbiPassMode, NativeAbiError> {
    super::sysv_x64::classify(classifier, type_node, position)
}
