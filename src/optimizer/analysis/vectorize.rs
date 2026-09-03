use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, CandidateKey, CanonicalLoopDescriptor, FunctionId, InstructionId, KirAlignmentClass,
    KirArithmeticSemantics, KirCostEstimate, KirCostKey, KirCostSemantics, KirInstruction,
    KirInstructionKind, KirLaneType, KirOperationAvailability, KirProfileOperation,
    LoopCandidateKind, LoopCandidateVariant, LoopId, LoopTripCount, MemoryVersionId, MirBinaryOp,
    MirCompareOp, MirPrimitiveTypeName, MirType, MirUnaryOp,
};

use super::{
    AffineMemoryAccess, IntegerType, analyze_affine_loop_accesses,
    analyze_canonical_loops_for_discovery, analyze_loop_legality,
};
use crate::optimizer::KirVerifiedProgramState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorCandidateOperation {
    pub scalar: InstructionId,
    pub operation: KirProfileOperation,
    pub lane_type: KirLaneType,
    pub result_lane_type: KirLaneType,
    pub semantics: KirCostSemantics,
    pub alignment: KirAlignmentClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorizationCandidate {
    pub key: CandidateKey,
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub preheader: BlockId,
    pub header: BlockId,
    pub body: BlockId,
    pub latch: BlockId,
    pub exit: BlockId,
    pub scalar_blocks: Vec<BlockId>,
    pub diamond: Option<VectorDiamond>,
    pub predicated_update: Option<VectorPredicatedUpdate>,
    pub reduction: Option<VectorReduction>,
    pub induction: crate::ValueId,
    pub bound: crate::ValueId,
    pub induction_update: InstructionId,
    pub vf: u16,
    pub uf: u8,
    pub minimum_trip: u32,
    pub operations: Vec<VectorCandidateOperation>,
    pub accesses: Vec<AffineMemoryAccess>,
    pub version_predicate: Option<super::TotalVersionPredicate>,
    pub predicted_cost: KirCostEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorDiamond {
    pub then_block: BlockId,
    pub else_block: BlockId,
    pub merge_block: BlockId,
    pub condition: crate::ValueId,
    pub condition_instruction: InstructionId,
    pub selected_param_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorPredicatedUpdate {
    pub then_block: BlockId,
    pub merge_block: BlockId,
    pub condition_instruction: InstructionId,
    pub old_load_instruction: InstructionId,
    pub store_instruction: InstructionId,
    pub store_when_true: bool,
    pub memory_input: MemoryVersionId,
    pub memory_output: MemoryVersionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorReduction {
    pub header_value: crate::ValueId,
    pub body_value: crate::ValueId,
    pub instruction: InstructionId,
    pub operation: KirProfileOperation,
    pub binary_op: MirBinaryOp,
    pub lane_type: KirLaneType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorizationFallback {
    pub function: FunctionId,
    pub loop_id: Option<LoopId>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VectorizationDiscovery {
    pub candidates: Vec<VectorizationCandidate>,
    pub fallbacks: Vec<VectorizationFallback>,
}

#[must_use]
pub fn discover_vectorization_candidates(
    state: &KirVerifiedProgramState,
) -> VectorizationDiscovery {
    discover_vectorization_candidates_internal(state, true)
}

/// Discovers legal measurement-owned candidates without applying the ordinary
/// O3 static-profitability cutoff. All target, proof, and shape checks remain.
#[must_use]
pub fn discover_tuning_vectorization_candidates(
    state: &KirVerifiedProgramState,
) -> VectorizationDiscovery {
    discover_vectorization_candidates_internal(state, false)
}

fn discover_vectorization_candidates_internal(
    state: &KirVerifiedProgramState,
    require_static_profitability: bool,
) -> VectorizationDiscovery {
    let mut discovery = VectorizationDiscovery::default();
    let module = state.module();
    if !matches!(
        module.config.consumer,
        crate::KirConsumer::NativeLibrary | crate::KirConsumer::NativeExecutable
    ) {
        return discovery;
    }
    if module.config.sanitizer_mode == crate::KirSanitizerMode::Contracts {
        discovery.fallbacks.push(VectorizationFallback {
            function: module
                .functions
                .first()
                .map_or(FunctionId::from_index(0), |function| function.id),
            loop_id: None,
            reason: "sanitizer-mode-disabled".to_string(),
        });
        return discovery;
    }
    if module.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || module.config.bounds_mode != crate::KirBoundsMode::Unchecked
    {
        discovery.fallbacks.push(VectorizationFallback {
            function: module
                .functions
                .first()
                .map_or(FunctionId::from_index(0), |function| function.id),
            loop_id: None,
            reason: "checked-mode-requires-lane-proof".to_string(),
        });
        return discovery;
    }
    if !module.profile.vector_operations_enabled() {
        return discovery;
    }

    for function in &module.functions {
        let loops = analyze_canonical_loops_for_discovery(function);
        for descriptor in loops.loops.iter().filter(|loop_| loop_.innermost) {
            match discover_one(state, function, descriptor, require_static_profitability) {
                Ok(candidates) => discovery.candidates.extend(candidates),
                Err(reason) => discovery.fallbacks.push(VectorizationFallback {
                    function: function.id,
                    loop_id: Some(descriptor.id),
                    reason,
                }),
            }
        }
    }
    discovery
        .candidates
        .sort_by(|left, right| left.key.cmp(&right.key));
    discovery.fallbacks.sort_by(|left, right| {
        (left.function, left.loop_id, left.reason.as_str()).cmp(&(
            right.function,
            right.loop_id,
            right.reason.as_str(),
        ))
    });
    discovery
}

fn discover_one(
    state: &KirVerifiedProgramState,
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    require_static_profitability: bool,
) -> Result<Vec<VectorizationCandidate>, String> {
    let shape = simple_shape(function, descriptor)
        .ok_or_else(|| "unsupported-vector-loop-shape".to_string())?;
    let preheader = shape.preheader;
    let body = shape.body;
    let exit = shape.exit;
    let induction = descriptor
        .induction
        .as_ref()
        .filter(|induction| {
            induction.type_node == IntegerType::U32
                && induction.start == BigInt::from(0)
                && induction.step == BigInt::from(1)
                && induction.comparison == MirCompareOp::Lt
                && induction.wrap_safe_for_strict_bound
        })
        .ok_or_else(|| "vector-loop-requires-zero-based-u32-unit-induction".to_string())?;
    if !matches!(
        descriptor.trip_count,
        LoopTripCount::Runtime { .. } | LoopTripCount::Exact { .. }
    ) {
        return Err("vector-loop-trip-is-not-countable".to_string());
    }
    let legality = analyze_loop_legality(
        function,
        descriptor,
        state.contract_facts().map(crate::ContractFactSet::facts),
    )?;
    if !legality.eligible {
        let predicated_same_place_pair = shape.predicated_update.as_ref().is_some_and(|update| {
            legality.dependences.pairs.iter().all(|pair| {
                pair.kind != super::LoopDependenceKind::Unknown
                    || [pair.left, pair.right].into_iter().collect::<BTreeSet<_>>()
                        == [update.old_load_instruction, update.store_instruction]
                            .into_iter()
                            .collect::<BTreeSet<_>>()
            })
        });
        let remaining_reason = legality.fallback_reasons.iter().find(|reason| {
            !predicated_same_place_pair
                || **reason != super::LoopFallbackReason::UnknownMemoryDependence
        });
        if let Some(reason) = remaining_reason {
            return Err(reason.stable_name().to_string());
        }
    }
    let mut runtime_predicates = legality
        .dependences
        .pairs
        .iter()
        .filter(|pair| pair.kind == super::LoopDependenceKind::RuntimeGuarded)
        .filter_map(|pair| pair.predicate.clone())
        .collect::<Vec<_>>();
    runtime_predicates.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    runtime_predicates.dedup();
    let version_predicate = if runtime_predicates.is_empty() {
        None
    } else {
        let address_bits = runtime_predicates[0].address_bits;
        if runtime_predicates
            .iter()
            .any(|predicate| predicate.address_bits != address_bits)
        {
            return Err("vector-version-predicate-address-width-conflict".to_string());
        }
        let mut conjuncts = runtime_predicates
            .into_iter()
            .flat_map(|predicate| predicate.conjuncts)
            .collect::<Vec<_>>();
        conjuncts.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
        conjuncts.dedup();
        let predicate = super::TotalVersionPredicate {
            address_bits,
            conjuncts,
        };
        predicate.validate()?;
        if predicate.conjuncts.len() > 3 {
            return Err("vector-version-predicate-exceeds-four-total-conjuncts".to_string());
        }
        Some(predicate)
    };
    let accesses = analyze_affine_loop_accesses(
        function,
        descriptor,
        state.contract_facts().map(crate::ContractFactSet::facts),
    )?;
    let access_lanes = accesses
        .accesses
        .iter()
        .filter_map(|access| lane_type(&access.element_type))
        .collect::<BTreeSet<_>>();
    if !accesses.rejected_instructions.is_empty()
        || accesses.accesses.is_empty()
        || accesses.accesses.iter().any(|access| {
            !access.vector_group_eligible
                || !access.slice_base
                || lane_type(&access.element_type).is_none()
        })
    {
        return Err("vector-loop-has-non-unit-slice-access".to_string());
    }
    let header_block = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header)
        .expect("descriptor header exists");
    let crate::KirTerminator::Branch {
        then_edge: body_edge,
        ..
    } = &header_block.terminator
    else {
        return Err("vector-loop-header-is-not-a-branch".to_string());
    };
    let induction_index = header_block
        .params
        .iter()
        .position(|param| param.value == induction.value)
        .ok_or_else(|| "vector-loop-induction-is-not-a-header-parameter".to_string())?;
    let latch_block = function
        .blocks
        .iter()
        .find(|block| block.id == shape.latch)
        .expect("simple shape latch exists");
    let crate::KirTerminator::Jump { edge: latch_edge } = &latch_block.terminator else {
        return Err("vector-loop-latch-is-not-a-jump".to_string());
    };
    let induction_update_value = *latch_edge
        .args
        .get(induction_index)
        .ok_or_else(|| "vector-loop-latch-omits-induction".to_string())?;
    let induction_update = latch_block
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == induction_update_value)
        })
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    semantics: KirArithmeticSemantics::Modular,
                    ..
                }
            )
        })
        .map(|instruction| instruction.id)
        .ok_or_else(|| "vector-loop-induction-update-is-not-canonical".to_string())?;

    let mut reductions = Vec::new();
    if shape.diamond.is_none() {
        for (header_index, header_param) in header_block.params.iter().enumerate() {
            if header_param.value == induction.value {
                continue;
            }
            let Some(latch_value) = latch_edge.args.get(header_index).copied() else {
                continue;
            };
            let Some(instruction) = latch_block.instructions.iter().find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == latch_value)
            }) else {
                continue;
            };
            let KirInstructionKind::Binary {
                op: binary_op @ (MirBinaryOp::Add | MirBinaryOp::Mul),
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = instruction.kind
            else {
                continue;
            };
            let Some(body_index) = body_edge
                .args
                .iter()
                .position(|value| *value == header_param.value)
            else {
                continue;
            };
            let Some(body_value) = function
                .blocks
                .iter()
                .find(|block| block.id == body)
                .and_then(|block| block.params.get(body_index))
                .map(|param| param.value)
            else {
                continue;
            };
            if left != body_value && right != body_value {
                continue;
            }
            let lane_type = header_param
                .type_node
                .as_scalar()
                .and_then(lane_type)
                .filter(|lane| *lane != KirLaneType::F64)
                .ok_or_else(|| "vector-reduction-lane-is-unsupported".to_string())?;
            reductions.push(VectorReduction {
                header_value: header_param.value,
                body_value,
                instruction: instruction.id,
                operation: if binary_op == MirBinaryOp::Add {
                    KirProfileOperation::ReduceAdd
                } else {
                    KirProfileOperation::ReduceMultiply
                },
                binary_op,
                lane_type,
            });
        }
    }
    if reductions.len() > 1 {
        return Err("vector-loop-has-multiple-reductions".to_string());
    }
    let reduction = reductions.pop();

    let access_ids = accesses
        .accesses
        .iter()
        .map(|access| access.instruction)
        .collect::<BTreeSet<_>>();
    let mut operations = Vec::new();
    let mut needs_splat = false;
    let scheduled_blocks = shape
        .scalar_blocks
        .iter()
        .filter_map(|id| function.blocks.iter().find(|block| block.id == *id));
    for instruction in scheduled_blocks.flat_map(|block| &block.instructions) {
        if instruction.id == induction_update || access_ids.contains(&instruction.id) {
            continue;
        }
        if reduction
            .as_ref()
            .is_some_and(|reduction| reduction.instruction == instruction.id)
        {
            let reduction = reduction.as_ref().expect("matched reduction");
            operations.push(VectorCandidateOperation {
                scalar: reduction.instruction,
                operation: reduction.operation,
                lane_type: reduction.lane_type,
                result_lane_type: reduction.lane_type,
                semantics: KirCostSemantics::Modular,
                alignment: KirAlignmentClass::NotApplicable,
            });
            continue;
        }
        match scalar_vector_operation(function, instruction) {
            Some(operation) => {
                needs_splat |= operation_has_scalar_invariant(function, descriptor, instruction);
                operations.push(operation);
            }
            None if matches!(
                instruction.kind,
                KirInstructionKind::ConstInt { .. } | KirInstructionKind::Copy { .. }
            ) => {}
            None => return Err("vector-loop-contains-unsupported-operation".to_string()),
        }
    }
    if let Some(diamond) = &shape.diamond {
        let selected = function
            .blocks
            .iter()
            .find(|block| block.id == diamond.merge_block)
            .and_then(|block| block.params.get(diamond.selected_param_index))
            .and_then(|param| param.type_node.as_scalar())
            .and_then(lane_type)
            .ok_or_else(|| "vector-diamond-selected-type-is-unsupported".to_string())?;
        operations.push(VectorCandidateOperation {
            scalar: diamond.condition_instruction,
            operation: KirProfileOperation::Select,
            lane_type: selected,
            result_lane_type: selected,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        });
    }
    if let Some(update) = &shape.predicated_update {
        let selected = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == update.old_load_instruction)
            .and_then(|instruction| instruction.results.first())
            .and_then(|result| result.type_node.as_scalar())
            .and_then(lane_type)
            .ok_or_else(|| "predicated-update-selected-type-is-unsupported".to_string())?;
        operations.push(VectorCandidateOperation {
            scalar: update.condition_instruction,
            operation: KirProfileOperation::Select,
            lane_type: selected,
            result_lane_type: selected,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        });
    }
    if operations.is_empty()
        || (reduction.is_none()
            && !accesses
                .accesses
                .iter()
                .any(|access| access.kind == super::LoopMemoryAccessKind::Write))
    {
        return Err("vector-loop-has-no-profitable-store-computation".to_string());
    }
    operations.sort_by_key(|operation| operation.scalar);
    let operation_lanes = operations
        .iter()
        .flat_map(|operation| [operation.lane_type, operation.result_lane_type])
        .collect::<BTreeSet<_>>();
    if !access_lanes.is_subset(&operation_lanes) {
        return Err("vector-loop-access-lanes-are-not-covered-by-operations".to_string());
    }

    let legal_vfs = [2_u16, 4, 8, 16]
        .into_iter()
        .filter(|vf| {
            profile_supports_candidate(
                &state.module().profile,
                *vf,
                &operations,
                &accesses.accesses,
                needs_splat,
                version_predicate.is_some(),
            )
        })
        .collect::<Vec<_>>();
    if legal_vfs.is_empty() {
        return Err("vector-loop-target-profile-is-unavailable".to_string());
    }
    let maximum_uf = state.module().profile.maximum_interleave_factor().min(4);
    let interleavable = shape.diamond.is_none() && reduction.is_none();
    let legal_ufs = [1_u8, 2, 4]
        .into_iter()
        .filter(|uf| *uf <= maximum_uf && (*uf == 1 || interleavable))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut profitability_error = None;
    for vf in legal_vfs {
        for &uf in &legal_ufs {
            let (predicted_cost, minimum_trip) = match candidate_cost_and_threshold(
                &state.module().profile,
                descriptor,
                (vf, uf),
                &operations,
                &accesses.accesses,
                needs_splat,
                version_predicate.as_ref(),
                require_static_profitability,
            ) {
                Ok(result) => result,
                Err(error) if error == "vector-profitability-threshold-not-met" => {
                    profitability_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            candidates.push(VectorizationCandidate {
                key: CandidateKey::LoopFrontier {
                    function: function.id,
                    loop_id: descriptor.id,
                    kind: LoopCandidateKind::LoopSimd,
                    variant: LoopCandidateVariant::Scalar,
                    vf,
                    uf,
                },
                function: function.id,
                loop_id: descriptor.id,
                preheader,
                header: descriptor.header,
                body,
                latch: shape.latch,
                exit,
                scalar_blocks: shape.scalar_blocks.clone(),
                diamond: shape.diamond.clone(),
                predicated_update: shape.predicated_update.clone(),
                reduction: reduction.clone(),
                induction: induction.value,
                bound: induction.bound,
                induction_update,
                vf,
                uf,
                minimum_trip,
                operations: operations.clone(),
                accesses: accesses.accesses.clone(),
                version_predicate: version_predicate.clone(),
                predicted_cost,
            });
        }
    }
    if candidates.is_empty() {
        return Err(profitability_error
            .unwrap_or_else(|| "vector-profitability-threshold-not-met".to_string()));
    }
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn candidate_cost_and_threshold(
    profile: &crate::KirTargetProfile,
    descriptor: &CanonicalLoopDescriptor,
    shape: (u16, u8),
    operations: &[VectorCandidateOperation],
    accesses: &[AffineMemoryAccess],
    needs_splat: bool,
    version_predicate: Option<&super::TotalVersionPredicate>,
    require_static_profitability: bool,
) -> Result<(KirCostEstimate, u32), String> {
    let (vf, uf) = shape;
    let lanes = u8::try_from(vf).map_err(|_| "vector VF exceeds cost schema".to_string())?;
    let mut scalar_iteration = 0_u32;
    let mut vector_lane_chunk = 0_u32;
    for operation in operations {
        let scalar_operation = match operation.operation {
            KirProfileOperation::ReduceAdd => KirProfileOperation::Add,
            KirProfileOperation::ReduceMultiply => KirProfileOperation::Multiply,
            operation => operation,
        };
        scalar_iteration = scalar_iteration.saturating_add(profile_cost(
            profile,
            KirCostKey {
                operation: scalar_operation,
                lane: operation.lane_type,
                lanes: 1,
                semantics: operation.semantics,
                alignment: operation.alignment,
            },
        )?);
        vector_lane_chunk = vector_lane_chunk.saturating_add(profile_cost(
            profile,
            KirCostKey {
                operation: operation.operation,
                lane: operation.lane_type,
                lanes,
                semantics: operation.semantics,
                alignment: operation.alignment,
            },
        )?);
    }
    for access in accesses {
        let lane = lane_type(&access.element_type)
            .ok_or_else(|| "vector memory lane is unavailable to the cost model".to_string())?;
        let alignment = u16::try_from(access.element_bytes)
            .map(KirAlignmentClass::Bytes)
            .map_err(|_| "vector memory alignment exceeds the cost schema".to_string())?;
        let operation = if access.kind == super::LoopMemoryAccessKind::Read {
            KirProfileOperation::Load
        } else {
            KirProfileOperation::Store
        };
        scalar_iteration = scalar_iteration.saturating_add(profile_cost(
            profile,
            KirCostKey {
                operation,
                lane,
                lanes: 1,
                semantics: KirCostSemantics::NotApplicable,
                alignment,
            },
        )?);
        vector_lane_chunk = vector_lane_chunk.saturating_add(profile_cost(
            profile,
            KirCostKey {
                operation,
                lane,
                lanes,
                semantics: KirCostSemantics::NotApplicable,
                alignment,
            },
        )?);
    }
    let mut splat_cost = 0_u32;
    if needs_splat {
        let mut splat_lanes = operations
            .iter()
            .map(|operation| operation.lane_type)
            .collect::<BTreeSet<_>>();
        if splat_lanes.is_empty() {
            splat_lanes.insert(KirLaneType::U32);
        }
        for lane in splat_lanes {
            splat_cost = splat_cost.saturating_add(profile_cost(
                profile,
                KirCostKey {
                    operation: KirProfileOperation::Splat,
                    lane,
                    lanes,
                    semantics: KirCostSemantics::NotApplicable,
                    alignment: KirAlignmentClass::NotApplicable,
                },
            )?);
        }
    }

    let scalar_control = profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Add,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::Modular,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )?
    .saturating_add(profile_control_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Compare,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )?)
    .saturating_add(profile_control_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Branch,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )?);
    scalar_iteration = scalar_iteration.saturating_add(scalar_control);
    let vector_chunk = vector_lane_chunk
        .saturating_mul(u32::from(uf))
        .saturating_add(splat_cost)
        .saturating_add(scalar_control);

    let predicate_base = profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Compare,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )?
    .saturating_add(profile_control_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Branch,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )?);
    let predicate_cost = if let Some(predicate) = version_predicate {
        let one = profile_cost(
            profile,
            KirCostKey {
                operation: KirProfileOperation::RuntimePredicate,
                lane: KirLaneType::U32,
                lanes,
                semantics: KirCostSemantics::NotApplicable,
                alignment: KirAlignmentClass::NotApplicable,
            },
        )?;
        predicate_base.saturating_add(
            one.saturating_mul(u32::try_from(predicate.conjuncts.len()).unwrap_or(u32::MAX)),
        )
    } else {
        predicate_base
    };
    let epilogue = profile_control_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Branch,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )?;
    let chunk_width = u32::from(vf).saturating_mul(u32::from(uf));
    let scalar_chunk = scalar_iteration.saturating_mul(chunk_width);
    if require_static_profitability
        && u64::from(vector_chunk).saturating_mul(100) >= u64::from(scalar_chunk).saturating_mul(80)
    {
        return Err("vector-profitability-threshold-not-met".to_string());
    }
    let minimum_trip = if require_static_profitability {
        (2_u32..=1024)
            .map(|chunks| chunks.saturating_mul(chunk_width))
            .find(|trip| {
                (0..chunk_width).all(|tail| {
                    let iterations = trip.saturating_add(tail);
                    let scalar = scalar_iteration.saturating_mul(iterations);
                    let transformed = vector_chunk
                        .saturating_mul(*trip / chunk_width)
                        .saturating_add(scalar_iteration.saturating_mul(tail))
                        .saturating_add(predicate_cost)
                        .saturating_add(epilogue.saturating_mul(u32::from(tail != 0)));
                    u64::from(transformed).saturating_mul(100)
                        <= u64::from(scalar).saturating_mul(80)
                })
            })
            .ok_or_else(|| "vector-profitability-threshold-not-met".to_string())?
    } else {
        chunk_width.saturating_mul(2)
    };
    if let LoopTripCount::Exact { iterations } = descriptor.trip_count
        && (iterations < u64::from(minimum_trip) || iterations > u64::from(u32::MAX))
    {
        return Err("vector-profitability-threshold-not-met".to_string());
    }
    let priced_tail = chunk_width.saturating_sub(1);
    let priced_trip = minimum_trip.saturating_add(priced_tail);
    let priced_chunks = minimum_trip / chunk_width;
    Ok((
        KirCostEstimate::new(
            scalar_iteration.saturating_mul(priced_trip),
            vector_chunk.saturating_mul(priced_chunks),
            predicate_cost,
            scalar_iteration
                .saturating_mul(priced_tail)
                .saturating_add(epilogue),
        ),
        minimum_trip,
    ))
}

fn profile_cost(profile: &crate::KirTargetProfile, key: KirCostKey) -> Result<u32, String> {
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Ok(cost.cost)
        }
        _ => Err(format!("vector cost entry is unavailable: {key:?}")),
    }
}

fn profile_control_cost(profile: &crate::KirTargetProfile, key: KirCostKey) -> Result<u32, String> {
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Ok(cost.cost)
        }
        Some(KirOperationAvailability::Unavailable)
            if key.operation == KirProfileOperation::Branch =>
        {
            // LLVM throughput models may report a zero-cost branch, which the
            // closed CK profile deliberately records as unavailable. Keep one
            // CK structural unit so loop control is never treated as free.
            Ok(1)
        }
        _ => Err(format!("vector control cost entry is unavailable: {key:?}")),
    }
}

#[derive(Debug, Clone)]
struct VectorLoopShape {
    preheader: BlockId,
    body: BlockId,
    latch: BlockId,
    exit: BlockId,
    scalar_blocks: Vec<BlockId>,
    diamond: Option<VectorDiamond>,
    predicated_update: Option<VectorPredicatedUpdate>,
}

fn simple_shape(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
) -> Option<VectorLoopShape> {
    if !descriptor.dedicated_exits || !descriptor.lcssa || descriptor.exits.len() != 1 {
        return None;
    }
    let preheader = descriptor.preheader?;
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header)?;
    let preheader_block = function.blocks.iter().find(|block| block.id == preheader)?;
    let crate::KirTerminator::Jump { edge: entry } = &preheader_block.terminator else {
        return None;
    };
    let crate::KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &header.terminator
    else {
        return None;
    };
    if entry.target != descriptor.header || else_edge.target != descriptor.exits[0] {
        return None;
    }
    let body = function
        .blocks
        .iter()
        .find(|block| block.id == then_edge.target)?;
    if descriptor.blocks.len() == 2 {
        let crate::KirTerminator::Jump { edge: latch } = &body.terminator else {
            return None;
        };
        return (descriptor.latch == Some(body.id) && latch.target == descriptor.header).then_some(
            VectorLoopShape {
                preheader,
                body: body.id,
                latch: body.id,
                exit: descriptor.exits[0],
                scalar_blocks: vec![body.id],
                diamond: None,
                predicated_update: None,
            },
        );
    }
    if descriptor.blocks.len() == 4 {
        return predicated_update_shape(function, descriptor, preheader, body);
    }
    if descriptor.blocks.len() != 5 {
        return None;
    }
    let crate::KirTerminator::Branch {
        condition,
        then_edge: diamond_then,
        else_edge: diamond_else,
    } = &body.terminator
    else {
        return None;
    };
    let then_block = function
        .blocks
        .iter()
        .find(|block| block.id == diamond_then.target)?;
    let else_block = function
        .blocks
        .iter()
        .find(|block| block.id == diamond_else.target)?;
    let crate::KirTerminator::Jump { edge: then_merge } = &then_block.terminator else {
        return None;
    };
    let crate::KirTerminator::Jump { edge: else_merge } = &else_block.terminator else {
        return None;
    };
    if then_merge.target != else_merge.target {
        return None;
    }
    let merge = function
        .blocks
        .iter()
        .find(|block| block.id == then_merge.target)?;
    let crate::KirTerminator::Jump { edge: latch } = &merge.terminator else {
        return None;
    };
    if descriptor.latch != Some(merge.id)
        || latch.target != descriptor.header
        || then_merge.args.len() != merge.params.len()
        || else_merge.args.len() != merge.params.len()
        || [then_block, else_block].into_iter().any(|block| {
            block.instructions.iter().any(|instruction| {
                instruction.memory.is_some()
                    || instruction.effect.is_some()
                    || !matches!(
                        instruction.kind,
                        KirInstructionKind::ConstInt { .. }
                            | KirInstructionKind::ConstFloat { .. }
                            | KirInstructionKind::ConstBool { .. }
                            | KirInstructionKind::Copy { .. }
                            | KirInstructionKind::Binary { .. }
                            | KirInstructionKind::Unary { .. }
                            | KirInstructionKind::Compare { .. }
                            | KirInstructionKind::Cast { .. }
                    )
            })
        })
    {
        return None;
    }
    let incoming_source = |block: &crate::KirBlock, edge: &crate::KirEdge, value| {
        block
            .params
            .iter()
            .position(|param| param.value == value)
            .and_then(|index| edge.args.get(index).copied())
            .unwrap_or(value)
    };
    let varying = then_merge
        .args
        .iter()
        .zip(&else_merge.args)
        .enumerate()
        .filter(|(_, (then_value, else_value))| {
            incoming_source(then_block, diamond_then, **then_value)
                != incoming_source(else_block, diamond_else, **else_value)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let [selected_param_index] = varying.as_slice() else {
        return None;
    };
    merge
        .params
        .get(*selected_param_index)
        .and_then(|param| param.type_node.as_scalar())
        .and_then(lane_type)?;
    let condition_instruction = body
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == *condition)
                && matches!(instruction.kind, KirInstructionKind::Compare { .. })
        })?
        .id;
    Some(VectorLoopShape {
        preheader,
        body: body.id,
        latch: merge.id,
        exit: descriptor.exits[0],
        scalar_blocks: vec![body.id, then_block.id, else_block.id, merge.id],
        diamond: Some(VectorDiamond {
            then_block: then_block.id,
            else_block: else_block.id,
            merge_block: merge.id,
            condition: *condition,
            condition_instruction,
            selected_param_index: *selected_param_index,
        }),
        predicated_update: None,
    })
}

fn predicated_update_shape(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    preheader: BlockId,
    body: &crate::KirBlock,
) -> Option<VectorLoopShape> {
    let crate::KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &body.terminator
    else {
        return None;
    };
    let store_block = function
        .blocks
        .iter()
        .find(|block| block.id == then_edge.target)?;
    let merge = function
        .blocks
        .iter()
        .find(|block| block.id == else_edge.target)?;
    let crate::KirTerminator::Jump { edge: store_merge } = &store_block.terminator else {
        return None;
    };
    let crate::KirTerminator::Jump { edge: latch } = &merge.terminator else {
        return None;
    };
    if store_block.id == merge.id
        || store_merge.target != merge.id
        || descriptor.latch != Some(merge.id)
        || latch.target != descriptor.header
        || then_edge.args.len() != store_block.params.len()
        || then_edge.memory_args.len() != store_block.memory_params.len()
        || else_edge.args.len() != merge.params.len()
        || else_edge.memory_args.len() != merge.memory_params.len()
        || store_merge.args.len() != merge.params.len()
        || store_merge.memory_args.len() != merge.memory_params.len()
    {
        return None;
    }

    let [store] = store_block.instructions.as_slice() else {
        return None;
    };
    let KirInstructionKind::Store { place, value } = &store.kind else {
        return None;
    };
    if !matches!(
        store.effect.as_ref().map(|effect| effect.kind),
        Some(crate::KirEffectKind::WriteMemory)
    ) {
        return None;
    }
    let store_memory = store.memory.as_ref()?;
    let memory_output = store_memory.output?;

    let value_aliases = store_block
        .params
        .iter()
        .zip(&then_edge.args)
        .map(|(param, source)| (param.value, *source))
        .collect::<BTreeMap<_, _>>();
    let memory_aliases = store_block
        .memory_params
        .iter()
        .zip(&then_edge.memory_args)
        .map(|(param, source)| (param.version, *source))
        .collect::<BTreeMap<_, _>>();

    if store_merge
        .args
        .iter()
        .zip(&else_edge.args)
        .any(|(stored, empty)| resolve_value_alias(*stored, &value_aliases) != *empty)
    {
        return None;
    }

    let store_input = resolve_memory_alias(store_memory.input, &memory_aliases);
    let mut merged_store_region = false;
    for (index, merge_param) in merge.memory_params.iter().enumerate() {
        let stored = resolve_memory_alias(store_merge.memory_args[index], &memory_aliases);
        let empty = else_edge.memory_args[index];
        if merge_param.region == store_memory.region {
            if stored != memory_output || empty != store_input {
                return None;
            }
            merged_store_region = true;
        } else if stored != empty {
            return None;
        }
    }
    if !merged_store_region {
        return None;
    }

    let normalized_place = remap_place_values(place, &value_aliases);
    let stored_value = resolve_value_alias(*value, &value_aliases);
    let compare = body.instructions.iter().find(|instruction| {
        instruction
            .results
            .iter()
            .any(|result| result.value == *condition)
    })?;
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: candidate,
        right: old,
    } = compare.kind
    else {
        return None;
    };
    if compare.memory.is_some() || compare.effect.is_some() || stored_value != candidate {
        return None;
    }
    let old_load = body
        .instructions
        .iter()
        .find(|instruction| instruction.results.iter().any(|result| result.value == old))?;
    let KirInstructionKind::Load { place: old_place } = &old_load.kind else {
        return None;
    };
    let old_memory = old_load.memory.as_ref()?;
    if old_place.as_ref() != &normalized_place
        || old_memory.region != store_memory.region
        || old_memory.input != store_input
        || old_load
            .results
            .first()
            .and_then(|result| result.type_node.as_scalar())
            != Some(&MirType::Primitive(MirPrimitiveTypeName::F64))
        || value_type(function, candidate) != Some(&MirType::Primitive(MirPrimitiveTypeName::F64))
        || body
            .instructions
            .iter()
            .position(|instruction| instruction.id == old_load.id)?
            >= body
                .instructions
                .iter()
                .position(|instruction| instruction.id == compare.id)?
    {
        return None;
    }

    Some(VectorLoopShape {
        preheader,
        body: body.id,
        latch: merge.id,
        exit: descriptor.exits[0],
        scalar_blocks: vec![body.id, store_block.id, merge.id],
        diamond: None,
        predicated_update: Some(VectorPredicatedUpdate {
            then_block: store_block.id,
            merge_block: merge.id,
            condition_instruction: compare.id,
            old_load_instruction: old_load.id,
            store_instruction: store.id,
            store_when_true: true,
            memory_input: store_input,
            memory_output,
        }),
    })
}

fn resolve_value_alias(
    value: crate::ValueId,
    aliases: &BTreeMap<crate::ValueId, crate::ValueId>,
) -> crate::ValueId {
    aliases.get(&value).copied().unwrap_or(value)
}

fn resolve_memory_alias(
    version: MemoryVersionId,
    aliases: &BTreeMap<MemoryVersionId, MemoryVersionId>,
) -> MemoryVersionId {
    aliases.get(&version).copied().unwrap_or(version)
}

fn remap_place_values(
    place: &crate::KirPlace,
    aliases: &BTreeMap<crate::ValueId, crate::ValueId>,
) -> crate::KirPlace {
    match place {
        crate::KirPlace::Value {
            value,
            type_node,
            region,
        } => crate::KirPlace::Value {
            value: resolve_value_alias(*value, aliases),
            type_node: type_node.clone(),
            region: *region,
        },
        crate::KirPlace::Deref {
            pointer,
            type_node,
            region,
        } => crate::KirPlace::Deref {
            pointer: resolve_value_alias(*pointer, aliases),
            type_node: type_node.clone(),
            region: *region,
        },
        crate::KirPlace::Index {
            base,
            index,
            type_node,
            region,
        } => crate::KirPlace::Index {
            base: Box::new(remap_place_values(base, aliases)),
            index: resolve_value_alias(*index, aliases),
            type_node: type_node.clone(),
            region: *region,
        },
        crate::KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } => crate::KirPlace::SliceIndex {
            slice: resolve_value_alias(*slice, aliases),
            index: resolve_value_alias(*index, aliases),
            type_node: type_node.clone(),
            region: *region,
        },
        crate::KirPlace::Field {
            base,
            field_name,
            type_node,
            region,
        } => crate::KirPlace::Field {
            base: Box::new(remap_place_values(base, aliases)),
            field_name: field_name.clone(),
            type_node: type_node.clone(),
            region: *region,
        },
    }
}

fn scalar_vector_operation(
    function: &crate::KirFunction,
    instruction: &KirInstruction,
) -> Option<VectorCandidateOperation> {
    if let KirInstructionKind::Compare { left, right, .. } = instruction.kind {
        let left_lane = value_type(function, left).and_then(lane_type)?;
        if value_type(function, right).and_then(lane_type)? != left_lane {
            return None;
        }
        return Some(VectorCandidateOperation {
            scalar: instruction.id,
            operation: KirProfileOperation::Compare,
            lane_type: left_lane,
            result_lane_type: left_lane,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        });
    }
    let result_lane_type = instruction
        .results
        .first()
        .and_then(|result| result.type_node.as_scalar())
        .and_then(lane_type)?;
    let (operation, semantics, lane_type) = match instruction.kind {
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => KirProfileOperation::Add,
                MirBinaryOp::Sub => KirProfileOperation::Subtract,
                MirBinaryOp::Mul => KirProfileOperation::Multiply,
                MirBinaryOp::Div if semantics == KirArithmeticSemantics::StrictFloat => {
                    KirProfileOperation::Divide
                }
                MirBinaryOp::Div | MirBinaryOp::Mod => return None,
            },
            match semantics {
                KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
                KirArithmeticSemantics::Checked => return None,
            },
            result_lane_type,
        ),
        KirInstructionKind::Unary {
            op: MirUnaryOp::Neg,
            semantics,
            ..
        } => (
            KirProfileOperation::Negate,
            match semantics {
                KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
                KirArithmeticSemantics::Checked => return None,
            },
            result_lane_type,
        ),
        KirInstructionKind::Cast { op, value } => (
            match op {
                crate::MirCastOp::I32ToF64 | crate::MirCastOp::U32ToF64 => {
                    KirProfileOperation::Cast
                }
            },
            KirCostSemantics::NotApplicable,
            value_type(function, value).and_then(lane_type)?,
        ),
        _ => return None,
    };
    Some(VectorCandidateOperation {
        scalar: instruction.id,
        operation,
        lane_type,
        result_lane_type,
        semantics,
        alignment: KirAlignmentClass::NotApplicable,
    })
}

fn value_type(function: &crate::KirFunction, value: crate::ValueId) -> Option<&MirType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| &param.type_node)
        .or_else(|| {
            function.blocks.iter().find_map(|block| {
                block
                    .params
                    .iter()
                    .find(|param| param.value == value)
                    .and_then(|param| param.type_node.as_scalar())
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

fn lane_type(type_node: &MirType) -> Option<KirLaneType> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(KirLaneType::I64),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some(KirLaneType::U64),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn operation_has_scalar_invariant(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    instruction: &KirInstruction,
) -> bool {
    let loop_values = descriptor
        .blocks
        .iter()
        .filter_map(|id| function.blocks.iter().find(|block| block.id == *id))
        .flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        })
        .collect::<BTreeSet<_>>();
    match instruction.kind {
        KirInstructionKind::Binary { left, right, .. } => {
            !loop_values.contains(&left) || !loop_values.contains(&right)
        }
        KirInstructionKind::Unary { operand, .. } => !loop_values.contains(&operand),
        KirInstructionKind::Cast { value, .. } => !loop_values.contains(&value),
        _ => false,
    }
}

fn profile_supports_candidate(
    profile: &crate::KirTargetProfile,
    vf: u16,
    operations: &[VectorCandidateOperation],
    accesses: &[AffineMemoryAccess],
    needs_splat: bool,
    needs_runtime_predicate: bool,
) -> bool {
    let Ok(lanes) = u8::try_from(vf) else {
        return false;
    };
    let legal = |key: KirCostKey| {
        matches!(
            profile.operation_availability(&key),
            Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1
        )
    };
    if operations.iter().any(|operation| {
        !legal(KirCostKey {
            operation: operation.operation,
            lane: operation.lane_type,
            lanes,
            semantics: operation.semantics,
            alignment: operation.alignment,
        })
    }) {
        return false;
    }
    if needs_splat
        && operations.iter().any(|operation| {
            !legal(KirCostKey {
                operation: KirProfileOperation::Splat,
                lane: operation.lane_type,
                lanes,
                semantics: KirCostSemantics::NotApplicable,
                alignment: KirAlignmentClass::NotApplicable,
            })
        })
    {
        return false;
    }
    if needs_runtime_predicate
        && !legal(KirCostKey {
            operation: KirProfileOperation::RuntimePredicate,
            lane: KirLaneType::U32,
            lanes,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        })
    {
        return false;
    }
    accesses.iter().all(|access| {
        let Some(lane) = lane_type(&access.element_type) else {
            return false;
        };
        let Ok(alignment) = u16::try_from(access.element_bytes) else {
            return false;
        };
        legal(KirCostKey {
            operation: if access.kind == super::LoopMemoryAccessKind::Read {
                KirProfileOperation::Load
            } else {
                KirProfileOperation::Store
            },
            lane,
            lanes,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::Bytes(alignment),
        })
    })
}
