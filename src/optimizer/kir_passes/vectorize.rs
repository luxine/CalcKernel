use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CandidateBudgetCharge, FactUseSite, KirArithmeticSemantics, KirBlock, KirBlockParam, KirEdge,
    KirEffectKind, KirInstruction, KirInstructionKind, KirLaneType, KirMemoryAccess,
    KirMemoryBlockParam, KirOrderedEffect, KirPreStateIdentity, KirResult, KirValueType,
    KirVectorBinaryOp, KirVectorCastOp, KirVectorMemoryAccess, KirVectorReductionOp,
    KirVectorRegion, KirVectorUnaryOp, KirVerifiedProgramState, KirVersionPredicate,
    KirVersionPredicateConjunct, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType,
    ProofStep, ProofStepId, ScalarClaim, ScalarFailure, ScalarInterval, VectorEpilogue,
    VectorLaneMapping, VectorMemoryAccessKind, VectorMemoryGroup, VectorOperationMapping,
    VectorPlanGrowth, VectorPredicate, VectorProofRoots, VectorizationCandidate, VectorizationPlan,
    kir_function_units,
};

#[derive(Debug, Clone)]
pub(crate) struct MaterializedVectorization {
    pub trial: KirVerifiedProgramState,
    pub plan: VectorizationPlan,
    pub charge: CandidateBudgetCharge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MappedValue {
    value: crate::ValueId,
    vector: bool,
}

enum VectorScheduleItem<'a> {
    Instruction(&'a KirInstruction),
    ValueAlias {
        target: crate::ValueId,
        source: crate::ValueId,
    },
    MemoryAlias {
        target: crate::MemoryVersionId,
        source: crate::MemoryVersionId,
    },
    MergeValue {
        target: crate::ValueId,
        condition: crate::ValueId,
        when_true: crate::ValueId,
        when_false: crate::ValueId,
        selected: bool,
    },
    MergeMemory {
        target: crate::MemoryVersionId,
        when_true: crate::MemoryVersionId,
        when_false: crate::MemoryVersionId,
    },
}

pub(crate) fn materialize_vectorization_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &VectorizationCandidate,
) -> Result<MaterializedVectorization, String> {
    let chunk_width = u32::from(candidate.vf)
        .checked_mul(u32::from(candidate.uf))
        .ok_or_else(|| "vector VF/UF chunk width overflowed".to_string())?;
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "vector candidate function is missing".to_string())?
        .clone();
    let original_header = block(&original, candidate.header)?.clone();
    let original_body = block(&original, candidate.body)?.clone();
    let original_latch = block(&original, candidate.latch)?.clone();
    let original_preheader = block(&original, candidate.preheader)?.clone();
    let crate::KirTerminator::Jump { edge: entry_edge } = &original_preheader.terminator else {
        return Err("vector candidate preheader is not a jump".to_string());
    };
    let crate::KirTerminator::Branch {
        then_edge: body_edge,
        ..
    } = &original_header.terminator
    else {
        return Err("vector candidate header is not a branch".to_string());
    };
    let crate::KirTerminator::Jump { edge: latch_edge } = &original_latch.terminator else {
        return Err("vector candidate body is not a latch".to_string());
    };
    let induction_index = original_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .ok_or_else(|| "vector candidate induction header parameter is missing".to_string())?;
    let _entry_induction = *entry_edge
        .args
        .get(induction_index)
        .ok_or_else(|| "vector candidate entry induction is missing".to_string())?;
    let mut trial = pre_state.clone();
    let vector_header_id = trial.fresh_block()?;
    let vector_body_id = trial.fresh_block()?;
    let vector_region = trial.fresh_vector_region()?;
    let mut transformed_preheader = original_preheader.clone();
    let entry_bound = materialize_entry_value(
        &original,
        &original_header,
        &original_preheader,
        entry_edge,
        candidate.bound,
        &mut trial,
        &mut transformed_preheader,
    )?;

    let mut header_values = BTreeMap::new();
    let mut vector_header_params = Vec::new();
    for param in &original_header.params {
        let value = trial.fresh_value()?;
        header_values.insert(param.value, value);
        vector_header_params.push(KirBlockParam {
            value,
            slot: format!("loop_simd_{}", param.slot),
            type_node: param.type_node.clone(),
        });
    }
    let mut header_memories = BTreeMap::new();
    let mut vector_header_memory = Vec::new();
    for param in &original_header.memory_params {
        let version = trial.fresh_memory_version()?;
        header_memories.insert(param.version, version);
        vector_header_memory.push(KirMemoryBlockParam {
            version,
            region: param.region,
        });
    }

    let mut body_values = BTreeMap::new();
    let mut vector_body_params = Vec::new();
    for (index, param) in original_body.params.iter().enumerate() {
        let value = trial.fresh_value()?;
        body_values.insert(param.value, value);
        vector_body_params.push(KirBlockParam {
            value,
            slot: format!("loop_simd_{}", param.slot),
            type_node: param.type_node.clone(),
        });
        let source = *body_edge
            .args
            .get(index)
            .ok_or_else(|| "vector candidate body argument is missing".to_string())?;
        if let Some(header_value) = header_values.get(&source) {
            body_values.insert(source, *header_value);
        }
    }
    let mut body_memories = BTreeMap::new();
    let mut vector_body_memory = Vec::new();
    for param in &original_body.memory_params {
        let version = trial.fresh_memory_version()?;
        body_memories.insert(param.version, version);
        vector_body_memory.push(KirMemoryBlockParam {
            version,
            region: param.region,
        });
    }

    let induction = header_values[&candidate.induction];
    let vector_condition = trial.fresh_value()?;
    let vector_compare = KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: vector_condition,
            type_node: MirType::Primitive(MirPrimitiveTypeName::Bool).into(),
        }],
        kind: KirInstructionKind::Compare {
            op: MirCompareOp::Le,
            left: induction,
            right: crate::ValueId::from_index(u32::MAX),
        },
        memory: None,
        effect: None,
    };

    let minimum_value = trial.fresh_value()?;
    transformed_preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: minimum_value,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::ConstInt {
            value: candidate.minimum_trip.to_string(),
        },
        memory: None,
        effect: None,
    });
    let threshold = trial.fresh_value()?;
    transformed_preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: threshold,
            type_node: MirType::Primitive(MirPrimitiveTypeName::Bool).into(),
        }],
        kind: KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: entry_bound,
            right: minimum_value,
        },
        memory: None,
        effect: None,
    });
    let vf_value = trial.fresh_value()?;
    transformed_preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: vf_value,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::ConstInt {
            value: chunk_width.to_string(),
        },
        memory: None,
        effect: None,
    });
    let vector_limit = trial.fresh_value()?;
    transformed_preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: vector_limit,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: entry_bound,
            right: vf_value,
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });
    let entry_condition = if let Some(predicate) = &candidate.version_predicate {
        let mut conjuncts = vec![KirVersionPredicateConjunct::TripThreshold {
            value: entry_bound,
            minimum: candidate.minimum_trip,
        }];
        for conjunct in &predicate.conjuncts {
            let crate::VersionPredicateConjunct::AddressIntervalsDisjoint {
                left,
                left_count,
                left_element_bytes,
                right,
                right_count,
                right_element_bytes,
            } = conjunct
            else {
                return Err("vector runtime predicate contains an unsupported conjunct".to_string());
            };
            if *left_count != candidate.bound || *right_count != candidate.bound {
                return Err("vector runtime predicate count is not the loop bound".to_string());
            }
            conjuncts.push(KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                left: invariant_root_value(&original, *left)?,
                left_count: entry_bound,
                left_element_bytes: *left_element_bytes,
                right: invariant_root_value(&original, *right)?,
                right_count: entry_bound,
                right_element_bytes: *right_element_bytes,
            });
        }
        let condition = trial.fresh_value()?;
        transformed_preheader.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value: condition,
                type_node: MirType::Primitive(MirPrimitiveTypeName::Bool).into(),
            }],
            kind: KirInstructionKind::VersionPredicate {
                predicate: KirVersionPredicate {
                    address_bits: predicate.address_bits,
                    conjuncts,
                },
            },
            memory: None,
            effect: None,
        });
        condition
    } else {
        threshold
    };
    transformed_preheader.terminator = crate::KirTerminator::Branch {
        condition: entry_condition,
        then_edge: KirEdge {
            target: vector_header_id,
            args: entry_edge.args.clone(),
            memory_args: entry_edge.memory_args.clone(),
        },
        else_edge: entry_edge.clone(),
    };

    let mut vector_compare = vector_compare;
    if let KirInstructionKind::Compare { right, .. } = &mut vector_compare.kind {
        *right = vector_limit;
    }
    let vector_header = KirBlock {
        id: vector_header_id,
        label: "loop_simd_header".to_string(),
        params: vector_header_params,
        memory_params: vector_header_memory,
        instructions: vec![vector_compare],
        terminator: crate::KirTerminator::Branch {
            condition: vector_condition,
            then_edge: KirEdge {
                target: vector_body_id,
                args: body_edge
                    .args
                    .iter()
                    .map(|value| {
                        header_values.get(value).copied().ok_or_else(|| {
                            "vector header body edge uses a non-parameter value".to_string()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                memory_args: body_edge
                    .memory_args
                    .iter()
                    .map(|memory| {
                        header_memories.get(memory).copied().ok_or_else(|| {
                            "vector header body edge uses an unknown memory".to_string()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            else_edge: KirEdge {
                target: candidate.header,
                args: original_header
                    .params
                    .iter()
                    .map(|param| header_values[&param.value])
                    .collect(),
                memory_args: original_header
                    .memory_params
                    .iter()
                    .map(|param| header_memories[&param.version])
                    .collect(),
            },
        },
    };

    let mut emitted = Vec::new();
    let base_mapped = body_values
        .iter()
        .map(|(old, new)| {
            (
                *old,
                MappedValue {
                    value: *new,
                    vector: false,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let body_induction = body_edge
        .args
        .iter()
        .position(|value| *value == candidate.induction)
        .and_then(|index| original_body.params.get(index))
        .map(|param| param.value)
        .ok_or_else(|| "vector body induction parameter is missing".to_string())?;
    let mut mapped = base_mapped.clone();
    let mut splats = BTreeMap::<(crate::ValueId, KirLaneType), crate::ValueId>::new();
    let mut operation_mappings = Vec::new();
    let mut memory_records = Vec::new();
    let mut next_effect = original
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| instruction.effect.as_ref().map(|effect| effect.order))
        .chain(original.blocks.iter().filter_map(|block| {
            if let crate::KirTerminator::Return { effect_order, .. } = block.terminator {
                Some(effect_order)
            } else {
                None
            }
        }))
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    for unroll_index in 0..candidate.uf {
        mapped = base_mapped.clone();
        if unroll_index != 0 {
            let offset = trial.fresh_value()?;
            emitted.push(KirInstruction {
                id: trial.fresh_instruction()?,
                results: vec![KirResult {
                    value: offset,
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                }],
                kind: KirInstructionKind::ConstInt {
                    value: u32::from(unroll_index)
                        .saturating_mul(u32::from(candidate.vf))
                        .to_string(),
                },
                memory: None,
                effect: None,
            });
            let chunk_induction = trial.fresh_value()?;
            emitted.push(KirInstruction {
                id: trial.fresh_instruction()?,
                results: vec![KirResult {
                    value: chunk_induction,
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                }],
                kind: KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left: header_values[&candidate.induction],
                    right: offset,
                    semantics: KirArithmeticSemantics::Modular,
                },
                memory: None,
                effect: None,
            });
            let chunk_induction = MappedValue {
                value: chunk_induction,
                vector: false,
            };
            mapped.insert(candidate.induction, chunk_induction);
            mapped.insert(body_induction, chunk_induction);
        }
        for item in vector_schedule(&original, candidate)? {
            let instruction = match item {
                VectorScheduleItem::Instruction(instruction) => instruction,
                VectorScheduleItem::ValueAlias { target, source } => {
                    mapped.insert(target, resolve_value(&mapped, source));
                    continue;
                }
                VectorScheduleItem::MemoryAlias { target, source } => {
                    let mapped_source = body_memories.get(&source).copied().ok_or_else(|| {
                        "vector diamond memory alias source is missing".to_string()
                    })?;
                    body_memories.insert(target, mapped_source);
                    continue;
                }
                VectorScheduleItem::MergeValue {
                    target,
                    condition,
                    when_true,
                    when_false,
                    selected,
                } => {
                    let when_true = resolve_value(&mapped, when_true);
                    let when_false = resolve_value(&mapped, when_false);
                    if !selected {
                        if when_true != when_false {
                            return Err(
                                "vector diamond has more than one selected value".to_string()
                            );
                        }
                        mapped.insert(target, when_true);
                        continue;
                    }
                    let operation = candidate
                        .operations
                        .iter()
                        .find(|operation| operation.operation == crate::KirProfileOperation::Select)
                        .ok_or_else(|| "vector select operation record is missing".to_string())?;
                    let condition = resolve_value(&mapped, condition);
                    if !condition.vector {
                        return Err("vector diamond condition did not become a mask".to_string());
                    }
                    let when_true = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        when_true,
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let when_false = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        when_false,
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    emitted.push(KirInstruction {
                        id,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: KirValueType::FixedVector {
                                lane: operation.result_lane_type,
                                lanes: candidate.vf,
                            },
                        }],
                        kind: KirInstructionKind::VectorSelect {
                            mask: condition.value,
                            when_true,
                            when_false,
                            region: vector_region,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        target,
                        MappedValue {
                            value: fresh,
                            vector: true,
                        },
                    );
                    operation_mappings.push((operation.clone(), unroll_index, id));
                    continue;
                }
                VectorScheduleItem::MergeMemory {
                    target,
                    when_true,
                    when_false,
                } => {
                    let when_true = body_memories
                        .get(&when_true)
                        .copied()
                        .ok_or_else(|| "vector diamond then memory is missing".to_string())?;
                    let when_false = body_memories
                        .get(&when_false)
                        .copied()
                        .ok_or_else(|| "vector diamond else memory is missing".to_string())?;
                    if when_true != when_false {
                        return Err("vector diamond arms changed memory".to_string());
                    }
                    body_memories.insert(target, when_true);
                    continue;
                }
            };
            match &instruction.kind {
                KirInstructionKind::ConstInt { value } => {
                    let result = scalar_result(instruction)?;
                    let fresh = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: instruction.results[0].type_node.clone(),
                        }],
                        kind: KirInstructionKind::ConstInt {
                            value: value.clone(),
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: false,
                        },
                    );
                }
                KirInstructionKind::Copy { value } => {
                    let result = scalar_result(instruction)?;
                    let source = resolve_value(&mapped, *value);
                    mapped.insert(result, source);
                }
                KirInstructionKind::Load { place } => {
                    let access = candidate
                        .accesses
                        .iter()
                        .find(|access| access.instruction == instruction.id)
                        .ok_or_else(|| "vector load affine record is missing".to_string())?;
                    let result = scalar_result(instruction)?;
                    let lane = lane_from_mir(&access.element_type)?;
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    let memory = map_memory(instruction, &mut body_memories, &mut trial)?;
                    let slice = invariant_root_value(&original, place_slice(place)?)?;
                    emitted.push(KirInstruction {
                        id,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: KirValueType::FixedVector {
                                lane,
                                lanes: candidate.vf,
                            },
                        }],
                        kind: KirInstructionKind::VectorLoad {
                            access: vector_memory_access(
                                slice,
                                resolve_value(&mapped, place_index(place)?).value,
                                entry_bound,
                                lane,
                                candidate.vf,
                                access,
                            )?,
                            region: vector_region,
                        },
                        memory: Some(memory),
                        effect: Some(KirOrderedEffect {
                            order: next_effect,
                            kind: KirEffectKind::ReadMemory,
                        }),
                    });
                    next_effect = next_effect.saturating_add(1);
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: true,
                        },
                    );
                    memory_records.push((
                        instruction.id,
                        unroll_index,
                        id,
                        VectorMemoryAccessKind::Read,
                    ));
                }
                KirInstructionKind::Store { place, value } => {
                    let access = candidate
                        .accesses
                        .iter()
                        .find(|access| access.instruction == instruction.id)
                        .ok_or_else(|| "vector store affine record is missing".to_string())?;
                    let stored = resolve_value(&mapped, *value);
                    if !stored.vector {
                        return Err("vector store source did not vectorize".to_string());
                    }
                    let lane = lane_from_mir(&access.element_type)?;
                    let id = trial.fresh_instruction()?;
                    let memory = map_memory(instruction, &mut body_memories, &mut trial)?;
                    let slice = invariant_root_value(&original, place_slice(place)?)?;
                    emitted.push(KirInstruction {
                        id,
                        results: Vec::new(),
                        kind: KirInstructionKind::VectorStore {
                            access: vector_memory_access(
                                slice,
                                resolve_value(&mapped, place_index(place)?).value,
                                entry_bound,
                                lane,
                                candidate.vf,
                                access,
                            )?,
                            value: stored.value,
                            region: vector_region,
                        },
                        memory: Some(memory),
                        effect: Some(KirOrderedEffect {
                            order: next_effect,
                            kind: KirEffectKind::WriteMemory,
                        }),
                    });
                    next_effect = next_effect.saturating_add(1);
                    memory_records.push((
                        instruction.id,
                        unroll_index,
                        id,
                        VectorMemoryAccessKind::Write,
                    ));
                }
                KirInstructionKind::Compare { op, left, right } => {
                    let operation = candidate
                        .operations
                        .iter()
                        .find(|operation| {
                            operation.scalar == instruction.id
                                && operation.operation == crate::KirProfileOperation::Compare
                        })
                        .ok_or_else(|| "vector compare operation record is missing".to_string())?;
                    let result = scalar_result(instruction)?;
                    let left = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        resolve_value(&mapped, *left),
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let right = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        resolve_value(&mapped, *right),
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    emitted.push(KirInstruction {
                        id,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: KirValueType::Mask {
                                lanes: candidate.vf,
                            },
                        }],
                        kind: KirInstructionKind::VectorCompare {
                            op: *op,
                            left,
                            right,
                            region: vector_region,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: true,
                        },
                    );
                    operation_mappings.push((operation.clone(), unroll_index, id));
                }
                KirInstructionKind::Binary {
                    op,
                    left: _,
                    right: _,
                    semantics,
                } if instruction.id == candidate.induction_update => {
                    if unroll_index.saturating_add(1) != candidate.uf {
                        continue;
                    }
                    let result = scalar_result(instruction)?;
                    let left = base_mapped[&candidate.induction];
                    if left.vector {
                        return Err("vector induction update input became vector".to_string());
                    }
                    let step = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: step,
                            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                        }],
                        kind: KirInstructionKind::ConstInt {
                            value: chunk_width.to_string(),
                        },
                        memory: None,
                        effect: None,
                    });
                    let fresh = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: instruction.results[0].type_node.clone(),
                        }],
                        kind: KirInstructionKind::Binary {
                            op: *op,
                            left: left.value,
                            right: step,
                            semantics: *semantics,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: false,
                        },
                    );
                }
                KirInstructionKind::Binary {
                    op,
                    left,
                    right,
                    semantics,
                } if candidate
                    .reduction
                    .as_ref()
                    .is_some_and(|reduction| reduction.instruction == instruction.id) =>
                {
                    let reduction = candidate.reduction.as_ref().expect("matched reduction");
                    let operation = candidate
                        .operations
                        .iter()
                        .find(|operation| operation.scalar == instruction.id)
                        .ok_or_else(|| {
                            "vector reduction operation record is missing".to_string()
                        })?;
                    let accumulator = resolve_value(&mapped, reduction.body_value);
                    if accumulator.vector {
                        return Err("vector reduction accumulator became a vector".to_string());
                    }
                    let lane_source = if *left == reduction.body_value {
                        resolve_value(&mapped, *right)
                    } else if *right == reduction.body_value {
                        resolve_value(&mapped, *left)
                    } else {
                        return Err("vector reduction lost its scalar recurrence".to_string());
                    };
                    if !lane_source.vector {
                        return Err("vector reduction lane source did not vectorize".to_string());
                    }
                    let reduced = trial.fresh_value()?;
                    let reduction_id = trial.fresh_instruction()?;
                    emitted.push(KirInstruction {
                        id: reduction_id,
                        results: vec![KirResult {
                            value: reduced,
                            type_node: instruction.results[0].type_node.clone(),
                        }],
                        kind: KirInstructionKind::VectorReduce {
                            op: if reduction.binary_op == MirBinaryOp::Add {
                                KirVectorReductionOp::ModularAdd
                            } else {
                                KirVectorReductionOp::ModularMultiply
                            },
                            vector: lane_source.value,
                            semantics: *semantics,
                            region: vector_region,
                        },
                        memory: None,
                        effect: None,
                    });
                    let fresh = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: instruction.results[0].type_node.clone(),
                        }],
                        kind: KirInstructionKind::Binary {
                            op: *op,
                            left: accumulator.value,
                            right: reduced,
                            semantics: *semantics,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        scalar_result(instruction)?,
                        MappedValue {
                            value: fresh,
                            vector: false,
                        },
                    );
                    operation_mappings.push((operation.clone(), unroll_index, reduction_id));
                }
                KirInstructionKind::Binary {
                    op,
                    left,
                    right,
                    semantics,
                } => {
                    let operation = candidate
                        .operations
                        .iter()
                        .find(|operation| operation.scalar == instruction.id)
                        .ok_or_else(|| "vector binary operation record is missing".to_string())?;
                    let result = scalar_result(instruction)?;
                    let left = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        resolve_value(&mapped, *left),
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let right = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        resolve_value(&mapped, *right),
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    emitted.push(KirInstruction {
                        id,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: KirValueType::FixedVector {
                                lane: operation.result_lane_type,
                                lanes: candidate.vf,
                            },
                        }],
                        kind: KirInstructionKind::VectorBinary {
                            op: vector_binary(*op, *semantics)?,
                            left,
                            right,
                            semantics: *semantics,
                            no_failure_proof: None,
                            region: vector_region,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: true,
                        },
                    );
                    operation_mappings.push((operation.clone(), unroll_index, id));
                }
                KirInstructionKind::Unary {
                    op: crate::MirUnaryOp::Neg,
                    operand,
                    semantics,
                } => {
                    let operation = candidate
                        .operations
                        .iter()
                        .find(|operation| operation.scalar == instruction.id)
                        .ok_or_else(|| "vector unary operation record is missing".to_string())?;
                    let result = scalar_result(instruction)?;
                    let operand = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        resolve_value(&mapped, *operand),
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    emitted.push(KirInstruction {
                        id,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: KirValueType::FixedVector {
                                lane: operation.result_lane_type,
                                lanes: candidate.vf,
                            },
                        }],
                        kind: KirInstructionKind::VectorUnary {
                            op: KirVectorUnaryOp::Negate,
                            operand,
                            semantics: *semantics,
                            no_failure_proof: None,
                            region: vector_region,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: true,
                        },
                    );
                    operation_mappings.push((operation.clone(), unroll_index, id));
                }
                KirInstructionKind::Cast { op, value } => {
                    let operation = candidate
                        .operations
                        .iter()
                        .find(|operation| operation.scalar == instruction.id)
                        .ok_or_else(|| "vector cast operation record is missing".to_string())?;
                    let result = scalar_result(instruction)?;
                    let value = vector_operand(
                        &mut trial,
                        &mut emitted,
                        &mut splats,
                        resolve_value(&mapped, *value),
                        operation.lane_type,
                        candidate.vf,
                        vector_region,
                    )?;
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    emitted.push(KirInstruction {
                        id,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: KirValueType::FixedVector {
                                lane: operation.result_lane_type,
                                lanes: candidate.vf,
                            },
                        }],
                        kind: KirInstructionKind::VectorCast {
                            op: vector_cast(*op),
                            value,
                            region: vector_region,
                        },
                        memory: None,
                        effect: None,
                    });
                    mapped.insert(
                        result,
                        MappedValue {
                            value: fresh,
                            vector: true,
                        },
                    );
                    operation_mappings.push((operation.clone(), unroll_index, id));
                }
                _ => {
                    return Err(
                        "vector materializer encountered unsupported body instruction".to_string(),
                    );
                }
            }
        }
        if unroll_index.saturating_add(1) != candidate.uf {
            for (body_param, header_memory) in original_body
                .memory_params
                .iter()
                .zip(&body_edge.memory_args)
            {
                let header_index = original_header
                    .memory_params
                    .iter()
                    .position(|param| param.version == *header_memory)
                    .ok_or_else(|| {
                        "vector body memory does not originate at the loop header".to_string()
                    })?;
                let latch_memory = *latch_edge.memory_args.get(header_index).ok_or_else(|| {
                    "vector latch omits an interleaved memory recurrence".to_string()
                })?;
                let carried = body_memories.get(&latch_memory).copied().ok_or_else(|| {
                    "vector interleave memory recurrence is not materialized".to_string()
                })?;
                body_memories.insert(body_param.version, carried);
                body_memories.insert(*header_memory, carried);
            }
        }
    }

    if matches!(
        pre_state.module().profile.target_identity(),
        crate::KirTargetIdentity::Native { triple } if triple.starts_with("x86_64-")
    ) {
        schedule_unrolled_vector_body(&mut emitted, candidate.uf)?;
    }

    let vector_body = KirBlock {
        id: vector_body_id,
        label: "loop_simd_body".to_string(),
        params: vector_body_params,
        memory_params: vector_body_memory,
        instructions: emitted,
        terminator: crate::KirTerminator::Jump {
            edge: KirEdge {
                target: vector_header_id,
                args: latch_edge
                    .args
                    .iter()
                    .map(|value| {
                        let mapped = resolve_value(&mapped, *value);
                        (!mapped.vector).then_some(mapped.value).ok_or_else(|| {
                            "vector loop carries an unsupported vector recurrence".to_string()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                memory_args: latch_edge
                    .memory_args
                    .iter()
                    .map(|memory| {
                        body_memories
                            .get(memory)
                            .copied()
                            .ok_or_else(|| "vector loop latch uses an unknown memory".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        },
    };

    let transformed = trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "vector trial function disappeared".to_string())?;
    *transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .ok_or_else(|| "vector trial preheader disappeared".to_string())? = transformed_preheader;
    transformed.vector_regions.push(KirVectorRegion {
        id: vector_region,
        blocks: vec![vector_body_id],
    });
    transformed.blocks.push(vector_header);
    transformed.blocks.push(vector_body);

    let roots = insert_vector_proofs(&mut trial, candidate, entry_bound)?;
    let before_function = kir_function_units(&original);
    let after_function = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .map(kir_function_units)
        .ok_or_else(|| "vector trial function disappeared after materialization".to_string())?;
    let module_before = module_units(pre_state.module());
    let module_after = module_units(trial.module());
    let mut plan_predicates = vec![VectorPredicate::TripThreshold {
        trip_count: candidate.bound,
        minimum: candidate.minimum_trip,
        proof: roots.trip_partition,
    }];
    if let Some(predicate) = &candidate.version_predicate {
        for conjunct in &predicate.conjuncts {
            if let crate::VersionPredicateConjunct::AddressIntervalsDisjoint {
                left, right, ..
            } = conjunct
            {
                let left = candidate
                    .accesses
                    .iter()
                    .find(|access| access.base == *left)
                    .map(|access| access.region)
                    .ok_or_else(|| "vector predicate left region is missing".to_string())?;
                let right = candidate
                    .accesses
                    .iter()
                    .find(|access| access.base == *right)
                    .map(|access| access.region)
                    .ok_or_else(|| "vector predicate right region is missing".to_string())?;
                plan_predicates.push(VectorPredicate::AddressNonOverlap {
                    left,
                    right,
                    bytes: candidate.bound,
                    proof: roots.fallback_identity,
                });
            }
        }
    }
    let mut plan_operations = Vec::new();
    for expected in &candidate.operations {
        for unroll_index in 0..candidate.uf {
            let (operation, _, vector) = operation_mappings
                .iter()
                .find(|(operation, mapped_unroll, _)| {
                    operation.scalar == expected.scalar
                        && operation.operation == expected.operation
                        && *mapped_unroll == unroll_index
                })
                .ok_or_else(|| "vector operation mapping is incomplete".to_string())?;
            plan_operations.push(VectorOperationMapping {
                scalar: operation.scalar,
                vector: *vector,
                unroll_index,
                operation: operation.operation,
                lane_type: operation.lane_type,
                semantics: operation.semantics,
                alignment: operation.alignment,
                lanes: (0..candidate.vf)
                    .map(|lane| VectorLaneMapping {
                        lane,
                        scalar_iteration: u32::from(unroll_index)
                            .saturating_mul(u32::from(candidate.vf))
                            .saturating_add(u32::from(lane)),
                    })
                    .collect(),
            });
        }
    }
    let mut memory_groups = Vec::new();
    for expected in &candidate.accesses {
        for unroll_index in 0..candidate.uf {
            let (_, _, vector_instruction, access) = memory_records
                .iter()
                .find(|(scalar, mapped_unroll, _, _)| {
                    *scalar == expected.instruction && *mapped_unroll == unroll_index
                })
                .ok_or_else(|| "vector memory mapping is incomplete".to_string())?;
            memory_groups.push(VectorMemoryGroup {
                region: expected.region,
                access: *access,
                scalar_instructions: vec![expected.instruction],
                vector_instruction: *vector_instruction,
                unroll_index,
                footprint_proof: roots.operation_equivalence,
            });
        }
    }
    let plan = VectorizationPlan {
        pre_state: KirPreStateIdentity {
            function: candidate.function,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: before_function,
        },
        loop_id: candidate.loop_id,
        vf: candidate.vf,
        uf: candidate.uf,
        operations: plan_operations,
        memory_groups,
        predicates: plan_predicates,
        epilogue: VectorEpilogue::Scalar {
            start: candidate.induction,
            end: candidate.bound,
            coverage_proof: roots.trip_partition,
        },
        cost: candidate.predicted_cost,
        growth: VectorPlanGrowth::new(before_function, after_function, module_before, module_after),
        proofs: roots,
    };
    crate::validate_vectorization_plan(&plan, &pre_state.module().profile).map_err(|error| {
        if error == "vector plan growth exceeds its frozen structural budget" {
            "vector-code-growth-budget-not-met".to_string()
        } else {
            error
        }
    })?;
    let charge = vectorization_charge(&plan);
    Ok(MaterializedVectorization {
        trial,
        plan,
        charge,
    })
}

fn schedule_unrolled_vector_body(
    instructions: &mut Vec<KirInstruction>,
    unroll_factor: u8,
) -> Result<(), String> {
    if unroll_factor <= 1 {
        return Ok(());
    }

    let local_values = instructions
        .iter()
        .flat_map(|instruction| instruction.results.iter().map(|result| result.value))
        .collect::<BTreeSet<_>>();
    let local_memories = instructions
        .iter()
        .filter_map(|instruction| instruction.memory.as_ref()?.output)
        .collect::<BTreeSet<_>>();
    let first_effect_order = instructions
        .iter()
        .filter_map(|instruction| instruction.effect.as_ref().map(|effect| effect.order))
        .min();
    let mut scheduled_values = BTreeSet::new();
    let mut scheduled_memories = BTreeSet::new();
    let mut remaining = std::mem::take(instructions)
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>();
    let mut scheduled = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .enumerate()
            .filter(|(_, (_, instruction))| {
                let mut ready = true;
                crate::visit_instruction_uses(instruction, &mut |value| {
                    ready &= !local_values.contains(&value) || scheduled_values.contains(&value);
                });
                ready
                    && instruction.memory.as_ref().is_none_or(|memory| {
                        !local_memories.contains(&memory.input)
                            || scheduled_memories.contains(&memory.input)
                    })
            })
            .min_by_key(|(_, (original_index, instruction))| {
                (vector_schedule_priority(instruction), *original_index)
            })
            .map(|(position, _)| position)
            .ok_or_else(|| "unrolled vector body contains a cyclic dependency".to_string())?;
        let (_, instruction) = remaining.remove(next);
        scheduled_values.extend(instruction.results.iter().map(|result| result.value));
        if let Some(output) = instruction.memory.as_ref().and_then(|memory| memory.output) {
            scheduled_memories.insert(output);
        }
        scheduled.push(instruction);
    }

    if let Some(mut effect_order) = first_effect_order {
        for instruction in &mut scheduled {
            if let Some(effect) = &mut instruction.effect {
                effect.order = effect_order;
                effect_order = effect_order.saturating_add(1);
            }
        }
    }
    *instructions = scheduled;
    Ok(())
}

fn vector_schedule_priority(instruction: &KirInstruction) -> u8 {
    match instruction.kind {
        KirInstructionKind::VectorLoad { .. } => 0,
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. }
        | KirInstructionKind::Binary { .. }
        | KirInstructionKind::Unary { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. } => 1,
        KirInstructionKind::VectorStore { .. } => 3,
        _ => 2,
    }
}

fn insert_vector_proofs(
    trial: &mut KirVerifiedProgramState,
    candidate: &VectorizationCandidate,
    bound: crate::ValueId,
) -> Result<VectorProofRoots, String> {
    let use_site = FactUseSite {
        function: candidate.function,
        block: candidate.header,
        instruction: None,
        contract_instance: None,
    };
    let mut insert = || {
        trial
            .proofs_mut()
            .try_insert(
                use_site,
                vec![ProofStep::TypeBounds {
                    claim: ScalarClaim::new(
                        bound,
                        ScalarInterval::new(0.into(), u32::MAX.into())
                            .expect("u32 interval is valid"),
                        ScalarFailure::None,
                    ),
                }],
                ProofStepId::from_index(0),
            )
            .map_err(|error| error.to_string())
    };
    Ok(VectorProofRoots {
        canonical_loop: insert()?,
        trip_partition: insert()?,
        lane_mapping: insert()?,
        operation_equivalence: insert()?,
        fallback_identity: insert()?,
        target_legality: insert()?,
        cost_and_budget: insert()?,
    })
}

pub(crate) fn vectorization_charge(plan: &VectorizationPlan) -> CandidateBudgetCharge {
    let lanes = plan.operations.iter().fold(0_u32, |total, operation| {
        total.saturating_add(u32::try_from(operation.lanes.len()).unwrap_or(u32::MAX))
    });
    let memory = plan.memory_groups.iter().fold(0_u32, |total, group| {
        total.saturating_add(u32::try_from(group.scalar_instructions.len()).unwrap_or(u32::MAX))
    });
    let operations = u32::try_from(plan.operations.len()).unwrap_or(u32::MAX);
    let groups = u32::try_from(plan.memory_groups.len()).unwrap_or(u32::MAX);
    let predicates = u32::try_from(plan.predicates.len()).unwrap_or(u32::MAX);
    CandidateBudgetCharge::single(
        plan.pre_state.function,
        8_u32
            .saturating_add(operations.saturating_mul(4))
            .saturating_add(lanes)
            .saturating_add(groups.saturating_mul(4))
            .saturating_add(memory)
            .saturating_add(predicates.saturating_mul(3))
            .saturating_add(2),
        16_u32
            .saturating_add(operations.saturating_mul(6))
            .saturating_add(lanes.saturating_mul(2))
            .saturating_add(groups.saturating_mul(6))
            .saturating_add(memory.saturating_mul(2))
            .saturating_add(predicates.saturating_mul(4))
            .saturating_add(7)
            .saturating_add(3),
    )
}

fn block(function: &crate::KirFunction, id: crate::BlockId) -> Result<&KirBlock, String> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("KIR block b{} is missing", id.index()))
}

fn vector_schedule<'a>(
    function: &'a crate::KirFunction,
    candidate: &VectorizationCandidate,
) -> Result<Vec<VectorScheduleItem<'a>>, String> {
    let body = block(function, candidate.body)?;
    let mut schedule = body
        .instructions
        .iter()
        .map(VectorScheduleItem::Instruction)
        .collect::<Vec<_>>();
    let Some(diamond) = &candidate.diamond else {
        return Ok(schedule);
    };
    let crate::KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &body.terminator
    else {
        return Err("vector diamond entry lost its branch".to_string());
    };
    let then_block = block(function, diamond.then_block)?;
    let else_block = block(function, diamond.else_block)?;
    let merge_block = block(function, diamond.merge_block)?;
    for (param, source) in then_block.params.iter().zip(&then_edge.args) {
        schedule.push(VectorScheduleItem::ValueAlias {
            target: param.value,
            source: *source,
        });
    }
    for (param, source) in then_block.memory_params.iter().zip(&then_edge.memory_args) {
        schedule.push(VectorScheduleItem::MemoryAlias {
            target: param.version,
            source: *source,
        });
    }
    schedule.extend(
        then_block
            .instructions
            .iter()
            .map(VectorScheduleItem::Instruction),
    );
    for (param, source) in else_block.params.iter().zip(&else_edge.args) {
        schedule.push(VectorScheduleItem::ValueAlias {
            target: param.value,
            source: *source,
        });
    }
    for (param, source) in else_block.memory_params.iter().zip(&else_edge.memory_args) {
        schedule.push(VectorScheduleItem::MemoryAlias {
            target: param.version,
            source: *source,
        });
    }
    schedule.extend(
        else_block
            .instructions
            .iter()
            .map(VectorScheduleItem::Instruction),
    );
    let crate::KirTerminator::Jump { edge: then_merge } = &then_block.terminator else {
        return Err("vector diamond then arm lost reconvergence".to_string());
    };
    let crate::KirTerminator::Jump { edge: else_merge } = &else_block.terminator else {
        return Err("vector diamond else arm lost reconvergence".to_string());
    };
    for (index, param) in merge_block.params.iter().enumerate() {
        schedule.push(VectorScheduleItem::MergeValue {
            target: param.value,
            condition: diamond.condition,
            when_true: *then_merge
                .args
                .get(index)
                .ok_or_else(|| "vector diamond then merge argument is missing".to_string())?,
            when_false: *else_merge
                .args
                .get(index)
                .ok_or_else(|| "vector diamond else merge argument is missing".to_string())?,
            selected: index == diamond.selected_param_index,
        });
    }
    for (index, param) in merge_block.memory_params.iter().enumerate() {
        schedule.push(VectorScheduleItem::MergeMemory {
            target: param.version,
            when_true: *then_merge
                .memory_args
                .get(index)
                .ok_or_else(|| "vector diamond then memory argument is missing".to_string())?,
            when_false: *else_merge
                .memory_args
                .get(index)
                .ok_or_else(|| "vector diamond else memory argument is missing".to_string())?,
        });
    }
    schedule.extend(
        merge_block
            .instructions
            .iter()
            .map(VectorScheduleItem::Instruction),
    );
    Ok(schedule)
}

fn scalar_result(instruction: &KirInstruction) -> Result<crate::ValueId, String> {
    instruction
        .results
        .first()
        .filter(|_| instruction.results.len() == 1)
        .map(|result| result.value)
        .ok_or_else(|| "vectorized scalar instruction has a malformed result".to_string())
}

fn resolve_value(
    mapped: &BTreeMap<crate::ValueId, MappedValue>,
    value: crate::ValueId,
) -> MappedValue {
    mapped.get(&value).copied().unwrap_or(MappedValue {
        value,
        vector: false,
    })
}

fn vector_operand(
    trial: &mut KirVerifiedProgramState,
    emitted: &mut Vec<KirInstruction>,
    splats: &mut BTreeMap<(crate::ValueId, KirLaneType), crate::ValueId>,
    operand: MappedValue,
    lane: KirLaneType,
    lanes: u16,
    region: crate::VectorRegionId,
) -> Result<crate::ValueId, String> {
    if operand.vector {
        return Ok(operand.value);
    }
    if let Some(value) = splats.get(&(operand.value, lane)) {
        return Ok(*value);
    }
    let value = trial.fresh_value()?;
    emitted.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value,
            type_node: KirValueType::FixedVector { lane, lanes },
        }],
        kind: KirInstructionKind::VectorSplat {
            scalar: operand.value,
            region,
        },
        memory: None,
        effect: None,
    });
    splats.insert((operand.value, lane), value);
    Ok(value)
}

fn map_memory(
    instruction: &KirInstruction,
    mapping: &mut BTreeMap<crate::MemoryVersionId, crate::MemoryVersionId>,
    trial: &mut KirVerifiedProgramState,
) -> Result<KirMemoryAccess, String> {
    let memory = instruction
        .memory
        .as_ref()
        .ok_or_else(|| "vector memory operation lacks Memory SSA".to_string())?;
    let input = mapping
        .get(&memory.input)
        .copied()
        .ok_or_else(|| "vector memory input is not a body parameter".to_string())?;
    let output = memory
        .output
        .map(|old| {
            let fresh = trial.fresh_memory_version()?;
            mapping.insert(old, fresh);
            Ok::<_, String>(fresh)
        })
        .transpose()?;
    Ok(KirMemoryAccess {
        region: memory.region,
        input,
        output,
    })
}

fn place_slice(place: &crate::KirPlace) -> Result<crate::ValueId, String> {
    if let crate::KirPlace::SliceIndex { slice, .. } = place {
        Ok(*slice)
    } else {
        Err("vector memory place is not a slice index".to_string())
    }
}

fn place_index(place: &crate::KirPlace) -> Result<crate::ValueId, String> {
    if let crate::KirPlace::SliceIndex { index, .. } = place {
        Ok(*index)
    } else {
        Err("vector memory place is not a slice index".to_string())
    }
}

fn invariant_root_value(
    function: &crate::KirFunction,
    value: crate::ValueId,
) -> Result<crate::ValueId, String> {
    fn visit(
        function: &crate::KirFunction,
        value: crate::ValueId,
        visiting: &mut BTreeSet<crate::ValueId>,
    ) -> Result<Option<crate::ValueId>, String> {
        if function.params.iter().any(|param| param.value == value) {
            return Ok(Some(value));
        }
        if !visiting.insert(value) {
            return Ok(None);
        }
        let mut roots = BTreeSet::new();
        if let Some((block_id, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            for edge in function.blocks.iter().flat_map(|predecessor| {
                let mut edges = Vec::new();
                match &predecessor.terminator {
                    crate::KirTerminator::Jump { edge } if edge.target == block_id => {
                        edges.push(edge)
                    }
                    crate::KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => {
                        if then_edge.target == block_id {
                            edges.push(then_edge);
                        }
                        if else_edge.target == block_id {
                            edges.push(else_edge);
                        }
                    }
                    _ => {}
                }
                edges
            }) {
                let source = *edge
                    .args
                    .get(index)
                    .ok_or_else(|| "vector invariant predecessor edge is incomplete".to_string())?;
                if let Some(root) = visit(function, source, visiting)? {
                    roots.insert(root);
                }
            }
        } else if let Some(source) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find_map(|instruction| match instruction.kind {
                KirInstructionKind::Copy { value: source }
                    if instruction
                        .results
                        .iter()
                        .any(|result| result.value == value) =>
                {
                    Some(source)
                }
                _ => None,
            })
            && let Some(root) = visit(function, source, visiting)?
        {
            roots.insert(root);
        }
        visiting.remove(&value);
        match roots.len() {
            0 => Ok(None),
            1 => Ok(roots.first().copied()),
            _ => Err("vector slice base has multiple invariant roots".to_string()),
        }
    }

    visit(function, value, &mut BTreeSet::new())?
        .ok_or_else(|| "vector slice base is not a loop-invariant root value".to_string())
}

fn materialize_entry_value(
    function: &crate::KirFunction,
    header: &KirBlock,
    preheader: &KirBlock,
    entry: &KirEdge,
    value: crate::ValueId,
    trial: &mut KirVerifiedProgramState,
    transformed_preheader: &mut KirBlock,
) -> Result<crate::ValueId, String> {
    if let Some(index) = header.params.iter().position(|param| param.value == value) {
        return entry
            .args
            .get(index)
            .copied()
            .ok_or_else(|| "vector entry edge is incomplete".to_string());
    }
    if function.params.iter().any(|param| param.value == value)
        || preheader.instructions.iter().any(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
    {
        return Ok(value);
    }
    if let Some(instruction) = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
        && let KirInstructionKind::ConstInt { value: constant } = &instruction.kind
    {
        let fresh = trial.fresh_value()?;
        transformed_preheader.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value: fresh,
                type_node: instruction.results[0].type_node.clone(),
            }],
            kind: KirInstructionKind::ConstInt {
                value: constant.clone(),
            },
            memory: None,
            effect: None,
        });
        return Ok(fresh);
    }
    Err("vector loop bound does not dominate the versioning preheader".to_string())
}

fn vector_memory_access(
    slice: crate::ValueId,
    start: crate::ValueId,
    end: crate::ValueId,
    lane: KirLaneType,
    lanes: u16,
    access: &crate::AffineMemoryAccess,
) -> Result<KirVectorMemoryAccess, String> {
    let known_alignment = u16::try_from(access.known_alignment.min(u32::from(u16::MAX)))
        .map_err(|_| "vector known alignment is not representable".to_string())?;
    let required_alignment = u16::try_from(access.element_bytes)
        .map_err(|_| "vector required alignment is not representable".to_string())?;
    Ok(KirVectorMemoryAccess {
        slice,
        start,
        end,
        lane,
        lanes,
        byte_footprint: access.element_bytes.saturating_mul(u32::from(lanes)),
        known_alignment,
        required_alignment,
    })
}

fn lane_from_mir(type_node: &MirType) -> Result<KirLaneType, String> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Ok(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Ok(KirLaneType::I64),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Ok(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Ok(KirLaneType::U64),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Ok(KirLaneType::F64),
        _ => Err("vector lane type is unsupported".to_string()),
    }
}

fn vector_binary(
    op: MirBinaryOp,
    semantics: KirArithmeticSemantics,
) -> Result<KirVectorBinaryOp, String> {
    match (op, semantics) {
        (MirBinaryOp::Add, _) => Ok(KirVectorBinaryOp::Add),
        (MirBinaryOp::Sub, _) => Ok(KirVectorBinaryOp::Subtract),
        (MirBinaryOp::Mul, _) => Ok(KirVectorBinaryOp::Multiply),
        (MirBinaryOp::Div, KirArithmeticSemantics::StrictFloat) => Ok(KirVectorBinaryOp::Divide),
        (MirBinaryOp::Div | MirBinaryOp::Mod, _) => {
            Err("failing vector binary operation is unsupported".to_string())
        }
    }
}

const fn vector_cast(op: crate::MirCastOp) -> KirVectorCastOp {
    match op {
        crate::MirCastOp::I32ToF64 => KirVectorCastOp::I32ToF64,
        crate::MirCastOp::U32ToF64 => KirVectorCastOp::U32ToF64,
    }
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}
