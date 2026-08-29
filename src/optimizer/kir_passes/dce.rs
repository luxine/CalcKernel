use std::collections::BTreeSet;

use crate::{
    InstructionId, KirInstruction, KirInstructionKind, KirModule, KirPlace, KirTerminator, ValueId,
};

pub(crate) fn run_dead_code_elimination(
    module: &mut KirModule,
    protected: &BTreeSet<InstructionId>,
) -> bool {
    let mut changed = false;
    loop {
        let used = collect_used_values(module);
        let mut removed = false;
        for function in &mut module.functions {
            for block in &mut function.blocks {
                let before = block.instructions.len();
                block
                    .instructions
                    .retain(|instruction| !is_dead_pure_instruction(instruction, &used, protected));
                removed |= block.instructions.len() != before;
            }
        }
        changed |= removed;
        if !removed {
            break;
        }
    }
    changed
}

fn is_dead_pure_instruction(
    instruction: &KirInstruction,
    used: &BTreeSet<ValueId>,
    protected: &BTreeSet<InstructionId>,
) -> bool {
    !protected.contains(&instruction.id)
        && instruction.effect.is_none()
        && instruction.memory.is_none()
        && is_pure(&instruction.kind)
        && !instruction.results.is_empty()
        && instruction
            .results
            .iter()
            .all(|result| !used.contains(&result.value))
}

fn is_pure(kind: &KirInstructionKind) -> bool {
    !matches!(
        kind,
        KirInstructionKind::Guard { .. }
            | KirInstructionKind::Load { .. }
            | KirInstructionKind::Store { .. }
            | KirInstructionKind::Call { .. }
            | KirInstructionKind::RuntimeCall { .. }
    )
}

fn collect_used_values(module: &KirModule) -> BTreeSet<ValueId> {
    let mut used = BTreeSet::new();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                used.extend(instruction_uses(instruction));
            }
            match &block.terminator {
                KirTerminator::Return { value, .. } => used.extend(value),
                KirTerminator::Jump { edge } => used.extend(&edge.args),
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => {
                    used.insert(*condition);
                    used.extend(&then_edge.args);
                    used.extend(&else_edge.args);
                }
            }
        }
    }
    used
}

fn instruction_uses(instruction: &KirInstruction) -> Vec<ValueId> {
    match &instruction.kind {
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => Vec::new(),
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => vec![*value],
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => vec![*left, *right],
        KirInstructionKind::Unary { operand, .. } => vec![*operand],
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.clone(),
        KirInstructionKind::Guard { condition, .. } => vec![*condition],
        KirInstructionKind::Address { place } | KirInstructionKind::Load { place } => {
            place_uses(place)
        }
        KirInstructionKind::Store { place, value } => {
            let mut values = place_uses(place);
            values.push(*value);
            values
        }
        KirInstructionKind::MakeSlice { data, len } => vec![*data, *len],
        KirInstructionKind::SliceData { slice } | KirInstructionKind::SliceLen { slice } => {
            vec![*slice]
        }
        KirInstructionKind::Subslice { slice, start, end } => vec![*slice, *start, *end],
    }
}

fn place_uses(place: &KirPlace) -> Vec<ValueId> {
    match place {
        KirPlace::Value { value, .. } => vec![*value],
        KirPlace::Deref { pointer, .. } => vec![*pointer],
        KirPlace::Index { base, index, .. } => {
            let mut values = place_uses(base);
            values.push(*index);
            values
        }
        KirPlace::SliceIndex { slice, index, .. } => vec![*slice, *index],
        KirPlace::Field { base, .. } => place_uses(base),
    }
}
