use std::collections::BTreeMap;

use crate::{KirInstruction, KirInstructionKind, KirPlace, KirTerminator, ValueId};

pub(super) fn replace_value_uses(function: &mut crate::KirFunction, old: ValueId, new: ValueId) {
    let values = BTreeMap::from([(old, new)]);
    for region in &mut function.regions {
        match &mut region.origin {
            crate::KirMemoryRegionOrigin::Parameter(value)
            | crate::KirMemoryRegionOrigin::RawSlice(value)
            | crate::KirMemoryRegionOrigin::Subslice(value) => remap_value(value, &values),
            crate::KirMemoryRegionOrigin::Conservative => {}
        }
        if let Some(interval) = &mut region.byte_interval {
            remap_value(&mut interval.start, &values);
            remap_value(&mut interval.end, &values);
        }
    }
    for block in &mut function.blocks {
        for instruction in &mut block.instructions {
            remap_instruction_values(instruction, &values);
        }
        remap_terminator_values(&mut block.terminator, &values);
    }
}

pub(super) fn remap_instruction_values(
    instruction: &mut KirInstruction,
    values: &BTreeMap<ValueId, ValueId>,
) {
    match &mut instruction.kind {
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => {}
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => {
            remap_value(value, values);
        }
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => {
            remap_value(left, values);
            remap_value(right, values);
        }
        KirInstructionKind::Unary { operand, .. } => remap_value(operand, values),
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => {
            for value in args {
                remap_value(value, values);
            }
        }
        KirInstructionKind::Guard { condition, .. } => remap_value(condition, values),
        KirInstructionKind::Address { place } | KirInstructionKind::Load { place } => {
            remap_place(place, values);
        }
        KirInstructionKind::Store { place, value } => {
            remap_place(place, values);
            remap_value(value, values);
        }
        KirInstructionKind::MakeSlice { data, len } => {
            remap_value(data, values);
            remap_value(len, values);
        }
        KirInstructionKind::SliceData { slice } | KirInstructionKind::SliceLen { slice } => {
            remap_value(slice, values);
        }
        KirInstructionKind::Subslice { slice, start, end } => {
            remap_value(slice, values);
            remap_value(start, values);
            remap_value(end, values);
        }
    }
}

pub(super) fn remap_terminator_values(
    terminator: &mut KirTerminator,
    values: &BTreeMap<ValueId, ValueId>,
) {
    match terminator {
        KirTerminator::Return { value, .. } => {
            if let Some(value) = value {
                remap_value(value, values);
            }
        }
        KirTerminator::Jump { edge } => {
            for value in &mut edge.args {
                remap_value(value, values);
            }
        }
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            remap_value(condition, values);
            for value in then_edge.args.iter_mut().chain(&mut else_edge.args) {
                remap_value(value, values);
            }
        }
    }
}

fn remap_place(place: &mut KirPlace, values: &BTreeMap<ValueId, ValueId>) {
    match place {
        KirPlace::Value { value, .. } => remap_value(value, values),
        KirPlace::Deref { pointer, .. } => remap_value(pointer, values),
        KirPlace::Index { base, index, .. } => {
            remap_place(base, values);
            remap_value(index, values);
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            remap_value(slice, values);
            remap_value(index, values);
        }
        KirPlace::Field { base, .. } => remap_place(base, values),
    }
}

fn remap_value(value: &mut ValueId, values: &BTreeMap<ValueId, ValueId>) {
    if let Some(replacement) = values.get(value) {
        *value = *replacement;
    }
}
