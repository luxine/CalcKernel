use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CandidateBudgetCharge, KirArithmeticSemantics, KirBlock, KirBlockParam, KirEdge,
    KirInstruction, KirInstructionKind, KirMemoryAccess, KirMemoryBlockParam, KirResult,
    KirTerminator, KirVerifiedProgramState, KirVersionPredicate, KirVersionPredicateConjunct,
    MemoryVersionId, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, ValueId,
    WasmInvariantLoadCandidate, WasmInvariantLoadPlan, kir_function_units,
};

use super::super::PreparedWasmInvariantLoad;
use super::rewrite::{remap_instruction_values, remap_terminator_values};

type Values = BTreeMap<ValueId, ValueId>;
type Memories = BTreeMap<MemoryVersionId, MemoryVersionId>;
const GUARDED_FAST_UNROLL_FACTOR: u32 = 8;

pub fn prepare_wasm_invariant_load_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &WasmInvariantLoadCandidate,
) -> Result<PreparedWasmInvariantLoad, String> {
    preflight(pre_state, candidate)?;
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "invariant-load source function is missing".to_string())?
        .clone();
    let shape = source_shape(&original, candidate)?;
    let source_preheader = shape.preheader.clone();
    let source_header = shape.header.clone();
    let source_body = shape.body.clone();
    let source_load = instruction(&original, candidate.load)?.clone();
    let source_load_memory = source_load
        .memory
        .as_ref()
        .ok_or_else(|| "invariant load has no MemorySSA input".to_string())?;

    let mut trial = pre_state.clone();
    let fast_entry_id = trial.fresh_block()?;
    let fast_header_id = trial.fresh_block()?;
    let fast_body_id = trial.fresh_block()?;
    let mut preheader = source_preheader.clone();

    let header_entry_values = source_header
        .params
        .iter()
        .zip(&shape.incoming.args)
        .map(|(parameter, value)| (parameter.value, *value))
        .collect::<Values>();
    let mut body_entry_values = BTreeMap::new();
    for (parameter, argument) in source_body.params.iter().zip(&shape.then_edge.args) {
        let mapped = header_entry_values
            .get(argument)
            .copied()
            .unwrap_or(*argument);
        body_entry_values.insert(parameter.value, mapped);
    }
    let mut index_values = body_entry_values.clone();
    let mut index_instruction_map = Vec::new();
    for source_id in &candidate.index_dag {
        let source = instruction(&original, *source_id)?;
        ensure_pure_index_instruction(source)?;
        let mut cloned = source.clone();
        cloned.id = trial.fresh_instruction()?;
        for result in &mut cloned.results {
            let fresh = trial.fresh_value()?;
            index_values.insert(result.value, fresh);
            result.value = fresh;
        }
        remap_instruction_values(&mut cloned, &index_values);
        cloned.memory = None;
        cloned.effect = None;
        index_instruction_map.push((source.id, cloned.id));
        preheader.instructions.push(cloned);
    }
    let materialized_index = index_values
        .get(&candidate.index)
        .copied()
        .or_else(|| {
            value_dominates_block(&original, candidate.index, source_preheader.id)
                .then_some(candidate.index)
        })
        .ok_or_else(|| "invariant index could not be rematerialized at loop entry".to_string())?;
    let output_zero = u32_constant(&mut trial, &mut preheader, 0)?;
    let body_induction = source_body
        .params
        .iter()
        .zip(&shape.then_edge.args)
        .find_map(|(parameter, argument)| {
            (*argument == candidate.induction).then_some(parameter.value)
        })
        .ok_or_else(|| "loop induction has no body parameter".to_string())?;
    let input_dag = candidate.index_dag.iter().copied().collect::<BTreeSet<_>>();
    let mut output_values = index_values.clone();
    output_values.insert(body_induction, output_zero);
    let mut output_instruction_map = Vec::new();
    for source_id in &candidate.output_range_dag {
        let source = instruction(&original, *source_id)?;
        if input_dag.contains(source_id) {
            let result = source
                .results
                .first()
                .ok_or_else(|| "shared range instruction has no result".to_string())?;
            let mapped = index_values
                .get(&result.value)
                .copied()
                .ok_or_else(|| "shared range result was not materialized".to_string())?;
            output_values.insert(result.value, mapped);
            output_instruction_map.push((
                *source_id,
                index_instruction_map
                    .iter()
                    .find(|(old, _)| old == source_id)
                    .map(|(_, new)| *new)
                    .ok_or_else(|| "shared range instruction map is missing".to_string())?,
            ));
            continue;
        }
        ensure_pure_index_instruction(source)?;
        let mut cloned = source.clone();
        cloned.id = trial.fresh_instruction()?;
        for result in &mut cloned.results {
            let fresh = trial.fresh_value()?;
            output_values.insert(result.value, fresh);
            result.value = fresh;
        }
        remap_instruction_values(&mut cloned, &output_values);
        cloned.memory = None;
        cloned.effect = None;
        output_instruction_map.push((source.id, cloned.id));
        preheader.instructions.push(cloned);
    }
    let materialized_output_start = output_values
        .get(&candidate.output_index)
        .copied()
        .ok_or_else(|| "output write interval start could not be materialized".to_string())?;
    let entry_bound = header_entry_values
        .get(&candidate.bound)
        .copied()
        .unwrap_or(candidate.bound);
    let count_one = u32_constant(&mut trial, &mut preheader, 1)?;
    let guard_value = trial.fresh_value()?;
    let guard_instruction = trial.fresh_instruction()?;
    preheader.instructions.push(KirInstruction {
        id: guard_instruction,
        results: vec![KirResult {
            value: guard_value,
            type_node: MirType::Primitive(MirPrimitiveTypeName::Bool).into(),
        }],
        kind: KirInstructionKind::VersionPredicate {
            predicate: KirVersionPredicate {
                address_bits: 32,
                conjuncts: vec![
                    KirVersionPredicateConjunct::TripThreshold {
                        value: entry_bound,
                        minimum: candidate.minimum_trip,
                    },
                    KirVersionPredicateConjunct::WasmSliceRange {
                        slice: candidate.input_slice,
                        start: materialized_index,
                        count: count_one,
                        element_bytes: 8,
                    },
                    KirVersionPredicateConjunct::WasmSliceRange {
                        slice: candidate.output_slice,
                        start: materialized_output_start,
                        count: entry_bound,
                        element_bytes: 8,
                    },
                ],
            },
        },
        memory: None,
        effect: None,
    });

    let mut fast_entry = KirBlock {
        id: fast_entry_id,
        label: format!("invariant_load_entry_{}", candidate.loop_id.index()),
        params: Vec::with_capacity(source_header.params.len() + 1),
        memory_params: Vec::with_capacity(source_header.memory_params.len()),
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: None,
            memory: Vec::new(),
            effect_order: 0,
        },
    };
    let mut entry_values = BTreeMap::new();
    for parameter in &source_header.params {
        let value = trial.fresh_value()?;
        entry_values.insert(parameter.value, value);
        fast_entry.params.push(KirBlockParam {
            value,
            slot: format!("hoist_entry_{}", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    let entry_index_param = trial.fresh_value()?;
    fast_entry.params.push(KirBlockParam {
        value: entry_index_param,
        slot: "hoist_a_index".to_string(),
        type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
    });
    let mut entry_memories = BTreeMap::new();
    for parameter in &source_header.memory_params {
        let version = trial.fresh_memory_version()?;
        entry_memories.insert(parameter.version, version);
        fast_entry.memory_params.push(KirMemoryBlockParam {
            version,
            region: parameter.region,
        });
    }

    let bulk_limit_input = entry_values
        .get(&candidate.bound)
        .copied()
        .unwrap_or(candidate.bound);
    let bulk_limit_offset =
        u32_constant(&mut trial, &mut fast_entry, GUARDED_FAST_UNROLL_FACTOR - 1)?;
    let materialized_bulk_limit = trial.fresh_value()?;
    fast_entry.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: materialized_bulk_limit,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: bulk_limit_input,
            right: bulk_limit_offset,
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });

    let mut fast_header = KirBlock {
        id: fast_header_id,
        label: format!("invariant_load_header_{}", candidate.loop_id.index()),
        params: Vec::with_capacity(source_header.params.len() + 2),
        memory_params: Vec::with_capacity(source_header.memory_params.len()),
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: None,
            memory: Vec::new(),
            effect_order: 0,
        },
    };
    let mut header_values = BTreeMap::new();
    let mut value_mapping = Vec::new();
    for parameter in &source_header.params {
        let value = trial.fresh_value()?;
        header_values.insert(parameter.value, value);
        value_mapping.push((parameter.value, value));
        fast_header.params.push(KirBlockParam {
            value,
            slot: format!("hoist_{}", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    let bulk_limit = trial.fresh_value()?;
    fast_header.params.push(KirBlockParam {
        value: bulk_limit,
        slot: "invariant_load_bulk_limit".to_string(),
        type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
    });
    let cached_value = trial.fresh_value()?;
    value_mapping.push((candidate.load_value, cached_value));
    let load_result_type = source_load
        .results
        .first()
        .ok_or_else(|| "invariant load result is missing".to_string())?
        .type_node
        .clone();
    fast_header.params.push(KirBlockParam {
        value: cached_value,
        slot: "hoisted_a_value".to_string(),
        type_node: load_result_type.clone(),
    });
    let mut header_memories = BTreeMap::new();
    for parameter in &source_header.memory_params {
        let version = trial.fresh_memory_version()?;
        header_memories.insert(parameter.version, version);
        fast_header.memory_params.push(KirMemoryBlockParam {
            version,
            region: parameter.region,
        });
    }

    let mut fast_body = KirBlock {
        id: fast_body_id,
        label: format!("invariant_load_body_{}", candidate.loop_id.index()),
        params: Vec::with_capacity(source_body.params.len()),
        memory_params: Vec::with_capacity(source_body.memory_params.len()),
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: None,
            memory: Vec::new(),
            effect_order: 0,
        },
    };
    let mut body_values = header_values.clone();
    for parameter in &source_body.params {
        let value = trial.fresh_value()?;
        body_values.insert(parameter.value, value);
        value_mapping.push((parameter.value, value));
        fast_body.params.push(KirBlockParam {
            value,
            slot: format!("hoist_body_{}", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    body_values.insert(candidate.load_value, cached_value);
    let mut body_memories = BTreeMap::new();
    let mut memory_mapping = Vec::new();
    for parameter in &source_body.memory_params {
        let version = trial.fresh_memory_version()?;
        body_memories.insert(parameter.version, version);
        memory_mapping.push((parameter.version, version));
        fast_body.memory_params.push(KirMemoryBlockParam {
            version,
            region: parameter.region,
        });
    }
    for parameter in &source_header.memory_params {
        if let Some(version) = header_memories.get(&parameter.version) {
            memory_mapping.push((parameter.version, *version));
        }
    }

    let mut effect_order = fresh_effect_order(&original)?;
    let input_memory_parameter = source_body
        .memory_params
        .iter()
        .find(|parameter| parameter.version == source_load_memory.input)
        .ok_or_else(|| "invariant load memory input is not a body parameter".to_string())?;
    let entry_input_memory = fast_entry
        .memory_params
        .iter()
        .find(|parameter| parameter.region == input_memory_parameter.region)
        .ok_or_else(|| "invariant load input partition is absent at entry".to_string())?;
    let hoisted_load = trial.fresh_instruction()?;
    let hoisted_value = trial.fresh_value()?;
    let mut load_effect = source_load
        .effect
        .clone()
        .ok_or_else(|| "invariant load has no ordered effect record".to_string())?;
    load_effect.order = next_effect_order(&mut effect_order)?;
    fast_entry.instructions.push(KirInstruction {
        id: hoisted_load,
        results: vec![KirResult {
            value: hoisted_value,
            type_node: load_result_type,
        }],
        kind: KirInstructionKind::Load {
            place: Box::new(crate::KirPlace::SliceIndex {
                slice: candidate.input_slice,
                index: entry_index_param,
                type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
                region: candidate.input_region,
            }),
        },
        memory: Some(KirMemoryAccess {
            region: source_load_memory.region,
            input: entry_input_memory.version,
            output: None,
        }),
        effect: Some(load_effect),
    });

    let mut header_instruction_map = Vec::new();
    for source in &source_header.instructions {
        let (mut cloned, outputs) = clone_instruction(
            &mut trial,
            source,
            &mut header_values,
            &mut header_memories,
            &mut effect_order,
        )?;
        if source.id == source_header.instructions[0].id {
            let KirInstructionKind::Compare {
                op: MirCompareOp::Lt,
                left,
                ..
            } = &cloned.kind
            else {
                return Err("invariant-load cloned loop test is not U32 less-than".into());
            };
            cloned.kind = KirInstructionKind::Compare {
                op: MirCompareOp::Lt,
                left: *left,
                right: bulk_limit,
            };
        }
        for (source_value, target_value) in outputs {
            value_mapping.push((source_value, target_value));
        }
        if let Some(memory) = &cloned.memory
            && let Some(source_memory) = &source.memory
            && let Some(target) = memory.output
        {
            memory_mapping.push((
                source_memory
                    .output
                    .ok_or_else(|| "header memory output mapping is malformed".to_string())?,
                target,
            ));
        }
        header_instruction_map.push((source.id, cloned.id));
        fast_header.instructions.push(cloned);
    }
    body_values.extend(header_values.iter().map(|(old, new)| (*old, *new)));

    let mut body_instruction_map = Vec::new();
    let elidable = candidate
        .elidable_index_dag
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let lane_one = u32_constant(&mut trial, &mut fast_body, 1)?;
    let mut lane_induction_values = vec![
        *body_values
            .get(&shape.induction_body_value)
            .ok_or_else(|| "fast body induction parameter is missing".to_string())?,
    ];
    for _ in 1..GUARDED_FAST_UNROLL_FACTOR {
        let previous = *lane_induction_values
            .last()
            .ok_or_else(|| "fast lane induction chain is empty".to_string())?;
        let next = trial.fresh_value()?;
        fast_body.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value: next,
                type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
            }],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: previous,
                right: lane_one,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        });
        lane_induction_values.push(next);
    }

    for lane_induction in lane_induction_values.iter().copied() {
        let mut lane_values = body_values.clone();
        lane_values.insert(shape.induction_body_value, lane_induction);
        lane_values.insert(candidate.load_value, cached_value);
        for source in &source_body.instructions {
            if source.id == candidate.load
                || source.id == candidate.induction_update
                || elidable.contains(&source.id)
            {
                continue;
            }
            let (cloned, outputs) = clone_instruction(
                &mut trial,
                source,
                &mut lane_values,
                &mut body_memories,
                &mut effect_order,
            )?;
            for (source_value, target_value) in outputs {
                value_mapping.push((source_value, target_value));
            }
            if let Some(source_memory) = &source.memory
                && let Some(source_output) = source_memory.output
                && let Some(target_output) = cloned.memory.as_ref().and_then(|memory| memory.output)
            {
                memory_mapping.push((source_output, target_output));
            }
            body_instruction_map.push((source.id, cloned.id));
            fast_body.instructions.push(cloned);
        }
    }
    let fast_induction_step_constant =
        u32_constant(&mut trial, &mut fast_body, GUARDED_FAST_UNROLL_FACTOR)?;
    let fast_induction_step = trial.fresh_value()?;
    fast_body.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: fast_induction_step,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: lane_induction_values[0],
            right: fast_induction_step_constant,
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });

    let (
        KirTerminator::Branch {
            condition: _,
            then_edge: source_then,
            else_edge: _,
        },
        KirTerminator::Jump {
            edge: source_backedge,
        },
    ) = (&source_header.terminator, &source_body.terminator)
    else {
        return Err("invariant-load loop shape changed during materialization".to_string());
    };
    let mut header_terminator = source_header.terminator.clone();
    remap_terminator_values(&mut header_terminator, &header_values);
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge: _,
    } = header_terminator
    else {
        return Err("invariant-load header clone lost its branch".to_string());
    };
    fast_header.terminator = KirTerminator::Branch {
        condition,
        then_edge: KirEdge {
            target: fast_body.id,
            args: then_edge.args,
            memory_args: remap_memories(&source_then.memory_args, &header_memories)?,
        },
        // A partial strip-mined loop must hand its remainder back to the
        // untouched scalar header, which retains the exact source tail.
        else_edge: KirEdge {
            target: source_header.id,
            args: fast_header.params[..source_header.params.len()]
                .iter()
                .map(|parameter| parameter.value)
                .collect(),
            memory_args: fast_header
                .memory_params
                .iter()
                .map(|parameter| parameter.version)
                .collect(),
        },
    };

    let mut backedge_args = Vec::with_capacity(source_header.params.len() + 2);
    for (index, argument) in source_backedge.args.iter().enumerate() {
        if index == shape.induction_header_index {
            backedge_args.push(fast_induction_step);
        } else {
            backedge_args.push(
                body_values
                    .get(argument)
                    .or_else(|| header_values.get(argument))
                    .copied()
                    .unwrap_or(*argument),
            );
        }
    }
    backedge_args.push(bulk_limit);
    backedge_args.push(cached_value);
    fast_body.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: fast_header.id,
            args: backedge_args,
            memory_args: remap_memories(&source_backedge.memory_args, &body_memories)?,
        },
    };

    let header_args = shape
        .incoming
        .args
        .iter()
        .copied()
        .chain(std::iter::once(materialized_index))
        .collect::<Vec<_>>();
    preheader.terminator = KirTerminator::Branch {
        condition: guard_value,
        then_edge: KirEdge {
            target: fast_entry.id,
            args: header_args,
            memory_args: shape.incoming.memory_args.clone(),
        },
        else_edge: shape.incoming.clone(),
    };
    // Make sure cloned header arguments follow source block-parameter order.
    let fast_entry_args = source_header
        .params
        .iter()
        .map(|parameter| entry_values[&parameter.value])
        .chain(std::iter::once(materialized_bulk_limit))
        .chain(std::iter::once(hoisted_value))
        .collect::<Vec<_>>();
    fast_entry.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: fast_header.id,
            args: fast_entry_args,
            memory_args: fast_entry
                .memory_params
                .iter()
                .map(|parameter| parameter.version)
                .collect(),
        },
    };

    {
        let function = trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == candidate.function)
            .ok_or_else(|| "invariant-load function disappeared from trial".to_string())?;
        let preheader_slot = function
            .blocks
            .iter_mut()
            .find(|block| block.id == candidate.preheader)
            .ok_or_else(|| "invariant-load preheader disappeared from trial".to_string())?;
        *preheader_slot = preheader;
        function.blocks.extend([fast_entry, fast_header, fast_body]);
    }

    let before_units = kir_function_units(&original);
    let transformed = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "invariant-load transformed function is missing".to_string())?;
    let after_units = kir_function_units(transformed);
    let plan = WasmInvariantLoadPlan {
        candidate: candidate.clone(),
        pre_state: crate::KirPreStateIdentity {
            function: candidate.function,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: before_units,
        },
        fast_entry: fast_entry_id,
        fast_header: fast_header_id,
        fast_body: fast_body_id,
        unroll_factor: GUARDED_FAST_UNROLL_FACTOR as u8,
        guard_instruction,
        guard_value,
        materialized_index,
        bulk_limit_offset,
        materialized_bulk_limit,
        bulk_limit,
        lane_induction_values,
        fast_induction_step_constant,
        fast_induction_step,
        output_zero,
        materialized_output_start,
        count_one,
        hoisted_load,
        hoisted_value,
        cached_value,
        noalias_fact: candidate.noalias_fact,
        cost: candidate.predicted_cost,
        index_instruction_map,
        output_instruction_map,
        header_instruction_map,
        body_instruction_map,
        value_mapping,
        memory_mapping,
        before_units,
        after_units,
    };
    let charge = CandidateBudgetCharge::single(
        candidate.function,
        after_units.saturating_sub(before_units).saturating_add(16),
        before_units.saturating_add(after_units).saturating_add(32),
    );
    Ok(PreparedWasmInvariantLoad {
        trial,
        plan,
        charge,
    })
}

fn preflight(
    state: &KirVerifiedProgramState,
    candidate: &WasmInvariantLoadCandidate,
) -> Result<(), String> {
    let module = state.module();
    if module.config.consumer != crate::KirConsumer::WebAssembly
        || module.profile.wasm_features() != Some(crate::KirWasmFeatures::Baseline)
        || module.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || module.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || module.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
        || candidate.key
            != (crate::CandidateKey::LoopFrontier {
                function: candidate.function,
                loop_id: candidate.loop_id,
                kind: crate::LoopCandidateKind::WasmInvariantLoad,
                variant: crate::LoopCandidateVariant::Scalar,
                vf: 1,
                uf: GUARDED_FAST_UNROLL_FACTOR as u8,
            })
    {
        return Err("invariant-load target or candidate identity is invalid".into());
    }
    let function = module
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "invariant-load function is missing".to_string())?;
    let _ = source_shape(function, candidate)?;
    Ok(())
}

#[derive(Clone)]
struct SourceShape {
    preheader: KirBlock,
    header: KirBlock,
    body: KirBlock,
    incoming: KirEdge,
    then_edge: KirEdge,
    induction_body_value: ValueId,
    induction_header_index: usize,
}

fn source_shape(
    function: &crate::KirFunction,
    candidate: &WasmInvariantLoadCandidate,
) -> Result<SourceShape, String> {
    let get_block = |id| {
        function
            .blocks
            .iter()
            .find(|block| block.id == id)
            .cloned()
            .ok_or_else(|| "invariant-load source block is missing".to_string())
    };
    let preheader = get_block(candidate.preheader)?;
    let header = get_block(candidate.header)?;
    let body = get_block(candidate.body)?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return Err("invariant-load preheader is no longer a single entry edge".into());
    };
    let KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &header.terminator
    else {
        return Err("invariant-load header is no longer a single branch".into());
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err("invariant-load body has no canonical backedge".into());
    };
    if incoming.target != header.id
        || then_edge.target != body.id
        || else_edge.target != candidate.exit
        || backedge.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || then_edge.args.len() != body.params.len()
        || then_edge.memory_args.len() != body.memory_params.len()
        || backedge.args.len() != header.params.len()
        || backedge.memory_args.len() != header.memory_params.len()
    {
        return Err("invariant-load source loop identity changed".into());
    }
    let incoming = incoming.clone();
    let then_edge = then_edge.clone();
    let backedge = backedge.clone();
    let induction_header_index = header
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.induction)
        .ok_or_else(|| "invariant-load induction parameter is missing".to_string())?;
    let induction_body_value = body
        .params
        .get(induction_header_index)
        .filter(|_| then_edge.args.get(induction_header_index) == Some(&candidate.induction))
        .map(|parameter| parameter.value)
        .ok_or_else(|| "invariant-load body induction mapping is malformed".to_string())?;
    if header.params.iter().enumerate().any(|(index, parameter)| {
        if index == induction_header_index {
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
    let next_induction = *backedge
        .args
        .get(induction_header_index)
        .ok_or_else(|| "invariant-load induction backedge is missing".to_string())?;
    let update = instruction(function, candidate.induction_update)?;
    let (step_constant, update_value) = match &update.kind {
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } if *left == induction_body_value => (*right, update.results.first().map(|r| r.value)),
        _ => return Err("invariant-load source update is not modular IV+1".into()),
    };
    let update_value = update_value.ok_or_else(|| "induction update has no result".to_string())?;
    let (_, constant) = definition(function, step_constant)
        .ok_or_else(|| "induction step constant has no source definition".to_string())?;
    if next_induction != update_value
        || update.results.len() != 1
        || update.results[0].type_node.as_scalar()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || update.memory.is_some()
        || update.effect.is_some()
        || !matches!(&constant.kind, KirInstructionKind::ConstInt { value } if value == "1")
        || constant.results.len() != 1
        || constant.results.first().map(|result| result.value) != Some(step_constant)
        || constant
            .results
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || function.blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|user| user.id != update.id && instruction_uses_value(user, update_value))
                || (block.id != body.id && terminator_uses_value(&block.terminator, update_value))
        })
        || backedge
            .args
            .iter()
            .enumerate()
            .any(|(index, argument)| index != induction_header_index && *argument == update_value)
    {
        return Err("invariant-load source update does not close the unit-step induction".into());
    }
    Ok(SourceShape {
        preheader,
        header,
        body,
        incoming,
        then_edge,
        induction_body_value,
        induction_header_index,
    })
}

fn clone_instruction(
    state: &mut KirVerifiedProgramState,
    source: &KirInstruction,
    values: &mut Values,
    memories: &mut Memories,
    effect_order: &mut u32,
) -> Result<(KirInstruction, Vec<(ValueId, ValueId)>), String> {
    let mut cloned = source.clone();
    cloned.id = state.fresh_instruction()?;
    let mut outputs = Vec::new();
    for result in &mut cloned.results {
        let old = result.value;
        let new = state.fresh_value()?;
        values.insert(old, new);
        outputs.push((old, new));
        result.value = new;
    }
    remap_instruction_values(&mut cloned, values);
    if let Some(memory) = &mut cloned.memory {
        memory.input = memories
            .get(&memory.input)
            .copied()
            .ok_or_else(|| "invariant-load clone has an unmapped MemorySSA input".to_string())?;
        if let Some(old_output) = memory.output {
            let new_output = state.fresh_memory_version()?;
            memories.insert(old_output, new_output);
            memory.output = Some(new_output);
        }
    }
    if let Some(effect) = &mut cloned.effect {
        effect.order = next_effect_order(effect_order)?;
    }
    Ok((cloned, outputs))
}

fn ensure_pure_index_instruction(instruction: &KirInstruction) -> Result<(), String> {
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
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
        return Err("invariant-load index DAG contains a non-total instruction".into());
    }
    Ok(())
}

fn u32_constant(
    state: &mut KirVerifiedProgramState,
    block: &mut KirBlock,
    value: u32,
) -> Result<ValueId, String> {
    let result = state.fresh_value()?;
    block.instructions.push(KirInstruction {
        id: state.fresh_instruction()?,
        results: vec![KirResult {
            value: result,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::ConstInt {
            value: value.to_string(),
        },
        memory: None,
        effect: None,
    });
    Ok(result)
}

fn fresh_effect_order(function: &crate::KirFunction) -> Result<u32, String> {
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
        .ok_or_else(|| "invariant-load ordered effect identity is exhausted".into())
}

fn next_effect_order(next: &mut u32) -> Result<u32, String> {
    let order = *next;
    *next = next
        .checked_add(1)
        .ok_or_else(|| "invariant-load ordered effect identity is exhausted".to_string())?;
    Ok(order)
}

fn remap_memories(
    source: &[MemoryVersionId],
    mapping: &Memories,
) -> Result<Vec<MemoryVersionId>, String> {
    source
        .iter()
        .map(|version| {
            mapping
                .get(version)
                .copied()
                .ok_or_else(|| "invariant-load edge has an unmapped MemorySSA version".into())
        })
        .collect()
}

fn instruction(
    function: &crate::KirFunction,
    id: crate::InstructionId,
) -> Result<&KirInstruction, String> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
        .ok_or_else(|| "invariant-load source instruction is missing".to_string())
}

fn definition(
    function: &crate::KirFunction,
    value: ValueId,
) -> Option<(crate::BlockId, &KirInstruction)> {
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

fn instruction_uses_value(instruction: &KirInstruction, value: ValueId) -> bool {
    let mut found = false;
    crate::optimizer::analysis::visit_instruction_uses(instruction, &mut |used| {
        found |= used == value;
    });
    found
}

fn terminator_uses_value(terminator: &KirTerminator, value: ValueId) -> bool {
    let edge_uses = |edge: &KirEdge| edge.args.contains(&value);
    match terminator {
        KirTerminator::Return {
            value: returned, ..
        } => *returned == Some(value),
        KirTerminator::Jump { edge } => edge_uses(edge),
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => *condition == value || edge_uses(then_edge) || edge_uses(else_edge),
    }
}

fn value_dominates_block(
    function: &crate::KirFunction,
    value: ValueId,
    target: crate::BlockId,
) -> bool {
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
