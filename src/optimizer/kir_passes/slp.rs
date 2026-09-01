use crate::{
    CandidateBudgetCharge, KirAlignmentClass, KirArithmeticSemantics, KirCostEstimate, KirCostKey,
    KirCostSemantics, KirInstruction, KirInstructionKind, KirOperationAvailability, KirPlace,
    KirPreStateIdentity, KirProfileOperation, KirResult, KirValueType, KirVectorBinaryOp,
    KirVectorMemoryAccess, KirVectorRegion, KirVerifiedProgramState, MirPrimitiveTypeName, MirType,
    SlpCandidate, SlpMemoryPlan, SlpPlan, SlpProofRecord, VectorPlanGrowth, kir_function_units,
};

use super::rewrite::replace_value_uses_batch;

#[derive(Debug, Clone)]
pub(crate) struct MaterializedSlp {
    pub trial: KirVerifiedProgramState,
    pub plan: SlpPlan,
    pub charge: CandidateBudgetCharge,
}

pub(crate) fn materialize_slp_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &SlpCandidate,
) -> Result<MaterializedSlp, String> {
    if !matches!(candidate.lanes, 2 | 4) {
        return Err("SLP lane count is outside the closed schema".to_string());
    }
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "SLP candidate function is missing".to_string())?
        .clone();
    let source_block = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.block)
        .ok_or_else(|| "SLP candidate block is missing".to_string())?;
    if candidate.memory.is_some() {
        return materialize_memory_slp(pre_state, candidate, &original);
    }
    let positions = candidate
        .scalar_instructions
        .iter()
        .map(|id| {
            source_block
                .instructions
                .iter()
                .position(|instruction| instruction.id == *id)
                .ok_or_else(|| "SLP scalar instruction is missing".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if positions.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("SLP scalar instruction order is false".to_string());
    }
    let insertion = positions[0];
    let mut trial = pre_state.clone();
    let (vector_region, new_vector_region) =
        slp_vector_region(&mut trial, &original, candidate.block)?;
    let vector_type = KirValueType::FixedVector {
        lane: candidate.lane_type,
        lanes: candidate.lanes,
    };
    let scalar_type = source_block
        .instructions
        .iter()
        .find(|instruction| instruction.id == candidate.scalar_instructions[0])
        .and_then(|instruction| instruction.results.first())
        .map(|result| result.type_node.clone())
        .ok_or_else(|| "SLP scalar result type is missing".to_string())?;
    let mut emitted = Vec::new();
    let mut operations = Vec::new();
    let left = build_operand_vector(
        &mut trial,
        &candidate.left,
        vector_region,
        vector_type.clone(),
        &mut emitted,
        &mut operations,
    )?;
    let right = build_operand_vector(
        &mut trial,
        &candidate.right,
        vector_region,
        vector_type.clone(),
        &mut emitted,
        &mut operations,
    )?;
    let vector_result = trial.fresh_value()?;
    let vector_binary = trial.fresh_instruction()?;
    emitted.push(KirInstruction {
        id: vector_binary,
        results: vec![KirResult {
            value: vector_result,
            type_node: vector_type,
        }],
        kind: KirInstructionKind::VectorBinary {
            op: vector_binary_op(candidate.operation)?,
            left,
            right,
            semantics: arithmetic_semantics(candidate.semantics)?,
            no_failure_proof: None,
            region: vector_region,
        },
        memory: None,
        effect: None,
    });
    operations.push(candidate.operation);
    let mut extracts = Vec::new();
    let mut replacements = Vec::new();
    for (lane, old) in candidate.results.iter().enumerate() {
        let value = trial.fresh_value()?;
        let id = trial.fresh_instruction()?;
        emitted.push(KirInstruction {
            id,
            results: vec![KirResult {
                value,
                type_node: scalar_type.clone(),
            }],
            kind: KirInstructionKind::VectorExtract {
                vector: vector_result,
                lane_index: u16::try_from(lane).expect("closed lane index fits u16"),
                region: vector_region,
            },
            memory: None,
            effect: None,
        });
        operations.push(KirProfileOperation::Extract);
        extracts.push(id);
        replacements.push((*old, value));
    }
    let vector_instructions = emitted
        .iter()
        .filter(|instruction| !extracts.contains(&instruction.id))
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>();

    let mut transformed = original.clone();
    if new_vector_region {
        transformed.vector_regions.push(KirVectorRegion {
            id: vector_region,
            blocks: vec![candidate.block],
        });
    }
    let block = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.block)
        .expect("candidate block established");
    let scalar_ids = candidate
        .scalar_instructions
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    block
        .instructions
        .retain(|instruction| !scalar_ids.contains(&instruction.id));
    block.instructions.splice(insertion..insertion, emitted);
    replace_value_uses_batch(&mut transformed, &replacements);

    let module_before = module_units(pre_state.module());
    let before_function = kir_function_units(&original);
    let after_function = kir_function_units(&transformed);
    let module_after = module_before
        .saturating_sub(before_function)
        .saturating_add(after_function);
    let scalar_cost = scalar_cost(pre_state, candidate)?;
    let transformed_cost = operations.iter().try_fold(0_u32, |total, operation| {
        vector_cost(pre_state, candidate, *operation).map(|cost| total.saturating_add(cost))
    })?;
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "SLP trial function disappeared".to_string())? = transformed;
    let plan = SlpPlan {
        pre_state: KirPreStateIdentity {
            function: candidate.function,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: before_function,
        },
        block: candidate.block,
        root: candidate.root,
        lanes: candidate.lanes,
        lane_type: candidate.lane_type,
        semantics: candidate.semantics,
        scalar_instructions: candidate.scalar_instructions.clone(),
        setup_instructions: Vec::new(),
        vector_instructions,
        extracts,
        operations,
        memory: None,
        cost: KirCostEstimate::new(scalar_cost, transformed_cost, 0, 0),
        growth: VectorPlanGrowth::new(
            u32::try_from(candidate.scalar_instructions.len()).unwrap_or(u32::MAX),
            u32::from(candidate.lanes)
                .saturating_mul(3)
                .saturating_add(1),
            module_before,
            module_after,
        ),
        proof: SlpProofRecord {
            block: candidate.block,
            source_order: candidate.scalar_instructions.clone(),
            identity_lanes: (0..candidate.lanes).collect(),
            barrier_free: true,
            exact_memory_footprint: None,
        },
    };
    let emitted_units = u32::try_from(plan.operations.len()).unwrap_or(u32::MAX);
    let charge = CandidateBudgetCharge::single(
        candidate.function,
        8_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(emitted_units),
        16_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(plan.growth.transformed_units)
            .saturating_add(emitted_units.saturating_mul(2)),
    );
    Ok(MaterializedSlp {
        trial,
        plan,
        charge,
    })
}

fn materialize_memory_slp(
    pre_state: &KirVerifiedProgramState,
    candidate: &SlpCandidate,
    original: &crate::KirFunction,
) -> Result<MaterializedSlp, String> {
    let memory = candidate
        .memory
        .as_ref()
        .ok_or_else(|| "SLP memory candidate is missing".to_string())?;
    let lanes = usize::from(candidate.lanes);
    if memory.left_loads.len() != lanes
        || memory.right_loads.len() != lanes
        || memory.stores.len() != lanes
        || candidate.scalar_instructions.len() != lanes
    {
        return Err("SLP memory candidate does not cover every lane".to_string());
    }
    let source_block = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.block)
        .ok_or_else(|| "SLP memory candidate block is missing".to_string())?;
    let find = |id| {
        source_block
            .instructions
            .iter()
            .find(|instruction| instruction.id == id)
            .ok_or_else(|| "SLP memory source instruction is missing".to_string())
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
    let binary_sources = candidate
        .scalar_instructions
        .iter()
        .map(|id| find(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let store_sources = memory
        .stores
        .iter()
        .map(|id| find(*id))
        .collect::<Result<Vec<_>, _>>()?;

    let (left_slice, start, lane_type_node) = load_components(left_sources[0])?;
    let (right_slice, right_start, right_type_node) = load_components(right_sources[0])?;
    let (output_slice, output_start, output_type_node, _) = store_components(store_sources[0])?;
    if lane_type_node != right_type_node
        || lane_type_node != output_type_node
        || left_slice == right_slice
        || left_slice == output_slice
        || right_slice == output_slice
    {
        return Err("SLP memory bases, starts, or lane types are inconsistent".to_string());
    }
    let lane_bytes = lane_bytes(candidate.lane_type)?;
    let footprint = lane_bytes.saturating_mul(u32::from(candidate.lanes));
    let start_number = constant_u32(source_block, start)?;
    if constant_u32(source_block, right_start)? != start_number
        || constant_u32(source_block, output_start)? != start_number
    {
        return Err("SLP memory starts are not equivalent constants".to_string());
    }
    let end_number = start_number
        .checked_add(u32::from(candidate.lanes))
        .ok_or_else(|| "SLP memory footprint overflows u32 indexing".to_string())?;
    for lane in 0..lanes {
        let expected_index = start_number
            .checked_add(u32::try_from(lane).map_err(|_| "SLP lane index exceeds u32")?)
            .ok_or_else(|| "SLP lane index overflows u32".to_string())?;
        let (lane_left_slice, lane_left_index, lane_left_type) =
            load_components(left_sources[lane])?;
        let (lane_right_slice, lane_right_index, lane_right_type) =
            load_components(right_sources[lane])?;
        let (lane_output_slice, lane_output_index, lane_output_type, stored) =
            store_components(store_sources[lane])?;
        let binary = binary_sources[lane];
        let KirInstructionKind::Binary {
            left,
            right,
            semantics,
            ..
        } = binary.kind
        else {
            return Err("SLP memory compute is not a scalar binary".to_string());
        };
        if lane_left_slice != left_slice
            || lane_right_slice != right_slice
            || lane_output_slice != output_slice
            || lane_left_type != lane_type_node
            || lane_right_type != lane_type_node
            || lane_output_type != lane_type_node
            || constant_u32(source_block, lane_left_index)? != expected_index
            || constant_u32(source_block, lane_right_index)? != expected_index
            || constant_u32(source_block, lane_output_index)? != expected_index
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
            || arithmetic_semantics(candidate.semantics)? != semantics
        {
            return Err("SLP memory lane is not an isomorphic contiguous operation".to_string());
        }
    }

    let mut source_order = memory
        .left_loads
        .iter()
        .chain(&memory.right_loads)
        .chain(&candidate.scalar_instructions)
        .chain(&memory.stores)
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    source_order.sort_by_key(|id| {
        source_block
            .instructions
            .iter()
            .position(|instruction| instruction.id == *id)
            .unwrap_or(usize::MAX)
    });
    if source_order.len() != lanes.saturating_mul(4) {
        return Err("SLP memory source mapping contains duplicate instructions".to_string());
    }
    let insertion = source_order
        .iter()
        .filter_map(|id| {
            source_block
                .instructions
                .iter()
                .position(|instruction| instruction.id == *id)
        })
        .min()
        .ok_or_else(|| "SLP memory insertion point is missing".to_string())?;

    let mut trial = pre_state.clone();
    let (vector_region, new_vector_region) =
        slp_vector_region(&mut trial, original, candidate.block)?;
    let vector_type = KirValueType::FixedVector {
        lane: candidate.lane_type,
        lanes: candidate.lanes,
    };
    let end_value = trial.fresh_value()?;
    let end_instruction = trial.fresh_instruction()?;
    let mut emitted = vec![KirInstruction {
        id: end_instruction,
        results: vec![KirResult {
            value: end_value,
            type_node: KirValueType::scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
        }],
        kind: KirInstructionKind::ConstInt {
            value: end_number.to_string(),
        },
        memory: None,
        effect: None,
    }];
    let access = |slice, start| KirVectorMemoryAccess {
        slice,
        start,
        end: end_value,
        lane: candidate.lane_type,
        lanes: candidate.lanes,
        byte_footprint: footprint,
        known_alignment: u16::try_from(lane_bytes).unwrap_or(u16::MAX),
        required_alignment: u16::try_from(lane_bytes).unwrap_or(u16::MAX),
    };
    let left_value = trial.fresh_value()?;
    let left_vector_load = trial.fresh_instruction()?;
    emitted.push(KirInstruction {
        id: left_vector_load,
        results: vec![KirResult {
            value: left_value,
            type_node: vector_type.clone(),
        }],
        kind: KirInstructionKind::VectorLoad {
            access: access(left_slice, start),
            region: vector_region,
        },
        memory: left_sources[0].memory.clone(),
        effect: left_sources[0].effect.clone(),
    });
    let right_value = trial.fresh_value()?;
    let right_vector_load = trial.fresh_instruction()?;
    emitted.push(KirInstruction {
        id: right_vector_load,
        results: vec![KirResult {
            value: right_value,
            type_node: vector_type.clone(),
        }],
        kind: KirInstructionKind::VectorLoad {
            access: access(right_slice, start),
            region: vector_region,
        },
        memory: right_sources[0].memory.clone(),
        effect: right_sources[0].effect.clone(),
    });
    let vector_value = trial.fresh_value()?;
    let vector_binary = trial.fresh_instruction()?;
    emitted.push(KirInstruction {
        id: vector_binary,
        results: vec![KirResult {
            value: vector_value,
            type_node: vector_type,
        }],
        kind: KirInstructionKind::VectorBinary {
            op: vector_binary_op(candidate.operation)?,
            left: left_value,
            right: right_value,
            semantics: arithmetic_semantics(candidate.semantics)?,
            no_failure_proof: None,
            region: vector_region,
        },
        memory: None,
        effect: None,
    });
    let vector_store = trial.fresh_instruction()?;
    let mut store_memory = store_sources[0]
        .memory
        .clone()
        .ok_or_else(|| "SLP scalar store MemorySSA input is missing".to_string())?;
    store_memory.output = store_sources
        .last()
        .and_then(|instruction| instruction.memory.as_ref())
        .and_then(|memory| memory.output);
    if store_memory.output.is_none() {
        return Err("SLP scalar store MemorySSA output is missing".to_string());
    }
    emitted.push(KirInstruction {
        id: vector_store,
        results: Vec::new(),
        kind: KirInstructionKind::VectorStore {
            access: access(output_slice, start),
            value: vector_value,
            region: vector_region,
        },
        memory: Some(store_memory),
        effect: store_sources
            .last()
            .and_then(|instruction| instruction.effect.clone()),
    });

    let mut transformed = original.clone();
    if new_vector_region {
        transformed.vector_regions.push(KirVectorRegion {
            id: vector_region,
            blocks: vec![candidate.block],
        });
    }
    let block = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.block)
        .expect("SLP memory candidate block established");
    let removed = source_order
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    block
        .instructions
        .retain(|instruction| !removed.contains(&instruction.id));
    block.instructions.splice(insertion..insertion, emitted);

    let module_before = module_units(pre_state.module());
    let before_function = kir_function_units(original);
    let after_function = kir_function_units(&transformed);
    let module_after = module_before
        .saturating_sub(before_function)
        .saturating_add(after_function);
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "SLP memory trial function disappeared".to_string())? = transformed;
    let operations = vec![
        KirProfileOperation::Load,
        KirProfileOperation::Load,
        candidate.operation,
        KirProfileOperation::Store,
    ];
    let scalar_cost = memory_scalar_cost(pre_state, candidate, lane_bytes)?;
    let transformed_cost = operations.iter().try_fold(0_u32, |total, operation| {
        memory_operation_cost(
            pre_state,
            candidate,
            *operation,
            lane_bytes,
            candidate.lanes,
        )
        .map(|cost| total.saturating_add(cost))
    })?;
    let plan = SlpPlan {
        pre_state: KirPreStateIdentity {
            function: candidate.function,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: before_function,
        },
        block: candidate.block,
        root: candidate.root,
        lanes: candidate.lanes,
        lane_type: candidate.lane_type,
        semantics: candidate.semantics,
        scalar_instructions: candidate.scalar_instructions.clone(),
        setup_instructions: vec![end_instruction],
        vector_instructions: vec![
            left_vector_load,
            right_vector_load,
            vector_binary,
            vector_store,
        ],
        extracts: Vec::new(),
        operations,
        memory: Some(SlpMemoryPlan {
            left_loads: memory.left_loads.clone(),
            right_loads: memory.right_loads.clone(),
            stores: memory.stores.clone(),
            vector_loads: vec![left_vector_load, right_vector_load],
            vector_store,
        }),
        cost: KirCostEstimate::new(scalar_cost, transformed_cost, 0, 0),
        growth: VectorPlanGrowth::new(
            u32::try_from(source_order.len()).unwrap_or(u32::MAX),
            5,
            module_before,
            module_after,
        ),
        proof: SlpProofRecord {
            block: candidate.block,
            source_order,
            identity_lanes: (0..candidate.lanes).collect(),
            barrier_free: true,
            exact_memory_footprint: Some(footprint),
        },
    };
    let emitted_units = u32::try_from(
        plan.operations
            .len()
            .saturating_add(plan.setup_instructions.len()),
    )
    .unwrap_or(u32::MAX);
    let charge = CandidateBudgetCharge::single(
        candidate.function,
        8_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(emitted_units),
        16_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(plan.growth.transformed_units)
            .saturating_add(emitted_units.saturating_mul(2)),
    );
    Ok(MaterializedSlp {
        trial,
        plan,
        charge,
    })
}

fn load_components(
    instruction: &KirInstruction,
) -> Result<(crate::ValueId, crate::ValueId, MirType), String> {
    let KirInstructionKind::Load { place } = &instruction.kind else {
        return Err("SLP memory source is not a load".to_string());
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        ..
    } = place.as_ref()
    else {
        return Err("SLP memory load is not a slice index".to_string());
    };
    Ok((*slice, *index, type_node.clone()))
}

fn store_components(
    instruction: &KirInstruction,
) -> Result<(crate::ValueId, crate::ValueId, MirType, crate::ValueId), String> {
    let KirInstructionKind::Store { place, value } = &instruction.kind else {
        return Err("SLP memory source is not a store".to_string());
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        ..
    } = place.as_ref()
    else {
        return Err("SLP memory store is not a slice index".to_string());
    };
    Ok((*slice, *index, type_node.clone(), *value))
}

fn constant_u32(block: &crate::KirBlock, value: crate::ValueId) -> Result<u32, String> {
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
        .ok_or_else(|| "SLP memory index is not a u32 constant".to_string())
}

fn lane_bytes(lane: crate::KirLaneType) -> Result<u32, String> {
    match lane {
        crate::KirLaneType::I32 | crate::KirLaneType::U32 => Ok(4),
        crate::KirLaneType::I64 | crate::KirLaneType::U64 | crate::KirLaneType::F64 => Ok(8),
    }
}

fn slp_vector_region(
    trial: &mut KirVerifiedProgramState,
    function: &crate::KirFunction,
    block: crate::BlockId,
) -> Result<(crate::VectorRegionId, bool), String> {
    let existing = function
        .vector_regions
        .iter()
        .filter(|region| region.blocks.contains(&block))
        .map(|region| region.id)
        .collect::<Vec<_>>();
    match existing.as_slice() {
        [] => Ok((trial.fresh_vector_region()?, true)),
        [region] => Ok((*region, false)),
        _ => Err("SLP block already belongs to multiple vector regions".to_string()),
    }
}

fn memory_scalar_cost(
    state: &KirVerifiedProgramState,
    candidate: &SlpCandidate,
    lane_bytes: u32,
) -> Result<u32, String> {
    [
        KirProfileOperation::Load,
        KirProfileOperation::Load,
        candidate.operation,
        KirProfileOperation::Store,
    ]
    .into_iter()
    .try_fold(0_u32, |total, operation| {
        memory_operation_cost(state, candidate, operation, lane_bytes, 1)
            .map(|cost| total.saturating_add(cost.saturating_mul(u32::from(candidate.lanes))))
    })
}

fn memory_operation_cost(
    state: &KirVerifiedProgramState,
    candidate: &SlpCandidate,
    operation: KirProfileOperation,
    lane_bytes: u32,
    lanes: u16,
) -> Result<u32, String> {
    let semantics = if operation == candidate.operation {
        candidate.semantics
    } else {
        KirCostSemantics::NotApplicable
    };
    let alignment = if matches!(
        operation,
        KirProfileOperation::Load | KirProfileOperation::Store
    ) {
        KirAlignmentClass::Bytes(
            u16::try_from(lane_bytes).map_err(|_| "SLP alignment exceeds u16")?,
        )
    } else {
        KirAlignmentClass::NotApplicable
    };
    legal_cost(
        state,
        &KirCostKey {
            operation,
            lane: candidate.lane_type,
            lanes: u8::try_from(lanes).map_err(|_| "SLP lanes exceed u8")?,
            semantics,
            alignment,
        },
    )
}

fn build_operand_vector(
    state: &mut KirVerifiedProgramState,
    values: &[crate::ValueId],
    region: crate::VectorRegionId,
    vector_type: KirValueType,
    emitted: &mut Vec<KirInstruction>,
    operations: &mut Vec<KirProfileOperation>,
) -> Result<crate::ValueId, String> {
    let Some(first) = values.first().copied() else {
        return Err("SLP operand pack is empty".to_string());
    };
    let mut vector = state.fresh_value()?;
    emitted.push(KirInstruction {
        id: state.fresh_instruction()?,
        results: vec![KirResult {
            value: vector,
            type_node: vector_type.clone(),
        }],
        kind: KirInstructionKind::VectorSplat {
            scalar: first,
            region,
        },
        memory: None,
        effect: None,
    });
    operations.push(KirProfileOperation::Splat);
    for (lane, scalar) in values.iter().enumerate().skip(1) {
        let next = state.fresh_value()?;
        emitted.push(KirInstruction {
            id: state.fresh_instruction()?,
            results: vec![KirResult {
                value: next,
                type_node: vector_type.clone(),
            }],
            kind: KirInstructionKind::VectorInsert {
                vector,
                scalar: *scalar,
                lane_index: u16::try_from(lane).expect("closed lane index fits u16"),
                region,
            },
            memory: None,
            effect: None,
        });
        operations.push(KirProfileOperation::Insert);
        vector = next;
    }
    Ok(vector)
}

fn scalar_cost(state: &KirVerifiedProgramState, candidate: &SlpCandidate) -> Result<u32, String> {
    let key = KirCostKey {
        operation: candidate.operation,
        lane: candidate.lane_type,
        lanes: 1,
        semantics: candidate.semantics,
        alignment: KirAlignmentClass::NotApplicable,
    };
    let cost = legal_cost(state, &key)?;
    Ok(cost.saturating_mul(u32::from(candidate.lanes)))
}

fn vector_cost(
    state: &KirVerifiedProgramState,
    candidate: &SlpCandidate,
    operation: KirProfileOperation,
) -> Result<u32, String> {
    let semantics = if operation == candidate.operation {
        candidate.semantics
    } else {
        KirCostSemantics::NotApplicable
    };
    legal_cost(
        state,
        &KirCostKey {
            operation,
            lane: candidate.lane_type,
            lanes: u8::try_from(candidate.lanes).map_err(|_| "SLP lanes exceed u8")?,
            semantics,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )
}

fn legal_cost(state: &KirVerifiedProgramState, key: &KirCostKey) -> Result<u32, String> {
    match state.module().profile.operation_availability(key) {
        Some(KirOperationAvailability::Legal(cost)) => Ok(cost.cost),
        Some(KirOperationAvailability::Unavailable) | None => {
            Err("SLP target operation is unavailable".to_string())
        }
    }
}

fn vector_binary_op(operation: KirProfileOperation) -> Result<KirVectorBinaryOp, String> {
    match operation {
        KirProfileOperation::Add => Ok(KirVectorBinaryOp::Add),
        KirProfileOperation::Subtract => Ok(KirVectorBinaryOp::Subtract),
        KirProfileOperation::Multiply => Ok(KirVectorBinaryOp::Multiply),
        KirProfileOperation::Divide => Ok(KirVectorBinaryOp::Divide),
        _ => Err("SLP scalar operation has no closed vector mapping".to_string()),
    }
}

fn arithmetic_semantics(semantics: KirCostSemantics) -> Result<KirArithmeticSemantics, String> {
    match semantics {
        KirCostSemantics::Modular => Ok(KirArithmeticSemantics::Modular),
        KirCostSemantics::StrictFloat => Ok(KirArithmeticSemantics::StrictFloat),
        KirCostSemantics::NotApplicable | KirCostSemantics::Checked => {
            Err("SLP arithmetic semantics are unsupported".to_string())
        }
    }
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}
