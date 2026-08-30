use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    BlockId, KirArithmeticSemantics, KirInstructionKind, KirModule, KirTerminator, MirBinaryOp,
    MirCastOp, MirCompareOp, MirType, MirUnaryOp, ValueId, compute_kir_dominators,
};

use super::rewrite::replace_value_uses_batch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Expression<'a> {
    Integer(&'a str),
    Float(&'a str),
    Boolean(bool),
    Copy(ValueId),
    Binary(MirBinaryOp, ValueId, ValueId, KirArithmeticSemantics),
    Unary(MirUnaryOp, ValueId, KirArithmeticSemantics),
    Compare(MirCompareOp, ValueId, ValueId),
    Cast(MirCastOp, ValueId),
    SliceData(ValueId),
    SliceLen(ValueId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ExpressionKey<'a> {
    type_node: &'a MirType,
    expression: Expression<'a>,
}

fn expression_key<'a>(
    type_node: &'a MirType,
    kind: &'a KirInstructionKind,
    canonical: &BTreeMap<ValueId, ValueId>,
) -> Option<ExpressionKey<'a>> {
    let remap = |value: &ValueId| canonical.get(value).copied().unwrap_or(*value);
    let expression = match kind {
        KirInstructionKind::ConstInt { value } => Expression::Integer(value),
        KirInstructionKind::ConstFloat { value } => Expression::Float(value),
        KirInstructionKind::ConstBool { value } => Expression::Boolean(*value),
        KirInstructionKind::Copy { value } => Expression::Copy(remap(value)),
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } if *semantics != KirArithmeticSemantics::Checked => {
            Expression::Binary(*op, remap(left), remap(right), *semantics)
        }
        KirInstructionKind::Unary {
            op,
            operand,
            semantics,
        } if *semantics != KirArithmeticSemantics::Checked => {
            Expression::Unary(*op, remap(operand), *semantics)
        }
        KirInstructionKind::Compare { op, left, right } => {
            Expression::Compare(*op, remap(left), remap(right))
        }
        KirInstructionKind::Cast { op, value } => Expression::Cast(*op, remap(value)),
        KirInstructionKind::SliceData { slice } => Expression::SliceData(remap(slice)),
        KirInstructionKind::SliceLen { slice } => Expression::SliceLen(remap(slice)),
        _ => return None,
    };
    Some(ExpressionKey {
        type_node,
        expression,
    })
}

pub(crate) fn run_gvn(module: &mut KirModule, protected: &BTreeSet<crate::InstructionId>) -> u32 {
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
        let mut removed = BTreeSet::new();
        // These maps are queried only by key, never iterated to select a rewrite.
        // Block/instruction order and each definition list remain deterministic.
        let mut global = HashMap::<ExpressionKey<'_>, Vec<(BlockId, ValueId)>>::new();
        let mut global_constants = HashMap::<ExpressionKey<'_>, Vec<(BlockId, ValueId)>>::new();
        for block in &function.blocks {
            let mut available = HashMap::<ExpressionKey<'_>, ValueId>::new();
            let mut constants = HashMap::<ExpressionKey<'_>, ValueId>::new();
            for instruction in &block.instructions {
                if instruction.effect.is_some() || instruction.memory.is_some() {
                    available.clear();
                    continue;
                }
                let Some(result) = instruction.results.first() else {
                    continue;
                };
                if instruction.results.len() != 1 {
                    continue;
                }
                let Some(key) =
                    expression_key(&result.type_node, &instruction.kind, &canonical_values)
                else {
                    continue;
                };
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
                if let Some(existing) = existing
                    && !protected.contains(&instruction.id)
                {
                    replacements.push((result.value, existing));
                    removed.insert(instruction.id);
                    rewrites = rewrites.saturating_add(1);
                } else {
                    table.insert(key, result.value);
                    global_table
                        .entry(key)
                        .or_default()
                        .push((block.id, result.value));
                }
            }
        }
        replace_value_uses_batch(function, &replacements);
        for block in &mut function.blocks {
            block
                .instructions
                .retain(|instruction| !removed.contains(&instruction.id));
        }
    }
    rewrites
}

fn canonical_block_params(function: &crate::KirFunction) -> BTreeMap<ValueId, ValueId> {
    let mut incoming = BTreeMap::<BlockId, Vec<Vec<ValueId>>>::new();
    for predecessor in &function.blocks {
        let edges = match &predecessor.terminator {
            KirTerminator::Return { .. } => Vec::new(),
            KirTerminator::Jump { edge } => vec![edge],
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
        };
        for edge in edges {
            incoming
                .entry(edge.target)
                .or_default()
                .push(edge.args.clone());
        }
    }
    let mut canonical = BTreeMap::new();
    loop {
        let before = canonical.len();
        for block in &function.blocks {
            for (index, param) in block.params.iter().enumerate() {
                let values = incoming
                    .get(&block.id)
                    .into_iter()
                    .flatten()
                    .filter_map(|args| args.get(index))
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

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::super::rewrite::remap_instruction_values;
    use super::*;
    use crate::{
        InstructionId, KirInstruction, KirResult, MirBinaryOp, MirCastOp, MirCompareOp,
        MirPrimitiveTypeName, MirType, MirUnaryOp,
    };

    #[test]
    fn borrowed_expression_keys_preserve_legacy_identity() {
        let values = [0, 1, 2, u32::MAX].map(ValueId::from_index);
        // Remapping is one lookup, not transitive closure (including cycles).
        let canonical = BTreeMap::from([(values[0], values[1]), (values[1], values[0])]);
        let mut kinds = Vec::new();
        for text in ["0", "-0", "1", "01", "NaN", "a\":b"] {
            kinds.push(KirInstructionKind::ConstInt { value: text.into() });
            kinds.push(KirInstructionKind::ConstFloat { value: text.into() });
        }
        for value in [false, true] {
            kinds.push(KirInstructionKind::ConstBool { value });
        }
        for &value in &values {
            kinds.push(KirInstructionKind::Copy { value });
            kinds.push(KirInstructionKind::SliceData { slice: value });
            kinds.push(KirInstructionKind::SliceLen { slice: value });
            for op in [MirCastOp::I32ToF64, MirCastOp::U32ToF64] {
                kinds.push(KirInstructionKind::Cast { op, value });
            }
            for semantics in [
                KirArithmeticSemantics::Modular,
                KirArithmeticSemantics::StrictFloat,
                KirArithmeticSemantics::Checked,
            ] {
                for op in [MirUnaryOp::Neg, MirUnaryOp::Not] {
                    kinds.push(KirInstructionKind::Unary {
                        op,
                        operand: value,
                        semantics,
                    });
                }
                for &right in &values {
                    for op in [
                        MirBinaryOp::Add,
                        MirBinaryOp::Sub,
                        MirBinaryOp::Mul,
                        MirBinaryOp::Div,
                        MirBinaryOp::Mod,
                    ] {
                        kinds.push(KirInstructionKind::Binary {
                            op,
                            left: value,
                            right,
                            semantics,
                        });
                    }
                }
            }
            for &right in &values {
                for op in [
                    MirCompareOp::Eq,
                    MirCompareOp::Ne,
                    MirCompareOp::Lt,
                    MirCompareOp::Le,
                    MirCompareOp::Gt,
                    MirCompareOp::Ge,
                ] {
                    kinds.push(KirInstructionKind::Compare {
                        op,
                        left: value,
                        right,
                    });
                }
            }
        }
        let types = [
            MirType::Primitive(MirPrimitiveTypeName::I32),
            MirType::Primitive(MirPrimitiveTypeName::U32),
            MirType::Primitive(MirPrimitiveTypeName::F64),
            MirType::Pointer(Box::new(MirType::Primitive(MirPrimitiveTypeName::I32))),
            MirType::Slice(Box::new(MirType::Primitive(MirPrimitiveTypeName::I32))),
            MirType::Struct("a\":b".into()),
            MirType::Void,
        ];
        let mut old_to_new = BTreeMap::new();
        let mut new_to_old = std::collections::HashMap::new();
        for type_node in &types {
            for kind in &kinds {
                let key = expression_key(type_node, kind, &canonical);
                assert_eq!(key.is_some(), is_gvn_expression(kind));
                let Some(key) = key else { continue };
                let mut instruction = KirInstruction {
                    id: InstructionId::from_index(0),
                    results: vec![KirResult {
                        value: values[0],
                        type_node: type_node.clone(),
                    }],
                    kind: kind.clone(),
                    memory: None,
                    effect: None,
                };
                remap_instruction_values(&mut instruction, &canonical);
                let old = format!("{type_node:?}:{:?}", instruction.kind);
                if let Some(previous) = old_to_new.insert(old.clone(), key) {
                    assert_eq!(
                        previous, key,
                        "legacy-equivalent expressions must share a key"
                    );
                }
                if let Some(previous) = new_to_old.insert(key, old.clone()) {
                    assert_eq!(
                        previous, old,
                        "typed keys must not merge distinct legacy expressions"
                    );
                }
            }
        }
        assert_eq!(old_to_new.len(), new_to_old.len());
    }
}
