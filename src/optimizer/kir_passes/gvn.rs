use std::collections::BTreeMap;

use crate::{
    BlockId, KirArithmeticSemantics, KirInstructionKind, KirModule, KirTerminator, ValueId,
    compute_kir_dominators,
};

use super::rewrite::{remap_instruction_values, replace_value_uses};

pub(crate) fn run_gvn(module: &mut KirModule) -> u32 {
    let mut rewrites = 0_u32;
    for function in &mut module.functions {
        let dominators = compute_kir_dominators(function);
        let canonical_values = canonical_block_params(function);
        let globally_pure = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .all(|instruction| instruction.effect.is_none() && instruction.memory.is_none());
        let mut replacements = Vec::new();
        let mut removed = Vec::new();
        let mut global = BTreeMap::<String, Vec<(BlockId, ValueId)>>::new();
        let mut global_constants = BTreeMap::<String, Vec<(BlockId, ValueId)>>::new();
        for block in &function.blocks {
            let mut available = BTreeMap::<String, ValueId>::new();
            let mut constants = BTreeMap::<String, ValueId>::new();
            for instruction in &block.instructions {
                if instruction.effect.is_some() || instruction.memory.is_some() {
                    available.clear();
                    continue;
                }
                let Some(result) = instruction.results.first() else {
                    continue;
                };
                if instruction.results.len() != 1 || !is_gvn_expression(&instruction.kind) {
                    continue;
                }
                let mut canonical = instruction.clone();
                remap_instruction_values(&mut canonical, &canonical_values);
                let key = format!("{:?}:{:?}", result.type_node, canonical.kind);
                let table = if is_constant(&instruction.kind) {
                    &mut constants
                } else {
                    &mut available
                };
                let global_table = if is_constant(&instruction.kind) {
                    &mut global_constants
                } else {
                    &mut global
                };
                let existing = table.get(&key).copied().or_else(|| {
                    (globally_pure || is_constant(&instruction.kind)).then(|| {
                        global_table.get(&key).and_then(|definitions| {
                            definitions.iter().find_map(|(definition_block, value)| {
                                dominators
                                    .dominates(*definition_block, block.id)
                                    .then_some(*value)
                            })
                        })
                    })?
                });
                if let Some(existing) = existing {
                    replacements.push((result.value, existing));
                    removed.push(instruction.id);
                    rewrites = rewrites.saturating_add(1);
                } else {
                    table.insert(key.clone(), result.value);
                    global_table
                        .entry(key)
                        .or_default()
                        .push((block.id, result.value));
                }
            }
        }
        for (old, new) in replacements {
            replace_value_uses(function, old, new);
        }
        for block in &mut function.blocks {
            block
                .instructions
                .retain(|instruction| !removed.contains(&instruction.id));
        }
    }
    rewrites
}

fn canonical_block_params(function: &crate::KirFunction) -> BTreeMap<ValueId, ValueId> {
    let mut canonical = BTreeMap::new();
    loop {
        let before = canonical.len();
        for block in &function.blocks {
            for (index, param) in block.params.iter().enumerate() {
                let values = function
                    .blocks
                    .iter()
                    .flat_map(|predecessor| match &predecessor.terminator {
                        KirTerminator::Return { .. } => Vec::new(),
                        KirTerminator::Jump { edge } => vec![edge],
                        KirTerminator::Branch {
                            then_edge,
                            else_edge,
                            ..
                        } => vec![then_edge, else_edge],
                    })
                    .filter(|edge| edge.target == block.id)
                    .filter_map(|edge| edge.args.get(index))
                    .map(|value| canonical.get(value).copied().unwrap_or(*value))
                    .collect::<Vec<_>>();
                if let Some(first) = values.first().copied()
                    && values.iter().all(|value| *value == first)
                {
                    canonical.insert(param.value, first);
                }
            }
        }
        if canonical.len() == before {
            break;
        }
    }
    canonical
}

fn is_constant(kind: &KirInstructionKind) -> bool {
    matches!(
        kind,
        KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::ConstFloat { .. }
            | KirInstructionKind::ConstBool { .. }
    )
}

fn is_gvn_expression(kind: &KirInstructionKind) -> bool {
    match kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. }
        | KirInstructionKind::SliceData { .. }
        | KirInstructionKind::SliceLen { .. } => true,
        KirInstructionKind::Binary { semantics, .. }
        | KirInstructionKind::Unary { semantics, .. } => {
            *semantics != KirArithmeticSemantics::Checked
        }
        _ => false,
    }
}
