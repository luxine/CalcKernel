use std::collections::BTreeSet;

use crate::{
    AliasKind, CandidateBudgetCharge, KirAlignmentClass, KirArithmeticSemantics, KirCostEstimate,
    KirCostKey, KirCostSemantics, KirEffectKind, KirInstruction, KirInstructionKind, KirLaneType,
    KirOperationAvailability, KirPlace, KirPreStateIdentity, KirProfileOperation, KirValueType,
    MirBinaryOp, MirPrimitiveTypeName, MirType, SlpPlan, TransactionCheckError, analyze_regions,
    kir_function_units, query_alias,
};

#[must_use]
pub const fn slp_profitability_threshold(cost: KirCostEstimate) -> bool {
    cost.scalar >= cost.total.saturating_add(2)
        && (cost.total as u64).saturating_mul(100) <= (cost.scalar as u64).saturating_mul(90)
}

pub fn check_slp_plan_independently(
    pre_state: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_slp_plan_internal(pre_state, trial, plan, charge, true)
}

pub fn check_tuned_slp_plan_independently(
    pre_state: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_slp_plan_internal(pre_state, trial, plan, charge, false)
}

fn check_slp_plan_internal(
    pre_state: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
    charge: &CandidateBudgetCharge,
    require_static_profitability: bool,
) -> Result<(), TransactionCheckError> {
    if plan.memory.is_some() {
        return check_memory_slp_plan(pre_state, trial, plan, charge, require_static_profitability);
    }
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    let function_id = plan.pre_state.function;
    if plan.pre_state != pre_identity(pre_state, function_id)
        || !matches!(plan.lanes, 2 | 4)
        || plan.proof.block != plan.block
        || plan.proof.source_order != plan.scalar_instructions
        || plan.proof.identity_lanes != (0..plan.lanes).collect::<Vec<_>>()
        || !plan.proof.barrier_free
        || plan.proof.exact_memory_footprint.is_some()
        || !plan.setup_instructions.is_empty()
        || plan.scalar_instructions.first().copied() != Some(plan.root)
    {
        return malformed("SLP pre-state, lane identity, or proof record is false");
    }
    let Some(original) = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == function_id)
    else {
        return malformed("SLP original function is missing");
    };
    let Some(block) = original.blocks.iter().find(|block| block.id == plan.block) else {
        return malformed("SLP original block is missing");
    };
    if plan.scalar_instructions.len() != usize::from(plan.lanes) {
        return malformed("SLP source pack does not cover every lane");
    }
    let protected = pre_state.proofs().instruction_dependencies();
    let mut scalar_results = BTreeSet::new();
    let mut operation = None;
    let mut scalar_cost = 0_u32;
    let mut positions = Vec::new();
    for id in &plan.scalar_instructions {
        let Some((position, instruction)) = block
            .instructions
            .iter()
            .enumerate()
            .find(|(_, instruction)| instruction.id == *id)
        else {
            return malformed("SLP source instruction is missing");
        };
        if protected.contains(id)
            || instruction.effect.is_some()
            || instruction.memory.is_some()
            || instruction.results.len() != 1
        {
            return malformed("SLP source crosses a barrier or certificate dependency");
        }
        let KirInstructionKind::Binary {
            op,
            left: _,
            right: _,
            semantics,
        } = instruction.kind
        else {
            return malformed("SLP source operation is not a scalar binary");
        };
        let (profile_operation, cost_semantics) = scalar_operation(op, semantics)?;
        let lane = lane_type(&instruction.results[0].type_node)
            .ok_or_else(|| TransactionCheckError::compiler("SLP scalar lane type is invalid"))?;
        if lane != plan.lane_type || cost_semantics != plan.semantics {
            return malformed("SLP lane type or arithmetic semantics changed");
        }
        if operation
            .replace(profile_operation)
            .is_some_and(|previous| previous != profile_operation)
        {
            return malformed("SLP operations are not isomorphic");
        }
        scalar_cost = scalar_cost.saturating_add(profile_cost(
            pre_state,
            profile_operation,
            lane,
            1,
            cost_semantics,
        )?);
        scalar_results.insert(instruction.results[0].value);
        positions.push(position);
    }
    if positions.windows(2).any(|pair| pair[0] >= pair[1]) {
        return malformed("SLP scalar pack is not in source order");
    }
    let first = positions[0];
    let last = *positions.last().expect("nonempty SLP positions");
    if block.instructions[first..=last]
        .iter()
        .any(|instruction| is_barrier(instruction, &protected))
    {
        return malformed("SLP pack crosses an ordered barrier");
    }
    for id in &plan.scalar_instructions {
        let instruction = block
            .instructions
            .iter()
            .find(|instruction| instruction.id == *id)
            .expect("source established");
        let KirInstructionKind::Binary { left, right, .. } = instruction.kind else {
            unreachable!()
        };
        if scalar_results.contains(&left) || scalar_results.contains(&right) {
            return malformed("SLP lanes are not independent");
        }
    }
    let operation = operation.expect("nonempty SLP operation");
    let expected_operations = expected_operations(operation, plan.lanes);
    if plan.operations != expected_operations {
        return malformed("SLP emitted operation sequence is false");
    }
    let transformed_cost = plan.operations.iter().try_fold(0_u32, |total, item| {
        let semantics = if *item == operation {
            plan.semantics
        } else {
            KirCostSemantics::NotApplicable
        };
        profile_cost(
            pre_state,
            *item,
            plan.lane_type,
            u8::try_from(plan.lanes).unwrap_or(u8::MAX),
            semantics,
        )
        .map(|cost| total.saturating_add(cost))
    })?;
    let expected_cost = KirCostEstimate::new(scalar_cost, transformed_cost, 0, 0);
    if plan.cost != expected_cost {
        return malformed("SLP cost decomposition is false");
    }
    if require_static_profitability && !slp_profitability_threshold(expected_cost) {
        return Err(TransactionCheckError::reject(
            "slp-profitability-threshold-not-met",
        ));
    }
    verify_trial(pre_state, trial, plan, operation)?;
    verify_growth(pre_state, trial, plan)?;
    if charge != &recompute_charge(plan) {
        return malformed("SLP budget charge is false");
    }
    Ok(())
}

fn check_memory_slp_plan(
    pre_state: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
    charge: &CandidateBudgetCharge,
    require_static_profitability: bool,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    let function_id = plan.pre_state.function;
    let Some(memory) = plan.memory.as_ref() else {
        return malformed("SLP memory plan is missing");
    };
    let lanes = usize::from(plan.lanes);
    let lane_bytes = checker_lane_bytes(plan.lane_type)?;
    let footprint = lane_bytes.saturating_mul(u32::from(plan.lanes));
    if plan.pre_state != pre_identity(pre_state, function_id)
        || !matches!(plan.lanes, 2 | 4)
        || plan.proof.block != plan.block
        || plan.proof.identity_lanes != (0..plan.lanes).collect::<Vec<_>>()
        || !plan.proof.barrier_free
        || plan.proof.exact_memory_footprint != Some(footprint)
        || plan.scalar_instructions.first().copied() != Some(plan.root)
        || plan.scalar_instructions.len() != lanes
        || memory.left_loads.len() != lanes
        || memory.right_loads.len() != lanes
        || memory.stores.len() != lanes
        || memory.vector_loads.len() != 2
        || plan.setup_instructions.len() != 1
        || plan.vector_instructions.len() != 4
        || !plan.extracts.is_empty()
    {
        return malformed("SLP memory pre-state, lane identity, or proof record is false");
    }
    let Some(original) = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == function_id)
    else {
        return malformed("SLP memory original function is missing");
    };
    let Some(block) = original.blocks.iter().find(|block| block.id == plan.block) else {
        return malformed("SLP memory original block is missing");
    };
    let find = |id| {
        block
            .instructions
            .iter()
            .find(|instruction| instruction.id == id)
            .ok_or_else(|| TransactionCheckError::compiler("SLP memory source is missing"))
    };
    let left_sources = memory
        .left_loads
        .iter()
        .map(|id| find(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let right_sources = memory
        .right_loads
        .iter()
        .map(|id| find(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let binary_sources = plan
        .scalar_instructions
        .iter()
        .map(|id| find(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let store_sources = memory
        .stores
        .iter()
        .map(|id| find(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let protected = pre_state.proofs().instruction_dependencies();
    let selected = memory
        .left_loads
        .iter()
        .chain(&memory.right_loads)
        .chain(&plan.scalar_instructions)
        .chain(&memory.stores)
        .copied()
        .collect::<BTreeSet<_>>();
    if selected.len() != lanes.saturating_mul(4) || selected.iter().any(|id| protected.contains(id))
    {
        return malformed("SLP memory source mapping is duplicated or proof-protected");
    }
    let mut expected_source_order = selected.iter().copied().collect::<Vec<_>>();
    expected_source_order.sort_by_key(|id| {
        block
            .instructions
            .iter()
            .position(|instruction| instruction.id == *id)
            .unwrap_or(usize::MAX)
    });
    if plan.proof.source_order != expected_source_order {
        return malformed("SLP memory source order is false");
    }
    let positions = expected_source_order
        .iter()
        .filter_map(|id| {
            block
                .instructions
                .iter()
                .position(|instruction| instruction.id == *id)
        })
        .collect::<Vec<_>>();
    if positions.len() != selected.len() || positions.windows(2).any(|pair| pair[0] >= pair[1]) {
        return malformed("SLP memory source order is not present in the original block");
    }
    let first_position = positions[0];
    let last_position = *positions.last().expect("nonempty memory SLP source");
    if block.instructions[first_position..=last_position]
        .iter()
        .any(|instruction| {
            (instruction.memory.is_some() || instruction.effect.is_some())
                && !selected.contains(&instruction.id)
        })
    {
        return malformed("SLP memory source crosses an unselected ordered barrier");
    }

    let (left_slice, left_start, left_type, _) = checker_load_components(left_sources[0])?;
    let (right_slice, right_start, right_type, _) = checker_load_components(right_sources[0])?;
    let (output_slice, output_start, output_type, _, _) =
        checker_store_components(store_sources[0])?;
    let start_number = checker_constant_u32(block, left_start)?;
    if checker_constant_u32(block, right_start)? != start_number
        || checker_constant_u32(block, output_start)? != start_number
        || left_type != right_type
        || left_type != output_type
        || lane_type(&KirValueType::scalar(left_type.clone())) != Some(plan.lane_type)
        || left_slice == right_slice
        || left_slice == output_slice
        || right_slice == output_slice
    {
        return malformed("SLP memory roots, starts, or lane types are false");
    }
    let regions = analyze_regions(
        original,
        pre_state
            .contract_facts()
            .map(crate::ContractFactSet::facts),
    )
    .map_err(|error| TransactionCheckError::compiler(error.message))?;
    let (Some(left_region), Some(right_region), Some(output_region)) = (
        regions.region_for_value(left_slice),
        regions.region_for_value(right_slice),
        regions.region_for_value(output_slice),
    ) else {
        return malformed("SLP memory roots have no stable region identity");
    };
    if [
        query_alias(&regions, left_region, right_region).kind,
        query_alias(&regions, left_region, output_region).kind,
        query_alias(&regions, right_region, output_region).kind,
    ]
    .into_iter()
    .any(|kind| kind != AliasKind::NoAlias)
    {
        return malformed("SLP memory reordering lacks pairwise noalias evidence");
    }

    let mut operation = None;
    let mut scalar_cost = 0_u32;
    let mut removed_results = BTreeSet::new();
    for lane in 0..lanes {
        let expected_index = start_number
            .checked_add(u32::try_from(lane).unwrap_or(u32::MAX))
            .ok_or_else(|| TransactionCheckError::compiler("SLP memory lane index overflows"))?;
        let (lane_left_slice, lane_left_index, lane_left_type, lane_left_region) =
            checker_load_components(left_sources[lane])?;
        let (lane_right_slice, lane_right_index, lane_right_type, lane_right_region) =
            checker_load_components(right_sources[lane])?;
        let (lane_output_slice, lane_output_index, lane_output_type, lane_output_region, stored) =
            checker_store_components(store_sources[lane])?;
        let binary = binary_sources[lane];
        let KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } = binary.kind
        else {
            return malformed("SLP memory compute is not a scalar binary");
        };
        let (profile_operation, cost_semantics) = scalar_operation(op, semantics)?;
        if operation
            .replace(profile_operation)
            .is_some_and(|previous| previous != profile_operation)
            || cost_semantics != plan.semantics
            || lane_left_slice != left_slice
            || lane_right_slice != right_slice
            || lane_output_slice != output_slice
            || lane_left_type != left_type
            || lane_right_type != left_type
            || lane_output_type != left_type
            || checker_constant_u32(block, lane_left_index)? != expected_index
            || checker_constant_u32(block, lane_right_index)? != expected_index
            || checker_constant_u32(block, lane_output_index)? != expected_index
            || left_sources[lane]
                .results
                .first()
                .map(|result| result.value)
                != Some(left)
            || right_sources[lane]
                .results
                .first()
                .map(|result| result.value)
                != Some(right)
            || binary.results.first().map(|result| result.value) != Some(stored)
            || left_sources[lane].memory.as_ref().is_none_or(|memory| {
                regions.partition(lane_left_region) != Some(memory.region)
                    || memory.output.is_some()
            })
            || right_sources[lane].memory.as_ref().is_none_or(|memory| {
                regions.partition(lane_right_region) != Some(memory.region)
                    || memory.output.is_some()
            })
            || store_sources[lane].memory.as_ref().is_none_or(|memory| {
                regions.partition(lane_output_region) != Some(memory.region)
                    || memory.output.is_none()
            })
            || left_sources[lane]
                .effect
                .as_ref()
                .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
            || right_sources[lane]
                .effect
                .as_ref()
                .is_none_or(|effect| effect.kind != KirEffectKind::ReadMemory)
            || store_sources[lane]
                .effect
                .as_ref()
                .is_none_or(|effect| effect.kind != KirEffectKind::WriteMemory)
            || binary.memory.is_some()
            || binary.effect.is_some()
            || binary.results.len() != 1
        {
            return malformed("SLP memory lane is not an isomorphic contiguous operation");
        }
        removed_results.extend(left_sources[lane].results.iter().map(|result| result.value));
        removed_results.extend(
            right_sources[lane]
                .results
                .iter()
                .map(|result| result.value),
        );
        removed_results.extend(binary.results.iter().map(|result| result.value));
        scalar_cost = scalar_cost
            .saturating_add(memory_profile_cost(
                pre_state,
                KirProfileOperation::Load,
                plan.lane_type,
                1,
                KirCostSemantics::NotApplicable,
                lane_bytes,
            )?)
            .saturating_add(memory_profile_cost(
                pre_state,
                KirProfileOperation::Load,
                plan.lane_type,
                1,
                KirCostSemantics::NotApplicable,
                lane_bytes,
            )?)
            .saturating_add(memory_profile_cost(
                pre_state,
                profile_operation,
                plan.lane_type,
                1,
                plan.semantics,
                lane_bytes,
            )?)
            .saturating_add(memory_profile_cost(
                pre_state,
                KirProfileOperation::Store,
                plan.lane_type,
                1,
                KirCostSemantics::NotApplicable,
                lane_bytes,
            )?);
    }
    for instruction in block
        .instructions
        .iter()
        .filter(|instruction| !selected.contains(&instruction.id))
    {
        let mut escaped = false;
        super::analysis::visit_instruction_uses(instruction, &mut |value| {
            escaped |= removed_results.contains(&value);
        });
        if escaped {
            return malformed("SLP memory removed value has an external instruction use");
        }
    }
    if terminator_uses(&block.terminator)
        .into_iter()
        .any(|value| removed_results.contains(&value))
    {
        return malformed("SLP memory removed value escapes through the terminator");
    }
    let operation = operation.expect("nonempty memory SLP operation");
    let expected_operations = vec![
        KirProfileOperation::Load,
        KirProfileOperation::Load,
        operation,
        KirProfileOperation::Store,
    ];
    if plan.operations != expected_operations {
        return malformed("SLP memory emitted operation sequence is false");
    }
    let transformed_cost = plan.operations.iter().try_fold(0_u32, |total, item| {
        memory_profile_cost(
            pre_state,
            *item,
            plan.lane_type,
            u8::try_from(plan.lanes).unwrap_or(u8::MAX),
            if *item == operation {
                plan.semantics
            } else {
                KirCostSemantics::NotApplicable
            },
            lane_bytes,
        )
        .map(|cost| total.saturating_add(cost))
    })?;
    let expected_cost = KirCostEstimate::new(scalar_cost, transformed_cost, 0, 0);
    if plan.cost != expected_cost {
        return malformed("SLP memory cost decomposition is false");
    }
    if require_static_profitability && !slp_profitability_threshold(expected_cost) {
        return Err(TransactionCheckError::reject(
            "slp-profitability-threshold-not-met",
        ));
    }
    verify_memory_trial(
        pre_state,
        trial,
        plan,
        operation,
        footprint,
        left_slice,
        left_start,
        right_slice,
        output_slice,
        left_sources[0],
        right_sources[0],
        store_sources[0],
        store_sources[lanes - 1],
    )?;
    verify_growth(pre_state, trial, plan)?;
    if charge != &recompute_charge(plan) {
        return malformed("SLP memory budget charge is false");
    }
    Ok(())
}

fn verify_trial(
    pre: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
    operation: KirProfileOperation,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    if pre.module().config != trial.module().config
        || pre.module().profile != trial.module().profile
        || pre.module().entry != trial.module().entry
        || pre.module().structs != trial.module().structs
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
        || pre.module().functions.len() != trial.module().functions.len()
    {
        return malformed("SLP trial mutated nonlocal state or evidence");
    }
    for original in &pre.module().functions {
        if original.id != plan.pre_state.function
            && trial
                .module()
                .functions
                .iter()
                .find(|function| function.id == original.id)
                != Some(original)
        {
            return malformed("SLP trial mutated another function");
        }
    }
    let function = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
        .ok_or_else(|| TransactionCheckError::compiler("SLP trial function is missing"))?;
    let block = function
        .blocks
        .iter()
        .find(|block| block.id == plan.block)
        .ok_or_else(|| TransactionCheckError::compiler("SLP trial block is missing"))?;
    if plan.scalar_instructions.iter().any(|id| {
        block
            .instructions
            .iter()
            .any(|instruction| instruction.id == *id)
    }) {
        return malformed("SLP trial retained a packed scalar instruction");
    }
    let actual = plan
        .vector_instructions
        .iter()
        .chain(&plan.extracts)
        .map(|id| {
            block
                .instructions
                .iter()
                .find(|instruction| instruction.id == *id)
                .ok_or_else(|| TransactionCheckError::compiler("SLP vector mapping is missing"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual.len() != plan.operations.len()
        || actual
            .iter()
            .zip(&plan.operations)
            .any(|(instruction, expected)| instruction_operation(instruction) != Some(*expected))
        || actual
            .iter()
            .filter(|instruction| {
                matches!(instruction.kind, KirInstructionKind::VectorBinary { .. })
            })
            .count()
            != 1
        || !actual.iter().any(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VectorBinary { op, .. }
                    if vector_operation(op) == operation
            )
        })
    {
        return malformed("SLP vector operation mapping is false");
    }
    if !function.vector_regions.iter().any(|region| {
        region.blocks == [plan.block]
            && actual
                .iter()
                .all(|instruction| instruction_region(instruction) == Some(region.id))
    }) {
        return malformed("SLP vector region does not own exactly the packed block");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn verify_memory_trial(
    pre: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
    operation: KirProfileOperation,
    footprint: u32,
    left_slice: crate::ValueId,
    left_start: crate::ValueId,
    right_slice: crate::ValueId,
    output_slice: crate::ValueId,
    first_left: &KirInstruction,
    first_right: &KirInstruction,
    first_store: &KirInstruction,
    last_store: &KirInstruction,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    if pre.module().config != trial.module().config
        || pre.module().profile != trial.module().profile
        || pre.module().entry != trial.module().entry
        || pre.module().structs != trial.module().structs
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
        || pre.module().functions.len() != trial.module().functions.len()
    {
        return malformed("SLP memory trial mutated nonlocal state or evidence");
    }
    for original in &pre.module().functions {
        if original.id != plan.pre_state.function
            && trial
                .module()
                .functions
                .iter()
                .find(|function| function.id == original.id)
                != Some(original)
        {
            return malformed("SLP memory trial mutated another function");
        }
    }
    let original = pre
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
        .ok_or_else(|| {
            TransactionCheckError::compiler("SLP memory original function is missing")
        })?;
    let original_block = original
        .blocks
        .iter()
        .find(|block| block.id == plan.block)
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory original block is missing"))?;
    let function = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory trial function is missing"))?;
    let block = function
        .blocks
        .iter()
        .find(|block| block.id == plan.block)
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory trial block is missing"))?;
    let memory = plan.memory.as_ref().expect("memory plan established");
    if memory.vector_loads != plan.vector_instructions[..2]
        || memory.vector_store != plan.vector_instructions[3]
    {
        return malformed("SLP memory vector mapping is false");
    }
    let setup = block
        .instructions
        .iter()
        .find(|instruction| instruction.id == plan.setup_instructions[0])
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory setup is missing"))?;
    let end_value = setup
        .results
        .first()
        .filter(|result| {
            result.type_node == KirValueType::scalar(MirType::Primitive(MirPrimitiveTypeName::U32))
        })
        .map(|result| result.value)
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory end value is not u32"))?;
    let expected_end = checker_constant_u32(original_block, left_start)?
        .checked_add(u32::from(plan.lanes))
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory end overflows u32"))?;
    if !matches!(&setup.kind, KirInstructionKind::ConstInt { value } if value.parse::<u32>() == Ok(expected_end))
        || setup.memory.is_some()
        || setup.effect.is_some()
    {
        return malformed("SLP memory end setup is false");
    }
    let actual = plan
        .vector_instructions
        .iter()
        .map(|id| {
            block
                .instructions
                .iter()
                .find(|instruction| instruction.id == *id)
                .ok_or_else(|| {
                    TransactionCheckError::compiler("SLP memory vector instruction is missing")
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [left_load, right_load, binary, store] = actual.as_slice() else {
        return malformed("SLP memory vector operation count is false");
    };
    let KirInstructionKind::VectorLoad {
        access: left_access,
        region,
    } = &left_load.kind
    else {
        return malformed("SLP memory left vector load is false");
    };
    let vector_region = *region;
    let KirInstructionKind::VectorLoad {
        access: right_access,
        region: right_vector_region,
    } = &right_load.kind
    else {
        return malformed("SLP memory right vector load is false");
    };
    let KirInstructionKind::VectorBinary {
        op,
        left,
        right,
        semantics,
        no_failure_proof,
        region: binary_region,
    } = &binary.kind
    else {
        return malformed("SLP memory vector compute is false");
    };
    let KirInstructionKind::VectorStore {
        access: output_access,
        value,
        region: store_region,
    } = &store.kind
    else {
        return malformed("SLP memory vector store is false");
    };
    let expected_access = |slice, start| crate::KirVectorMemoryAccess {
        slice,
        start,
        end: end_value,
        lane: plan.lane_type,
        lanes: plan.lanes,
        byte_footprint: footprint,
        known_alignment: u16::try_from(checker_lane_bytes(plan.lane_type).unwrap_or(u32::MAX))
            .unwrap_or(u16::MAX),
        required_alignment: u16::try_from(checker_lane_bytes(plan.lane_type).unwrap_or(u32::MAX))
            .unwrap_or(u16::MAX),
    };
    let left_value = left_load.results.first().map(|result| result.value);
    let right_value = right_load.results.first().map(|result| result.value);
    let binary_value = binary.results.first().map(|result| result.value);
    let vector_type = KirValueType::FixedVector {
        lane: plan.lane_type,
        lanes: plan.lanes,
    };
    if left_access != &expected_access(left_slice, left_start)
        || right_access != &expected_access(right_slice, left_start)
        || output_access != &expected_access(output_slice, left_start)
        || *right_vector_region != vector_region
        || *binary_region != vector_region
        || *store_region != vector_region
        || vector_operation(*op) != operation
        || *semantics != checker_arithmetic_semantics(plan.semantics)?
        || no_failure_proof.is_some()
        || Some(*left) != left_value
        || Some(*right) != right_value
        || Some(*value) != binary_value
        || left_load.results.len() != 1
        || right_load.results.len() != 1
        || binary.results.len() != 1
        || left_load.results[0].type_node != vector_type
        || right_load.results[0].type_node != vector_type
        || binary.results[0].type_node != vector_type
        || !store.results.is_empty()
    {
        return malformed("SLP memory vector dataflow or footprint is false");
    }
    let mut expected_store_memory = first_store
        .memory
        .clone()
        .ok_or_else(|| TransactionCheckError::compiler("SLP first store MemorySSA is missing"))?;
    expected_store_memory.output = last_store.memory.as_ref().and_then(|memory| memory.output);
    if left_load.memory != first_left.memory
        || left_load.effect != first_left.effect
        || right_load.memory != first_right.memory
        || right_load.effect != first_right.effect
        || binary.memory.is_some()
        || binary.effect.is_some()
        || store.memory.as_ref() != Some(&expected_store_memory)
        || store.effect != last_store.effect
    {
        return malformed("SLP memory MemorySSA or effect ordering is false");
    }
    let original_region = original
        .vector_regions
        .iter()
        .find(|candidate| candidate.blocks.contains(&plan.block));
    let region_shape_is_valid = function
        .vector_regions
        .iter()
        .any(|candidate| candidate.id == vector_region && candidate.blocks == [plan.block]);
    let region_lifecycle_is_valid = original_region.map_or_else(
        || {
            function.vector_regions.len() == original.vector_regions.len().saturating_add(1)
                && !original
                    .vector_regions
                    .iter()
                    .any(|candidate| candidate.id == vector_region)
        },
        |existing| {
            existing.id == vector_region
                && function.vector_regions.len() == original.vector_regions.len()
                && function.vector_regions == original.vector_regions
        },
    );
    if !region_shape_is_valid || !region_lifecycle_is_valid {
        return malformed("SLP memory vector region ownership is false");
    }
    let emitted = plan
        .setup_instructions
        .iter()
        .chain(&plan.vector_instructions)
        .copied()
        .collect::<BTreeSet<_>>();
    if emitted.len() != 5 {
        return malformed("SLP memory emitted instruction mapping is duplicated");
    }
    let removed = plan
        .proof
        .source_order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if removed.iter().any(|id| {
        block
            .instructions
            .iter()
            .any(|instruction| instruction.id == *id)
    }) {
        return malformed("SLP memory trial retained a packed scalar instruction");
    }
    let expected_remaining = original_block
        .instructions
        .iter()
        .filter(|instruction| !removed.contains(&instruction.id))
        .collect::<Vec<_>>();
    let actual_remaining = block
        .instructions
        .iter()
        .filter(|instruction| !emitted.contains(&instruction.id))
        .collect::<Vec<_>>();
    if actual_remaining != expected_remaining
        || block.params != original_block.params
        || block.memory_params != original_block.memory_params
        || block.terminator != original_block.terminator
    {
        return malformed("SLP memory trial changed unrelated block state");
    }
    let insertion = original_block
        .instructions
        .iter()
        .position(|instruction| removed.contains(&instruction.id))
        .expect("memory SLP source exists");
    let mut expected_ids = Vec::new();
    for (position, instruction) in original_block.instructions.iter().enumerate() {
        if position == insertion {
            expected_ids.extend(plan.setup_instructions.iter().copied());
            expected_ids.extend(plan.vector_instructions.iter().copied());
        }
        if !removed.contains(&instruction.id) {
            expected_ids.push(instruction.id);
        }
    }
    if block
        .instructions
        .iter()
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>()
        != expected_ids
    {
        return malformed("SLP memory replacement position or emitted order is false");
    }
    let mut normalized = function.clone();
    if original_region.is_none() {
        normalized
            .vector_regions
            .retain(|candidate| candidate.id != vector_region);
    }
    *normalized
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == plan.block)
        .expect("SLP memory block established") = original_block.clone();
    if normalized != *original {
        return malformed("SLP memory trial changed unrelated function state");
    }
    Ok(())
}

fn verify_growth(
    pre: &crate::KirVerifiedProgramState,
    trial: &crate::KirVerifiedProgramState,
    plan: &SlpPlan,
) -> Result<(), TransactionCheckError> {
    let before = module_units(pre.module());
    let after = module_units(trial.module());
    let original = if plan.memory.is_some() {
        u32::try_from(plan.proof.source_order.len()).unwrap_or(u32::MAX)
    } else {
        u32::try_from(plan.scalar_instructions.len()).unwrap_or(u32::MAX)
    };
    let transformed = u32::try_from(
        plan.operations
            .len()
            .saturating_add(plan.setup_instructions.len()),
    )
    .unwrap_or(u32::MAX);
    if plan.growth.original_units != original
        || plan.growth.transformed_units != transformed
        || plan.growth.module_before_units != before
        || plan.growth.module_after_units != after
    {
        return Err(TransactionCheckError::compiler(
            "SLP structural growth record is false",
        ));
    }
    if transformed > original.saturating_mul(3).saturating_add(32)
        || after > pre.optimization_entry_module_units().saturating_mul(2)
    {
        return Err(TransactionCheckError::reject("slp-code-growth-limit"));
    }
    Ok(())
}

fn expected_operations(operation: KirProfileOperation, lanes: u16) -> Vec<KirProfileOperation> {
    let mut operations = vec![KirProfileOperation::Splat];
    operations.extend(std::iter::repeat_n(
        KirProfileOperation::Insert,
        usize::from(lanes.saturating_sub(1)),
    ));
    operations.push(KirProfileOperation::Splat);
    operations.extend(std::iter::repeat_n(
        KirProfileOperation::Insert,
        usize::from(lanes.saturating_sub(1)),
    ));
    operations.push(operation);
    operations.extend(std::iter::repeat_n(
        KirProfileOperation::Extract,
        usize::from(lanes),
    ));
    operations
}

fn scalar_operation(
    op: MirBinaryOp,
    semantics: KirArithmeticSemantics,
) -> Result<(KirProfileOperation, KirCostSemantics), TransactionCheckError> {
    let operation = match op {
        MirBinaryOp::Add => KirProfileOperation::Add,
        MirBinaryOp::Sub => KirProfileOperation::Subtract,
        MirBinaryOp::Mul => KirProfileOperation::Multiply,
        MirBinaryOp::Div if semantics == KirArithmeticSemantics::StrictFloat => {
            KirProfileOperation::Divide
        }
        MirBinaryOp::Div | MirBinaryOp::Mod => {
            return Err(TransactionCheckError::compiler(
                "SLP source may fail before a later lane",
            ));
        }
    };
    let semantics = match semantics {
        KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
        KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
        KirArithmeticSemantics::Checked => {
            return Err(TransactionCheckError::compiler(
                "SLP source uses checked arithmetic",
            ));
        }
    };
    Ok((operation, semantics))
}

fn lane_type(type_node: &KirValueType) -> Option<KirLaneType> {
    match type_node.as_scalar()? {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(KirLaneType::I64),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some(KirLaneType::U64),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn is_barrier(
    instruction: &crate::KirInstruction,
    protected: &BTreeSet<crate::InstructionId>,
) -> bool {
    protected.contains(&instruction.id)
        || instruction.effect.is_some()
        || instruction.memory.is_some()
        || matches!(
            instruction.kind,
            KirInstructionKind::Guard { .. }
                | KirInstructionKind::CheckCondition { .. }
                | KirInstructionKind::Call { .. }
                | KirInstructionKind::RuntimeCall { .. }
                | KirInstructionKind::Load { .. }
                | KirInstructionKind::Store { .. }
        )
}

fn profile_cost(
    state: &crate::KirVerifiedProgramState,
    operation: KirProfileOperation,
    lane: KirLaneType,
    lanes: u8,
    semantics: KirCostSemantics,
) -> Result<u32, TransactionCheckError> {
    let key = KirCostKey {
        operation,
        lane,
        lanes,
        semantics,
        alignment: KirAlignmentClass::NotApplicable,
    };
    match state.module().profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) => Ok(cost.cost),
        Some(KirOperationAvailability::Unavailable) | None => Err(TransactionCheckError::compiler(
            "SLP target operation is unavailable",
        )),
    }
}

fn memory_profile_cost(
    state: &crate::KirVerifiedProgramState,
    operation: KirProfileOperation,
    lane: KirLaneType,
    lanes: u8,
    semantics: KirCostSemantics,
    lane_bytes: u32,
) -> Result<u32, TransactionCheckError> {
    let alignment = if matches!(
        operation,
        KirProfileOperation::Load | KirProfileOperation::Store
    ) {
        KirAlignmentClass::Bytes(
            u16::try_from(lane_bytes)
                .map_err(|_| TransactionCheckError::compiler("SLP memory alignment exceeds u16"))?,
        )
    } else {
        KirAlignmentClass::NotApplicable
    };
    let key = KirCostKey {
        operation,
        lane,
        lanes,
        semantics,
        alignment,
    };
    match state.module().profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) => Ok(cost.cost),
        Some(KirOperationAvailability::Unavailable) | None => Err(TransactionCheckError::compiler(
            "SLP memory target operation is unavailable",
        )),
    }
}

fn checker_load_components(
    instruction: &KirInstruction,
) -> Result<
    (
        crate::ValueId,
        crate::ValueId,
        MirType,
        crate::MemoryRegionId,
    ),
    TransactionCheckError,
> {
    let KirInstructionKind::Load { place } = &instruction.kind else {
        return Err(TransactionCheckError::compiler(
            "SLP memory source is not a load",
        ));
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        region,
    } = place.as_ref()
    else {
        return Err(TransactionCheckError::compiler(
            "SLP memory load is not a slice index",
        ));
    };
    Ok((*slice, *index, type_node.clone(), *region))
}

fn checker_store_components(
    instruction: &KirInstruction,
) -> Result<
    (
        crate::ValueId,
        crate::ValueId,
        MirType,
        crate::MemoryRegionId,
        crate::ValueId,
    ),
    TransactionCheckError,
> {
    let KirInstructionKind::Store { place, value } = &instruction.kind else {
        return Err(TransactionCheckError::compiler(
            "SLP memory source is not a store",
        ));
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        region,
    } = place.as_ref()
    else {
        return Err(TransactionCheckError::compiler(
            "SLP memory store is not a slice index",
        ));
    };
    Ok((*slice, *index, type_node.clone(), *region, *value))
}

fn checker_constant_u32(
    block: &crate::KirBlock,
    value: crate::ValueId,
) -> Result<u32, TransactionCheckError> {
    block
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
        .and_then(|instruction| {
            let KirInstructionKind::ConstInt { value } = &instruction.kind else {
                return None;
            };
            value.parse::<u32>().ok()
        })
        .ok_or_else(|| TransactionCheckError::compiler("SLP memory index is not a u32 constant"))
}

fn checker_lane_bytes(lane: KirLaneType) -> Result<u32, TransactionCheckError> {
    match lane {
        KirLaneType::I32 | KirLaneType::U32 => Ok(4),
        KirLaneType::I64 | KirLaneType::U64 | KirLaneType::F64 => Ok(8),
    }
}

fn checker_arithmetic_semantics(
    semantics: KirCostSemantics,
) -> Result<KirArithmeticSemantics, TransactionCheckError> {
    match semantics {
        KirCostSemantics::Modular => Ok(KirArithmeticSemantics::Modular),
        KirCostSemantics::StrictFloat => Ok(KirArithmeticSemantics::StrictFloat),
        KirCostSemantics::NotApplicable | KirCostSemantics::Checked => Err(
            TransactionCheckError::compiler("SLP memory arithmetic semantics are unsupported"),
        ),
    }
}

fn terminator_uses(terminator: &crate::KirTerminator) -> Vec<crate::ValueId> {
    match terminator {
        crate::KirTerminator::Return { value, .. } => value.iter().copied().collect(),
        crate::KirTerminator::Jump { edge } => edge.args.clone(),
        crate::KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => std::iter::once(*condition)
            .chain(then_edge.args.iter().copied())
            .chain(else_edge.args.iter().copied())
            .collect(),
    }
}

fn instruction_operation(instruction: &crate::KirInstruction) -> Option<KirProfileOperation> {
    match instruction.kind {
        KirInstructionKind::VectorSplat { .. } => Some(KirProfileOperation::Splat),
        KirInstructionKind::VectorLoad { .. } => Some(KirProfileOperation::Load),
        KirInstructionKind::VectorStore { .. } => Some(KirProfileOperation::Store),
        KirInstructionKind::VectorInsert { .. } => Some(KirProfileOperation::Insert),
        KirInstructionKind::VectorExtract { .. } => Some(KirProfileOperation::Extract),
        KirInstructionKind::VectorBinary { op, .. } => Some(vector_operation(op)),
        _ => None,
    }
}

fn vector_operation(op: crate::KirVectorBinaryOp) -> KirProfileOperation {
    match op {
        crate::KirVectorBinaryOp::Add => KirProfileOperation::Add,
        crate::KirVectorBinaryOp::Subtract => KirProfileOperation::Subtract,
        crate::KirVectorBinaryOp::Multiply => KirProfileOperation::Multiply,
        crate::KirVectorBinaryOp::Divide => KirProfileOperation::Divide,
        crate::KirVectorBinaryOp::Remainder => KirProfileOperation::Remainder,
    }
}

fn instruction_region(instruction: &crate::KirInstruction) -> Option<crate::VectorRegionId> {
    match instruction.kind {
        KirInstructionKind::VectorSplat { region, .. }
        | KirInstructionKind::VectorLoad { region, .. }
        | KirInstructionKind::VectorStore { region, .. }
        | KirInstructionKind::VectorInsert { region, .. }
        | KirInstructionKind::VectorExtract { region, .. }
        | KirInstructionKind::VectorBinary { region, .. } => Some(region),
        _ => None,
    }
}

fn recompute_charge(plan: &SlpPlan) -> CandidateBudgetCharge {
    let emitted = u32::try_from(
        plan.operations
            .len()
            .saturating_add(plan.setup_instructions.len()),
    )
    .unwrap_or(u32::MAX);
    CandidateBudgetCharge::single(
        plan.pre_state.function,
        8_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(emitted),
        16_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(plan.growth.transformed_units)
            .saturating_add(emitted.saturating_mul(2)),
    )
}

fn pre_identity(
    state: &crate::KirVerifiedProgramState,
    function: crate::FunctionId,
) -> KirPreStateIdentity {
    KirPreStateIdentity {
        function,
        kir_digest: state.kir_digest(),
        profile_digest: state.module().profile.digest_hex(),
        evidence_generation: state.evidence_generation(),
        frozen_kir_units: state
            .module()
            .functions
            .iter()
            .find(|item| item.id == function)
            .map_or(0, kir_function_units),
    }
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}
