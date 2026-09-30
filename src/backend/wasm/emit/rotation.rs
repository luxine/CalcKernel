//! Conservative rotation for the common two-block, top-tested natural loop.
//!
//! This module intentionally recognizes a small subset of structured loops.
//! Any uncertainty leaves the existing structured emitter in control.

use super::super::ir::WasmPhysicalType;
use super::*;

struct RotatedLoop<'a> {
    header: &'a WasmLoweredBlock<'a>,
    body: &'a WasmLoweredBlock<'a>,
    condition: crate::ValueId,
    body_edge: &'a WasmLoweredEdge<'a>,
    latch_edge: &'a WasmLoweredEdge<'a>,
    exit_edge: &'a WasmLoweredEdge<'a>,
    body_when_true: bool,
}

/// Emits a rotated loop when the complete shape has been proved safe.
///
/// The caller must invoke this before emitting the ordinary `loop` region.
/// Recognition and all checks finish before the first byte is written, so a
/// rejected loop always falls through to the original emitter unchanged.
pub(super) fn try_emit_rotated_loop(
    emission: &StructuredEmission<'_, '_>,
    out: &mut impl WasmOutput,
    header_id: crate::BlockId,
    region: &StructureRegion,
    loop_indent: usize,
) -> Result<bool, String> {
    let Some(loop_shape) = recognize_loop(emission, header_id, region) else {
        return Ok(false);
    };

    let pad = |indent| " ".repeat(indent);
    let header_indent = loop_indent + 2;
    let loop_body_indent = loop_indent + 4;
    let instruction_indent = loop_indent + 6;

    emission.emit_block_instructions(out, loop_shape.header, instruction_indent)?;
    emit_condition(
        emission,
        out,
        loop_shape.condition,
        loop_shape.body_when_true,
        header_indent,
    );
    out.push_str(&format!("{}if\n", pad(header_indent)));

    let loop_label = emission
        .structure
        .loop_labels
        .get(&header_id)
        .expect("structure planner labels every recognized loop");
    out.push_str(&format!("{}loop ${loop_label}\n", pad(loop_body_indent)));
    emission.emit_edge_prelude(
        out,
        loop_shape.body_edge,
        loop_shape.header,
        instruction_indent,
    )?;
    emission.emit_block_instructions(out, loop_shape.body, instruction_indent)?;
    emission.emit_edge_prelude(
        out,
        loop_shape.latch_edge,
        loop_shape.body,
        instruction_indent,
    )?;
    emission.emit_block_instructions(out, loop_shape.header, instruction_indent)?;
    emit_condition(
        emission,
        out,
        loop_shape.condition,
        loop_shape.body_when_true,
        instruction_indent,
    );
    out.push_str(&format!("{}br_if ${loop_label}\n", pad(instruction_indent)));
    out.push_str(&format!("{}end\n", pad(loop_body_indent)));
    out.push_str(&format!("{}end\n", pad(header_indent)));

    // Both the initial zero-trip case and the final failed condition reach
    // here. The loop's exit phi copies and branch therefore execute once.
    emission.emit_edge(out, loop_shape.exit_edge, loop_shape.header, header_indent)?;
    Ok(true)
}

fn recognize_loop<'a>(
    emission: &'a StructuredEmission<'_, '_>,
    header_id: crate::BlockId,
    region: &'a StructureRegion,
) -> Option<RotatedLoop<'a>> {
    if region.owner != Some(header_id) || region.items.len() != 2 {
        return None;
    }
    let (StructureItem::Block(found_header), StructureItem::Block(body_id)) =
        (&region.items[0], &region.items[1])
    else {
        return None;
    };
    if *found_header != header_id || *body_id == header_id {
        return None;
    }

    let lowered = emission.lowered;
    let header = lowered
        .blocks
        .iter()
        .find(|block| block.source.id == header_id)?;
    let body = lowered
        .blocks
        .iter()
        .find(|block| block.source.id == *body_id)?;
    if !header_is_pure(header) {
        return None;
    }

    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.source.terminator
    else {
        return None;
    };
    let condition_value = lowered.values.get(condition)?;
    if condition_value.physical != WasmPhysicalType::I32 || condition_value.operand.is_none() {
        return None;
    }

    let (body_source_edge, exit_source_edge, body_when_true) = if then_edge.target == *body_id {
        if else_edge.target == *body_id {
            return None;
        }
        (then_edge, else_edge, true)
    } else if else_edge.target == *body_id {
        (else_edge, then_edge, false)
    } else {
        return None;
    };
    if exit_source_edge.target == header_id || exit_source_edge.target == *body_id {
        return None;
    }

    let KirTerminator::Jump {
        edge: latch_source_edge,
    } = &body.source.terminator
    else {
        return None;
    };
    if latch_source_edge.target != header_id {
        return None;
    }

    // The body has exactly one incoming edge. The header may have one external
    // entry in addition to this latch; multiple entries and extra latches are
    // intentionally left to the general structured emitter.
    if predecessor_sources(lowered, *body_id).as_slice() != [header_id] {
        return None;
    }
    let header_predecessors = predecessor_sources(lowered, header_id);
    let internal_latches = header_predecessors
        .iter()
        .filter(|source| **source == *body_id)
        .count();
    if internal_latches != 1
        || header_predecessors
            .iter()
            .filter(|source| **source != *body_id)
            .count()
            > 1
    {
        return None;
    }

    let body_edge = header.edges.iter().find(|edge| {
        edge.arm == if body_when_true { 0 } else { 1 }
            && edge.source.target == *body_id
            && edge.source == body_source_edge
    })?;
    let exit_edge = header.edges.iter().find(|edge| {
        edge.arm == if body_when_true { 1 } else { 0 }
            && edge.source.target == exit_source_edge.target
            && edge.source == exit_source_edge
    })?;
    let latch_edge = body
        .edges
        .iter()
        .find(|edge| edge.arm == 0 && edge.source == latch_source_edge)?;

    if emission
        .structure
        .branch_targets
        .get(&(header_id, body_edge.arm))
        != Some(&BranchTarget::Forward(*body_id))
        || emission
            .structure
            .branch_targets
            .get(&(body.source.id, latch_edge.arm))
            != Some(&BranchTarget::Loop(header_id))
        || !edge_has_no_cursor_actions(lowered, header.source.id, body_edge.arm)
        || !edge_has_no_cursor_actions(lowered, header.source.id, exit_edge.arm)
        || !edge_has_no_cursor_actions(lowered, body.source.id, latch_edge.arm)
    {
        return None;
    }

    Some(RotatedLoop {
        header,
        body,
        condition: *condition,
        body_edge,
        latch_edge,
        exit_edge,
        body_when_true,
    })
}

fn header_is_pure(header: &WasmLoweredBlock<'_>) -> bool {
    header.instructions.len() <= 8
        && header.instructions.iter().all(|instruction| {
            instruction.source.memory.is_none()
                && instruction.source.effect.is_none()
                && matches!(
                    instruction.source.kind,
                    KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::ConstFloat { .. }
                        | KirInstructionKind::ConstBool { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::SliceLen { .. }
                        | KirInstructionKind::SliceData { .. }
                        | KirInstructionKind::Compare { .. }
                )
        })
}

fn predecessor_sources(
    lowered: &WasmLoweredFunction<'_>,
    target: crate::BlockId,
) -> Vec<crate::BlockId> {
    lowered
        .source
        .blocks
        .iter()
        .filter_map(|block| {
            terminator_edges(&block.terminator)
                .iter()
                .any(|edge| edge.target == target)
                .then_some(block.id)
        })
        .collect()
}

fn terminator_edges(terminator: &KirTerminator) -> Vec<&KirEdge> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
    }
}

fn edge_has_no_cursor_actions(
    lowered: &WasmLoweredFunction<'_>,
    source: crate::BlockId,
    arm: u8,
) -> bool {
    lowered
        .memory_plan
        .edge_actions
        .get(&(source, arm))
        .is_none_or(Vec::is_empty)
}

fn emit_condition(
    emission: &StructuredEmission<'_, '_>,
    out: &mut impl WasmOutput,
    condition: crate::ValueId,
    body_when_true: bool,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    let value = emission
        .lowered
        .values
        .get(&condition)
        .and_then(|value| value.operand.as_ref())
        .expect("recognizer requires a scalar condition value");
    if let Some(plan) = emission.paired {
        emit_wat_paired_scalar_value(out, value, plan, indent);
    } else {
        emit_wat_value(out, value, indent);
    }
    if !body_when_true {
        out.push_str(&format!("{pad}i32.eqz\n"));
    }
}
