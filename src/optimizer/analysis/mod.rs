mod affine;
mod alias;
mod budget;
mod congruence;
mod effects;
mod known_bits;
mod memory_ssa;
mod regions;
mod scalar;

pub use affine::*;
pub use alias::*;
pub use budget::*;
pub use congruence::*;
pub use effects::*;
pub use known_bits::*;
pub use memory_ssa::*;
pub use regions::*;
pub use scalar::*;

use crate::{MirInstruction, MirPlace, MirPrimitiveTypeName, MirType, MirValue};

pub(in crate::optimizer) fn instruction_target(instruction: &MirInstruction) -> Option<&MirValue> {
    match instruction {
        MirInstruction::ConstInt { target, .. }
        | MirInstruction::ConstFloat { target, .. }
        | MirInstruction::ConstBool { target, .. }
        | MirInstruction::Move { target, .. }
        | MirInstruction::Binary { target, .. }
        | MirInstruction::Unary { target, .. }
        | MirInstruction::Compare { target, .. }
        | MirInstruction::Cast { target, .. }
        | MirInstruction::Address { target, .. }
        | MirInstruction::Load { target, .. }
        | MirInstruction::MakeSlice { target, .. }
        | MirInstruction::SliceData { target, .. }
        | MirInstruction::SliceLen { target, .. }
        | MirInstruction::Subslice { target, .. } => Some(target),
        MirInstruction::Call { target, .. } => target.as_ref(),
        MirInstruction::Store { .. } | MirInstruction::RuntimeCall { .. } => None,
    }
}

pub(in crate::optimizer) fn value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}

pub(in crate::optimizer) fn place_type(place: &MirPlace) -> &MirType {
    match place {
        MirPlace::Param { type_node, .. }
        | MirPlace::Local { type_node, .. }
        | MirPlace::Deref { type_node, .. }
        | MirPlace::Index { type_node, .. }
        | MirPlace::SliceIndex { type_node, .. }
        | MirPlace::Field { type_node, .. } => type_node,
    }
}

pub(in crate::optimizer) fn temp_name(value: &MirValue) -> Option<&str> {
    match value {
        MirValue::Temp { name, .. } => Some(name),
        MirValue::Param { .. }
        | MirValue::Local { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::ConstBool { .. } => None,
    }
}

pub(in crate::optimizer) fn is_integer_type(type_node: &MirType) -> bool {
    IntegerType::from_mir(type_node).is_some()
}

pub(in crate::optimizer) fn is_f64_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
}

pub(in crate::optimizer) fn contains_slice_type(type_node: &MirType) -> bool {
    match type_node {
        MirType::Slice(_) => true,
        MirType::Pointer(element_type) => contains_slice_type(element_type),
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => false,
    }
}

pub(in crate::optimizer) fn is_signed_integer_type(type_node: &MirType) -> bool {
    IntegerType::from_mir(type_node).is_some_and(IntegerType::is_signed)
}

pub(in crate::optimizer) fn integer_min(type_node: &MirType) -> Option<i128> {
    IntegerType::from_mir(type_node).map(IntegerType::minimum_i128)
}

pub(in crate::optimizer) fn fits_integer_type(value: i128, type_node: &MirType) -> bool {
    IntegerType::from_mir(type_node).is_some_and(|integer| integer.contains_i128(value))
}
