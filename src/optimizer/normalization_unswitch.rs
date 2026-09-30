//! Strict WASM normalization-loop unswitching.
//!
//! The original loop remains intact as the scalar fallback. A nontrapping,
//! profile-priced length gate selects a new two-block loop that contains only
//! the original `range != +0` arm. The `range == +0` edge always enters the
//! original loop, whose selected body stores positive zero without loading the
//! input. SIMD discovery and its alias/range guards run later.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, CandidateBudgetCharge, CanonicalLoopDescriptor, FunctionId, InstructionId,
    KirAlignmentClass, KirArithmeticSemantics, KirBlock, KirCostKey, KirCostSemantics,
    KirEffectKind, KirFunction, KirInstruction, KirInstructionKind, KirLaneType,
    KirOperationAvailability, KirPlace, KirProfileOperation, KirResult, KirTargetProfile,
    KirTerminator, KirValueType, KirVerifiedProgramState, LoopId, MemoryRegionId, MemoryVersionId,
    MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, TransactionCheckError, ValueId,
    analyze_canonical_loops, kir_function_units, validate_kir_module,
};

const MAX_SOURCE_BLOCKS: usize = 5;
const MAX_FUNCTION_GROWTH: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationUnswitchCandidate {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationUnswitchPlan {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
    pub preheader: BlockId,
    pub zero_arm: BlockId,
    pub false_arm: BlockId,
    pub latch: BlockId,
    pub exit: BlockId,
    pub pre_state_digest: String,
    pub profile_digest: String,
    pub minimum_trip: u32,
    pub dispatch: BlockId,
    pub fast_preheader: BlockId,
    pub fast_header: BlockId,
    pub fast_body: BlockId,
    pub fast_exit: BlockId,
    pub dispatch_instructions: [InstructionId; 3],
    pub range_instructions: [InstructionId; 2],
    pub fast_header_instruction_mapping: Vec<(InstructionId, InstructionId)>,
    pub fast_body_instruction_mapping: Vec<(InstructionId, InstructionId)>,
    pub fast_counter_update: InstructionId,
    pub fast_counter_value: ValueId,
    pub before_units: u32,
    pub after_units: u32,
}

#[derive(Debug, Clone)]
pub struct PreparedNormalizationUnswitch {
    pub trial: KirVerifiedProgramState,
    pub plan: NormalizationUnswitchPlan,
    pub charge: CandidateBudgetCharge,
}

#[derive(Debug, Clone)]
struct SourceShape {
    descriptor: CanonicalLoopDescriptor,
    preheader: BlockId,
    zero: BlockId,
    false_arm: BlockId,
    latch: BlockId,
    exit: BlockId,
    input_slot: usize,
    range_slot: usize,
    induction_slot: usize,
    one: ValueId,
    exit_edge: crate::KirEdge,
    incoming: crate::KirEdge,
}

#[derive(Debug, Clone)]
struct CheckerSource {
    descriptor: CanonicalLoopDescriptor,
    preheader: BlockId,
    zero: BlockId,
    false_arm: BlockId,
    latch: BlockId,
    exit: BlockId,
    input_slot: usize,
    induction_slot: usize,
    incoming: crate::KirEdge,
    exit_edge: crate::KirEdge,
    range_value: ValueId,
    one: ValueId,
}

#[derive(Debug)]
struct ClonedSequence {
    instruction_mapping: Vec<(InstructionId, InstructionId)>,
    memories: BTreeMap<MemoryVersionId, MemoryVersionId>,
}

#[derive(Debug)]
struct CheckedClone {
    values: BTreeMap<ValueId, ValueId>,
    memories: BTreeMap<MemoryRegionId, MemoryVersionId>,
    next_effect_order: u32,
}

fn f64_type() -> KirValueType {
    MirType::Primitive(MirPrimitiveTypeName::F64).into()
}

fn u32_type() -> KirValueType {
    MirType::Primitive(MirPrimitiveTypeName::U32).into()
}

fn bool_type() -> KirValueType {
    MirType::Primitive(MirPrimitiveTypeName::Bool).into()
}

fn block(function: &KirFunction, id: BlockId) -> Option<&KirBlock> {
    function.blocks.iter().find(|candidate| candidate.id == id)
}

fn value_type(function: &KirFunction, value: ValueId) -> Option<KirValueType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| param.type_node.clone().into())
        .or_else(|| {
            function.blocks.iter().find_map(|candidate| {
                candidate
                    .params
                    .iter()
                    .find(|param| param.value == value)
                    .map(|param| param.type_node.clone())
                    .or_else(|| {
                        candidate
                            .instructions
                            .iter()
                            .flat_map(|instruction| &instruction.results)
                            .find(|result| result.value == value)
                            .map(|result| result.type_node.clone())
                    })
            })
        })
}

fn definition(function: &KirFunction, value: ValueId) -> Option<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|candidate| &candidate.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
}

fn result_of(instruction: &KirInstruction, ty: &KirValueType) -> Option<ValueId> {
    (instruction.results.len() == 1 && instruction.results[0].type_node == *ty)
        .then(|| instruction.results[0].value)
}

fn parameter_slot(block: &KirBlock, slot: &str, ty: &KirValueType) -> Option<usize> {
    let mut matches = block
        .params
        .iter()
        .enumerate()
        .filter(|(_, param)| param.slot == slot && param.type_node == *ty)
        .map(|(index, _)| index);
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn constant_u32(function: &KirFunction, value: ValueId) -> Option<u32> {
    if value_type(function, value)? != u32_type() {
        return None;
    }
    match &definition(function, value)?.kind {
        KirInstructionKind::ConstInt { value } => value.parse().ok(),
        _ => None,
    }
}

fn no_effect(instruction: &KirInstruction) -> bool {
    instruction.memory.is_none() && instruction.effect.is_none()
}

/// Reconstructs the exact, strict output-loop pattern from the source KIR.
/// This deliberately rejects loops with extra blocks, effects, exits, guards,
/// changed bound operands, or a non-identical `maximum - minimum` range.
fn recognize_source(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
) -> Option<SourceShape> {
    if !descriptor.innermost
        || !descriptor.lcssa
        || descriptor.preheader.is_none()
        || descriptor.exits.len() != 1
        || descriptor.blocks.len() != MAX_SOURCE_BLOCKS
    {
        return None;
    }
    let preheader_id = descriptor.preheader?;
    let preheader = block(function, preheader_id)?;
    let header = block(function, descriptor.header)?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return None;
    };
    if incoming.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || !edge_forwards_memory_by_region(preheader, incoming, header)
        || preheader.instructions.len() != 1
        || descriptor.latch.is_none()
    {
        return None;
    }

    let input_slot = parameter_slot(
        header,
        "input",
        &KirValueType::Scalar(MirType::Slice(Box::new(MirType::Primitive(
            MirPrimitiveTypeName::F64,
        )))),
    )?;
    let output_slot = parameter_slot(
        header,
        "out",
        &KirValueType::Scalar(MirType::Slice(Box::new(MirType::Primitive(
            MirPrimitiveTypeName::F64,
        )))),
    )?;
    let minimum_slot = parameter_slot(header, "minimum", &f64_type())?;
    let range_slot = parameter_slot(header, "range", &f64_type())?;
    let induction_slot = parameter_slot(header, "j", &u32_type())?;
    let input = header.params[input_slot].value;
    let induction = header.params[induction_slot].value;

    let range_sub = &preheader.instructions[0];
    let range_result = result_of(range_sub, &f64_type())?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Sub,
        left: maximum,
        right: pre_minimum,
        semantics: KirArithmeticSemantics::StrictFloat,
    } = &range_sub.kind
    else {
        return None;
    };
    if preheader
        .params
        .iter()
        .any(|param| param.value == range_result)
        || *pre_minimum != incoming.args[minimum_slot]
        || incoming.args[range_slot] != range_result
        || value_type(function, *maximum) != Some(f64_type())
    {
        return None;
    }
    if incoming.args[output_slot] != preheader.params.iter().find(|p| p.slot == "out")?.value
        || constant_u32(function, incoming.args[induction_slot]) != Some(0)
        || preheader.params.iter().find(|p| p.slot == "minimum")?.value != *pre_minimum
        || preheader.params.iter().find(|p| p.slot == "maximum")?.value != *maximum
    {
        return None;
    }

    let KirInstructionKind::SliceLen { slice } = &block(function, descriptor.header)?
        .instructions
        .first()?
        .kind
    else {
        return None;
    };
    let header = block(function, descriptor.header)?;
    if header.instructions.len() != 2 || *slice != input {
        return None;
    }
    let len_value = result_of(&header.instructions[0], &u32_type())?;
    if !no_effect(&header.instructions[0]) {
        return None;
    }
    let loop_condition = result_of(&header.instructions[1], &bool_type())?;
    if header.instructions[1].kind
        != (KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: induction,
            right: len_value,
        })
        || !no_effect(&header.instructions[1])
    {
        return None;
    }
    let KirTerminator::Branch {
        condition,
        then_edge: body_edge,
        else_edge: exit_edge,
    } = &header.terminator
    else {
        return None;
    };
    if *condition != loop_condition
        || body_edge.target == descriptor.header
        || !descriptor.blocks.contains(&body_edge.target)
        || descriptor.blocks.contains(&exit_edge.target)
        || exit_edge.target != descriptor.exits[0]
        || exit_edge.args.len() != block(function, exit_edge.target)?.params.len()
        || exit_edge.memory_args.len() != block(function, exit_edge.target)?.memory_params.len()
        || !edge_forwards_memory_by_region(header, exit_edge, block(function, exit_edge.target)?)
    {
        return None;
    }
    let root = block(function, body_edge.target)?;
    if root.instructions.len() != 2 {
        return None;
    }
    let zero_const = result_of(&root.instructions[0], &f64_type())?;
    if root.instructions[0].kind
        != (KirInstructionKind::ConstFloat {
            value: "0.0".into(),
        })
        || !no_effect(&root.instructions[0])
    {
        return None;
    }
    let arm_condition = result_of(&root.instructions[1], &bool_type())?;
    if root.instructions[1].kind
        != (KirInstructionKind::Compare {
            op: MirCompareOp::Eq,
            left: root.params.get(range_slot)?.value,
            right: zero_const,
        })
        || !no_effect(&root.instructions[1])
    {
        return None;
    }
    if root.params.len() != header.params.len()
        || root
            .params
            .iter()
            .zip(&header.params)
            .any(|(actual, expected)| {
                actual.slot != expected.slot || actual.type_node != expected.type_node
            })
        || !edge_forwards_values_by_slot(header, body_edge, root)
        || root
            .memory_params
            .iter()
            .map(|p| p.region)
            .collect::<Vec<_>>()
            != header
                .memory_params
                .iter()
                .map(|p| p.region)
                .collect::<Vec<_>>()
    {
        return None;
    }
    let KirTerminator::Branch {
        condition: arm_value,
        then_edge: zero_edge,
        else_edge: false_edge,
    } = &root.terminator
    else {
        return None;
    };
    if *arm_value != arm_condition
        || zero_edge.target == false_edge.target
        || zero_edge.target == header.id
        || false_edge.target == header.id
        || !descriptor.blocks.contains(&zero_edge.target)
        || !descriptor.blocks.contains(&false_edge.target)
    {
        return None;
    }
    let zero = block(function, zero_edge.target)?;
    let false_arm = block(function, false_edge.target)?;
    if !edge_forwards_values_by_slot(root, zero_edge, zero)
        || !edge_forwards_values_by_slot(root, false_edge, false_arm)
    {
        return None;
    }
    if zero.instructions.len() != 1 || false_arm.instructions.len() != 4 {
        return None;
    }
    let zero_store = &zero.instructions[0];
    if !is_exact_store(
        zero_store,
        zero.params.get(output_slot)?.value,
        zero.params.get(induction_slot)?.value,
        zero_const,
    ) || zero_store
        .effect
        .as_ref()
        .is_none_or(|effect| effect.kind != KirEffectKind::WriteMemory)
    {
        return None;
    }
    if zero
        .instructions
        .iter()
        .any(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
    {
        return None;
    }
    let [load, subtract, divide, store] = false_arm.instructions.as_slice() else {
        return None;
    };
    let loaded = result_of(load, &f64_type())?;
    let difference = result_of(subtract, &f64_type())?;
    let normalized = result_of(divide, &f64_type())?;
    let KirInstructionKind::Load { place } = &load.kind else {
        return None;
    };
    let KirPlace::SliceIndex {
        slice: load_slice,
        index: load_index,
        ..
    } = place.as_ref()
    else {
        return None;
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Sub,
        left,
        right,
        semantics: KirArithmeticSemantics::StrictFloat,
    } = &subtract.kind
    else {
        return None;
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Div,
        left: numerator,
        right: denominator,
        semantics: KirArithmeticSemantics::StrictFloat,
    } = &divide.kind
    else {
        return None;
    };
    if *load_slice != false_arm.params[input_slot].value
        || *load_index != false_arm.params[induction_slot].value
        || *left != loaded
        || *right != false_arm.params[minimum_slot].value
        || *numerator != difference
        || *denominator != false_arm.params[range_slot].value
        || !is_exact_store(
            store,
            false_arm.params[output_slot].value,
            false_arm.params[induction_slot].value,
            normalized,
        )
        || load.memory.as_ref().is_none_or(|memory| {
            matches!(place.as_ref(), KirPlace::SliceIndex { region, .. } if memory.region != *region)
        })
        || load
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
        || store
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::WriteMemory)
        || subtract.effect.is_some()
        || subtract.memory.is_some()
        || divide.effect.is_some()
        || divide.memory.is_some()
    {
        return None;
    }
    if !edge_forwards_values_by_slot(
        false_arm,
        jump_edge(&false_arm.terminator)?,
        block(function, jump_edge(&false_arm.terminator)?.target)?,
    ) || !edge_forwards_values_by_slot(
        zero,
        jump_edge(&zero.terminator)?,
        block(function, jump_edge(&zero.terminator)?.target)?,
    ) {
        return None;
    }
    let latch_edge = jump_edge(&zero.terminator)?;
    if jump_edge(&false_arm.terminator)?.target != latch_edge.target {
        return None;
    }
    let latch = block(function, latch_edge.target)?;
    if latch.id != descriptor.latch?
        || latch.instructions.len() != 1
        || latch.params.len() != header.params.len()
        || !latch
            .params
            .iter()
            .zip(&header.params)
            .all(|(a, b)| a.slot == b.slot && a.type_node == b.type_node)
    {
        return None;
    }
    let increment = &latch.instructions[0];
    let increment_result = result_of(increment, &u32_type())?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left: increment_left,
        right: increment_right,
        semantics: KirArithmeticSemantics::Modular,
    } = &increment.kind
    else {
        return None;
    };
    let one = if *increment_left == latch.params[induction_slot].value {
        *increment_right
    } else if *increment_right == latch.params[induction_slot].value {
        *increment_left
    } else {
        return None;
    };
    if constant_u32(function, one) != Some(1) || !no_effect(increment) {
        return None;
    }
    let KirTerminator::Jump { edge: backedge } = &latch.terminator else {
        return None;
    };
    if backedge.target != header.id
        || backedge.args.len() != header.params.len()
        || backedge.memory_args.len() != header.memory_params.len()
        || backedge.args[induction_slot] != increment_result
        || backedge
            .args
            .iter()
            .enumerate()
            .any(|(index, value)| index != induction_slot && *value != latch.params[index].value)
        || !edge_forwards_memory_by_region(latch, backedge, header)
    {
        return None;
    }
    if zero_edge.args.len() != zero.params.len()
        || false_edge.args.len() != false_arm.params.len()
        || zero_edge.memory_args.len() != zero.memory_params.len()
        || false_edge.memory_args.len() != false_arm.memory_params.len()
    {
        return None;
    }
    if descriptor
        .blocks
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .len()
        != MAX_SOURCE_BLOCKS
        || [header.id, root.id, zero.id, false_arm.id, latch.id]
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != MAX_SOURCE_BLOCKS
        || descriptor.exits != [exit_edge.target]
    {
        return None;
    }
    Some(SourceShape {
        descriptor: descriptor.clone(),
        preheader: preheader_id,
        zero: zero.id,
        false_arm: false_arm.id,
        latch: latch.id,
        exit: exit_edge.target,
        input_slot,
        range_slot,
        induction_slot,
        one,
        exit_edge: exit_edge.clone(),
        incoming: incoming.clone(),
    })
}

fn jump_edge(terminator: &KirTerminator) -> Option<&crate::KirEdge> {
    match terminator {
        KirTerminator::Jump { edge } => Some(edge),
        KirTerminator::Branch { .. } | KirTerminator::Return { .. } => None,
    }
}

fn edge_forwards_values_by_slot(
    source: &KirBlock,
    edge: &crate::KirEdge,
    target: &KirBlock,
) -> bool {
    if edge.target != target.id
        || edge.args.len() != source.params.len()
        || source.params.len() != target.params.len()
        || !edge
            .args
            .iter()
            .zip(&source.params)
            .zip(&target.params)
            .all(|((value, source_param), target_param)| {
                source_param.slot == target_param.slot
                    && source_param.type_node == target_param.type_node
                    && *value == source_param.value
            })
        || !edge_forwards_memory_by_region(source, edge, target)
    {
        return false;
    }

    true
}

fn edge_forwards_memory_by_region(
    source: &KirBlock,
    edge: &crate::KirEdge,
    target: &KirBlock,
) -> bool {
    if edge.target != target.id || edge.memory_args.len() != target.memory_params.len() {
        return false;
    }
    let mut current_memory = BTreeMap::new();
    for param in &source.memory_params {
        if current_memory.insert(param.region, param.version).is_some() {
            return false;
        }
    }
    for instruction in &source.instructions {
        let Some(memory) = &instruction.memory else {
            continue;
        };
        if current_memory.get(&memory.region) != Some(&memory.input) {
            return false;
        }
        if let Some(output) = memory.output {
            current_memory.insert(memory.region, output);
        }
    }
    edge.memory_args
        .iter()
        .zip(&target.memory_params)
        .all(|(argument, param)| current_memory.get(&param.region) == Some(argument))
}

fn is_exact_store(
    instruction: &KirInstruction,
    slice: ValueId,
    index: ValueId,
    value: ValueId,
) -> bool {
    let KirInstructionKind::Store {
        place,
        value: stored,
    } = &instruction.kind
    else {
        return false;
    };
    let KirPlace::SliceIndex {
        slice: actual_slice,
        index: actual_index,
        type_node,
        ..
    } = place.as_ref()
    else {
        return false;
    };
    *actual_slice == slice
        && *actual_index == index
        && *stored == value
        && *type_node == MirType::Primitive(MirPrimitiveTypeName::F64)
}

fn profile_cost(
    profile: &KirTargetProfile,
    operation: KirProfileOperation,
    lane: KirLaneType,
) -> Option<u32> {
    let key = KirCostKey {
        operation,
        lane,
        lanes: 1,
        semantics: KirCostSemantics::NotApplicable,
        alignment: KirAlignmentClass::NotApplicable,
    };
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        _ => None,
    }
}

fn priced_minimum_trip(profile: &KirTargetProfile) -> Option<u32> {
    let f64_compare = profile_cost(profile, KirProfileOperation::Compare, KirLaneType::F64)?;
    let u32_compare = profile_cost(profile, KirProfileOperation::Compare, KirLaneType::U32)?;
    let branch = profile_cost(profile, KirProfileOperation::Branch, KirLaneType::U32)?;
    let removed_per_iteration = f64_compare.checked_add(branch)?;
    if removed_per_iteration == 0 {
        return None;
    }
    // This is a path-local threshold for the nonzero fast arm. The +0 arm
    // bypasses this gate and retains the original scalar loop, so this cost
    // establishes amortization only when the nonzero arm is selected; it is
    // not an aggregate cost claim for an unknown zero/nonzero input mix.
    // SliceLen is a local descriptor read. The gate compares the trip
    // threshold and branches, then performs range==+0 and branches once.
    let gate_cost = 1_u32
        .checked_add(u32_compare)?
        .checked_add(branch)?
        .checked_add(f64_compare)?
        .checked_add(branch)?;
    let threshold = gate_cost
        .checked_div(removed_per_iteration)?
        .checked_add(1)?;
    Some(threshold.max(2))
}

#[must_use]
pub fn discover_normalization_unswitch_candidates(
    state: &KirVerifiedProgramState,
) -> Vec<NormalizationUnswitchCandidate> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly
        || state.module().config.overflow_mode != crate::KirOverflowMode::Unchecked
        || state.module().config.bounds_mode != crate::KirBoundsMode::Unchecked
        || state.module().config.sanitizer_mode != crate::KirSanitizerMode::Disabled
    {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for function in &state.module().functions {
        if !function.vector_regions.is_empty() {
            continue;
        }
        for descriptor in analyze_canonical_loops(function)
            .loops
            .into_iter()
            .filter(|loop_info| loop_info.innermost)
        {
            if recognize_source(function, &descriptor).is_some()
                && priced_minimum_trip(&state.module().profile).is_some()
            {
                candidates.push(NormalizationUnswitchCandidate {
                    function: function.id,
                    loop_id: descriptor.id,
                    header: descriptor.header,
                });
            }
        }
    }
    candidates.sort_by_key(|candidate| (candidate.function, candidate.loop_id, candidate.header));
    candidates
}

fn next_effect(function: &KirFunction) -> Result<u32, String> {
    function
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .instructions
                .iter()
                .filter_map(|instruction| instruction.effect.as_ref().map(|effect| effect.order))
                .chain(match block.terminator {
                    KirTerminator::Return { effect_order, .. } => Some(effect_order),
                    KirTerminator::Jump { .. } | KirTerminator::Branch { .. } => None,
                })
        })
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "normalization unswitch effect identity exhausted".into())
}

fn emit_pure(
    state: &mut KirVerifiedProgramState,
    instructions: &mut Vec<KirInstruction>,
    kind: KirInstructionKind,
    type_node: KirValueType,
) -> Result<ValueId, String> {
    let value = state.fresh_value()?;
    instructions.push(KirInstruction {
        id: state.fresh_instruction()?,
        results: vec![KirResult { value, type_node }],
        kind,
        memory: None,
        effect: None,
    });
    Ok(value)
}

fn clone_parameters(
    state: &mut KirVerifiedProgramState,
    source: &KirBlock,
    id: BlockId,
    label: &str,
) -> Result<KirBlock, String> {
    let mut cloned = source.clone();
    cloned.id = id;
    cloned.label = label.to_string();
    cloned.instructions.clear();
    for param in &mut cloned.params {
        param.value = state.fresh_value()?;
    }
    for param in &mut cloned.memory_params {
        param.version = state.fresh_memory_version()?;
    }
    Ok(cloned)
}

fn mapped(value: ValueId, values: &BTreeMap<ValueId, ValueId>) -> ValueId {
    values.get(&value).copied().unwrap_or(value)
}

fn remap_instruction(
    instruction: &mut KirInstruction,
    values: &BTreeMap<ValueId, ValueId>,
    memories: &BTreeMap<MemoryVersionId, MemoryVersionId>,
    effect_order: &mut u32,
) -> Result<(), String> {
    fn remap_place(place: &mut KirPlace, values: &BTreeMap<ValueId, ValueId>) {
        match place {
            KirPlace::SliceIndex { slice, index, .. } => {
                *slice = mapped(*slice, values);
                *index = mapped(*index, values);
            }
            KirPlace::Value { value, .. } => *value = mapped(*value, values),
            KirPlace::Deref { pointer, .. } => *pointer = mapped(*pointer, values),
            KirPlace::Index { base, index, .. } => {
                remap_place(base, values);
                *index = mapped(*index, values);
            }
            KirPlace::Field { base, .. } => remap_place(base, values),
        }
    }
    match &mut instruction.kind {
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => {
            *value = mapped(*value, values);
        }
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => {
            *left = mapped(*left, values);
            *right = mapped(*right, values);
        }
        KirInstructionKind::Unary { operand, .. } => *operand = mapped(*operand, values),
        KirInstructionKind::Load { place } | KirInstructionKind::Address { place } => {
            remap_place(place, values)
        }
        KirInstructionKind::Store { place, value } => {
            remap_place(place, values);
            *value = mapped(*value, values);
        }
        KirInstructionKind::SliceLen { slice } | KirInstructionKind::SliceData { slice } => {
            *slice = mapped(*slice, values);
        }
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => {}
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::CheckCondition { .. }
        | KirInstructionKind::Guard { .. }
        | KirInstructionKind::MakeSlice { .. }
        | KirInstructionKind::Subslice { .. }
        | KirInstructionKind::Call { .. }
        | KirInstructionKind::RuntimeCall { .. }
        | KirInstructionKind::VersionPredicate { .. }
        | KirInstructionKind::VectorSplat { .. }
        | KirInstructionKind::VectorLoad { .. }
        | KirInstructionKind::VectorStore { .. }
        | KirInstructionKind::VectorBinary { .. }
        | KirInstructionKind::VectorUnary { .. }
        | KirInstructionKind::VectorCompare { .. }
        | KirInstructionKind::VectorSelect { .. }
        | KirInstructionKind::VectorCast { .. }
        | KirInstructionKind::VectorInsert { .. }
        | KirInstructionKind::VectorExtract { .. }
        | KirInstructionKind::VectorReduce { .. } => {
            return Err("normalization clone encountered unsupported source operation".into());
        }
    }
    if let Some(memory) = &mut instruction.memory {
        memory.input = memories.get(&memory.input).copied().unwrap_or(memory.input);
    }
    if let Some(effect) = &mut instruction.effect {
        effect.order = *effect_order;
        *effect_order = effect_order
            .checked_add(1)
            .ok_or_else(|| "normalization unswitch effect identity exhausted".to_string())?;
    }
    Ok(())
}

fn clone_instruction_sequence(
    state: &mut KirVerifiedProgramState,
    source: &KirBlock,
    target: &mut KirBlock,
    effect_order: &mut u32,
) -> Result<ClonedSequence, String> {
    if source.params.len() != target.params.len()
        || source.memory_params.len() != target.memory_params.len()
    {
        return Err("normalization clone parameter shape changed".into());
    }
    let mut values = source
        .params
        .iter()
        .zip(&target.params)
        .map(|(old, new)| (old.value, new.value))
        .collect::<BTreeMap<_, _>>();
    let mut memories = source
        .memory_params
        .iter()
        .zip(&target.memory_params)
        .map(|(old, new)| (old.version, new.version))
        .collect::<BTreeMap<_, _>>();
    let mut mapping = Vec::new();
    for old in &source.instructions {
        let mut cloned = old.clone();
        let old_id = old.id;
        cloned.id = state.fresh_instruction()?;
        remap_instruction(&mut cloned, &values, &memories, effect_order)?;
        for (old_result, new_result) in old.results.iter().zip(&mut cloned.results) {
            let fresh = state.fresh_value()?;
            values.insert(old_result.value, fresh);
            new_result.value = fresh;
        }
        if let (Some(old_memory), Some(new_memory)) = (&old.memory, &mut cloned.memory)
            && let Some(old_output) = old_memory.output
        {
            let fresh = state.fresh_memory_version()?;
            memories.insert(old_output, fresh);
            new_memory.output = Some(fresh);
        }
        mapping.push((old_id, cloned.id));
        target.instructions.push(cloned);
    }
    Ok(ClonedSequence {
        instruction_mapping: mapping,
        memories,
    })
}

fn forward_edge(source: &KirBlock, target: &KirBlock) -> crate::KirEdge {
    crate::KirEdge {
        target: target.id,
        args: source.params.iter().map(|param| param.value).collect(),
        memory_args: source
            .memory_params
            .iter()
            .map(|param| param.version)
            .collect(),
    }
}

fn cost_charge(plan: &NormalizationUnswitchPlan) -> CandidateBudgetCharge {
    CandidateBudgetCharge::single(
        plan.function,
        plan.after_units
            .saturating_sub(plan.before_units)
            .saturating_add(16),
        plan.before_units
            .saturating_add(plan.after_units)
            .saturating_add(32),
    )
}

pub fn prepare_normalization_unswitch_trial(
    state: &KirVerifiedProgramState,
    candidate: &NormalizationUnswitchCandidate,
) -> Result<PreparedNormalizationUnswitch, String> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly {
        return Err("normalization unswitch requires WebAssembly KIR".into());
    }
    let original = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or("normalization unswitch function missing")?;
    let descriptor = analyze_canonical_loops(original)
        .loops
        .into_iter()
        .find(|loop_info| loop_info.id == candidate.loop_id && loop_info.header == candidate.header)
        .ok_or("stale normalization unswitch loop")?;
    let shape =
        recognize_source(original, &descriptor).ok_or("unsupported strict normalization loop")?;
    let minimum_trip = priced_minimum_trip(&state.module().profile)
        .ok_or("target profile does not price the strict normalization dispatch")?;
    let mut trial = state.clone();
    let mut function = original.clone();
    let header = block(original, shape.descriptor.header).ok_or("normalization header missing")?;
    let false_arm = block(original, shape.false_arm).ok_or("normalization false arm missing")?;
    let dispatch_id = trial.fresh_block()?;
    let fast_preheader_id = trial.fresh_block()?;
    let fast_header_id = trial.fresh_block()?;
    let fast_body_id = trial.fresh_block()?;
    let fast_exit_id = trial.fresh_block()?;
    let mut effect_order = next_effect(original)?;

    let mut preheader_extra = Vec::new();
    let preheader_input = shape.incoming.args[shape.input_slot];
    let len = emit_pure(
        &mut trial,
        &mut preheader_extra,
        KirInstructionKind::SliceLen {
            slice: preheader_input,
        },
        u32_type(),
    )?;
    let threshold = emit_pure(
        &mut trial,
        &mut preheader_extra,
        KirInstructionKind::ConstInt {
            value: minimum_trip.to_string(),
        },
        u32_type(),
    )?;
    let enough = emit_pure(
        &mut trial,
        &mut preheader_extra,
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: len,
            right: threshold,
        },
        bool_type(),
    )?;
    let dispatch_instructions = preheader_extra
        .iter()
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>();
    let dispatch_edge = crate::KirEdge {
        target: dispatch_id,
        args: Vec::new(),
        memory_args: Vec::new(),
    };
    let mut dispatch = KirBlock {
        id: dispatch_id,
        label: "normalization.range_dispatch".into(),
        params: Vec::new(),
        memory_params: Vec::new(),
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: None,
            memory: Vec::new(),
            effect_order: 0,
        },
    };
    let positive_zero = emit_pure(
        &mut trial,
        &mut dispatch.instructions,
        KirInstructionKind::ConstFloat {
            value: "0.0".into(),
        },
        f64_type(),
    )?;
    let is_zero = emit_pure(
        &mut trial,
        &mut dispatch.instructions,
        KirInstructionKind::Compare {
            op: MirCompareOp::Eq,
            left: shape.incoming.args[shape.range_slot],
            right: positive_zero,
        },
        bool_type(),
    )?;
    let range_instructions = [dispatch.instructions[0].id, dispatch.instructions[1].id];

    // Keep the range==+0 choice outside a simple loop preheader. The loop
    // vectorizer requires a dedicated Jump-only preheader so it can insert its
    // checked SIMD entry and retain the scalar loop as the guard-failure path.
    let mut fast_header = clone_parameters(
        &mut trial,
        header,
        fast_header_id,
        "normalization.nonzero.header",
    )?;
    let fast_header_clone =
        clone_instruction_sequence(&mut trial, header, &mut fast_header, &mut effect_order)?;
    let fast_header_instruction_mapping = fast_header_clone.instruction_mapping;
    let header_memories = fast_header_clone.memories;
    let mut header_values = header
        .params
        .iter()
        .zip(&fast_header.params)
        .map(|(source, cloned)| (source.value, cloned.value))
        .collect::<BTreeMap<_, _>>();
    for (source_id, cloned_id) in &fast_header_instruction_mapping {
        let source_instruction = header
            .instructions
            .iter()
            .find(|instruction| instruction.id == *source_id)
            .ok_or("normalization source header instruction missing")?;
        let cloned_instruction = fast_header
            .instructions
            .iter()
            .find(|instruction| instruction.id == *cloned_id)
            .ok_or("normalization cloned header instruction missing")?;
        for (source, cloned) in source_instruction
            .results
            .iter()
            .zip(&cloned_instruction.results)
        {
            header_values.insert(source.value, cloned.value);
        }
    }
    let header_condition = fast_header
        .instructions
        .get(1)
        .and_then(|instruction| result_of(instruction, &bool_type()))
        .ok_or("normalization cloned header condition missing")?;
    let mut fast_preheader = clone_parameters(
        &mut trial,
        header,
        fast_preheader_id,
        "normalization.nonzero.preheader",
    )?;
    fast_preheader.params.remove(shape.induction_slot);
    let mut fast_preheader_args = Vec::with_capacity(fast_header.params.len());
    let mut forwarded_param = 0;
    for (index, _) in header.params.iter().enumerate() {
        if index == shape.induction_slot {
            // Keep the original source zero visible to the vector checker; do
            // not hide it behind a newly introduced block parameter.
            fast_preheader_args.push(shape.incoming.args[index]);
        } else {
            fast_preheader_args.push(fast_preheader.params[forwarded_param].value);
            forwarded_param += 1;
        }
    }
    fast_preheader.terminator = KirTerminator::Jump {
        edge: crate::KirEdge {
            target: fast_header_id,
            args: fast_preheader_args,
            memory_args: fast_preheader
                .memory_params
                .iter()
                .map(|param| param.version)
                .collect(),
        },
    };

    let mut fast_body = clone_parameters(
        &mut trial,
        false_arm,
        fast_body_id,
        "normalization.nonzero.body",
    )?;
    let fast_body_clone =
        clone_instruction_sequence(&mut trial, false_arm, &mut fast_body, &mut effect_order)?;
    let fast_body_instruction_mapping = fast_body_clone.instruction_mapping;
    let update = KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: trial.fresh_value()?,
            type_node: u32_type(),
        }],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: fast_body.params[shape.induction_slot].value,
            right: shape.one,
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    };
    let fast_counter_value = update.results[0].value;
    let fast_counter_update = update.id;
    fast_body.instructions.push(update);
    let mut backedge_args = fast_body
        .params
        .iter()
        .map(|param| param.value)
        .collect::<Vec<_>>();
    backedge_args[shape.induction_slot] = fast_counter_value;
    let mut backedge_memories = Vec::with_capacity(fast_header.memory_params.len());
    for header_memory in &fast_header.memory_params {
        let body_memory = fast_body
            .memory_params
            .iter()
            .find(|param| param.region == header_memory.region)
            .ok_or("normalization cloned loop memory region missing")?;
        let mut current = body_memory.version;
        for (old_id, new_id) in &fast_body_instruction_mapping {
            let old_instruction = false_arm
                .instructions
                .iter()
                .find(|instruction| instruction.id == *old_id)
                .ok_or("normalization source memory instruction missing")?;
            let new_instruction = fast_body
                .instructions
                .iter()
                .find(|instruction| instruction.id == *new_id)
                .ok_or("normalization cloned memory instruction missing")?;
            if old_instruction
                .memory
                .as_ref()
                .is_some_and(|memory| memory.region == header_memory.region)
                && new_instruction
                    .memory
                    .as_ref()
                    .is_some_and(|memory| memory.region == header_memory.region)
            {
                current = new_instruction
                    .memory
                    .as_ref()
                    .and_then(|memory| memory.output)
                    .unwrap_or(current);
            }
        }
        backedge_memories.push(current);
    }
    fast_body.terminator = KirTerminator::Jump {
        edge: crate::KirEdge {
            target: fast_header_id,
            args: backedge_args,
            memory_args: backedge_memories,
        },
    };
    let mut fast_exit = shape.exit_edge.clone();
    let source_exit = block(original, shape.exit).ok_or("normalization scalar exit missing")?;
    let mut fast_exit_block = clone_parameters(
        &mut trial,
        source_exit,
        fast_exit_id,
        "normalization.nonzero.exit",
    )?;
    fast_exit.args = shape
        .exit_edge
        .args
        .iter()
        .map(|value| mapped(*value, &header_values))
        .collect();
    fast_exit.memory_args = shape
        .exit_edge
        .memory_args
        .iter()
        .map(|version| header_memories.get(version).copied().unwrap_or(*version))
        .collect();
    fast_exit.target = fast_exit_id;
    fast_exit_block.terminator = KirTerminator::Jump {
        edge: forward_edge(&fast_exit_block, source_exit),
    };
    fast_header.terminator = KirTerminator::Branch {
        condition: header_condition,
        then_edge: forward_edge(&fast_header, &fast_body),
        else_edge: fast_exit,
    };
    dispatch.terminator = KirTerminator::Branch {
        condition: is_zero,
        then_edge: shape.incoming.clone(),
        else_edge: crate::KirEdge {
            target: fast_preheader_id,
            args: shape
                .incoming
                .args
                .iter()
                .enumerate()
                .filter_map(|(index, value)| (index != shape.induction_slot).then_some(*value))
                .collect(),
            memory_args: shape.incoming.memory_args.clone(),
        },
    };
    let preheader_block = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == shape.preheader)
        .ok_or("normalization preheader disappeared")?;
    preheader_block.instructions.extend(preheader_extra);
    preheader_block.terminator = KirTerminator::Branch {
        condition: enough,
        then_edge: dispatch_edge,
        else_edge: shape.incoming.clone(),
    };
    function.blocks.extend([
        dispatch,
        fast_preheader,
        fast_header,
        fast_body,
        fast_exit_block,
    ]);
    let before_units = kir_function_units(original);
    let after_units = kir_function_units(&function);
    if after_units.saturating_sub(before_units) > MAX_FUNCTION_GROWTH {
        return Err("normalization unswitch exceeds its fixed growth budget".into());
    }
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|candidate| candidate.id == original.id)
        .ok_or("normalization function disappeared")? = function;

    let plan = NormalizationUnswitchPlan {
        function: original.id,
        loop_id: candidate.loop_id,
        header: candidate.header,
        preheader: shape.preheader,
        zero_arm: shape.zero,
        false_arm: shape.false_arm,
        latch: shape.latch,
        exit: shape.exit,
        pre_state_digest: state.kir_digest(),
        profile_digest: state.module().profile.digest_hex(),
        minimum_trip,
        dispatch: dispatch_id,
        fast_preheader: fast_preheader_id,
        fast_header: fast_header_id,
        fast_body: fast_body_id,
        fast_exit: fast_exit_id,
        dispatch_instructions: [
            dispatch_instructions[0],
            dispatch_instructions[1],
            dispatch_instructions[2],
        ],
        range_instructions,
        fast_header_instruction_mapping,
        fast_body_instruction_mapping,
        fast_counter_update,
        fast_counter_value,
        before_units,
        after_units,
    };
    let charge = cost_charge(&plan);
    check_normalization_unswitch_independently(state, &trial, &plan, &charge)
        .map_err(|error| format!("normalization unswitch self-check failed: {error:?}"))?;
    Ok(PreparedNormalizationUnswitch {
        trial,
        plan,
        charge,
    })
}

/// Checks a normalized trial without invoking discovery or trial construction.
/// The source is reconstructed from immutable pre-state KIR; every cloned value,
/// effect and MemorySSA edge is then checked against that reconstruction.
pub fn check_normalization_unswitch_independently(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &NormalizationUnswitchPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_unswitch(pre, trial, plan, charge).map_err(TransactionCheckError::reject)
}

fn check_unswitch(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &NormalizationUnswitchPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), String> {
    if pre.module().config.consumer != crate::KirConsumer::WebAssembly
        || plan.pre_state_digest != pre.kir_digest()
        || plan.profile_digest != pre.module().profile.digest_hex()
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
        || pre.evidence_generation() != trial.evidence_generation()
        || pre.optimization_entry_module_units() != trial.optimization_entry_module_units()
    {
        return Err("normalization unswitch source/evidence identity changed".into());
    }
    let original = pre
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.function)
        .ok_or("normalization unswitch source function missing")?;
    let transformed = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.function)
        .ok_or("normalization unswitch trial function missing")?;
    let source = reconstruct_source_for_checker(original, plan)?;
    if source.descriptor.id != plan.loop_id
        || source.descriptor.header != plan.header
        || source.preheader != plan.preheader
        || source.zero != plan.zero_arm
        || source.false_arm != plan.false_arm
        || source.latch != plan.latch
        || source.exit != plan.exit
    {
        return Err("normalization unswitch plan does not identify reconstructed source".into());
    }
    let minimum_trip = priced_minimum_trip(&pre.module().profile)
        .ok_or("normalization unswitch target costs are unavailable")?;
    if plan.minimum_trip != minimum_trip {
        return Err("normalization unswitch threshold is not profile-derived".into());
    }
    let mut untouched = trial.module().clone();
    *untouched
        .functions
        .iter_mut()
        .find(|function| function.id == plan.function)
        .ok_or("normalization unswitch trial function missing")? = original.clone();
    if &untouched != pre.module() {
        return Err("normalization unswitch changed module metadata or another function".into());
    }
    let mut metadata = transformed.clone();
    metadata.blocks = original.blocks.clone();
    if &metadata != original {
        return Err("normalization unswitch changed function metadata".into());
    }
    let added_ids = [
        plan.dispatch,
        plan.fast_preheader,
        plan.fast_header,
        plan.fast_body,
        plan.fast_exit,
    ];
    if added_ids.iter().copied().collect::<BTreeSet<_>>().len() != added_ids.len()
        || added_ids.iter().any(|id| block(original, *id).is_some())
        || transformed.blocks.len() != original.blocks.len() + added_ids.len()
        || transformed.blocks[original.blocks.len()..]
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>()
            != added_ids
    {
        return Err("normalization unswitch added-block coverage is false".into());
    }
    let preheader_before = block(original, plan.preheader).ok_or("source preheader missing")?;
    let preheader_after = block(transformed, plan.preheader).ok_or("trial preheader missing")?;
    if preheader_before.params != preheader_after.params
        || preheader_before.memory_params != preheader_after.memory_params
        || preheader_before.label != preheader_after.label
        || preheader_after.instructions.len() != preheader_before.instructions.len() + 3
        || preheader_after.instructions[..preheader_before.instructions.len()]
            != preheader_before.instructions
    {
        return Err("normalization unswitch changed range evaluation or preheader state".into());
    }
    let extra = &preheader_after.instructions[preheader_before.instructions.len()..];
    let input_from_scan = source.incoming.args[source.input_slot];
    let len_value = check_pure_result(
        &extra[0],
        KirInstructionKind::SliceLen {
            slice: input_from_scan,
        },
        &u32_type(),
        plan.dispatch_instructions[0],
    )?;
    let threshold_value = check_pure_result(
        &extra[1],
        KirInstructionKind::ConstInt {
            value: minimum_trip.to_string(),
        },
        &u32_type(),
        plan.dispatch_instructions[1],
    )?;
    let enough = check_pure_result(
        &extra[2],
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: len_value,
            right: threshold_value,
        },
        &bool_type(),
        plan.dispatch_instructions[2],
    )?;
    let original_entry = source.incoming.clone();
    if preheader_after.terminator
        != (KirTerminator::Branch {
            condition: enough,
            then_edge: crate::KirEdge {
                target: plan.dispatch,
                args: Vec::new(),
                memory_args: Vec::new(),
            },
            else_edge: original_entry.clone(),
        })
    {
        return Err("normalization unswitch length guard or full-loop fallback is false".into());
    }
    for (before, after) in original.blocks.iter().zip(&transformed.blocks) {
        if before.id != plan.preheader && before != after {
            return Err("normalization unswitch modified the original scalar loop".into());
        }
        if before.id != after.id {
            return Err("normalization unswitch reordered original blocks".into());
        }
    }

    let dispatch = block(transformed, plan.dispatch).ok_or("range dispatch block missing")?;
    if !dispatch.params.is_empty()
        || !dispatch.memory_params.is_empty()
        || dispatch.instructions.len() != 2
        || dispatch
            .instructions
            .iter()
            .any(|instruction| !no_effect(instruction))
    {
        return Err("normalization range dispatch contains effects or unstable state".into());
    }
    let positive_zero = check_pure_result(
        &dispatch.instructions[0],
        KirInstructionKind::ConstFloat {
            value: "0.0".into(),
        },
        &f64_type(),
        plan.range_instructions[0],
    )?;
    let range_source = source.range_value;
    let is_zero = check_pure_result(
        &dispatch.instructions[1],
        KirInstructionKind::Compare {
            op: MirCompareOp::Eq,
            left: range_source,
            right: positive_zero,
        },
        &bool_type(),
        plan.range_instructions[1],
    )?;
    let header_before = block(original, plan.header).ok_or("source output header missing")?;
    let fast_preheader =
        block(transformed, plan.fast_preheader).ok_or("false-path loop preheader missing")?;
    let fast_header =
        block(transformed, plan.fast_header).ok_or("false-path loop header missing")?;
    let fast_body = block(transformed, plan.fast_body).ok_or("false-path loop body missing")?;
    let mut expected_fast_preheader_shape = header_before.clone();
    expected_fast_preheader_shape
        .params
        .remove(source.induction_slot);
    check_cloned_parameters(&expected_fast_preheader_shape, fast_preheader, original)?;
    let mut forwarded_param = 0;
    let expected_fast_header_args = header_before
        .params
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index == source.induction_slot {
                source.incoming.args[index]
            } else {
                let value = fast_preheader.params[forwarded_param].value;
                forwarded_param += 1;
                value
            }
        })
        .collect::<Vec<_>>();
    if fast_preheader.label != "normalization.nonzero.preheader"
        || !fast_preheader.instructions.is_empty()
        || fast_preheader.terminator
            != (KirTerminator::Jump {
                edge: crate::KirEdge {
                    target: plan.fast_header,
                    args: expected_fast_header_args,
                    memory_args: fast_preheader
                        .memory_params
                        .iter()
                        .map(|param| param.version)
                        .collect(),
                },
            })
    {
        return Err(
            "normalization false-path preheader is not a pure state-forwarding jump".into(),
        );
    }
    check_cloned_parameters(header_before, fast_header, original)?;
    let checked_header = verify_cloned_instructions(
        header_before,
        fast_header,
        &plan.fast_header_instruction_mapping,
        &BTreeMap::new(),
        next_effect(original)?,
        original,
    )?;
    let header_values = checked_header.values;
    let header_memories = checked_header.memories;
    let next_effect_order = checked_header.next_effect_order;
    let source_exit = block(original, source.exit).ok_or("source scalar exit missing")?;
    let fast_exit =
        block(transformed, plan.fast_exit).ok_or("false-path dedicated exit missing")?;
    check_cloned_parameters(source_exit, fast_exit, original)?;
    if fast_exit.label != "normalization.nonzero.exit"
        || !fast_exit.instructions.is_empty()
        || fast_exit.terminator
            != (KirTerminator::Jump {
                edge: forward_edge(fast_exit, source_exit),
            })
    {
        return Err(
            "normalization false-path exit does not forward exact state and MemorySSA".into(),
        );
    }
    if header_values.len() < header_before.params.len()
        || fast_header.instructions.len() != header_before.instructions.len()
        || fast_header.params.iter().any(|param| {
            header_before
                .params
                .iter()
                .any(|old| old.value == param.value)
        })
    {
        return Err("normalization cloned header value map is incomplete".into());
    }
    let loop_condition = fast_header
        .instructions
        .get(1)
        .and_then(|instruction| result_of(instruction, &bool_type()))
        .ok_or("cloned loop condition missing")?;
    let expected_exit = crate::KirEdge {
        target: plan.fast_exit,
        args: source
            .exit_edge
            .args
            .iter()
            .map(|value| header_values.get(value).copied().unwrap_or(*value))
            .collect(),
        memory_args: source
            .exit_edge
            .memory_args
            .iter()
            .map(|version| {
                let region = header_before
                    .memory_params
                    .iter()
                    .find(|param| param.version == *version)
                    .map(|param| param.region)
                    .ok_or_else(|| "source header exit MemorySSA parameter missing".to_string())?;
                header_memories
                    .get(&region)
                    .copied()
                    .ok_or_else(|| "cloned header exit MemorySSA parameter missing".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let mut fast_loop = analyze_canonical_loops(transformed)
        .loops
        .into_iter()
        .find(|loop_info| loop_info.header == plan.fast_header)
        .ok_or("normalization false-path loop is not canonical")?;
    fast_loop.blocks.sort();
    let mut expected_blocks = vec![plan.fast_header, plan.fast_body];
    expected_blocks.sort();
    if fast_loop.blocks != expected_blocks
        || fast_loop.preheader != Some(plan.fast_preheader)
        || !fast_loop.dedicated_exits
        || fast_loop.exits != [plan.fast_exit]
        || fast_header.terminator
            != (KirTerminator::Branch {
                condition: loop_condition,
                then_edge: forward_edge(fast_header, fast_body),
                else_edge: expected_exit,
            })
    {
        return Err("normalization false-path loop shape or exit is false".into());
    }
    if fast_body.params.len() != block(original, source.false_arm).unwrap().params.len()
        || fast_body.instructions.len()
            != block(original, source.false_arm)
                .unwrap()
                .instructions
                .len()
                + 1
        || fast_body
            .memory_params
            .iter()
            .map(|param| param.region)
            .collect::<Vec<_>>()
            != fast_header
                .memory_params
                .iter()
                .map(|param| param.region)
                .collect::<Vec<_>>()
    {
        return Err("normalization false-path body shape is false".into());
    }
    let source_false = block(original, source.false_arm).unwrap();
    check_cloned_parameters(source_false, fast_body, original)?;
    let checked_body = verify_cloned_instructions(
        source_false,
        fast_body,
        &plan.fast_body_instruction_mapping,
        &header_values,
        next_effect_order,
        original,
    )?;
    let body_values = checked_body.values;
    let body_memories = checked_body.memories;
    let final_effect_order = checked_body.next_effect_order;
    let update = fast_body
        .instructions
        .last()
        .ok_or("false-path counter update missing")?;
    let update_value = check_pure_result(
        update,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: fast_body.params[source.induction_slot].value,
            right: source.one,
            semantics: KirArithmeticSemantics::Modular,
        },
        &u32_type(),
        plan.fast_counter_update,
    )?;
    if update_value != plan.fast_counter_value || final_effect_order <= next_effect_order {
        return Err("normalization false-path counter or cloned effects are false".into());
    }
    let mut backedge_args = fast_body
        .params
        .iter()
        .map(|param| param.value)
        .collect::<Vec<_>>();
    backedge_args[source.induction_slot] = update_value;
    let backedge_memories = fast_header
        .memory_params
        .iter()
        .map(|header_param| {
            body_memories
                .get(&header_param.region)
                .copied()
                .ok_or_else(|| "cloned body omitted a MemorySSA region".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if fast_body.terminator
        != (KirTerminator::Jump {
            edge: crate::KirEdge {
                target: plan.fast_header,
                args: backedge_args,
                memory_args: backedge_memories,
            },
        })
    {
        return Err("normalization false-path backedge changes state or memory".into());
    }
    let _ = body_values;

    let dispatch_predecessors = transformed
        .blocks
        .iter()
        .flat_map(|candidate| successor_edges(&candidate.terminator))
        .filter(|edge| edge.target == plan.dispatch)
        .collect::<Vec<_>>();
    if dispatch_predecessors.len() != 1
        || transformed
            .blocks
            .iter()
            .find(|candidate| candidate.id == plan.preheader)
            .is_none_or(|candidate| !matches!(&candidate.terminator, KirTerminator::Branch { then_edge: crate::KirEdge { target, .. }, .. } if *target == plan.dispatch))
    {
        return Err("normalization dispatch operands are not invariant on every path".into());
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &dispatch.terminator
    else {
        return Err("normalization range dispatch is not a branch".into());
    };
    if *condition != is_zero
        || then_edge != &original_entry
        || else_edge.target != plan.fast_preheader
        || else_edge.args
            != source
                .incoming
                .args
                .iter()
                .enumerate()
                .filter_map(|(index, value)| (index != source.induction_slot).then_some(*value))
                .collect::<Vec<_>>()
        || else_edge.memory_args != source.incoming.memory_args
    {
        return Err("normalization range==+0 dispatch or false-arm selection is false".into());
    }
    if zero_path_contains_load(original, &source)
        || !matches!(
            &dispatch.instructions[1].kind,
            KirInstructionKind::Compare {
                op: MirCompareOp::Eq,
                ..
            }
        )
    {
        return Err("normalization zero path can load input or range test is not exact Eq".into());
    }
    if plan.before_units != kir_function_units(original)
        || plan.after_units != kir_function_units(transformed)
        || plan.after_units.saturating_sub(plan.before_units) > MAX_FUNCTION_GROWTH
        || charge != &cost_charge(plan)
    {
        return Err("normalization unswitch growth or transaction budget is false".into());
    }
    let validation = validate_kir_module(trial.module());
    if !validation.errors.is_empty() {
        return Err(format!(
            "normalization unswitch KIR validation failed: {:?}",
            validation.errors
        ));
    }
    Ok(())
}

fn reconstruct_source_for_checker(
    function: &KirFunction,
    plan: &NormalizationUnswitchPlan,
) -> Result<CheckerSource, String> {
    let descriptor = analyze_canonical_loops(function)
        .loops
        .into_iter()
        .find(|loop_info| loop_info.id == plan.loop_id && loop_info.header == plan.header)
        .ok_or("normalization checker could not reconstruct canonical source loop")?;
    if !descriptor.innermost
        || !descriptor.lcssa
        || descriptor.blocks.len() != MAX_SOURCE_BLOCKS
        || descriptor.exits != [plan.exit]
    {
        return Err("normalization checker source loop has extra blocks or exits".into());
    }
    let preheader_id = descriptor
        .preheader
        .ok_or("source loop preheader missing")?;
    let preheader = block(function, preheader_id).ok_or("source loop preheader missing")?;
    let header = block(function, descriptor.header).ok_or("source loop header missing")?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return Err("source range block no longer directly enters output loop".into());
    };
    if preheader_id != plan.preheader
        || incoming.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || !edge_forwards_memory_by_region(preheader, incoming, header)
        || preheader.instructions.len() != 1
    {
        return Err("source range computation or entry state changed".into());
    }
    let input_ty = KirValueType::Scalar(MirType::Slice(Box::new(MirType::Primitive(
        MirPrimitiveTypeName::F64,
    ))));
    let input_slot =
        parameter_slot(header, "input", &input_ty).ok_or("source input slot changed")?;
    let output_slot =
        parameter_slot(header, "out", &input_ty).ok_or("source output slot changed")?;
    let minimum_slot =
        parameter_slot(header, "minimum", &f64_type()).ok_or("source minimum slot changed")?;
    let range_slot =
        parameter_slot(header, "range", &f64_type()).ok_or("source range slot changed")?;
    let induction_slot =
        parameter_slot(header, "j", &u32_type()).ok_or("source induction slot changed")?;
    let range_result =
        result_of(&preheader.instructions[0], &f64_type()).ok_or("source range result changed")?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Sub,
        left: maximum,
        right: minimum,
        semantics: KirArithmeticSemantics::StrictFloat,
    } = &preheader.instructions[0].kind
    else {
        return Err("source range must remain strict maximum minus minimum".into());
    };
    if !no_effect(&preheader.instructions[0])
        || incoming.args[range_slot] != range_result
        || incoming.args[minimum_slot] != *minimum
        || preheader
            .params
            .iter()
            .find(|param| param.slot == "minimum")
            .map(|p| p.value)
            != Some(*minimum)
        || preheader
            .params
            .iter()
            .find(|param| param.slot == "maximum")
            .map(|p| p.value)
            != Some(*maximum)
        || incoming.args[output_slot]
            != preheader
                .params
                .iter()
                .find(|param| param.slot == "out")
                .map(|p| p.value)
                .ok_or("preheader output missing")?
        || constant_u32(function, incoming.args[induction_slot]) != Some(0)
    {
        return Err("source range, minimum, maximum or initial output index changed".into());
    }
    if header.instructions.len() != 2 {
        return Err("source output bound contains extra work".into());
    }
    let input = header.params[input_slot].value;
    let induction = header.params[induction_slot].value;
    let len =
        result_of(&header.instructions[0], &u32_type()).ok_or("source output length missing")?;
    let loop_condition = result_of(&header.instructions[1], &bool_type())
        .ok_or("source output condition missing")?;
    if header.instructions[0].kind != (KirInstructionKind::SliceLen { slice: input })
        || !no_effect(&header.instructions[0])
        || header.instructions[1].kind
            != (KirInstructionKind::Compare {
                op: MirCompareOp::Lt,
                left: induction,
                right: len,
            })
        || !no_effect(&header.instructions[1])
    {
        return Err("source output bound is not the invariant input length".into());
    }
    let KirTerminator::Branch {
        condition,
        then_edge: body_edge,
        else_edge: exit_edge,
    } = &header.terminator
    else {
        return Err("source output loop header lost its single exit".into());
    };
    if *condition != loop_condition
        || !descriptor.blocks.contains(&body_edge.target)
        || descriptor.blocks.contains(&exit_edge.target)
        || exit_edge.target != plan.exit
        || !edge_forwards_memory_by_region(
            header,
            exit_edge,
            block(function, exit_edge.target).ok_or("source output exit block missing")?,
        )
    {
        return Err("source output loop iteration partition changed".into());
    }
    let root = block(function, body_edge.target).ok_or("source range branch missing")?;
    if root.instructions.len() != 2
        || result_of(&root.instructions[0], &f64_type()).is_none()
        || root.instructions[0].kind
            != (KirInstructionKind::ConstFloat {
                value: "0.0".into(),
            })
        || !no_effect(&root.instructions[0])
    {
        return Err("source zero comparison no longer uses positive zero".into());
    }
    if root.params.len() != header.params.len()
        || root
            .params
            .iter()
            .zip(&header.params)
            .any(|(actual, expected)| {
                actual.slot != expected.slot || actual.type_node != expected.type_node
            })
        || !edge_forwards_values_by_slot(header, body_edge, root)
    {
        return Err("source range branch changes header state".into());
    }
    let positive_zero = root.instructions[0].results[0].value;
    let range_param = root
        .params
        .get(range_slot)
        .ok_or("source range parameter missing")?
        .value;
    let root_condition =
        result_of(&root.instructions[1], &bool_type()).ok_or("source zero predicate missing")?;
    if root.instructions[1].kind
        != (KirInstructionKind::Compare {
            op: MirCompareOp::Eq,
            left: range_param,
            right: positive_zero,
        })
        || !no_effect(&root.instructions[1])
    {
        return Err("source branch must preserve exact range == +0 semantics".into());
    }
    let KirTerminator::Branch {
        condition: root_condition_edge,
        then_edge: zero_edge,
        else_edge: false_edge,
    } = &root.terminator
    else {
        return Err("source range branch no longer has two closed arms".into());
    };
    if *root_condition_edge != root_condition {
        return Err("source range branch condition changed".into());
    }
    let zero = block(function, zero_edge.target).ok_or("source zero arm missing")?;
    let false_arm = block(function, false_edge.target).ok_or("source nonzero arm missing")?;
    if zero.id != plan.zero_arm
        || false_arm.id != plan.false_arm
        || zero.instructions.len() != 1
        || false_arm.instructions.len() != 4
    {
        return Err("source normalization arms gained operations or changed identity".into());
    }
    if !is_exact_store(
        &zero.instructions[0],
        zero.params[output_slot].value,
        zero.params[induction_slot].value,
        positive_zero,
    ) || zero.instructions[0]
        .effect
        .as_ref()
        .is_none_or(|effect| effect.kind != KirEffectKind::WriteMemory)
        || zero
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
    {
        return Err("source zero arm must store positive zero without an input load".into());
    }
    let [load, subtract, divide, store] = false_arm.instructions.as_slice() else {
        return Err("source false arm instruction sequence changed".into());
    };
    let loaded = result_of(load, &f64_type()).ok_or("source input load changed")?;
    let difference = result_of(subtract, &f64_type()).ok_or("source subtraction changed")?;
    let normalized = result_of(divide, &f64_type()).ok_or("source division changed")?;
    let KirInstructionKind::Load { place } = &load.kind else {
        return Err("source false arm must load input first".into());
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        region,
        type_node,
    } = place.as_ref()
    else {
        return Err("source false arm input access changed".into());
    };
    if *slice != false_arm.params[input_slot].value
        || *index != false_arm.params[induction_slot].value
        || *type_node != MirType::Primitive(MirPrimitiveTypeName::F64)
        || load
            .memory
            .as_ref()
            .is_none_or(|memory| memory.region != *region)
        || load
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
    {
        return Err("source input access, order or MemorySSA changed".into());
    }
    if subtract.kind
        != (KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: loaded,
            right: false_arm.params[minimum_slot].value,
            semantics: KirArithmeticSemantics::StrictFloat,
        })
        || divide.kind
            != (KirInstructionKind::Binary {
                op: MirBinaryOp::Div,
                left: difference,
                right: false_arm.params[range_slot].value,
                semantics: KirArithmeticSemantics::StrictFloat,
            })
        || !is_exact_store(
            store,
            false_arm.params[output_slot].value,
            false_arm.params[induction_slot].value,
            normalized,
        )
        || store
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::WriteMemory)
        || [subtract, divide]
            .iter()
            .any(|instruction| !no_effect(instruction))
    {
        return Err("source false arm strict operation order, range or minimum changed".into());
    }
    let latch = block(
        function,
        descriptor.latch.ok_or("source output latch missing")?,
    )
    .ok_or("source output latch missing")?;
    if latch.id != plan.latch || latch.instructions.len() != 1 {
        return Err("source output latch gained effects or changed".into());
    }
    let increment = &latch.instructions[0];
    let increment_value =
        result_of(increment, &u32_type()).ok_or("source loop counter update missing")?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &increment.kind
    else {
        return Err("source loop counter is not modular +1".into());
    };
    let one = if *left == latch.params[induction_slot].value {
        *right
    } else if *right == latch.params[induction_slot].value {
        *left
    } else {
        return Err("source loop counter update changed operand".into());
    };
    if constant_u32(function, one) != Some(1) || !no_effect(increment) {
        return Err("source loop counter increment changed".into());
    }
    let KirTerminator::Jump { edge: backedge } = &latch.terminator else {
        return Err("source output latch gained an extra exit".into());
    };
    if backedge.target != header.id
        || backedge.args.len() != header.params.len()
        || backedge.args[induction_slot] != increment_value
        || backedge
            .args
            .iter()
            .enumerate()
            .any(|(slot, value)| slot != induction_slot && *value != latch.params[slot].value)
    {
        return Err("source output backedge changes bound, range or minimum".into());
    }
    if !edge_forwards_values_by_slot(root, zero_edge, zero)
        || !edge_forwards_values_by_slot(root, false_edge, false_arm)
        || jump_edge(&zero.terminator)
            .is_none_or(|edge| !edge_forwards_values_by_slot(zero, edge, latch))
        || jump_edge(&false_arm.terminator)
            .is_none_or(|edge| !edge_forwards_values_by_slot(false_arm, edge, latch))
        || !edge_forwards_memory_by_region(latch, backedge, header)
    {
        return Err("source branch arms no longer join the one shared latch".into());
    }
    Ok(CheckerSource {
        descriptor,
        preheader: preheader_id,
        zero: zero.id,
        false_arm: false_arm.id,
        latch: latch.id,
        exit: exit_edge.target,
        input_slot,
        induction_slot,
        incoming: incoming.clone(),
        exit_edge: exit_edge.clone(),
        range_value: range_result,
        one,
    })
}

fn check_pure_result(
    instruction: &KirInstruction,
    kind: KirInstructionKind,
    ty: &KirValueType,
    expected_id: InstructionId,
) -> Result<ValueId, String> {
    if instruction.id != expected_id || instruction.kind != kind || !no_effect(instruction) {
        return Err("normalization unswitch generated operation changed".into());
    }
    result_of(instruction, ty).ok_or_else(|| "normalization unswitch result type changed".into())
}

fn check_cloned_parameters(
    source: &KirBlock,
    cloned: &KirBlock,
    original: &KirFunction,
) -> Result<(), String> {
    if source.params.len() != cloned.params.len()
        || source.memory_params.len() != cloned.memory_params.len()
        || source
            .params
            .iter()
            .zip(&cloned.params)
            .any(|(before, after)| {
                before.slot != after.slot
                    || before.type_node != after.type_node
                    || before.value == after.value
            })
        || source
            .memory_params
            .iter()
            .zip(&cloned.memory_params)
            .any(|(before, after)| before.region != after.region || before.version == after.version)
        || cloned.params.iter().any(|param| {
            original.blocks.iter().any(|block| {
                block.params.iter().any(|old| old.value == param.value)
                    || block.instructions.iter().any(|instruction| {
                        instruction
                            .results
                            .iter()
                            .any(|result| result.value == param.value)
                    })
            })
        })
    {
        return Err("normalization cloned parameter or MemorySSA identity changed".into());
    }
    Ok(())
}

fn verify_cloned_instructions(
    source: &KirBlock,
    cloned: &KirBlock,
    mapping: &[(InstructionId, InstructionId)],
    inherited_values: &BTreeMap<ValueId, ValueId>,
    mut next_effect: u32,
    original: &KirFunction,
) -> Result<CheckedClone, String> {
    if source.instructions.len() != mapping.len()
        || cloned.instructions.len() < source.instructions.len()
        || mapping
            .iter()
            .zip(&source.instructions)
            .any(|((source_id, _), instruction)| *source_id != instruction.id)
    {
        return Err("normalization clone instruction mapping coverage is false".into());
    }
    let mut values = inherited_values.clone();
    for (before, after) in source.params.iter().zip(&cloned.params) {
        values.insert(before.value, after.value);
    }
    let mut memories = source
        .memory_params
        .iter()
        .zip(&cloned.memory_params)
        .map(|(before, after)| (before.region, after.version))
        .collect::<BTreeMap<_, _>>();
    let old_values = original
        .params
        .iter()
        .map(|param| param.value)
        .chain(original.blocks.iter().flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        }))
        .collect::<BTreeSet<_>>();
    let old_memory = original
        .initial_memory
        .iter()
        .map(|param| param.version)
        .chain(original.blocks.iter().flat_map(|block| {
            block.memory_params.iter().map(|param| param.version).chain(
                block.instructions.iter().flat_map(|instruction| {
                    instruction.memory.iter().flat_map(|memory| {
                        [Some(memory.input), memory.output].into_iter().flatten()
                    })
                }),
            )
        }))
        .collect::<BTreeSet<_>>();
    let mut fresh_values = BTreeSet::new();
    let mut fresh_memories = BTreeSet::new();
    for (index, (before, after)) in source
        .instructions
        .iter()
        .zip(&cloned.instructions[..source.instructions.len()])
        .enumerate()
    {
        let expected_kind = remap_kind_for_checker(&before.kind, &values)
            .ok_or("normalization source clone contains an unsupported instruction")?;
        if after.kind != expected_kind
            || after.results.len() != before.results.len()
            || after
                .results
                .iter()
                .zip(&before.results)
                .any(|(new, old)| new.type_node != old.type_node)
        {
            return Err("normalization clone changes a strict operation or operand".into());
        }
        if before.id == after.id || after.id != mapping[index].1 {
            return Err("normalization clone instruction identity is false".into());
        }
        for (old_result, new_result) in before.results.iter().zip(&after.results) {
            if old_values.contains(&new_result.value) || !fresh_values.insert(new_result.value) {
                return Err("normalization clone reuses an existing value identity".into());
            }
            values.insert(old_result.value, new_result.value);
        }
        match (&before.memory, &after.memory) {
            (None, None) => {}
            (Some(old), Some(new)) => {
                if new.region != old.region
                    || memories.get(&old.region).copied() != Some(new.input)
                    || old.output.is_some() != new.output.is_some()
                {
                    return Err("normalization clone MemorySSA input/output changed".into());
                }
                if let Some(output) = new.output {
                    if old_memory.contains(&output) || !fresh_memories.insert(output) {
                        return Err("normalization clone reuses a MemorySSA identity".into());
                    }
                    memories.insert(old.region, output);
                }
            }
            _ => return Err("normalization clone adds or removes a memory access".into()),
        }
        match (&before.effect, &after.effect) {
            (None, None) => {}
            (Some(old), Some(new)) => {
                if new.kind != old.kind || new.order != next_effect {
                    return Err("normalization clone effect order or kind changed".into());
                }
                next_effect = next_effect.checked_add(1).ok_or("effect order overflow")?;
            }
            _ => return Err("normalization clone adds or removes an ordered effect".into()),
        }
    }
    for (old, new) in source.memory_params.iter().zip(&cloned.memory_params) {
        if old.region != new.region
            || old.version == new.version
            || old_memory.contains(&new.version)
        {
            return Err("normalization cloned MemorySSA parameter is not fresh".into());
        }
    }
    for memory in cloned
        .instructions
        .iter()
        .filter_map(|instruction| instruction.memory.as_ref())
    {
        if memory
            .output
            .is_some_and(|output| old_memory.contains(&output) || !fresh_memories.contains(&output))
        {
            return Err("normalization clone output MemorySSA identity is invalid".into());
        }
    }
    Ok(CheckedClone {
        values,
        memories,
        next_effect_order: next_effect,
    })
}

fn remap_kind_for_checker(
    kind: &KirInstructionKind,
    values: &BTreeMap<ValueId, ValueId>,
) -> Option<KirInstructionKind> {
    Some(match kind {
        KirInstructionKind::ConstInt { value } => KirInstructionKind::ConstInt {
            value: value.clone(),
        },
        KirInstructionKind::ConstFloat { value } => KirInstructionKind::ConstFloat {
            value: value.clone(),
        },
        KirInstructionKind::ConstBool { value } => KirInstructionKind::ConstBool { value: *value },
        KirInstructionKind::SliceLen { slice } => KirInstructionKind::SliceLen {
            slice: mapped(*slice, values),
        },
        KirInstructionKind::Copy { value } => KirInstructionKind::Copy {
            value: mapped(*value, values),
        },
        KirInstructionKind::Compare { op, left, right } => KirInstructionKind::Compare {
            op: *op,
            left: mapped(*left, values),
            right: mapped(*right, values),
        },
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } => KirInstructionKind::Binary {
            op: *op,
            left: mapped(*left, values),
            right: mapped(*right, values),
            semantics: *semantics,
        },
        KirInstructionKind::Load { place } => KirInstructionKind::Load {
            place: Box::new(remap_place_for_checker(place, values)?),
        },
        KirInstructionKind::Store { place, value } => KirInstructionKind::Store {
            place: Box::new(remap_place_for_checker(place, values)?),
            value: mapped(*value, values),
        },
        _ => return None,
    })
}

fn remap_place_for_checker(
    place: &KirPlace,
    values: &BTreeMap<ValueId, ValueId>,
) -> Option<KirPlace> {
    Some(match place {
        KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } => KirPlace::SliceIndex {
            slice: mapped(*slice, values),
            index: mapped(*index, values),
            type_node: type_node.clone(),
            region: *region,
        },
        _ => return None,
    })
}

fn successor_edges(terminator: &KirTerminator) -> Vec<&crate::KirEdge> {
    match terminator {
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        KirTerminator::Return { .. } => Vec::new(),
    }
}

fn zero_path_contains_load(function: &KirFunction, source: &CheckerSource) -> bool {
    block(function, source.zero).is_none_or(|zero| {
        zero.instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
    })
}

#[derive(Debug, Default)]
pub struct NormalizationUnswitchFrontierResult {
    pub accepted: u32,
    pub rejected: u32,
    pub fallbacks: Vec<crate::KirAnalysisFallback>,
}

/// Try each strict normalization output loop at most once in this frontier.
/// Rejected proposals retain the original scalar loop through the verified
/// transaction mechanism; the cloned false loop remains scalar until a later
/// vector transaction supplies alias and WASM slice-range proofs.
pub fn run_normalization_unswitch_frontier(
    state: &mut KirVerifiedProgramState,
    audit: &mut crate::KirOptimizationAuditState,
) -> Result<NormalizationUnswitchFrontierResult, String> {
    let mut result = NormalizationUnswitchFrontierResult::default();
    let mut processed = BTreeSet::new();
    loop {
        let mut candidates = discover_normalization_unswitch_candidates(state);
        candidates.sort_by_key(|candidate| (candidate.function, candidate.header));
        let Some(candidate) = candidates
            .into_iter()
            .find(|candidate| processed.insert((candidate.function, candidate.header)))
        else {
            break;
        };
        let key = crate::CandidateKey::LoopFrontier {
            function: candidate.function,
            loop_id: candidate.loop_id,
            kind: crate::LoopCandidateKind::NormalizationUnswitch,
            variant: crate::LoopCandidateVariant::Scalar,
            vf: 1,
            uf: 1,
        };
        let prepared = match prepare_normalization_unswitch_trial(state, &candidate) {
            Ok(prepared) => prepared,
            Err(reason) => {
                audit.record_noncommitting_attempt(
                    key,
                    CandidateBudgetCharge::single(candidate.function, 16, 32),
                    crate::CandidateDisposition::Rejected,
                    &reason,
                )?;
                result.rejected = result.rejected.saturating_add(1);
                result.fallbacks.push(crate::KirAnalysisFallback {
                    function: candidate.function,
                    pass: "normalization-unswitch".into(),
                    reason,
                });
                continue;
            }
        };
        let plan = prepared.plan;
        let charge = prepared.charge;
        let proposed = prepared.trial;
        match crate::execute_verified_transaction(
            state,
            audit,
            key,
            charge.clone(),
            move |trial| {
                *trial = proposed;
                Ok(())
            },
            |before, after| {
                check_normalization_unswitch_independently(before, after, &plan, &charge)
            },
        ) {
            crate::TransactionOutcome::Committed => {
                result.accepted = result.accepted.saturating_add(1);
            }
            crate::TransactionOutcome::Rejected | crate::TransactionOutcome::BudgetExhausted => {
                result.rejected = result.rejected.saturating_add(1);
                result.fallbacks.push(crate::KirAnalysisFallback {
                    function: candidate.function,
                    pass: "normalization-unswitch".into(),
                    reason: "independent-check-or-budget-rejected".into(),
                });
            }
            crate::TransactionOutcome::CompilerError(error) => return Err(error),
        }
    }
    Ok(result)
}
