use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, KirFunction, KirInstructionKind, KirTerminator, MirBinaryOp, MirCompareOp, ValueId,
    compute_kir_dominators_with_budget,
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
    pub budget_exhausted: bool,
}

#[must_use]
pub fn analyze_natural_loops(function: &KirFunction) -> NaturalLoopAnalysis {
    analyze_natural_loops_with_config(function, super::ScalarAnalysisConfig::default())
}

#[must_use]
pub fn analyze_natural_loops_with_config(
    function: &KirFunction,
    config: super::ScalarAnalysisConfig,
) -> NaturalLoopAnalysis {
    let mut budget = LoopBudget {
        remaining: super::ScalarAnalysisBudget::for_function(function, config).max_steps(),
        exhausted: false,
    };
    analyze_with_budget(function, &mut budget).unwrap_or_else(|| NaturalLoopAnalysis {
        budget_exhausted: true,
        ..NaturalLoopAnalysis::default()
    })
}

struct LoopBudget {
    remaining: u32,
    exhausted: bool,
}

impl LoopBudget {
    fn spend(&mut self, units: usize) -> Option<()> {
        let next = u32::try_from(units)
            .ok()
            .and_then(|units| self.remaining.checked_sub(units));
        if let Some(remaining) = next.filter(|_| !self.exhausted) {
            self.remaining = remaining;
            Some(())
        } else {
            self.exhausted = true;
            None
        }
    }
}

fn analyze_with_budget(
    function: &KirFunction,
    budget: &mut LoopBudget,
) -> Option<NaturalLoopAnalysis> {
    budget.spend(1)?;
    let dominators = compute_kir_dominators_with_budget(function, &mut budget.remaining)?;
    let irreducible_blocks = irreducible_blocks(function, &dominators, budget)?;
    if !irreducible_blocks.is_empty() {
        return Some(NaturalLoopAnalysis {
            irreducible_blocks,
            ..NaturalLoopAnalysis::default()
        });
    }
    let predecessors = predecessor_map(function);
    let mut by_header = BTreeMap::<BlockId, BTreeSet<BlockId>>::new();
    for block in &function.blocks {
        budget.spend(1)?;
        for target in successor_ids(&block.terminator) {
            if dominators.dominates(target, block.id) {
                by_header.entry(target).or_default().insert(block.id);
            }
        }
    }
    let mut loops = Vec::new();
    for (header, latches) in by_header {
        let mut blocks = BTreeSet::from([header]);
        let mut stack = latches.iter().copied().collect::<Vec<_>>();
        blocks.extend(latches.iter().copied());
        while let Some(block) = stack.pop() {
            budget.spend(1)?;
            if block == header {
                continue;
            }
            for predecessor in predecessors.get(&block).into_iter().flatten() {
                budget.spend(1)?;
                if blocks.insert(*predecessor) && *predecessor != header {
                    stack.push(*predecessor);
                }
            }
        }
        loops.push(NaturalLoop {
            header,
            latches: latches.into_iter().collect(),
            blocks: blocks.into_iter().collect(),
            parent: None,
            depth: 1,
        });
    }
    loops.sort_by_key(|loop_info| loop_info.header);
    for child in 0..loops.len() {
        let mut parent = None;
        for candidate in 0..loops.len() {
            budget.spend(1)?;
            if loops[candidate].blocks.len() <= loops[child].blocks.len() {
                continue;
            }
            budget.spend(loops[child].blocks.len())?;
            if loops[child]
                .blocks
                .iter()
                .all(|block| loops[candidate].blocks.binary_search(block).is_ok())
                && parent.is_none_or(|parent: usize| {
                    loops[candidate].blocks.len() < loops[parent].blocks.len()
                })
            {
                parent = Some(candidate);
            }
        }
        loops[child].parent = parent;
    }
    for index in 0..loops.len() {
        let mut depth = 1_u32;
        let mut parent = loops[index].parent;
        while let Some(parent_index) = parent {
            budget.spend(1)?;
            depth = depth.saturating_add(1);
            parent = loops[parent_index].parent;
        }
        loops[index].depth = depth;
    }
    let mut inductions = Vec::new();
    for loop_info in &loops {
        budget.spend(1)?;
        inductions.extend(detect_inductions(function, loop_info, budget));
        budget.spend(0)?;
    }
    Some(NaturalLoopAnalysis {
        loops,
        inductions,
        irreducible_blocks: Vec::new(),
        budget_exhausted: false,
    })
}

fn irreducible_blocks(
    function: &KirFunction,
    dominators: &crate::KirDominators,
    budget: &mut LoopBudget,
) -> Option<Vec<BlockId>> {
    // A reducible graph is acyclic after removing all dominance backedges.
    // Inspect the residual graph, not just maximal SCC entry counts: an outer
    // natural loop can contain a multi-entry inner cycle.
    let mut forward = BTreeMap::<BlockId, Vec<BlockId>>::new();
    let mut reverse = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for block in &function.blocks {
        budget.spend(1)?;
        let targets = forward.entry(block.id).or_default();
        for target in successor_ids(&block.terminator) {
            if !dominators.dominates(target, block.id) {
                targets.push(target);
                reverse.entry(target).or_default().push(block.id);
            }
        }
        targets.sort_unstable();
        targets.dedup();
    }
    for predecessors in reverse.values_mut() {
        predecessors.sort_unstable();
        predecessors.dedup();
    }
    let mut visited = BTreeSet::new();
    let mut finish = Vec::new();
    for &root in forward.keys() {
        let mut stack = vec![(root, false)];
        while let Some((block, returning)) = stack.pop() {
            budget.spend(1)?;
            if returning {
                finish.push(block);
            } else if visited.insert(block) {
                stack.push((block, true));
                for &target in forward.get(&block).into_iter().flatten().rev() {
                    stack.push((target, false));
                }
            }
        }
    }
    let mut assigned = BTreeSet::new();
    let mut irreducible = BTreeSet::new();
    for root in finish.into_iter().rev() {
        if assigned.contains(&root) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![root];
        while let Some(block) = stack.pop() {
            budget.spend(1)?;
            if assigned.insert(block) {
                component.push(block);
                stack.extend(reverse.get(&block).into_iter().flatten().copied());
            }
        }
        if component.len() > 1 {
            irreducible.extend(component);
        }
    }
    Some(irreducible.into_iter().collect())
}

fn detect_inductions(
    function: &KirFunction,
    loop_info: &NaturalLoop,
    budget: &mut LoopBudget,
) -> Vec<InductionVariable> {
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
            budget.spend(1)?;
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
                budget.spend(1)?;
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
                let next_step = if value_is_forwarded_from(function, left, param.value, budget) {
                    let amount = resolve_constant(function, right)?;
                    match op {
                        MirBinaryOp::Add => amount,
                        MirBinaryOp::Sub => -amount,
                        _ => return None,
                    }
                } else if op == MirBinaryOp::Add
                    && value_is_forwarded_from(function, right, param.value, budget)
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
            let bound = normalize_loop_invariant_bound(function, loop_info, header, bound, budget);
            Some(InductionVariable {
                header: header.id,
                value: param.value,
                type_node,
                start,
                step: step.clone(),
                bound,
                comparison,
                // A strict same-type bound leaves room for one step towards
                // it, including descending steps near the integer minimum.
                wrap_safe_for_strict_bound: (comparison == MirCompareOp::Lt
                    && step == BigInt::from(1))
                    || (comparison == MirCompareOp::Gt && step == BigInt::from(-1)),
            })
        })
        .collect()
}

fn value_is_forwarded_from(
    function: &KirFunction,
    value: ValueId,
    origin: ValueId,
    budget: &mut LoopBudget,
) -> bool {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut reaches_origin = false;
    while let Some(value) = pending.pop() {
        if budget.spend(1).is_none() {
            return false;
        }
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
    budget: &mut LoopBudget,
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
        .all(|(_, value)| value_is_forwarded_from(function, *value, *entry, budget))
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
