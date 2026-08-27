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
    matches!(
        type_node,
        MirType::Primitive(
            MirPrimitiveTypeName::I32
                | MirPrimitiveTypeName::I64
                | MirPrimitiveTypeName::U32
                | MirPrimitiveTypeName::U64
        )
    )
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
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)
    )
}

pub(in crate::optimizer) fn integer_min(type_node: &MirType) -> Option<i128> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(-(1_i128 << 31)),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(-(1_i128 << 63)),
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64) => Some(0),
        MirType::Primitive(MirPrimitiveTypeName::F64 | MirPrimitiveTypeName::Bool)
        | MirType::Pointer(_)
        | MirType::Slice(_)
        | MirType::Struct(_)
        | MirType::Void => None,
    }
}

pub(in crate::optimizer) fn integer_max(type_node: &MirType) -> Option<i128> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some((1_i128 << 31) - 1),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some((1_i128 << 63) - 1),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some((1_i128 << 32) - 1),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some((1_i128 << 64) - 1),
        MirType::Primitive(MirPrimitiveTypeName::F64 | MirPrimitiveTypeName::Bool)
        | MirType::Pointer(_)
        | MirType::Slice(_)
        | MirType::Struct(_)
        | MirType::Void => None,
    }
}

pub(in crate::optimizer) fn fits_integer_type(value: i128, type_node: &MirType) -> bool {
    is_integer_type(type_node)
        && integer_min(type_node).is_some_and(|min| value >= min)
        && integer_max(type_node).is_some_and(|max| value <= max)
}
