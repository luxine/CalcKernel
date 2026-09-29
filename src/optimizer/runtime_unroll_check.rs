use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AliasKind, BlockId, CandidateBudgetCharge, FunctionId, InstructionId, KirAlignmentClass,
    KirArithmeticSemantics, KirBlock, KirCostEstimate, KirCostKey, KirCostSemantics, KirEffectKind,
    KirInstruction, KirInstructionKind, KirLaneType, KirOperationAvailability, KirPlace,
    KirPreStateIdentity, KirProfileOperation, KirTerminator, KirValueType, KirVerifiedProgramState,
    LoopCandidateKind, LoopCandidateVariant, LoopTripCount, MemoryRegionId, MirBinaryOp,
    MirCompareOp, MirPrimitiveTypeName, MirType, RuntimeScalarUnrollCandidate,
    RuntimeScalarUnrollKind, TransactionCheckError, ValueId, VectorPlanGrowth,
    analyze_canonical_loops, analyze_regions, kir_function_units, query_alias, validate_kir_module,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeScalarUnrollPlan {
    pub pre_state: KirPreStateIdentity,
    pub candidate: RuntimeScalarUnrollCandidate,
    pub fast_entry: BlockId,
    pub main_header: BlockId,
    pub main_body: BlockId,
    pub dispatch_bound: ValueId,
    pub dispatch_guard: ValueId,
    pub dispatch_guard_instruction: InstructionId,
    pub limit_remainder: ValueId,
    pub limit_value: ValueId,
    pub limit_remainder_instruction: InstructionId,
    pub limit_instruction: InstructionId,
    pub main_condition: ValueId,
    pub main_condition_instruction: InstructionId,
    pub lane_induction_values: Vec<ValueId>,
    pub lane_index_instruction_ids: Vec<InstructionId>,
    pub group_induction_value: ValueId,
    pub group_induction_instruction: InstructionId,
    pub induction_aux_instruction_ids: Vec<InstructionId>,
    pub lane_instruction_ids: Vec<Vec<InstructionId>>,
    pub lane_result_values: Vec<Vec<ValueId>>,
    pub kind: RuntimeScalarUnrollKind,
    pub factor: u8,
    pub minimum_trip: u32,
    pub cost: KirCostEstimate,
    pub growth: VectorPlanGrowth,
}

#[derive(Debug, Clone)]
pub struct PreparedRuntimeScalarUnroll {
    pub trial: KirVerifiedProgramState,
    pub plan: RuntimeScalarUnrollPlan,
    pub charge: CandidateBudgetCharge,
}

pub fn check_wasm_runtime_scalar_unroll_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &RuntimeScalarUnrollPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    let source_validation = validate_kir_module(pre_state.module());
    if !source_validation.errors.is_empty() {
        return Err(TransactionCheckError::compiler(format!(
            "runtime scalar UF4 source state is invalid: {:?}",
            source_validation.errors
        )));
    }
    check_trial(pre_state, trial, plan, charge).map_err(TransactionCheckError::reject)
}

fn check_trial(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &RuntimeScalarUnrollPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), String> {
    check_state_identity(pre_state, trial, plan)?;
    let original = function(pre_state, plan.candidate.function)?;
    let transformed = function(trial, plan.candidate.function)?;
    let source = source_shape(pre_state, original, &plan.candidate)?;
    if plan.kind != plan.candidate.kind
        || plan.factor != 4
        || plan.factor != plan.candidate.factor
        || plan.minimum_trip != plan.candidate.minimum_trip
        || plan.cost != plan.candidate.predicted_cost
    {
        return Err("runtime scalar UF4 plan metadata is inconsistent".into());
    }
    check_function_diff(pre_state, original, transformed, plan)?;
    check_dispatch(transformed, &source, plan)?;
    check_main_header(transformed, &source, plan)?;
    check_lanes(transformed, &source, plan)?;
    check_cost_and_charge(pre_state, original, transformed, plan, charge)?;
    let validation = validate_kir_module(trial.module());
    if !validation.errors.is_empty() {
        return Err(format!(
            "runtime scalar UF4 produced invalid KIR: {:?}",
            validation.errors
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Source<'a> {
    preheader: &'a KirBlock,
    header: &'a KirBlock,
    body: &'a KirBlock,
    incoming: &'a crate::KirEdge,
    then_edge: &'a crate::KirEdge,
    backedge: &'a crate::KirEdge,
    induction_index: usize,
    bound_slice: ValueId,
}

fn check_state_identity(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &RuntimeScalarUnrollPlan,
) -> Result<(), String> {
    let before = pre.module();
    let after = trial.module();
    if before.config.consumer != crate::KirConsumer::WebAssembly
        || before.profile.wasm_features() != Some(crate::KirWasmFeatures::Baseline)
        || before.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || before.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || before.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
        || before.config != after.config
        || before.profile != after.profile
        || before.structs != after.structs
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
        || pre.evidence_generation() != trial.evidence_generation()
        || pre.optimization_entry_module_units() != trial.optimization_entry_module_units()
        || plan.pre_state.function != plan.candidate.function
        || plan.pre_state.kir_digest != pre.kir_digest()
        || plan.pre_state.profile_digest != before.profile.digest_hex()
        || plan.pre_state.evidence_generation != pre.evidence_generation()
    {
        return Err("runtime scalar UF4 pre-state identity or evidence changed".into());
    }
    if before.functions.len() != after.functions.len()
        || before
            .functions
            .iter()
            .filter(|function| function.id != plan.candidate.function)
            .any(|function| {
                after
                    .functions
                    .iter()
                    .find(|candidate| candidate.id == function.id)
                    != Some(function)
            })
    {
        return Err("runtime scalar UF4 changed another module function".into());
    }
    Ok(())
}

fn source_shape<'a>(
    pre: &KirVerifiedProgramState,
    function: &'a crate::KirFunction,
    candidate: &RuntimeScalarUnrollCandidate,
) -> Result<Source<'a>, String> {
    if candidate.function != function.id
        || candidate.factor != 4
        || candidate.key
            != (crate::CandidateKey::LoopFrontier {
                function: candidate.function,
                loop_id: candidate.loop_id,
                kind: LoopCandidateKind::RuntimeScalarUnroll,
                variant: LoopCandidateVariant::Scalar,
                vf: 1,
                uf: 4,
            })
    {
        return Err("runtime scalar UF4 candidate key or factor is false".into());
    }
    let descriptor = analyze_canonical_loops(function)
        .loops
        .into_iter()
        .find(|item| item.id == candidate.loop_id && item.header == candidate.header)
        .ok_or_else(|| "runtime scalar UF4 source loop is missing".to_string())?;
    let induction = descriptor
        .induction
        .as_ref()
        .ok_or_else(|| "runtime scalar UF4 source induction is missing".to_string())?;
    if !descriptor.innermost
        || !descriptor.dedicated_exits
        || !descriptor.lcssa
        || descriptor.blocks.len() != 2
        || descriptor.preheader != Some(candidate.preheader)
        || descriptor.latch != Some(candidate.body)
        || descriptor.exits != [candidate.exit]
        || !matches!(descriptor.trip_count, LoopTripCount::Runtime { .. })
        || induction.type_node != crate::IntegerType::U32
        || induction.start != num_bigint::BigInt::from(0_u8)
        || induction.step != num_bigint::BigInt::from(1_u8)
        || induction.comparison != MirCompareOp::Lt
        || !induction.wrap_safe_for_strict_bound
        || induction.value != candidate.induction
        || induction.bound != candidate.bound
    {
        return Err("runtime scalar UF4 source loop preconditions are false".into());
    }
    let preheader = find_block(function, candidate.preheader)?;
    let header = find_block(function, candidate.header)?;
    let body = find_block(function, candidate.body)?;
    let exit = find_block(function, candidate.exit)?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return Err("runtime scalar UF4 source preheader is not a jump".into());
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return Err("runtime scalar UF4 source header is not conditional".into());
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err("runtime scalar UF4 source body is not a backedge".into());
    };
    if incoming.target != header.id
        || then_edge.target != body.id
        || else_edge.target != exit.id
        || backedge.target != header.id
        || incoming.args.len() != header.params.len()
        || then_edge.args.len() != body.params.len()
        || backedge.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || then_edge.memory_args.len() != body.memory_params.len()
        || backedge.memory_args.len() != header.memory_params.len()
        || then_edge.args.iter().any(|value| {
            !header
                .params
                .iter()
                .any(|parameter| parameter.value == *value)
        })
    {
        return Err("runtime scalar UF4 source loop edges are not canonical".into());
    }
    let induction_index = header
        .params
        .iter()
        .position(|parameter| parameter.value == induction.value)
        .ok_or_else(|| "runtime scalar UF4 induction is not a header parameter".to_string())?;
    let bound_instruction = find_result_instruction(header, induction.bound)?;
    let KirInstructionKind::SliceLen { slice: bound_slice } = &bound_instruction.kind else {
        return Err("runtime scalar UF4 bound is not derived from SliceLen".into());
    };
    let bound_slice = *bound_slice;
    let compare_instruction = find_result_instruction(header, *condition)?;
    if candidate.bound_slice != bound_slice
        || !header
            .params
            .iter()
            .any(|parameter| parameter.value == bound_slice)
        || !matches!(&compare_instruction.kind,
            KirInstructionKind::Compare { op: MirCompareOp::Lt, left, right }
                if *left == induction.value && *right == induction.bound)
        || header.instructions.len() != 2
        || bound_instruction.memory.is_some()
        || bound_instruction.effect.is_some()
        || compare_instruction.memory.is_some()
        || compare_instruction.effect.is_some()
    {
        return Err("runtime scalar UF4 header bound or comparison is not source closed".into());
    }
    let body_induction_index = then_edge
        .args
        .iter()
        .position(|value| *value == induction.value)
        .ok_or_else(|| "runtime scalar UF4 induction is not passed to the body".to_string())?;
    let body_induction = body.params[body_induction_index].value;
    let update = body
        .instructions
        .last()
        .ok_or_else(|| "runtime scalar UF4 body is empty".to_string())?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &update.kind
    else {
        return Err("runtime scalar UF4 index update is not modular addition".into());
    };
    if *left != body_induction
        || const_u32(function, *right) != Some(1)
        || update.results.len() != 1
        || !is_type(&update.results[0].type_node, MirPrimitiveTypeName::U32)
        || backedge.args[induction_index] != update.results[0].value
    {
        return Err("runtime scalar UF4 index update is not a source-proven unit step".into());
    }
    let step_value = *right;
    let semantic = body.instructions[..body.instructions.len() - 1]
        .iter()
        .filter(|instruction| {
            !instruction
                .results
                .iter()
                .any(|result| result.value == step_value)
        })
        .collect::<Vec<_>>();
    for step_definition in body.instructions[..body.instructions.len() - 1]
        .iter()
        .filter(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == step_value)
        })
    {
        if !matches!(&step_definition.kind, KirInstructionKind::ConstInt { value } if value == "1")
            || step_definition.results.len() != 1
            || !is_type(
                &step_definition.results[0].type_node,
                MirPrimitiveTypeName::U32,
            )
        {
            return Err("runtime scalar UF4 step literal was changed".into());
        }
    }
    let body_context = SourceBodyContext {
        function,
        header,
        body,
        then_edge,
        backedge,
        induction_index,
        body_induction,
    };
    let recognized = match candidate.kind {
        RuntimeScalarUnrollKind::U32ModularSum => check_sum_source(&body_context, &semantic)?,
        RuntimeScalarUnrollKind::StrictF64DirectMap => {
            check_map_source(pre, &body_context, &semantic, candidate)?
        }
    };
    if recognized.input_slice != candidate.input_slice
        || recognized.input_region != candidate.input_region
        || recognized.output_slice != candidate.output_slice
        || recognized.output_region != candidate.output_region
        || recognized.accumulator != candidate.accumulator
        || recognized.noalias_fact != candidate.noalias_fact
        || candidate.induction_update != update.id
        || candidate.body_instructions
            != body
                .instructions
                .iter()
                .map(|instruction| instruction.id)
                .collect::<Vec<_>>()
    {
        return Err(
            "runtime scalar UF4 candidate does not match independent source reconstruction".into(),
        );
    }
    if recognized.input_slice != bound_slice {
        return Err("runtime scalar UF4 load domain differs from the loop bound slice".into());
    }
    let input_body_index = body
        .params
        .iter()
        .position(|parameter| parameter.value == recognized.input_body_slice)
        .ok_or_else(|| "runtime scalar UF4 input slice body parameter is missing".to_string())?;
    let input_header_index = header
        .params
        .iter()
        .position(|parameter| parameter.value == recognized.input_slice)
        .ok_or_else(|| "runtime scalar UF4 input slice header parameter is missing".to_string())?;
    if then_edge.args[input_body_index] != recognized.input_slice
        || backedge.args[input_header_index] != recognized.input_body_slice
    {
        return Err("runtime scalar UF4 input descriptor is not loop invariant".into());
    }
    if let (Some(output_body), Some(output_header)) =
        (recognized.output_body_slice, recognized.output_slice)
    {
        let body_index = body
            .params
            .iter()
            .position(|parameter| parameter.value == output_body)
            .ok_or_else(|| {
                "runtime scalar UF4 output slice body parameter is missing".to_string()
            })?;
        let header_index = header
            .params
            .iter()
            .position(|parameter| parameter.value == output_header)
            .ok_or_else(|| {
                "runtime scalar UF4 output slice header parameter is missing".to_string()
            })?;
        if then_edge.args[body_index] != output_header || backedge.args[header_index] != output_body
        {
            return Err("runtime scalar UF4 output descriptor is not loop invariant".into());
        }
    }
    check_source_memory_backedge(header, body, then_edge, backedge, candidate.kind)?;
    Ok(Source {
        preheader,
        header,
        body,
        incoming,
        then_edge,
        backedge,
        induction_index,
        bound_slice,
    })
}

fn check_source_memory_backedge(
    header: &KirBlock,
    body: &KirBlock,
    then_edge: &crate::KirEdge,
    backedge: &crate::KirEdge,
    kind: RuntimeScalarUnrollKind,
) -> Result<(), String> {
    let written = if kind == RuntimeScalarUnrollKind::StrictF64DirectMap {
        let store = body
            .instructions
            .iter()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
            .ok_or_else(|| "runtime scalar map source store is missing".to_string())?;
        let access = store
            .memory
            .as_ref()
            .ok_or_else(|| "runtime scalar map source store has no MemorySSA output".to_string())?;
        Some((
            access.region,
            access.output.ok_or_else(|| {
                "runtime scalar map source store has no MemorySSA version".to_string()
            })?,
        ))
    } else {
        None
    };
    if header.memory_params.len() != then_edge.memory_args.len()
        || header.memory_params.len() != backedge.memory_args.len()
        || body.memory_params.len() != then_edge.memory_args.len()
    {
        return Err("runtime scalar UF4 source MemorySSA edge arity changed".into());
    }
    for (index, header_memory) in header.memory_params.iter().enumerate() {
        let body_memory = &body.memory_params[index];
        if then_edge.memory_args[index] != header_memory.version
            || body_memory.region != header_memory.region
        {
            return Err("runtime scalar UF4 source MemorySSA entry does not map by region".into());
        }
        let expected = written
            .filter(|(region, _)| *region == header_memory.region)
            .map_or(body_memory.version, |(_, version)| version);
        if backedge.memory_args[index] != expected {
            return Err("runtime scalar UF4 source MemorySSA backedge is not closed".into());
        }
    }
    Ok(())
}

struct Recognized {
    input_slice: ValueId,
    input_body_slice: ValueId,
    input_region: MemoryRegionId,
    output_slice: Option<ValueId>,
    output_body_slice: Option<ValueId>,
    output_region: Option<MemoryRegionId>,
    accumulator: Option<ValueId>,
    noalias_fact: Option<crate::FactId>,
}

#[derive(Clone, Copy)]
struct SourceBodyContext<'a> {
    function: &'a crate::KirFunction,
    header: &'a KirBlock,
    body: &'a KirBlock,
    then_edge: &'a crate::KirEdge,
    backedge: &'a crate::KirEdge,
    induction_index: usize,
    body_induction: ValueId,
}

fn check_sum_source(
    source: &SourceBodyContext<'_>,
    semantic: &[&KirInstruction],
) -> Result<Recognized, String> {
    let SourceBodyContext {
        header,
        body,
        then_edge,
        backedge,
        induction_index,
        body_induction,
        ..
    } = *source;
    let [load, add] = semantic else {
        return Err("runtime scalar sum is not one load and one modular add".into());
    };
    let KirInstructionKind::Load { place } = &load.kind else {
        return Err("runtime scalar sum does not start with a load".into());
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node: MirType::Primitive(MirPrimitiveTypeName::U32),
        region,
    } = place.as_ref()
    else {
        return Err("runtime scalar sum load is not u32 slice indexing".into());
    };
    let [loaded] = load.results.as_slice() else {
        return Err("runtime scalar sum load result is malformed".into());
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &add.kind
    else {
        return Err("runtime scalar sum is not modular u32 addition".into());
    };
    let accumulator_index = body
        .params
        .iter()
        .position(|parameter| parameter.value == *left)
        .ok_or_else(|| "runtime scalar sum accumulator is not carried".to_string())?;
    let accumulator = then_edge.args[accumulator_index];
    let result = add
        .results
        .first()
        .ok_or_else(|| "runtime scalar sum add result is missing".to_string())?;
    if *index != body_induction
        || *right != loaded.value
        || !is_type(&loaded.type_node, MirPrimitiveTypeName::U32)
        || !is_type(&result.type_node, MirPrimitiveTypeName::U32)
        || load
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
        || load.memory.as_ref().is_none_or(|access| {
            access.output.is_some()
                || !body
                    .memory_params
                    .iter()
                    .any(|param| param.region == access.region && param.version == access.input)
        })
        || body.instructions.iter().any(|instruction| {
            instruction.id != load.id
                && (instruction.memory.is_some() || instruction.effect.is_some())
        })
        || header.params.iter().enumerate().any(|(index, parameter)| {
            if index == induction_index {
                return false;
            }
            let Some(back_value) = backedge.args.get(index) else {
                return true;
            };
            if parameter.value == accumulator {
                return *back_value != result.value;
            }
            then_edge
                .args
                .iter()
                .position(|arg| *arg == parameter.value)
                .is_none_or(|body_index| *back_value != body.params[body_index].value)
        })
    {
        return Err("runtime scalar sum has unexpected effect or loop-carried state".into());
    }
    let input_index = body
        .params
        .iter()
        .position(|parameter| parameter.value == *slice)
        .ok_or_else(|| "runtime scalar sum input slice is not invariant".to_string())?;
    Ok(Recognized {
        input_slice: then_edge.args[input_index],
        input_body_slice: *slice,
        input_region: *region,
        output_slice: None,
        output_body_slice: None,
        output_region: None,
        accumulator: Some(accumulator),
        noalias_fact: None,
    })
}

fn check_map_source(
    pre: &KirVerifiedProgramState,
    source: &SourceBodyContext<'_>,
    semantic: &[&KirInstruction],
    candidate: &RuntimeScalarUnrollCandidate,
) -> Result<Recognized, String> {
    let SourceBodyContext {
        function,
        header,
        body,
        then_edge,
        backedge,
        induction_index,
        body_induction,
    } = *source;
    let [load, mul_const, mul, add_const, add, store] = semantic else {
        return Err("runtime scalar map is not one strict load-mul-add-store chain".into());
    };
    let KirInstructionKind::Load { place: load_place } = &load.kind else {
        return Err("runtime scalar map load is missing".into());
    };
    let KirPlace::SliceIndex {
        slice: input_body,
        index: load_index,
        type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
        region: input_region,
    } = load_place.as_ref()
    else {
        return Err("runtime scalar map load is not f64 slice indexing".into());
    };
    let KirInstructionKind::ConstFloat { .. } = &mul_const.kind else {
        return Err("runtime scalar map multiply constant changed".into());
    };
    let loaded = load
        .results
        .first()
        .ok_or_else(|| "runtime scalar map load result is missing".to_string())?;
    let multiplier = mul_const
        .results
        .first()
        .ok_or_else(|| "runtime scalar map multiplier result is missing".to_string())?;
    if !matches!(&mul.kind, KirInstructionKind::Binary { op: MirBinaryOp::Mul, left, right, semantics: KirArithmeticSemantics::StrictFloat }
        if *left == loaded.value && *right == multiplier.value)
    {
        return Err("runtime scalar map multiply is not strict source order".into());
    }
    let product = mul
        .results
        .first()
        .ok_or_else(|| "runtime scalar map product is missing".to_string())?;
    let KirInstructionKind::ConstFloat { .. } = &add_const.kind else {
        return Err("runtime scalar map add constant changed".into());
    };
    let addend = add_const
        .results
        .first()
        .ok_or_else(|| "runtime scalar map addend is missing".to_string())?;
    if !matches!(&add.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left, right, semantics: KirArithmeticSemantics::StrictFloat }
        if *left == product.value && *right == addend.value)
    {
        return Err("runtime scalar map add is not strict source order".into());
    }
    let added = add
        .results
        .first()
        .ok_or_else(|| "runtime scalar map addition result is missing".to_string())?;
    let KirInstructionKind::Store {
        place: store_place,
        value,
    } = &store.kind
    else {
        return Err("runtime scalar map store is missing".into());
    };
    let KirPlace::SliceIndex {
        slice: output_body,
        index: store_index,
        type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
        region: output_region,
    } = store_place.as_ref()
    else {
        return Err("runtime scalar map store is not f64 slice indexing".into());
    };
    let load_memory = load
        .memory
        .as_ref()
        .ok_or_else(|| "runtime scalar map has no load MemorySSA input".to_string())?;
    let store_memory = store
        .memory
        .as_ref()
        .ok_or_else(|| "runtime scalar map has no store MemorySSA input".to_string())?;
    if *value != added.value
        || *load_index != body_induction
        || *store_index != body_induction
        || load
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
        || store
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::WriteMemory)
        || load_memory.output.is_some()
        || store_memory.output.is_none()
        || !body
            .memory_params
            .iter()
            .any(|param| param.region == load_memory.region && param.version == load_memory.input)
        || !body
            .memory_params
            .iter()
            .any(|param| param.region == store_memory.region && param.version == store_memory.input)
        || load.effect.as_ref().unwrap().order >= store.effect.as_ref().unwrap().order
        || body.instructions.iter().any(|instruction| {
            instruction.id != load.id
                && instruction.id != store.id
                && (instruction.memory.is_some() || instruction.effect.is_some())
        })
        || header.params.iter().enumerate().any(|(index, parameter)| {
            if index == induction_index {
                return false;
            }
            let Some(back_value) = backedge.args.get(index) else {
                return true;
            };
            then_edge
                .args
                .iter()
                .position(|arg| *arg == parameter.value)
                .is_none_or(|body_index| *back_value != body.params[body_index].value)
        })
    {
        return Err("runtime scalar map effects or carried state are not closed".into());
    }
    let input_index = body
        .params
        .iter()
        .position(|parameter| parameter.value == *input_body)
        .ok_or_else(|| "runtime scalar map input slice is not invariant".to_string())?;
    let output_index = body
        .params
        .iter()
        .position(|parameter| parameter.value == *output_body)
        .ok_or_else(|| "runtime scalar map output slice is not invariant".to_string())?;
    let input_slice = then_edge.args[input_index];
    let output_slice = then_edge.args[output_index];
    if input_slice != candidate.bound_slice {
        return Err("runtime scalar map bound does not come from its input slice".into());
    }
    let regions = analyze_regions(
        function,
        pre.contract_facts().map(crate::ContractFactSet::facts),
    )
    .map_err(|error| format!("runtime scalar map alias proof failed: {error:?}"))?;
    let alias = query_alias(&regions, *output_region, *input_region);
    let (AliasKind::NoAlias, Some(noalias_fact)) = (alias.kind, alias.fact) else {
        return Err("runtime scalar map lacks a source-derived noalias fact".into());
    };
    verify_trusted_noalias(pre, function, input_slice, output_slice, noalias_fact)?;
    Ok(Recognized {
        input_slice,
        input_body_slice: *input_body,
        input_region: *input_region,
        output_slice: Some(output_slice),
        output_body_slice: Some(*output_body),
        output_region: Some(*output_region),
        accumulator: None,
        noalias_fact: Some(noalias_fact),
    })
}

fn verify_trusted_noalias(
    state: &KirVerifiedProgramState,
    function: &crate::KirFunction,
    input: ValueId,
    output: ValueId,
    fact_id: crate::FactId,
) -> Result<(), String> {
    let fact = state
        .contract_facts()
        .and_then(|contracts| contracts.facts().get(fact_id))
        .ok_or_else(|| "runtime scalar map NoAlias fact is absent".to_string())?;
    let trusted_instance = match fact.origin {
        crate::FactOrigin::TrustedContract { instance } => Some(instance),
        crate::FactOrigin::Proven => None,
    };
    let source_contract_matches = state
        .contract_facts()
        .and_then(|contracts| {
            contracts
                .instances()
                .iter()
                .find(|instance| Some(instance.id) == trusted_instance)
        })
        .is_some_and(|instance| {
            instance.callee == function.id
                && instance.source == crate::ContractInstanceSource::FunctionEntry
                && instance.facts.contains(&fact_id)
        });
    let scope_available = matches!(
        fact.scope,
        crate::FactScope::FunctionEntry(owner) if owner == function.id
    );
    let input_root = stable_root(function, input);
    let output_root = stable_root(function, output);
    let predicate_matches = matches!(
        fact.predicate,
        crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias { left, right })
            if (stable_root(function, left) == input_root && stable_root(function, right) == output_root)
                || (stable_root(function, right) == input_root && stable_root(function, left) == output_root)
    );
    if !scope_available
        || !source_contract_matches
        || fact.generation != state.evidence_generation()
        || trusted_instance.is_none()
        || fact.derivation != crate::FactDerivation::TrustedContractLeaf
        || !predicate_matches
    {
        return Err(
            "runtime scalar map noalias proof is not a dominating trusted source contract".into(),
        );
    }
    Ok(())
}

fn stable_root(function: &crate::KirFunction, value: ValueId) -> Option<ValueId> {
    crate::optimizer::vectorize_check::stable_invariant_descriptor_root(function, value)
}

fn check_function_diff(
    pre: &KirVerifiedProgramState,
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    plan: &RuntimeScalarUnrollPlan,
) -> Result<(), String> {
    let mut expected = original
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    for id in [plan.fast_entry, plan.main_header, plan.main_body] {
        if !expected.insert(id) {
            return Err("runtime scalar UF4 generated block id is not fresh".into());
        }
    }
    let actual = transformed
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    if actual != expected
        || transformed.blocks.len() != expected.len()
        || transformed.regions != original.regions
        || transformed.initial_memory != original.initial_memory
        || transformed.vector_regions != original.vector_regions
        || transformed.params != original.params
        || transformed.return_type != original.return_type
        || transformed.name != original.name
        || transformed.exported != original.exported
    {
        return Err(
            "runtime scalar UF4 changed source function metadata or unrelated blocks".into(),
        );
    }
    for old in &original.blocks {
        let new = find_block(transformed, old.id)?;
        if old.id == plan.candidate.preheader {
            if new.label != old.label
                || new.params != old.params
                || new.memory_params != old.memory_params
                || new.instructions.len() != old.instructions.len() + 3
                || new.instructions[..old.instructions.len()] != old.instructions
            {
                return Err(
                    "runtime scalar UF4 changed preheader instructions or parameters".into(),
                );
            }
        } else if new != old {
            return Err(
                "runtime scalar UF4 modified the retained source loop or another source block"
                    .into(),
            );
        }
    }
    let before_modules = module_units(pre.module());
    if plan.growth.module_before_units != before_modules {
        return Err("runtime scalar UF4 module growth base is false".into());
    }
    Ok(())
}

fn check_dispatch(
    transformed: &crate::KirFunction,
    source: &Source<'_>,
    plan: &RuntimeScalarUnrollPlan,
) -> Result<(), String> {
    let preheader = find_block(transformed, source.preheader.id)?;
    let old_len = source.preheader.instructions.len();
    let appended = preheader
        .instructions
        .get(old_len..)
        .ok_or_else(|| "runtime scalar UF4 preheader lost source instructions".to_string())?;
    let [length, threshold, guard] = appended else {
        return Err("runtime scalar UF4 dispatch setup is not exactly three instructions".into());
    };
    let length_result = one_result(length)?;
    let threshold_result = one_result(threshold)?;
    if !matches!(&length.kind, KirInstructionKind::SliceLen { slice } if *slice == entry_value(source, source.bound_slice)?)
        || !matches!(&threshold.kind, KirInstructionKind::ConstInt { value } if value.parse::<u32>().ok() == Some(plan.minimum_trip))
        || !matches!(&guard.kind, KirInstructionKind::Compare { op: MirCompareOp::Ge, left, right }
            if *left == length_result && *right == threshold_result)
        || length_result != plan.dispatch_bound
        || one_result(guard)? != plan.dispatch_guard
        || guard.id != plan.dispatch_guard_instruction
        || [length, threshold, guard]
            .iter()
            .any(|instruction| instruction.memory.is_some() || instruction.effect.is_some())
    {
        return Err(
            "runtime scalar UF4 dispatch is not a non-trapping source-derived threshold".into(),
        );
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &preheader.terminator
    else {
        return Err("runtime scalar UF4 dispatch does not preserve a scalar fallback".into());
    };
    if *condition != plan.dispatch_guard
        || then_edge.target != plan.fast_entry
        || then_edge.args
            != source
                .incoming
                .args
                .iter()
                .copied()
                .chain([plan.dispatch_bound])
                .collect::<Vec<_>>()
        || then_edge.memory_args != source.incoming.memory_args
        || else_edge.target != source.header.id
        || else_edge.args != source.incoming.args
        || else_edge.memory_args != source.incoming.memory_args
    {
        return Err("runtime scalar UF4 dispatch edges changed scalar entry state".into());
    }
    let fast = find_block(transformed, plan.fast_entry)?;
    if fast.params.len() != source.header.params.len() + 1
        || fast.memory_params.len() != source.header.memory_params.len()
        || fast.instructions.len() != 3
    {
        return Err("runtime scalar UF4 limit preheader shape is false".into());
    }
    for (actual, expected) in fast.params.iter().zip(&source.header.params) {
        if actual.type_node != expected.type_node {
            return Err("runtime scalar UF4 fast-entry parameter type changed".into());
        }
    }
    let bound_param = fast
        .params
        .last()
        .map(|parameter| parameter.value)
        .ok_or_else(|| "runtime scalar UF4 bound parameter is missing".to_string())?;
    let four = one_result(&fast.instructions[0])?;
    if !matches!(&fast.instructions[0].kind, KirInstructionKind::ConstInt { value } if value == "4")
        || !matches!(&fast.instructions[1].kind, KirInstructionKind::Binary { op: MirBinaryOp::Mod, left, right, semantics: KirArithmeticSemantics::Modular }
            if *left == bound_param && *right == four)
        || one_result(&fast.instructions[1])? != plan.limit_remainder
        || fast.instructions[1].id != plan.limit_remainder_instruction
        || !matches!(&fast.instructions[2].kind, KirInstructionKind::Binary { op: MirBinaryOp::Sub, left, right, semantics: KirArithmeticSemantics::Modular }
            if *left == bound_param && *right == plan.limit_remainder)
        || one_result(&fast.instructions[2])? != plan.limit_value
        || fast.instructions[2].id != plan.limit_instruction
        || fast
            .instructions
            .iter()
            .any(|instruction| instruction.memory.is_some() || instruction.effect.is_some())
    {
        return Err("runtime scalar UF4 limit is not `bound - bound % 4`".into());
    }
    let KirTerminator::Jump { edge } = &fast.terminator else {
        return Err("runtime scalar UF4 fast-entry does not jump to the grouped loop".into());
    };
    let fast_values = fast.params[..source.header.params.len()]
        .iter()
        .map(|parameter| parameter.value)
        .collect::<Vec<_>>();
    if edge.target != plan.main_header
        || edge.args
            != fast_values
                .into_iter()
                .chain([plan.limit_value])
                .collect::<Vec<_>>()
        || edge.memory_args
            != fast
                .memory_params
                .iter()
                .map(|parameter| parameter.version)
                .collect::<Vec<_>>()
    {
        return Err("runtime scalar UF4 fast-entry does not preserve initial loop state".into());
    }
    Ok(())
}

fn check_main_header(
    transformed: &crate::KirFunction,
    source: &Source<'_>,
    plan: &RuntimeScalarUnrollPlan,
) -> Result<(), String> {
    let header = find_block(transformed, plan.main_header)?;
    let body = find_block(transformed, plan.main_body)?;
    if header.params.len() != source.header.params.len() + 1
        || header.memory_params.len() != source.header.memory_params.len()
        || body.params.len() != source.body.params.len()
        || body.memory_params.len() != source.body.memory_params.len()
        || header.instructions.len() != 1
        || body.instructions.len() != 8 + 4 * source_iteration_instructions(source, plan)?.len()
    {
        return Err("runtime scalar UF4 main-loop shape is false".into());
    }
    let limit = header
        .params
        .last()
        .map(|parameter| parameter.value)
        .ok_or_else(|| "runtime scalar UF4 grouped limit is missing".to_string())?;
    let induction = header.params[source.induction_index].value;
    if !matches!(&header.instructions[0].kind, KirInstructionKind::Compare { op: MirCompareOp::Lt, left, right }
        if *left == induction && *right == limit)
        || one_result(&header.instructions[0])? != plan.main_condition
        || header.instructions[0].id != plan.main_condition_instruction
        || header.instructions[0].memory.is_some()
        || header.instructions[0].effect.is_some()
    {
        return Err("runtime scalar UF4 group condition is not `index < limit`".into());
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return Err("runtime scalar UF4 main header is not conditional".into());
    };
    let grouped_values = header.params[..source.header.params.len()]
        .iter()
        .map(|parameter| parameter.value)
        .collect::<Vec<_>>();
    let grouped_memories = header
        .memory_params
        .iter()
        .map(|parameter| parameter.version)
        .collect::<Vec<_>>();
    let body_values = source
        .then_edge
        .args
        .iter()
        .map(|value| {
            source
                .header
                .params
                .iter()
                .position(|parameter| parameter.value == *value)
                .map(|index| header.params[index].value)
                .ok_or_else(|| {
                    "runtime scalar UF4 source body argument is not header invariant".to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if *condition != plan.main_condition
        || then_edge.target != plan.main_body
        || then_edge.args != body_values
        || then_edge.memory_args != grouped_memories
        || else_edge.target != source.header.id
        || else_edge.args != grouped_values
        || else_edge.memory_args != grouped_memories
    {
        return Err("runtime scalar UF4 group branch does not join the scalar tail".into());
    }
    for (actual, expected) in body.params.iter().zip(&source.body.params) {
        if actual.type_node != expected.type_node {
            return Err("runtime scalar UF4 generated body parameter type changed".into());
        }
    }
    Ok(())
}

fn check_lanes(
    transformed: &crate::KirFunction,
    source: &Source<'_>,
    plan: &RuntimeScalarUnrollPlan,
) -> Result<(), String> {
    let body = find_block(transformed, plan.main_body)?;
    let update = source
        .body
        .instructions
        .last()
        .filter(|instruction| instruction.id == plan.candidate.induction_update)
        .ok_or_else(|| "runtime scalar UF4 source induction update is missing".to_string())?;
    let update_result = update
        .results
        .first()
        .ok_or_else(|| "runtime scalar UF4 source induction update has no result".to_string())?
        .value;
    if source.backedge.args[source.induction_index] != update_result
        || source
            .backedge
            .args
            .iter()
            .filter(|value| **value == update_result)
            .count()
            != 1
    {
        return Err("runtime scalar UF4 source induction recurrence is not closed".into());
    }
    let iteration_instructions = source_iteration_instructions(source, plan)?;
    if plan.lane_instruction_ids.len() != 4
        || plan.lane_result_values.len() != 4
        || plan
            .lane_instruction_ids
            .iter()
            .any(|lane| lane.len() != iteration_instructions.len())
        || plan.lane_induction_values.len() != 4
        || plan.lane_index_instruction_ids.len() != 3
        || plan.induction_aux_instruction_ids.len() != 8
    {
        return Err("runtime scalar UF4 lane map is incomplete".into());
    }
    let header = find_block(transformed, plan.main_header)?;
    let body_induction_index = body_induction_index(source)?;
    let group_base = body
        .params
        .get(body_induction_index)
        .ok_or_else(|| {
            "runtime scalar UF4 grouped induction body parameter is missing".to_string()
        })?
        .value;
    if plan.lane_induction_values[0] != group_base {
        return Err("runtime scalar UF4 lane zero is not the group induction base".into());
    }
    let mut expected_aux_ids = Vec::with_capacity(8);
    let mut lane_values = vec![group_base];
    for offset in 1..4_u32 {
        let constant_index = ((offset - 1) * 2) as usize;
        let add_index = constant_index + 1;
        let constant_id = plan.induction_aux_instruction_ids[constant_index];
        let constant = check_generated_u32_constant(body, constant_index, constant_id, offset)?;
        let add_id = plan.induction_aux_instruction_ids[add_index];
        let value = check_generated_modular_add(body, add_index, add_id, group_base, constant)?;
        if plan.lane_index_instruction_ids[(offset - 1) as usize] != add_id
            || plan.lane_induction_values[offset as usize] != value
        {
            return Err("runtime scalar UF4 lane index proof metadata is false".into());
        }
        expected_aux_ids.extend([constant_id, add_id]);
        lane_values.push(value);
    }
    let grouped_lane_start: usize = 6;
    let grouped_lane_end = grouped_lane_start
        .checked_add(4_usize.saturating_mul(iteration_instructions.len()))
        .ok_or_else(|| "runtime scalar UF4 instruction count overflowed".to_string())?;
    if body.instructions.len() != grouped_lane_end + 2 {
        return Err("runtime scalar UF4 body contains unexpected instructions".into());
    }
    let group_constant_index = grouped_lane_end;
    let group_add_index = grouped_lane_end + 1;
    let group_constant_id = plan.induction_aux_instruction_ids[6];
    let group_step =
        check_generated_u32_constant(body, group_constant_index, group_constant_id, 4)?;
    let group_add_id = plan.induction_aux_instruction_ids[7];
    let group_result =
        check_generated_modular_add(body, group_add_index, group_add_id, group_base, group_step)?;
    if plan.group_induction_instruction != group_add_id
        || plan.group_induction_value != group_result
    {
        return Err("runtime scalar UF4 group induction proof metadata is false".into());
    }
    expected_aux_ids.extend([group_constant_id, group_add_id]);
    if plan.induction_aux_instruction_ids != expected_aux_ids {
        return Err("runtime scalar UF4 auxiliary induction instruction map is false".into());
    }
    let mut current_values = source
        .header
        .params
        .iter()
        .zip(&header.params)
        .map(|(source, target)| (source.value, target.value))
        .collect::<BTreeMap<_, _>>();
    let mut current_memories = source
        .header
        .memory_params
        .iter()
        .zip(&header.memory_params)
        .map(|(source, target)| (source.version, target.version))
        .collect::<BTreeMap<_, _>>();
    let mut effect_orders = BTreeSet::new();
    let mut previous_effect = None;
    for (lane, lane_induction_value) in lane_values.iter().enumerate() {
        let mut values = BTreeMap::new();
        let mut memories = BTreeMap::new();
        if lane == 0 {
            for (source_param, target_param) in source.body.params.iter().zip(&body.params) {
                values.insert(source_param.value, target_param.value);
            }
            for (source_param, target_param) in
                source.body.memory_params.iter().zip(&body.memory_params)
            {
                memories.insert(source_param.version, target_param.version);
            }
        } else {
            for (source_param, arg) in source.body.params.iter().zip(&source.then_edge.args) {
                values.insert(
                    source_param.value,
                    *current_values
                        .get(arg)
                        .ok_or_else(|| "runtime scalar UF4 lane state is missing".to_string())?,
                );
            }
            for (source_param, arg) in source
                .body
                .memory_params
                .iter()
                .zip(&source.then_edge.memory_args)
            {
                memories.insert(
                    source_param.version,
                    *current_memories.get(arg).ok_or_else(|| {
                        "runtime scalar UF4 lane memory state is missing".to_string()
                    })?,
                );
            }
        }
        seed_preheader_constants(source.preheader, &mut values);
        values.insert(
            source.body.params[body_induction_index].value,
            *lane_induction_value,
        );
        let mut lane_results = Vec::new();
        for (index, source_instruction) in iteration_instructions.iter().enumerate() {
            let actual = body
                .instructions
                .get(grouped_lane_start + lane * iteration_instructions.len() + index)
                .ok_or_else(|| {
                    "runtime scalar UF4 emitted lane instruction is missing".to_string()
                })?;
            if actual.id != plan.lane_instruction_ids[lane][index]
                || actual.id == source_instruction.id
            {
                return Err("runtime scalar UF4 emitted instruction mapping is stale".into());
            }
            let expected_kind = checker_remap_kind(&source_instruction.kind, &values).map_err(|error| {
                format!(
                    "{error} while independently mapping source instruction i{} on lane {lane}: {:?}",
                    source_instruction.id.index(),
                    source_instruction.kind,
                )
            })?;
            if actual.kind != expected_kind
                || actual.results.len() != source_instruction.results.len()
            {
                return Err(format!(
                    "runtime scalar UF4 lane {lane} changed source operation order or operands"
                ));
            }
            for (expected_result, actual_result) in
                source_instruction.results.iter().zip(&actual.results)
            {
                if expected_result.type_node != actual_result.type_node {
                    return Err("runtime scalar UF4 lane result type changed".into());
                }
                values.insert(expected_result.value, actual_result.value);
                lane_results.push(actual_result.value);
            }
            match (&source_instruction.memory, &actual.memory) {
                (None, None) => {}
                (Some(expected), Some(actual)) => {
                    if memories.get(&expected.input).copied() != Some(actual.input)
                        || expected.region != actual.region
                    {
                        return Err("runtime scalar UF4 lane MemorySSA input changed".into());
                    }
                    match expected.output {
                        None if actual.output.is_none() => {}
                        Some(output) => {
                            let actual_output = actual.output.ok_or_else(|| {
                                "runtime scalar UF4 lane dropped a store".to_string()
                            })?;
                            memories.insert(output, actual_output);
                        }
                        _ => return Err("runtime scalar UF4 lane introduced a memory write".into()),
                    }
                }
                _ => return Err("runtime scalar UF4 lane effect shape changed".into()),
            }
            match (&source_instruction.effect, &actual.effect) {
                (None, None) => {}
                (Some(expected), Some(actual)) if expected.kind == actual.kind => {
                    if !effect_orders.insert(actual.order)
                        || previous_effect.is_some_and(|order| actual.order <= order)
                    {
                        return Err(
                            "runtime scalar UF4 reordered or duplicated load/store effects".into(),
                        );
                    }
                    previous_effect = Some(actual.order);
                }
                _ => return Err("runtime scalar UF4 changed ordered effect kind".into()),
            }
        }
        if lane_results != plan.lane_result_values[lane] {
            return Err("runtime scalar UF4 lane result mapping changed".into());
        }
        let mut next_values = BTreeMap::new();
        for (index, (param, arg)) in source
            .header
            .params
            .iter()
            .zip(&source.backedge.args)
            .enumerate()
        {
            if index == source.induction_index {
                next_values.insert(param.value, group_base);
                continue;
            }
            next_values.insert(
                param.value,
                *values.get(arg).ok_or_else(|| {
                    "runtime scalar UF4 source backedge value is unmapped".to_string()
                })?,
            );
        }
        let mut next_memories = BTreeMap::new();
        for (param, arg) in source
            .header
            .memory_params
            .iter()
            .zip(&source.backedge.memory_args)
        {
            next_memories.insert(
                param.version,
                *memories.get(arg).ok_or_else(|| {
                    "runtime scalar UF4 source memory backedge is unmapped".to_string()
                })?,
            );
        }
        current_values = next_values;
        current_memories = next_memories;
    }
    let limit = header
        .params
        .last()
        .map(|parameter| parameter.value)
        .ok_or_else(|| "runtime scalar UF4 grouped limit is absent".to_string())?;
    let KirTerminator::Jump { edge } = &body.terminator else {
        return Err("runtime scalar UF4 grouped body is not a loop backedge".into());
    };
    let values = source
        .header
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            if index == source.induction_index {
                plan.group_induction_value
            } else {
                current_values[&parameter.value]
            }
        })
        .chain([limit])
        .collect::<Vec<_>>();
    let memories = source
        .header
        .memory_params
        .iter()
        .map(|parameter| current_memories[&parameter.version])
        .collect::<Vec<_>>();
    if edge.target != plan.main_header || edge.args != values || edge.memory_args != memories {
        return Err("runtime scalar UF4 backedge does not carry the state after lane four".into());
    }
    Ok(())
}

fn body_induction_index(source: &Source<'_>) -> Result<usize, String> {
    source
        .then_edge
        .args
        .iter()
        .position(|value| *value == source.header.params[source.induction_index].value)
        .ok_or_else(|| "runtime scalar UF4 induction is not passed to its body".into())
}

fn source_iteration_instructions<'a>(
    source: &'a Source<'_>,
    plan: &RuntimeScalarUnrollPlan,
) -> Result<Vec<&'a KirInstruction>, String> {
    let update = source
        .body
        .instructions
        .last()
        .filter(|instruction| instruction.id == plan.candidate.induction_update)
        .ok_or_else(|| "runtime scalar UF4 source induction update is missing".to_string())?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &update.kind
    else {
        return Err("runtime scalar UF4 source induction update changed shape".into());
    };
    if *left != source.body.params[body_induction_index(source)?].value
        || source.backedge.args[source.induction_index]
            != update
                .results
                .first()
                .map(|result| result.value)
                .ok_or_else(|| {
                    "runtime scalar UF4 source induction update has no result".to_string()
                })?
    {
        return Err("runtime scalar UF4 source induction recurrence is not closed".into());
    }
    let local_step_definitions = source.body.instructions[..source.body.instructions.len() - 1]
        .iter()
        .filter(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == *right)
        })
        .collect::<Vec<_>>();
    if local_step_definitions.len() > 1
        || local_step_definitions.iter().any(|instruction| {
            !matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value.parse::<u32>().ok() == Some(1))
                || instruction.results.len() != 1
                || !is_type(&instruction.results[0].type_node, MirPrimitiveTypeName::U32)
                || instruction.memory.is_some()
                || instruction.effect.is_some()
        })
    {
        return Err("runtime scalar UF4 source step literal is not a pure unit constant".into());
    }
    Ok(source
        .body
        .instructions
        .iter()
        .filter(|instruction| {
            instruction.id != update.id
                && !instruction
                    .results
                    .iter()
                    .any(|result| result.value == *right)
        })
        .collect())
}

fn check_generated_u32_constant(
    block: &KirBlock,
    index: usize,
    expected_id: InstructionId,
    expected_value: u32,
) -> Result<ValueId, String> {
    let instruction = block
        .instructions
        .get(index)
        .ok_or_else(|| "runtime scalar UF4 generated constant is missing".to_string())?;
    if instruction.id != expected_id
        || instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
        || !is_type(&instruction.results[0].type_node, MirPrimitiveTypeName::U32)
        || !matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value.parse::<u32>().ok() == Some(expected_value))
    {
        return Err("runtime scalar UF4 generated induction constant is false".into());
    }
    Ok(instruction.results[0].value)
}

fn check_generated_modular_add(
    block: &KirBlock,
    index: usize,
    expected_id: InstructionId,
    expected_left: ValueId,
    expected_right: ValueId,
) -> Result<ValueId, String> {
    let instruction = block
        .instructions
        .get(index)
        .ok_or_else(|| "runtime scalar UF4 generated induction addition is missing".to_string())?;
    if instruction.id != expected_id
        || instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
        || !is_type(&instruction.results[0].type_node, MirPrimitiveTypeName::U32)
        || !matches!(&instruction.kind, KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } if *left == expected_left && *right == expected_right)
    {
        return Err("runtime scalar UF4 generated induction addition is false".into());
    }
    Ok(instruction.results[0].value)
}

fn seed_preheader_constants(preheader: &KirBlock, values: &mut BTreeMap<ValueId, ValueId>) {
    for instruction in &preheader.instructions {
        if matches!(
            instruction.kind,
            KirInstructionKind::ConstInt { .. } | KirInstructionKind::ConstFloat { .. }
        ) && instruction.memory.is_none()
            && instruction.effect.is_none()
            && instruction.results.len() == 1
        {
            let value = instruction.results[0].value;
            values.insert(value, value);
        }
    }
}

fn check_cost_and_charge(
    pre: &KirVerifiedProgramState,
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    plan: &RuntimeScalarUnrollPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), String> {
    let (minimum, cost) = checker_profitability(
        &pre.module().profile,
        source_body(original, plan.candidate.body)?,
        original,
    )?;
    if minimum != plan.minimum_trip || cost != plan.cost || cost != plan.candidate.predicted_cost {
        return Err("runtime scalar UF4 cost estimate or minimum trip is false".into());
    }
    let before_function = kir_function_units(original);
    let after_function = kir_function_units(transformed);
    let before_module = module_units(pre.module());
    let after_module = before_module
        .saturating_sub(before_function)
        .saturating_add(after_function);
    let expected_growth =
        VectorPlanGrowth::new(before_function, after_function, before_module, after_module);
    if expected_growth != plan.growth {
        return Err("runtime scalar UF4 growth accounting is false".into());
    }
    let mapped = plan
        .lane_instruction_ids
        .iter()
        .map(|lane| u32::try_from(lane.len()).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add)
        .saturating_add(
            u32::try_from(plan.induction_aux_instruction_ids.len()).unwrap_or(u32::MAX),
        );
    let expected_charge = CandidateBudgetCharge::single(
        plan.candidate.function,
        after_function
            .saturating_sub(before_function)
            .saturating_add(mapped)
            .saturating_add(16),
        before_module
            .saturating_add(after_module)
            .saturating_add(mapped.saturating_mul(2))
            .saturating_add(32),
    );
    if expected_charge != *charge {
        return Err("runtime scalar UF4 independent checker charge is false".into());
    }
    Ok(())
}

fn function(
    state: &KirVerifiedProgramState,
    id: FunctionId,
) -> Result<&crate::KirFunction, String> {
    state
        .module()
        .functions
        .iter()
        .find(|function| function.id == id)
        .ok_or_else(|| "runtime scalar UF4 function is missing".into())
}

fn find_block(function: &crate::KirFunction, id: BlockId) -> Result<&KirBlock, String> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("runtime scalar UF4 block b{} is missing", id.index()))
}

fn find_result_instruction(block: &KirBlock, value: ValueId) -> Result<&KirInstruction, String> {
    let found = block
        .instructions
        .iter()
        .filter(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
        .collect::<Vec<_>>();
    match found.as_slice() {
        [instruction] => Ok(*instruction),
        _ => Err("runtime scalar UF4 result has no unique local definition".into()),
    }
}

fn const_u32(function: &crate::KirFunction, value: ValueId) -> Option<u32> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find_map(|instruction| {
            if instruction.results.len() != 1 || instruction.results[0].value != value {
                return None;
            }
            match &instruction.kind {
                KirInstructionKind::ConstInt { value } => value.parse().ok(),
                _ => None,
            }
        })
}

fn is_type(ty: &KirValueType, primitive: MirPrimitiveTypeName) -> bool {
    ty.as_scalar() == Some(&MirType::Primitive(primitive))
}

fn checker_remap_kind(
    source: &KirInstructionKind,
    values: &BTreeMap<ValueId, ValueId>,
) -> Result<KirInstructionKind, String> {
    let map_value = |value: ValueId| {
        values
            .get(&value)
            .copied()
            .ok_or_else(|| format!("source value v{} is not lane-mapped", value.index()))
    };
    let map_place = |place: &KirPlace| -> Result<KirPlace, String> {
        match place {
            KirPlace::SliceIndex {
                slice,
                index,
                type_node,
                region,
            } => Ok(KirPlace::SliceIndex {
                slice: map_value(*slice)?,
                index: map_value(*index)?,
                type_node: type_node.clone(),
                region: *region,
            }),
            _ => Err("runtime scalar UF4 source place is not a slice index".into()),
        }
    };
    match source {
        KirInstructionKind::ConstInt { value } => Ok(KirInstructionKind::ConstInt {
            value: value.clone(),
        }),
        KirInstructionKind::ConstFloat { value } => Ok(KirInstructionKind::ConstFloat {
            value: value.clone(),
        }),
        KirInstructionKind::Load { place } => Ok(KirInstructionKind::Load {
            place: Box::new(map_place(place)?),
        }),
        KirInstructionKind::Store { place, value } => Ok(KirInstructionKind::Store {
            place: Box::new(map_place(place)?),
            value: map_value(*value)?,
        }),
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } => Ok(KirInstructionKind::Binary {
            op: *op,
            left: map_value(*left)?,
            right: map_value(*right)?,
            semantics: *semantics,
        }),
        _ => Err("runtime scalar UF4 source instruction is outside the checker whitelist".into()),
    }
}

fn checker_profitability(
    profile: &crate::KirTargetProfile,
    body: &[KirInstruction],
    function: &crate::KirFunction,
) -> Result<(u32, KirCostEstimate), String> {
    let body_cost = body
        .iter()
        .try_fold(0_u32, |sum, instruction| {
            sum.checked_add(checker_instruction_cost(profile, instruction, function)?)
        })
        .ok_or_else(|| "runtime scalar UF4 profile has no body cost".to_string())?;
    let compare = checker_operation_cost(
        profile,
        KirProfileOperation::Compare,
        KirLaneType::U32,
        KirCostSemantics::NotApplicable,
    )?;
    let remainder = checker_operation_cost(
        profile,
        KirProfileOperation::Remainder,
        KirLaneType::U32,
        KirCostSemantics::Modular,
    )?;
    let subtract = checker_operation_cost(
        profile,
        KirProfileOperation::Subtract,
        KirLaneType::U32,
        KirCostSemantics::Modular,
    )?;
    let control = compare.saturating_add(1);
    let source_control = control.saturating_add(1);
    let setup = 1_u32
        .saturating_add(control)
        .saturating_add(remainder)
        .saturating_add(subtract);
    if body_cost == 0 {
        return Err("runtime scalar UF4 body has zero modeled cost".into());
    }
    for trip in 4_u32..=4096 {
        let groups = trip / 4;
        let tail = trip % 4;
        let scalar = body_cost
            .saturating_mul(trip)
            .saturating_add(source_control.saturating_mul(trip.saturating_add(1)));
        let grouped = body_cost
            .saturating_mul(groups.saturating_mul(4))
            .saturating_add(control.saturating_mul(groups.saturating_add(1)));
        let epilogue = body_cost
            .saturating_mul(tail)
            .saturating_add(source_control.saturating_mul(tail.saturating_add(1)));
        let total = grouped.saturating_add(epilogue).saturating_add(setup);
        if u64::from(total).saturating_mul(100) <= u64::from(scalar).saturating_mul(90) {
            return Ok((trip, KirCostEstimate::new(scalar, grouped, setup, epilogue)));
        }
    }
    Err("runtime scalar UF4 has no modeled 10-percent win".into())
}

fn checker_instruction_cost(
    profile: &crate::KirTargetProfile,
    instruction: &KirInstruction,
    function: &crate::KirFunction,
) -> Option<u32> {
    let (operation, lane, semantics, alignment) = match &instruction.kind {
        KirInstructionKind::ConstInt { .. } | KirInstructionKind::ConstFloat { .. } => {
            return Some(0);
        }
        KirInstructionKind::Load { place } => {
            let ty = checker_place_type(place.as_ref())?;
            (
                KirProfileOperation::Load,
                checker_lane(ty)?,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::Bytes(checker_byte_width(ty)?),
            )
        }
        KirInstructionKind::Store { place, .. } => {
            let ty = checker_place_type(place.as_ref())?;
            (
                KirProfileOperation::Store,
                checker_lane(ty)?,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::Bytes(checker_byte_width(ty)?),
            )
        }
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => KirProfileOperation::Add,
                MirBinaryOp::Sub => KirProfileOperation::Subtract,
                MirBinaryOp::Mul => KirProfileOperation::Multiply,
                MirBinaryOp::Div => KirProfileOperation::Divide,
                MirBinaryOp::Mod => KirProfileOperation::Remainder,
            },
            checker_lane(checker_value_type(
                function,
                instruction.results.first()?.value,
            )?)?,
            match semantics {
                KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
                KirArithmeticSemantics::Checked => return None,
            },
            KirAlignmentClass::NotApplicable,
        ),
        _ => return None,
    };
    checker_cost(
        profile,
        KirCostKey {
            operation,
            lane,
            lanes: 1,
            semantics,
            alignment,
        },
    )
}

fn checker_operation_cost(
    profile: &crate::KirTargetProfile,
    operation: KirProfileOperation,
    lane: KirLaneType,
    semantics: KirCostSemantics,
) -> Result<u32, String> {
    checker_cost(
        profile,
        KirCostKey {
            operation,
            lane,
            lanes: 1,
            semantics,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )
    .ok_or_else(|| format!("runtime scalar UF4 cost for {operation:?} is unavailable"))
}

fn checker_cost(profile: &crate::KirTargetProfile, key: KirCostKey) -> Option<u32> {
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        Some(KirOperationAvailability::Unavailable)
            if key.operation == KirProfileOperation::Branch =>
        {
            Some(1)
        }
        _ => None,
    }
}

fn checker_value_type(function: &crate::KirFunction, value: ValueId) -> Option<&MirType> {
    function
        .params
        .iter()
        .find_map(|parameter| (parameter.value == value).then_some(&parameter.type_node))
        .or_else(|| {
            function.blocks.iter().find_map(|block| {
                block.params.iter().find_map(|parameter| {
                    (parameter.value == value)
                        .then(|| parameter.type_node.as_scalar())
                        .flatten()
                })
            })
        })
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find_map(|instruction| {
                    instruction.results.iter().find_map(|result| {
                        (result.value == value)
                            .then(|| result.type_node.as_scalar())
                            .flatten()
                    })
                })
        })
}

fn checker_place_type(place: &KirPlace) -> Option<&MirType> {
    match place {
        KirPlace::SliceIndex { type_node, .. }
        | KirPlace::Index { type_node, .. }
        | KirPlace::Deref { type_node, .. }
        | KirPlace::Value { type_node, .. }
        | KirPlace::Field { type_node, .. } => Some(type_node),
    }
}

fn checker_lane(ty: &MirType) -> Option<KirLaneType> {
    match ty {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn checker_byte_width(ty: &MirType) -> Option<u16> {
    match ty {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(8),
        _ => None,
    }
}

fn source_body(function: &crate::KirFunction, id: BlockId) -> Result<&[KirInstruction], String> {
    Ok(&find_block(function, id)?.instructions)
}

fn entry_value(source: &Source<'_>, value: ValueId) -> Result<ValueId, String> {
    let index = source
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == value)
        .ok_or_else(|| "runtime scalar UF4 invariant does not come from header".to_string())?;
    source
        .incoming
        .args
        .get(index)
        .copied()
        .ok_or_else(|| "runtime scalar UF4 incoming value is missing".into())
}

fn one_result(instruction: &KirInstruction) -> Result<ValueId, String> {
    match instruction.results.as_slice() {
        [result] => Ok(result.value),
        _ => Err("runtime scalar UF4 instruction does not have one result".into()),
    }
}

fn module_units(module: &crate::KirModule) -> u32 {
    module
        .functions
        .iter()
        .map(kir_function_units)
        .fold(0_u32, u32::saturating_add)
}
