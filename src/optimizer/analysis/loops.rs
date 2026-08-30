use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, KirFunction, KirInstructionKind, KirTerminator, MirBinaryOp, MirCompareOp, ValueId,
    compute_kir_dominators,
};

use super::IntegerType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturalLoop {
    pub header: BlockId,
    pub latches: Vec<BlockId>,
    pub blocks: Vec<BlockId>,
    pub parent: Option<usize>,
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InductionVariable {
    pub header: BlockId,
    pub value: ValueId,
    pub type_node: IntegerType,
    pub start: BigInt,
    pub step: BigInt,
    pub bound: ValueId,
    pub comparison: MirCompareOp,
    pub wrap_safe_for_strict_bound: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NaturalLoopAnalysis {
    pub loops: Vec<NaturalLoop>,
    pub inductions: Vec<InductionVariable>,
    pub irreducible_blocks: Vec<BlockId>,
}

#[must_use]
pub fn analyze_natural_loops(function: &KirFunction) -> NaturalLoopAnalysis {
    let dominators = compute_kir_dominators(function);
    let predecessors = predecessor_map(function);
    let mut by_header = BTreeMap::<BlockId, BTreeSet<BlockId>>::new();
    for block in &function.blocks {
        for target in successor_ids(&block.terminator) {
            if dominators.dominates(target, block.id) {
                by_header.entry(target).or_default().insert(block.id);
            }
        }
    }
    let mut loops = by_header
        .into_iter()
        .map(|(header, latches)| {
            let mut blocks = BTreeSet::from([header]);
            let mut stack = latches.iter().copied().collect::<Vec<_>>();
            blocks.extend(latches.iter().copied());
            while let Some(block) = stack.pop() {
                for predecessor in predecessors.get(&block).into_iter().flatten() {
                    if blocks.insert(*predecessor) && *predecessor != header {
                        stack.push(*predecessor);
                    }
                }
            }
            NaturalLoop {
                header,
                latches: latches.into_iter().collect(),
                blocks: blocks.into_iter().collect(),
                parent: None,
                depth: 1,
            }
        })
        .collect::<Vec<_>>();
    loops.sort_by_key(|loop_info| loop_info.header);
    for child in 0..loops.len() {
        let child_blocks = loops[child].blocks.iter().copied().collect::<BTreeSet<_>>();
        let parent = (0..loops.len())
            .filter(|candidate| *candidate != child)
            .filter(|candidate| {
                let candidate_blocks = loops[*candidate]
                    .blocks
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();
                child_blocks.is_subset(&candidate_blocks) && child_blocks != candidate_blocks
            })
            .min_by_key(|candidate| loops[*candidate].blocks.len());
        loops[child].parent = parent;
    }
    for index in 0..loops.len() {
        let mut depth = 1_u32;
        let mut parent = loops[index].parent;
        while let Some(parent_index) = parent {
            depth = depth.saturating_add(1);
            parent = loops[parent_index].parent;
        }
        loops[index].depth = depth;
    }
    let inductions = loops
        .iter()
        .flat_map(|loop_info| detect_inductions(function, loop_info))
        .collect();
    NaturalLoopAnalysis {
        loops,
        inductions,
        irreducible_blocks: Vec::new(),
    }
}

fn detect_inductions(function: &KirFunction, loop_info: &NaturalLoop) -> Vec<InductionVariable> {
    let Some(header) = function
        .blocks
        .iter()
        .find(|block| block.id == loop_info.header)
    else {
        return Vec::new();
    };
    let Some((header_comparison, left, right)) = header_comparison(function, header) else {
        return Vec::new();
    };
    header
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| {
            let type_node = IntegerType::from_mir(&param.type_node)?;
            let (comparison, bound) = if left == param.value {
                (header_comparison, right)
            } else if right == param.value {
                (reverse_comparison(header_comparison), left)
            } else {
                return None;
            };
            let incoming = incoming_edges(function, header.id)
                .into_iter()
                .map(|(predecessor, edge)| edge.args.get(index).map(|value| (predecessor, *value)))
                .collect::<Option<Vec<_>>>()?;
            let starts = incoming
                .iter()
                .filter(|(block, _)| loop_info.blocks.binary_search(block).is_err())
                .map(|(_, value)| resolve_constant(function, *value))
                .collect::<Option<Vec<_>>>()?;
            let start = starts.first()?.clone();
            if starts.iter().any(|value| value != &start)
                || super::ScalarValue::constant(type_node, start.clone()).is_err()
            {
                return None;
            }
            let mut step = None;
            for (_, value) in incoming
                .iter()
                .filter(|(block, _)| loop_info.blocks.binary_search(block).is_ok())
            {
                let transfer = defining_instruction(function, *value)?;
                if transfer.results.first()?.value != *value {
                    return None;
                }
                let KirInstructionKind::Binary {
                    op, left, right, ..
                } = transfer.kind
                else {
                    return None;
                };
                let next_step = if value_is_forwarded_from(function, left, param.value) {
                    let amount = resolve_constant(function, right)?;
                    match op {
                        MirBinaryOp::Add => amount,
                        MirBinaryOp::Sub => -amount,
                        _ => return None,
                    }
                } else if op == MirBinaryOp::Add
                    && value_is_forwarded_from(function, right, param.value)
                {
                    resolve_constant(function, left)?
                } else {
                    return None;
                };
                if step.as_ref().is_some_and(|step| step != &next_step) {
                    return None;
                }
                step = Some(next_step);
            }
            let step = step?;
            let bound = normalize_loop_invariant_bound(function, loop_info, header, bound);
            Some(InductionVariable {
                header: header.id,
                value: param.value,
                type_node,
                start,
                step: step.clone(),
                bound,
                comparison,
                // On the loop-taken edge, `i < bound` and a unit step imply
                // `i + 1 <= bound`. Both operands have the induction type, so
                // its bound cannot exceed that type's maximum. This proves the
                // increment safe for signed and unsigned integers alike.
                wrap_safe_for_strict_bound: comparison == MirCompareOp::Lt
                    && step == BigInt::from(1),
            })
        })
        .collect()
}

fn value_is_forwarded_from(function: &KirFunction, value: ValueId, origin: ValueId) -> bool {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut reaches_origin = false;
    while let Some(value) = pending.pop() {
        if value == origin {
            reaches_origin = true;
            continue;
        }
        if !visited.insert(value) {
            continue;
        }
        if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block, index))
        }) {
            let edges = incoming_edges(function, block.id);
            if edges.is_empty() {
                return false;
            }
            for (_, edge) in edges {
                let Some(value) = edge.args.get(index) else {
                    return false;
                };
                pending.push(*value);
            }
        } else if let Some(crate::KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = defining_instruction(function, value)
        {
            pending.push(*value);
        } else {
            return false;
        }
    }
    // Cyclic forwarding must reach a real source, and every noncyclic input must
    // reach that same source. Source-language slot names are not SSA evidence.
    reaches_origin
}

fn normalize_loop_invariant_bound(
    function: &KirFunction,
    loop_info: &NaturalLoop,
    header: &crate::KirBlock,
    bound: ValueId,
) -> ValueId {
    let Some((index, _)) = header
        .params
        .iter()
        .enumerate()
        .find(|(_, param)| param.value == bound)
    else {
        return bound;
    };
    let incoming = incoming_edges(function, header.id)
        .into_iter()
        .filter_map(|(predecessor, edge)| edge.args.get(index).map(|value| (predecessor, *value)))
        .collect::<Vec<_>>();
    let Some((_, entry)) = incoming
        .iter()
        .find(|(predecessor, _)| loop_info.blocks.binary_search(predecessor).is_err())
    else {
        return bound;
    };
    if incoming
        .iter()
        .all(|(_, value)| value_is_forwarded_from(function, *value, *entry))
    {
        *entry
    } else {
        bound
    }
}

fn header_comparison(
    function: &KirFunction,
    header: &crate::KirBlock,
) -> Option<(MirCompareOp, ValueId, ValueId)> {
    let KirTerminator::Branch { condition, .. } = header.terminator else {
        return None;
    };
    let instruction = defining_instruction(function, condition)?;
    let KirInstructionKind::Compare { op, left, right } = instruction.kind else {
        return None;
    };
    Some((op, left, right))
}

fn reverse_comparison(comparison: MirCompareOp) -> MirCompareOp {
    match comparison {
        MirCompareOp::Eq => MirCompareOp::Eq,
        MirCompareOp::Ne => MirCompareOp::Ne,
        MirCompareOp::Lt => MirCompareOp::Gt,
        MirCompareOp::Le => MirCompareOp::Ge,
        MirCompareOp::Gt => MirCompareOp::Lt,
        MirCompareOp::Ge => MirCompareOp::Le,
    }
}

fn defining_instruction(function: &KirFunction, value: ValueId) -> Option<&crate::KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
}

fn resolve_constant(function: &KirFunction, value: ValueId) -> Option<BigInt> {
    let instruction = defining_instruction(function, value)?;
    let KirInstructionKind::ConstInt { value } = &instruction.kind else {
        return None;
    };
    value.parse().ok()
}

fn predecessor_map(function: &KirFunction) -> BTreeMap<BlockId, Vec<BlockId>> {
    let mut result = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for block in &function.blocks {
        for target in successor_ids(&block.terminator) {
            result.entry(target).or_default().push(block.id);
        }
    }
    for predecessors in result.values_mut() {
        predecessors.sort_unstable();
        predecessors.dedup();
    }
    result
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

fn incoming_edges(function: &KirFunction, target: BlockId) -> Vec<(BlockId, &crate::KirEdge)> {
    function
        .blocks
        .iter()
        .flat_map(|block| {
            let edges = match &block.terminator {
                KirTerminator::Return { .. } => Vec::new(),
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
            };
            edges
                .into_iter()
                .filter(move |edge| edge.target == target)
                .map(move |edge| (block.id, edge))
        })
        .collect()
}
