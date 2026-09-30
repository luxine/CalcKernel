//! Bounded scalar unrolling which retains every original loop-header test.
//!
//! This changes only structured control placement: no KIR instruction, edge
//! copy, memory access, or arithmetic operation is reordered or speculated.

use super::*;

const UNROLL_FACTOR: usize = 4;
const MAX_BLOCKS: usize = 9;
const MAX_INSTRUCTIONS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuardedUnrollPlan {
    blocks: Vec<BlockId>,
    header: BlockId,
    latch: BlockId,
    exit: BlockId,
    factor: usize,
}

/// Recognition and independent source-CFG checking finish before output starts.
pub(super) fn try_emit_guarded_unroll(
    emission: &StructuredEmission<'_, '_>,
    out: &mut impl WasmOutput,
    header: BlockId,
    region: &StructureRegion,
    indent: usize,
) -> Result<bool, String> {
    let Some(plan) = propose(emission, header, region) else {
        return Ok(false);
    };
    if !independently_check(emission, region, &plan) {
        return Ok(false);
    }
    let Some(loop_label) = emission.structure.loop_labels.get(&plan.header) else {
        return Ok(false);
    };
    let Some(latch_label) = emission.structure.forward_labels.get(&plan.latch) else {
        return Ok(false);
    };
    let Some(latch) = emission
        .lowered
        .blocks
        .iter()
        .find(|block| block.source.id == plan.latch)
    else {
        return Ok(false);
    };
    let Some(edge) = latch.edges.first() else {
        return Ok(false);
    };
    let prefix = StructureRegion {
        owner: Some(plan.header),
        items: plan.blocks[..plan.blocks.len() - 1]
            .iter()
            .copied()
            .map(StructureItem::Block)
            .collect(),
    };
    let pad = " ".repeat(indent);
    out.push_str(&format!("{pad}loop ${loop_label}\n"));
    for iteration in 0..plan.factor {
        // Each copy keeps its own forward-label scopes. Reusing a label name
        // across disjoint scopes is valid in both WAT and the typed sink.
        out.push_str(&format!("{pad}  block ${latch_label}\n"));
        emission.emit_region(out, &prefix, indent + 2)?;
        out.push_str(&format!("{pad}  end\n"));
        emission.emit_block_instructions(out, latch, indent + 4)?;
        emission.emit_edge_prelude(out, edge, latch, indent + 4)?;
        if iteration + 1 == plan.factor {
            out.push_str(&format!("{pad}    br ${loop_label}\n"));
        }
        // Earlier copies fall through to the next original header. Its test
        // and complete exit edge still execute before any next-body work.
    }
    out.push_str(&format!("{pad}end\n"));
    Ok(true)
}

fn propose(
    emission: &StructuredEmission<'_, '_>,
    header: BlockId,
    region: &StructureRegion,
) -> Option<GuardedUnrollPlan> {
    if region.owner != Some(header) || !(4..=MAX_BLOCKS).contains(&region.items.len()) {
        return None;
    }
    let blocks = region
        .items
        .iter()
        .map(|item| match item {
            StructureItem::Block(id) => Some(*id),
            StructureItem::Loop { .. } => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let members = blocks.iter().copied().collect::<BTreeSet<_>>();
    if blocks.first() != Some(&header) || members.len() != blocks.len() {
        return None;
    }
    let source = emission.lowered.source;
    let header_block = source.blocks.iter().find(|block| block.id == header)?;
    let KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &header_block.terminator
    else {
        return None;
    };
    let exit = match (
        members.contains(&then_edge.target),
        members.contains(&else_edge.target),
    ) {
        (true, false) => else_edge.target,
        (false, true) => then_edge.target,
        _ => return None,
    };
    Some(GuardedUnrollPlan {
        header,
        latch: *blocks.last()?,
        blocks,
        exit,
        factor: UNROLL_FACTOR,
    })
}

fn independently_check(
    emission: &StructuredEmission<'_, '_>,
    region: &StructureRegion,
    plan: &GuardedUnrollPlan,
) -> bool {
    if plan.factor != UNROLL_FACTOR
        || !(4..=MAX_BLOCKS).contains(&plan.blocks.len())
        || region.owner != Some(plan.header)
        || region.items.len() != plan.blocks.len()
        || !region
            .items
            .iter()
            .zip(&plan.blocks)
            .all(|(item, id)| matches!(item, StructureItem::Block(actual) if actual == id))
        || plan.blocks.first() != Some(&plan.header)
        || plan.blocks.last() != Some(&plan.latch)
    {
        return false;
    }
    let source = emission.lowered.source;
    let members = plan.blocks.iter().copied().collect::<BTreeSet<_>>();
    let positions = plan
        .blocks
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect::<BTreeMap<_, _>>();
    if members.len() != plan.blocks.len() || members.contains(&plan.exit) {
        return false;
    }
    let loops = crate::analyze_canonical_loops(source);
    let Some(canonical) = loops
        .loops
        .iter()
        .find(|candidate| candidate.header == plan.header)
    else {
        return false;
    };
    if !canonical.innermost
        || !canonical.lcssa
        || !canonical.dedicated_exits
        || canonical.preheader.is_none()
        || canonical.latch != Some(plan.latch)
        || canonical.exits != [plan.exit]
        || canonical.blocks.iter().copied().collect::<BTreeSet<_>>() != members
    {
        return false;
    }
    let dominators = crate::compute_kir_dominators(source);
    let mut instructions = 0_usize;
    let mut branches = 0_usize;
    let mut memory_operations = 0_usize;
    let mut loop_memories = BTreeSet::new();
    for id in &plan.blocks {
        let Some(block) = source.blocks.iter().find(|block| block.id == *id) else {
            return false;
        };
        if !dominators.dominates(plan.header, *id) {
            return false;
        }
        instructions = instructions.saturating_add(block.instructions.len());
        if instructions > MAX_INSTRUCTIONS {
            return false;
        }
        loop_memories.extend(block.memory_params.iter().map(|param| param.version));
        for instruction in &block.instructions {
            if !instruction
                .results
                .iter()
                .all(|result| matches!(result.type_node, KirValueType::Scalar(_)))
                || !matches!(
                    instruction.kind,
                    KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::ConstFloat { .. }
                        | KirInstructionKind::ConstBool { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::Unary { .. }
                        | KirInstructionKind::Binary { .. }
                        | KirInstructionKind::Compare { .. }
                        | KirInstructionKind::Cast { .. }
                        | KirInstructionKind::SliceData { .. }
                        | KirInstructionKind::SliceLen { .. }
                        | KirInstructionKind::Load { .. }
                        | KirInstructionKind::Store { .. }
                )
            {
                return false;
            }
            if let Some(memory) = &instruction.memory {
                memory_operations += 1;
                loop_memories.extend(memory.output);
            }
        }
        if *id == plan.header
            && block.instructions.iter().any(|instruction| {
                instruction.memory.is_some()
                    || instruction.effect.is_some()
                    || !matches!(
                        instruction.kind,
                        KirInstructionKind::ConstInt { .. }
                            | KirInstructionKind::ConstFloat { .. }
                            | KirInstructionKind::ConstBool { .. }
                            | KirInstructionKind::Copy { .. }
                            | KirInstructionKind::Compare { .. }
                            | KirInstructionKind::SliceData { .. }
                            | KirInstructionKind::SliceLen { .. }
                    )
            })
        {
            return false;
        }
        let edges = edges(&block.terminator);
        if matches!(block.terminator, KirTerminator::Branch { .. }) {
            branches += 1;
        }
        if edges.is_empty() {
            return false;
        }
        if *id == plan.header {
            if edges.len() != 2
                || !edges.iter().any(|(_, edge)| edge.target == plan.blocks[1])
                || !edges.iter().any(|(_, edge)| edge.target == plan.exit)
            {
                return false;
            }
        } else if *id == plan.latch {
            if edges.len() != 1
                || edges[0].1.target != plan.header
                || !matches!(block.terminator, KirTerminator::Jump { .. })
            {
                return false;
            }
        } else if edges.iter().any(|(_, edge)| {
            positions
                .get(&edge.target)
                .is_none_or(|target| *target <= positions[id])
        }) {
            return false;
        }
        for (arm, edge) in edges {
            let expected = if *id == plan.latch {
                BranchTarget::Loop(plan.header)
            } else {
                BranchTarget::Forward(edge.target)
            };
            if emission.structure.branch_targets.get(&(*id, arm)) != Some(&expected) {
                return false;
            }
        }
        // Leave existing complete-store-tree lowering in control of its loops.
        if super::super::lower::piecewise_closed_store_tree(emission.lowered, *id, &members)
            .is_some()
        {
            return false;
        }
    }
    if branches < 2 || memory_operations == 0 {
        return false;
    }
    let mut entries = 0_usize;
    for block in &source.blocks {
        if members.contains(&block.id) {
            continue;
        }
        // A loop MemorySSA definition must escape only through the original
        // exit edge and fresh continuation parameters, never a direct use.
        if block.instructions.iter().any(|instruction| {
            instruction
                .memory
                .as_ref()
                .is_some_and(|memory| loop_memories.contains(&memory.input))
        }) {
            return false;
        }
        for (_, edge) in edges(&block.terminator) {
            if edge
                .memory_args
                .iter()
                .any(|memory| loop_memories.contains(memory))
            {
                return false;
            }
            if members.contains(&edge.target) {
                if edge.target != plan.header || Some(block.id) != canonical.preheader {
                    return false;
                }
                entries += 1;
            }
        }
        if let KirTerminator::Return { memory, .. } = &block.terminator
            && memory.iter().any(|state| loop_memories.contains(&state.1))
        {
            return false;
        }
    }
    entries == 1
}

fn edges(terminator: &KirTerminator) -> Vec<(u8, &KirEdge)> {
    match terminator {
        KirTerminator::Jump { edge } => vec![(0, edge)],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![(0, then_edge), (1, else_edge)],
        KirTerminator::Return { .. } => Vec::new(),
    }
}
