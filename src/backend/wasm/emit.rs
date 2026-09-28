use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::*;

use super::super::{collect_temps, is_f64_type, is_unsigned_integer_type, place_type, value_type};
use super::{
    EmitWasmOptions,
    bulk::{CheckedWasmBulkMemory, WasmBulkKind, checked_wasm_bulk_memory_candidate},
    control::{BranchTarget, StructureItem, StructurePlan, StructureRegion, plan_structure},
    final_ir::{FinalWasmBuilder, FinalWasmModule, WasmOutput},
    ir::{
        WasmLoweredBlock, WasmLoweredEdge, WasmLoweredFunction, WasmLoweredModule, WasmSourceType,
    },
    layout::*,
    memory::{CursorId, CursorOperand, WasmMemoryEdgeAction},
    plan::*,
};

pub(super) fn emit_final_module_with_options(
    module: &MirModule,
    options: EmitWasmOptions,
) -> Result<FinalWasmModule, String> {
    let layout = WasmStructLayout::new(module);
    let mut out = FinalWasmBuilder::new();
    out.push_str("(module\n");
    out.push_str("  (memory (export \"memory\") 1)\n");
    out.push_str("  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n");
    for function in &module.functions {
        out.push('\n');
        emit_wat_function(&mut out, function, &layout, options);
    }
    out.push_str(")\n");
    let mut final_module = out.finish()?;
    super::placement::optimize_final_module(&mut final_module, options.opt_level);
    Ok(final_module)
}

pub(super) fn emit_final_module_with_lowering(
    module: &MirModule,
    lowered: &WasmLoweredModule<'_>,
    options: EmitWasmOptions,
) -> Result<FinalWasmModule, String> {
    let mut out = FinalWasmBuilder::new();
    emit_module_with_lowering_into(module, lowered, options, &mut out)?;
    let mut final_module = out.finish()?;
    super::placement::optimize_final_module(&mut final_module, options.opt_level);
    Ok(final_module)
}

fn emit_module_with_lowering_into(
    module: &MirModule,
    lowered: &WasmLoweredModule<'_>,
    options: EmitWasmOptions,
    out: &mut impl WasmOutput,
) -> Result<(), String> {
    debug_assert_eq!(module.functions.len(), lowered.functions.len());
    debug_assert!(
        lowered
            .functions
            .iter()
            .all(WasmLoweredFunction::source_metadata_is_consistent)
    );
    let layout = WasmStructLayout::new(module);
    out.push_str("(module\n");
    out.push_str("  (memory (export \"memory\") 1)\n");
    out.push_str("  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n");
    for function in &module.functions {
        out.push('\n');
        let typed = lowered
            .functions
            .iter()
            .find(|candidate| candidate.source.name == function.name);
        let has_vectors = typed.is_some_and(|candidate| !candidate.vector_values.is_empty());
        let has_version_predicate = typed.is_some_and(|candidate| {
            candidate.source.blocks.iter().any(|block| {
                block.instructions.iter().any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::VersionPredicate { .. }
                    )
                })
            })
        });
        let has_memory_cursors = options.opt_level >= 3
            && typed.is_some_and(|candidate| !candidate.memory_plan.cursors.is_empty());
        let has_memarg_offsets = options.opt_level >= 3
            && typed.is_some_and(|candidate| {
                !candidate
                    .memory_plan
                    .memarg_offset_by_instruction
                    .is_empty()
                    && single_block_scalar_memarg_sink_is_eligible(candidate)
            });
        let bulk_candidate = (options.opt_level >= 3)
            .then(|| {
                typed.and_then(|candidate| checked_wasm_bulk_memory_candidate(candidate.source))
            })
            .flatten();
        let has_bulk_candidate = bulk_candidate.is_some();
        let needs_typed_lowering = has_vectors
            || has_version_predicate
            || has_memory_cursors
            || has_memarg_offsets
            || has_bulk_candidate;
        let mut structure = if options.opt_level >= 3
            && (needs_typed_lowering || detect_simple_wasm_while(function).is_none())
        {
            typed
                .filter(|candidate| {
                    candidate.source.blocks.len() != 1
                        || !matches!(
                            &candidate.source.blocks[0].terminator,
                            KirTerminator::Return { .. }
                        )
                })
                .and_then(|candidate| {
                    plan_structure(candidate.source)
                        .filter(|plan| structured_plan_is_emittable(candidate, plan))
                })
        } else {
            None
        };
        if let Some(structure) = &mut structure {
            uniquify_structure_labels(structure, function);
        }
        if has_memarg_offsets
            && let Some(typed) =
                typed.filter(|candidate| single_block_scalar_memarg_sink_is_eligible(candidate))
        {
            emit_wat_typed_single_block_function(out, typed, &layout)?;
            continue;
        }
        if let Some(typed) = typed.filter(|_| needs_typed_lowering) {
            let vector_names = vector_local_names(typed);
            let cursor_names = if options.opt_level >= 3 {
                memory_cursor_local_names(typed, &vector_names)
            } else {
                BTreeMap::new()
            };
            if let Some(structure) = structure.as_ref() {
                emit_wat_structured_function(
                    out,
                    typed,
                    structure,
                    &layout,
                    &vector_names,
                    &cursor_names,
                    bulk_candidate.as_ref(),
                )?;
            } else {
                emit_wat_typed_dispatcher_function(
                    out,
                    typed,
                    &layout,
                    &vector_names,
                    &cursor_names,
                    bulk_candidate.as_ref(),
                )?;
            }
        } else if let (Some(typed), Some(structure)) = (typed, structure.as_ref()) {
            let vector_names = BTreeMap::new();
            let cursor_names = BTreeMap::new();
            emit_wat_structured_function(
                out,
                typed,
                structure,
                &layout,
                &vector_names,
                &cursor_names,
                bulk_candidate.as_ref(),
            )?;
        } else {
            emit_wat_function(out, function, &layout, options);
        }
    }
    out.push_str(")\n");
    Ok(())
}

fn single_block_scalar_memarg_sink_is_eligible(lowered: &WasmLoweredFunction<'_>) -> bool {
    if lowered.source.blocks.len() != 1
        || lowered.blocks.len() != 1
        || !lowered.source.blocks[0].params.is_empty()
        || !matches!(
            lowered.source.blocks[0].terminator,
            KirTerminator::Return { .. }
        )
        || !matches!(
            lowered
                .local_view
                .blocks
                .first()
                .map(|block| &block.terminator),
            Some(MirTerminator::Return { .. })
        )
        || !lowered.vector_values.is_empty()
        || !lowered.memory_plan.cursors.is_empty()
        || !lowered.memory_plan.edge_actions.is_empty()
        || wasm_function_uses_slices(&lowered.local_view)
    {
        return false;
    }
    !lowered.source.blocks[0]
        .instructions
        .iter()
        .any(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            ) || is_vector_instruction(&instruction.kind)
        })
}

fn vector_local_names(lowered: &WasmLoweredFunction<'_>) -> BTreeMap<crate::ValueId, String> {
    let mut used_names = collect_wasm_function_names(&lowered.local_view);
    lowered
        .vector_values
        .iter()
        .map(|value| {
            let preferred = format!("ik_v{}", value.index());
            (
                *value,
                unique_wasm_internal_name(&preferred, &mut used_names),
            )
        })
        .collect()
}

fn memory_cursor_local_names(
    lowered: &WasmLoweredFunction<'_>,
    vector_names: &BTreeMap<crate::ValueId, String>,
) -> BTreeMap<CursorId, String> {
    if lowered.memory_plan.cursors.is_empty() {
        return BTreeMap::new();
    }
    let mut used_names = collect_wasm_function_names(&lowered.local_view);
    used_names.extend(vector_names.values().cloned());
    let plan = WasmFunctionPlan::new(&lowered.local_view);
    for physical in plan.values.values() {
        match physical {
            WasmPhysicalValue::Scalar(name) => {
                used_names.insert(name.clone());
            }
            WasmPhysicalValue::Slice { data, len } => {
                used_names.insert(data.clone());
                used_names.insert(len.clone());
            }
        }
    }
    used_names.extend([
        plan.address_local,
        plan.block_local,
        plan.return_scalar,
        plan.return_data,
        plan.return_len,
    ]);
    lowered
        .memory_plan
        .cursors
        .iter()
        .map(|cursor| {
            (
                cursor.id,
                unique_wasm_internal_name(
                    &format!("ik_mem_cursor{}", cursor.id.0),
                    &mut used_names,
                ),
            )
        })
        .collect()
}

fn emit_wat_memory_cursor_locals(
    out: &mut impl WasmOutput,
    cursor_names: &BTreeMap<CursorId, String>,
) {
    for name in cursor_names.values() {
        out.push_str(&format!("    (local ${name} i32)\n"));
    }
}

struct WasmBulkScratch {
    length: String,
    destination_start: String,
    destination_end: String,
    source_start: String,
    source_end: String,
    memory_end: String,
}

// At shorter ranges the complete-bounds guard can cost more than the scalar
// loop. Keep those calls on the original path; this threshold is a size policy,
// not a safety assumption.
const MIN_BULK_ELEMENTS: u32 = 16;

impl WasmBulkScratch {
    fn new(used_names: &mut HashSet<String>) -> Self {
        Self {
            length: unique_wasm_internal_name("ik_bulk_length", used_names),
            destination_start: unique_wasm_internal_name("ik_bulk_dst_start", used_names),
            destination_end: unique_wasm_internal_name("ik_bulk_dst_end", used_names),
            source_start: unique_wasm_internal_name("ik_bulk_src_start", used_names),
            source_end: unique_wasm_internal_name("ik_bulk_src_end", used_names),
            memory_end: unique_wasm_internal_name("ik_bulk_memory_end", used_names),
        }
    }

    fn emit_locals(&self, out: &mut impl WasmOutput) {
        for name in [
            &self.length,
            &self.destination_start,
            &self.destination_end,
            &self.source_start,
            &self.source_end,
            &self.memory_end,
        ] {
            out.push_str(&format!("    (local ${name} i64)\n"));
        }
    }
}

fn emit_wat_bulk_memory_guard(
    out: &mut impl WasmOutput,
    candidate: &CheckedWasmBulkMemory,
    scratch: &WasmBulkScratch,
    lowered: &WasmLoweredFunction<'_>,
    indent: usize,
) -> Result<(), String> {
    let destination = scalar_operand(lowered, candidate.destination)?;
    let start = scalar_operand(lowered, candidate.start)?;
    let end = scalar_operand(lowered, candidate.end)?;
    if !matches!(
        destination,
        MirValue::Param {
            type_node: MirType::Pointer(_),
            ..
        }
    ) || value_type(start) != &MirType::Primitive(MirPrimitiveTypeName::U32)
        || value_type(end) != &MirType::Primitive(MirPrimitiveTypeName::U32)
    {
        return Err("WebAssembly bulk memory candidate has invalid entry operands".into());
    }
    let source = candidate
        .source
        .map(|value| scalar_operand(lowered, value))
        .transpose()?;
    if let Some(source) = source
        && !matches!(
            source,
            MirValue::Param {
                type_node: MirType::Pointer(_),
                ..
            }
        )
    {
        return Err("WebAssembly bulk copy source is not a pointer parameter".into());
    }

    let pad = " ".repeat(indent);
    emit_wat_value(out, end, indent);
    emit_wat_value(out, start, indent);
    out.push_str(&format!("{pad}i32.gt_u\n",));
    emit_wat_value(out, end, indent);
    emit_wat_value(out, start, indent);
    out.push_str(&format!(
        "{pad}i32.sub\n{pad}i32.const {MIN_BULK_ELEMENTS}\n{pad}i32.ge_u\n{pad}i32.and\n{pad}if\n{}",
        " ".repeat(indent + 2)
    ));

    emit_wat_value(out, end, indent + 2);
    out.push_str(&format!("{}i64.extend_i32_u\n", " ".repeat(indent + 2)));
    emit_wat_value(out, start, indent + 2);
    out.push_str(&format!(
        "{}i64.extend_i32_u\n{}i64.sub\n{}i64.const {}\n{}i64.mul\n{}local.tee ${}\n{}i64.const 4294967295\n{}i64.le_u\n{}if\n",
        " ".repeat(indent + 2),
        " ".repeat(indent + 2),
        " ".repeat(indent + 2),
        candidate.element_bytes,
        " ".repeat(indent + 2),
        " ".repeat(indent + 2),
        scratch.length,
        " ".repeat(indent + 2),
        " ".repeat(indent + 2),
        " ".repeat(indent + 2),
    ));

    emit_wat_bulk_range_start(
        out,
        destination,
        start,
        candidate.element_bytes,
        &scratch.destination_start,
        indent + 4,
    );
    emit_wat_bulk_range_end(
        out,
        &scratch.destination_start,
        &scratch.length,
        &scratch.destination_end,
        indent + 4,
    );
    if let Some(source) = source {
        emit_wat_bulk_range_start(
            out,
            source,
            start,
            candidate.element_bytes,
            &scratch.source_start,
            indent + 4,
        );
        emit_wat_bulk_range_end(
            out,
            &scratch.source_start,
            &scratch.length,
            &scratch.source_end,
            indent + 4,
        );
    }
    out.push_str(&format!(
        "{}memory.size\n{}i64.extend_i32_u\n{}i64.const 65536\n{}i64.mul\n{}local.set ${}\n",
        " ".repeat(indent + 4),
        " ".repeat(indent + 4),
        " ".repeat(indent + 4),
        " ".repeat(indent + 4),
        " ".repeat(indent + 4),
        scratch.memory_end,
    ));

    emit_wat_bulk_range_is_in_bounds(
        out,
        &scratch.destination_start,
        &scratch.destination_end,
        &scratch.memory_end,
        indent + 4,
    );
    if source.is_some() {
        emit_wat_bulk_range_is_in_bounds(
            out,
            &scratch.source_start,
            &scratch.source_end,
            &scratch.memory_end,
            indent + 4,
        );
        out.push_str(&format!("{}i32.and\n", " ".repeat(indent + 4)));
        out.push_str(&format!(
            "{}local.get ${}\n{}local.get ${}\n{}i64.le_u\n{}local.get ${}\n{}local.get ${}\n{}i64.le_u\n{}i32.or\n{}i32.and\n",
            " ".repeat(indent + 4),
            scratch.source_end,
            " ".repeat(indent + 4),
            scratch.destination_start,
            " ".repeat(indent + 4),
            " ".repeat(indent + 4),
            scratch.destination_end,
            " ".repeat(indent + 4),
            scratch.source_start,
            " ".repeat(indent + 4),
            " ".repeat(indent + 4),
            " ".repeat(indent + 4),
        ));
    }

    out.push_str(&format!("{}if\n", " ".repeat(indent + 4)));
    match candidate.kind {
        WasmBulkKind::Copy => {
            out.push_str(&format!(
                "{}local.get ${}\n{}i32.wrap_i64\n{}local.get ${}\n{}i32.wrap_i64\n{}local.get ${}\n{}i32.wrap_i64\n{}memory.copy\n{}return\n",
                " ".repeat(indent + 6),
                scratch.destination_start,
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
                scratch.source_start,
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
                scratch.length,
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
            ));
        }
        WasmBulkKind::Fill { byte } => {
            out.push_str(&format!(
                "{}local.get ${}\n{}i32.wrap_i64\n{}i32.const {}\n{}local.get ${}\n{}i32.wrap_i64\n{}memory.fill\n{}return\n",
                " ".repeat(indent + 6),
                scratch.destination_start,
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
                byte,
                " ".repeat(indent + 6),
                scratch.length,
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
                " ".repeat(indent + 6),
            ));
        }
    }
    out.push_str(&format!(
        "{}end\n{}end\n{}end\n",
        " ".repeat(indent + 4),
        " ".repeat(indent + 2),
        pad,
    ));
    Ok(())
}

fn emit_wat_bulk_range_start(
    out: &mut impl WasmOutput,
    pointer: &MirValue,
    start: &MirValue,
    element_bytes: u32,
    local: &str,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    emit_wat_value(out, pointer, indent);
    out.push_str(&format!("{pad}i64.extend_i32_u\n"));
    emit_wat_value(out, start, indent);
    out.push_str(&format!(
        "{pad}i64.extend_i32_u\n{pad}i64.const {element_bytes}\n{pad}i64.mul\n{pad}i64.add\n{pad}local.set ${local}\n"
    ));
}

fn emit_wat_bulk_range_end(
    out: &mut impl WasmOutput,
    range_start: &str,
    length: &str,
    range_end: &str,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    out.push_str(&format!(
        "{pad}local.get ${range_start}\n{pad}local.get ${length}\n{pad}i64.add\n{pad}local.set ${range_end}\n"
    ));
}

fn emit_wat_bulk_range_is_in_bounds(
    out: &mut impl WasmOutput,
    range_start: &str,
    range_end: &str,
    memory_end: &str,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    out.push_str(&format!(
        "{pad}local.get ${range_start}\n{pad}i64.const 4294967295\n{pad}i64.le_u\n{pad}local.get ${range_end}\n{pad}i64.const 4294967296\n{pad}i64.le_u\n{pad}i32.and\n{pad}local.get ${range_end}\n{pad}local.get ${memory_end}\n{pad}i64.le_u\n{pad}i32.and\n"
    ));
}

#[derive(Debug)]
struct VersionPredicateScratch {
    left_end: String,
    right_end: String,
}

struct VersionPredicateSliceContext<'a, 'source> {
    lowered: &'a WasmLoweredFunction<'source>,
    plan: &'a WasmFunctionPlan,
}

struct WasmSpecialLocals<'a> {
    vector_names: &'a BTreeMap<crate::ValueId, String>,
    memory_cursor_names: &'a BTreeMap<CursorId, String>,
    predicate_scratch: Option<&'a VersionPredicateScratch>,
}

fn version_predicate_scratch(
    lowered: &WasmLoweredFunction<'_>,
    plan: Option<&WasmFunctionPlan>,
    vector_names: &BTreeMap<crate::ValueId, String>,
) -> Option<VersionPredicateScratch> {
    let uses_intervals = lowered.source.blocks.iter().any(|block| {
        block.instructions.iter().any(|instruction| {
            matches!(
                &instruction.kind,
                KirInstructionKind::VersionPredicate { predicate }
                    if predicate.conjuncts.iter().any(|conjunct| matches!(
                        conjunct,
                        KirVersionPredicateConjunct::AddressIntervalsDisjoint { .. }
                    ))
            )
        })
    });
    if !uses_intervals {
        return None;
    }

    let mut used_names = collect_wasm_function_names(&lowered.local_view);
    used_names.extend(vector_names.values().cloned());
    if let Some(plan) = plan {
        for value in plan.values.values() {
            match value {
                WasmPhysicalValue::Scalar(name) => {
                    used_names.insert(name.clone());
                }
                WasmPhysicalValue::Slice { data, len } => {
                    used_names.insert(data.clone());
                    used_names.insert(len.clone());
                }
            }
        }
        used_names.extend([
            plan.address_local.clone(),
            plan.block_local.clone(),
            plan.return_scalar.clone(),
            plan.return_data.clone(),
            plan.return_len.clone(),
        ]);
    }
    Some(VersionPredicateScratch {
        left_end: unique_wasm_internal_name("ik_pred_left_end", &mut used_names),
        right_end: unique_wasm_internal_name("ik_pred_right_end", &mut used_names),
    })
}

fn emit_version_predicate_scratch_locals(
    out: &mut impl WasmOutput,
    scratch: Option<&VersionPredicateScratch>,
) {
    if let Some(scratch) = scratch {
        out.push_str(&format!("    (local ${} i64)\n", scratch.left_end));
        out.push_str(&format!("    (local ${} i64)\n", scratch.right_end));
    }
}

fn uniquify_structure_labels(structure: &mut StructurePlan, function: &MirFunction) {
    let mut used_names = collect_wasm_function_names(function);
    for label in structure
        .forward_labels
        .values_mut()
        .chain(structure.loop_labels.values_mut())
    {
        *label = unique_wasm_internal_name(label, &mut used_names);
    }
}

fn structured_plan_is_emittable(
    lowered: &WasmLoweredFunction<'_>,
    structure: &StructurePlan,
) -> bool {
    if !lowered.source_metadata_is_consistent() {
        return false;
    }
    let mut emitted = BTreeSet::new();
    if !collect_planned_blocks(&structure.root, &mut emitted)
        || emitted != structure.reachable
        || lowered.local_view.blocks.len() != lowered.blocks.len()
        || lowered.local_view.name != lowered.source.name
        || lowered.local_view.exported != lowered.source.exported
        || lowered.local_view.return_type != lowered.source.return_type
    {
        return false;
    }

    let types = super::kir::value_types(lowered.source);
    let all_types = super::kir::value_kir_types(lowered.source);
    if lowered.values.len() != all_types.len()
        || all_types
            .keys()
            .any(|value| !lowered.values.contains_key(value))
        || structure.root.owner.is_some()
    {
        return false;
    }
    let params = lowered
        .source
        .params
        .iter()
        .map(|param| (param.value, (param.name.clone(), param.type_node.clone())))
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected_params = lowered
        .source
        .params
        .iter()
        .map(|param| MirParam {
            name: param.name.clone(),
            type_node: param.type_node.clone(),
        })
        .collect::<Vec<_>>();
    if lowered.local_view.params != expected_params {
        return false;
    }
    let local_values = lowered
        .source
        .blocks
        .iter()
        .flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        })
        .collect::<BTreeSet<_>>();
    let expected_locals = local_values
        .iter()
        .filter_map(|value| {
            types.get(value).map(|type_node| MirLocal {
                name: super::kir::local_name(*value),
                type_node: type_node.clone(),
            })
        })
        .collect::<Vec<_>>();
    if lowered.local_view.locals != expected_locals {
        return false;
    }

    let mut expected_edges = BTreeSet::new();
    for (lowered_block, view_block) in lowered.blocks.iter().zip(&lowered.local_view.blocks) {
        if view_block.label != super::kir::block_label(lowered_block.source.id) {
            return false;
        }
        let mut expected_instructions = Vec::new();
        for instruction in &lowered_block.source.instructions {
            let Ok(leaves) = super::kir::adapt_instruction(instruction, &types, &params) else {
                return false;
            };
            expected_instructions.extend(leaves);
        }
        for edge in &lowered_block.edges {
            let Some(target) = lowered
                .source
                .blocks
                .iter()
                .find(|candidate| candidate.id == edge.source.target)
            else {
                return false;
            };
            if edge.source.args.len() != target.params.len()
                || edge.source.memory_args.len() != target.memory_params.len()
            {
                return false;
            }
            let label = super::kir::edge_label(
                lowered_block.source.id,
                edge.source.target,
                u32::from(edge.arm),
            );
            let copies =
                super::kir::edge_copy_instructions(&label, target, edge.source, &types, &params);
            if edge.copies != copies {
                return false;
            }
            expected_instructions.extend(copies);
            if structure.reachable.contains(&lowered_block.source.id) {
                let key = (lowered_block.source.id, edge.arm);
                if !expected_edges.insert(key) {
                    return false;
                }
                let Some(target_kind) = structure.branch_targets.get(&key) else {
                    return false;
                };
                let label = match target_kind {
                    BranchTarget::Forward(target) => structure.forward_labels.get(target),
                    BranchTarget::Loop(target) => structure.loop_labels.get(target),
                };
                if !structure.reachable.contains(&edge.source.target)
                    || label.is_none_or(String::is_empty)
                {
                    return false;
                }
            }
        }
        if view_block.instructions != expected_instructions {
            return false;
        }
        let expected_terminator = match &lowered_block.source.terminator {
            KirTerminator::Return { value, .. } => MirTerminator::Return {
                value: value.map(|value| super::kir::mir_value(value, &types, &params)),
            },
            KirTerminator::Jump { edge } => MirTerminator::Jump {
                label: super::kir::block_label(edge.target),
            },
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => MirTerminator::Branch {
                condition: super::kir::mir_value(*condition, &types, &params),
                then_label: super::kir::block_label(then_edge.target),
                else_label: super::kir::block_label(else_edge.target),
            },
        };
        if view_block.terminator != expected_terminator {
            return false;
        }
    }
    expected_edges.len() == structure.branch_targets.len()
}

fn collect_planned_blocks(region: &StructureRegion, blocks: &mut BTreeSet<crate::BlockId>) -> bool {
    for item in &region.items {
        match item {
            StructureItem::Block(block) => {
                if !blocks.insert(*block) {
                    return false;
                }
            }
            StructureItem::Loop { header, body } => {
                if body.owner != Some(*header)
                    || !matches!(body.items.first(), Some(StructureItem::Block(block)) if block == header)
                    || !collect_planned_blocks(body, blocks)
                {
                    return false;
                }
            }
        }
    }
    true
}

pub(super) fn emit_wat_function(
    out: &mut impl WasmOutput,
    function: &MirFunction,
    layout: &WasmStructLayout,
    options: EmitWasmOptions,
) {
    if wasm_function_uses_slices(function) {
        emit_wat_slice_function(out, function, layout, options);
        return;
    }
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        out.push_str(&format!(
            "    (param ${} {})\n",
            param.name,
            wasm_type(&param.type_node)
        ));
    }
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (result {})\n",
            wasm_type(&function.return_type)
        ));
    }

    let mut locals = HashSet::new();
    for local in &function.locals {
        if locals.insert(local.name.clone()) {
            out.push_str(&format!(
                "    (local ${} {})\n",
                local.name,
                wasm_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if locals.insert(name.clone()) {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(&type_node)));
        }
    }

    if function.blocks.len() == 1 {
        for instruction in &function.blocks[0].instructions {
            emit_wat_instruction(out, instruction, layout, 4);
        }
        emit_wat_terminator(out, &function.blocks[0].terminator, None, 4);
    } else if let Some(loop_context) = (options.opt_level >= 3)
        .then(|| detect_simple_wasm_while(function))
        .flatten()
    {
        emit_structured_wasm_while(out, &loop_context, layout);
    } else {
        emit_dispatched_wasm_function(out, function, layout);
    }
    out.push_str("  )\n");
}

fn emit_wat_structured_function(
    out: &mut impl WasmOutput,
    lowered: &WasmLoweredFunction<'_>,
    structure: &StructurePlan,
    layout: &WasmStructLayout,
    vector_names: &BTreeMap<crate::ValueId, String>,
    cursor_names: &BTreeMap<CursorId, String>,
    bulk_candidate: Option<&CheckedWasmBulkMemory>,
) -> Result<(), String> {
    let function = &lowered.local_view;
    if wasm_function_uses_slices(function) {
        emit_wat_structured_slice_function(
            out,
            function,
            lowered,
            structure,
            layout,
            vector_names,
            cursor_names,
        )
    } else {
        emit_wat_structured_scalar_function(
            out,
            lowered,
            structure,
            layout,
            vector_names,
            cursor_names,
            bulk_candidate,
        )
    }
}

fn emit_wat_typed_single_block_function(
    out: &mut impl WasmOutput,
    lowered: &WasmLoweredFunction<'_>,
    layout: &WasmStructLayout,
) -> Result<(), String> {
    if lowered.source.blocks.len() != 1 || lowered.blocks.len() != 1 {
        return Err("WebAssembly memarg offset sink requires one source block".into());
    }
    let function = &lowered.local_view;
    if wasm_function_uses_slices(function) || !lowered.vector_values.is_empty() {
        return Err("WebAssembly memarg offset sink accepts scalar single-block functions".into());
    }
    let block = &lowered.blocks[0];
    let view_block = function
        .blocks
        .first()
        .ok_or_else(|| "WebAssembly memarg offset sink has no MIR block".to_string())?;
    if !matches!(block.source.terminator, KirTerminator::Return { .. })
        || !matches!(view_block.terminator, MirTerminator::Return { .. })
    {
        return Err("WebAssembly memarg offset sink requires a return terminator".into());
    }

    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        out.push_str(&format!(
            "    (param ${} {})\n",
            param.name,
            wasm_type(&param.type_node)
        ));
    }
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (result {})\n",
            wasm_type(&function.return_type)
        ));
    }
    let mut locals = HashSet::new();
    for local in &function.locals {
        if locals.insert(local.name.clone()) {
            out.push_str(&format!(
                "    (local ${} {})\n",
                local.name,
                wasm_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if locals.insert(name.clone()) {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(&type_node)));
        }
    }

    let no_vector_names = BTreeMap::new();
    let no_cursor_names = BTreeMap::new();
    let special_locals = WasmSpecialLocals {
        vector_names: &no_vector_names,
        memory_cursor_names: &no_cursor_names,
        predicate_scratch: None,
    };
    for instruction in &block.instructions {
        if lowered
            .memory_plan
            .memarg_offset_by_instruction
            .contains_key(&instruction.source.id)
        {
            emit_wat_memarg_offset_instruction(out, instruction, lowered, layout, None, 4)?;
        } else {
            emit_wat_lowered_instruction(
                out,
                instruction,
                lowered,
                layout,
                None,
                &special_locals,
                4,
            )?;
        }
    }
    emit_wat_terminator(out, &view_block.terminator, None, 4);
    out.push_str("  )\n");
    Ok(())
}

fn emit_wat_structured_scalar_function(
    out: &mut impl WasmOutput,
    lowered: &WasmLoweredFunction<'_>,
    structure: &StructurePlan,
    layout: &WasmStructLayout,
    vector_names: &BTreeMap<crate::ValueId, String>,
    cursor_names: &BTreeMap<CursorId, String>,
    bulk_candidate: Option<&CheckedWasmBulkMemory>,
) -> Result<(), String> {
    let function = &lowered.local_view;
    let predicate_scratch = version_predicate_scratch(lowered, None, vector_names);
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        out.push_str(&format!(
            "    (param ${} {})\n",
            param.name,
            wasm_type(&param.type_node)
        ));
    }
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (result {})\n",
            wasm_type(&function.return_type)
        ));
    }

    let mut locals = HashSet::new();
    for local in &function.locals {
        if locals.insert(local.name.clone()) {
            out.push_str(&format!(
                "    (local ${} {})\n",
                local.name,
                wasm_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if locals.insert(name.clone()) {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(&type_node)));
        }
    }
    emit_wat_vector_locals(out, vector_names);
    emit_wat_memory_cursor_locals(out, cursor_names);
    emit_version_predicate_scratch_locals(out, predicate_scratch.as_ref());

    let mut used_names = collect_wasm_function_names(function);
    let bulk_scratch = bulk_candidate.map(|_| {
        let scratch = WasmBulkScratch::new(&mut used_names);
        scratch.emit_locals(out);
        scratch
    });
    let exit_label = unique_wasm_internal_name("ik_exit", &mut used_names);
    let return_local = if matches!(function.return_type, MirType::Void) {
        None
    } else {
        let name = unique_wasm_internal_name("ik_ret", &mut used_names);
        out.push_str(&format!(
            "    (local ${name} {})\n",
            wasm_type(&function.return_type)
        ));
        Some(name)
    };
    if let (Some(candidate), Some(scratch)) = (bulk_candidate, bulk_scratch.as_ref()) {
        emit_wat_bulk_memory_guard(out, candidate, scratch, lowered, 4)?;
    }
    out.push_str(&format!("    block ${exit_label}\n"));
    StructuredEmission {
        lowered,
        structure,
        layout,
        paired: None,
        exit_label: &exit_label,
        return_local: return_local.as_deref(),
        vector_names,
        memory_cursor_names: cursor_names,
        predicate_scratch: predicate_scratch.as_ref(),
    }
    .emit_region(out, &structure.root, 6)?;
    out.push_str("    end\n");
    if let Some(return_local) = return_local {
        out.push_str(&format!("    local.get ${return_local}\n"));
    }
    out.push_str("  )\n");
    Ok(())
}

fn emit_wat_structured_slice_function(
    out: &mut impl WasmOutput,
    function: &MirFunction,
    lowered: &WasmLoweredFunction<'_>,
    structure: &StructurePlan,
    layout: &WasmStructLayout,
    vector_names: &BTreeMap<crate::ValueId, String>,
    cursor_names: &BTreeMap<CursorId, String>,
) -> Result<(), String> {
    let plan = WasmFunctionPlan::new(function);
    let predicate_scratch = version_predicate_scratch(lowered, Some(&plan), vector_names);
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        match plan
            .values
            .get(&format!("param:{}", param.name))
            .expect("param must have physical WASM names")
        {
            WasmPhysicalValue::Scalar(name) => out.push_str(&format!(
                "    (param ${name} {})\n",
                wasm_type(&param.type_node)
            )),
            WasmPhysicalValue::Slice { data, len } => {
                out.push_str(&format!("    (param ${data} i32)\n"));
                out.push_str(&format!("    (param ${len} i32)\n"));
            }
        }
    }
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str("    (result i32 i32)\n"),
        type_node => out.push_str(&format!("    (result {})\n", wasm_type(type_node))),
    }
    for local in &function.locals {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("local:{}", local.name))
                .expect("local must have physical WASM names"),
            &local.type_node,
        );
    }
    for (name, type_node) in collect_temps(function) {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("temp:{name}"))
                .expect("temp must have physical WASM names"),
            &type_node,
        );
    }
    emit_wat_vector_locals(out, vector_names);
    emit_wat_memory_cursor_locals(out, cursor_names);
    emit_version_predicate_scratch_locals(out, predicate_scratch.as_ref());
    out.push_str(&format!("    (local ${} i32)\n", plan.address_local));
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => {
            out.push_str(&format!("    (local ${} i32)\n", plan.return_data));
            out.push_str(&format!("    (local ${} i32)\n", plan.return_len));
        }
        type_node => out.push_str(&format!(
            "    (local ${} {})\n",
            plan.return_scalar,
            wasm_type(type_node)
        )),
    }
    let mut used_names = collect_wasm_function_names(function);
    let exit_label = unique_wasm_internal_name("ik_exit", &mut used_names);
    out.push_str(&format!("    block ${exit_label}\n"));
    StructuredEmission {
        lowered,
        structure,
        layout,
        paired: Some(&plan),
        exit_label: &exit_label,
        return_local: None,
        vector_names,
        memory_cursor_names: cursor_names,
        predicate_scratch: predicate_scratch.as_ref(),
    }
    .emit_region(out, &structure.root, 6)?;
    out.push_str("    end\n");
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str(&format!(
            "    local.get ${}\n    local.get ${}\n",
            plan.return_data, plan.return_len
        )),
        _ => out.push_str(&format!("    local.get ${}\n", plan.return_scalar)),
    }
    out.push_str("  )\n");
    Ok(())
}

fn emit_wat_vector_locals(
    out: &mut impl WasmOutput,
    vector_names: &BTreeMap<crate::ValueId, String>,
) {
    for name in vector_names.values() {
        out.push_str(&format!("    (local ${name} v128)\n"));
    }
}

fn emit_wat_typed_dispatcher_function(
    out: &mut impl WasmOutput,
    lowered: &WasmLoweredFunction<'_>,
    layout: &WasmStructLayout,
    vector_names: &BTreeMap<crate::ValueId, String>,
    cursor_names: &BTreeMap<CursorId, String>,
    bulk_candidate: Option<&CheckedWasmBulkMemory>,
) -> Result<(), String> {
    let function = &lowered.local_view;
    let plan = WasmFunctionPlan::new(function);
    let predicate_scratch = version_predicate_scratch(lowered, Some(&plan), vector_names);
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        match plan
            .values
            .get(&format!("param:{}", param.name))
            .expect("param must have physical WASM names")
        {
            WasmPhysicalValue::Scalar(name) => out.push_str(&format!(
                "    (param ${name} {})\n",
                wasm_type(&param.type_node)
            )),
            WasmPhysicalValue::Slice { data, len } => {
                out.push_str(&format!("    (param ${data} i32)\n"));
                out.push_str(&format!("    (param ${len} i32)\n"));
            }
        }
    }
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str("    (result i32 i32)\n"),
        type_node => out.push_str(&format!("    (result {})\n", wasm_type(type_node))),
    }
    for local in &function.locals {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("local:{}", local.name))
                .expect("local must have physical WASM names"),
            &local.type_node,
        );
    }
    for (name, type_node) in collect_temps(function) {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("temp:{name}"))
                .expect("temp must have physical WASM names"),
            &type_node,
        );
    }
    emit_wat_vector_locals(out, vector_names);
    emit_wat_memory_cursor_locals(out, cursor_names);
    emit_version_predicate_scratch_locals(out, predicate_scratch.as_ref());
    let mut used_names = collect_wasm_function_names(function);
    let bulk_scratch = bulk_candidate.map(|_| {
        let scratch = WasmBulkScratch::new(&mut used_names);
        scratch.emit_locals(out);
        scratch
    });
    out.push_str(&format!("    (local ${} i32)\n", plan.address_local));
    out.push_str(&format!("    (local ${} i32)\n", plan.block_local));
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => {
            out.push_str(&format!("    (local ${} i32)\n", plan.return_data));
            out.push_str(&format!("    (local ${} i32)\n", plan.return_len));
        }
        type_node => out.push_str(&format!(
            "    (local ${} {})\n",
            plan.return_scalar,
            wasm_type(type_node)
        )),
    }

    if let (Some(candidate), Some(scratch)) = (bulk_candidate, bulk_scratch.as_ref()) {
        emit_wat_bulk_memory_guard(out, candidate, scratch, lowered, 4)?;
    }

    let exit_label = unique_wasm_internal_name("ik_exit", &mut used_names);
    let dispatch_label = unique_wasm_internal_name("ik_dispatch", &mut used_names);
    let case_labels = (0..lowered.blocks.len())
        .map(|index| unique_wasm_internal_name(&format!("ik_case{index}"), &mut used_names))
        .collect::<Vec<_>>();
    let Some(default_case) = case_labels.first() else {
        return Err("WebAssembly typed dispatcher requires an entry block".to_string());
    };
    let block_indices = lowered
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            u32::try_from(index)
                .map(|index| (block.source.id, index))
                .map_err(|_| "WebAssembly typed dispatcher block index overflow".to_string())
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let dispatcher = TypedDispatcher {
        lowered,
        block_indices: &block_indices,
        layout,
        plan: &plan,
        cursor_names,
        exit_label: &exit_label,
        dispatch_label: &dispatch_label,
    };
    out.push_str("    i32.const 0\n");
    out.push_str(&format!("    local.set ${}\n", plan.block_local));
    out.push_str(&format!(
        "    block ${exit_label}\n      loop ${dispatch_label}\n"
    ));
    for case in &case_labels {
        out.push_str(&format!("        block ${case}\n"));
    }
    out.push_str(&format!(
        "{}local.get ${}\n{}br_table {} ${}\n",
        " ".repeat(8 + case_labels.len() * 2),
        plan.block_local,
        " ".repeat(8 + case_labels.len() * 2),
        case_labels
            .iter()
            .map(|label| format!("${label}"))
            .collect::<Vec<_>>()
            .join(" "),
        default_case,
    ));
    for index in (0..lowered.blocks.len()).rev() {
        let block = &lowered.blocks[index];
        let indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(indent)));
        let special_locals = WasmSpecialLocals {
            vector_names,
            memory_cursor_names: cursor_names,
            predicate_scratch: predicate_scratch.as_ref(),
        };
        for instruction in &block.instructions {
            emit_wat_lowered_instruction(
                out,
                instruction,
                lowered,
                layout,
                Some(&plan),
                &special_locals,
                indent,
            )?;
        }
        dispatcher.emit_terminator(out, block, indent)?;
    }
    out.push_str("      end\n    end\n");
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str(&format!(
            "    local.get ${}\n    local.get ${}\n",
            plan.return_data, plan.return_len
        )),
        _ => out.push_str(&format!("    local.get ${}\n", plan.return_scalar)),
    }
    out.push_str("  )\n");
    Ok(())
}

struct TypedDispatcher<'a, 'source> {
    lowered: &'a WasmLoweredFunction<'source>,
    block_indices: &'a BTreeMap<crate::BlockId, u32>,
    layout: &'a WasmStructLayout,
    plan: &'a WasmFunctionPlan,
    cursor_names: &'a BTreeMap<CursorId, String>,
    exit_label: &'a str,
    dispatch_label: &'a str,
}

impl TypedDispatcher<'_, '_> {
    fn emit_terminator(
        &self,
        out: &mut impl WasmOutput,
        block: &WasmLoweredBlock<'_>,
        indent: usize,
    ) -> Result<(), String> {
        let pad = " ".repeat(indent);
        match &block.source.terminator {
            KirTerminator::Return { value, .. } => {
                if let Some(value) = value {
                    let operand = scalar_operand(self.lowered, *value)?;
                    if matches!(value_type(operand), MirType::Slice(_)) {
                        emit_wat_paired_slice_value(out, operand, self.plan, indent);
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}local.set ${}\n",
                            self.plan.return_len, self.plan.return_data
                        ));
                    } else {
                        emit_wat_paired_scalar_value(out, operand, self.plan, indent);
                        out.push_str(&format!("{pad}local.set ${}\n", self.plan.return_scalar));
                    }
                }
                out.push_str(&format!("{pad}br ${}\n", self.exit_label));
            }
            KirTerminator::Jump { edge } => {
                let lowered_edge = block
                    .edges
                    .first()
                    .ok_or_else(|| "typed dispatcher is missing a jump edge".to_string())?;
                self.emit_edge(out, lowered_edge, block.source.id, edge.target, indent)?;
            }
            KirTerminator::Branch {
                condition,
                then_edge: source_then_edge,
                else_edge: source_else_edge,
            } => {
                let condition = scalar_operand(self.lowered, *condition)?;
                emit_wat_paired_scalar_value(out, condition, self.plan, indent);
                out.push_str(&format!("{pad}if\n"));
                let then_edge = block
                    .edges
                    .iter()
                    .find(|edge| edge.arm == 0)
                    .ok_or_else(|| "typed dispatcher is missing the then edge".to_string())?;
                self.emit_edge(
                    out,
                    then_edge,
                    block.source.id,
                    source_then_edge.target,
                    indent + 2,
                )?;
                out.push_str(&format!("{pad}else\n"));
                let else_edge = block
                    .edges
                    .iter()
                    .find(|edge| edge.arm == 1)
                    .ok_or_else(|| "typed dispatcher is missing the else edge".to_string())?;
                self.emit_edge(
                    out,
                    else_edge,
                    block.source.id,
                    source_else_edge.target,
                    indent + 2,
                )?;
                out.push_str(&format!("{pad}end\n{pad}br ${}\n", self.dispatch_label));
            }
        }
        Ok(())
    }

    fn emit_edge(
        &self,
        out: &mut impl WasmOutput,
        edge: &WasmLoweredEdge<'_>,
        source: crate::BlockId,
        target: crate::BlockId,
        indent: usize,
    ) -> Result<(), String> {
        emit_wat_memory_edge_actions(
            out,
            self.lowered,
            edge,
            source,
            Some(self.plan),
            self.cursor_names,
            indent,
        )?;
        for copy in &edge.copies {
            emit_wat_paired_instruction(out, copy, self.layout, self.plan, indent);
        }
        let index = self
            .block_indices
            .get(&target)
            .ok_or_else(|| format!("typed dispatcher has no block {}", target.index()))?;
        let pad = " ".repeat(indent);
        out.push_str(&format!(
            "{pad}i32.const {index}\n{pad}local.set ${}\n{pad}br ${}\n",
            self.plan.block_local, self.dispatch_label
        ));
        Ok(())
    }
}

fn scalar_operand<'a>(
    lowered: &'a WasmLoweredFunction<'_>,
    value: crate::ValueId,
) -> Result<&'a MirValue, String> {
    lowered
        .values
        .get(&value)
        .and_then(|typed| typed.operand.as_ref())
        .ok_or_else(|| {
            format!(
                "WebAssembly scalar operand {} has no MIR view",
                value.index()
            )
        })
}

fn emit_wat_version_predicate(
    out: &mut impl WasmOutput,
    instruction: &KirInstruction,
    predicate: &KirVersionPredicate,
    lowered: &WasmLoweredFunction<'_>,
    plan: Option<&WasmFunctionPlan>,
    scratch: Option<&VersionPredicateScratch>,
    indent: usize,
) -> Result<(), String> {
    let [result] = instruction.results.as_slice() else {
        return Err("WebAssembly version predicate result is malformed".into());
    };
    if predicate.conjuncts.is_empty() {
        return Err("WebAssembly version predicate has no conjuncts".into());
    }
    let pad = " ".repeat(indent);
    for (index, conjunct) in predicate.conjuncts.iter().enumerate() {
        match conjunct {
            KirVersionPredicateConjunct::TripThreshold { value, minimum } => {
                emit_version_predicate_scalar(out, *value, lowered, plan, indent)?;
                out.push_str(&format!("{pad}i32.const {minimum}\n{pad}i32.ge_u\n"));
            }
            KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                left,
                left_count,
                left_element_bytes,
                right,
                right_count,
                right_element_bytes,
            } => {
                let plan = plan.ok_or_else(|| {
                    "WebAssembly address predicates require the paired slice ABI plan".to_string()
                })?;
                let scratch = scratch.ok_or_else(|| {
                    "WebAssembly address predicates are missing their i64 temporaries".to_string()
                })?;
                let context = VersionPredicateSliceContext { lowered, plan };
                emit_version_predicate_scalar(out, *left_count, lowered, Some(plan), indent)?;
                out.push_str(&format!("{pad}i32.eqz\n"));
                emit_version_predicate_scalar(out, *right_count, lowered, Some(plan), indent)?;
                out.push_str(&format!(
                    "{pad}i32.eqz\n{pad}i32.or\n{pad}if (result i32)\n"
                ));
                out.push_str(&format!("{pad}  i32.const 1\n{pad}else\n"));

                emit_wat_version_interval_end(
                    out,
                    *left_count,
                    *left,
                    *left_element_bytes,
                    &scratch.left_end,
                    &context,
                    indent + 2,
                )?;
                emit_wat_version_interval_end(
                    out,
                    *right_count,
                    *right,
                    *right_element_bytes,
                    &scratch.right_end,
                    &context,
                    indent + 2,
                )?;
                out.push_str(&format!("{}i32.and\n", " ".repeat(indent + 2)));
                emit_wat_version_end_le_address(
                    out,
                    &scratch.left_end,
                    *right,
                    &context,
                    indent + 2,
                )?;
                emit_wat_version_end_le_address(
                    out,
                    &scratch.right_end,
                    *left,
                    &context,
                    indent + 2,
                )?;
                out.push_str(&format!(
                    "{}i32.or\n{}i32.and\n{}end\n",
                    " ".repeat(indent + 2),
                    " ".repeat(indent + 2),
                    pad
                ));
            }
        }
        if index > 0 {
            out.push_str(&format!("{pad}i32.and\n"));
        }
    }

    let result = scalar_operand(lowered, result.value)?;
    let local = plan.map_or_else(|| wat_local_name(result), |plan| plan.scalar(result));
    out.push_str(&format!("{pad}local.set ${local}\n"));
    Ok(())
}

fn emit_version_predicate_scalar(
    out: &mut impl WasmOutput,
    value: crate::ValueId,
    lowered: &WasmLoweredFunction<'_>,
    plan: Option<&WasmFunctionPlan>,
    indent: usize,
) -> Result<(), String> {
    let operand = scalar_operand(lowered, value)?;
    if matches!(value_type(operand), MirType::Slice(_)) {
        return Err("WebAssembly version predicate expected a scalar value".into());
    }
    if let Some(plan) = plan {
        emit_wat_paired_scalar_value(out, operand, plan, indent);
    } else {
        emit_wat_value(out, operand, indent);
    }
    Ok(())
}

fn emit_wat_version_interval_end(
    out: &mut impl WasmOutput,
    count: crate::ValueId,
    slice: crate::ValueId,
    element_bytes: u32,
    end_local: &str,
    context: &VersionPredicateSliceContext<'_, '_>,
    indent: usize,
) -> Result<(), String> {
    let pad = " ".repeat(indent);
    emit_version_predicate_scalar(out, count, context.lowered, Some(context.plan), indent)?;
    out.push_str(&format!(
        "{pad}i64.extend_i32_u\n{pad}i64.const {element_bytes}\n{pad}i64.mul\n"
    ));
    emit_wat_version_slice_data_i64(out, slice, context, indent)?;
    out.push_str(&format!(
        "{pad}i64.add\n{pad}local.tee ${end_local}\n{pad}i64.const 4294967295\n{pad}i64.le_u\n"
    ));
    Ok(())
}

fn emit_wat_version_end_le_address(
    out: &mut impl WasmOutput,
    end_local: &str,
    address_slice: crate::ValueId,
    context: &VersionPredicateSliceContext<'_, '_>,
    indent: usize,
) -> Result<(), String> {
    let pad = " ".repeat(indent);
    out.push_str(&format!("{pad}local.get ${end_local}\n"));
    emit_wat_version_slice_data_i64(out, address_slice, context, indent)?;
    out.push_str(&format!("{pad}i64.le_u\n"));
    Ok(())
}

fn emit_wat_version_slice_data_i64(
    out: &mut impl WasmOutput,
    slice: crate::ValueId,
    context: &VersionPredicateSliceContext<'_, '_>,
    indent: usize,
) -> Result<(), String> {
    let operand = scalar_operand(context.lowered, slice)?;
    if !matches!(value_type(operand), MirType::Slice(_)) {
        return Err("WebAssembly version predicate address must be a slice".into());
    }
    let (data, _) = context.plan.slice(operand);
    let pad = " ".repeat(indent);
    out.push_str(&format!("{pad}local.get ${data}\n{pad}i64.extend_i32_u\n"));
    Ok(())
}

fn emit_wat_lowered_instruction(
    out: &mut impl WasmOutput,
    instruction: &super::ir::WasmLoweredInstruction<'_>,
    lowered: &WasmLoweredFunction<'_>,
    layout: &WasmStructLayout,
    paired: Option<&WasmFunctionPlan>,
    special_locals: &WasmSpecialLocals<'_>,
    indent: usize,
) -> Result<(), String> {
    if let KirInstructionKind::VersionPredicate { predicate } = &instruction.source.kind {
        return emit_wat_version_predicate(
            out,
            instruction.source,
            predicate,
            lowered,
            paired,
            special_locals.predicate_scratch,
            indent,
        );
    }
    if is_vector_instruction(&instruction.source.kind) {
        let plan = paired.ok_or_else(|| {
            "WebAssembly vector instructions require the paired slice ABI plan".to_string()
        })?;
        return emit_wat_vector_instruction(
            out,
            instruction.source,
            lowered,
            plan,
            special_locals.vector_names,
            special_locals.memory_cursor_names,
            indent,
        );
    }
    if lowered
        .memory_plan
        .access_by_instruction
        .get(&instruction.source.id)
        .is_some_and(|cursor| special_locals.memory_cursor_names.contains_key(cursor))
    {
        return emit_wat_scalar_memory_cursor_instruction(
            out,
            instruction.source,
            lowered,
            layout,
            paired,
            special_locals.memory_cursor_names,
            indent,
        );
    }
    for leaf in &instruction.leaves {
        if let Some(plan) = paired {
            emit_wat_paired_instruction(out, leaf, layout, plan, indent);
        } else {
            emit_wat_instruction(out, leaf, layout, indent);
        }
    }
    Ok(())
}

fn emit_wat_memarg_offset_instruction(
    out: &mut impl WasmOutput,
    instruction: &super::ir::WasmLoweredInstruction<'_>,
    lowered: &WasmLoweredFunction<'_>,
    layout: &WasmStructLayout,
    paired: Option<&WasmFunctionPlan>,
    indent: usize,
) -> Result<(), String> {
    if paired.is_some() {
        return Err("WebAssembly memarg offset sink does not support paired slice ABI".into());
    }
    let fold = lowered
        .memory_plan
        .memarg_offset_by_instruction
        .get(&instruction.source.id)
        .ok_or_else(|| "WebAssembly memarg offset plan is missing".to_string())?;
    let KirInstructionKind::Load {
        place: source_place,
    } = &instruction.source.kind
    else {
        return Err("WebAssembly memarg offset plan must target a load".into());
    };
    let KirPlace::Field {
        base: source_indexed,
        field_name: source_field,
        type_node: source_type,
        ..
    } = source_place.as_ref()
    else {
        return Err("WebAssembly memarg offset load must target a struct field".into());
    };
    let KirPlace::Index {
        base: source_base,
        index: source_index,
        type_node: source_struct,
        ..
    } = source_indexed.as_ref()
    else {
        return Err("WebAssembly memarg offset load must use a direct pointer index".into());
    };
    let KirPlace::Value {
        value: source_pointer,
        type_node: pointer_type,
        ..
    } = source_base.as_ref()
    else {
        return Err("WebAssembly memarg offset load must use a pointer parameter".into());
    };
    let MirType::Struct(struct_name) = source_struct else {
        return Err("WebAssembly memarg offset base must have a struct element type".into());
    };
    if *source_pointer != fold.base || *source_index != fold.index {
        return Err("WebAssembly memarg offset source values do not match the checked plan".into());
    }
    let MirType::Pointer(pointee) = pointer_type else {
        return Err("WebAssembly memarg offset base must be a pointer".into());
    };
    if pointee.as_ref() != source_struct
        || u32::try_from(layout.size_of(source_struct)).ok() != Some(fold.stride_bytes)
        || u32::try_from(layout.field_offset(struct_name, source_field)).ok()
            != Some(fold.offset_bytes)
        || u32::try_from(layout.size_of(source_type)).ok() != Some(fold.access_bytes)
        || fold.base_alignment != 16
        || fold.stride_bytes % fold.base_alignment != 0
        || fold.offset_bytes == 0
        || fold.offset_bytes >= fold.base_alignment
        || fold.offset_bytes >= fold.stride_bytes
        || fold.memarg_alignment == 0
        || !fold.memarg_alignment.is_power_of_two()
        || fold.memarg_alignment > fold.access_bytes
    {
        return Err("WebAssembly memarg offset plan does not match the source layout".into());
    }
    let [MirInstruction::Load { target, place }] = instruction.leaves.as_slice() else {
        return Err("WebAssembly memarg offset load must lower to one scalar load".into());
    };
    let MirPlace::Field {
        base: residual_place,
        field_name,
        type_node,
    } = place
    else {
        return Err("WebAssembly memarg offset MIR load lost its field base".into());
    };
    if field_name != source_field || type_node != source_type || value_type(target) != source_type {
        return Err("WebAssembly memarg offset MIR load disagrees with source KIR".into());
    }
    let pad = " ".repeat(indent);
    emit_wat_address(out, residual_place, layout, indent);
    out.push_str(&format!(
        "{pad}{}.load offset={} align={}\n{pad}local.set ${}\n",
        wasm_type(value_type(target)),
        fold.offset_bytes,
        fold.memarg_alignment,
        wat_local_name(target)
    ));
    Ok(())
}

fn emit_wat_scalar_memory_cursor_instruction(
    out: &mut impl WasmOutput,
    instruction: &KirInstruction,
    lowered: &WasmLoweredFunction<'_>,
    layout: &WasmStructLayout,
    paired: Option<&WasmFunctionPlan>,
    cursor_names: &BTreeMap<CursorId, String>,
    indent: usize,
) -> Result<(), String> {
    let cursor = lowered
        .memory_plan
        .access_by_instruction
        .get(&instruction.id)
        .ok_or_else(|| "WebAssembly scalar memory cursor plan is missing".to_string())?;
    let cursor_name = cursor_names
        .get(cursor)
        .ok_or_else(|| format!("WebAssembly memory cursor {} has no local", cursor.0))?;
    let cursor_spec = lowered
        .memory_plan
        .cursors
        .iter()
        .find(|candidate| candidate.id == *cursor)
        .ok_or_else(|| format!("WebAssembly memory cursor {} is missing", cursor.0))?;
    let pad = " ".repeat(indent);
    match &instruction.kind {
        KirInstructionKind::Load { .. } => {
            let [result] = instruction.results.as_slice() else {
                return Err("WebAssembly cursor load result is malformed".into());
            };
            let type_node = result.type_node.as_scalar().ok_or_else(|| {
                "WebAssembly cursor load result must have a scalar type".to_string()
            })?;
            validate_cursor_memory_type(type_node, cursor_spec.element_bytes, layout)?;
            let target = scalar_operand(lowered, result.value)?;
            out.push_str(&format!("{pad}local.get ${cursor_name}\n"));
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n",
                wasm_type(type_node),
                layout.align_of(type_node)
            ));
            let local = paired.map_or_else(|| wat_local_name(target), |plan| plan.scalar(target));
            out.push_str(&format!("{pad}local.set ${local}\n"));
        }
        KirInstructionKind::Store { value, .. } => {
            let stored = scalar_operand(lowered, *value)?;
            let type_node = value_type(stored);
            validate_cursor_memory_type(type_node, cursor_spec.element_bytes, layout)?;
            out.push_str(&format!("{pad}local.get ${cursor_name}\n"));
            if let Some(plan) = paired {
                emit_wat_paired_scalar_value(out, stored, plan, indent);
            } else {
                emit_wat_value(out, stored, indent);
            }
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(type_node),
                layout.align_of(type_node)
            ));
        }
        _ => {
            return Err("WebAssembly cursor plan targets a non-scalar memory access".into());
        }
    }
    Ok(())
}

fn validate_cursor_memory_type(
    type_node: &MirType,
    element_bytes: u32,
    layout: &WasmStructLayout,
) -> Result<(), String> {
    if !matches!(type_node, MirType::Primitive(_) | MirType::Pointer(_))
        || u32::try_from(layout.size_of(type_node)).ok() != Some(element_bytes)
        || !matches!(element_bytes, 4 | 8)
    {
        return Err(format!(
            "WebAssembly cursor memory type {type_node:?} does not match {element_bytes}-byte access"
        ));
    }
    Ok(())
}

fn emit_wat_memory_edge_actions(
    out: &mut impl WasmOutput,
    lowered: &WasmLoweredFunction<'_>,
    edge: &WasmLoweredEdge<'_>,
    source: crate::BlockId,
    paired: Option<&WasmFunctionPlan>,
    cursor_names: &BTreeMap<CursorId, String>,
    indent: usize,
) -> Result<(), String> {
    if cursor_names.is_empty() {
        return Ok(());
    }
    let Some(actions) = lowered.memory_plan.edge_actions.get(&(source, edge.arm)) else {
        return Ok(());
    };
    let pad = " ".repeat(indent);
    for action in actions {
        match action {
            WasmMemoryEdgeAction::Initialize {
                cursor,
                base,
                induction_arg_index,
                bias_bytes,
            } => {
                let cursor_name = cursor_names.get(cursor).ok_or_else(|| {
                    format!("WebAssembly memory cursor {} has no local", cursor.0)
                })?;
                let cursor_spec = lowered
                    .memory_plan
                    .cursors
                    .iter()
                    .find(|candidate| candidate.id == *cursor)
                    .ok_or_else(|| format!("WebAssembly memory cursor {} is missing", cursor.0))?;
                let base = match base {
                    CursorOperand::HeaderArgument(index) => {
                        edge.source.args.get(*index).copied().ok_or_else(|| {
                            "WebAssembly memory cursor entry edge omits its base argument"
                                .to_string()
                        })?
                    }
                    CursorOperand::Value(value) => *value,
                };
                let induction = edge
                    .source
                    .args
                    .get(*induction_arg_index)
                    .copied()
                    .ok_or_else(|| {
                        "WebAssembly memory cursor entry edge omits its induction argument"
                            .to_string()
                    })?;
                emit_wat_memory_cursor_base(out, base, lowered, paired, indent)?;
                emit_wat_memory_cursor_index(out, induction, lowered, paired, indent)?;
                out.push_str(&format!(
                    "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                    cursor_spec.element_bytes
                ));
                if *bias_bytes != 0 {
                    out.push_str(&format!("{pad}i32.const {bias_bytes}\n{pad}i32.add\n"));
                }
                out.push_str(&format!("{pad}local.set ${cursor_name}\n"));
            }
            WasmMemoryEdgeAction::Advance {
                cursor,
                delta_bytes,
            } => {
                let cursor_name = cursor_names.get(cursor).ok_or_else(|| {
                    format!("WebAssembly memory cursor {} has no local", cursor.0)
                })?;
                out.push_str(&format!(
                    "{pad}local.get ${cursor_name}\n{pad}i32.const {delta_bytes}\n{pad}i32.add\n{pad}local.set ${cursor_name}\n"
                ));
            }
        }
    }
    Ok(())
}

fn emit_wat_memory_cursor_base(
    out: &mut impl WasmOutput,
    value: crate::ValueId,
    lowered: &WasmLoweredFunction<'_>,
    paired: Option<&WasmFunctionPlan>,
    indent: usize,
) -> Result<(), String> {
    let operand = scalar_operand(lowered, value)?;
    match value_type(operand) {
        MirType::Pointer(_) => {
            if let Some(plan) = paired {
                emit_wat_paired_scalar_value(out, operand, plan, indent);
            } else {
                emit_wat_value(out, operand, indent);
            }
        }
        MirType::Slice(_) => {
            let plan = paired.ok_or_else(|| {
                "WebAssembly slice cursor base requires the paired slice ABI plan".to_string()
            })?;
            let (data, _) = plan.slice(operand);
            out.push_str(&format!("{}local.get ${data}\n", " ".repeat(indent)));
        }
        type_node => {
            return Err(format!(
                "WebAssembly memory cursor base must be a pointer or slice, got {type_node:?}"
            ));
        }
    }
    Ok(())
}

fn emit_wat_memory_cursor_index(
    out: &mut impl WasmOutput,
    value: crate::ValueId,
    lowered: &WasmLoweredFunction<'_>,
    paired: Option<&WasmFunctionPlan>,
    indent: usize,
) -> Result<(), String> {
    let operand = scalar_operand(lowered, value)?;
    if !matches!(
        value_type(operand),
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
    ) {
        return Err("WebAssembly memory cursor induction must be i32 or u32".into());
    }
    if let Some(plan) = paired {
        emit_wat_paired_scalar_value(out, operand, plan, indent);
    } else {
        emit_wat_value(out, operand, indent);
    }
    Ok(())
}

fn is_vector_instruction(kind: &KirInstructionKind) -> bool {
    matches!(
        kind,
        KirInstructionKind::VectorSplat { .. }
            | KirInstructionKind::VectorLoad { .. }
            | KirInstructionKind::VectorStore { .. }
            | KirInstructionKind::VectorBinary { .. }
            | KirInstructionKind::VectorUnary { .. }
            | KirInstructionKind::VectorCompare { .. }
            | KirInstructionKind::VectorSelect { .. }
            | KirInstructionKind::VectorCast { .. }
            | KirInstructionKind::VectorInsert { .. }
            | KirInstructionKind::VectorExtract { .. }
            | KirInstructionKind::VectorReduce { .. }
    )
}

fn emit_wat_vector_instruction(
    out: &mut impl WasmOutput,
    instruction: &KirInstruction,
    lowered: &WasmLoweredFunction<'_>,
    plan: &WasmFunctionPlan,
    vector_names: &BTreeMap<crate::ValueId, String>,
    cursor_names: &BTreeMap<CursorId, String>,
    indent: usize,
) -> Result<(), String> {
    let pad = " ".repeat(indent);
    let address_context = WasmVectorAddressContext {
        lowered,
        plan,
        cursor_names,
    };
    let result = |index: usize| {
        instruction
            .results
            .get(index)
            .map(|result| result.value)
            .ok_or_else(|| "WebAssembly vector instruction has no result".to_string())
    };
    let vector_local = |value: crate::ValueId| {
        vector_names.get(&value).map(String::as_str).ok_or_else(|| {
            format!(
                "WebAssembly vector value {} has no v128 local",
                value.index()
            )
        })
    };
    match &instruction.kind {
        KirInstructionKind::VectorLoad { access, .. } => {
            let target = result(0)?;
            if vector_lane(lowered, target)? != (access.lane, access.lanes) {
                return Err(
                    "WebAssembly vector load result shape does not match its access".into(),
                );
            }
            let element_bytes = lane_element_bytes(access.lane)?;
            emit_wat_vector_address(
                out,
                instruction,
                access,
                &address_context,
                element_bytes,
                indent,
            )?;
            out.push_str(&format!(
                "{pad}{} offset=0 align={}\n{pad}local.set ${}\n",
                vector_load_opcode(access.lane, access.lanes)?,
                access.required_alignment,
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            if vector_lane(lowered, *value)? != (access.lane, access.lanes) {
                return Err(
                    "WebAssembly vector store value shape does not match its access".into(),
                );
            }
            let element_bytes = lane_element_bytes(access.lane)?;
            emit_wat_vector_address(
                out,
                instruction,
                access,
                &address_context,
                element_bytes,
                indent,
            )?;
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*value)?));
            out.push_str(&format!(
                "{pad}{} offset=0 align={}\n",
                vector_store_opcode(access.lane, access.lanes)?,
                access.required_alignment
            ));
        }
        KirInstructionKind::VectorSplat { scalar, .. } => {
            let target = result(0)?;
            let lane = vector_lane(lowered, target)?.0;
            emit_wat_paired_scalar_value(out, scalar_operand(lowered, *scalar)?, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                vector_lane_prefix(lane)?,
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorBinary {
            op, left, right, ..
        } => {
            let target = result(0)?;
            let lane = vector_lane(lowered, target)?.0;
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*left)?));
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*right)?));
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                vector_binary_opcode(lane, *op)?,
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorUnary { op, operand, .. } => {
            let target = result(0)?;
            let lane = vector_lane(lowered, target)?.0;
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*operand)?));
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                vector_unary_opcode(lane, *op)?,
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorCompare {
            op, left, right, ..
        } => {
            let target = result(0)?;
            let (lane, lanes) = vector_lane(lowered, *left)?;
            if vector_lane(lowered, *right)? != (lane, lanes) {
                return Err("WebAssembly vector compare operands have different shapes".into());
            }
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*left)?));
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*right)?));
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                vector_compare_opcode(lane, *op)?,
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => {
            let target = result(0)?;
            let shape = vector_lane(lowered, target)?;
            if vector_lane(lowered, *when_true)? != shape
                || vector_lane(lowered, *when_false)? != shape
            {
                return Err("WebAssembly vector select values have different shapes".into());
            }
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*when_true)?));
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*when_false)?));
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*mask)?));
            out.push_str(&format!(
                "{pad}v128.bitselect\n{pad}local.set ${}\n",
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorCast { op, value, .. } => {
            let target = result(0)?;
            let target_shape = vector_lane(lowered, target)?;
            let source_shape = vector_lane(lowered, *value)?;
            let opcode = vector_cast_opcode(*op, source_shape, target_shape)?;
            out.push_str(&format!("{pad}local.get ${}\n", vector_local(*value)?));
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                vector_local(target)?
            ));
        }
        KirInstructionKind::VectorReduce {
            op,
            vector,
            semantics,
            ..
        } => {
            if *semantics != KirArithmeticSemantics::Modular || instruction.results.len() != 1 {
                return Err(
                    "WebAssembly vector reduction shape or semantics are unsupported".into(),
                );
            }
            let (lane, lanes) = vector_lane(lowered, *vector)?;
            let scalar_opcode = vector_reduction_opcode(*op, lane, lanes)?;
            let target = result(0)?;
            let target_operand = scalar_operand(lowered, target)?;
            let expected_result = match lane {
                KirLaneType::I32 => MirType::Primitive(MirPrimitiveTypeName::I32),
                KirLaneType::U32 => MirType::Primitive(MirPrimitiveTypeName::U32),
                _ => {
                    return Err("WebAssembly vector reduction result lane is unsupported".into());
                }
            };
            if value_type(target_operand) != &expected_result {
                return Err(
                    "WebAssembly vector reduction result type does not match its lane".into(),
                );
            }
            for lane_index in 0..4 {
                out.push_str(&format!("{pad}local.get ${}\n", vector_local(*vector)?));
                out.push_str(&format!("{pad}i32x4.extract_lane {lane_index}\n"));
                if lane_index > 0 {
                    out.push_str(&format!("{pad}{scalar_opcode}\n"));
                }
            }
            out.push_str(&format!(
                "{pad}local.set ${}\n",
                plan.scalar(target_operand)
            ));
        }
        _ => {
            return Err(
                "WebAssembly KIR vector instruction is outside the supported subset".to_string(),
            );
        }
    }
    Ok(())
}

struct WasmVectorAddressContext<'a, 'source> {
    lowered: &'a WasmLoweredFunction<'source>,
    plan: &'a WasmFunctionPlan,
    cursor_names: &'a BTreeMap<CursorId, String>,
}

fn emit_wat_vector_address(
    out: &mut impl WasmOutput,
    instruction: &KirInstruction,
    access: &KirVectorMemoryAccess,
    context: &WasmVectorAddressContext<'_, '_>,
    element_bytes: u32,
    indent: usize,
) -> Result<(), String> {
    let pad = " ".repeat(indent);
    if let Some(cursor) = context
        .lowered
        .memory_plan
        .access_by_instruction
        .get(&instruction.id)
        .filter(|cursor| context.cursor_names.contains_key(cursor))
    {
        let cursor_spec = context
            .lowered
            .memory_plan
            .cursors
            .iter()
            .find(|candidate| candidate.id == *cursor)
            .ok_or_else(|| format!("WebAssembly memory cursor {} is missing", cursor.0))?;
        if cursor_spec.element_bytes != element_bytes {
            return Err("WebAssembly vector cursor stride does not match lane width".into());
        }
        let name = context
            .cursor_names
            .get(cursor)
            .ok_or_else(|| format!("WebAssembly memory cursor {} has no local", cursor.0))?;
        out.push_str(&format!("{pad}local.get ${name}\n"));
        return Ok(());
    }
    let slice = scalar_operand(context.lowered, access.slice)?;
    let (data, _) = context.plan.slice(slice);
    out.push_str(&format!("{pad}local.get ${data}\n"));
    emit_wat_paired_scalar_value(
        out,
        scalar_operand(context.lowered, access.start)?,
        context.plan,
        indent,
    );
    out.push_str(&format!(
        "{pad}i32.const {element_bytes}\n{pad}i32.mul\n{pad}i32.add\n"
    ));
    Ok(())
}

fn vector_lane(
    lowered: &WasmLoweredFunction<'_>,
    value: crate::ValueId,
) -> Result<(KirLaneType, u16), String> {
    match lowered.values.get(&value).map(|typed| typed.source_type) {
        Some(WasmSourceType::Kir(KirValueType::FixedVector { lane, lanes })) => Ok((*lane, *lanes)),
        _ => Err(format!(
            "WebAssembly value {} is not a fixed vector",
            value.index()
        )),
    }
}

fn lane_element_bytes(lane: KirLaneType) -> Result<u32, String> {
    match lane {
        KirLaneType::F64 => Ok(8),
        KirLaneType::I32 | KirLaneType::U32 => Ok(4),
        _ => Err(format!(
            "WebAssembly P6 does not support {lane:?} vector lanes"
        )),
    }
}

fn vector_load_opcode(lane: KirLaneType, lanes: u16) -> Result<&'static str, String> {
    match (lane, lanes) {
        (KirLaneType::F64, 2) | (KirLaneType::I32 | KirLaneType::U32, 4) => Ok("v128.load"),
        (KirLaneType::I32 | KirLaneType::U32, 2) => Ok("v128.load64_zero"),
        _ => Err(format!(
            "WebAssembly does not support a {lane:?}x{lanes} vector load"
        )),
    }
}

fn vector_store_opcode(lane: KirLaneType, lanes: u16) -> Result<&'static str, String> {
    match (lane, lanes) {
        (KirLaneType::F64, 2) | (KirLaneType::I32 | KirLaneType::U32, 4) => Ok("v128.store"),
        _ => Err(format!(
            "WebAssembly does not support a {lane:?}x{lanes} vector store"
        )),
    }
}

fn vector_cast_opcode(
    op: KirVectorCastOp,
    source: (KirLaneType, u16),
    target: (KirLaneType, u16),
) -> Result<&'static str, String> {
    match (op, source, target) {
        (KirVectorCastOp::I32ToF64, (KirLaneType::I32, 2), (KirLaneType::F64, 2)) => {
            Ok("f64x2.convert_low_i32x4_s")
        }
        (KirVectorCastOp::U32ToF64, (KirLaneType::U32, 2), (KirLaneType::F64, 2)) => {
            Ok("f64x2.convert_low_i32x4_u")
        }
        _ => Err(format!(
            "WebAssembly does not support {op:?} from {source:?} to {target:?}"
        )),
    }
}

fn vector_reduction_opcode(
    op: KirVectorReductionOp,
    lane: KirLaneType,
    lanes: u16,
) -> Result<&'static str, String> {
    match (op, lane, lanes) {
        (KirVectorReductionOp::ModularAdd, KirLaneType::I32 | KirLaneType::U32, 4) => Ok("i32.add"),
        (KirVectorReductionOp::ModularMultiply, KirLaneType::I32 | KirLaneType::U32, 4) => {
            Ok("i32.mul")
        }
        _ => Err(format!(
            "WebAssembly SIMD128 does not support {op:?} reduction for {lane:?}x{lanes}"
        )),
    }
}

fn vector_lane_prefix(lane: KirLaneType) -> Result<&'static str, String> {
    match lane {
        KirLaneType::F64 => Ok("f64x2.splat"),
        KirLaneType::I32 | KirLaneType::U32 => Ok("i32x4.splat"),
        _ => Err(format!(
            "WebAssembly P6 does not support {lane:?} vector lanes"
        )),
    }
}

fn vector_binary_opcode(lane: KirLaneType, op: KirVectorBinaryOp) -> Result<&'static str, String> {
    match (lane, op) {
        (KirLaneType::F64, KirVectorBinaryOp::Add) => Ok("f64x2.add"),
        (KirLaneType::F64, KirVectorBinaryOp::Subtract) => Ok("f64x2.sub"),
        (KirLaneType::F64, KirVectorBinaryOp::Multiply) => Ok("f64x2.mul"),
        (KirLaneType::F64, KirVectorBinaryOp::Divide) => Ok("f64x2.div"),
        (KirLaneType::I32 | KirLaneType::U32, KirVectorBinaryOp::Add) => Ok("i32x4.add"),
        (KirLaneType::I32 | KirLaneType::U32, KirVectorBinaryOp::Subtract) => Ok("i32x4.sub"),
        (KirLaneType::I32 | KirLaneType::U32, KirVectorBinaryOp::Multiply) => Ok("i32x4.mul"),
        _ => Err(format!(
            "WebAssembly P6 does not support {op:?} for {lane:?}"
        )),
    }
}

fn vector_unary_opcode(lane: KirLaneType, op: KirVectorUnaryOp) -> Result<&'static str, String> {
    match (lane, op) {
        (KirLaneType::F64, KirVectorUnaryOp::Negate) => Ok("f64x2.neg"),
        (KirLaneType::I32 | KirLaneType::U32, KirVectorUnaryOp::Negate) => Ok("i32x4.neg"),
        _ => Err(format!(
            "WebAssembly P6 does not support {op:?} for {lane:?}"
        )),
    }
}

fn vector_compare_opcode(lane: KirLaneType, op: MirCompareOp) -> Result<&'static str, String> {
    match (lane, op) {
        (KirLaneType::F64, MirCompareOp::Eq) => Ok("f64x2.eq"),
        (KirLaneType::F64, MirCompareOp::Ne) => Ok("f64x2.ne"),
        (KirLaneType::F64, MirCompareOp::Lt) => Ok("f64x2.lt"),
        (KirLaneType::F64, MirCompareOp::Le) => Ok("f64x2.le"),
        (KirLaneType::F64, MirCompareOp::Gt) => Ok("f64x2.gt"),
        (KirLaneType::F64, MirCompareOp::Ge) => Ok("f64x2.ge"),
        (KirLaneType::I32, MirCompareOp::Eq) => Ok("i32x4.eq"),
        (KirLaneType::I32, MirCompareOp::Ne) => Ok("i32x4.ne"),
        (KirLaneType::I32, MirCompareOp::Lt) => Ok("i32x4.lt_s"),
        (KirLaneType::I32, MirCompareOp::Le) => Ok("i32x4.le_s"),
        (KirLaneType::I32, MirCompareOp::Gt) => Ok("i32x4.gt_s"),
        (KirLaneType::I32, MirCompareOp::Ge) => Ok("i32x4.ge_s"),
        (KirLaneType::U32, MirCompareOp::Eq) => Ok("i32x4.eq"),
        (KirLaneType::U32, MirCompareOp::Ne) => Ok("i32x4.ne"),
        (KirLaneType::U32, MirCompareOp::Lt) => Ok("i32x4.lt_u"),
        (KirLaneType::U32, MirCompareOp::Le) => Ok("i32x4.le_u"),
        (KirLaneType::U32, MirCompareOp::Gt) => Ok("i32x4.gt_u"),
        (KirLaneType::U32, MirCompareOp::Ge) => Ok("i32x4.ge_u"),
        _ => Err(format!(
            "WebAssembly does not support {op:?} comparison for {lane:?} lanes"
        )),
    }
}

fn structure_item_target(item: &StructureItem) -> crate::BlockId {
    match item {
        StructureItem::Block(block) => *block,
        StructureItem::Loop { header, .. } => *header,
    }
}

struct StructuredEmission<'a, 'source> {
    lowered: &'a WasmLoweredFunction<'source>,
    structure: &'a StructurePlan,
    layout: &'a WasmStructLayout,
    paired: Option<&'a WasmFunctionPlan>,
    exit_label: &'a str,
    return_local: Option<&'a str>,
    vector_names: &'a BTreeMap<crate::ValueId, String>,
    memory_cursor_names: &'a BTreeMap<CursorId, String>,
    predicate_scratch: Option<&'a VersionPredicateScratch>,
}

impl StructuredEmission<'_, '_> {
    fn emit_region(
        &self,
        out: &mut impl WasmOutput,
        region: &StructureRegion,
        indent: usize,
    ) -> Result<(), String> {
        for item in region.items.iter().skip(1).rev() {
            let target = structure_item_target(item);
            if let Some(label) = self.structure.forward_labels.get(&target) {
                out.push_str(&format!("{}block ${label}\n", " ".repeat(indent)));
            }
        }
        for (index, item) in region.items.iter().enumerate() {
            if index > 0 {
                let target = structure_item_target(item);
                if self.structure.forward_labels.contains_key(&target) {
                    out.push_str(&format!("{}end\n", " ".repeat(indent)));
                }
            }
            match item {
                StructureItem::Block(block_id) => {
                    let block = self
                        .lowered
                        .blocks
                        .iter()
                        .find(|block| block.source.id == *block_id)
                        .expect("planner only references blocks in typed lowering");
                    for instruction in &block.instructions {
                        let special_locals = WasmSpecialLocals {
                            vector_names: self.vector_names,
                            memory_cursor_names: self.memory_cursor_names,
                            predicate_scratch: self.predicate_scratch,
                        };
                        emit_wat_lowered_instruction(
                            out,
                            instruction,
                            self.lowered,
                            self.layout,
                            self.paired,
                            &special_locals,
                            indent + 2,
                        )?;
                    }
                    self.emit_terminator(out, block, indent + 2)?;
                }
                StructureItem::Loop { header, body } => {
                    let label = self
                        .structure
                        .loop_labels
                        .get(header)
                        .expect("planner assigns a label to every natural loop");
                    out.push_str(&format!("{}loop ${label}\n", " ".repeat(indent + 2)));
                    self.emit_region(out, body, indent + 4)?;
                    out.push_str(&format!("{}end\n", " ".repeat(indent + 2)));
                }
            }
        }
        Ok(())
    }

    fn emit_terminator(
        &self,
        out: &mut impl WasmOutput,
        block: &WasmLoweredBlock<'_>,
        indent: usize,
    ) -> Result<(), String> {
        let index = self
            .lowered
            .blocks
            .iter()
            .position(|candidate| candidate.source.id == block.source.id)
            .expect("planner source blocks are retained by typed lowering");
        let view_block = self
            .lowered
            .local_view
            .blocks
            .get(index)
            .expect("local view preserves source block order");
        let pad = " ".repeat(indent);
        match (&block.source.terminator, &view_block.terminator) {
            (KirTerminator::Return { .. }, MirTerminator::Return { value }) => {
                if let Some(value) = value {
                    if let Some(plan) = self.paired {
                        if matches!(value_type(value), MirType::Slice(_)) {
                            emit_wat_paired_slice_value(out, value, plan, indent);
                            out.push_str(&format!(
                                "{pad}local.set ${}\n{pad}local.set ${}\n",
                                plan.return_len, plan.return_data
                            ));
                        } else {
                            emit_wat_paired_scalar_value(out, value, plan, indent);
                            out.push_str(&format!("{pad}local.set ${}\n", plan.return_scalar));
                        }
                    } else {
                        emit_wat_value(out, value, indent);
                        if let Some(return_local) = self.return_local {
                            out.push_str(&format!("{pad}local.set ${return_local}\n"));
                        }
                    }
                }
                out.push_str(&format!("{pad}br ${}\n", self.exit_label));
            }
            (KirTerminator::Jump { .. }, MirTerminator::Jump { .. }) => {
                let edge = block
                    .edges
                    .first()
                    .expect("preflight verifies one lowered edge per jump");
                self.emit_edge(out, edge, block, indent)?;
            }
            (
                KirTerminator::Branch { .. },
                MirTerminator::Branch {
                    condition,
                    then_label: _,
                    else_label: _,
                },
            ) => {
                if let Some(plan) = self.paired {
                    emit_wat_paired_scalar_value(out, condition, plan, indent);
                } else {
                    emit_wat_value(out, condition, indent);
                }
                out.push_str(&format!("{pad}if\n"));
                let then_edge = block
                    .edges
                    .iter()
                    .find(|edge| edge.arm == 0)
                    .expect("preflight verifies the then edge");
                self.emit_edge(out, then_edge, block, indent + 2)?;
                out.push_str(&format!("{pad}else\n"));
                let else_edge = block
                    .edges
                    .iter()
                    .find(|edge| edge.arm == 1)
                    .expect("preflight verifies the else edge");
                self.emit_edge(out, else_edge, block, indent + 2)?;
                out.push_str(&format!("{pad}end\n"));
            }
            _ => unreachable!("typed terminator and leaf view preserve the same CFG"),
        }
        Ok(())
    }

    fn emit_edge(
        &self,
        out: &mut impl WasmOutput,
        edge: &WasmLoweredEdge<'_>,
        source: &WasmLoweredBlock<'_>,
        indent: usize,
    ) -> Result<(), String> {
        emit_wat_memory_edge_actions(
            out,
            self.lowered,
            edge,
            source.source.id,
            self.paired,
            self.memory_cursor_names,
            indent,
        )?;
        for copy in &edge.copies {
            if let Some(plan) = self.paired {
                emit_wat_paired_instruction(out, copy, self.layout, plan, indent);
            } else {
                emit_wat_instruction(out, copy, self.layout, indent);
            }
        }
        let target = self
            .structure
            .branch_targets
            .get(&(source.source.id, edge.arm))
            .expect("planner validates every edge before emission");
        let label = match target {
            BranchTarget::Forward(block) => self.structure.forward_labels.get(block),
            BranchTarget::Loop(block) => self.structure.loop_labels.get(block),
        }
        .expect("planner validates every target label before emission");
        out.push_str(&format!("{}br ${label}\n", " ".repeat(indent)));
        Ok(())
    }
}

pub(super) fn emit_wat_slice_function(
    out: &mut impl WasmOutput,
    function: &MirFunction,
    layout: &WasmStructLayout,
    options: EmitWasmOptions,
) {
    let plan = WasmFunctionPlan::new(function);
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        match plan
            .values
            .get(&format!("param:{}", param.name))
            .expect("param must have physical WASM names")
        {
            WasmPhysicalValue::Scalar(name) => out.push_str(&format!(
                "    (param ${name} {})\n",
                wasm_type(&param.type_node)
            )),
            WasmPhysicalValue::Slice { data, len } => {
                out.push_str(&format!("    (param ${data} i32)\n"));
                out.push_str(&format!("    (param ${len} i32)\n"));
            }
        }
    }
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str("    (result i32 i32)\n"),
        type_node => out.push_str(&format!("    (result {})\n", wasm_type(type_node))),
    }

    for local in &function.locals {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("local:{}", local.name))
                .expect("local must have physical WASM names"),
            &local.type_node,
        );
    }
    for (name, type_node) in collect_temps(function) {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("temp:{name}"))
                .expect("temp must have physical WASM names"),
            &type_node,
        );
    }
    out.push_str(&format!("    (local ${} i32)\n", plan.address_local));

    if function.blocks.len() == 1 {
        for instruction in &function.blocks[0].instructions {
            emit_wat_paired_instruction(out, instruction, layout, &plan, 4);
        }
        emit_wat_paired_terminator(out, &function.blocks[0].terminator, None, &plan, 4);
    } else if let Some(loop_context) = (options.opt_level >= 3)
        .then(|| detect_simple_wasm_while(function))
        .flatten()
    {
        emit_structured_wasm_while_paired(out, &loop_context, layout, &plan);
    } else {
        emit_dispatched_wasm_function_paired(out, function, layout, &plan);
    }
    out.push_str("  )\n");
}

pub(super) fn emit_wat_physical_local(
    out: &mut impl WasmOutput,
    value: &WasmPhysicalValue,
    type_node: &MirType,
) {
    match value {
        WasmPhysicalValue::Scalar(name) => {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(type_node)));
        }
        WasmPhysicalValue::Slice { data, len } => {
            out.push_str(&format!("    (local ${data} i32)\n"));
            out.push_str(&format!("    (local ${len} i32)\n"));
        }
    }
}

pub(super) fn emit_structured_wasm_while_paired(
    out: &mut impl WasmOutput,
    loop_context: &StructuredWasmWhile<'_>,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
) {
    for instruction in &loop_context.entry.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 4);
    }
    out.push_str(&format!("    block ${}\n", loop_context.exit_label));
    out.push_str(&format!("      loop ${}\n", loop_context.loop_label));
    for instruction in &loop_context.header.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 8);
    }
    let MirTerminator::Branch { condition, .. } = &loop_context.header.terminator else {
        unreachable!("structured while header is always a branch")
    };
    emit_wat_paired_scalar_value(out, condition, plan, 8);
    out.push_str("        i32.eqz\n");
    out.push_str(&format!("        br_if ${}\n", loop_context.exit_label));
    for instruction in &loop_context.body.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 8);
    }
    out.push_str(&format!("        br ${}\n", loop_context.loop_label));
    out.push_str("      end\n    end\n");
    for instruction in &loop_context.exit.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 4);
    }
    emit_wat_paired_terminator(out, &loop_context.exit.terminator, None, plan, 4);
}

pub(super) fn emit_dispatched_wasm_function_paired(
    out: &mut impl WasmOutput,
    function: &MirFunction,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
) {
    out.push_str(&format!("    (local ${} i32)\n", plan.block_local));
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => {
            out.push_str(&format!("    (local ${} i32)\n", plan.return_data));
            out.push_str(&format!("    (local ${} i32)\n", plan.return_len));
        }
        type_node => out.push_str(&format!(
            "    (local ${} {})\n",
            plan.return_scalar,
            wasm_type(type_node)
        )),
    }
    out.push_str("    i32.const 0\n");
    out.push_str(&format!("    local.set ${}\n", plan.block_local));
    out.push_str("    block $ik_exit\n      loop $ik_dispatch\n");
    for index in 0..function.blocks.len() {
        out.push_str(&format!(
            "{}block $ik_case{index}\n",
            " ".repeat(8 + index * 2)
        ));
    }
    let case_labels = (0..function.blocks.len())
        .map(|index| format!("$ik_case{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dispatch_indent = " ".repeat(8 + function.blocks.len() * 2);
    out.push_str(&format!(
        "{dispatch_indent}local.get ${}\n{dispatch_indent}br_table {case_labels} $ik_case0\n",
        plan.block_local
    ));
    for index in (0..function.blocks.len()).rev() {
        let block_indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(block_indent)));
        let block = &function.blocks[index];
        for instruction in &block.instructions {
            emit_wat_paired_instruction(out, instruction, layout, plan, block_indent);
        }
        emit_wat_paired_terminator(out, &block.terminator, Some(function), plan, block_indent);
    }
    out.push_str("      end\n    end\n");
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str(&format!(
            "    local.get ${}\n    local.get ${}\n",
            plan.return_data, plan.return_len
        )),
        _ => out.push_str(&format!("    local.get ${}\n", plan.return_scalar)),
    }
}

pub(super) fn emit_wat_paired_instruction(
    out: &mut impl WasmOutput,
    instruction: &MirInstruction,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match instruction {
        MirInstruction::ConstInt { target, value } => out.push_str(&format!(
            "{pad}{}.const {value}\n{pad}local.set ${}\n",
            wasm_type(value_type(target)),
            plan.scalar(target)
        )),
        MirInstruction::ConstFloat { target, value } => out.push_str(&format!(
            "{pad}f64.const {value}\n{pad}local.set ${}\n",
            plan.scalar(target)
        )),
        MirInstruction::ConstBool { target, value } => out.push_str(&format!(
            "{pad}i32.const {}\n{pad}local.set ${}\n",
            if *value { 1 } else { 0 },
            plan.scalar(target)
        )),
        MirInstruction::Move { target, value }
            if matches!(value_type(target), MirType::Slice(_)) =>
        {
            let (target_data, target_len) = plan.slice(target);
            let (value_data, value_len) = plan.slice(value);
            out.push_str(&format!(
                "{pad}local.get ${value_data}\n{pad}local.set ${target_data}\n\
                 {pad}local.get ${value_len}\n{pad}local.set ${target_len}\n"
            ));
        }
        MirInstruction::Move { target, value } => {
            emit_wat_paired_scalar_value(out, value, plan, indent);
            out.push_str(&format!("{pad}local.set ${}\n", plan.scalar(target)));
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_paired_scalar_value(out, left, plan, indent);
            emit_wat_paired_scalar_value(out, right, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_binary_instruction(*op, value_type(left)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => emit_wat_paired_unary(out, *op, operand, target, plan, indent),
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_paired_scalar_value(out, left, plan, indent);
            emit_wat_paired_scalar_value(out, right, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_compare_instruction(*op, value_type(left)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Cast { target, op, value } => {
            emit_wat_paired_scalar_value(out, value, plan, indent);
            let opcode = match op {
                MirCastOp::I32ToF64 => "f64.convert_i32_s",
                MirCastOp::U32ToF64 => "f64.convert_i32_u",
            };
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::Address { target, place } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!("{pad}local.set ${}\n", plan.scalar(target)));
        }
        MirInstruction::Load { target, place }
            if matches!(value_type(target), MirType::Slice(_)) =>
        {
            let (data, len) = plan.slice(target);
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}local.set ${}\n\
                 {pad}local.get ${}\n{pad}i32.load offset=0 align=4\n{pad}local.set ${data}\n\
                 {pad}local.get ${}\n{pad}i32.load offset=4 align=4\n{pad}local.set ${len}\n",
                plan.address_local, plan.address_local, plan.address_local
            ));
        }
        MirInstruction::Load { target, place } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                layout.align_of(value_type(target)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Store { place, value }
            if matches!(value_type(value), MirType::Slice(_)) =>
        {
            let (data, len) = plan.slice(value);
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}local.set ${}\n\
                 {pad}local.get ${}\n{pad}local.get ${data}\n{pad}i32.store offset=0 align=4\n\
                 {pad}local.get ${}\n{pad}local.get ${len}\n{pad}i32.store offset=4 align=4\n",
                plan.address_local, plan.address_local, plan.address_local
            ));
        }
        MirInstruction::Store { place, value } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            emit_wat_paired_scalar_value(out, value, plan, indent);
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(value_type(value)),
                layout.align_of(value_type(value))
            ));
        }
        MirInstruction::MakeSlice { target, data, len } => {
            let (target_data, target_len) = plan.slice(target);
            emit_wat_paired_scalar_value(out, data, plan, indent);
            out.push_str(&format!("{pad}local.set ${target_data}\n"));
            emit_wat_paired_scalar_value(out, len, plan, indent);
            out.push_str(&format!("{pad}local.set ${target_len}\n"));
        }
        MirInstruction::SliceData { target, slice } => {
            let (data, _) = plan.slice(slice);
            out.push_str(&format!(
                "{pad}local.get ${data}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::SliceLen { target, slice } => {
            let (_, len) = plan.slice(slice);
            out.push_str(&format!(
                "{pad}local.get ${len}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let (target_data, target_len) = plan.slice(target);
            let (slice_data, _) = plan.slice(slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("subslice source must be a slice")
            };
            out.push_str(&format!("{pad}local.get ${slice_data}\n"));
            emit_wat_paired_scalar_value(out, start, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n{pad}local.set ${target_data}\n",
                layout.size_of(element_type)
            ));
            emit_wat_paired_scalar_value(out, end, plan, indent);
            emit_wat_paired_scalar_value(out, start, plan, indent);
            out.push_str(&format!("{pad}i32.sub\n{pad}local.set ${target_len}\n"));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            for arg in args {
                if matches!(value_type(arg), MirType::Slice(_)) {
                    emit_wat_paired_slice_value(out, arg, plan, indent);
                } else {
                    emit_wat_paired_scalar_value(out, arg, plan, indent);
                }
            }
            out.push_str(&format!("{pad}call ${function_name}\n"));
            if let Some(target) = target {
                match plan.value(target) {
                    WasmPhysicalValue::Scalar(name) => {
                        out.push_str(&format!("{pad}local.set ${name}\n"));
                    }
                    WasmPhysicalValue::Slice { data, len } => {
                        out.push_str(&format!("{pad}local.set ${len}\n{pad}local.set ${data}\n"))
                    }
                }
            }
        }
        MirInstruction::RuntimeCall { .. } => {
            unreachable!("runtime calls must be rejected before WebAssembly emission")
        }
    }
}

pub(super) fn emit_wat_paired_scalar_value(
    out: &mut impl WasmOutput,
    value: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match value {
        MirValue::Param { .. } | MirValue::Local { .. } | MirValue::Temp { .. } => {
            out.push_str(&format!("{pad}local.get ${}\n", plan.scalar(value)));
        }
        MirValue::ConstInt { text, type_node } => {
            out.push_str(&format!("{pad}{}.const {text}\n", wasm_type(type_node)));
        }
        MirValue::ConstFloat { text, .. } => out.push_str(&format!("{pad}f64.const {text}\n")),
        MirValue::ConstBool { value, .. } => {
            out.push_str(&format!("{pad}i32.const {}\n", if *value { 1 } else { 0 }))
        }
    }
}

pub(super) fn emit_wat_paired_slice_value(
    out: &mut impl WasmOutput,
    value: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    let (data, len) = plan.slice(value);
    out.push_str(&format!("{pad}local.get ${data}\n{pad}local.get ${len}\n"));
}

pub(super) fn emit_wat_paired_unary(
    out: &mut impl WasmOutput,
    op: MirUnaryOp,
    operand: &MirValue,
    target: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match op {
        MirUnaryOp::Not => {
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}i32.eqz\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirUnaryOp::Neg if is_f64_type(value_type(operand)) => {
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}f64.neg\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirUnaryOp::Neg => {
            out.push_str(&format!(
                "{pad}{}.const 0\n",
                wasm_type(value_type(operand))
            ));
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}{}.sub\n{pad}local.set ${}\n",
                wasm_type(value_type(operand)),
                plan.scalar(target)
            ));
        }
    }
}

pub(super) fn emit_wat_paired_address(
    out: &mut impl WasmOutput,
    place: &MirPlace,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => match plan.place_value(place) {
            WasmPhysicalValue::Scalar(name) => {
                out.push_str(&format!("{pad}local.get ${name}\n"));
            }
            WasmPhysicalValue::Slice { .. } => {
                panic!("a logical slice local is not directly addressable")
            }
        },
        MirPlace::Deref { pointer, .. } => emit_wat_paired_scalar_value(out, pointer, plan, indent),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("WAT index base must be pointer");
            };
            emit_wat_paired_address(out, base, layout, plan, indent);
            emit_wat_paired_scalar_value(out, index, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            let (data, _) = plan.slice(slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("slice index base must be a slice")
            };
            out.push_str(&format!("{pad}local.get ${data}\n"));
            emit_wat_paired_scalar_value(out, index, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("WAT field base must be struct");
            };
            emit_wat_paired_address(out, base, layout, plan, indent);
            let offset = layout.field_offset(struct_name, field_name);
            if offset != 0 {
                out.push_str(&format!("{pad}i32.const {offset}\n{pad}i32.add\n"));
            }
        }
    }
}

pub(super) fn emit_wat_paired_terminator(
    out: &mut impl WasmOutput,
    terminator: &MirTerminator,
    function: Option<&MirFunction>,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                if matches!(value_type(value), MirType::Slice(_)) {
                    emit_wat_paired_slice_value(out, value, plan, indent);
                    if function.is_some() {
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}local.set ${}\n{pad}br $ik_exit\n",
                            plan.return_len, plan.return_data
                        ));
                    } else {
                        out.push_str(&format!("{pad}return\n"));
                    }
                } else {
                    emit_wat_paired_scalar_value(out, value, plan, indent);
                    if function.is_some() {
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}br $ik_exit\n",
                            plan.return_scalar
                        ));
                    } else {
                        out.push_str(&format!("{pad}return\n"));
                    }
                }
            } else if function.is_some() {
                out.push_str(&format!("{pad}br $ik_exit\n"));
            } else {
                out.push_str(&format!("{pad}return\n"));
            }
        }
        MirTerminator::Jump { label } => {
            let index = block_index(function.expect("dispatcher function"), label);
            out.push_str(&format!(
                "{pad}i32.const {index}\n{pad}local.set ${}\n{pad}br $ik_dispatch\n",
                plan.block_local
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let function = function.expect("dispatcher function");
            emit_wat_paired_scalar_value(out, condition, plan, indent);
            out.push_str(&format!(
                "{pad}if\n{pad}  i32.const {}\n{pad}  local.set ${}\n{pad}else\n{pad}  i32.const {}\n{pad}  local.set ${}\n{pad}end\n{pad}br $ik_dispatch\n",
                block_index(function, then_label),
                plan.block_local,
                block_index(function, else_label),
                plan.block_local
            ));
        }
    }
}

pub(super) struct StructuredWasmWhile<'a> {
    entry: &'a MirBlock,
    header: &'a MirBlock,
    body: &'a MirBlock,
    exit: &'a MirBlock,
    loop_label: String,
    exit_label: String,
}

pub(super) fn detect_simple_wasm_while(function: &MirFunction) -> Option<StructuredWasmWhile<'_>> {
    if function.blocks.len() != 4 {
        return None;
    }

    let entry = function.blocks.first()?;
    let MirTerminator::Jump {
        label: header_label,
    } = &entry.terminator
    else {
        return None;
    };

    let header = wasm_block_by_label(function, header_label)?;
    let MirTerminator::Branch {
        then_label,
        else_label,
        ..
    } = &header.terminator
    else {
        return None;
    };

    let body = wasm_block_by_label(function, then_label)?;
    let exit = wasm_block_by_label(function, else_label)?;
    if !matches!(&body.terminator, MirTerminator::Jump { label } if label == &header.label)
        || !matches!(exit.terminator, MirTerminator::Return { .. })
    {
        return None;
    }

    let matched_labels = [
        entry.label.as_str(),
        header.label.as_str(),
        body.label.as_str(),
        exit.label.as_str(),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    if matched_labels.len() != function.blocks.len() {
        return None;
    }

    let mut used_names = collect_wasm_function_names(function);
    let loop_label = unique_wasm_internal_name("ik_loop", &mut used_names);
    let exit_label = unique_wasm_internal_name("ik_exit", &mut used_names);
    Some(StructuredWasmWhile {
        entry,
        header,
        body,
        exit,
        loop_label,
        exit_label,
    })
}

pub(super) fn wasm_block_by_label<'a>(
    function: &'a MirFunction,
    label: &str,
) -> Option<&'a MirBlock> {
    function.blocks.iter().find(|block| block.label == label)
}

pub(super) fn emit_structured_wasm_while(
    out: &mut impl WasmOutput,
    loop_context: &StructuredWasmWhile<'_>,
    layout: &WasmStructLayout,
) {
    for instruction in &loop_context.entry.instructions {
        emit_wat_instruction(out, instruction, layout, 4);
    }
    out.push_str(&format!("    block ${}\n", loop_context.exit_label));
    out.push_str(&format!("      loop ${}\n", loop_context.loop_label));

    for instruction in &loop_context.header.instructions {
        emit_wat_instruction(out, instruction, layout, 8);
    }
    let MirTerminator::Branch { condition, .. } = &loop_context.header.terminator else {
        unreachable!("structured while header is always a branch")
    };
    emit_wat_value(out, condition, 8);
    out.push_str("        i32.eqz\n");
    out.push_str(&format!("        br_if ${}\n", loop_context.exit_label));

    for instruction in &loop_context.body.instructions {
        emit_wat_instruction(out, instruction, layout, 8);
    }
    out.push_str(&format!("        br ${}\n", loop_context.loop_label));
    out.push_str("      end\n");
    out.push_str("    end\n");

    for instruction in &loop_context.exit.instructions {
        emit_wat_instruction(out, instruction, layout, 4);
    }
    emit_wat_terminator(out, &loop_context.exit.terminator, None, 4);
}

pub(super) fn emit_dispatched_wasm_function(
    out: &mut impl WasmOutput,
    function: &MirFunction,
    layout: &WasmStructLayout,
) {
    out.push_str("    (local $ik_bb i32)\n");
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (local $ik_ret {})\n",
            wasm_type(&function.return_type)
        ));
    }
    out.push_str("    i32.const 0\n");
    out.push_str("    local.set $ik_bb\n");
    out.push_str("    block $ik_exit\n");
    out.push_str("      loop $ik_dispatch\n");
    for index in 0..function.blocks.len() {
        out.push_str(&format!(
            "{}block $ik_case{index}\n",
            " ".repeat(8 + index * 2)
        ));
    }
    let case_labels = (0..function.blocks.len())
        .map(|index| format!("$ik_case{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dispatch_indent = " ".repeat(8 + function.blocks.len() * 2);
    out.push_str(&format!("{dispatch_indent}local.get $ik_bb\n"));
    out.push_str(&format!(
        "{dispatch_indent}br_table {case_labels} $ik_case0\n"
    ));
    for index in (0..function.blocks.len()).rev() {
        let block_indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(block_indent)));
        let block = &function.blocks[index];
        for instruction in &block.instructions {
            emit_wat_instruction(out, instruction, layout, block_indent);
        }
        emit_wat_terminator(out, &block.terminator, Some(function), block_indent);
    }
    out.push_str("      end\n");
    out.push_str("    end\n");
    if !matches!(function.return_type, MirType::Void) {
        out.push_str("    local.get $ik_ret\n");
    }
}

pub(super) fn emit_wat_instruction(
    out: &mut impl WasmOutput,
    instruction: &MirInstruction,
    layout: &WasmStructLayout,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            out.push_str(&format!(
                "{pad}{}.const {value}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                wat_local_name(target)
            ));
        }
        MirInstruction::ConstFloat { target, value } => {
            out.push_str(&format!(
                "{pad}f64.const {value}\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirInstruction::ConstBool { target, value } => {
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}local.set ${}\n",
                if *value { 1 } else { 0 },
                wat_local_name(target)
            ));
        }
        MirInstruction::Move { target, value } => {
            emit_wat_value(out, value, indent);
            out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_value(out, left, indent);
            emit_wat_value(out, right, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_binary_instruction(*op, value_type(left)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            emit_wat_unary(out, *op, operand, target, indent);
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_value(out, left, indent);
            emit_wat_value(out, right, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_compare_instruction(*op, value_type(left)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Cast { target, op, value } => {
            emit_wat_value(out, value, indent);
            let opcode = match op {
                MirCastOp::I32ToF64 => "f64.convert_i32_s",
                MirCastOp::U32ToF64 => "f64.convert_i32_u",
            };
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirInstruction::Address { target, place } => {
            emit_wat_address(out, place, layout, indent);
            out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
        }
        MirInstruction::Load { target, place } => {
            emit_wat_address(out, place, layout, indent);
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                layout.align_of(value_type(target)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Store { place, value } => {
            emit_wat_address(out, place, layout, indent);
            emit_wat_value(out, value, indent);
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(value_type(value)),
                layout.align_of(value_type(value))
            ));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            for arg in args {
                emit_wat_value(out, arg, indent);
            }
            out.push_str(&format!("{pad}call ${function_name}\n"));
            if let Some(target) = target {
                out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
            }
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::RuntimeCall { .. } => {
            unreachable!("slice/runtime functions require a validated artifact plan")
        }
    }
}

pub(super) fn emit_wat_terminator(
    out: &mut impl WasmOutput,
    terminator: &MirTerminator,
    function: Option<&MirFunction>,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                emit_wat_value(out, value, indent);
                if function.is_some() {
                    out.push_str(&format!("{pad}local.set $ik_ret\n{pad}br $ik_exit\n"));
                } else {
                    out.push_str(&format!("{pad}return\n"));
                }
            } else if function.is_some() {
                out.push_str(&format!("{pad}br $ik_exit\n"));
            } else {
                out.push_str(&format!("{pad}return\n"));
            }
        }
        MirTerminator::Jump { label } => {
            let index = block_index(function.expect("dispatcher function"), label);
            out.push_str(&format!(
                "{pad}i32.const {index}\n{pad}local.set $ik_bb\n{pad}br $ik_dispatch\n"
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let function = function.expect("dispatcher function");
            emit_wat_value(out, condition, indent);
            out.push_str(&format!(
                "{pad}if\n{pad}  i32.const {}\n{pad}  local.set $ik_bb\n{pad}else\n{pad}  i32.const {}\n{pad}  local.set $ik_bb\n{pad}end\n{pad}br $ik_dispatch\n",
                block_index(function, then_label),
                block_index(function, else_label)
            ));
        }
    }
}

pub(super) fn emit_wat_value(out: &mut impl WasmOutput, value: &MirValue, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => {
            out.push_str(&format!("{pad}local.get ${name}\n"));
        }
        MirValue::ConstInt { text, type_node } => {
            out.push_str(&format!("{pad}{}.const {text}\n", wasm_type(type_node)));
        }
        MirValue::ConstFloat { text, .. } => {
            out.push_str(&format!("{pad}f64.const {text}\n"));
        }
        MirValue::ConstBool { value, .. } => {
            out.push_str(&format!("{pad}i32.const {}\n", if *value { 1 } else { 0 }));
        }
    }
}

pub(super) fn emit_wat_unary(
    out: &mut impl WasmOutput,
    op: MirUnaryOp,
    operand: &MirValue,
    target: &MirValue,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match op {
        MirUnaryOp::Not => {
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}i32.eqz\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirUnaryOp::Neg if is_f64_type(value_type(operand)) => {
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}f64.neg\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirUnaryOp::Neg => {
            out.push_str(&format!(
                "{pad}{}.const 0\n",
                wasm_type(value_type(operand))
            ));
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}{}.sub\n{pad}local.set ${}\n",
                wasm_type(value_type(operand)),
                wat_local_name(target)
            ));
        }
    }
}

pub(super) fn wat_local_name(value: &MirValue) -> &str {
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => name,
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("WAT locals cannot be MIR constants")
        }
    }
}

pub(super) fn emit_wat_address(
    out: &mut impl WasmOutput,
    place: &MirPlace,
    layout: &WasmStructLayout,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match place {
        MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => {
            out.push_str(&format!("{pad}local.get ${name}\n"));
        }
        MirPlace::Deref { pointer, .. } => emit_wat_value(out, pointer, indent),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("WAT index base must be pointer");
            };
            emit_wat_address(out, base, layout, indent);
            emit_wat_value(out, index, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::SliceIndex { .. } => {
            unreachable!("slice functions must use the paired WAT emitter")
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("WAT field base must be struct");
            };
            emit_wat_address(out, base, layout, indent);
            let offset = layout.field_offset(struct_name, field_name);
            if offset != 0 {
                out.push_str(&format!("{pad}i32.const {offset}\n{pad}i32.add\n"));
            }
        }
    }
}

pub(super) fn wat_binary_instruction(op: MirBinaryOp, type_node: &MirType) -> String {
    if is_f64_type(type_node) {
        return match op {
            MirBinaryOp::Add => "f64.add".to_string(),
            MirBinaryOp::Sub => "f64.sub".to_string(),
            MirBinaryOp::Mul => "f64.mul".to_string(),
            MirBinaryOp::Div => "f64.div".to_string(),
            MirBinaryOp::Mod => panic!("WAT backend does not support f64 modulo"),
        };
    }
    let wasm = wasm_type(type_node);
    match op {
        MirBinaryOp::Add => format!("{wasm}.add"),
        MirBinaryOp::Sub => format!("{wasm}.sub"),
        MirBinaryOp::Mul => format!("{wasm}.mul"),
        MirBinaryOp::Div if is_unsigned_integer_type(type_node) => format!("{wasm}.div_u"),
        MirBinaryOp::Div => format!("{wasm}.div_s"),
        MirBinaryOp::Mod if is_unsigned_integer_type(type_node) => format!("{wasm}.rem_u"),
        MirBinaryOp::Mod => format!("{wasm}.rem_s"),
    }
}

pub(super) fn wat_compare_instruction(op: MirCompareOp, type_node: &MirType) -> String {
    if is_f64_type(type_node) {
        return match op {
            MirCompareOp::Eq => "f64.eq",
            MirCompareOp::Ne => "f64.ne",
            MirCompareOp::Lt => "f64.lt",
            MirCompareOp::Le => "f64.le",
            MirCompareOp::Gt => "f64.gt",
            MirCompareOp::Ge => "f64.ge",
        }
        .to_string();
    }
    let wasm = wasm_type(type_node);
    match op {
        MirCompareOp::Eq => format!("{wasm}.eq"),
        MirCompareOp::Ne => format!("{wasm}.ne"),
        MirCompareOp::Lt if is_unsigned_integer_type(type_node) => format!("{wasm}.lt_u"),
        MirCompareOp::Lt => format!("{wasm}.lt_s"),
        MirCompareOp::Le if is_unsigned_integer_type(type_node) => format!("{wasm}.le_u"),
        MirCompareOp::Le => format!("{wasm}.le_s"),
        MirCompareOp::Gt if is_unsigned_integer_type(type_node) => format!("{wasm}.gt_u"),
        MirCompareOp::Gt => format!("{wasm}.gt_s"),
        MirCompareOp::Ge if is_unsigned_integer_type(type_node) => format!("{wasm}.ge_u"),
        MirCompareOp::Ge => format!("{wasm}.ge_s"),
    }
}

pub(super) fn wasm_type(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => "i32",
        MirType::Slice(_) => unreachable!("logical slices must be lowered to paired WASM values"),
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => "i64",
        MirType::Primitive(MirPrimitiveTypeName::F64) => "f64",
        MirType::Struct(_) => panic!("struct values are not WASM scalar values"),
        MirType::Void => panic!("void is not a WASM scalar value"),
    }
}

pub(super) fn block_index(function: &MirFunction, label: &str) -> usize {
    function
        .blocks
        .iter()
        .position(|block| block.label == label)
        .unwrap_or_else(|| panic!("unknown WAT block label {label}"))
}
