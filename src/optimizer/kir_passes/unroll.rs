use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CandidateBudgetCharge, CanonicalLoopDescriptor, KirBlock, KirBlockParam, KirEdge,
    KirInstruction, KirInstructionKind, KirMemoryBlockParam, KirPreStateIdentity, KirResult,
    KirTerminator, KirVerifiedProgramState, UnrollCandidate, UnrollInstructionMapping, UnrollPlan,
    UnrollProofRecord, ValueId, VectorPlanGrowth, analyze_canonical_loops, kir_function_units,
    loop_cfg_digest, simple_unroll_shape,
};

use super::rewrite::remap_instruction_values;

type ValueMap = BTreeMap<ValueId, ValueId>;
type MemoryMap = BTreeMap<crate::MemoryVersionId, crate::MemoryVersionId>;

struct RemainderTemplate<'a> {
    header: &'a KirBlock,
    body: &'a KirBlock,
    shape: crate::SimpleUnrollShape,
    factor: u8,
    remainder: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedUnroll {
    pub trial: KirVerifiedProgramState,
    pub plan: UnrollPlan,
    pub charge: CandidateBudgetCharge,
}

pub(crate) fn materialize_unroll_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &UnrollCandidate,
) -> Result<MaterializedUnroll, String> {
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "unroll candidate function is missing".to_string())?
        .clone();
    let analysis = analyze_canonical_loops(&original);
    let descriptor = analysis
        .loops
        .iter()
        .find(|descriptor| {
            descriptor.id == candidate.loop_id && descriptor.header == candidate.header
        })
        .ok_or_else(|| "unroll candidate loop is stale".to_string())?;
    let shape = simple_unroll_shape(&original, descriptor)
        .ok_or_else(|| "unsupported-unroll-shape".to_string())?;
    validate_candidate_against_descriptor(candidate, descriptor)?;

    let protected = pre_state.proofs().instruction_dependencies();
    let loop_instruction_ids = descriptor
        .blocks
        .iter()
        .flat_map(|id| {
            original
                .blocks
                .iter()
                .find(|block| block.id == *id)
                .into_iter()
                .flat_map(|block| block.instructions.iter().map(|instruction| instruction.id))
        })
        .collect::<BTreeSet<_>>();
    if protected
        .iter()
        .any(|instruction| loop_instruction_ids.contains(instruction))
    {
        return Err("unroll-certificate-dependency".to_string());
    }

    let mut trial = pre_state.clone();
    let mut transformed = original.clone();
    let source_order = block(&original, shape.body)?
        .instructions
        .iter()
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>();
    let mut mapping = Vec::new();
    if candidate.full {
        materialize_full(
            &mut trial,
            &mut transformed,
            descriptor,
            shape,
            candidate.trip_count,
            &mut mapping,
        )?;
    } else {
        materialize_partial(
            &mut trial,
            &mut transformed,
            descriptor,
            shape,
            candidate.factor,
            candidate.remainder,
            &mut mapping,
        )?;
    }
    mapping.sort_by_key(|item| (item.scalar_iteration, item.source));

    let original_units = loop_units(&original, descriptor);
    let module_before_units = module_units(pre_state.module());
    let before_function_units = kir_function_units(&original);
    let after_function_units = kir_function_units(&transformed);
    let transformed_units =
        after_function_units.saturating_sub(before_function_units.saturating_sub(original_units));
    let module_after_units = module_before_units
        .saturating_sub(before_function_units)
        .saturating_add(after_function_units);
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == original.id)
        .ok_or_else(|| "unroll trial function disappeared".to_string())? = transformed;

    let plan = UnrollPlan {
        pre_state: KirPreStateIdentity {
            function: original.id,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: kir_function_units(&original),
        },
        function: original.id,
        loop_id: candidate.loop_id,
        header: candidate.header,
        factor: candidate.factor,
        full: candidate.full,
        trip_count: candidate.trip_count,
        remainder: candidate.remainder,
        body_units: candidate.body_units,
        o3_entry_module_units: pre_state.optimization_entry_module_units(),
        instruction_mapping: mapping,
        cost: candidate.predicted_cost,
        growth: VectorPlanGrowth::new(
            original_units,
            transformed_units,
            module_before_units,
            module_after_units,
        ),
        proof: UnrollProofRecord {
            cfg_digest: loop_cfg_digest(&original),
            source_order,
            iterations: candidate.trip_count,
            factor: candidate.factor,
            remainder: candidate.remainder,
            dedicated_exits: descriptor.dedicated_exits,
            lcssa: descriptor.lcssa,
        },
    };
    let mapping_units = u32::try_from(plan.instruction_mapping.len()).unwrap_or(u32::MAX);
    let charge = CandidateBudgetCharge::single(
        plan.function,
        8_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(mapping_units),
        16_u32
            .saturating_add(plan.growth.original_units)
            .saturating_add(plan.growth.transformed_units)
            .saturating_add(mapping_units.saturating_mul(2)),
    );
    Ok(MaterializedUnroll {
        trial,
        plan,
        charge,
    })
}

fn validate_candidate_against_descriptor(
    candidate: &UnrollCandidate,
    descriptor: &CanonicalLoopDescriptor,
) -> Result<(), String> {
    let trip_count = match descriptor.trip_count {
        crate::LoopTripCount::Zero => 0,
        crate::LoopTripCount::Exact { iterations } => u32::try_from(iterations)
            .map_err(|_| "unroll exact trip count exceeds u32".to_string())?,
        crate::LoopTripCount::Runtime { .. } | crate::LoopTripCount::Unknown => {
            return Err("non-exact-unroll-trip".to_string());
        }
    };
    if trip_count != candidate.trip_count
        || (candidate.full
            && (trip_count > 8 || candidate.body_units > 16 || candidate.factor != 1))
        || (!candidate.full
            && (!matches!(candidate.factor, 2 | 4)
                || candidate.remainder
                    != u8::try_from(trip_count % u32::from(candidate.factor)).unwrap_or(u8::MAX)))
    {
        return Err("unroll candidate limits or trip partition are false".to_string());
    }
    Ok(())
}

fn materialize_full(
    state: &mut KirVerifiedProgramState,
    function: &mut crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    shape: crate::SimpleUnrollShape,
    trip_count: u32,
    mapping: &mut Vec<UnrollInstructionMapping>,
) -> Result<(), String> {
    let header = block(function, shape.header)?.clone();
    let body = block(function, shape.body)?.clone();
    let preheader = block(function, shape.preheader)?.clone();
    let KirTerminator::Branch { else_edge, .. } = &header.terminator else {
        return Err("unroll header branch disappeared".to_string());
    };
    if trip_count == 0 {
        let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
            return Err("unroll preheader edge disappeared".to_string());
        };
        let values = header
            .params
            .iter()
            .zip(&incoming.args)
            .map(|(parameter, value)| (parameter.value, *value))
            .collect::<BTreeMap<_, _>>();
        let memories = header
            .memory_params
            .iter()
            .zip(&incoming.memory_args)
            .map(|(parameter, version)| (parameter.version, *version))
            .collect::<BTreeMap<_, _>>();
        block_mut(function, shape.preheader)?.terminator = KirTerminator::Jump {
            edge: remap_edge(else_edge, &values, &memories),
        };
        let (body_values, body_memories) = body_backedge_state(&header, &body)?;
        block_mut(function, shape.body)?.terminator = KirTerminator::Jump {
            edge: remap_edge(else_edge, &body_values, &body_memories),
        };
        return Ok(());
    }

    for instruction in &body.instructions {
        mapping.push(UnrollInstructionMapping {
            scalar_iteration: 0,
            source: instruction.id,
            transformed: instruction.id,
        });
    }
    let (mut current, memories) = body_backedge_state(&header, &body)?;
    let mut appended = Vec::new();
    for iteration in 1..trip_count {
        let next = clone_body_iteration(
            state,
            &header,
            &body,
            &current,
            iteration,
            &mut appended,
            mapping,
        )?;
        current = next;
    }
    let exit = remap_edge(else_edge, &current, &memories);
    let transformed_body = block_mut(function, shape.body)?;
    transformed_body.instructions.extend(appended);
    transformed_body.terminator = KirTerminator::Jump { edge: exit };
    let _ = descriptor;
    Ok(())
}

fn materialize_partial(
    state: &mut KirVerifiedProgramState,
    function: &mut crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    shape: crate::SimpleUnrollShape,
    factor: u8,
    remainder: u8,
    mapping: &mut Vec<UnrollInstructionMapping>,
) -> Result<(), String> {
    let header = block(function, shape.header)?.clone();
    let body = block(function, shape.body)?.clone();
    let induction = descriptor
        .induction
        .as_ref()
        .ok_or_else(|| "partial unroll induction is missing".to_string())?;
    if !((induction.comparison == crate::MirCompareOp::Lt && induction.step == 1.into())
        || (induction.comparison == crate::MirCompareOp::Gt && induction.step == (-1).into()))
    {
        return Err("partial-unroll-requires-strict-unit-induction".to_string());
    }
    for instruction in &body.instructions {
        mapping.push(UnrollInstructionMapping {
            scalar_iteration: 0,
            source: instruction.id,
            transformed: instruction.id,
        });
    }
    let (mut current, _) = body_backedge_state(&header, &body)?;
    let mut appended = Vec::new();
    for iteration in 1..u32::from(factor) {
        current = clone_body_iteration(
            state,
            &header,
            &body,
            &current,
            iteration,
            &mut appended,
            mapping,
        )?;
    }
    let body_block = block_mut(function, shape.body)?;
    body_block.instructions.extend(appended);
    let KirTerminator::Jump { edge } = &mut body_block.terminator else {
        return Err("partial unroll body backedge disappeared".to_string());
    };
    edge.args = header
        .params
        .iter()
        .map(|parameter| remap_value(parameter.value, &current))
        .collect();

    let grouped_iterations = descriptor_trip(descriptor)?.saturating_sub(u32::from(remainder));
    let grouped_bound = &induction.start + &induction.step * grouped_iterations;
    let bound_type = header
        .params
        .iter()
        .find(|parameter| parameter.value == induction.value)
        .map(|parameter| parameter.type_node.clone())
        .ok_or_else(|| "partial unroll induction parameter is missing".to_string())?;
    let new_bound = state.fresh_value()?;
    let bound_instruction = state.fresh_instruction()?;
    block_mut(function, shape.preheader)?
        .instructions
        .push(KirInstruction {
            id: bound_instruction,
            results: vec![KirResult {
                value: new_bound,
                type_node: bound_type,
            }],
            kind: KirInstructionKind::ConstInt {
                value: grouped_bound.to_string(),
            },
            memory: None,
            effect: None,
        });
    replace_header_bound(function, shape.header, induction.bound, new_bound)?;
    if remainder != 0 {
        append_remainder_block(
            state,
            function,
            RemainderTemplate {
                header: &header,
                body: &body,
                shape,
                factor,
                remainder,
            },
            mapping,
        )?;
    }
    Ok(())
}

fn append_remainder_block(
    state: &mut KirVerifiedProgramState,
    function: &mut crate::KirFunction,
    template: RemainderTemplate<'_>,
    mapping: &mut Vec<UnrollInstructionMapping>,
) -> Result<(), String> {
    let RemainderTemplate {
        header,
        body,
        shape,
        factor,
        remainder,
    } = template;
    let KirTerminator::Branch { else_edge, .. } = &header.terminator else {
        return Err("partial unroll exit edge disappeared".to_string());
    };
    let remainder_id = state.fresh_block()?;
    let mut params = Vec::new();
    let mut current = BTreeMap::new();
    for parameter in &header.params {
        let value = state.fresh_value()?;
        current.insert(parameter.value, value);
        params.push(KirBlockParam {
            value,
            slot: format!("{}_unroll_remainder", parameter.slot),
            type_node: parameter.type_node.clone(),
        });
    }
    let mut memory_params = Vec::new();
    let mut memory_map = BTreeMap::new();
    for parameter in &header.memory_params {
        let version = state.fresh_memory_version()?;
        memory_map.insert(parameter.version, version);
        memory_params.push(KirMemoryBlockParam {
            version,
            region: parameter.region,
        });
    }
    let mut instructions = Vec::new();
    for offset in 0..u32::from(remainder) {
        current = clone_body_iteration(
            state,
            header,
            body,
            &current,
            u32::from(factor).saturating_add(offset),
            &mut instructions,
            mapping,
        )?;
    }
    let jump = remap_edge(else_edge, &current, &memory_map);
    function.blocks.push(KirBlock {
        id: remainder_id,
        label: format!("b{}_unroll_remainder", shape.header.index()),
        params,
        memory_params,
        instructions,
        terminator: KirTerminator::Jump { edge: jump },
    });
    let header_block = block_mut(function, shape.header)?;
    let KirTerminator::Branch { else_edge, .. } = &mut header_block.terminator else {
        return Err("partial unroll header branch disappeared".to_string());
    };
    else_edge.target = remainder_id;
    else_edge.args = header
        .params
        .iter()
        .map(|parameter| parameter.value)
        .collect();
    else_edge.memory_args = header
        .memory_params
        .iter()
        .map(|parameter| parameter.version)
        .collect();
    Ok(())
}

fn clone_body_iteration(
    state: &mut KirVerifiedProgramState,
    header: &KirBlock,
    body: &KirBlock,
    current_header_values: &BTreeMap<ValueId, ValueId>,
    iteration: u32,
    output: &mut Vec<KirInstruction>,
    mapping: &mut Vec<UnrollInstructionMapping>,
) -> Result<BTreeMap<ValueId, ValueId>, String> {
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return Err("unroll header branch disappeared".to_string());
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err("unroll body backedge disappeared".to_string());
    };
    let mut values = current_header_values.clone();
    for (parameter, argument) in body.params.iter().zip(&then_edge.args) {
        values.insert(
            parameter.value,
            remap_value(*argument, current_header_values),
        );
    }
    for source in &body.instructions {
        let mut cloned = source.clone();
        cloned.id = state.fresh_instruction()?;
        for result in &mut cloned.results {
            let fresh = state.fresh_value()?;
            values.insert(result.value, fresh);
            result.value = fresh;
        }
        remap_instruction_values(&mut cloned, &values);
        mapping.push(UnrollInstructionMapping {
            scalar_iteration: iteration,
            source: source.id,
            transformed: cloned.id,
        });
        output.push(cloned);
    }
    Ok(header
        .params
        .iter()
        .zip(&backedge.args)
        .map(|(parameter, argument)| (parameter.value, remap_value(*argument, &values)))
        .collect())
}

fn body_backedge_state(
    header: &KirBlock,
    body: &KirBlock,
) -> Result<(ValueMap, MemoryMap), String> {
    let KirTerminator::Jump { edge } = &body.terminator else {
        return Err("unroll body backedge disappeared".to_string());
    };
    Ok((
        header
            .params
            .iter()
            .zip(&edge.args)
            .map(|(parameter, value)| (parameter.value, *value))
            .collect(),
        header
            .memory_params
            .iter()
            .zip(&edge.memory_args)
            .map(|(parameter, value)| (parameter.version, *value))
            .collect(),
    ))
}

fn replace_header_bound(
    function: &mut crate::KirFunction,
    header: crate::BlockId,
    old: ValueId,
    new: ValueId,
) -> Result<(), String> {
    let block = block_mut(function, header)?;
    let KirTerminator::Branch { condition, .. } = block.terminator else {
        return Err("partial unroll header is not conditional".to_string());
    };
    let instruction = block
        .instructions
        .iter_mut()
        .find(|instruction| {
            instruction
                .results
                .first()
                .is_some_and(|r| r.value == condition)
        })
        .ok_or_else(|| "partial unroll header comparison is missing".to_string())?;
    let KirInstructionKind::Compare { left, right, .. } = &mut instruction.kind else {
        return Err("partial unroll condition is not a comparison".to_string());
    };
    if *left == old {
        *left = new;
    } else if *right == old {
        *right = new;
    } else {
        return Err("partial unroll comparison bound is stale".to_string());
    }
    Ok(())
}

fn remap_edge(
    edge: &KirEdge,
    values: &BTreeMap<ValueId, ValueId>,
    memories: &BTreeMap<crate::MemoryVersionId, crate::MemoryVersionId>,
) -> KirEdge {
    KirEdge {
        target: edge.target,
        args: edge
            .args
            .iter()
            .map(|value| remap_value(*value, values))
            .collect(),
        memory_args: edge
            .memory_args
            .iter()
            .map(|version| memories.get(version).copied().unwrap_or(*version))
            .collect(),
    }
}

fn remap_value(value: ValueId, values: &BTreeMap<ValueId, ValueId>) -> ValueId {
    values.get(&value).copied().unwrap_or(value)
}

fn descriptor_trip(descriptor: &CanonicalLoopDescriptor) -> Result<u32, String> {
    match descriptor.trip_count {
        crate::LoopTripCount::Zero => Ok(0),
        crate::LoopTripCount::Exact { iterations } => {
            u32::try_from(iterations).map_err(|_| "unroll trip count exceeds u32".to_string())
        }
        crate::LoopTripCount::Runtime { .. } | crate::LoopTripCount::Unknown => {
            Err("non-exact-unroll-trip".to_string())
        }
    }
}

fn loop_units(function: &crate::KirFunction, descriptor: &CanonicalLoopDescriptor) -> u32 {
    descriptor.blocks.iter().fold(0_u32, |total, id| {
        total.saturating_add(block(function, *id).map_or(0, |block| {
            1_u32
                .saturating_add(u32::try_from(block.params.len()).unwrap_or(u32::MAX))
                .saturating_add(u32::try_from(block.memory_params.len()).unwrap_or(u32::MAX))
                .saturating_add(u32::try_from(block.instructions.len()).unwrap_or(u32::MAX))
        }))
    })
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}

fn block(function: &crate::KirFunction, id: crate::BlockId) -> Result<&KirBlock, String> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("unroll block b{} is missing", id.index()))
}

fn block_mut(
    function: &mut crate::KirFunction,
    id: crate::BlockId,
) -> Result<&mut KirBlock, String> {
    function
        .blocks
        .iter_mut()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("unroll block b{} is missing", id.index()))
}
