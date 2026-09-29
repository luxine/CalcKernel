use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, CandidateBudgetCharge, FactId, InstructionId, KirAlignmentClass,
    KirArithmeticSemantics, KirCostEstimate, KirCostKey, KirCostSemantics, KirEffectKind,
    KirInstruction, KirInstructionKind, KirLaneType, KirOperationAvailability, KirProfileOperation,
    KirTerminator, KirValueType, KirVerifiedProgramState, KirVersionPredicate,
    KirVersionPredicateConjunct, MemoryRegionId, MemoryVersionId, MirBinaryOp, MirCompareOp,
    MirPrimitiveTypeName, MirType, TransactionCheckError, ValueId, WasmInvariantLoadCandidate,
    kir_function_units, validate_kir_module,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInvariantLoadPlan {
    pub candidate: WasmInvariantLoadCandidate,
    pub pre_state: crate::KirPreStateIdentity,
    pub fast_entry: BlockId,
    pub fast_header: BlockId,
    pub fast_body: BlockId,
    pub unroll_factor: u8,
    pub guard_instruction: InstructionId,
    pub guard_value: ValueId,
    pub materialized_index: ValueId,
    pub bulk_limit_offset: ValueId,
    pub materialized_bulk_limit: ValueId,
    pub bulk_limit: ValueId,
    pub lane_induction_values: Vec<ValueId>,
    pub fast_induction_step_constant: ValueId,
    pub fast_induction_step: ValueId,
    pub output_zero: ValueId,
    pub materialized_output_start: ValueId,
    pub count_one: ValueId,
    pub hoisted_load: InstructionId,
    pub hoisted_value: ValueId,
    pub cached_value: ValueId,
    pub noalias_fact: FactId,
    pub cost: KirCostEstimate,
    pub index_instruction_map: Vec<(InstructionId, InstructionId)>,
    pub output_instruction_map: Vec<(InstructionId, InstructionId)>,
    pub header_instruction_map: Vec<(InstructionId, InstructionId)>,
    pub body_instruction_map: Vec<(InstructionId, InstructionId)>,
    pub value_mapping: Vec<(ValueId, ValueId)>,
    pub memory_mapping: Vec<(MemoryVersionId, MemoryVersionId)>,
    pub before_units: u32,
    pub after_units: u32,
}

#[derive(Debug, Clone)]
pub struct PreparedWasmInvariantLoad {
    pub trial: KirVerifiedProgramState,
    pub plan: WasmInvariantLoadPlan,
    pub charge: CandidateBudgetCharge,
}

const MAX_INDEX_DAG_INSTRUCTIONS: usize = 16;
const MAX_COST_SEARCH_TRIP: u32 = 4096;
const MINIMUM_COST_REDUCTION_PERCENT: u32 = 10;
const BASELINE_RANGE_PREDICATE_COST: u32 = 8;
const GUARDED_FAST_UNROLL_FACTOR: u32 = 8;
const BASELINE_LOOP_BRANCH_COST: u32 = 1;

type Values = BTreeMap<ValueId, ValueId>;
type Memories = BTreeMap<MemoryVersionId, MemoryVersionId>;

#[derive(Debug)]
struct Source<'a> {
    function: &'a crate::KirFunction,
    preheader: &'a crate::KirBlock,
    header: &'a crate::KirBlock,
    body: &'a crate::KirBlock,
    incoming: &'a crate::KirEdge,
    then_edge: &'a crate::KirEdge,
    backedge: &'a crate::KirEdge,
    load: &'a KirInstruction,
    index_dag: Vec<InstructionId>,
    elidable_index_dag: Vec<InstructionId>,
    output_range_dag: Vec<InstructionId>,
}

#[derive(Clone, Copy)]
struct LoopProofContext<'a> {
    function: &'a crate::KirFunction,
    preheader: &'a crate::KirBlock,
    header: &'a crate::KirBlock,
    body: &'a crate::KirBlock,
    incoming: &'a crate::KirEdge,
    then_edge: &'a crate::KirEdge,
    backedge: &'a crate::KirEdge,
    induction: ValueId,
}

pub fn check_wasm_invariant_load_trial_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &WasmInvariantLoadPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_trial(pre_state, trial, plan, charge).map_err(TransactionCheckError::compiler)
}

fn check_trial(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &WasmInvariantLoadPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), String> {
    let before = pre_state.module();
    let after = trial.module();
    let candidate = &plan.candidate;
    if before.config.consumer != crate::KirConsumer::WebAssembly
        || before.profile.wasm_features() != Some(crate::KirWasmFeatures::Baseline)
        || before.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || before.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || before.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
        || before.config != after.config
        || before.profile != after.profile
        || before.entry != after.entry
        || before.structs != after.structs
        || before.functions.len() != after.functions.len()
        || before
            .functions
            .iter()
            .map(|function| function.id)
            .ne(after.functions.iter().map(|function| function.id))
    {
        return Err("invariant-load target or module identity differs".into());
    }
    if plan.pre_state.function != candidate.function
        || plan.pre_state.kir_digest != pre_state.kir_digest()
        || plan.pre_state.profile_digest != before.profile.digest_hex()
        || plan.pre_state.evidence_generation != pre_state.evidence_generation()
        || plan.pre_state.frozen_kir_units
            != before
                .functions
                .iter()
                .find(|function| function.id == candidate.function)
                .map(kir_function_units)
                .ok_or_else(|| "invariant-load source function is missing".to_string())?
        || trial.evidence_generation() != pre_state.evidence_generation()
        || trial.contract_facts() != pre_state.contract_facts()
        || trial.eliminated_guards() != pre_state.eliminated_guards()
        || trial.proofs() != pre_state.proofs()
        || trial.optimization_entry_module_units() != pre_state.optimization_entry_module_units()
    {
        return Err("invariant-load pre-state or evidence identity is stale".into());
    }
    let original = before
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "invariant-load source function is missing".to_string())?;
    let transformed = after
        .functions
        .iter()
        .find(|function| function.id == original.id)
        .ok_or_else(|| "invariant-load transformed function is missing".to_string())?;
    if original.name != transformed.name
        || original.exported != transformed.exported
        || original.params != transformed.params
        || original.return_type != transformed.return_type
        || original.regions != transformed.regions
        || original.initial_memory != transformed.initial_memory
        || original.vector_regions != transformed.vector_regions
        || before
            .functions
            .iter()
            .filter(|function| function.id != original.id)
            .any(|function| {
                after.functions.iter().find(|other| other.id == function.id) != Some(function)
            })
    {
        return Err("invariant-load trial changed function ABI or unrelated functions".into());
    }
    let source = reconstruct_source(pre_state, original, candidate)?;
    verify_candidate_and_noalias(pre_state, &source, candidate)?;
    verify_fallback_is_unchanged(&source, transformed, plan)?;
    verify_preheader_guard(&source, transformed, plan)?;
    verify_fast_entry_and_header(&source, transformed, plan)?;
    verify_fast_body(&source, transformed, plan)?;
    verify_plan_maps(&source, transformed, plan)?;
    verify_cost_and_charge(pre_state, original, transformed, &source, plan, charge)?;
    let validation = validate_kir_module(after);
    if !validation.errors.is_empty() {
        return Err(format!(
            "invariant-load trial KIR is invalid: {:?}",
            validation.errors
        ));
    }
    Ok(())
}

fn reconstruct_source<'a>(
    pre_state: &KirVerifiedProgramState,
    function: &'a crate::KirFunction,
    candidate: &WasmInvariantLoadCandidate,
) -> Result<Source<'a>, String> {
    if candidate.key
        != (crate::CandidateKey::LoopFrontier {
            function: candidate.function,
            loop_id: candidate.loop_id,
            kind: crate::LoopCandidateKind::WasmInvariantLoad,
            variant: crate::LoopCandidateVariant::Scalar,
            vf: 1,
            uf: GUARDED_FAST_UNROLL_FACTOR as u8,
        })
        || function.id != candidate.function
    {
        return Err("invariant-load candidate key does not identify the source loop".into());
    }
    let canonical = crate::analyze_canonical_loops(function)
        .loops
        .into_iter()
        .find(|loop_| loop_.id == candidate.loop_id)
        .ok_or_else(|| "invariant-load loop is not canonical".to_string())?;
    if !canonical.innermost
        || !canonical.lcssa
        || !canonical.dedicated_exits
        || canonical.preheader != Some(candidate.preheader)
        || canonical.header != candidate.header
        || canonical.latch != Some(candidate.body)
        || canonical.exits != [candidate.exit]
        || canonical.blocks.len() != 2
        || canonical.blocks.iter().copied().collect::<BTreeSet<_>>()
            != BTreeSet::from([candidate.header, candidate.body])
    {
        return Err("invariant-load candidate is not a simple innermost loop".into());
    }
    let preheader = block(function, candidate.preheader)?;
    let header = block(function, candidate.header)?;
    let body = block(function, candidate.body)?;
    let exit = block(function, candidate.exit)?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return Err("invariant-load preheader has no direct entry edge".into());
    };
    let KirTerminator::Branch {
        condition: _,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return Err("invariant-load header is not a two-way loop test".into());
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err("invariant-load loop body has no single backedge".into());
    };
    if incoming.target != header.id
        || then_edge.target != body.id
        || else_edge.target != exit.id
        || backedge.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || then_edge.args.len() != body.params.len()
        || then_edge.memory_args.len() != body.memory_params.len()
        || backedge.args.len() != header.params.len()
        || backedge.memory_args.len() != header.memory_params.len()
        || else_edge.args.len() != exit.params.len()
        || else_edge.memory_args.len() != exit.memory_params.len()
    {
        return Err("invariant-load source loop edges do not match their block parameters".into());
    }
    let induction = canonical
        .induction
        .as_ref()
        .ok_or_else(|| "invariant-load loop has no induction variable".to_string())?;
    if induction.value != candidate.induction
        || induction.start != 0.into()
        || induction.step != 1.into()
        || induction.comparison != crate::MirCompareOp::Lt
        || value_type(function, candidate.induction)
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
    {
        return Err("invariant-load induction is not zero-based unit stride".into());
    }
    let proof = LoopProofContext {
        function,
        preheader,
        header,
        body,
        incoming,
        then_edge,
        backedge,
        induction: candidate.induction,
    };
    let induction_parameter = header
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.induction)
        .ok_or_else(|| "invariant-load induction parameter is missing".to_string())?;
    if header.params.iter().enumerate().any(|(index, parameter)| {
        if index == induction_parameter {
            return false;
        }
        let Some(backedge_value) = backedge.args.get(index) else {
            return true;
        };
        *backedge_value != parameter.value
            && !body.params.get(index).is_some_and(|body_parameter| {
                *backedge_value == body_parameter.value
                    && then_edge.args.get(index) == Some(&parameter.value)
            })
    }) {
        return Err("invariant-load source loop carries unsupported non-IV state".into());
    }
    verify_source_unit_induction_update(function, header, body, backedge, candidate)?;
    let KirTerminator::Branch { condition, .. } = &header.terminator else {
        unreachable!()
    };
    if header.instructions.len() != 1 {
        return Err("invariant-load loop header contains extra work".into());
    }
    let compare = &header.instructions[0];
    let (bound, compare_value) = match compare.kind {
        KirInstructionKind::Compare {
            op: crate::MirCompareOp::Lt,
            left,
            right,
        } => (right, left),
        _ => return Err("invariant-load loop header is not a less-than test".into()),
    };
    if compare_value != candidate.induction
        || compare
            .results
            .as_slice()
            .first()
            .map(|result| result.value)
            != Some(*condition)
        || compare.memory.is_some()
        || compare.effect.is_some()
        || bound != candidate.bound
        || !bound_is_stable(&proof, bound)
    {
        return Err("invariant-load loop bound is not stable on every backedge".into());
    }

    let stores = body
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
        .collect::<Vec<_>>();
    let [store] = stores.as_slice() else {
        return Err("invariant-load source needs one output store".into());
    };
    let (out_slice, out_index, out_region) = slice_place(store, false)?;
    if out_index != candidate.output_index {
        return Err("invariant-load output index differs from the candidate".into());
    }
    if !depends_on_loop_value(function, body, then_edge, candidate.induction, out_index) {
        return Err("invariant-load output address does not depend on the loop column".into());
    }
    let output_range_dag = reconstruct_output_range_dag(&proof, out_index)?;
    if output_range_dag != candidate.output_range_dag {
        return Err("invariant-load output range proof is stale".into());
    }
    let out_root = stable_root(function, out_slice)
        .ok_or_else(|| "invariant-load output slice has no stable source root".to_string())?;
    let out_partition = partition(function, out_region)?;
    if out_root != candidate.output_slice
        || out_region != candidate.output_region
        || out_partition != candidate.output_partition
    {
        return Err("invariant-load output place differs from the candidate".into());
    }

    let loads = body
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .collect::<Vec<_>>();
    if loads.len() != 3
        || body
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
            .count()
            != 1
        || body.instructions.iter().any(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Call { .. }
                    | KirInstructionKind::RuntimeCall { .. }
                    | KirInstructionKind::Guard { .. }
                    | KirInstructionKind::CheckCondition { .. }
                    | KirInstructionKind::VersionPredicate { .. }
                    | KirInstructionKind::Subslice { .. }
                    | KirInstructionKind::MakeSlice { .. }
                    | KirInstructionKind::Address { .. }
            )
        })
    {
        return Err("invariant-load scalar body is outside the closed load/store shape".into());
    }
    let load = instruction(function, candidate.load)?;
    let (input_slice, index, input_region) = slice_place(load, true)?;
    let [load_result] = load.results.as_slice() else {
        return Err("invariant-load source load has no single result".into());
    };
    let input_root = stable_root(function, input_slice)
        .ok_or_else(|| "invariant-load input slice has no stable source root".to_string())?;
    let input_partition = partition(function, input_region)?;
    if input_root != candidate.input_slice
        || input_region != candidate.input_region
        || input_partition != candidate.input_partition
        || load_result.value != candidate.load_value
        || index != candidate.index
        || !uses_reach_store(body, candidate.load_value, store.id)
    {
        return Err("invariant-load source load differs from the candidate".into());
    }
    if load
        .memory
        .as_ref()
        .is_none_or(|memory| memory.output.is_some() || memory.region != input_partition)
    {
        return Err("invariant-load source memory access is not a partitioned read".into());
    }
    if candidate.input_partition == candidate.output_partition {
        return Err("invariant-load source regions share one MemorySSA partition".into());
    }
    if body.instructions.iter().any(|instruction| {
        instruction.id != store.id
            && instruction
                .memory
                .as_ref()
                .is_some_and(|memory| memory.output.is_some())
    }) {
        return Err("invariant-load loop contains another write".into());
    }
    let (index_dag, elidable_index_dag) =
        reconstruct_index_dag(&proof, candidate.index, candidate.load)?;
    if candidate.index_dag != index_dag || candidate.elidable_index_dag != elidable_index_dag {
        return Err("invariant-load index or dead-code proof is stale".into());
    }
    // Contract facts are imported and validated at the public unsafe boundary.
    let _ = pre_state;
    Ok(Source {
        function,
        preheader,
        header,
        body,
        incoming,
        then_edge,
        backedge,
        load,
        index_dag,
        elidable_index_dag,
        output_range_dag,
    })
}

fn verify_candidate_and_noalias(
    pre_state: &KirVerifiedProgramState,
    source: &Source<'_>,
    candidate: &WasmInvariantLoadCandidate,
) -> Result<(), String> {
    let function = source.function;
    let fact = pre_state
        .contract_facts()
        .and_then(|contracts| contracts.facts().get(candidate.noalias_fact))
        .ok_or_else(|| "invariant-load NoAlias fact is missing".to_string())?;
    let scope_available = match &fact.scope {
        crate::FactScope::FunctionEntry(owner) => *owner == function.id,
        crate::FactScope::Block {
            function: owner,
            block,
        } => {
            *owner == function.id
                && crate::compute_kir_dominators(function).dominates(*block, source.preheader.id)
        }
        crate::FactScope::CalleeInstance { .. } | crate::FactScope::InlineClone { .. } => false,
    };
    let noalias = matches!(
        fact.predicate,
        crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias { left, right })
            if (stable_root(function, left) == Some(candidate.input_slice)
                && stable_root(function, right) == Some(candidate.output_slice))
                || (stable_root(function, right) == Some(candidate.input_slice)
                    && stable_root(function, left) == Some(candidate.output_slice))
    );
    if !scope_available
        || fact.generation != pre_state.evidence_generation()
        || !matches!(fact.origin, crate::FactOrigin::TrustedContract { .. })
        || fact.derivation != crate::FactDerivation::TrustedContractLeaf
        || !noalias
    {
        return Err("invariant-load requires a dominating trusted NoAlias(a,out) contract".into());
    }
    Ok(())
}

fn verify_source_unit_induction_update(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    body: &crate::KirBlock,
    backedge: &crate::KirEdge,
    candidate: &WasmInvariantLoadCandidate,
) -> Result<(), String> {
    let index = header
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.induction)
        .ok_or_else(|| "invariant-load source induction parameter is missing".to_string())?;
    let body_induction = body
        .params
        .get(index)
        .filter(|_| {
            matches!(&body.terminator, KirTerminator::Jump { edge } if edge.target == header.id)
        })
        .map(|parameter| parameter.value)
        .ok_or_else(|| "invariant-load body induction mapping is malformed".to_string())?;
    let update_value = *backedge
        .args
        .get(index)
        .ok_or_else(|| "invariant-load source IV backedge argument is missing".to_string())?;
    let (definition_block, update) = definition(function, update_value)
        .ok_or_else(|| "invariant-load source IV update is undefined".to_string())?;
    let (step, result) = match &update.kind {
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } if *left == body_induction => (*right, update.results.first().map(|result| result.value)),
        _ => return Err("invariant-load source IV update is not modular IV+1".into()),
    };
    let result =
        result.ok_or_else(|| "invariant-load source IV update has no result".to_string())?;
    let (constant_block, constant) = definition(function, step)
        .ok_or_else(|| "invariant-load source unit step is undefined".to_string())?;
    let dominators = crate::compute_kir_dominators(function);
    if definition_block != body.id
        || update.id != candidate.induction_update
        || result != update_value
        || update.results.len() != 1
        || update.results[0].type_node.as_scalar()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || update.memory.is_some()
        || update.effect.is_some()
        || !dominators.dominates(constant_block, body.id)
        || !matches!(&constant.kind, KirInstructionKind::ConstInt { value } if value == "1")
        || constant.results.len() != 1
        || constant.results[0].value != step
        || constant.results[0].type_node.as_scalar()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || constant.memory.is_some()
        || constant.effect.is_some()
        || function.blocks.iter().any(|block| {
            block.instructions.iter().any(|instruction| {
                instruction.id != update.id && instruction_uses_value(instruction, update_value)
            }) || (block.id != body.id && terminator_uses_value(&block.terminator, update_value))
        })
        || backedge
            .args
            .iter()
            .enumerate()
            .any(|(argument_index, argument)| argument_index != index && *argument == update_value)
    {
        return Err(
            "invariant-load source IV update has another use or is not exact U32 +1".into(),
        );
    }
    Ok(())
}

fn verify_fallback_is_unchanged(
    source: &Source<'_>,
    transformed: &crate::KirFunction,
    plan: &WasmInvariantLoadPlan,
) -> Result<(), String> {
    if transformed.blocks.len() != source.function.blocks.len() + 3 {
        return Err("invariant-load trial added an unexpected number of blocks".into());
    }
    for original in &source.function.blocks {
        let actual = block(transformed, original.id)?;
        if original.id == source.preheader.id {
            if original.params != actual.params
                || original.memory_params != actual.memory_params
                || actual.instructions.get(..original.instructions.len())
                    != Some(original.instructions.as_slice())
            {
                return Err("invariant-load changed source preheader instructions".into());
            }
        } else if actual != original {
            return Err(
                "invariant-load changed the scalar fallback or another source block".into(),
            );
        }
    }
    for id in [plan.fast_entry, plan.fast_header, plan.fast_body] {
        if source.function.blocks.iter().any(|block| block.id == id) {
            return Err("invariant-load clone reused a source block identity".into());
        }
    }
    Ok(())
}

fn verify_preheader_guard(
    source: &Source<'_>,
    transformed: &crate::KirFunction,
    plan: &WasmInvariantLoadPlan,
) -> Result<(), String> {
    let preheader = block(transformed, source.preheader.id)?;
    let extra_start = source.preheader.instructions.len();
    if plan.unroll_factor != GUARDED_FAST_UNROLL_FACTOR as u8
        || plan.candidate.minimum_trip < GUARDED_FAST_UNROLL_FACTOR
    {
        return Err("invariant-load fast path lacks its minimum-trip-eight proof".into());
    }
    let input_dag = source.index_dag.iter().copied().collect::<BTreeSet<_>>();
    let output_new = source
        .output_range_dag
        .iter()
        .filter(|id| !input_dag.contains(id))
        .count();
    if plan.index_instruction_map.len() != source.index_dag.len()
        || plan.output_instruction_map.len() != source.output_range_dag.len()
        || preheader.instructions.len() != extra_start + source.index_dag.len() + 1 + output_new + 2
    {
        return Err("invariant-load preheader does not contain the exact index and guard".into());
    }
    let incoming_values = source
        .header
        .params
        .iter()
        .zip(&source.incoming.args)
        .map(|(parameter, value)| (parameter.value, *value))
        .collect::<Values>();
    let mut expected_values = BTreeMap::new();
    for (body_parameter, argument) in source.body.params.iter().zip(&source.then_edge.args) {
        let mapped = incoming_values.get(argument).copied().unwrap_or(*argument);
        expected_values.insert(body_parameter.value, mapped);
    }
    let mut all_materialized_values = expected_values.clone();
    for (index, (source_id, transformed_id)) in plan.index_instruction_map.iter().enumerate() {
        if *source_id != source.index_dag[index] {
            return Err("invariant-load index map is not source ordered".into());
        }
        let original = instruction(source.function, *source_id)?;
        let actual = &preheader.instructions[extra_start + index];
        if actual.id != *transformed_id
            || original.results.len() != actual.results.len()
            || original.memory.is_some()
            || original.effect.is_some()
        {
            return Err("invariant-load preheader index instruction identity changed".into());
        }
        for (source_result, target_result) in original.results.iter().zip(&actual.results) {
            if source_result.type_node != target_result.type_node {
                return Err("invariant-load materialized index result type changed".into());
            }
            all_materialized_values.insert(source_result.value, target_result.value);
        }
        let mut expected = (*original).clone();
        remap_values(&mut expected, &all_materialized_values);
        expected.id = actual.id;
        expected.results = actual.results.clone();
        if actual.kind != expected.kind || actual.memory.is_some() || actual.effect.is_some() {
            return Err("invariant-load materialized a different index expression".into());
        }
    }
    let materialized_index = all_materialized_values
        .get(&plan.candidate.index)
        .copied()
        .or_else(|| {
            value_dominates_block(source.function, plan.candidate.index, source.preheader.id)
                .then_some(plan.candidate.index)
        })
        .ok_or_else(|| "invariant-load range start was not materialized".to_string())?;
    if materialized_index != plan.materialized_index {
        return Err("invariant-load plan records another range start".into());
    }
    let zero = &preheader.instructions[extra_start + source.index_dag.len()];
    if zero.results.as_slice().first().map(|result| result.value) != Some(plan.output_zero)
        || zero
            .results
            .as_slice()
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || zero.kind
            != (KirInstructionKind::ConstInt {
                value: "0".to_string(),
            })
        || zero.memory.is_some()
        || zero.effect.is_some()
    {
        return Err("invariant-load output-range induction zero is malformed".into());
    }
    let body_induction = source
        .body
        .params
        .iter()
        .zip(&source.then_edge.args)
        .find_map(|(parameter, argument)| {
            (*argument == plan.candidate.induction).then_some(parameter.value)
        })
        .ok_or_else(|| "invariant-load output range has no body induction".to_string())?;
    let mut output_values = all_materialized_values.clone();
    output_values.insert(body_induction, plan.output_zero);
    let mut next_instruction = extra_start + source.index_dag.len() + 1;
    for (index, (source_id, transformed_id)) in plan.output_instruction_map.iter().enumerate() {
        if *source_id != source.output_range_dag[index] {
            return Err("invariant-load output range map is not source ordered".into());
        }
        let original = instruction(source.function, *source_id)?;
        let actual = if input_dag.contains(source_id) {
            let mapped = plan
                .index_instruction_map
                .iter()
                .find(|(old, _)| old == source_id)
                .map(|(_, new)| *new)
                .ok_or_else(|| "shared output-range instruction is not materialized".to_string())?;
            if mapped != *transformed_id {
                return Err("shared output-range instruction maps to a different clone".into());
            }
            preheader
                .instructions
                .iter()
                .find(|instruction| instruction.id == mapped)
                .ok_or_else(|| "shared output-range clone is missing".to_string())?
        } else {
            let actual = preheader
                .instructions
                .get(next_instruction)
                .ok_or_else(|| "output-range materialization is truncated".to_string())?;
            if actual.id != *transformed_id
                || original.results.len() != actual.results.len()
                || original.memory.is_some()
                || original.effect.is_some()
            {
                return Err("invariant-load output-range instruction identity changed".into());
            }
            for (old, new) in original.results.iter().zip(&actual.results) {
                if old.type_node != new.type_node {
                    return Err("invariant-load output-range result type changed".into());
                }
                output_values.insert(old.value, new.value);
            }
            let mut expected = (*original).clone();
            remap_values(&mut expected, &output_values);
            expected.id = actual.id;
            expected.results = actual.results.clone();
            if actual.kind != expected.kind || actual.memory.is_some() || actual.effect.is_some() {
                return Err(
                    "invariant-load materialized a different output range expression".into(),
                );
            }
            next_instruction += 1;
            actual
        };
        if original.results.len() == 1 {
            let old = original.results[0].value;
            let new = actual
                .results
                .first()
                .ok_or_else(|| "output-range clone has no result".to_string())?
                .value;
            output_values.insert(old, new);
        }
    }
    let materialized_output_start = output_values
        .get(&plan.candidate.output_index)
        .copied()
        .ok_or_else(|| "output write interval start was not materialized".to_string())?;
    if materialized_output_start != plan.materialized_output_start {
        return Err("invariant-load plan records another output range start".into());
    }
    let count = &preheader.instructions[next_instruction];
    if count.results.as_slice().first().map(|result| result.value) != Some(plan.count_one)
        || count
            .results
            .as_slice()
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || count.kind
            != (KirInstructionKind::ConstInt {
                value: "1".to_string(),
            })
        || count.memory.is_some()
        || count.effect.is_some()
    {
        return Err("invariant-load range count is not the exact one-element footprint".into());
    }
    let guard = &preheader.instructions[next_instruction + 1];
    if guard.id != plan.guard_instruction
        || guard.results.as_slice().first().map(|result| result.value) != Some(plan.guard_value)
        || guard
            .results
            .as_slice()
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::Bool))
        || guard.memory.is_some()
        || guard.effect.is_some()
    {
        return Err("invariant-load total guard identity or type changed".into());
    }
    let expected_bound = source
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == plan.candidate.bound)
        .and_then(|index| source.incoming.args.get(index).copied())
        .unwrap_or(plan.candidate.bound);
    let expected_predicate = KirInstructionKind::VersionPredicate {
        predicate: KirVersionPredicate {
            address_bits: 32,
            conjuncts: vec![
                KirVersionPredicateConjunct::TripThreshold {
                    value: expected_bound,
                    minimum: plan.candidate.minimum_trip,
                },
                KirVersionPredicateConjunct::WasmSliceRange {
                    slice: plan.candidate.input_slice,
                    start: plan.materialized_index,
                    count: plan.count_one,
                    element_bytes: 8,
                },
                KirVersionPredicateConjunct::WasmSliceRange {
                    slice: plan.candidate.output_slice,
                    start: plan.materialized_output_start,
                    count: expected_bound,
                    element_bytes: 8,
                },
            ],
        },
    };
    if guard.kind != expected_predicate {
        return Err("invariant-load guard does not bind the proven bound and A element".into());
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &preheader.terminator
    else {
        return Err("invariant-load guard does not select fast and scalar paths".into());
    };
    if *condition != plan.guard_value
        || then_edge.target != plan.fast_entry
        || then_edge.args
            != source
                .incoming
                .args
                .iter()
                .copied()
                .chain(std::iter::once(plan.materialized_index))
                .collect::<Vec<_>>()
        || then_edge.memory_args != source.incoming.memory_args
        || else_edge != source.incoming
    {
        return Err("invariant-load guard changed the fallback or fast-entry state".into());
    }
    Ok(())
}

fn verify_fast_entry_and_header(
    source: &Source<'_>,
    transformed: &crate::KirFunction,
    plan: &WasmInvariantLoadPlan,
) -> Result<(), String> {
    let entry = block(transformed, plan.fast_entry)?;
    if entry.params.len() != source.header.params.len() + 1
        || entry.memory_params.len() != source.header.memory_params.len()
        || entry.instructions.len() != 3
    {
        return Err("invariant-load fast entry has an invalid signature or body".into());
    }
    for (source_param, target_param) in source.header.params.iter().zip(&entry.params) {
        if source_param.type_node != target_param.type_node {
            return Err("invariant-load fast-entry value parameter type changed".into());
        }
    }
    let index_param = entry
        .params
        .last()
        .ok_or_else(|| "fast-entry index parameter is absent".to_string())?;
    if index_param.type_node != KirValueType::scalar(MirType::Primitive(MirPrimitiveTypeName::U32))
    {
        return Err("invariant-load fast-entry index is not u32".into());
    }
    for (source_param, target_param) in source.header.memory_params.iter().zip(&entry.memory_params)
    {
        if source_param.region != target_param.region {
            return Err("invariant-load fast-entry MemorySSA partition changed".into());
        }
    }
    let offset = &entry.instructions[0];
    if offset.results.first().map(|result| result.value) != Some(plan.bulk_limit_offset)
        || offset
            .results
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || offset.kind
            != (KirInstructionKind::ConstInt {
                value: (GUARDED_FAST_UNROLL_FACTOR - 1).to_string(),
            })
        || offset.memory.is_some()
        || offset.effect.is_some()
    {
        return Err("invariant-load bulk-limit offset is not the exact seven".into());
    }
    let entry_bound = source
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == plan.candidate.bound)
        .map(|index| entry.params[index].value)
        .unwrap_or(plan.candidate.bound);
    let subtract = &entry.instructions[1];
    if subtract.results.first().map(|result| result.value) != Some(plan.materialized_bulk_limit)
        || subtract
            .results
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || subtract.kind
            != (KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                left: entry_bound,
                right: plan.bulk_limit_offset,
                semantics: KirArithmeticSemantics::Modular,
            })
        || subtract.memory.is_some()
        || subtract.effect.is_some()
    {
        return Err("invariant-load fast-entry bulk limit is not bound minus seven".into());
    }
    let hoisted = &entry.instructions[2];
    if hoisted.id != plan.hoisted_load
        || hoisted
            .results
            .as_slice()
            .first()
            .map(|result| result.value)
            != Some(plan.hoisted_value)
        || hoisted
            .results
            .as_slice()
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::F64))
    {
        return Err("invariant-load fast entry did not perform exactly one f64 load".into());
    }
    let KirInstructionKind::Load { place } = &hoisted.kind else {
        return Err("invariant-load fast-entry instruction is not a load".into());
    };
    if !matches!(place.as_ref(), crate::KirPlace::SliceIndex { slice, index, type_node, region }
        if *slice == plan.candidate.input_slice
            && *index == index_param.value
            && *type_node == MirType::Primitive(MirPrimitiveTypeName::F64)
            && *region == plan.candidate.input_region)
        || hoisted.memory.as_ref().is_none_or(|memory| {
            memory.output.is_some()
                || memory.region != plan.candidate.input_partition
                || !entry.memory_params.iter().any(|parameter| {
                    parameter.region == memory.region && parameter.version == memory.input
                })
        })
        || hoisted
            .effect
            .as_ref()
            .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
    {
        return Err("invariant-load fast-entry load changed its guarded place or MemorySSA".into());
    }
    let KirTerminator::Jump { edge: entry_edge } = &entry.terminator else {
        return Err("invariant-load fast-entry does not enter its cloned loop".into());
    };
    let fast_header = block(transformed, plan.fast_header)?;
    if entry_edge.target != fast_header.id
        || entry_edge.args
            != entry.params[..source.header.params.len()]
                .iter()
                .map(|parameter| parameter.value)
                .chain(std::iter::once(plan.materialized_bulk_limit))
                .chain(std::iter::once(hoisted.results[0].value))
                .collect::<Vec<_>>()
        || entry_edge.memory_args
            != entry
                .memory_params
                .iter()
                .map(|param| param.version)
                .collect::<Vec<_>>()
    {
        return Err("invariant-load cached A value is not loop invariant".into());
    }
    if fast_header.params.len() != source.header.params.len() + 2
        || fast_header.memory_params.len() != source.header.memory_params.len()
        || fast_header.instructions.len() != source.header.instructions.len()
    {
        return Err("invariant-load cloned header shape changed".into());
    }
    for (source_param, target_param) in source.header.params.iter().zip(&fast_header.params) {
        if source_param.type_node != target_param.type_node {
            return Err("invariant-load cloned header parameter type changed".into());
        }
    }
    let bulk_limit_param = fast_header
        .params
        .get(source.header.params.len())
        .ok_or_else(|| "fast loop bulk-limit parameter is missing".to_string())?;
    let cached_param = fast_header
        .params
        .get(source.header.params.len() + 1)
        .ok_or_else(|| "cached A header parameter is missing".to_string())?;
    if bulk_limit_param.value != plan.bulk_limit
        || bulk_limit_param.type_node
            != KirValueType::scalar(MirType::Primitive(MirPrimitiveTypeName::U32))
        || cached_param.value != plan.cached_value
        || cached_param.type_node
            != KirValueType::scalar(MirType::Primitive(MirPrimitiveTypeName::F64))
    {
        return Err("invariant-load cached value phi has the wrong type".into());
    }
    for (source_param, target_param) in source
        .header
        .memory_params
        .iter()
        .zip(&fast_header.memory_params)
    {
        if source_param.region != target_param.region {
            return Err("invariant-load cloned header MemorySSA partition changed".into());
        }
    }
    verify_header_instructions(source, fast_header, plan)?;
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &fast_header.terminator
    else {
        return Err("invariant-load cloned header lost its loop branch".into());
    };
    let mapped_condition = plan
        .value_mapping
        .iter()
        .find_map(|(old, new)| {
            (*old
                == match &source.header.terminator {
                    KirTerminator::Branch { condition, .. } => *condition,
                    _ => ValueId::from_index(u32::MAX),
                })
            .then_some(*new)
        })
        .ok_or_else(|| "invariant-load loop condition mapping is missing".to_string())?;
    let fast_body = block(transformed, plan.fast_body)?;
    if *condition != mapped_condition
        || then_edge.target != fast_body.id
        || else_edge.target != source.header.id
        || then_edge.args.len() != source.body.params.len()
        || then_edge.memory_args.len() != fast_body.memory_params.len()
        || else_edge.args.len() != source.header.params.len()
        || else_edge.memory_args.len() != fast_header.memory_params.len()
    {
        return Err("invariant-load cloned header edges do not preserve trip control".into());
    }
    let header_map = source
        .header
        .params
        .iter()
        .zip(&fast_header.params)
        .map(|(old, new)| (old.value, new.value))
        .collect::<Values>();
    let mut expected_then_args = source.then_edge.args.clone();
    remap_values_list(&mut expected_then_args, &header_map);
    if then_edge.args != expected_then_args
        || then_edge.memory_args
            != fast_header
                .memory_params
                .iter()
                .map(|parameter| parameter.version)
                .collect::<Vec<_>>()
    {
        return Err("invariant-load cloned loop body entry mapping is not exact".into());
    }
    let expected_else_args = fast_header.params[..source.header.params.len()]
        .iter()
        .map(|parameter| parameter.value)
        .collect::<Vec<_>>();
    if else_edge.args != expected_else_args
        || else_edge.memory_args
            != fast_header
                .memory_params
                .iter()
                .map(|parameter| parameter.version)
                .collect::<Vec<_>>()
    {
        return Err("invariant-load fast loop exit does not rejoin the scalar continuation".into());
    }
    Ok(())
}

fn verify_header_instructions(
    source: &Source<'_>,
    transformed: &crate::KirBlock,
    plan: &WasmInvariantLoadPlan,
) -> Result<(), String> {
    if plan.header_instruction_map.len() != source.header.instructions.len() {
        return Err("invariant-load header instruction map is incomplete".into());
    }
    let source_values = source
        .header
        .params
        .iter()
        .zip(&transformed.params)
        .map(|(old, new)| (old.value, new.value))
        .collect::<Values>();
    let mut values = source_values.clone();
    for (index, (source_id, transformed_id)) in plan.header_instruction_map.iter().enumerate() {
        if *source_id != source.header.instructions[index].id {
            return Err("invariant-load cloned header mapping is not source ordered".into());
        }
        let original = &source.header.instructions[index];
        let actual = transformed
            .instructions
            .get(index)
            .ok_or_else(|| "invariant-load cloned header instruction is missing".to_string())?;
        if actual.id != *transformed_id || original.results.len() != actual.results.len() {
            return Err("invariant-load cloned header instruction identity changed".into());
        }
        for (old, new) in original.results.iter().zip(&actual.results) {
            if old.type_node != new.type_node {
                return Err("invariant-load cloned header result type changed".into());
            }
            values.insert(old.value, new.value);
        }
        let mut expected = (*original).clone();
        remap_values(&mut expected, &values);
        let source_condition = match source.header.terminator {
            KirTerminator::Branch { condition, .. } => condition,
            _ => return Err("invariant-load source header lost its branch".into()),
        };
        if original
            .results
            .first()
            .is_some_and(|result| result.value == source_condition)
        {
            let KirInstructionKind::Compare {
                op: MirCompareOp::Lt,
                left,
                ..
            } = &expected.kind
            else {
                return Err("invariant-load source trip test is not less-than".into());
            };
            expected.kind = KirInstructionKind::Compare {
                op: MirCompareOp::Lt,
                left: *left,
                right: plan.bulk_limit,
            };
        }
        expected.id = actual.id;
        expected.results = actual.results.clone();
        if expected.kind != actual.kind
            || expected.memory != actual.memory
            || expected.effect != actual.effect
        {
            return Err("invariant-load cloned a different loop-bound test".into());
        }
    }
    Ok(())
}

fn verify_fast_body(
    source: &Source<'_>,
    transformed: &crate::KirFunction,
    plan: &WasmInvariantLoadPlan,
) -> Result<(), String> {
    let body = block(transformed, plan.fast_body)?;
    let retained = source
        .body
        .instructions
        .iter()
        .filter(|instruction| {
            instruction.id != source.load.id
                && instruction.id != plan.candidate.induction_update
                && !source.elidable_index_dag.contains(&instruction.id)
        })
        .collect::<Vec<_>>();
    if body.params.len() != source.body.params.len()
        || body.memory_params.len() != source.body.memory_params.len()
        || plan.unroll_factor != GUARDED_FAST_UNROLL_FACTOR as u8
        || plan.lane_induction_values.len() != GUARDED_FAST_UNROLL_FACTOR as usize
        || body.instructions.len()
            != retained
                .len()
                .checked_mul(GUARDED_FAST_UNROLL_FACTOR as usize)
                .and_then(|lanes| lanes.checked_add(GUARDED_FAST_UNROLL_FACTOR as usize + 2))
                .ok_or_else(|| "invariant-load strip-mined body size overflowed".to_string())?
        || plan.body_instruction_map.len() != retained.len() * GUARDED_FAST_UNROLL_FACTOR as usize
    {
        return Err("invariant-load fast body does not retain eight exact scalar lanes".into());
    }
    let fast_header = block(transformed, plan.fast_header)?;
    let mut header_values = source
        .header
        .params
        .iter()
        .zip(&fast_header.params[..source.header.params.len()])
        .map(|(old, new)| (old.value, new.value))
        .collect::<Values>();
    for (old, new) in source
        .header
        .instructions
        .iter()
        .zip(&fast_header.instructions)
    {
        for (old_result, new_result) in old.results.iter().zip(&new.results) {
            header_values.insert(old_result.value, new_result.value);
        }
    }
    let mut base_values = header_values.clone();
    base_values.extend(
        source
            .body
            .params
            .iter()
            .zip(&body.params)
            .map(|(old, new)| (old.value, new.value)),
    );
    base_values.insert(source.load.results[0].value, plan.cached_value);
    let induction_body = source
        .body
        .params
        .iter()
        .zip(&source.then_edge.args)
        .find_map(|(parameter, argument)| {
            (*argument == plan.candidate.induction).then_some(parameter.value)
        })
        .ok_or_else(|| "invariant-load source induction body parameter is missing".to_string())?;
    if plan.lane_induction_values[0]
        != base_values
            .get(&induction_body)
            .copied()
            .ok_or_else(|| "invariant-load fast body induction mapping is missing".to_string())?
    {
        return Err("invariant-load lane zero does not use the source loop IV".into());
    }
    let one = &body.instructions[0];
    let lane_one = one
        .results
        .first()
        .map(|result| result.value)
        .ok_or_else(|| "invariant-load lane increment constant has no result".to_string())?;
    if one.kind
        != (KirInstructionKind::ConstInt {
            value: "1".to_string(),
        })
        || one
            .results
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || one.memory.is_some()
        || one.effect.is_some()
    {
        return Err("invariant-load lane increment is not a pure u32 one".into());
    }
    for lane in 1..GUARDED_FAST_UNROLL_FACTOR as usize {
        let actual = &body.instructions[lane];
        let expected_value = plan.lane_induction_values[lane];
        if actual.results.first().map(|result| result.value) != Some(expected_value)
            || actual
                .results
                .first()
                .and_then(|result| result.type_node.as_scalar())
                != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        {
            return Err("invariant-load lane induction result type or identity changed".into());
        }
        let previous = plan.lane_induction_values[lane - 1];
        let expected = KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: previous,
            right: lane_one,
            semantics: KirArithmeticSemantics::Modular,
        };
        if actual.kind != expected || actual.memory.is_some() || actual.effect.is_some() {
            return Err("invariant-load lane IV is not the exact modular unit increment".into());
        }
    }
    let lane_prefix = GUARDED_FAST_UNROLL_FACTOR as usize;
    let mut memories = source
        .body
        .memory_params
        .iter()
        .zip(&body.memory_params)
        .map(|(old, new)| (old.version, new.version))
        .collect::<Memories>();
    let mut memory_mappings = memories
        .iter()
        .map(|(old, new)| (*old, *new))
        .collect::<BTreeSet<_>>();
    memory_mappings.extend(
        source
            .header
            .memory_params
            .iter()
            .zip(&fast_header.memory_params)
            .map(|(old, new)| (old.version, new.version)),
    );
    let mut used_effects = BTreeSet::new();
    let mut last_effect_order = block(transformed, plan.fast_entry)?
        .instructions
        .iter()
        .find(|instruction| instruction.id == plan.hoisted_load)
        .and_then(|instruction| instruction.effect.as_ref())
        .map(|effect| effect.order)
        .ok_or_else(|| "invariant-load fast-entry read has no ordered effect".to_string())?;
    let mut final_lane_values = base_values.clone();
    let mut instruction_index = lane_prefix;
    for lane in 0..GUARDED_FAST_UNROLL_FACTOR as usize {
        let mut values = base_values.clone();
        values.insert(induction_body, plan.lane_induction_values[lane]);
        for original in &retained {
            let actual = body
                .instructions
                .get(instruction_index)
                .ok_or_else(|| "invariant-load fast lane is truncated".to_string())?;
            let map = plan
                .body_instruction_map
                .get(instruction_index - lane_prefix)
                .ok_or_else(|| "invariant-load body instruction map is truncated".to_string())?;
            if *map != (original.id, actual.id) || original.results.len() != actual.results.len() {
                return Err("invariant-load body lane mapping is incomplete or reordered".into());
            }
            for (old, new) in original.results.iter().zip(&actual.results) {
                if old.type_node != new.type_node {
                    return Err("invariant-load cloned body result type changed".into());
                }
                values.insert(old.value, new.value);
            }
            let mut expected = (*original).clone();
            remap_values(&mut expected, &values);
            expected.id = actual.id;
            expected.results = actual.results.clone();
            if expected.kind != actual.kind {
                return Err(
                    "invariant-load fast body changed source arithmetic or memory address".into(),
                );
            }
            match (&original.memory, &actual.memory) {
                (None, None) => {}
                (Some(old), Some(new)) => {
                    let mapped_input = memories.get(&old.input).copied().ok_or_else(|| {
                        "invariant-load clone MemorySSA input is not mapped".to_string()
                    })?;
                    if old.region != new.region || mapped_input != new.input {
                        return Err("invariant-load clone changed MemorySSA input partition".into());
                    }
                    if let Some(old_output) = old.output {
                        let new_output = new.output.ok_or_else(|| {
                            "invariant-load clone dropped a memory definition".to_string()
                        })?;
                        memories.insert(old_output, new_output);
                        memory_mappings.insert((old_output, new_output));
                    } else if new.output.is_some() {
                        return Err("invariant-load clone added a MemorySSA definition".into());
                    }
                }
                _ => return Err("invariant-load clone changed memory effects".into()),
            }
            match (&original.effect, &actual.effect) {
                (None, None) => {}
                (Some(old), Some(new)) => {
                    if old.kind != new.kind
                        || new.order <= last_effect_order
                        || !used_effects.insert(new.order)
                    {
                        return Err("invariant-load clone changed ordered effect kind".into());
                    }
                    last_effect_order = new.order;
                }
                _ => return Err("invariant-load clone changed ordered effects".into()),
            }
            instruction_index += 1;
        }
        final_lane_values = values;
    }
    let step_constant = body
        .instructions
        .get(instruction_index)
        .ok_or_else(|| "invariant-load fast induction step constant is missing".to_string())?;
    let step = body
        .instructions
        .get(instruction_index + 1)
        .ok_or_else(|| "invariant-load fast induction step is missing".to_string())?;
    if step_constant.results.first().map(|result| result.value)
        != Some(plan.fast_induction_step_constant)
        || step_constant.kind
            != (KirInstructionKind::ConstInt {
                value: GUARDED_FAST_UNROLL_FACTOR.to_string(),
            })
        || step_constant
            .results
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || step_constant.memory.is_some()
        || step_constant.effect.is_some()
        || step.results.first().map(|result| result.value) != Some(plan.fast_induction_step)
        || step.kind
            != (KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: plan.lane_induction_values[0],
                right: plan.fast_induction_step_constant,
                semantics: KirArithmeticSemantics::Modular,
            })
        || step.memory.is_some()
        || step.effect.is_some()
    {
        return Err("invariant-load fast backedge is not exact modular col+8".into());
    }
    if plan.memory_mapping.iter().copied().collect::<BTreeSet<_>>() != memory_mappings {
        return Err("invariant-load MemorySSA mapping is false or incomplete".into());
    }
    let KirTerminator::Jump {
        edge: actual_backedge,
    } = &body.terminator
    else {
        return Err("invariant-load fast body does not return to its header".into());
    };
    let mut expected_args = Vec::with_capacity(source.header.params.len() + 2);
    for (index, argument) in source.backedge.args.iter().enumerate() {
        if index
            == source
                .header
                .params
                .iter()
                .position(|parameter| parameter.value == plan.candidate.induction)
                .ok_or_else(|| "invariant-load source IV parameter is missing".to_string())?
        {
            expected_args.push(plan.fast_induction_step);
        } else {
            expected_args.push(
                final_lane_values
                    .get(argument)
                    .or_else(|| header_values.get(argument))
                    .copied()
                    .unwrap_or(*argument),
            );
        }
    }
    expected_args.push(plan.bulk_limit);
    expected_args.push(plan.cached_value);
    let expected_memories = source
        .backedge
        .memory_args
        .iter()
        .map(|memory| memories.get(memory).copied())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "invariant-load backedge has an unmapped memory version".to_string())?;
    if actual_backedge.target != plan.fast_header
        || actual_backedge.args != expected_args
        || actual_backedge.memory_args != expected_memories
    {
        return Err("invariant-load cached value or loop-carried state changed on backedge".into());
    }
    Ok(())
}

fn verify_plan_maps(
    source: &Source<'_>,
    transformed: &crate::KirFunction,
    plan: &WasmInvariantLoadPlan,
) -> Result<(), String> {
    let header = block(transformed, plan.fast_header)?;
    let body = block(transformed, plan.fast_body)?;
    let mut expected = source
        .header
        .params
        .iter()
        .zip(&header.params)
        .map(|(old, new)| (old.value, new.value))
        .collect::<BTreeSet<_>>();
    expected.insert((source.load.results[0].value, plan.cached_value));
    for (old, new) in source.header.instructions.iter().zip(&header.instructions) {
        expected.extend(
            old.results
                .iter()
                .zip(&new.results)
                .map(|(a, b)| (a.value, b.value)),
        );
    }
    expected.extend(
        source
            .body
            .params
            .iter()
            .zip(&body.params)
            .map(|(a, b)| (a.value, b.value)),
    );
    for (source_id, transformed_id) in &plan.body_instruction_map {
        let old = instruction(source.function, *source_id)?;
        let new = instruction(transformed, *transformed_id)?;
        expected.extend(
            old.results
                .iter()
                .zip(&new.results)
                .map(|(a, b)| (a.value, b.value)),
        );
    }
    if plan.value_mapping.iter().copied().collect::<BTreeSet<_>>() != expected {
        return Err("invariant-load value map does not match its source and clone".into());
    }
    Ok(())
}

fn verify_cost_and_charge(
    pre_state: &KirVerifiedProgramState,
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    source: &Source<'_>,
    plan: &WasmInvariantLoadPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), String> {
    let (cost, minimum_trip) = independently_price(IndependentPriceInput {
        profile: &pre_state.module().profile,
        body: source.body,
        function: original,
        load: source.load.id,
        induction_update: plan.candidate.induction_update,
        elidable: &source.elidable_index_dag,
        index_dag: &source.index_dag,
        output_range_dag: &source.output_range_dag,
    })
    .ok_or_else(|| "invariant-load cost is not profitable in the baseline profile".to_string())?;
    if minimum_trip != plan.candidate.minimum_trip
        || cost != plan.cost
        || cost != plan.candidate.predicted_cost
        || plan.noalias_fact != plan.candidate.noalias_fact
        || plan.before_units != kir_function_units(original)
        || plan.after_units != kir_function_units(transformed)
    {
        return Err("invariant-load cost, threshold or size accounting is false".into());
    }
    let expected_charge = CandidateBudgetCharge::single(
        original.id,
        plan.after_units
            .saturating_sub(plan.before_units)
            .saturating_add(16),
        plan.before_units
            .saturating_add(plan.after_units)
            .saturating_add(32),
    );
    if *charge != expected_charge {
        return Err("invariant-load checker budget charge is false".into());
    }
    Ok(())
}

struct IndependentPriceInput<'a> {
    profile: &'a crate::KirTargetProfile,
    body: &'a crate::KirBlock,
    function: &'a crate::KirFunction,
    load: InstructionId,
    induction_update: InstructionId,
    elidable: &'a [InstructionId],
    index_dag: &'a [InstructionId],
    output_range_dag: &'a [InstructionId],
}

fn independently_price(input: IndependentPriceInput<'_>) -> Option<(KirCostEstimate, u32)> {
    let body_cost = input
        .body
        .instructions
        .iter()
        .try_fold(0_u32, |sum, item| {
            sum.checked_add(profile_instruction_cost(
                input.profile,
                item,
                input.function,
            )?)
        })?;
    let load_cost = profile_instruction_cost(
        input.profile,
        instruction(input.function, input.load).ok()?,
        input.function,
    )?;
    let _unit_update_cost = profile_instruction_cost(
        input.profile,
        instruction(input.function, input.induction_update).ok()?,
        input.function,
    )?;
    let savings = input.elidable.iter().try_fold(load_cost, |sum, id| {
        sum.checked_add(profile_instruction_cost(
            input.profile,
            instruction(input.function, *id).ok()?,
            input.function,
        )?)
    })?;
    let index_setup = input.index_dag.iter().try_fold(0_u32, |sum, id| {
        sum.checked_add(profile_instruction_cost(
            input.profile,
            instruction(input.function, *id).ok()?,
            input.function,
        )?)
    })?;
    let output_setup = input.output_range_dag.iter().try_fold(0_u32, |sum, id| {
        sum.checked_add(profile_instruction_cost(
            input.profile,
            instruction(input.function, *id).ok()?,
            input.function,
        )?)
    })?;
    if body_cost == 0 || savings == 0 || savings >= body_cost {
        return None;
    }
    let compare_cost = profile_operation_cost(
        input.profile,
        KirProfileOperation::Compare,
        KirLaneType::U32,
        KirCostSemantics::NotApplicable,
    )?;
    let bulk_limit_cost = profile_operation_cost(
        input.profile,
        KirProfileOperation::Subtract,
        KirLaneType::U32,
        KirCostSemantics::Modular,
    )?;
    let predicates = index_setup
        .checked_add(output_setup)?
        .checked_add(load_cost)?
        .checked_add(bulk_limit_cost)?
        .checked_add(
            BASELINE_RANGE_PREDICATE_COST
                .checked_mul(3)?
                .checked_add(1)?,
        )?;
    let loop_control = compare_cost.checked_add(BASELINE_LOOP_BRANCH_COST)?;
    for trip in GUARDED_FAST_UNROLL_FACTOR..=MAX_COST_SEARCH_TRIP {
        let chunks = trip / GUARDED_FAST_UNROLL_FACTOR;
        let tail = trip % GUARDED_FAST_UNROLL_FACTOR;
        let bulk_trip_count = chunks.checked_mul(GUARDED_FAST_UNROLL_FACTOR)?;
        let scalar = body_cost
            .checked_mul(trip)?
            .checked_add(loop_control.checked_mul(trip)?)?
            .checked_add(compare_cost)?;
        let transformed_body = body_cost
            .checked_sub(savings)?
            .checked_mul(bulk_trip_count)?
            .checked_add(body_cost.checked_mul(tail)?)?;
        let transformed_control = loop_control
            .checked_mul(chunks.checked_add(tail)?)?
            .checked_add(compare_cost.checked_mul(2)?)?
            .checked_add(BASELINE_LOOP_BRANCH_COST)?;
        let total = transformed_body
            .checked_add(predicates)?
            .checked_add(transformed_control)?;
        if u64::from(total).saturating_mul(100)
            <= u64::from(scalar).saturating_mul(u64::from(100 - MINIMUM_COST_REDUCTION_PERCENT))
        {
            return Some((
                KirCostEstimate::new(scalar, transformed_body, predicates, transformed_control),
                trip,
            ));
        }
    }
    None
}

fn profile_instruction_cost(
    profile: &crate::KirTargetProfile,
    instruction: &KirInstruction,
    function: &crate::KirFunction,
) -> Option<u32> {
    use KirProfileOperation as Op;
    let (operation, lane, semantics, alignment) = match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. } => return Some(0),
        KirInstructionKind::Load { place } => (
            Op::Load,
            primitive_lane(place_primitive_type(place.as_ref())?)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::Bytes(8),
        ),
        KirInstructionKind::Store { place, .. } => (
            Op::Store,
            primitive_lane(place_primitive_type(place.as_ref())?)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::Bytes(8),
        ),
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => Op::Add,
                MirBinaryOp::Sub => Op::Subtract,
                MirBinaryOp::Mul => Op::Multiply,
                MirBinaryOp::Div => Op::Divide,
                MirBinaryOp::Mod => Op::Remainder,
            },
            result_lane(instruction, function)?,
            match semantics {
                KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                KirArithmeticSemantics::Checked => KirCostSemantics::Checked,
                KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
            },
            KirAlignmentClass::NotApplicable,
        ),
        KirInstructionKind::SliceLen { .. } | KirInstructionKind::SliceData { .. } => {
            return Some(1);
        }
        _ => return None,
    };
    let key = KirCostKey {
        operation,
        lane,
        lanes: 1,
        semantics,
        alignment,
    };
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        _ => None,
    }
}

fn profile_operation_cost(
    profile: &crate::KirTargetProfile,
    operation: KirProfileOperation,
    lane: KirLaneType,
    semantics: KirCostSemantics,
) -> Option<u32> {
    let key = KirCostKey {
        operation,
        lane,
        lanes: 1,
        semantics,
        alignment: KirAlignmentClass::NotApplicable,
    };
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        _ => None,
    }
}

fn verify_source_index_invariance(
    source: &LoopProofContext<'_>,
    root: ValueId,
) -> Result<Vec<InstructionId>, String> {
    struct Walker<'a> {
        source: LoopProofContext<'a>,
        dominators: crate::KirDominators,
        stack: BTreeSet<ValueId>,
        visited: BTreeSet<ValueId>,
        ordered: Vec<InstructionId>,
    }

    impl Walker<'_> {
        fn visit(&mut self, value: ValueId) -> Result<(), String> {
            let source = self.source;
            if self.visited.contains(&value) {
                return Ok(());
            }
            if let Some(body_index) = source
                .body
                .params
                .iter()
                .position(|parameter| parameter.value == value)
            {
                let incoming = *source.then_edge.args.get(body_index).ok_or_else(|| {
                    "invariant-load body value is absent from entry edge".to_string()
                })?;
                if incoming == source.induction {
                    return Err("invariant-load address depends on the loop induction".into());
                }
                let header_index = source
                    .header
                    .params
                    .iter()
                    .position(|parameter| parameter.value == incoming)
                    .ok_or_else(|| {
                        "invariant-load entry value is not a source header parameter".to_string()
                    })?;
                if source.backedge.args.get(header_index) != Some(&value) {
                    return Err("invariant-load address changes on a loop backedge".into());
                }
                self.visited.insert(value);
                return Ok(());
            }
            let Some((definition_block, instruction)) = definition(source.function, value) else {
                if source
                    .function
                    .params
                    .iter()
                    .any(|parameter| parameter.value == value)
                {
                    self.visited.insert(value);
                    return Ok(());
                }
                if source.function.blocks.iter().any(|block| {
                    block.id != source.body.id
                        && block.id != source.header.id
                        && self.dominators.dominates(block.id, source.preheader.id)
                        && (block
                            .params
                            .iter()
                            .any(|parameter| parameter.value == value)
                            || block.instructions.iter().any(|instruction| {
                                instruction
                                    .results
                                    .iter()
                                    .any(|result| result.value == value)
                            }))
                }) {
                    self.visited.insert(value);
                    return Ok(());
                }
                return Err("invariant-load address has no dominating source definition".into());
            };
            if definition_block != source.body.id {
                if self
                    .dominators
                    .dominates(definition_block, source.preheader.id)
                {
                    self.visited.insert(value);
                    return Ok(());
                }
                return Err("invariant-load source index is not defined before its loop".into());
            }
            if instruction.memory.is_some()
                || instruction.effect.is_some()
                || instruction.results.len() != 1
                || instruction.results[0].value != value
                || instruction.results[0].type_node.as_scalar()
                    != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
                || !matches!(
                    instruction.kind,
                    KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::Binary {
                            semantics: KirArithmeticSemantics::Modular,
                            ..
                        }
                )
            {
                return Err("invariant-load address contains non-total or non-u32 work".into());
            }
            if !self.stack.insert(value) {
                return Err("invariant-load address contains an SSA cycle".into());
            }
            let operands = modular_operands(&instruction.kind)?;
            let instruction_id = instruction.id;
            for operand in operands {
                self.visit(operand)?;
            }
            self.stack.remove(&value);
            self.visited.insert(value);
            self.ordered.push(instruction_id);
            if self.ordered.len() > MAX_INDEX_DAG_INSTRUCTIONS {
                return Err(
                    "invariant-load address DAG exceeds the independent checker budget".into(),
                );
            }
            Ok(())
        }
    }

    let mut walker = Walker {
        source: *source,
        dominators: crate::compute_kir_dominators(source.function),
        stack: BTreeSet::new(),
        visited: BTreeSet::new(),
        ordered: Vec::new(),
    };
    walker.visit(root)?;
    Ok(walker.ordered)
}

fn reconstruct_index_dag(
    source: &LoopProofContext<'_>,
    index: ValueId,
    load: InstructionId,
) -> Result<(Vec<InstructionId>, Vec<InstructionId>), String> {
    let ordered = verify_source_index_invariance(source, index)?;
    let dag = ordered.iter().copied().collect::<BTreeSet<_>>();
    let mut elidable = dag.clone();
    loop {
        let mut changed = false;
        for id in dag.iter().copied() {
            if !elidable.contains(&id) {
                continue;
            }
            let instruction = instruction(source.function, id)?;
            let value = instruction
                .results
                .first()
                .ok_or_else(|| "invariant-load DAG instruction has no result".to_string())?
                .value;
            let external_user = source
                .body
                .instructions
                .iter()
                .filter(|user| user.id != id)
                .any(|user| {
                    instruction_uses_value(user, value)
                        && !elidable.contains(&user.id)
                        && user.id != load
                });
            if external_user || terminator_uses_value(&source.body.terminator, value) {
                elidable.remove(&id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok((ordered, elidable.into_iter().collect()))
}

fn reconstruct_output_range_dag(
    source: &LoopProofContext<'_>,
    output_index: ValueId,
) -> Result<Vec<InstructionId>, String> {
    let body_induction = source
        .body
        .params
        .iter()
        .zip(&source.then_edge.args)
        .find_map(|(parameter, argument)| {
            (*argument == source.induction).then_some(parameter.value)
        })
        .ok_or_else(|| "output-range induction has no body parameter".to_string())?;
    struct Walker<'a> {
        source: LoopProofContext<'a>,
        body_induction: ValueId,
        dag: BTreeSet<InstructionId>,
        stack: BTreeSet<ValueId>,
    }

    impl Walker<'_> {
        fn coefficient(&mut self, value: ValueId) -> Result<i32, String> {
            let source = self.source;
            if value == self.body_induction {
                return Ok(1);
            }
            if let Ok(stable_dag) = verify_source_index_invariance(&source, value) {
                self.dag.extend(stable_dag);
                return Ok(0);
            }
            if !self.stack.insert(value) {
                return Err("output-range expression contains a source cycle".into());
            }
            let (block_id, instruction) = definition(source.function, value).ok_or_else(|| {
                "output-range expression has no dominating definition".to_string()
            })?;
            if block_id != source.body.id
                || instruction.results.len() != 1
                || instruction.results[0].value != value
                || instruction.results[0].type_node.as_scalar()
                    != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
                || instruction.memory.is_some()
                || instruction.effect.is_some()
            {
                return Err("output-range expression is not pure U32 KIR".into());
            }
            let kind = instruction.kind.clone();
            let instruction_id = instruction.id;
            let coefficient = match &kind {
                KirInstructionKind::Copy { value: operand } => self.coefficient(*operand)?,
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add | MirBinaryOp::Sub,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } => {
                    let left_coefficient = self.coefficient(*left)?;
                    let right_coefficient = self.coefficient(*right)?;
                    if matches!(
                        &kind,
                        KirInstructionKind::Binary {
                            op: MirBinaryOp::Sub,
                            ..
                        }
                    ) {
                        left_coefficient.checked_sub(right_coefficient)
                    } else {
                        left_coefficient.checked_add(right_coefficient)
                    }
                    .ok_or_else(|| {
                        "output-range IV coefficient overflowed checker domain".to_string()
                    })?
                }
                _ => return Err("output-range IV path uses unsupported arithmetic".into()),
            };
            self.stack.remove(&value);
            self.dag.insert(instruction_id);
            if self.dag.len() > MAX_INDEX_DAG_INSTRUCTIONS {
                return Err("output-range DAG exceeds the independent checker budget".into());
            }
            Ok(coefficient)
        }
    }

    let mut walker = Walker {
        source: *source,
        body_induction,
        dag: BTreeSet::new(),
        stack: BTreeSet::new(),
    };
    if walker.coefficient(output_index)? != 1 {
        return Err("output index is not exactly base plus one loop induction".into());
    }
    Ok(source
        .body
        .instructions
        .iter()
        .filter(|instruction| walker.dag.contains(&instruction.id))
        .map(|instruction| instruction.id)
        .collect())
}

fn bound_is_stable(source: &LoopProofContext<'_>, bound: ValueId) -> bool {
    if let Some(index) = source
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == bound)
    {
        if source.incoming.args.get(index).is_none() {
            return false;
        }
        let Some(body_parameter) = source.body.params.get(index) else {
            return false;
        };
        return source.then_edge.args.get(index) == Some(&bound)
            && source.backedge.args.get(index) == Some(&body_parameter.value);
    }
    let dominators = crate::compute_kir_dominators(source.function);
    source
        .function
        .params
        .iter()
        .any(|parameter| parameter.value == bound)
        || source.function.blocks.iter().any(|block| {
            block.id != source.header.id
                && block.id != source.body.id
                && dominators.dominates(block.id, source.preheader.id)
                && (block
                    .params
                    .iter()
                    .any(|parameter| parameter.value == bound)
                    || block.instructions.iter().any(|instruction| {
                        instruction
                            .results
                            .iter()
                            .any(|result| result.value == bound)
                    }))
        })
}
fn stable_root(function: &crate::KirFunction, value: ValueId) -> Option<ValueId> {
    crate::optimizer::vectorize_check::stable_invariant_descriptor_root(function, value)
}

fn value_dominates_block(function: &crate::KirFunction, value: ValueId, target: BlockId) -> bool {
    if function
        .params
        .iter()
        .any(|parameter| parameter.value == value)
    {
        return true;
    }
    let dominators = crate::compute_kir_dominators(function);
    function.blocks.iter().any(|definition| {
        dominators.dominates(definition.id, target)
            && (definition
                .params
                .iter()
                .any(|parameter| parameter.value == value)
                || definition.instructions.iter().any(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == value)
                }))
    })
}

fn depends_on_loop_value(
    function: &crate::KirFunction,
    body: &crate::KirBlock,
    edge: &crate::KirEdge,
    induction: ValueId,
    root: ValueId,
) -> bool {
    fn visit(
        function: &crate::KirFunction,
        body: &crate::KirBlock,
        edge: &crate::KirEdge,
        induction: ValueId,
        value: ValueId,
        seen: &mut BTreeSet<ValueId>,
    ) -> bool {
        if !seen.insert(value) {
            return false;
        }
        if let Some(index) = body
            .params
            .iter()
            .position(|parameter| parameter.value == value)
        {
            return edge.args.get(index) == Some(&induction);
        }
        let Some((block_id, instruction)) = definition(function, value) else {
            return false;
        };
        if block_id != body.id {
            return false;
        }
        modular_operands(&instruction.kind).is_ok_and(|operands| {
            operands
                .into_iter()
                .any(|operand| visit(function, body, edge, induction, operand, seen))
        })
    }
    visit(function, body, edge, induction, root, &mut BTreeSet::new())
}

fn slice_place(
    instruction: &KirInstruction,
    load: bool,
) -> Result<(ValueId, ValueId, MemoryRegionId), String> {
    let place = match (&instruction.kind, load) {
        (KirInstructionKind::Load { place }, true) => place.as_ref(),
        (KirInstructionKind::Store { place, .. }, false) => place.as_ref(),
        _ => return Err("invariant-load memory instruction kind changed".into()),
    };
    match place {
        crate::KirPlace::SliceIndex {
            slice,
            index,
            type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
            region,
        } => Ok((*slice, *index, *region)),
        _ => Err("invariant-load only accepts indexed f64 slices".into()),
    }
}

fn partition(
    function: &crate::KirFunction,
    region: MemoryRegionId,
) -> Result<MemoryRegionId, String> {
    function
        .regions
        .iter()
        .find(|descriptor| descriptor.id == region)
        .map(|descriptor| descriptor.partition)
        .ok_or_else(|| "invariant-load region has no MemorySSA partition".into())
}

fn place_primitive_type(place: &crate::KirPlace) -> Option<&MirType> {
    match place {
        crate::KirPlace::SliceIndex { type_node, .. }
        | crate::KirPlace::Index { type_node, .. }
        | crate::KirPlace::Value { type_node, .. }
        | crate::KirPlace::Deref { type_node, .. }
        | crate::KirPlace::Field { type_node, .. } => Some(type_node),
    }
}

fn primitive_lane(type_node: &MirType) -> Option<KirLaneType> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn result_lane(instruction: &KirInstruction, function: &crate::KirFunction) -> Option<KirLaneType> {
    let [result] = instruction.results.as_slice() else {
        return None;
    };
    value_type(function, result.value).and_then(primitive_lane)
}

fn value_type(function: &crate::KirFunction, value: ValueId) -> Option<&MirType> {
    function
        .params
        .iter()
        .find(|parameter| parameter.value == value)
        .map(|parameter| &parameter.type_node)
        .or_else(|| {
            function.blocks.iter().find_map(|block| {
                block
                    .params
                    .iter()
                    .find(|parameter| parameter.value == value)
                    .and_then(|parameter| parameter.type_node.as_scalar())
                    .or_else(|| {
                        block.instructions.iter().find_map(|instruction| {
                            instruction
                                .results
                                .iter()
                                .find(|result| result.value == value)
                                .and_then(|result| result.type_node.as_scalar())
                        })
                    })
            })
        })
}

fn instruction_uses_value(instruction: &KirInstruction, value: ValueId) -> bool {
    match &instruction.kind {
        KirInstructionKind::Copy { value: operand }
        | KirInstructionKind::Cast { value: operand, .. }
        | KirInstructionKind::VectorSplat {
            scalar: operand, ..
        } => *operand == value,
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. }
        | KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => {
            *left == value || *right == value
        }
        KirInstructionKind::Unary { operand, .. }
        | KirInstructionKind::VectorUnary { operand, .. } => *operand == value,
        KirInstructionKind::Load { place } | KirInstructionKind::Address { place } => {
            place_uses_value(place, value)
        }
        KirInstructionKind::Store {
            place,
            value: stored,
        } => *stored == value || place_uses_value(place, value),
        KirInstructionKind::SliceLen { slice } | KirInstructionKind::SliceData { slice } => {
            *slice == value
        }
        KirInstructionKind::Subslice { slice, start, end } => {
            *slice == value || *start == value || *end == value
        }
        KirInstructionKind::MakeSlice { data, len } => *data == value || *len == value,
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.contains(&value),
        KirInstructionKind::Guard { condition, .. } => *condition == value,
        KirInstructionKind::VersionPredicate { predicate } => {
            predicate.conjuncts.iter().any(|conjunct| match conjunct {
                KirVersionPredicateConjunct::TripThreshold { value: actual, .. } => {
                    *actual == value
                }
                KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                    left,
                    left_count,
                    right,
                    right_count,
                    ..
                } => [left, left_count, right, right_count].contains(&&value),
                KirVersionPredicateConjunct::WasmSliceRange {
                    slice,
                    start,
                    count,
                    ..
                } => [slice, start, count].contains(&&value),
            })
        }
        KirInstructionKind::VectorLoad { access, .. } => {
            access.slice == value || access.start == value || access.end == value
        }
        KirInstructionKind::VectorStore {
            access,
            value: stored,
            ..
        } => {
            *stored == value
                || access.slice == value
                || access.start == value
                || access.end == value
        }
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => [mask, when_true, when_false].contains(&&value),
        KirInstructionKind::VectorCast { value: operand, .. }
        | KirInstructionKind::VectorExtract {
            vector: operand, ..
        }
        | KirInstructionKind::VectorReduce {
            vector: operand, ..
        } => *operand == value,
        KirInstructionKind::VectorInsert { vector, scalar, .. } => {
            *vector == value || *scalar == value
        }
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => false,
    }
}

fn place_uses_value(place: &crate::KirPlace, value: ValueId) -> bool {
    match place {
        crate::KirPlace::SliceIndex { slice, index, .. } => *slice == value || *index == value,
        crate::KirPlace::Index { base, index, .. } => {
            place_uses_value(base, value) || *index == value
        }
        crate::KirPlace::Field { base, .. } => place_uses_value(base, value),
        crate::KirPlace::Value { value: actual, .. } => *actual == value,
        crate::KirPlace::Deref { pointer, .. } => *pointer == value,
    }
}

fn uses_reach_store(body: &crate::KirBlock, source: ValueId, store: InstructionId) -> bool {
    fn visit(
        body: &crate::KirBlock,
        value: ValueId,
        store: InstructionId,
        seen: &mut BTreeSet<ValueId>,
    ) -> bool {
        if !seen.insert(value) {
            return false;
        }
        let mut reaches = false;
        for user in &body.instructions {
            if !instruction_uses_value(user, value) {
                continue;
            }
            if user.id == store {
                reaches = true;
            } else if user.results.len() == 1
                && matches!(
                    user.kind,
                    KirInstructionKind::Binary { .. }
                        | KirInstructionKind::Unary { .. }
                        | KirInstructionKind::Copy { .. }
                )
            {
                reaches |= visit(body, user.results[0].value, store, seen);
            } else {
                return false;
            }
        }
        reaches
    }
    visit(body, source, store, &mut BTreeSet::new())
}

fn terminator_uses_value(terminator: &KirTerminator, value: ValueId) -> bool {
    match terminator {
        KirTerminator::Jump { edge } => edge.args.contains(&value),
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            *condition == value
                || then_edge.args.contains(&value)
                || else_edge.args.contains(&value)
        }
        KirTerminator::Return {
            value: returned, ..
        } => *returned == Some(value),
    }
}

fn modular_operands(kind: &KirInstructionKind) -> Result<Vec<ValueId>, String> {
    match kind {
        KirInstructionKind::ConstInt { .. } => Ok(Vec::new()),
        KirInstructionKind::Copy { value } => Ok(vec![*value]),
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } => Ok(vec![*left, *right]),
        _ => Err("invariant-load index uses an unsupported source operation".into()),
    }
}

fn definition(function: &crate::KirFunction, value: ValueId) -> Option<(BlockId, &KirInstruction)> {
    function.blocks.iter().find_map(|block| {
        block
            .instructions
            .iter()
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            })
            .map(|instruction| (block.id, instruction))
    })
}

fn instruction(
    function: &crate::KirFunction,
    id: InstructionId,
) -> Result<&KirInstruction, String> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
        .ok_or_else(|| "invariant-load source instruction is missing".to_string())
}

fn block(function: &crate::KirFunction, id: BlockId) -> Result<&crate::KirBlock, String> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| "invariant-load block is missing".to_string())
}

fn remap_values(instruction: &mut KirInstruction, values: &Values) {
    use KirInstructionKind as Kind;
    fn value(target: &mut ValueId, values: &Values) {
        if let Some(mapped) = values.get(target) {
            *target = *mapped;
        }
    }
    fn place(target: &mut crate::KirPlace, values: &Values) {
        match target {
            crate::KirPlace::SliceIndex { slice, index, .. } => {
                value(slice, values);
                value(index, values);
            }
            crate::KirPlace::Value { value: actual, .. } => value(actual, values),
            crate::KirPlace::Deref { pointer, .. } => value(pointer, values),
            crate::KirPlace::Index { base, index, .. } => {
                place(base, values);
                value(index, values);
            }
            crate::KirPlace::Field { base, .. } => place(base, values),
        }
    }
    match &mut instruction.kind {
        Kind::Undef { .. }
        | Kind::ConstInt { .. }
        | Kind::ConstFloat { .. }
        | Kind::ConstBool { .. } => {}
        Kind::Copy { value: actual }
        | Kind::Cast { value: actual, .. }
        | Kind::VectorSplat { scalar: actual, .. } => value(actual, values),
        Kind::Binary { left, right, .. }
        | Kind::Compare { left, right, .. }
        | Kind::VectorBinary { left, right, .. }
        | Kind::VectorCompare { left, right, .. } => {
            value(left, values);
            value(right, values);
        }
        Kind::Unary { operand, .. } | Kind::VectorUnary { operand, .. } => value(operand, values),
        Kind::CheckCondition { args, .. }
        | Kind::Call { args, .. }
        | Kind::RuntimeCall { args, .. } => {
            for item in args {
                value(item, values);
            }
        }
        Kind::Guard { condition, .. } => value(condition, values),
        Kind::Address { place: target } | Kind::Load { place: target } => place(target, values),
        Kind::Store {
            place: target,
            value: stored,
        } => {
            place(target, values);
            value(stored, values);
        }
        Kind::MakeSlice { data, len } => {
            value(data, values);
            value(len, values);
        }
        Kind::SliceData { slice } | Kind::SliceLen { slice } => value(slice, values),
        Kind::Subslice { slice, start, end } => {
            value(slice, values);
            value(start, values);
            value(end, values);
        }
        Kind::VersionPredicate { predicate } => {
            for conjunct in &mut predicate.conjuncts {
                match conjunct {
                    KirVersionPredicateConjunct::TripThreshold { value: actual, .. } => {
                        value(actual, values)
                    }
                    KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left,
                        left_count,
                        right,
                        right_count,
                        ..
                    } => {
                        value(left, values);
                        value(left_count, values);
                        value(right, values);
                        value(right_count, values);
                    }
                    KirVersionPredicateConjunct::WasmSliceRange {
                        slice,
                        start,
                        count,
                        ..
                    } => {
                        value(slice, values);
                        value(start, values);
                        value(count, values);
                    }
                }
            }
        }
        Kind::VectorLoad { access, .. } => {
            value(&mut access.slice, values);
            value(&mut access.start, values);
            value(&mut access.end, values);
        }
        Kind::VectorStore {
            access,
            value: stored,
            ..
        } => {
            value(&mut access.slice, values);
            value(&mut access.start, values);
            value(&mut access.end, values);
            value(stored, values);
        }
        Kind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => {
            value(mask, values);
            value(when_true, values);
            value(when_false, values);
        }
        Kind::VectorCast { value: actual, .. }
        | Kind::VectorExtract { vector: actual, .. }
        | Kind::VectorReduce { vector: actual, .. } => value(actual, values),
        Kind::VectorInsert { vector, scalar, .. } => {
            value(vector, values);
            value(scalar, values);
        }
    }
}

fn remap_values_list(values: &mut [ValueId], mapping: &Values) {
    for value in values {
        if let Some(mapped) = mapping.get(value) {
            *value = *mapped;
        }
    }
}
