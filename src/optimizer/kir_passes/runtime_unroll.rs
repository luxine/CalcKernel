use std::collections::BTreeMap;

use crate::{
    CandidateBudgetCharge, KirArithmeticSemantics, KirBlock, KirBlockParam, KirEdge,
    KirInstruction, KirInstructionKind, KirMemoryBlockParam, KirPreStateIdentity, KirResult,
    KirTerminator, KirValueType, KirVerifiedProgramState, MirBinaryOp, MirCompareOp,
    MirPrimitiveTypeName, MirType, RuntimeScalarUnrollCandidate, RuntimeScalarUnrollPlan,
    analyze_canonical_loops, discover_wasm_runtime_scalar_unroll_candidates, kir_function_units,
};

use super::rewrite::remap_instruction_values;
use crate::optimizer::runtime_unroll_check::PreparedRuntimeScalarUnroll;

type Values = BTreeMap<crate::ValueId, crate::ValueId>;
type Memories = BTreeMap<crate::MemoryVersionId, crate::MemoryVersionId>;

pub fn prepare_wasm_runtime_scalar_unroll_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &RuntimeScalarUnrollCandidate,
) -> Result<PreparedRuntimeScalarUnroll, String> {
    validate_candidate(pre_state, candidate)?;
    let mut trial = pre_state.clone();
    let source = source_function(&trial, candidate.function)?.clone();
    let shape = source_shape(&source, candidate)?;
    let mut transformed = source.clone();

    let preheader_id = shape.preheader.id;
    let fast_entry_id = trial.fresh_block()?;
    let main_header_id = trial.fresh_block()?;
    let main_body_id = trial.fresh_block()?;

    let source_incoming = shape.incoming.clone();
    let source_then = shape.then_edge.clone();
    let header_induction_index = shape
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.induction)
        .ok_or_else(|| "runtime UF4 source induction is not a header parameter".to_string())?;
    let body_induction_index = source_then
        .args
        .iter()
        .position(|value| *value == candidate.induction)
        .ok_or_else(|| "runtime UF4 source induction is not passed to the body".to_string())?;
    let body_induction = shape.body.params[body_induction_index].value;
    let update_index = shape
        .body
        .instructions
        .iter()
        .position(|instruction| instruction.id == candidate.induction_update)
        .ok_or_else(|| "runtime UF4 source induction update is missing".to_string())?;
    if update_index + 1 != shape.body.instructions.len() {
        return Err("runtime UF4 source induction update is not the final body instruction".into());
    }
    let update = &shape.body.instructions[update_index];
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &update.kind
    else {
        return Err("runtime UF4 source induction update is not modular addition".into());
    };
    if *left != body_induction || constant_u32(&source, *right) != Some(1) {
        return Err("runtime UF4 source induction update is not an exact unit step".into());
    }
    let body_step_constant = shape.body.instructions.iter().find(|instruction| {
        instruction
            .results
            .iter()
            .any(|result| result.value == *right)
    });
    if body_step_constant.is_some_and(|instruction| {
        !matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "1")
            || instruction.results.len() != 1
            || instruction.memory.is_some()
            || instruction.effect.is_some()
    }) {
        return Err("runtime UF4 source step value is not a pure one literal".into());
    }
    let source_step_constant_id = body_step_constant.map(|instruction| instruction.id);
    let iteration_instructions = shape
        .body
        .instructions
        .iter()
        .filter(|instruction| {
            instruction.id != candidate.induction_update
                && Some(instruction.id) != source_step_constant_id
        })
        .collect::<Vec<_>>();

    let mut preheader = shape.preheader.clone();
    let bound_slice_at_entry = value_for_header_entry(&shape, candidate.bound_slice)?;
    let dispatch_bound = append_result_instruction(
        &mut trial,
        &mut preheader,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
        KirInstructionKind::SliceLen {
            slice: bound_slice_at_entry,
        },
    )?;
    let minimum_constant = append_u32_constant(&mut trial, &mut preheader, candidate.minimum_trip)?;
    let dispatch_guard = append_result_instruction(
        &mut trial,
        &mut preheader,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::Bool)),
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: dispatch_bound,
            right: minimum_constant,
        },
    )?;

    let mut fast_entry = block_shell(
        fast_entry_id,
        format!("runtime_scalar_unroll_entry_{}", candidate.loop_id.index()),
    );
    let mut fast_values = Values::new();
    for parameter in &shape.header.params {
        let value = trial.fresh_value()?;
        fast_values.insert(parameter.value, value);
        fast_entry.params.push(KirBlockParam {
            value,
            slot: format!("runtime_entry_{}", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    let fast_bound_param = trial.fresh_value()?;
    fast_entry.params.push(KirBlockParam {
        value: fast_bound_param,
        slot: "runtime_entry_bound".to_string(),
        type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
    });
    let fast_memories =
        clone_memory_params(&mut trial, &shape.header.memory_params, &mut fast_entry)?;
    let remainder_constant =
        append_u32_constant(&mut trial, &mut fast_entry, u32::from(candidate.factor))?;
    let limit_remainder = append_result_instruction(
        &mut trial,
        &mut fast_entry,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
        KirInstructionKind::Binary {
            op: MirBinaryOp::Mod,
            left: fast_bound_param,
            right: remainder_constant,
            semantics: KirArithmeticSemantics::Modular,
        },
    )?;
    let limit_value = append_result_instruction(
        &mut trial,
        &mut fast_entry,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: fast_bound_param,
            right: limit_remainder,
            semantics: KirArithmeticSemantics::Modular,
        },
    )?;

    let mut main_header = block_shell(
        main_header_id,
        format!("runtime_scalar_unroll_header_{}", candidate.loop_id.index()),
    );
    let mut main_header_values = Values::new();
    for parameter in &shape.header.params {
        let value = trial.fresh_value()?;
        main_header_values.insert(parameter.value, value);
        main_header.params.push(KirBlockParam {
            value,
            slot: format!("runtime_main_{}", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    let main_limit_param = trial.fresh_value()?;
    main_header.params.push(KirBlockParam {
        value: main_limit_param,
        slot: "runtime_main_limit".to_string(),
        type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
    });
    let main_header_memories =
        clone_memory_params(&mut trial, &shape.header.memory_params, &mut main_header)?;
    let main_condition = append_result_instruction(
        &mut trial,
        &mut main_header,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::Bool)),
        KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: *main_header_values
                .get(&candidate.induction)
                .ok_or_else(|| "runtime UF4 induction parameter is absent".to_string())?,
            right: main_limit_param,
        },
    )?;

    let mut main_body = block_shell(
        main_body_id,
        format!("runtime_scalar_unroll_body_{}", candidate.loop_id.index()),
    );
    let mut first_lane_values = Values::new();
    for parameter in &shape.body.params {
        let value = trial.fresh_value()?;
        first_lane_values.insert(parameter.value, value);
        main_body.params.push(KirBlockParam {
            value,
            slot: format!("runtime_body_{}", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    let first_lane_memories =
        clone_memory_params(&mut trial, &shape.body.memory_params, &mut main_body)?;
    let group_induction_base = main_body.params[body_induction_index].value;
    let mut lane_induction_values = vec![group_induction_base];
    let mut lane_index_instruction_ids = Vec::with_capacity(3);
    let mut induction_aux_instruction_ids = Vec::with_capacity(8);
    for offset in 1..u32::from(candidate.factor) {
        let constant = append_u32_constant(&mut trial, &mut main_body, offset)?;
        induction_aux_instruction_ids.push(
            main_body
                .instructions
                .last()
                .ok_or_else(|| "runtime UF4 lane offset constant disappeared".to_string())?
                .id,
        );
        let lane_index = append_result_instruction(
            &mut trial,
            &mut main_body,
            KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: group_induction_base,
                right: constant,
                semantics: KirArithmeticSemantics::Modular,
            },
        )?;
        let instruction = main_body
            .instructions
            .last()
            .ok_or_else(|| "runtime UF4 lane offset addition disappeared".to_string())?
            .id;
        lane_induction_values.push(lane_index);
        lane_index_instruction_ids.push(instruction);
        induction_aux_instruction_ids.push(instruction);
    }
    let mut effect_order = next_effect_order(&source)?;
    let mut lane_instruction_ids = Vec::with_capacity(usize::from(candidate.factor));
    let mut lane_result_values = Vec::with_capacity(usize::from(candidate.factor));
    let mut current_header_values = main_header_values.clone();
    let mut current_header_memories = main_header_memories
        .iter()
        .map(|(source_version, target_version)| (*source_version, *target_version))
        .collect::<Memories>();

    for lane in 0..candidate.factor {
        let mut lane_values = if lane == 0 {
            first_lane_values.clone()
        } else {
            bind_body_values(&shape, &current_header_values)?
        };
        lane_values.insert(body_induction, lane_induction_values[usize::from(lane)]);
        let mut lane_memories = if lane == 0 {
            first_lane_memories.clone()
        } else {
            bind_body_memories(&shape, &current_header_memories)?
        };
        let mut lane_ids = Vec::with_capacity(iteration_instructions.len());
        let mut lane_results = Vec::new();
        for source_instruction in &iteration_instructions {
            let mut cloned = (*source_instruction).clone();
            cloned.id = trial.fresh_instruction()?;
            for result in &mut cloned.results {
                let target = trial.fresh_value()?;
                lane_values.insert(result.value, target);
                result.value = target;
                lane_results.push(target);
            }
            remap_instruction_values(&mut cloned, &lane_values);
            remap_instruction_memory(&mut trial, &mut cloned, &mut lane_memories)?;
            if let Some(effect) = &mut cloned.effect {
                effect.order = take_effect_order(&mut effect_order)?;
            }
            lane_ids.push(cloned.id);
            main_body.instructions.push(cloned);
        }
        current_header_values = next_header_values_skipping_induction(
            &shape,
            &lane_values,
            header_induction_index,
            candidate.induction,
            group_induction_base,
        )?;
        current_header_memories = next_header_memories(&shape, &lane_memories)?;
        lane_instruction_ids.push(lane_ids);
        lane_result_values.push(lane_results);
    }

    let group_step_constant =
        append_u32_constant(&mut trial, &mut main_body, u32::from(candidate.factor))?;
    induction_aux_instruction_ids.push(
        main_body
            .instructions
            .last()
            .ok_or_else(|| "runtime UF4 group step constant disappeared".to_string())?
            .id,
    );
    let group_induction_value = append_result_instruction(
        &mut trial,
        &mut main_body,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: group_induction_base,
            right: group_step_constant,
            semantics: KirArithmeticSemantics::Modular,
        },
    )?;
    let group_induction_instruction = main_body
        .instructions
        .last()
        .ok_or_else(|| "runtime UF4 group step addition disappeared".to_string())?
        .id;
    induction_aux_instruction_ids.push(group_induction_instruction);
    current_header_values.insert(candidate.induction, group_induction_value);

    let body_entry_args = source_then
        .args
        .iter()
        .map(|value| {
            main_header_values.get(value).copied().ok_or_else(|| {
                "runtime UF4 body input is not a source header parameter".to_string()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let body_entry_memories = source_then
        .memory_args
        .iter()
        .map(|version| {
            main_header_memories.get(version).copied().ok_or_else(|| {
                "runtime UF4 body memory input is not a source header parameter".to_string()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut dispatch_fast_values = source_incoming.args.clone();
    dispatch_fast_values.push(dispatch_bound);
    set_block(&mut transformed, preheader_id, preheader)?;
    block_mut(&mut transformed, preheader_id)?.terminator = KirTerminator::Branch {
        condition: dispatch_guard,
        then_edge: KirEdge {
            target: fast_entry_id,
            args: dispatch_fast_values,
            memory_args: source_incoming.memory_args.clone(),
        },
        else_edge: KirEdge {
            target: candidate.header,
            args: source_incoming.args.clone(),
            memory_args: source_incoming.memory_args.clone(),
        },
    };

    fast_entry.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: main_header_id,
            args: shape
                .header
                .params
                .iter()
                .map(|parameter| {
                    fast_values
                        .get(&parameter.value)
                        .copied()
                        .ok_or_else(|| "runtime UF4 fast-entry value is missing".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .chain([limit_value])
                .collect(),
            memory_args: shape
                .header
                .memory_params
                .iter()
                .map(|parameter| {
                    fast_memories
                        .get(&parameter.version)
                        .copied()
                        .ok_or_else(|| "runtime UF4 fast-entry memory is missing".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
        },
    };
    main_header.terminator = KirTerminator::Branch {
        condition: main_condition,
        then_edge: KirEdge {
            target: main_body_id,
            args: body_entry_args,
            memory_args: body_entry_memories,
        },
        else_edge: KirEdge {
            target: candidate.header,
            args: shape
                .header
                .params
                .iter()
                .map(|parameter| main_header_values[&parameter.value])
                .collect(),
            memory_args: shape
                .header
                .memory_params
                .iter()
                .map(|parameter| main_header_memories[&parameter.version])
                .collect(),
        },
    };
    let final_header_args = shape
        .header
        .params
        .iter()
        .map(|parameter| {
            current_header_values
                .get(&parameter.value)
                .copied()
                .ok_or_else(|| "runtime UF4 final header value is missing".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let final_memory_args = shape
        .header
        .memory_params
        .iter()
        .map(|parameter| {
            current_header_memories
                .get(&parameter.version)
                .copied()
                .ok_or_else(|| "runtime UF4 final MemorySSA value is missing".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    main_body.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: main_header_id,
            args: final_header_args
                .into_iter()
                .chain([main_limit_param])
                .collect(),
            memory_args: final_memory_args,
        },
    };

    transformed.blocks.push(fast_entry);
    transformed.blocks.push(main_header);
    transformed.blocks.push(main_body);

    let before_units = kir_function_units(&source);
    let after_units = kir_function_units(&transformed);
    let module_before_units = pre_state
        .module()
        .functions
        .iter()
        .map(kir_function_units)
        .fold(0_u32, u32::saturating_add);
    let module_after_units = module_before_units
        .saturating_sub(before_units)
        .saturating_add(after_units);
    let dispatch_guard_instruction = instruction_result_id(&transformed, dispatch_guard)?;
    let limit_remainder_instruction = instruction_result_id(&transformed, limit_remainder)?;
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "runtime scalar UF4 transformed function disappeared".to_string())? =
        transformed.clone();
    let plan = RuntimeScalarUnrollPlan {
        pre_state: KirPreStateIdentity {
            function: source.id,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: before_units,
        },
        candidate: candidate.clone(),
        fast_entry: fast_entry_id,
        main_header: main_header_id,
        main_body: main_body_id,
        dispatch_bound,
        dispatch_guard,
        dispatch_guard_instruction,
        limit_remainder,
        limit_value,
        limit_remainder_instruction,
        limit_instruction: instruction_result_id(&transformed, limit_value)?,
        main_condition,
        main_condition_instruction: instruction_result_id(&transformed, main_condition)?,
        lane_induction_values,
        lane_index_instruction_ids,
        group_induction_value,
        group_induction_instruction,
        induction_aux_instruction_ids,
        lane_instruction_ids,
        lane_result_values,
        kind: candidate.kind,
        factor: candidate.factor,
        minimum_trip: candidate.minimum_trip,
        cost: candidate.predicted_cost,
        growth: crate::VectorPlanGrowth::new(
            before_units,
            after_units,
            module_before_units,
            module_after_units,
        ),
    };
    let charge = runtime_unroll_charge(&plan);
    Ok(PreparedRuntimeScalarUnroll {
        trial,
        plan,
        charge,
    })
}

pub(crate) fn runtime_unroll_charge(plan: &RuntimeScalarUnrollPlan) -> CandidateBudgetCharge {
    let instruction_map_count = plan
        .lane_instruction_ids
        .iter()
        .map(|lane| u32::try_from(lane.len()).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add)
        .saturating_add(
            u32::try_from(plan.induction_aux_instruction_ids.len()).unwrap_or(u32::MAX),
        );
    CandidateBudgetCharge::single(
        plan.candidate.function,
        plan.growth
            .transformed_units
            .saturating_sub(plan.growth.original_units)
            .saturating_add(instruction_map_count)
            .saturating_add(16),
        plan.growth
            .module_before_units
            .saturating_add(plan.growth.module_after_units)
            .saturating_add(instruction_map_count.saturating_mul(2))
            .saturating_add(32),
    )
}

fn validate_candidate(
    state: &KirVerifiedProgramState,
    candidate: &RuntimeScalarUnrollCandidate,
) -> Result<(), String> {
    let function = source_function(state, candidate.function)?;
    let loops = analyze_canonical_loops(function);
    let discovered = discover_wasm_runtime_scalar_unroll_candidates(state, &loops.loops);
    if !discovered.candidates.iter().any(|item| item == candidate) {
        return Err("runtime scalar UF4 candidate is stale or not source-proven".to_string());
    }
    Ok(())
}

struct Shape<'a> {
    preheader: &'a KirBlock,
    header: &'a KirBlock,
    body: &'a KirBlock,
    incoming: &'a KirEdge,
    then_edge: &'a KirEdge,
    backedge: &'a KirEdge,
}

fn source_shape<'a>(
    function: &'a crate::KirFunction,
    candidate: &RuntimeScalarUnrollCandidate,
) -> Result<Shape<'a>, String> {
    let preheader = block(function, candidate.preheader)?;
    let header = block(function, candidate.header)?;
    let body = block(function, candidate.body)?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return Err("runtime UF4 preheader no longer jumps to the source loop".to_string());
    };
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return Err("runtime UF4 source loop header is no longer conditional".to_string());
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err("runtime UF4 source body no longer jumps to its header".to_string());
    };
    Ok(Shape {
        preheader,
        header,
        body,
        incoming,
        then_edge,
        backedge,
    })
}

fn bind_body_values(shape: &Shape<'_>, header_values: &Values) -> Result<Values, String> {
    shape
        .body
        .params
        .iter()
        .zip(&shape.then_edge.args)
        .map(|(parameter, argument)| {
            header_values
                .get(argument)
                .copied()
                .map(|value| (parameter.value, value))
                .ok_or_else(|| {
                    "runtime UF4 source body value is not a loop-header parameter".to_string()
                })
        })
        .collect()
}

fn bind_body_memories(shape: &Shape<'_>, header_memories: &Memories) -> Result<Memories, String> {
    shape
        .body
        .memory_params
        .iter()
        .zip(&shape.then_edge.memory_args)
        .map(|(parameter, argument)| {
            header_memories
                .get(argument)
                .copied()
                .map(|version| (parameter.version, version))
                .ok_or_else(|| {
                    "runtime UF4 source MemorySSA input is not a loop-header parameter".to_string()
                })
        })
        .collect()
}

fn next_header_values_skipping_induction(
    shape: &Shape<'_>,
    values: &Values,
    induction_index: usize,
    induction: crate::ValueId,
    group_base: crate::ValueId,
) -> Result<Values, String> {
    shape
        .header
        .params
        .iter()
        .zip(&shape.backedge.args)
        .enumerate()
        .map(|(index, (parameter, argument))| {
            if index == induction_index {
                if parameter.value != induction {
                    return Err("runtime UF4 induction parameter index changed".into());
                }
                return Ok((parameter.value, group_base));
            }
            values
                .get(argument)
                .copied()
                .map(|value| (parameter.value, value))
                .ok_or_else(|| "runtime UF4 source backedge value is not mapped".to_string())
        })
        .collect()
}

fn next_header_memories(shape: &Shape<'_>, memories: &Memories) -> Result<Memories, String> {
    shape
        .header
        .memory_params
        .iter()
        .zip(&shape.backedge.memory_args)
        .map(|(parameter, argument)| {
            memories
                .get(argument)
                .copied()
                .map(|version| (parameter.version, version))
                .ok_or_else(|| "runtime UF4 source MemorySSA backedge is not mapped".to_string())
        })
        .collect()
}

fn remap_instruction_memory(
    state: &mut KirVerifiedProgramState,
    instruction: &mut KirInstruction,
    memories: &mut Memories,
) -> Result<(), String> {
    let Some(access) = &mut instruction.memory else {
        return Ok(());
    };
    access.input = memories
        .get(&access.input)
        .copied()
        .ok_or_else(|| "runtime UF4 memory input is not mapped".to_string())?;
    if let Some(source_output) = access.output {
        let output = state.fresh_memory_version()?;
        memories.insert(source_output, output);
        access.output = Some(output);
    }
    Ok(())
}

fn value_for_header_entry(
    shape: &Shape<'_>,
    header_value: crate::ValueId,
) -> Result<crate::ValueId, String> {
    let index = shape
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == header_value)
        .ok_or_else(|| "runtime UF4 bound slice is not a loop-header parameter".to_string())?;
    shape
        .incoming
        .args
        .get(index)
        .copied()
        .ok_or_else(|| "runtime UF4 entry bound slice is missing".to_string())
}

fn append_u32_constant(
    state: &mut KirVerifiedProgramState,
    block: &mut KirBlock,
    value: u32,
) -> Result<crate::ValueId, String> {
    append_result_instruction(
        state,
        block,
        KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
        KirInstructionKind::ConstInt {
            value: value.to_string(),
        },
    )
}

fn append_result_instruction(
    state: &mut KirVerifiedProgramState,
    block: &mut KirBlock,
    type_node: KirValueType,
    kind: KirInstructionKind,
) -> Result<crate::ValueId, String> {
    let value = state.fresh_value()?;
    let id = state.fresh_instruction()?;
    block.instructions.push(KirInstruction {
        id,
        results: vec![KirResult { value, type_node }],
        kind,
        memory: None,
        effect: None,
    });
    Ok(value)
}

fn clone_memory_params(
    state: &mut KirVerifiedProgramState,
    source: &[crate::KirMemoryBlockParam],
    target: &mut KirBlock,
) -> Result<Memories, String> {
    let mut mapping = Memories::new();
    for parameter in source {
        let version = state.fresh_memory_version()?;
        mapping.insert(parameter.version, version);
        target.memory_params.push(KirMemoryBlockParam {
            version,
            region: parameter.region,
        });
    }
    Ok(mapping)
}

fn block_shell(id: crate::BlockId, label: String) -> KirBlock {
    KirBlock {
        id,
        label,
        params: Vec::new(),
        memory_params: Vec::new(),
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: None,
            memory: Vec::new(),
            effect_order: 0,
        },
    }
}

fn next_effect_order(function: &crate::KirFunction) -> Result<u32, String> {
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
                    _ => None,
                })
        })
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "runtime UF4 ordered effect identity is exhausted".to_string())
}

fn take_effect_order(next: &mut u32) -> Result<u32, String> {
    let order = *next;
    *next = next
        .checked_add(1)
        .ok_or_else(|| "runtime UF4 ordered effect identity is exhausted".to_string())?;
    Ok(order)
}

fn constant_u32(function: &crate::KirFunction, value: crate::ValueId) -> Option<u32> {
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
        .and_then(|instruction| match &instruction.kind {
            KirInstructionKind::ConstInt { value } => value.parse().ok(),
            _ => None,
        })
}

fn source_function(
    state: &KirVerifiedProgramState,
    function: crate::FunctionId,
) -> Result<&crate::KirFunction, String> {
    state
        .module()
        .functions
        .iter()
        .find(|item| item.id == function)
        .ok_or_else(|| "runtime scalar UF4 source function is missing".to_string())
}

fn instruction_result_id(
    function: &crate::KirFunction,
    value: crate::ValueId,
) -> Result<crate::InstructionId, String> {
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
        .map(|instruction| instruction.id)
        .ok_or_else(|| "runtime UF4 planned instruction result is missing".to_string())
}

fn block(function: &crate::KirFunction, id: crate::BlockId) -> Result<&KirBlock, String> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("runtime UF4 block b{} is missing", id.index()))
}

fn block_mut(
    function: &mut crate::KirFunction,
    id: crate::BlockId,
) -> Result<&mut KirBlock, String> {
    function
        .blocks
        .iter_mut()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("runtime UF4 block b{} is missing", id.index()))
}

fn set_block(
    function: &mut crate::KirFunction,
    id: crate::BlockId,
    replacement: KirBlock,
) -> Result<(), String> {
    *block_mut(function, id)? = replacement;
    Ok(())
}
