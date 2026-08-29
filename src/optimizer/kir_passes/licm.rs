use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, InstructionId, KirArithmeticSemantics, KirInstruction, KirInstructionKind, KirModule,
    KirPlace, KirTerminator, ValueId,
};

use super::super::NaturalLoopAnalysis;

pub(crate) fn run_licm(
    module: &mut KirModule,
    protected: &BTreeSet<InstructionId>,
    analyses: &[NaturalLoopAnalysis],
) -> u32 {
    let mut hoisted = 0_u32;
    for (function, analysis) in module.functions.iter_mut().zip(analyses) {
        let definitions = value_definitions(function);
        for loop_info in analysis.loops.iter().rev() {
            let loop_blocks = loop_info.blocks.iter().copied().collect::<BTreeSet<_>>();
            let preheaders = function
                .blocks
                .iter()
                .filter(|block| !loop_blocks.contains(&block.id))
                .filter(|block| successor_ids(&block.terminator).contains(&loop_info.header))
                .map(|block| block.id)
                .collect::<Vec<_>>();
            let [preheader] = preheaders.as_slice() else {
                continue;
            };
            let mut invariant_values = BTreeSet::new();
            let mut moved = Vec::<(BlockId, KirInstruction)>::new();
            loop {
                let before = moved.len();
                for block in &function.blocks {
                    if !loop_blocks.contains(&block.id) || block.id == loop_info.header {
                        continue;
                    }
                    for instruction in &block.instructions {
                        if moved
                            .iter()
                            .any(|(_, candidate)| candidate.id == instruction.id)
                            || protected.contains(&instruction.id)
                            || !is_licm_pure(instruction)
                        {
                            continue;
                        }
                        if instruction_uses(instruction).iter().all(|value| {
                            invariant_values.contains(value)
                                || definitions.get(value).is_none_or(|block| {
                                    block.is_none_or(|block| !loop_blocks.contains(&block))
                                })
                        }) {
                            invariant_values
                                .extend(instruction.results.iter().map(|result| result.value));
                            moved.push((block.id, instruction.clone()));
                        }
                    }
                }
                if moved.len() == before {
                    break;
                }
            }
            if moved.is_empty() {
                continue;
            }
            let moved_ids = moved
                .iter()
                .map(|(_, instruction)| instruction.id)
                .collect::<BTreeSet<_>>();
            for block in &mut function.blocks {
                block
                    .instructions
                    .retain(|instruction| !moved_ids.contains(&instruction.id));
            }
            if let Some(block) = function
                .blocks
                .iter_mut()
                .find(|block| block.id == *preheader)
            {
                moved.sort_by_key(|(source, instruction)| (*source, instruction.id));
                block
                    .instructions
                    .extend(moved.into_iter().map(|(_, instruction)| instruction));
                hoisted =
                    hoisted.saturating_add(u32::try_from(moved_ids.len()).unwrap_or(u32::MAX));
            }
        }
    }
    hoisted
}

fn is_licm_pure(instruction: &KirInstruction) -> bool {
    if instruction.effect.is_some()
        || instruction.memory.is_some()
        || instruction.results.is_empty()
    {
        return false;
    }
    match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. } => true,
        KirInstructionKind::Binary { semantics, .. }
        | KirInstructionKind::Unary { semantics, .. } => {
            *semantics == KirArithmeticSemantics::Modular
        }
        _ => false,
    }
}

fn value_definitions(function: &crate::KirFunction) -> BTreeMap<ValueId, Option<BlockId>> {
    function
        .params
        .iter()
        .map(|param| (param.value, None))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, Some(block.id)))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, Some(block.id)))
                }))
        }))
        .collect()
}

fn successor_ids(terminator: &KirTerminator) -> Vec<BlockId> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge.target],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge.target, else_edge.target],
    }
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
