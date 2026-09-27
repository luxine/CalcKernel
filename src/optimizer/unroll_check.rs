use std::collections::BTreeSet;

use crate::{
    CandidateBudgetCharge, KirCostEstimate, KirInstructionKind, KirPreStateIdentity, KirTerminator,
    KirVerifiedProgramState, LoopTripCount, SlpPlan, TransactionCheckError, UnrollPlan,
    analyze_canonical_loops, kir_function_units, loop_cfg_digest,
};

#[must_use]
pub const fn unroll_profitability_threshold(cost: KirCostEstimate) -> bool {
    cost.scalar >= cost.total.saturating_add(2)
        && (cost.total as u64).saturating_mul(100) <= (cost.scalar as u64).saturating_mul(90)
}

pub fn check_unroll_plan_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &UnrollPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_unroll_plan_internal(pre_state, trial, plan, charge, true)
}

pub fn check_unroll_structure_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &UnrollPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_unroll_plan_internal(pre_state, trial, plan, charge, false)
}

fn check_unroll_plan_internal(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &UnrollPlan,
    charge: &CandidateBudgetCharge,
    require_standalone_profitability: bool,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    if plan.pre_state != pre_identity(pre_state, plan.function)
        || plan.o3_entry_module_units != pre_state.optimization_entry_module_units()
        || plan.pre_state.function != plan.function
    {
        return malformed("unroll pre-state identity is stale");
    }
    let Some(original) = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.function)
    else {
        return malformed("unroll original function is missing");
    };
    let analysis = analyze_canonical_loops(original);
    let Some(descriptor) = analysis
        .loops
        .iter()
        .find(|descriptor| descriptor.id == plan.loop_id && descriptor.header == plan.header)
    else {
        return malformed("unroll canonical loop identity is false");
    };
    if !descriptor.innermost
        || !descriptor.dedicated_exits
        || !descriptor.lcssa
        || descriptor.blocks.len() != 2
        || descriptor.exits.len() != 1
        || plan.proof.cfg_digest != loop_cfg_digest(original)
        || !plan.proof.dedicated_exits
        || !plan.proof.lcssa
    {
        return malformed("unroll canonical shape or LCSSA proof is false");
    }
    let trip_count = exact_trip(&descriptor.trip_count)
        .ok_or_else(|| TransactionCheckError::compiler("unroll trip is not exact"))?;
    let body = descriptor
        .latch
        .and_then(|id| original.blocks.iter().find(|block| block.id == id))
        .ok_or_else(|| TransactionCheckError::compiler("unroll body is missing"))?;
    let source_order = body
        .instructions
        .iter()
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>();
    let body_units = u32::try_from(body.instructions.len()).unwrap_or(u32::MAX);
    if trip_count != plan.trip_count
        || plan.proof.iterations != trip_count
        || plan.proof.source_order != source_order
        || plan.body_units != body_units
        || plan.proof.factor != plan.factor
        || plan.proof.remainder != plan.remainder
        || (plan.full
            && (trip_count > 8 || body_units > 16 || plan.factor != 1 || plan.remainder != 0))
        || (!plan.full
            && (!matches!(plan.factor, 2 | 4)
                || plan.remainder
                    != u8::try_from(trip_count % u32::from(plan.factor)).unwrap_or(u8::MAX)))
    {
        return malformed("unroll trip/body/factor/remainder proof is false");
    }
    if body.instructions.iter().any(|instruction| {
        instruction.effect.is_some()
            || instruction.memory.is_some()
            || matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Mod,
                    ..
                } | KirInstructionKind::Binary {
                    semantics: crate::KirArithmeticSemantics::Checked,
                    ..
                } | KirInstructionKind::Unary {
                    semantics: crate::KirArithmeticSemantics::Checked,
                    ..
                }
            )
            || matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Div,
                    semantics,
                    ..
                } if semantics != crate::KirArithmeticSemantics::StrictFloat
            )
            || matches!(
                instruction.kind,
                KirInstructionKind::CheckCondition { .. }
                    | KirInstructionKind::Guard { .. }
                    | KirInstructionKind::Call { .. }
                    | KirInstructionKind::RuntimeCall { .. }
                    | KirInstructionKind::Load { .. }
                    | KirInstructionKind::Store { .. }
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
                    | KirInstructionKind::VectorReduce { .. }
            )
    }) {
        return malformed("unroll duplicates an ordered stop point or vector operation");
    }

    let expected_cost = recompute_cost(trip_count, body_units, plan.factor, plan.full);
    if plan.cost != expected_cost {
        return malformed("unroll cost decomposition is false");
    }
    if require_standalone_profitability && !unroll_profitability_threshold(expected_cost) {
        return Err(TransactionCheckError::reject(
            "unroll-profitability-threshold-not-met",
        ));
    }
    verify_mapping(original, trial, plan, &source_order)?;
    verify_only_function_changed(pre_state, trial, plan.function)?;
    verify_growth(pre_state, trial, original, descriptor, plan)?;
    if charge != &recompute_charge(plan) {
        return malformed("unroll budget charge is false");
    }
    Ok(())
}

#[must_use]
pub fn combined_unroll_slp_cost(unroll: &UnrollPlan, slp: &SlpPlan) -> KirCostEstimate {
    let executions = if unroll.full {
        1
    } else {
        unroll.trip_count / u32::from(unroll.factor)
    };
    let local_savings = slp.cost.scalar.saturating_sub(slp.cost.total);
    let transformed = unroll
        .cost
        .total
        .saturating_sub(local_savings.saturating_mul(executions));
    KirCostEstimate::new(unroll.cost.scalar, transformed, 0, 0)
}

#[allow(clippy::too_many_arguments)]
pub fn check_unroll_slp_trial_independently(
    pre_state: &KirVerifiedProgramState,
    intermediate: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    unroll_plan: &UnrollPlan,
    unroll_charge: &CandidateBudgetCharge,
    slp_plan: &SlpPlan,
    slp_charge: &CandidateBudgetCharge,
    combined_cost: KirCostEstimate,
    combined_charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_unroll_structure_independently(pre_state, intermediate, unroll_plan, unroll_charge)?;
    super::check_slp_plan_independently(intermediate, trial, slp_plan, slp_charge)?;
    let expected_cost = combined_unroll_slp_cost(unroll_plan, slp_plan);
    if combined_cost != expected_cost {
        return Err(TransactionCheckError::compiler(
            "combined unroll+SLP cost decomposition is false",
        ));
    }
    if !unroll_profitability_threshold(expected_cost) {
        return Err(TransactionCheckError::reject(
            "combined-unroll-slp-profitability-threshold-not-met",
        ));
    }
    let expected_charge = CandidateBudgetCharge::single(
        unroll_plan.function,
        unroll_charge
            .proposer_units
            .saturating_add(slp_charge.proposer_units),
        unroll_charge
            .checker_units
            .saturating_add(slp_charge.checker_units),
    );
    if combined_charge != &expected_charge {
        return Err(TransactionCheckError::compiler(
            "combined unroll+SLP budget charge is false",
        ));
    }
    Ok(())
}

fn verify_mapping(
    original: &crate::KirFunction,
    trial: &KirVerifiedProgramState,
    plan: &UnrollPlan,
    source_order: &[crate::InstructionId],
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    let emitted_iterations = if plan.full {
        plan.trip_count
    } else {
        u32::from(plan.factor).saturating_add(u32::from(plan.remainder))
    };
    let expected_len = usize::try_from(emitted_iterations)
        .ok()
        .and_then(|iterations| iterations.checked_mul(source_order.len()))
        .ok_or_else(|| TransactionCheckError::compiler("unroll mapping length overflows"))?;
    if plan.instruction_mapping.len() != expected_len {
        return malformed("unroll iteration coverage is incomplete");
    }
    let mut transformed_ids = BTreeSet::new();
    let trial_function = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.function)
        .ok_or_else(|| TransactionCheckError::compiler("unroll trial function is missing"))?;
    for (index, mapping) in plan.instruction_mapping.iter().enumerate() {
        let iteration = u32::try_from(index / source_order.len()).unwrap_or(u32::MAX);
        let source = source_order[index % source_order.len()];
        if mapping.scalar_iteration != iteration || mapping.source != source {
            return malformed("unroll iteration or source order mapping is false");
        }
        if iteration == 0 && mapping.transformed != source {
            return malformed("unroll first scalar iteration identity is false");
        }
        if !transformed_ids.insert(mapping.transformed) {
            return malformed("unroll transformed instruction identity is reused");
        }
        let source_instruction = original
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == source)
            .expect("source order came from original body");
        let Some(transformed) = trial_function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == mapping.transformed)
        else {
            return malformed("unroll mapped instruction is missing from trial");
        };
        if std::mem::discriminant(&source_instruction.kind)
            != std::mem::discriminant(&transformed.kind)
            || source_instruction.results.len() != transformed.results.len()
        {
            return malformed("unroll mapped operation is not isomorphic");
        }
    }
    if plan.full {
        let loops = analyze_canonical_loops(trial_function);
        if loops
            .loops
            .iter()
            .any(|descriptor| descriptor.header == plan.header)
        {
            return malformed("full unroll left the source backedge live");
        }
    } else {
        let Some(header) = trial_function
            .blocks
            .iter()
            .find(|block| block.id == plan.header)
        else {
            return malformed("partial unroll removed the loop header");
        };
        let KirTerminator::Branch { else_edge, .. } = &header.terminator else {
            return malformed("partial unroll removed the header branch");
        };
        if plan.remainder != 0
            && !trial_function.blocks.iter().any(|block| {
                block.id == else_edge.target && block.label.contains("unroll_remainder")
            })
        {
            return malformed("partial unroll exact remainder block is missing");
        }
    }
    Ok(())
}

fn verify_only_function_changed(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    function: crate::FunctionId,
) -> Result<(), TransactionCheckError> {
    if pre.module().config != trial.module().config
        || pre.module().profile != trial.module().profile
        || pre.module().entry != trial.module().entry
        || pre.module().structs != trial.module().structs
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
    {
        return Err(TransactionCheckError::compiler(
            "unroll trial mutated non-KIR or evidence state",
        ));
    }
    for original in &pre.module().functions {
        if original.id == function {
            continue;
        }
        if trial
            .module()
            .functions
            .iter()
            .find(|candidate| candidate.id == original.id)
            != Some(original)
        {
            return Err(TransactionCheckError::compiler(
                "unroll trial mutated another function",
            ));
        }
    }
    if pre.module().functions.len() != trial.module().functions.len() {
        return Err(TransactionCheckError::compiler(
            "unroll trial changed the function set",
        ));
    }
    Ok(())
}

fn verify_growth(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    original: &crate::KirFunction,
    descriptor: &crate::CanonicalLoopDescriptor,
    plan: &UnrollPlan,
) -> Result<(), TransactionCheckError> {
    let module_before = module_units(pre.module());
    let module_after = module_units(trial.module());
    let original_loop_units = descriptor.blocks.iter().fold(0_u32, |total, id| {
        total.saturating_add(
            original
                .blocks
                .iter()
                .find(|block| block.id == *id)
                .map_or(0, block_units),
        )
    });
    let before_function = kir_function_units(original);
    let after_function = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.function)
        .map(kir_function_units)
        .ok_or_else(|| TransactionCheckError::compiler("unroll trial function is missing"))?;
    let transformed_loop_units =
        after_function.saturating_sub(before_function.saturating_sub(original_loop_units));
    if plan.growth.original_units != original_loop_units
        || plan.growth.transformed_units != transformed_loop_units
        || plan.growth.module_before_units != module_before
        || plan.growth.module_after_units != module_after
    {
        return Err(TransactionCheckError::compiler(
            "unroll structural growth record is false",
        ));
    }
    if transformed_loop_units > original_loop_units.saturating_mul(3).saturating_add(32)
        || module_after > plan.o3_entry_module_units.saturating_mul(2)
    {
        return Err(TransactionCheckError::reject("unroll-code-growth-limit"));
    }
    Ok(())
}

fn recompute_cost(trip: u32, body: u32, factor: u8, full: bool) -> KirCostEstimate {
    let scalar = if trip == 0 {
        2
    } else {
        trip.saturating_mul(body.saturating_add(2))
            .saturating_add(1)
    };
    let transformed = if full {
        trip.saturating_mul(body)
    } else {
        trip.saturating_mul(body)
            .saturating_add((trip / u32::from(factor)).saturating_mul(2))
            .saturating_add(2)
    };
    KirCostEstimate::new(scalar, transformed, 0, 0)
}

fn recompute_charge(plan: &UnrollPlan) -> CandidateBudgetCharge {
    let mappings = u32::try_from(plan.instruction_mapping.len()).unwrap_or(u32::MAX);
    CandidateBudgetCharge::single(
        plan.function,
        8_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(mappings),
        16_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(plan.growth.transformed_units)
            .saturating_add(mappings.saturating_mul(2)),
    )
}

fn pre_identity(
    state: &KirVerifiedProgramState,
    function: crate::FunctionId,
) -> KirPreStateIdentity {
    let frozen = state
        .module()
        .functions
        .iter()
        .find(|item| item.id == function)
        .map_or(0, kir_function_units);
    KirPreStateIdentity {
        function,
        kir_digest: state.kir_digest(),
        profile_digest: state.module().profile.digest_hex(),
        evidence_generation: state.evidence_generation(),
        frozen_kir_units: frozen,
    }
}

fn exact_trip(trip: &LoopTripCount) -> Option<u32> {
    match trip {
        LoopTripCount::Zero => Some(0),
        LoopTripCount::Exact { iterations } => u32::try_from(*iterations).ok(),
        LoopTripCount::Runtime { .. } | LoopTripCount::Unknown => None,
    }
}

fn block_units(block: &crate::KirBlock) -> u32 {
    1_u32
        .saturating_add(u32::try_from(block.params.len()).unwrap_or(u32::MAX))
        .saturating_add(u32::try_from(block.memory_params.len()).unwrap_or(u32::MAX))
        .saturating_add(u32::try_from(block.instructions.len()).unwrap_or(u32::MAX))
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}
