use std::collections::BTreeMap;

use crate::{KirInstruction, KirInstructionKind, KirPlace, KirTerminator, ValueId};

pub(super) fn replace_value_uses(function: &mut crate::KirFunction, old: ValueId, new: ValueId) {
    let values = BTreeMap::from([(old, new)]);
    remap_function_values(function, &values);
}

/// Compose the ordered substitutions before traversing the function. This is
/// deliberately sequential substitution, not a simultaneous phi assignment.
pub(super) fn replace_value_uses_batch(
    function: &mut crate::KirFunction,
    replacements: &[(ValueId, ValueId)],
) {
    if replacements.is_empty() {
        return;
    }
    let mut values = BTreeMap::new();
    for &(old, new) in replacements {
        for target in values.values_mut() {
            if *target == old {
                *target = new;
            }
        }
        values.entry(old).or_insert(new);
    }
    remap_function_values(function, &values);
}

fn remap_function_values(function: &mut crate::KirFunction, values: &BTreeMap<ValueId, ValueId>) {
    for region in &mut function.regions {
        match &mut region.origin {
            crate::KirMemoryRegionOrigin::Parameter(value)
            | crate::KirMemoryRegionOrigin::RawSlice(value)
            | crate::KirMemoryRegionOrigin::Subslice(value) => remap_value(value, values),
            crate::KirMemoryRegionOrigin::Conservative => {}
        }
        if let Some(interval) = &mut region.byte_interval {
            remap_value(&mut interval.start, values);
            remap_value(&mut interval.end, values);
        }
    }
    for block in &mut function.blocks {
        for instruction in &mut block.instructions {
            remap_instruction_values(instruction, values);
        }
        remap_terminator_values(&mut block.terminator, values);
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
        KirInstructionKind::VersionPredicate { predicate } => {
            for conjunct in &mut predicate.conjuncts {
                match conjunct {
                    crate::KirVersionPredicateConjunct::TripThreshold { value, .. } => {
                        remap_value(value, values);
                    }
                    crate::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left,
                        left_count,
                        right,
                        right_count,
                        ..
                    } => {
                        remap_value(left, values);
                        remap_value(left_count, values);
                        remap_value(right, values);
                        remap_value(right_count, values);
                    }
                }
            }
        }
        KirInstructionKind::VectorSplat { scalar, .. } => remap_value(scalar, values),
        KirInstructionKind::VectorLoad { access, .. } => {
            remap_value(&mut access.slice, values);
            remap_value(&mut access.start, values);
            remap_value(&mut access.end, values);
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            remap_value(&mut access.slice, values);
            remap_value(&mut access.start, values);
            remap_value(&mut access.end, values);
            remap_value(value, values);
        }
        KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => {
            remap_value(left, values);
            remap_value(right, values);
        }
        KirInstructionKind::VectorUnary { operand, .. } => remap_value(operand, values),
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => {
            remap_value(mask, values);
            remap_value(when_true, values);
            remap_value(when_false, values);
        }
        KirInstructionKind::VectorCast { value, .. } => remap_value(value, values),
        KirInstructionKind::VectorInsert { vector, scalar, .. } => {
            remap_value(vector, values);
            remap_value(scalar, values);
        }
        KirInstructionKind::VectorExtract { vector, .. }
        | KirInstructionKind::VectorReduce { vector, .. } => remap_value(vector, values),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KirBuildConfig, SourceFile, build_kir_module, check, lower_to_mir};

    #[test]
    fn batched_replacements_should_equal_ordered_single_rewrites() {
        let checked = check(&SourceFile::new(
            "substitutions.ck",
            "export fn kernel(items: slice<u32>, a: u32, b: u32) -> u32 { let tail: slice<u32> = items[a..b]; if a < b { tail[0] = a + b; return tail[0]; } return a; }",
        ));
        assert!(checked.diagnostics.is_empty());
        let module = build_kir_module(
            &lower_to_mir(&checked.checked_program).expect("MIR"),
            KirBuildConfig {
                consumer: crate::KirConsumer::Inspection,
                overflow_mode: crate::KirOverflowMode::Checked,
                bounds_mode: crate::KirBoundsMode::Checked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR");
        let original = &module.functions[0];
        let values = [
            original.params[0].value,
            original.params[1].value,
            original.params[2].value,
            ValueId::from_index(u32::MAX),
        ];
        let pairs = values
            .iter()
            .flat_map(|&old| values.iter().map(move |&new| (old, new)))
            .collect::<Vec<_>>();
        let mut unchanged = original.clone();
        replace_value_uses_batch(&mut unchanged, &[]);
        assert_eq!(&unchanged, original);
        // Include repeated sources, identities, chains, swaps, and sparse IDs.
        // Even malformed substitutions must retain the old rewrite semantics;
        // structural/proof validation remains a separate mandatory boundary.
        for &first in &pairs {
            for &second in &pairs {
                for &third in &pairs {
                    let replacements = [first, second, third];
                    let mut sequential = original.clone();
                    for &(old, new) in &replacements {
                        replace_value_uses(&mut sequential, old, new);
                    }
                    let mut batched = original.clone();
                    replace_value_uses_batch(&mut batched, &replacements);
                    assert_eq!(batched, sequential, "{replacements:?}");
                }
            }
        }
    }
}
