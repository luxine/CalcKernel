use std::collections::BTreeSet;

use num_bigint::BigInt;

use crate::{
    BlockId, CandidateKey, CanonicalLoopDescriptor, FunctionId, InstructionId, KirAlignmentClass,
    KirArithmeticSemantics, KirCostEstimate, KirCostKey, KirCostSemantics, KirCpuIdentity,
    KirInstruction, KirInstructionKind, KirLaneType, KirOperationAvailability, KirPlace,
    KirProfileOperation, KirTargetIdentity, LoopCandidateKind, LoopCandidateVariant, LoopId,
    LoopTripCount, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, MirUnaryOp,
    WasmRangeCount, WasmSliceRangeRequirement,
};

use super::{
    AffineMemoryAccess, AliasKind, IntegerType, analyze_affine_loop_accesses,
    analyze_canonical_loops_for_discovery, analyze_loop_legality_for_profile, analyze_regions,
    query_alias,
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
    pub reduction: Option<VectorReduction>,
    pub induction: crate::ValueId,
    pub bound: crate::ValueId,
    pub induction_update: InstructionId,
    pub vf: u16,
    pub uf: u8,
    pub minimum_trip: u32,
    pub operations: Vec<VectorCandidateOperation>,
    pub accesses: Vec<AffineMemoryAccess>,
    pub wasm_affine: Option<WasmAffineCandidate>,
    pub version_predicate: Option<super::TotalVersionPredicate>,
    pub predicted_cost: KirCostEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmAffineCandidate {
    pub accesses: Vec<WasmAffineAccessShape>,
    pub scalar_address_setup: Vec<InstructionId>,
    pub range_requirements: Vec<WasmSliceRangeRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmAffineAccessShape {
    pub instruction: InstructionId,
    pub slice: crate::ValueId,
    pub kind: super::LoopMemoryAccessKind,
    pub shape: WasmAffineShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmAffineShape {
    Contiguous { offset: Option<crate::ValueId> },
    Broadcast { index: crate::ValueId },
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
    let mut discovery = VectorizationDiscovery::default();
    let module = state.module();
    let native_consumer = matches!(
        module.config.consumer,
        crate::KirConsumer::NativeLibrary | crate::KirConsumer::NativeExecutable
    );
    let wasm_simd128_consumer = module.config.consumer == crate::KirConsumer::WebAssembly
        && module.profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128);
    if !native_consumer && !wasm_simd128_consumer {
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
            match discover_one(state, function, descriptor) {
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
) -> Result<Vec<VectorizationCandidate>, String> {
    let shape = simple_shape(function, descriptor)
        .ok_or_else(|| "unsupported-vector-loop-shape".to_string())?;
    let wasm_simd128_consumer = state.module().config.consumer == crate::KirConsumer::WebAssembly
        && state.module().profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128);
    if matches!(
        state.module().profile.target_identity(),
        KirTargetIdentity::Native { triple } if triple.starts_with("aarch64-")
    ) && matches!(
        state.module().profile.cpu_identity(),
        KirCpuIdentity::Native { features, .. }
            if features.iter().any(|feature| matches!(feature.as_str(), "+sve" | "+sve2"))
    ) {
        return Err("aarch64-sve-loop-deferred-to-native-loop-vectorizer".to_string());
    }
    if matches!(
        state.module().config.consumer,
        crate::KirConsumer::NativeLibrary | crate::KirConsumer::NativeExecutable
    ) && has_constant_call_bound(state.module(), function, descriptor)
    {
        return Err("constant-call-loop-deferred-to-native-loop-vectorizer".to_string());
    }
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
    let legality = analyze_loop_legality_for_profile(
        function,
        descriptor,
        state.contract_facts().map(crate::ContractFactSet::facts),
        &state.module().profile,
    )?;
    if !legality.eligible {
        return Err(legality
            .fallback_reasons
            .first()
            .map_or("vector-loop-is-illegal".to_string(), |reason| {
                reason.stable_name().to_string()
            }));
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
    let wasm_affine =
        (wasm_simd128_consumer && version_predicate.is_none() && shape.diamond.is_none())
            .then(|| {
                discover_wasm_affine_candidate(
                    function,
                    descriptor,
                    &accesses,
                    state.contract_facts().map(crate::ContractFactSet::facts),
                )
            })
            .flatten();
    let affine_shape_requires_wasm_route = wasm_simd128_consumer
        && accesses.accesses.iter().any(|access| {
            access.invariant_offset.is_some()
                || !access.unit_stride
                || access.bias != BigInt::from(0)
        });
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
        }) && wasm_affine.is_none()
    {
        return Err(if affine_shape_requires_wasm_route {
            "wasm-affine-access-shape-is-not-proven".to_string()
        } else {
            "vector-loop-has-non-unit-slice-access".to_string()
        });
    }
    if affine_shape_requires_wasm_route && wasm_affine.is_none() {
        return Err("wasm-affine-access-shape-is-not-proven".to_string());
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
    if state.module().profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128)
        && let Some(reduction) = reduction.as_ref()
    {
        // Neither per-chunk folds nor persistent vector accumulators preserve
        // every scalar partial result. Reject any observable intermediate sum
        // or product, even when the final update resembles a modular reduction.
        let body_block = function
            .blocks
            .iter()
            .find(|block| block.id == body)
            .ok_or_else(|| "vector reduction body is missing".to_string())?;
        let mut recurrent = BTreeSet::from([reduction.header_value, reduction.body_value]);
        let updated = body_block
            .instructions
            .iter()
            .find(|instruction| instruction.id == reduction.instruction)
            .and_then(|instruction| instruction.results.first())
            .map(|result| result.value)
            .ok_or_else(|| "vector reduction result is missing".to_string())?;
        recurrent.insert(updated);
        recurrent.extend(
            body_block
                .params
                .iter()
                .zip(&body_edge.args)
                .filter(|(_, source)| **source == reduction.header_value)
                .map(|(param, _)| param.value),
        );
        let accumulator_index = header_block
            .params
            .iter()
            .position(|param| param.value == reduction.header_value)
            .ok_or_else(|| "vector reduction header parameter is missing".to_string())?;
        // A previous-value alias can escape solely through the latch edge:
        // `previous = total; total += a[i]` need not leave any Copy instruction.
        // Only the updated value may reach the accumulator's own parameter.
        if latch_edge.args.iter().enumerate().any(|(index, value)| {
            let root = scalar_copy_root(function, *value);
            if index == accumulator_index {
                root != updated
            } else {
                recurrent.contains(&root)
            }
        }) {
            return Err("vector-reduction-exposes-partial-accumulator".to_string());
        }
        for instruction in &body_block.instructions {
            let operands = match instruction.kind {
                KirInstructionKind::Binary { left, right, .. }
                | KirInstructionKind::Compare { left, right, .. } => vec![left, right],
                KirInstructionKind::Unary { operand, .. } => vec![operand],
                KirInstructionKind::Cast { value, .. } | KirInstructionKind::Copy { value } => {
                    vec![value]
                }
                _ => Vec::new(),
            };
            let uses = operands
                .iter()
                .filter(|value| recurrent.contains(&scalar_copy_root(function, **value)))
                .count();
            if (instruction.id == reduction.instruction && uses != 1)
                || (instruction.id != reduction.instruction && uses != 0)
                || matches!(instruction.kind, KirInstructionKind::Store { .. })
            {
                return Err("vector-reduction-exposes-partial-accumulator".to_string());
            }
        }
    }
    if reduction.is_some()
        && matches!(
            state.module().profile.target_identity(),
            KirTargetIdentity::Native { triple } if triple.starts_with("x86_64-")
        )
    {
        return Err("x86-horizontal-reduction-deferred-to-native-loop-vectorizer".to_string());
    }

    if wasm_affine.is_some() && (shape.diamond.is_some() || reduction.is_some()) {
        return Err("wasm-affine-loop-has-diamond-or-reduction".to_string());
    }
    let access_ids = accesses
        .accesses
        .iter()
        .map(|access| access.instruction)
        .collect::<BTreeSet<_>>();
    let address_setup_ids = wasm_affine
        .as_ref()
        .map(|affine| {
            affine
                .scalar_address_setup
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut operations = Vec::new();
    let scheduled_blocks = shape
        .scalar_blocks
        .iter()
        .filter_map(|id| function.blocks.iter().find(|block| block.id == *id));
    for instruction in scheduled_blocks.flat_map(|block| &block.instructions) {
        if instruction.id == induction_update
            || access_ids.contains(&instruction.id)
            || address_setup_ids.contains(&instruction.id)
        {
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
            Some(operation) => operations.push(operation),
            None if matches!(
                instruction.kind,
                KirInstructionKind::ConstInt { .. }
                    | KirInstructionKind::ConstFloat { .. }
                    | KirInstructionKind::Copy { .. }
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

    let splat_inputs = vector_splat_inputs(
        function,
        &shape.scalar_blocks,
        &operations,
        &accesses.accesses,
        shape.diamond.as_ref(),
        reduction.as_ref(),
    );
    let needs_splat = !splat_inputs.is_empty()
        || wasm_affine.as_ref().is_some_and(|affine| {
            affine
                .accesses
                .iter()
                .any(|access| matches!(access.shape, WasmAffineShape::Broadcast { .. }))
        });

    let legal_vfs = if let Some(affine) = wasm_affine.as_ref() {
        [2_u16]
            .into_iter()
            .filter(|vf| {
                profile_supports_wasm_affine_candidate(
                    &state.module().profile,
                    &operations,
                    &accesses.accesses,
                    affine,
                    needs_splat,
                ) && *vf == 2
            })
            .collect::<Vec<_>>()
    } else {
        [2_u16, 4, 8, 16]
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
            .collect::<Vec<_>>()
    };
    if legal_vfs.is_empty() {
        return Err("vector-loop-target-profile-is-unavailable".to_string());
    }
    let maximum_uf = state.module().profile.maximum_interleave_factor().min(4);
    let interleavable = shape.diamond.is_none() && reduction.is_none();
    let legal_ufs = if wasm_affine.is_some() {
        vec![1]
    } else {
        [1_u8, 2, 4]
            .into_iter()
            .filter(|uf| *uf <= maximum_uf && (*uf == 1 || interleavable))
            .collect::<Vec<_>>()
    };
    let mut candidates = Vec::new();
    let mut profitability_error = None;
    for vf in legal_vfs {
        for &uf in &legal_ufs {
            let (predicted_cost, minimum_trip) = match candidate_cost_and_threshold(
                &state.module().profile,
                (function, descriptor),
                (vf, uf),
                CandidateCostModelInput {
                    operations: &operations,
                    accesses: &accesses.accesses,
                    splat_inputs: &splat_inputs,
                    version_predicate: version_predicate.as_ref(),
                    wasm_affine: wasm_affine.as_ref(),
                },
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
                reduction: reduction.clone(),
                induction: induction.value,
                bound: induction.bound,
                induction_update,
                vf,
                uf,
                minimum_trip,
                operations: operations.clone(),
                accesses: accesses.accesses.clone(),
                wasm_affine: wasm_affine.clone(),
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

fn discover_wasm_affine_candidate(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    accesses: &super::LoopAccessAnalysis,
    facts: Option<&crate::FactArena>,
) -> Option<WasmAffineCandidate> {
    if descriptor.blocks.len() != 2
        || accesses.accesses.is_empty()
        || !accesses.rejected_instructions.is_empty()
    {
        return None;
    }
    let induction = descriptor.induction.as_ref()?;
    let preheader = descriptor.preheader?;
    let induction_local = loop_body_value_for_header_value(function, descriptor, induction.value)?;
    if induction.type_node != IntegerType::U32
        || induction.start != BigInt::from(0)
        || induction.step != BigInt::from(1)
    {
        return None;
    }
    let dominators = crate::compute_kir_dominators(function);
    let mut candidate_accesses = Vec::with_capacity(accesses.accesses.len());
    let mut setup = BTreeSet::new();
    let mut written_slices = BTreeSet::new();
    let mut indices_by_slice = Vec::new();
    let mut output_regions = BTreeSet::new();
    let mut all_slice_regions = BTreeSet::new();

    for access in &accesses.accesses {
        if !access.slice_base
            || access.element_bytes != 8
            || access.element_type != MirType::Primitive(MirPrimitiveTypeName::F64)
            || access.trip_start != BigInt::from(0)
            || access.trip_bound != induction.bound
        {
            return None;
        }
        let instruction = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == access.instruction)?;
        let (slice, index, kind) = match &instruction.kind {
            KirInstructionKind::Load { place } => match place.as_ref() {
                KirPlace::SliceIndex {
                    slice,
                    index,
                    type_node,
                    ..
                } if *type_node == MirType::Primitive(MirPrimitiveTypeName::F64) => {
                    (*slice, *index, super::LoopMemoryAccessKind::Read)
                }
                _ => return None,
            },
            KirInstructionKind::Store { place, .. } => match place.as_ref() {
                KirPlace::SliceIndex {
                    slice,
                    index,
                    type_node,
                    ..
                } if *type_node == MirType::Primitive(MirPrimitiveTypeName::F64) => {
                    (*slice, *index, super::LoopMemoryAccessKind::Write)
                }
                _ => return None,
            },
            _ => return None,
        };
        if kind != access.kind || !is_u32_value(function, index) {
            return None;
        }
        let source_slice = loop_preheader_value_for_body_value(function, descriptor, slice)?;
        indices_by_slice.push((source_slice, index));
        all_slice_regions.insert((source_slice, access.region));

        let affine_shape = if access.coefficient == BigInt::from(1)
            && access.bias == BigInt::from(0)
        {
            match access.invariant_offset {
                Some(offset)
                    if is_preheader_u32_value(
                        function,
                        descriptor,
                        &dominators,
                        preheader,
                        offset,
                    ) && loop_body_value_for_invariant(function, descriptor, offset)
                        .and_then(|offset_local| {
                            address_add_definition(
                                function,
                                descriptor,
                                index,
                                induction_local,
                                offset_local,
                            )
                        })
                        .is_some() =>
                {
                    let offset_local = loop_body_value_for_invariant(function, descriptor, offset)?;
                    setup.insert(address_add_definition(
                        function,
                        descriptor,
                        index,
                        induction_local,
                        offset_local,
                    )?);
                    WasmAffineShape::Contiguous {
                        offset: Some(offset),
                    }
                }
                Some(_) => return None,
                None if index == induction_local => WasmAffineShape::Contiguous { offset: None },
                None => return None,
            }
        } else if access.coefficient == BigInt::from(0)
            && access.bias == BigInt::from(0)
            && kind == super::LoopMemoryAccessKind::Read
        {
            match access.invariant_offset {
                Some(invariant)
                    if loop_body_value_for_invariant(function, descriptor, invariant)
                        == Some(index)
                        && is_preheader_u32_value(
                            function,
                            descriptor,
                            &dominators,
                            preheader,
                            invariant,
                        ) =>
                {
                    WasmAffineShape::Broadcast { index: invariant }
                }
                Some(_) => return None,
                None if value_integer_constant(function, index) == Some(BigInt::from(0)) => {
                    WasmAffineShape::Broadcast { index }
                }
                None => return None,
            }
        } else {
            return None;
        };

        match (&affine_shape, kind) {
            (WasmAffineShape::Contiguous { .. }, super::LoopMemoryAccessKind::Read) => {}
            (WasmAffineShape::Broadcast { .. }, super::LoopMemoryAccessKind::Read) => {}
            (WasmAffineShape::Contiguous { .. }, super::LoopMemoryAccessKind::Write) => {
                written_slices.insert(source_slice);
                output_regions.insert(access.region);
            }
            (WasmAffineShape::Broadcast { .. }, super::LoopMemoryAccessKind::Write) => {
                return None;
            }
        }
        candidate_accesses.push(WasmAffineAccessShape {
            instruction: access.instruction,
            slice: source_slice,
            kind,
            shape: affine_shape,
        });
    }

    if written_slices.len() != 1 || output_regions.len() != 1 {
        return None;
    }
    let output_slice = *written_slices.first()?;
    let output_indices = indices_by_slice
        .iter()
        .filter(|(slice, _)| *slice == output_slice)
        .map(|(_, index)| *index)
        .collect::<BTreeSet<_>>();
    if output_indices.len() != 1 {
        return None;
    }
    let output_region = *output_regions.first()?;
    let output_shape = candidate_accesses
        .iter()
        .find(|access| {
            access.slice == output_slice && access.kind == super::LoopMemoryAccessKind::Write
        })?
        .shape
        .clone();
    if candidate_accesses
        .iter()
        .any(|access| access.slice == output_slice && access.shape != output_shape)
    {
        return None;
    }
    let source_contiguous_read = candidate_accesses.iter().any(|access| {
        access.slice != output_slice
            && access.kind == super::LoopMemoryAccessKind::Read
            && matches!(access.shape, WasmAffineShape::Contiguous { .. })
    });
    let source_broadcast_read = candidate_accesses.iter().any(|access| {
        access.slice != output_slice
            && access.kind == super::LoopMemoryAccessKind::Read
            && matches!(access.shape, WasmAffineShape::Broadcast { .. })
    });
    if !source_contiguous_read || !source_broadcast_read {
        return None;
    }

    let regions = analyze_regions(function, facts).ok()?;
    for (slice, region) in &all_slice_regions {
        if *slice == output_slice {
            if *region != output_region {
                return None;
            }
            continue;
        }
        let alias = query_alias(&regions, output_region, *region);
        if alias.kind != AliasKind::NoAlias || alias.fact.is_none() {
            return None;
        }
    }

    let covered_accesses = candidate_accesses
        .iter()
        .map(|access| access.instruction)
        .collect::<BTreeSet<_>>();
    for setup_id in &setup {
        let definition = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == *setup_id)?;
        for result in &definition.results {
            let value = result.value;
            let mut saw_use = false;
            for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
                if !instruction_uses_value(instruction, value) {
                    continue;
                }
                let valid_index_use = covered_accesses.contains(&instruction.id)
                    && matches!(
                        &instruction.kind,
                        KirInstructionKind::Load { place }
                            if matches!(place.as_ref(), KirPlace::SliceIndex { index, .. } if *index == value)
                    )
                    || covered_accesses.contains(&instruction.id)
                        && matches!(
                            &instruction.kind,
                            KirInstructionKind::Store { place, .. }
                                if matches!(place.as_ref(), KirPlace::SliceIndex { index, .. } if *index == value)
                        );
                if !valid_index_use {
                    return None;
                }
                saw_use = true;
            }
            for block in &function.blocks {
                if terminator_uses_value(&block.terminator, value) {
                    return None;
                }
            }
            if !saw_use {
                return None;
            }
        }
    }

    let mut ranges = BTreeSet::new();
    for access in &candidate_accesses {
        let affine = accesses
            .accesses
            .iter()
            .find(|source| source.instruction == access.instruction)?;
        let requirement = match access.shape {
            WasmAffineShape::Contiguous { offset } => WasmSliceRangeRequirement {
                slice: access.slice,
                start: offset,
                count: WasmRangeCount::TripBound(induction.bound),
                element_bytes: affine.element_bytes,
            },
            WasmAffineShape::Broadcast { index } => WasmSliceRangeRequirement {
                slice: access.slice,
                start: (value_integer_constant(function, index) != Some(BigInt::from(0)))
                    .then_some(index),
                count: WasmRangeCount::One,
                element_bytes: affine.element_bytes,
            },
        };
        ranges.insert(requirement);
    }
    if ranges.is_empty() || ranges.len() > 3 {
        return None;
    }
    let address_setup = setup.into_iter().collect::<Vec<_>>();
    if address_setup.len() > 2
        || !address_setup.iter().all(|id| {
            function
                .blocks
                .iter()
                .filter(|block| descriptor.blocks.contains(&block.id))
                .flat_map(|block| &block.instructions)
                .any(|instruction| instruction.id == *id)
        })
    {
        return None;
    }
    Some(WasmAffineCandidate {
        accesses: candidate_accesses,
        scalar_address_setup: address_setup,
        range_requirements: ranges.into_iter().collect(),
    })
}

fn is_u32_value(function: &crate::KirFunction, value: crate::ValueId) -> bool {
    value_type(function, value) == Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
}

fn loop_body_value_for_header_value(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    value: crate::ValueId,
) -> Option<crate::ValueId> {
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header)?;
    let param_index = header
        .params
        .iter()
        .position(|param| param.value == value)?;
    let crate::KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return None;
    };
    let body = function
        .blocks
        .iter()
        .find(|block| block.id == then_edge.target && descriptor.blocks.contains(&block.id))?;
    let body_param_index = then_edge
        .args
        .iter()
        .position(|argument| *argument == value)
        .or_else(|| (param_index < then_edge.args.len()).then_some(param_index))?;
    let result = body.params.get(body_param_index)?.value;
    is_u32_value(function, result).then_some(result)
}

fn loop_preheader_value_for_body_value(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    value: crate::ValueId,
) -> Option<crate::ValueId> {
    let body = function
        .blocks
        .iter()
        .find(|block| block.id != descriptor.header && descriptor.blocks.contains(&block.id))?;
    let body_index = body.params.iter().position(|param| param.value == value)?;
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header)?;
    let crate::KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return None;
    };
    if then_edge.target != body.id {
        return None;
    }
    let header_value = *then_edge.args.get(body_index)?;
    let header_index = header
        .params
        .iter()
        .position(|param| param.value == header_value)?;
    let preheader = function
        .blocks
        .iter()
        .find(|block| Some(block.id) == descriptor.preheader)?;
    let crate::KirTerminator::Jump { edge } = &preheader.terminator else {
        return None;
    };
    (edge.target == descriptor.header)
        .then(|| edge.args.get(header_index).copied())
        .flatten()
}

fn loop_body_value_for_invariant(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    value: crate::ValueId,
) -> Option<crate::ValueId> {
    let preheader = function
        .blocks
        .iter()
        .find(|block| Some(block.id) == descriptor.preheader)?;
    let crate::KirTerminator::Jump { edge } = &preheader.terminator else {
        return None;
    };
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header && block.id == edge.target)?;
    if let Some(header_index) = edge.args.iter().position(|argument| *argument == value) {
        let header_value = header.params.get(header_index)?.value;
        return loop_body_value_for_header_value(function, descriptor, header_value);
    }

    // A value computed in an enclosing loop or this loop's preheader can
    // dominate the inner body without being threaded through its header
    // parameters. Preserve that complete SSA value for the range guard.
    let dominators = crate::compute_kir_dominators(function);
    let owner = function.blocks.iter().find_map(|block| {
        (block.params.iter().any(|param| param.value == value)
            || block.instructions.iter().any(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            }))
        .then_some(block.id)
    });
    owner
        .is_none_or(|block| {
            !descriptor.blocks.contains(&block) && dominators.dominates(block, preheader.id)
        })
        .then_some(value)
}

fn is_preheader_u32_value(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    dominators: &crate::KirDominators,
    preheader: BlockId,
    value: crate::ValueId,
) -> bool {
    if !is_u32_value(function, value) {
        return false;
    }
    let owner = function.blocks.iter().find_map(|block| {
        (block.params.iter().any(|param| param.value == value)
            || block.instructions.iter().any(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            }))
        .then_some(block.id)
    });
    owner.is_none_or(|block| {
        !descriptor.blocks.contains(&block) && dominators.dominates(block, preheader)
    })
}

fn address_add_definition(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    index: crate::ValueId,
    induction: crate::ValueId,
    offset: crate::ValueId,
) -> Option<InstructionId> {
    let (block, instruction) = function.blocks.iter().find_map(|block| {
        block
            .instructions
            .iter()
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == index)
            })
            .map(|instruction| (block, instruction))
    })?;
    if !descriptor.blocks.contains(&block.id)
        || !is_u32_value(function, index)
        || !matches!(
            instruction.kind,
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                semantics: KirArithmeticSemantics::Modular,
                left,
                right,
            } if (left == induction && right == offset) || (left == offset && right == induction)
        )
    {
        return None;
    }
    Some(instruction.id)
}

fn value_integer_constant(function: &crate::KirFunction, value: crate::ValueId) -> Option<BigInt> {
    let instruction = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })?;
    match &instruction.kind {
        KirInstructionKind::ConstInt { value } => BigInt::parse_bytes(value.as_bytes(), 10),
        KirInstructionKind::Copy { value } => value_integer_constant(function, *value),
        _ => None,
    }
}

fn instruction_uses_value(instruction: &KirInstruction, value: crate::ValueId) -> bool {
    let mut found = false;
    super::visit_instruction_uses(instruction, &mut |used| found |= used == value);
    found
}

fn terminator_uses_value(terminator: &crate::KirTerminator, value: crate::ValueId) -> bool {
    match terminator {
        crate::KirTerminator::Return {
            value: returned, ..
        } => *returned == Some(value),
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
}

fn has_constant_call_bound(
    module: &crate::KirModule,
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
) -> bool {
    let Some(induction) = descriptor.induction.as_ref() else {
        return false;
    };
    let Some(parameter_index) = function
        .params
        .iter()
        .position(|parameter| parameter.value == induction.bound)
    else {
        return false;
    };
    let mut saw_call = false;
    for caller in &module.functions {
        for instruction in caller.blocks.iter().flat_map(|block| &block.instructions) {
            let KirInstructionKind::Call {
                function_name,
                args,
            } = &instruction.kind
            else {
                continue;
            };
            if function_name != &function.name {
                continue;
            }
            saw_call = true;
            let Some(argument) = args.get(parameter_index) else {
                return false;
            };
            if !is_constant_integer(caller, *argument, &mut BTreeSet::new()) {
                return false;
            }
        }
    }
    saw_call
}

fn is_constant_integer(
    function: &crate::KirFunction,
    value: crate::ValueId,
    active: &mut BTreeSet<crate::ValueId>,
) -> bool {
    if !active.insert(value) {
        return false;
    }
    let result = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
        .is_some_and(|instruction| match &instruction.kind {
            KirInstructionKind::ConstInt { .. } => true,
            KirInstructionKind::Copy { value } => is_constant_integer(function, *value, active),
            _ => false,
        });
    active.remove(&value);
    result
}

struct CandidateCostModelInput<'a> {
    operations: &'a [VectorCandidateOperation],
    accesses: &'a [AffineMemoryAccess],
    splat_inputs: &'a BTreeSet<(crate::ValueId, KirLaneType)>,
    version_predicate: Option<&'a super::TotalVersionPredicate>,
    wasm_affine: Option<&'a WasmAffineCandidate>,
}

fn candidate_cost_and_threshold(
    profile: &crate::KirTargetProfile,
    source: (&crate::KirFunction, &CanonicalLoopDescriptor),
    shape: (u16, u8),
    model: CandidateCostModelInput<'_>,
) -> Result<(KirCostEstimate, u32), String> {
    let (function, descriptor) = source;
    let (vf, uf) = shape;
    let CandidateCostModelInput {
        operations,
        accesses,
        splat_inputs,
        version_predicate,
        wasm_affine,
    } = model;
    let lanes = u8::try_from(vf).map_err(|_| "vector VF exceeds cost schema".to_string())?;
    let mut scalar_iteration = 0_u32;
    let mut vector_lane_chunk = 0_u32;
    let mut reduction_setup_cost = 0_u32;
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
        let persistent_add = profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128)
            && vf == 4
            && uf == 1
            && operation.operation == KirProfileOperation::ReduceAdd;
        if persistent_add {
            // Horizontal fold plus scalar seed combine execute once. Charge
            // initialization and the finalizer edge outside the chunk cost.
            reduction_setup_cost = reduction_setup_cost
                .saturating_add(profile_cost(
                    profile,
                    KirCostKey {
                        operation: KirProfileOperation::ReduceAdd,
                        lane: operation.lane_type,
                        lanes,
                        semantics: operation.semantics,
                        alignment: operation.alignment,
                    },
                )?)
                .saturating_add(profile_cost(
                    profile,
                    KirCostKey {
                        operation: KirProfileOperation::Splat,
                        lane: operation.lane_type,
                        lanes,
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
        }
        vector_lane_chunk = vector_lane_chunk.saturating_add(profile_cost(
            profile,
            KirCostKey {
                operation: if persistent_add {
                    KirProfileOperation::Add
                } else {
                    operation.operation
                },
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
        let affine_shape = wasm_affine.and_then(|affine| {
            affine
                .accesses
                .iter()
                .find(|candidate| candidate.instruction == access.instruction)
        });
        if affine_shape
            .is_some_and(|candidate| matches!(candidate.shape, WasmAffineShape::Broadcast { .. }))
        {
            let scalar_load = profile_cost(
                profile,
                KirCostKey {
                    operation,
                    lane,
                    lanes: 1,
                    semantics: KirCostSemantics::NotApplicable,
                    alignment,
                },
            )?;
            let splat = profile_cost(
                profile,
                KirCostKey {
                    operation: KirProfileOperation::Splat,
                    lane,
                    lanes,
                    semantics: KirCostSemantics::NotApplicable,
                    alignment: KirAlignmentClass::NotApplicable,
                },
            )?;
            vector_lane_chunk = vector_lane_chunk
                .saturating_add(scalar_load)
                .saturating_add(splat);
        } else {
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
    }
    if let Some(affine) = wasm_affine {
        for _ in &affine.scalar_address_setup {
            let setup_cost = profile_cost(
                profile,
                KirCostKey {
                    operation: KirProfileOperation::Add,
                    lane: KirLaneType::U32,
                    lanes: 1,
                    semantics: KirCostSemantics::Modular,
                    alignment: KirAlignmentClass::NotApplicable,
                },
            )?;
            scalar_iteration = scalar_iteration.saturating_add(setup_cost);
            vector_lane_chunk = vector_lane_chunk.saturating_add(setup_cost);
        }
    }
    let mut splat_cost = 0_u32;
    for (value, lane) in splat_inputs {
        let repetitions = if is_loop_local_scalar_constant(function, descriptor, *value) {
            u32::from(uf)
        } else {
            1
        };
        let cost = profile_cost(
            profile,
            KirCostKey {
                operation: KirProfileOperation::Splat,
                lane: *lane,
                lanes,
                semantics: KirCostSemantics::NotApplicable,
                alignment: KirAlignmentClass::NotApplicable,
            },
        )?;
        splat_cost = splat_cost.saturating_add(cost.saturating_mul(repetitions));
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
    let predicate_count = version_predicate
        .map_or(0, |predicate| {
            u32::try_from(predicate.conjuncts.len()).unwrap_or(u32::MAX)
        })
        .saturating_add(wasm_affine.map_or(0, |affine| {
            u32::try_from(affine.range_requirements.len()).unwrap_or(u32::MAX)
        }));
    let predicate_cost = (if predicate_count > 0 {
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
        predicate_base.saturating_add(one.saturating_mul(predicate_count))
    } else {
        predicate_base
    })
    .saturating_add(reduction_setup_cost);
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
    if u64::from(vector_chunk).saturating_mul(100) >= u64::from(scalar_chunk).saturating_mul(80) {
        return Err("vector-profitability-threshold-not-met".to_string());
    }
    // x86 lowering needs four actual vector operations to amortize the explicit
    // KIR loop's control. One group already contains UF operations, so the
    // group floor must scale down with UF. AArch64 retains two complete groups.
    let minimum_groups = match profile.target_identity() {
        KirTargetIdentity::Native { triple } if triple.starts_with("x86_64-") => {
            4_u32.div_ceil(u32::from(uf))
        }
        _ => 2_u32,
    };
    let minimum_trip = (minimum_groups..=1024)
        .map(|groups| groups.saturating_mul(chunk_width))
        .find(|trip| {
            (0..chunk_width).all(|tail| {
                let iterations = trip.saturating_add(tail);
                let scalar = scalar_iteration.saturating_mul(iterations);
                let transformed = vector_chunk
                    .saturating_mul(*trip / chunk_width)
                    .saturating_add(scalar_iteration.saturating_mul(tail))
                    .saturating_add(predicate_cost)
                    .saturating_add(epilogue.saturating_mul(u32::from(tail != 0)));
                u64::from(transformed).saturating_mul(100) <= u64::from(scalar).saturating_mul(80)
            })
        })
        .ok_or_else(|| "vector-profitability-threshold-not-met".to_string())?;
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

fn is_loop_local_scalar_constant(
    function: &crate::KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    value: crate::ValueId,
) -> bool {
    function.blocks.iter().any(|block| {
        descriptor.blocks.contains(&block.id)
            && block.instructions.iter().any(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
                    && matches!(
                        instruction.kind,
                        KirInstructionKind::ConstInt { .. } | KirInstructionKind::ConstFloat { .. }
                    )
            })
    })
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
            },
        );
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
    })
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

fn vector_splat_inputs(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    operations: &[VectorCandidateOperation],
    accesses: &[AffineMemoryAccess],
    diamond: Option<&VectorDiamond>,
    reduction: Option<&VectorReduction>,
) -> BTreeSet<(crate::ValueId, KirLaneType)> {
    let mut vectorized_values = BTreeSet::new();
    for access in accesses
        .iter()
        .filter(|access| access.kind == super::LoopMemoryAccessKind::Read)
    {
        if let Some(value) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == access.instruction)
            .and_then(|instruction| instruction.results.first())
            .map(|result| result.value)
        {
            vectorized_values.insert(value);
        }
    }
    for operation in operations.iter().filter(|operation| {
        !matches!(
            operation.operation,
            KirProfileOperation::Select
                | KirProfileOperation::ReduceAdd
                | KirProfileOperation::ReduceMultiply
        )
    }) {
        if let Some(instruction) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == operation.scalar)
        {
            vectorized_values.extend(instruction.results.iter().map(|result| result.value));
        }
    }
    if let Some(diamond) = diamond
        && let Some(value) = function
            .blocks
            .iter()
            .find(|block| block.id == diamond.merge_block)
            .and_then(|block| block.params.get(diamond.selected_param_index))
            .map(|param| param.value)
    {
        vectorized_values.insert(value);
    }

    let mut splats = BTreeSet::new();
    for operation in operations {
        if operation.operation == KirProfileOperation::Select {
            continue;
        }
        let Some(instruction) = scalar_blocks
            .iter()
            .filter_map(|id| function.blocks.iter().find(|block| block.id == *id))
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == operation.scalar)
        else {
            continue;
        };
        let operands = if let Some(reduction) = reduction
            && reduction.instruction == instruction.id
        {
            match instruction.kind {
                KirInstructionKind::Binary { left, right, .. } if left == reduction.body_value => {
                    vec![right]
                }
                KirInstructionKind::Binary { left, right, .. } if right == reduction.body_value => {
                    vec![left]
                }
                _ => Vec::new(),
            }
        } else {
            match instruction.kind {
                KirInstructionKind::Binary { left, right, .. }
                | KirInstructionKind::Compare { left, right, .. } => vec![left, right],
                KirInstructionKind::Unary { operand, .. } => vec![operand],
                KirInstructionKind::Cast { value, .. } => vec![value],
                _ => Vec::new(),
            }
        };
        for operand in operands {
            let root = scalar_copy_root(function, operand);
            if !vectorized_values.contains(&root) {
                splats.insert((root, operation.lane_type));
            }
        }
    }
    if let Some(diamond) = diamond
        && let Some(lane) = function
            .blocks
            .iter()
            .find(|block| block.id == diamond.merge_block)
            .and_then(|block| block.params.get(diamond.selected_param_index))
            .and_then(|param| param.type_node.as_scalar())
            .and_then(lane_type)
    {
        for operand in diamond_select_arm_values(function, scalar_blocks, diamond) {
            let root = scalar_diamond_arm_root(function, scalar_blocks, diamond, operand);
            if !vectorized_values.contains(&root) {
                splats.insert((root, lane));
            }
        }
    }
    splats
}

fn diamond_select_arm_values(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    diamond: &VectorDiamond,
) -> Vec<crate::ValueId> {
    let Some(merge_block) = function
        .blocks
        .iter()
        .find(|block| block.id == diamond.merge_block)
    else {
        return Vec::new();
    };
    let Some(body) = scalar_blocks
        .first()
        .and_then(|id| function.blocks.iter().find(|block| block.id == *id))
    else {
        return Vec::new();
    };
    let crate::KirTerminator::Branch { .. } = &body.terminator else {
        return Vec::new();
    };
    let Some(then_block) = function
        .blocks
        .iter()
        .find(|block| block.id == diamond.then_block)
    else {
        return Vec::new();
    };
    let Some(else_block) = function
        .blocks
        .iter()
        .find(|block| block.id == diamond.else_block)
    else {
        return Vec::new();
    };
    let crate::KirTerminator::Jump { edge: then_merge } = &then_block.terminator else {
        return Vec::new();
    };
    let crate::KirTerminator::Jump { edge: else_merge } = &else_block.terminator else {
        return Vec::new();
    };
    if merge_block
        .params
        .get(diamond.selected_param_index)
        .is_none()
    {
        return Vec::new();
    }
    then_merge
        .args
        .get(diamond.selected_param_index)
        .copied()
        .into_iter()
        .chain(else_merge.args.get(diamond.selected_param_index).copied())
        .collect()
}

fn scalar_diamond_arm_root(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    diamond: &VectorDiamond,
    value: crate::ValueId,
) -> crate::ValueId {
    let mut value = value;
    let mut visited = BTreeSet::new();
    while visited.insert(value) {
        if let Some(source) = function
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
            value = source;
            continue;
        }
        let Some((arm, index)) = [diamond.then_block, diamond.else_block]
            .into_iter()
            .find_map(|arm_id| {
                function
                    .blocks
                    .iter()
                    .find(|block| block.id == arm_id)
                    .and_then(|block| block.params.iter().position(|param| param.value == value))
                    .map(|index| (arm_id, index))
            })
        else {
            break;
        };
        let Some(body) = scalar_blocks
            .first()
            .and_then(|id| function.blocks.iter().find(|block| block.id == *id))
        else {
            break;
        };
        let crate::KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &body.terminator
        else {
            break;
        };
        let entry = if then_edge.target == arm {
            then_edge
        } else if else_edge.target == arm {
            else_edge
        } else {
            break;
        };
        let Some(source) = entry.args.get(index).copied() else {
            break;
        };
        value = source;
    }
    value
}

fn scalar_copy_root(function: &crate::KirFunction, value: crate::ValueId) -> crate::ValueId {
    let mut value = value;
    let mut visited = BTreeSet::new();
    while visited.insert(value) {
        let source = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            })
            .and_then(|instruction| match instruction.kind {
                KirInstructionKind::Copy { value } => Some(value),
                _ => None,
            });
        let Some(source) = source else {
            break;
        };
        value = source;
    }
    value
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

fn profile_supports_wasm_affine_candidate(
    profile: &crate::KirTargetProfile,
    operations: &[VectorCandidateOperation],
    accesses: &[AffineMemoryAccess],
    affine: &WasmAffineCandidate,
    needs_splat: bool,
) -> bool {
    let legal = |key: KirCostKey| {
        matches!(
            profile.operation_availability(&key),
            Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1
        )
    };
    let lanes = 2;
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
        && !legal(KirCostKey {
            operation: KirProfileOperation::Splat,
            lane: KirLaneType::F64,
            lanes,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        })
    {
        return false;
    }
    if affine.scalar_address_setup.iter().any(|_| {
        !legal(KirCostKey {
            operation: KirProfileOperation::Add,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::Modular,
            alignment: KirAlignmentClass::NotApplicable,
        })
    }) {
        return false;
    }
    accesses.iter().all(|access| {
        let Some(candidate) = affine
            .accesses
            .iter()
            .find(|candidate| candidate.instruction == access.instruction)
        else {
            return false;
        };
        let Some(lane) = lane_type(&access.element_type) else {
            return false;
        };
        let Ok(alignment) = u16::try_from(access.element_bytes) else {
            return false;
        };
        let operation = if access.kind == super::LoopMemoryAccessKind::Read {
            KirProfileOperation::Load
        } else {
            KirProfileOperation::Store
        };
        let lanes = if matches!(candidate.shape, WasmAffineShape::Broadcast { .. }) {
            if access.kind != super::LoopMemoryAccessKind::Read {
                return false;
            }
            1
        } else {
            2
        };
        legal(KirCostKey {
            operation,
            lane,
            lanes,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::Bytes(alignment),
        })
    })
}
