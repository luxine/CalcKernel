use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CandidateBudgetCharge, FactUseSite, KirArithmeticSemantics, KirBlock, KirBlockParam, KirEdge,
    KirEffectKind, KirInstruction, KirInstructionKind, KirLaneType, KirMemoryAccess,
    KirMemoryBlockParam, KirOrderedEffect, KirPlace, KirPreStateIdentity, KirResult, KirValueType,
    KirVectorBinaryOp, KirVectorCastOp, KirVectorMemoryAccess, KirVectorReductionOp,
    KirVectorRegion, KirVectorUnaryOp, KirVerifiedProgramState, KirVersionPredicate,
    KirVersionPredicateConjunct, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType,
    ProofStep, ProofStepId, ScalarClaim, ScalarFailure, ScalarInterval, VectorBroadcastGroup,
    VectorEpilogue, VectorLaneMapping, VectorMemoryAccessKind, VectorMemoryGroup,
    VectorOperationMapping, VectorPlanGrowth, VectorPredicate, VectorProofRoots,
    VectorizationCandidate, VectorizationPlan, WasmAffineAccessShape, WasmAffineShape,
    WasmRangeCount, kir_function_units,
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
    let wasm_affine = candidate.wasm_affine.as_ref();
    if wasm_affine.is_some()
        && !crate::optimizer::analysis::wasm_affine_loop_state_is_forwarded(
            &original,
            candidate.header,
            candidate.body,
            candidate.induction,
        )
    {
        return Err("WASM affine loop carries non-induction scalar state".to_string());
    }
    let matmul_interleave = wasm_affine.is_some_and(|affine| {
        crate::optimizer::analysis::wasm_matmul_interleave_eligible(
            affine,
            &candidate.operations,
            &candidate.accesses,
            candidate.bound,
        ) && crate::optimizer::analysis::wasm_matmul_interleave_source_is_closed(
            &original,
            candidate.header,
            candidate.body,
            candidate.induction,
            affine,
            &candidate.operations,
            &candidate.accesses,
            candidate.bound,
        )
    });
    if let Some(affine) = wasm_affine {
        let direct_map_interleave = direct_wasm_map_candidate(candidate, affine);
        if pre_state.module().config.consumer != crate::KirConsumer::WebAssembly
            || pre_state.module().profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
            || candidate.vf != 2
            || candidate.uf > 1 && !direct_map_interleave && !matmul_interleave
            || candidate.diamond.is_some()
            || candidate.reduction.is_some()
            || candidate.scalar_blocks.len() != 1
            || !(affine.range_requirements.len() == 3
                || direct_map_interleave && affine.range_requirements.len() == 2)
        {
            return Err("unsupported-wasm-affine-materialization-shape".to_string());
        }
        let affine_instruction_ids = affine
            .accesses
            .iter()
            .map(|access| access.instruction)
            .collect::<BTreeSet<_>>();
        let scalar_instruction_ids = candidate
            .accesses
            .iter()
            .map(|access| access.instruction)
            .collect::<BTreeSet<_>>();
        let setup_ids = affine
            .scalar_address_setup
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if affine_instruction_ids.len() != affine.accesses.len()
            || scalar_instruction_ids != affine_instruction_ids
            || setup_ids.len() != affine.scalar_address_setup.len()
            || setup_ids
                .iter()
                .any(|id| affine_instruction_ids.contains(id))
            || candidate.accesses.iter().any(|access| {
                access.element_bytes != 8
                    || access.element_type != MirType::Primitive(MirPrimitiveTypeName::F64)
            })
        {
            return Err("malformed-wasm-affine-access-partition".to_string());
        }
        for access in &affine.accesses {
            let source = original
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find(|instruction| instruction.id == access.instruction)
                .ok_or_else(|| "WASM affine access instruction is missing".to_string())?;
            match (&source.kind, &access.shape, access.kind) {
                (
                    KirInstructionKind::Load { .. },
                    WasmAffineShape::Broadcast { .. } | WasmAffineShape::Contiguous { .. },
                    crate::LoopMemoryAccessKind::Read,
                )
                | (
                    KirInstructionKind::Store { .. },
                    WasmAffineShape::Contiguous { .. },
                    crate::LoopMemoryAccessKind::Write,
                ) => {}
                _ => return Err("unsupported-wasm-affine-memory-access-kind".to_string()),
            }
        }
        for setup in &affine.scalar_address_setup {
            let source = original_body
                .instructions
                .iter()
                .find(|instruction| instruction.id == *setup)
                .ok_or_else(|| {
                    "WASM affine scalar address setup is outside the loop body".to_string()
                })?;
            if source.memory.is_some()
                || source.effect.is_some()
                || !(matches!(
                    source.kind,
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        semantics: KirArithmeticSemantics::Modular,
                        ..
                    }
                ) || affine
                    .range_requirements
                    .iter()
                    .any(|r| matches!(r.count, WasmRangeCount::ScaledInvariant { scale: 3, .. }))
                    && matches!(
                        source.kind,
                        KirInstructionKind::Compare { .. }
                            | KirInstructionKind::Binary {
                                op: MirBinaryOp::Sub,
                                semantics: KirArithmeticSemantics::Modular,
                                ..
                            }
                            | KirInstructionKind::ConstInt { .. }
                    ))
            {
                return Err("WASM affine address setup is not a pure modular add".to_string());
            }
        }
    }
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
    let persistent_add = pre_state.module().profile.wasm_features()
        == Some(crate::KirWasmFeatures::Simd128)
        && candidate.vf == 4
        && candidate.uf == 1
        && candidate.reduction.as_ref().is_some_and(|reduction| {
            reduction.binary_op == MirBinaryOp::Add
                && matches!(reduction.lane_type, KirLaneType::I32 | KirLaneType::U32)
        });
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
    let mut affine_vector_ends = BTreeMap::<Option<crate::ValueId>, crate::ValueId>::new();
    if let Some(affine) = wasm_affine {
        let contiguous_offsets = affine
            .accesses
            .iter()
            .filter_map(|access| match &access.shape {
                WasmAffineShape::Contiguous { offset } => Some(*offset),
                WasmAffineShape::Broadcast { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        for offset in contiguous_offsets {
            let end = if let Some(offset_value) = offset {
                let start = materialize_entry_value(
                    &original,
                    &original_header,
                    &original_preheader,
                    entry_edge,
                    offset_value,
                    &mut trial,
                    &mut transformed_preheader,
                )?;
                let end = trial.fresh_value()?;
                transformed_preheader.instructions.push(KirInstruction {
                    id: trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: end,
                        type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                    }],
                    kind: KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left: start,
                        right: entry_bound,
                        semantics: KirArithmeticSemantics::Modular,
                    },
                    memory: None,
                    effect: None,
                });
                end
            } else {
                entry_bound
            };
            affine_vector_ends.insert(offset, end);
        }
    }

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

    let compact_interleaved_body =
        candidate.uf > 1 && candidate.diamond.is_none() && candidate.reduction.is_none();
    let mut body_values = BTreeMap::new();
    let mut vector_body_params = Vec::new();
    for (index, param) in original_body.params.iter().enumerate() {
        let source = *body_edge
            .args
            .get(index)
            .ok_or_else(|| "vector candidate body argument is missing".to_string())?;
        let header_value = header_values
            .get(&source)
            .copied()
            .ok_or_else(|| "vector body value does not originate at the loop header".to_string())?;
        body_values.insert(source, header_value);
        if compact_interleaved_body {
            body_values.insert(param.value, header_value);
        } else {
            let value = trial.fresh_value()?;
            body_values.insert(param.value, value);
            vector_body_params.push(KirBlockParam {
                value,
                slot: format!("loop_simd_{}", param.slot),
                type_node: param.type_node.clone(),
            });
        }
    }
    // Keep the scalar seed unchanged across the vector loop. Each lane starts
    // at zero, and the seed is combined exactly once before the scalar tail.
    let vector_accumulator = if persistent_add {
        let reduction = candidate.reduction.as_ref().expect("persistent reduction");
        let scalar_zero = trial.fresh_value()?;
        let initial = trial.fresh_value()?;
        let header = trial.fresh_value()?;
        let body = trial.fresh_value()?;
        let vector_type = KirValueType::FixedVector {
            lane: reduction.lane_type,
            lanes: 4,
        };
        let scalar_type = original_header
            .params
            .iter()
            .find(|param| param.value == reduction.header_value)
            .ok_or_else(|| "vector reduction seed parameter is missing".to_string())?
            .type_node
            .clone();
        transformed_preheader.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value: scalar_zero,
                type_node: scalar_type,
            }],
            kind: KirInstructionKind::ConstInt {
                value: "0".to_string(),
            },
            memory: None,
            effect: None,
        });
        transformed_preheader.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value: initial,
                type_node: vector_type.clone(),
            }],
            kind: KirInstructionKind::VectorSplat {
                scalar: scalar_zero,
                region: vector_region,
            },
            memory: None,
            effect: None,
        });
        vector_header_params.push(KirBlockParam {
            value: header,
            slot: "loop_simd_accumulator".to_string(),
            type_node: vector_type.clone(),
        });
        vector_body_params.push(KirBlockParam {
            value: body,
            slot: "loop_simd_accumulator".to_string(),
            type_node: vector_type,
        });
        Some((initial, header, body))
    } else {
        None
    };
    let mut body_memories = BTreeMap::new();
    for (param, source) in original_body
        .memory_params
        .iter()
        .zip(&body_edge.memory_args)
    {
        let version = header_memories.get(source).copied().ok_or_else(|| {
            "vector body memory does not originate at the loop header".to_string()
        })?;
        body_memories.insert(param.version, version);
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
    let threshold = if candidate.version_predicate.is_none() && wasm_affine.is_none() {
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
        Some(threshold)
    } else {
        None
    };
    let vf_value = if candidate.minimum_trip == chunk_width {
        minimum_value
    } else {
        let value = trial.fresh_value()?;
        transformed_preheader.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value,
                type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
            }],
            kind: KirInstructionKind::ConstInt {
                value: chunk_width.to_string(),
            },
            memory: None,
            effect: None,
        });
        value
    };
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
    let entry_condition = if candidate.version_predicate.is_some() || wasm_affine.is_some() {
        let mut conjuncts = vec![KirVersionPredicateConjunct::TripThreshold {
            value: entry_bound,
            minimum: candidate.minimum_trip,
        }];
        if let Some(predicate) = &candidate.version_predicate {
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
                    return Err(
                        "vector runtime predicate contains an unsupported conjunct".to_string()
                    );
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
        }
        if let Some(affine) = wasm_affine {
            let final_count = conjuncts
                .len()
                .saturating_add(affine.range_requirements.len());
            if final_count > 4 {
                return Err("WASM affine predicate budget exceeds four conjuncts".to_string());
            }
            let mut constants = BTreeMap::new();
            for requirement in &affine.range_requirements {
                let slice = materialize_entry_value(
                    &original,
                    &original_header,
                    &original_preheader,
                    entry_edge,
                    requirement.slice,
                    &mut trial,
                    &mut transformed_preheader,
                )?;
                let start = if let Some(source_start) = requirement.start {
                    materialize_entry_value(
                        &original,
                        &original_header,
                        &original_preheader,
                        entry_edge,
                        source_start,
                        &mut trial,
                        &mut transformed_preheader,
                    )?
                } else {
                    preheader_u32_constant(
                        &mut trial,
                        &mut transformed_preheader,
                        &mut constants,
                        0,
                    )?
                };
                let count = match requirement.count {
                    WasmRangeCount::TripBound(source_count) => {
                        if source_count != candidate.bound {
                            return Err(
                                "WASM trip-bound footprint differs from the proven entry trip"
                                    .into(),
                            );
                        }
                        entry_bound
                    }
                    WasmRangeCount::Invariant(source_count) => materialize_entry_value(
                        &original,
                        &original_header,
                        &original_preheader,
                        entry_edge,
                        source_count,
                        &mut trial,
                        &mut transformed_preheader,
                    )?,
                    WasmRangeCount::ScaledInvariant { value, scale } => {
                        if scale != 3 {
                            return Err("unsupported affine invariant count scale".into());
                        }
                        let width = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            value,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        let factor = preheader_u32_constant(
                            &mut trial,
                            &mut transformed_preheader,
                            &mut constants,
                            scale,
                        )?;
                        let count = trial.fresh_value()?;
                        transformed_preheader.instructions.push(KirInstruction {
                            id: trial.fresh_instruction()?,
                            results: vec![KirResult {
                                value: count,
                                type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                            }],
                            kind: KirInstructionKind::Binary {
                                op: MirBinaryOp::Mul,
                                left: width,
                                right: factor,
                                semantics: crate::KirArithmeticSemantics::Modular,
                            },
                            memory: None,
                            effect: None,
                        });
                        count
                    }
                    WasmRangeCount::One => preheader_u32_constant(
                        &mut trial,
                        &mut transformed_preheader,
                        &mut constants,
                        1,
                    )?,
                };
                conjuncts.push(KirVersionPredicateConjunct::WasmSliceRange {
                    slice,
                    start,
                    count,
                    element_bytes: requirement.element_bytes,
                });
            }
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
                    address_bits: candidate
                        .version_predicate
                        .as_ref()
                        .map_or(32, |predicate| predicate.address_bits),
                    conjuncts,
                },
            },
            memory: None,
            effect: None,
        });
        condition
    } else {
        threshold.ok_or_else(|| "vector trip threshold predicate is missing".to_string())?
    };
    transformed_preheader.terminator = crate::KirTerminator::Branch {
        condition: entry_condition,
        then_edge: KirEdge {
            target: vector_header_id,
            args: entry_edge
                .args
                .iter()
                .copied()
                .chain(vector_accumulator.map(|(initial, _, _)| initial))
                .collect(),
            memory_args: entry_edge.memory_args.clone(),
        },
        else_edge: entry_edge.clone(),
    };

    let mut vector_compare = vector_compare;
    if let KirInstructionKind::Compare { right, .. } = &mut vector_compare.kind {
        *right = vector_limit;
    }
    let mut vector_header = KirBlock {
        id: vector_header_id,
        label: "loop_simd_header".to_string(),
        params: vector_header_params,
        memory_params: vector_header_memory,
        instructions: vec![vector_compare],
        terminator: crate::KirTerminator::Branch {
            condition: vector_condition,
            then_edge: KirEdge {
                target: vector_body_id,
                args: if compact_interleaved_body {
                    Vec::new()
                } else {
                    body_edge
                        .args
                        .iter()
                        .map(|value| {
                            header_values.get(value).copied().ok_or_else(|| {
                                "vector header body edge uses a non-parameter value".to_string()
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?
                },
                memory_args: Vec::new(),
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
    if let Some((_, accumulator, _)) = vector_accumulator {
        let crate::KirTerminator::Branch { then_edge, .. } = &mut vector_header.terminator else {
            unreachable!()
        };
        then_edge.args.push(accumulator);
    }

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
    let mut broadcast_records = Vec::new();
    let mut shared_broadcasts = BTreeMap::<crate::InstructionId, MappedValue>::new();
    let mut next_accumulator = None;
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

    let mut chunk_induction = base_mapped[&body_induction].value;
    let mut chunk_stride = None;
    for unroll_index in 0..candidate.uf {
        mapped = base_mapped.clone();
        if unroll_index != 0 {
            let stride = match chunk_stride {
                Some(stride) => stride,
                None => {
                    let stride = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: stride,
                            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                        }],
                        kind: KirInstructionKind::ConstInt {
                            value: candidate.vf.to_string(),
                        },
                        memory: None,
                        effect: None,
                    });
                    chunk_stride = Some(stride);
                    stride
                }
            };
            let next_chunk = trial.fresh_value()?;
            emitted.push(KirInstruction {
                id: trial.fresh_instruction()?,
                results: vec![KirResult {
                    value: next_chunk,
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                }],
                kind: KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left: chunk_induction,
                    right: stride,
                    semantics: KirArithmeticSemantics::Modular,
                },
                memory: None,
                effect: None,
            });
            chunk_induction = next_chunk;
            let mapped_induction = MappedValue {
                value: chunk_induction,
                vector: false,
            };
            mapped.insert(candidate.induction, mapped_induction);
            mapped.insert(body_induction, mapped_induction);
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
            if wasm_affine
                .is_some_and(|affine| affine.scalar_address_setup.contains(&instruction.id))
            {
                let kind = match &instruction.kind {
                    KirInstructionKind::Binary {
                        op,
                        left,
                        right,
                        semantics: KirArithmeticSemantics::Modular,
                    } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                        let left = resolve_value(&mapped, *left);
                        let right = resolve_value(&mapped, *right);
                        if left.vector || right.vector {
                            return Err("WASM affine address setup uses a vector operand".into());
                        }
                        KirInstructionKind::Binary {
                            op: *op,
                            left: left.value,
                            right: right.value,
                            semantics: KirArithmeticSemantics::Modular,
                        }
                    }
                    KirInstructionKind::Compare { op, left, right } => {
                        let left = resolve_value(&mapped, *left);
                        let right = resolve_value(&mapped, *right);
                        if left.vector || right.vector {
                            return Err("WASM scalar setup compare uses a vector operand".into());
                        }
                        KirInstructionKind::Compare {
                            op: *op,
                            left: left.value,
                            right: right.value,
                        }
                    }
                    KirInstructionKind::ConstInt { value } => KirInstructionKind::ConstInt {
                        value: value.clone(),
                    },
                    _ => return Err("WASM affine address setup changed shape".into()),
                };
                let result = scalar_result(instruction)?;
                let fresh = trial.fresh_value()?;
                emitted.push(KirInstruction {
                    id: trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: instruction.results[0].type_node.clone(),
                    }],
                    kind,
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
                continue;
            }
            match &instruction.kind {
                KirInstructionKind::ConstInt { value } => {
                    let result = scalar_result(instruction)?;
                    if instruction_uses_value(&original, candidate.induction_update, result)
                        && !value_has_use_outside_instruction(
                            &original,
                            result,
                            candidate.induction_update,
                        )
                    {
                        continue;
                    }
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
                KirInstructionKind::ConstFloat { value } => {
                    let result = scalar_result(instruction)?;
                    let fresh = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: instruction.results[0].type_node.clone(),
                        }],
                        kind: KirInstructionKind::ConstFloat {
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
                    let affine_access = wasm_affine.and_then(|affine| {
                        affine
                            .accesses
                            .iter()
                            .find(|candidate| candidate.instruction == instruction.id)
                    });
                    if let Some(WasmAffineAccessShape {
                        slice: affine_slice,
                        shape: WasmAffineShape::Broadcast { index },
                        ..
                    }) = affine_access
                    {
                        if matmul_interleave && unroll_index != 0 {
                            let shared = shared_broadcasts
                                .get(&instruction.id)
                                .copied()
                                .ok_or_else(|| {
                                    "shared matmul broadcast was not materialized by chunk zero"
                                        .to_string()
                                })?;
                            mapped.insert(result, shared);
                            continue;
                        }
                        let source_slice = place_slice(place)?;
                        let source_index = place_index(place)?;
                        let source_slice_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            source_slice,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        let affine_slice_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            *affine_slice,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        let source_index_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            source_index,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        let affine_index_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            *index,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        if source_slice_at_entry != affine_slice_at_entry
                            || source_index_at_entry != affine_index_at_entry
                        {
                            return Err(
                                "WASM affine broadcast source does not match its proof".to_string()
                            );
                        }
                        let slice = resolve_value(&mapped, source_slice);
                        let index = resolve_value(&mapped, source_index);
                        if slice.vector || index.vector {
                            return Err("WASM affine broadcast address became a vector".to_string());
                        }
                        let memory = map_memory(instruction, &mut body_memories, &mut trial)?;
                        let mut scalar_place = place.as_ref().clone();
                        let KirPlace::SliceIndex {
                            slice: scalar_slice,
                            index: scalar_index,
                            ..
                        } = &mut scalar_place
                        else {
                            return Err(
                                "WASM affine broadcast place is not a slice index".to_string()
                            );
                        };
                        *scalar_slice = slice.value;
                        *scalar_index = index.value;
                        let scalar = trial.fresh_value()?;
                        let scalar_load = trial.fresh_instruction()?;
                        emitted.push(KirInstruction {
                            id: scalar_load,
                            results: vec![KirResult {
                                value: scalar,
                                type_node: instruction.results[0].type_node.clone(),
                            }],
                            kind: KirInstructionKind::Load {
                                place: Box::new(scalar_place),
                            },
                            memory: Some(memory),
                            effect: Some(KirOrderedEffect {
                                order: next_effect,
                                kind: KirEffectKind::ReadMemory,
                            }),
                        });
                        next_effect = next_effect.saturating_add(1);
                        let splat = trial.fresh_value()?;
                        let splat_instruction = trial.fresh_instruction()?;
                        emitted.push(KirInstruction {
                            id: splat_instruction,
                            results: vec![KirResult {
                                value: splat,
                                type_node: KirValueType::FixedVector {
                                    lane,
                                    lanes: candidate.vf,
                                },
                            }],
                            kind: KirInstructionKind::VectorSplat {
                                scalar,
                                region: vector_region,
                            },
                            memory: None,
                            effect: None,
                        });
                        let broadcast_value = MappedValue {
                            value: splat,
                            vector: true,
                        };
                        mapped.insert(result, broadcast_value);
                        if matmul_interleave {
                            shared_broadcasts.insert(instruction.id, broadcast_value);
                        }
                        broadcast_records.push((
                            instruction.id,
                            unroll_index,
                            scalar_load,
                            splat_instruction,
                            access.region,
                        ));
                        continue;
                    }
                    let fresh = trial.fresh_value()?;
                    let id = trial.fresh_instruction()?;
                    let memory = map_memory(instruction, &mut body_memories, &mut trial)?;
                    let (slice, start, end) = if let Some(affine_access) = affine_access {
                        let WasmAffineShape::Contiguous { offset } = affine_access.shape else {
                            return Err("WASM affine load has an unsupported shape".to_string());
                        };
                        let source_slice = place_slice(place)?;
                        let source_slice_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            source_slice,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        let affine_slice_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            affine_access.slice,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        if source_slice_at_entry != affine_slice_at_entry {
                            return Err(
                                "WASM affine load slice does not match its proof".to_string()
                            );
                        }
                        let start = resolve_value(&mapped, place_index(place)?);
                        if start.vector {
                            return Err("WASM affine load address became a vector".to_string());
                        }
                        let end = affine_vector_ends
                            .get(&offset)
                            .copied()
                            .ok_or_else(|| "WASM affine load range end is missing".to_string())?;
                        (
                            vector_slice_origin(&original, access, memory.region)?,
                            start.value,
                            end,
                        )
                    } else {
                        (
                            invariant_root_value(&original, place_slice(place)?)?,
                            resolve_value(&mapped, place_index(place)?).value,
                            entry_bound,
                        )
                    };
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
                                start,
                                end,
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
                    let affine_access = wasm_affine.and_then(|affine| {
                        affine
                            .accesses
                            .iter()
                            .find(|candidate| candidate.instruction == instruction.id)
                    });
                    let (slice, start, end) = if let Some(affine_access) = affine_access {
                        let WasmAffineShape::Contiguous { offset } = &affine_access.shape else {
                            return Err("WASM affine store has an unsupported shape".to_string());
                        };
                        let source_slice = place_slice(place)?;
                        let source_slice_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            source_slice,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        let affine_slice_at_entry = materialize_entry_value(
                            &original,
                            &original_header,
                            &original_preheader,
                            entry_edge,
                            affine_access.slice,
                            &mut trial,
                            &mut transformed_preheader,
                        )?;
                        if source_slice_at_entry != affine_slice_at_entry {
                            return Err(
                                "WASM affine store slice does not match its proof".to_string()
                            );
                        }
                        let start = resolve_value(&mapped, place_index(place)?);
                        let slice = resolve_value(&mapped, source_slice);
                        if start.vector || slice.vector {
                            return Err("WASM affine store address became a vector".to_string());
                        }
                        let end = affine_vector_ends
                            .get(offset)
                            .copied()
                            .ok_or_else(|| "WASM affine store range end is missing".to_string())?;
                        (
                            vector_slice_origin(&original, access, memory.region)?,
                            start.value,
                            end,
                        )
                    } else {
                        (
                            invariant_root_value(&original, place_slice(place)?)?,
                            resolve_value(&mapped, place_index(place)?).value,
                            entry_bound,
                        )
                    };
                    emitted.push(KirInstruction {
                        id,
                        results: Vec::new(),
                        kind: KirInstructionKind::VectorStore {
                            access: vector_memory_access(
                                slice,
                                start,
                                end,
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
                    let step = match chunk_stride {
                        Some(step) => step,
                        None => {
                            let step = trial.fresh_value()?;
                            emitted.push(KirInstruction {
                                id: trial.fresh_instruction()?,
                                results: vec![KirResult {
                                    value: step,
                                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                                }],
                                kind: KirInstructionKind::ConstInt {
                                    value: candidate.vf.to_string(),
                                },
                                memory: None,
                                effect: None,
                            });
                            chunk_stride = Some(step);
                            step
                        }
                    };
                    let fresh = trial.fresh_value()?;
                    emitted.push(KirInstruction {
                        id: trial.fresh_instruction()?,
                        results: vec![KirResult {
                            value: fresh,
                            type_node: instruction.results[0].type_node.clone(),
                        }],
                        kind: KirInstructionKind::Binary {
                            op: *op,
                            left: chunk_induction,
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
                    if let Some((_, _, carried)) = vector_accumulator {
                        let fresh = trial.fresh_value()?;
                        let id = trial.fresh_instruction()?;
                        emitted.push(KirInstruction {
                            id,
                            results: vec![KirResult {
                                value: fresh,
                                type_node: KirValueType::FixedVector {
                                    lane: reduction.lane_type,
                                    lanes: 4,
                                },
                            }],
                            kind: KirInstructionKind::VectorBinary {
                                op: KirVectorBinaryOp::Add,
                                left: carried,
                                right: lane_source.value,
                                semantics: KirArithmeticSemantics::Modular,
                                no_failure_proof: None,
                                region: vector_region,
                            },
                            memory: None,
                            effect: None,
                        });
                        next_accumulator = Some(fresh);
                        mapped.insert(scalar_result(instruction)?, accumulator);
                        operation_mappings.push((operation.clone(), unroll_index, id));
                        continue;
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
        memory_params: Vec::new(),
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
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .chain(next_accumulator)
                    .collect(),
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

    let finalizer = if let Some((_, accumulator, _)) = vector_accumulator {
        let reduction = candidate.reduction.as_ref().expect("persistent reduction");
        let finalizer_id = trial.fresh_block()?;
        let reduced = trial.fresh_value()?;
        let combined = trial.fresh_value()?;
        let seed_index = original_header
            .params
            .iter()
            .position(|param| param.value == reduction.header_value)
            .ok_or_else(|| "vector reduction seed parameter is missing".to_string())?;
        let scalar_type = original_header.params[seed_index].type_node.clone();
        let crate::KirTerminator::Branch { else_edge, .. } = &mut vector_header.terminator else {
            unreachable!()
        };
        let mut tail_edge = else_edge.clone();
        let seed = tail_edge.args[seed_index];
        tail_edge.args[seed_index] = combined;
        *else_edge = KirEdge {
            target: finalizer_id,
            args: Vec::new(),
            memory_args: Vec::new(),
        };
        Some(KirBlock {
            id: finalizer_id,
            label: "loop_simd_finalize".to_string(),
            params: Vec::new(),
            memory_params: Vec::new(),
            instructions: vec![
                KirInstruction {
                    id: trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: reduced,
                        type_node: scalar_type.clone(),
                    }],
                    kind: KirInstructionKind::VectorReduce {
                        op: KirVectorReductionOp::ModularAdd,
                        vector: accumulator,
                        semantics: KirArithmeticSemantics::Modular,
                        region: vector_region,
                    },
                    memory: None,
                    effect: None,
                },
                KirInstruction {
                    id: trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: combined,
                        type_node: scalar_type,
                    }],
                    kind: KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left: seed,
                        right: reduced,
                        semantics: KirArithmeticSemantics::Modular,
                    },
                    memory: None,
                    effect: None,
                },
            ],
            terminator: crate::KirTerminator::Jump { edge: tail_edge },
        })
    } else {
        None
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
        blocks: finalizer.as_ref().map_or_else(
            || vec![vector_body_id],
            |block| {
                vec![
                    candidate.preheader,
                    vector_header_id,
                    vector_body_id,
                    block.id,
                ]
            },
        ),
    });
    transformed.blocks.push(vector_header);
    transformed.blocks.push(vector_body);
    transformed.blocks.extend(finalizer);

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
    if let Some(affine) = wasm_affine {
        plan_predicates.extend(
            affine
                .range_requirements
                .iter()
                .copied()
                .map(|requirement| VectorPredicate::WasmSliceRange {
                    requirement,
                    proof: roots.target_legality,
                }),
        );
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
        if wasm_affine.is_some_and(|affine| {
            affine.accesses.iter().any(|candidate| {
                candidate.instruction == expected.instruction
                    && matches!(candidate.shape, WasmAffineShape::Broadcast { .. })
            })
        }) {
            continue;
        }
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
    let broadcast_groups = broadcast_records
        .iter()
        .map(
            |(scalar_instruction, unroll_index, emitted_scalar_load, emitted_splat, region)| {
                VectorBroadcastGroup {
                    region: *region,
                    scalar_instruction: *scalar_instruction,
                    emitted_scalar_load: *emitted_scalar_load,
                    emitted_splat: *emitted_splat,
                    unroll_index: *unroll_index,
                    footprint_proof: roots.operation_equivalence,
                }
            },
        )
        .collect();
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
        broadcast_groups,
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

fn direct_wasm_map_candidate(
    candidate: &VectorizationCandidate,
    affine: &crate::WasmAffineCandidate,
) -> bool {
    if !matches!(candidate.uf, 1 | 2 | 4)
        || affine.accesses.len() != 2
        || !affine.scalar_address_setup.is_empty()
        || affine.range_requirements.len() != 2
        || candidate.accesses.len() != 2
        || candidate.operations.is_empty()
        || candidate.operations.iter().any(|operation| {
            operation.lane_type != KirLaneType::F64
                || operation.result_lane_type != KirLaneType::F64
                || operation.semantics != crate::KirCostSemantics::StrictFloat
                || !matches!(
                    operation.operation,
                    crate::KirProfileOperation::Add
                        | crate::KirProfileOperation::Subtract
                        | crate::KirProfileOperation::Multiply
                        | crate::KirProfileOperation::Divide
                        | crate::KirProfileOperation::Negate
                )
        })
    {
        return false;
    }
    let Some(input) = affine.accesses.iter().find(|access| {
        access.kind == crate::LoopMemoryAccessKind::Read
            && matches!(access.shape, WasmAffineShape::Contiguous { offset: None })
    }) else {
        return false;
    };
    let Some(output) = affine.accesses.iter().find(|access| {
        access.kind == crate::LoopMemoryAccessKind::Write
            && matches!(access.shape, WasmAffineShape::Contiguous { offset: None })
    }) else {
        return false;
    };
    input.slice != output.slice
        && affine
            .accesses
            .iter()
            .all(|access| matches!(access.shape, WasmAffineShape::Contiguous { offset: None }))
        && candidate.accesses.iter().all(|access| {
            access.element_bytes == 8
                && access.element_type == MirType::Primitive(MirPrimitiveTypeName::F64)
        })
        && affine.range_requirements.iter().all(|range| {
            range.element_bytes == 8
                && range.start.is_none()
                && range.count == WasmRangeCount::TripBound(candidate.bound)
                && (range.slice == input.slice || range.slice == output.slice)
        })
        && affine
            .range_requirements
            .iter()
            .map(|range| range.slice)
            .collect::<BTreeSet<_>>()
            == BTreeSet::from([input.slice, output.slice])
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

fn instruction_uses_value(
    function: &crate::KirFunction,
    instruction_id: crate::InstructionId,
    value: crate::ValueId,
) -> bool {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == instruction_id)
        .is_some_and(|instruction| {
            let mut used = false;
            crate::visit_instruction_uses(instruction, &mut |candidate| {
                used |= candidate == value;
            });
            used
        })
}

fn value_has_use_outside_instruction(
    function: &crate::KirFunction,
    value: crate::ValueId,
    excluded: crate::InstructionId,
) -> bool {
    function.blocks.iter().any(|block| {
        block.instructions.iter().any(|instruction| {
            if instruction.id == excluded {
                return false;
            }
            let mut used = false;
            crate::visit_instruction_uses(instruction, &mut |candidate| {
                used |= candidate == value;
            });
            used
        }) || match &block.terminator {
            crate::KirTerminator::Return { value: result, .. } => *result == Some(value),
            crate::KirTerminator::Jump { edge } => edge.args.contains(&value),
            crate::KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                *condition == value
                    || then_edge.args.contains(&value)
                    || else_edge.args.contains(&value)
            }
        }
    })
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
    let memory_steps = plan.memory_groups.iter().fold(0_u32, |total, group| {
        total.saturating_add(u32::try_from(group.scalar_instructions.len()).unwrap_or(u32::MAX))
    });
    let broadcasts = u32::try_from(plan.broadcast_groups.len()).unwrap_or(u32::MAX);
    let memory = memory_steps.saturating_add(broadcasts);
    let operations = u32::try_from(plan.operations.len()).unwrap_or(u32::MAX);
    let groups = u32::try_from(plan.memory_groups.len())
        .unwrap_or(u32::MAX)
        .saturating_add(broadcasts);
    let predicates = u32::try_from(plan.predicates.len()).unwrap_or(u32::MAX);
    CandidateBudgetCharge::single(
        plan.pre_state.function,
        8_u32
            .saturating_add(operations.saturating_mul(4))
            .saturating_add(lanes)
            .saturating_add(groups.saturating_mul(4))
            .saturating_add(memory)
            .saturating_add(predicates.saturating_mul(3))
            .saturating_add(broadcasts.saturating_mul(2))
            .saturating_add(2),
        16_u32
            .saturating_add(operations.saturating_mul(6))
            .saturating_add(lanes.saturating_mul(2))
            .saturating_add(groups.saturating_mul(6))
            .saturating_add(memory.saturating_mul(2))
            .saturating_add(predicates.saturating_mul(4))
            .saturating_add(broadcasts.saturating_mul(3))
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

fn stable_invariant_root_value(
    function: &crate::KirFunction,
    value: crate::ValueId,
) -> Result<crate::ValueId, String> {
    fn visit(
        function: &crate::KirFunction,
        value: crate::ValueId,
        visiting: &mut BTreeSet<crate::ValueId>,
    ) -> Result<Option<crate::ValueId>, ()> {
        if function.params.iter().any(|param| param.value == value) {
            return Ok(Some(value));
        }
        if !visiting.insert(value) {
            return Ok(None);
        }
        let result = if let Some((block_id, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            let mut incoming_count = 0_usize;
            let mut roots = BTreeSet::new();
            for predecessor in &function.blocks {
                let edges = match &predecessor.terminator {
                    crate::KirTerminator::Jump { edge } if edge.target == block_id => {
                        vec![edge]
                    }
                    crate::KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => [then_edge, else_edge]
                        .into_iter()
                        .filter(|edge| edge.target == block_id)
                        .collect(),
                    _ => Vec::new(),
                };
                for edge in edges {
                    incoming_count = incoming_count.saturating_add(1);
                    let source = *edge.args.get(index).ok_or(())?;
                    if let Some(root) = visit(function, source, visiting)? {
                        roots.insert(root);
                    }
                }
            }
            if incoming_count == 0 {
                Err(())
            } else {
                match roots.len() {
                    0 => Ok(None),
                    1 => Ok(roots.first().copied()),
                    _ => Err(()),
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
        {
            visit(function, source, visiting)
        } else {
            Err(())
        };
        visiting.remove(&value);
        result
    }

    visit(function, value, &mut BTreeSet::new())
        .map_err(|()| {
            "vector slice-length descriptor is not invariant across the loop".to_string()
        })?
        .ok_or_else(|| {
            "vector slice-length descriptor is not invariant across the loop".to_string()
        })
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
    materialize_entry_value_inner(
        function,
        header,
        preheader,
        entry,
        value,
        trial,
        transformed_preheader,
        &mut BTreeSet::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_entry_value_inner(
    function: &crate::KirFunction,
    header: &KirBlock,
    preheader: &KirBlock,
    entry: &KirEdge,
    value: crate::ValueId,
    trial: &mut KirVerifiedProgramState,
    transformed_preheader: &mut KirBlock,
    seen: &mut BTreeSet<crate::ValueId>,
) -> Result<crate::ValueId, String> {
    if !seen.insert(value) {
        return Err("cyclic loop value forwarding cannot be materialized".to_string());
    }
    if let Some(index) = header.params.iter().position(|param| param.value == value) {
        return entry
            .args
            .get(index)
            .copied()
            .ok_or_else(|| "vector entry edge is incomplete".to_string());
    }
    // Values used by the loop body often appear there as block parameters even
    // when they are just forwarded from the header. Resolve that forwarding
    // back through the loop's preheader edge before deciding that the value is
    // unavailable in the versioning preheader.
    if let crate::KirTerminator::Branch { then_edge, .. } = &header.terminator
        && let Some(body) = function
            .blocks
            .iter()
            .find(|body| body.id == then_edge.target)
        && let Some(index) = body.params.iter().position(|param| param.value == value)
    {
        let source_value = then_edge
            .args
            .get(index)
            .copied()
            .ok_or_else(|| "vector loop body edge is incomplete".to_string())?;
        if source_value != value {
            return materialize_entry_value_inner(
                function,
                header,
                preheader,
                entry,
                source_value,
                trial,
                transformed_preheader,
                seen,
            );
        }
    }
    if function.params.iter().any(|param| param.value == value)
        || preheader.params.iter().any(|param| param.value == value)
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
    {
        let legal_type = instruction.results.iter().any(|result| {
            result.value == value
                && matches!(
                    &result.type_node,
                    KirValueType::Scalar(
                        MirType::Primitive(MirPrimitiveTypeName::U32) | MirType::Slice(_)
                    )
                )
        });
        let definition_block = function.blocks.iter().find(|block| {
            block
                .instructions
                .iter()
                .any(|candidate| candidate.id == instruction.id)
        });
        if legal_type
            && definition_block.is_some_and(|definition| {
                crate::compute_kir_dominators(function).dominates(definition.id, preheader.id)
            })
        {
            // Reuse an SSA value whose original definition is guaranteed to
            // execute before the versioning guard. Recomputing it in this
            // preheader could reorder evaluation or change trap behavior.
            return Ok(value);
        }
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
        && let KirInstructionKind::Copy { value: source } = instruction.kind
    {
        return materialize_entry_value_inner(
            function,
            header,
            preheader,
            entry,
            source,
            trial,
            transformed_preheader,
            seen,
        );
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
        && let KirInstructionKind::SliceLen { slice } = instruction.kind
    {
        if instruction.results.len() != 1
            || instruction.memory.is_some()
            || instruction.effect.is_some()
        {
            return Err("vector slice-length projection is not a pure scalar value".to_string());
        }
        let root = stable_invariant_root_value(function, slice)?;
        let fresh = trial.fresh_value()?;
        transformed_preheader.instructions.push(KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value: fresh,
                type_node: instruction.results[0].type_node.clone(),
            }],
            kind: KirInstructionKind::SliceLen { slice: root },
            memory: None,
            effect: None,
        });
        return Ok(fresh);
    }
    Err("vector loop bound does not dominate the versioning preheader".to_string())
}

fn preheader_u32_constant(
    trial: &mut KirVerifiedProgramState,
    preheader: &mut KirBlock,
    cache: &mut BTreeMap<u32, crate::ValueId>,
    constant: u32,
) -> Result<crate::ValueId, String> {
    if let Some(value) = cache.get(&constant) {
        return Ok(*value);
    }
    let value = trial.fresh_value()?;
    preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::ConstInt {
            value: constant.to_string(),
        },
        memory: None,
        effect: None,
    });
    cache.insert(constant, value);
    Ok(value)
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

fn vector_slice_origin(
    function: &crate::KirFunction,
    access: &crate::AffineMemoryAccess,
    memory_region: crate::MemoryRegionId,
) -> Result<crate::ValueId, String> {
    let descriptor = function
        .regions
        .iter()
        .find(|region| region.id == access.source_region)
        .ok_or_else(|| "WASM affine source slice region is missing".to_string())?;
    if descriptor.partition != memory_region {
        return Err("WASM affine source slice partition differs from MemorySSA".to_string());
    }
    let value = match &descriptor.origin {
        crate::KirMemoryRegionOrigin::Parameter(value)
        | crate::KirMemoryRegionOrigin::RawSlice(value)
        | crate::KirMemoryRegionOrigin::Subslice(value) => *value,
        crate::KirMemoryRegionOrigin::Conservative => {
            return Err("WASM affine vector access lacks a descriptor-origin slice".to_string());
        }
    };
    let expected_type = MirType::Slice(Box::new(access.element_type.clone()));
    let actual_type = function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| &param.type_node)
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .flat_map(|instruction| &instruction.results)
                .find(|result| result.value == value)
                .and_then(|result| result.type_node.as_scalar())
        });
    if actual_type != Some(&expected_type) {
        return Err("WASM affine descriptor-origin slice has the wrong element type".to_string());
    }
    Ok(value)
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

#[cfg(test)]
mod wasm_affine_materialization_tests {
    use super::*;

    const SOURCE: &str = r#"
export unsafe fn affine_update(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32, offset: u32, column: u32) -> void
contract {
  requires offset + n <= b.len && offset + n <= out.len && column < a.len;
  requires noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let index: u32 = offset + i;
    let previous: f64 = out[index];
    let scalar: f64 = a[column];
    let varying: f64 = b[index];
    out[index] = previous + scalar * varying;
    i = i + 1;
  }
}
"#;

    const NESTED_MATMUL_COLUMN: &str = r#"
export unsafe fn matmul_column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        let previous: f64 = out[out_index];
        let scalar: f64 = a[a_index];
        let varying: f64 = b[b_index];
        out[out_index] = previous + scalar * varying;
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

    fn pre_state_for(
        source: &str,
        level: crate::KirOptimizationLevel,
        late_contracts: bool,
    ) -> KirVerifiedProgramState {
        let checked = crate::check(&crate::SourceFile::new("wasm-affine.ck", source));
        assert_eq!(checked.diagnostics, []);
        let mir = crate::lower_to_mir(&checked.checked_program).expect("valid MIR");
        let module = crate::build_kir_module_with_profile(
            &mir,
            crate::KirBuildConfig {
                consumer: crate::KirConsumer::WebAssembly,
                overflow_mode: crate::KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
            crate::KirTargetProfile::webassembly_with_features(crate::KirWasmFeatures::Simd128),
        )
        .expect("SIMD128 KIR");
        let contracts = crate::import_contract_facts(&module, &checked.checked_program, 0)
            .expect("contract facts");
        let optimized =
            crate::run_kir_pass_pipeline(module, level, (!late_contracts).then_some(&contracts));
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        let scalar = optimized.artifact.expect("optimized KIR");
        let facts = if late_contracts {
            Some(
                crate::import_contract_facts(&scalar, &checked.checked_program, 0)
                    .expect("late source contracts"),
            )
        } else {
            optimized.contract_facts
        };
        KirVerifiedProgramState::from_parts(
            scalar,
            facts,
            optimized.proofs,
            optimized.eliminated_guards,
            0,
        )
        .expect("verified pre-state")
    }

    fn pre_state() -> KirVerifiedProgramState {
        pre_state_for(SOURCE, crate::KirOptimizationLevel::O2, false)
    }

    fn assert_trial_is_valid(trial: &KirVerifiedProgramState) {
        let validation = crate::validate_kir_optimization_evidence(
            trial.module(),
            trial.contract_facts(),
            trial.proofs(),
            trial.eliminated_guards(),
            trial.evidence_generation(),
        );
        assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    }

    #[test]
    fn affine_materializer_should_emit_two_contiguous_ranges_and_an_in_body_broadcast() {
        let state = pre_state();
        let discovery = crate::optimizer::analysis::discover_vectorization_candidates(&state);
        assert!(
            !discovery.candidates.is_empty(),
            "affine discovery should explain candidate rejection: {:?}",
            discovery.fallbacks
        );
        let candidate = discovery
            .candidates
            .iter()
            .find(|candidate| {
                state.module().functions.iter().any(|function| {
                    function.id == candidate.function && function.name == "affine_update"
                }) && candidate.vf == 2
                    && candidate.uf == 1
            })
            .expect("strict f64 affine candidate");
        let affine = candidate
            .wasm_affine
            .as_ref()
            .expect("analysis records affine and broadcast accesses");
        assert_eq!(affine.range_requirements.len(), 3);

        let prepared = materialize_vectorization_trial(&state, candidate)
            .expect("candidate materializes before independent checking");
        assert_trial_is_valid(&prepared.trial);
        assert_eq!(prepared.plan.uf, 1);
        assert_eq!(prepared.plan.broadcast_groups.len(), 1);
        assert_eq!(
            prepared
                .plan
                .predicates
                .iter()
                .filter(|predicate| matches!(predicate, VectorPredicate::WasmSliceRange { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn nested_affine_materializer_should_reuse_outer_preheader_ssa_for_slice_and_offsets() {
        // Production O3 now accepts this candidate. Keep a scalar O3 shape by
        // attaching the same contracts after scalar optimization for this
        // direct materializer test.
        let state = pre_state_for(NESTED_MATMUL_COLUMN, crate::KirOptimizationLevel::O3, true);
        let discovery = crate::optimizer::analysis::discover_vectorization_candidates(&state);
        let candidate = discovery
            .candidates
            .iter()
            .find(|candidate| candidate.wasm_affine.is_some())
            .unwrap_or_else(|| panic!("nested affine discovery failed: {:?}", discovery.fallbacks));
        let prepared = materialize_vectorization_trial(&state, candidate).unwrap_or_else(|error| {
            panic!("nested affine materialization failed: {error}; candidate={candidate:#?}")
        });
        assert_trial_is_valid(&prepared.trial);
        assert_eq!(prepared.plan.broadcast_groups.len(), 1);
        assert_eq!(
            prepared
                .plan
                .predicates
                .iter()
                .filter(|predicate| matches!(predicate, VectorPredicate::WasmSliceRange { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn nested_matmul_materializer_should_share_one_broadcast_across_four_vector_chunks() {
        let state = pre_state_for(NESTED_MATMUL_COLUMN, crate::KirOptimizationLevel::O3, true);
        let discovery = crate::optimizer::analysis::discover_vectorization_candidates(&state);
        let candidate = discovery
            .candidates
            .iter()
            .find(|candidate| candidate.vf == 2 && candidate.uf == 4)
            .unwrap_or_else(|| {
                panic!(
                    "strict nested matmul should expose VF2/UF4: {:?}",
                    discovery.fallbacks
                )
            });
        let affine = candidate.wasm_affine.as_ref().expect("affine matmul proof");
        assert_eq!(affine.accesses.len(), 4);
        assert_eq!(affine.scalar_address_setup.len(), 2);
        assert_eq!(affine.range_requirements.len(), 3);

        let prepared = materialize_vectorization_trial(&state, candidate)
            .expect("strict matmul VF2/UF4 materialization");
        assert_trial_is_valid(&prepared.trial);
        crate::check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .expect("strict matmul VF2/UF4 independent proof check");
        assert_eq!((prepared.plan.vf, prepared.plan.uf), (2, 4));
        assert_eq!(prepared.plan.broadcast_groups.len(), 1);
        assert_eq!(prepared.plan.broadcast_groups[0].unroll_index, 0);
        assert_eq!(prepared.plan.memory_groups.len(), 12);
        assert_eq!(prepared.plan.operations.len(), 8);
        assert_eq!(
            prepared
                .plan
                .predicates
                .iter()
                .filter(|predicate| matches!(predicate, VectorPredicate::WasmSliceRange { .. }))
                .count(),
            3
        );

        let function = prepared
            .trial
            .module()
            .functions
            .iter()
            .find(|function| function.id == candidate.function)
            .expect("trial function");
        let vector_body = function
            .blocks
            .iter()
            .find(|block| {
                function
                    .vector_regions
                    .iter()
                    .any(|region| region.blocks.contains(&block.id))
                    && block.instructions.iter().any(|instruction| {
                        instruction.id == prepared.plan.broadcast_groups[0].emitted_splat
                    })
            })
            .expect("vector loop body");
        let splat = prepared.plan.broadcast_groups[0].emitted_splat;
        let splat_value = vector_body
            .instructions
            .iter()
            .find(|instruction| instruction.id == splat)
            .and_then(|instruction| instruction.results.first())
            .expect("shared A splat result")
            .value;
        assert_eq!(
            vector_body
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
                .count(),
            1,
            "the invariant A value is loaded once for the UF4 bundle"
        );
        assert_eq!(
            vector_body
                .instructions
                .iter()
                .filter(|instruction| {
                    matches!(instruction.kind, KirInstructionKind::VectorSplat { .. })
                })
                .count(),
            1,
            "one A splat feeds all four vector chunks"
        );
        let vector_multiplies = prepared
            .plan
            .operations
            .iter()
            .filter(|operation| operation.operation == crate::KirProfileOperation::Multiply)
            .map(|operation| operation.vector)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(vector_multiplies.len(), 4);
        assert_eq!(
            vector_body
                .instructions
                .iter()
                .filter(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::VectorBinary {
                            op: KirVectorBinaryOp::Multiply,
                            ..
                        }
                    )
                })
                .count(),
            4
        );
        for instruction in &vector_body.instructions {
            if vector_multiplies.contains(&instruction.id) {
                assert!(matches!(
                    instruction.kind,
                    KirInstructionKind::VectorBinary { left, right, .. }
                        if left == splat_value || right == splat_value
                ));
            }
        }
    }

    #[test]
    fn nested_matmul_o3_pipeline_should_commit_the_shared_broadcast_uf4_shape() {
        let checked = crate::check(&crate::SourceFile::new(
            "nested-matmul-uf4.ck",
            NESTED_MATMUL_COLUMN,
        ));
        assert_eq!(checked.diagnostics, []);
        let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR");
        let profile =
            crate::KirTargetProfile::webassembly_with_features(crate::KirWasmFeatures::Simd128);
        let module = crate::build_kir_module_with_profile(
            &mir,
            crate::KirBuildConfig {
                consumer: crate::KirConsumer::WebAssembly,
                overflow_mode: crate::KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
            profile,
        )
        .expect("SIMD128 KIR");
        let contracts = crate::import_contract_facts(&module, &checked.checked_program, 0)
            .expect("source contracts");
        let optimized =
            crate::run_kir_pass_pipeline(module, crate::KirOptimizationLevel::O3, Some(&contracts));
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        assert_eq!(
            optimized.stats.vectorized_loops, 1,
            "{:#?}",
            optimized.stats
        );
        let function = optimized
            .artifact
            .as_ref()
            .and_then(|module| {
                module
                    .functions
                    .iter()
                    .find(|function| function.name == "matmul_column")
            })
            .expect("optimized matmul function");
        assert_eq!(function.vector_regions.len(), 1);
        let body = function
            .blocks
            .iter()
            .find(|block| block.label == "loop_simd_body")
            .expect("SIMD matmul body");
        assert_eq!(
            body.instructions
                .iter()
                .filter(|instruction| matches!(
                    instruction.kind,
                    KirInstructionKind::VectorLoad { .. } | KirInstructionKind::VectorStore { .. }
                ))
                .count(),
            12
        );
        assert_eq!(
            body.instructions
                .iter()
                .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
                .count(),
            1
        );
        assert_eq!(
            body.instructions
                .iter()
                .filter(|instruction| matches!(
                    instruction.kind,
                    KirInstructionKind::VectorSplat { .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            body.instructions
                .iter()
                .filter(|instruction| matches!(
                    instruction.kind,
                    KirInstructionKind::VectorBinary {
                        op: KirVectorBinaryOp::Multiply,
                        ..
                    }
                ))
                .count(),
            4
        );
        assert_eq!(
            body.instructions
                .iter()
                .filter(|instruction| matches!(
                    instruction.kind,
                    KirInstructionKind::VectorBinary {
                        op: KirVectorBinaryOp::Add,
                        ..
                    }
                ))
                .count(),
            4
        );
    }
}
