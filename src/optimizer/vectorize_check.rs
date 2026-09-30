use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, CandidateBudgetCharge, FunctionId, InstructionId, KirAlignmentClass, KirCostEstimate,
    KirCostKey, KirCostSemantics, KirInstruction, KirInstructionKind, KirLaneType,
    KirOperationAvailability, KirProfileOperation, KirTargetIdentity, KirTerminator, KirValueType,
    LoopId, MemoryRegionId, MirBinaryOp, MirPrimitiveTypeName, MirType, TransactionCheckError,
    ValueId, VectorEpilogue, VectorMemoryAccessKind, VectorizationPlan, compute_kir_dominators,
    kir_function_units, validate_kir_module, validate_vectorization_plan,
};

use super::KirVerifiedProgramState;

#[derive(Debug, Clone)]
struct CheckedVectorOperation {
    scalar: InstructionId,
    operation: KirProfileOperation,
    lane_type: KirLaneType,
    semantics: KirCostSemantics,
}

#[derive(Debug, Clone)]
struct CheckedVectorAccess {
    instruction: InstructionId,
    kind: CheckedMemoryAccessKind,
    region: MemoryRegionId,
    base: ValueId,
    element_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckedMemoryAccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone)]
struct CheckedVersionPredicate {
    address_bits: u8,
    conjuncts: Vec<CheckedVersionConjunct>,
}

#[derive(Debug, Clone)]
enum CheckedVersionConjunct {
    AddressIntervalsDisjoint {
        left: ValueId,
        left_element_bytes: u32,
        right: ValueId,
        right_element_bytes: u32,
    },
}

#[derive(Debug, Clone)]
struct CheckedVectorReduction {
    header_value: ValueId,
    body_value: ValueId,
    instruction: InstructionId,
    binary_op: MirBinaryOp,
}

#[derive(Debug, Clone)]
struct CheckedVectorDiamond {
    then_block: BlockId,
    else_block: BlockId,
    merge_block: BlockId,
    condition_instruction: InstructionId,
    selected_param_index: usize,
}

#[derive(Debug, Clone)]
struct CheckedVectorSource {
    function: FunctionId,
    preheader: BlockId,
    header: BlockId,
    body: BlockId,
    exit: BlockId,
    scalar_blocks: Vec<BlockId>,
    diamond: Option<CheckedVectorDiamond>,
    reduction: Option<CheckedVectorReduction>,
    induction: ValueId,
    bound: ValueId,
    vf: u16,
    uf: u8,
    minimum_trip: u32,
    operations: Vec<CheckedVectorOperation>,
    accesses: Vec<CheckedVectorAccess>,
    version_predicate: Option<CheckedVersionPredicate>,
    predicted_cost: KirCostEstimate,
    affine: Option<CheckedWasmAffine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckedWasmAddress {
    Contiguous(Option<ValueId>),
    Broadcast(ValueId),
}

#[derive(Debug, Clone)]
struct CheckedWasmAffine {
    addresses: BTreeMap<InstructionId, CheckedWasmAddress>,
    setup: BTreeSet<InstructionId>,
    ranges: BTreeSet<crate::WasmSliceRangeRequirement>,
}

struct CheckedMatmulInterleaveSource<'a> {
    function: &'a crate::KirFunction,
    header_id: BlockId,
    body_id: BlockId,
    induction: ValueId,
    scalar_blocks: &'a [BlockId],
    has_diamond: bool,
    has_reduction: bool,
    bound: ValueId,
    plan: &'a VectorizationPlan,
    affine: &'a CheckedWasmAffine,
    operations: &'a [CheckedVectorOperation],
    accesses: &'a [CheckedVectorAccess],
}

#[derive(Debug, Clone, Copy)]
struct CheckedMatmulSourceInstructionIds {
    broadcast: InstructionId,
    output_read: InstructionId,
    b_read: InstructionId,
    output_write: InstructionId,
    multiply: InstructionId,
    add: InstructionId,
}

fn is_wasm_affine_plan(plan: &VectorizationPlan) -> bool {
    !plan.broadcast_groups.is_empty()
        || plan
            .predicates
            .iter()
            .any(|predicate| matches!(predicate, crate::VectorPredicate::WasmSliceRange { .. }))
}

pub fn check_vectorization_trial_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &VectorizationPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    let compiler = |message: &str| Err(TransactionCheckError::compiler(message));
    validate_vectorization_plan(plan, &pre_state.module().profile)
        .map_err(TransactionCheckError::compiler)?;
    if plan.pre_state.kir_digest != pre_state.kir_digest()
        || plan.pre_state.profile_digest != pre_state.module().profile.digest_hex()
        || plan.pre_state.evidence_generation != pre_state.evidence_generation()
    {
        return compiler("vector trial pre-state identity is stale");
    }
    let before_module = pre_state.module();
    let after_module = trial.module();
    if before_module.config != after_module.config
        || before_module.profile != after_module.profile
        || before_module.entry != after_module.entry
        || before_module.structs != after_module.structs
        || before_module
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>()
            != after_module
                .functions
                .iter()
                .map(|function| function.id)
                .collect::<Vec<_>>()
    {
        return compiler("vector trial changed module configuration or identity");
    }
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
        .ok_or_else(|| TransactionCheckError::compiler("vector original function is missing"))?;
    if plan.pre_state.frozen_kir_units != kir_function_units(original) {
        return compiler("vector frozen function size is false");
    }
    let candidate = reconstruct_vector_source_independently(pre_state, trial, plan)?;
    let shared_matmul_broadcast = candidate.uf > 1
        && candidate.affine.as_ref().is_some_and(|affine| {
            checked_wasm_matmul_interleave(CheckedMatmulInterleaveSource {
                function: original,
                header_id: candidate.header,
                body_id: candidate.body,
                induction: candidate.induction,
                scalar_blocks: &candidate.scalar_blocks,
                has_diamond: candidate.diamond.is_some(),
                has_reduction: candidate.reduction.is_some(),
                bound: candidate.bound,
                plan,
                affine,
                operations: &candidate.operations,
                accesses: &candidate.accesses,
            })
        });
    let persistent_add = pre_state.module().profile.wasm_features()
        == Some(crate::KirWasmFeatures::Simd128)
        && candidate.vf == 4
        && candidate.uf == 1
        && candidate
            .reduction
            .as_ref()
            .is_some_and(|reduction| reduction.binary_op == MirBinaryOp::Add);
    if plan.cost != candidate.predicted_cost {
        return Err(TransactionCheckError::compiler(format!(
            "vector cost is not independently reproducible: plan {:?}, reconstructed {:?}",
            plan.cost, candidate.predicted_cost
        )));
    }
    if plan.operations.len()
        != candidate
            .operations
            .len()
            .saturating_mul(usize::from(candidate.uf))
    {
        return compiler("vector operations do not cover every unrolled source operation");
    }
    if plan.memory_groups.len() + plan.broadcast_groups.len()
        != candidate
            .accesses
            .len()
            .saturating_mul(usize::from(candidate.uf))
            .saturating_sub(if shared_matmul_broadcast {
                usize::from(candidate.uf).saturating_sub(1)
            } else {
                0
            })
    {
        return compiler("vector memory groups do not close the unrolled source footprint");
    }
    let transformed = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| TransactionCheckError::compiler("vector trial function is missing"))?;
    if transformed.name != original.name
        || transformed.exported != original.exported
        || transformed.params != original.params
        || transformed.return_type != original.return_type
        || transformed.regions != original.regions
        || transformed.initial_memory != original.initial_memory
    {
        return compiler("vector trial changed target function ABI or memory metadata");
    }
    for source in pre_state
        .module()
        .functions
        .iter()
        .filter(|function| function.id != candidate.function)
    {
        if trial
            .module()
            .functions
            .iter()
            .find(|function| function.id == source.id)
            != Some(source)
        {
            return compiler("vector trial changed a different function");
        }
    }
    for block_id in std::iter::once(candidate.header)
        .chain(candidate.scalar_blocks.iter().copied())
        .chain(std::iter::once(candidate.exit))
    {
        let before = original.blocks.iter().find(|block| block.id == block_id);
        let after = transformed.blocks.iter().find(|block| block.id == block_id);
        if before.is_none() || before != after {
            return compiler("vector scalar fallback identity is not preserved");
        }
    }
    let preheader_before = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.preheader)
        .expect("candidate preheader exists");
    let preheader_after = transformed
        .blocks
        .iter()
        .find(|block| block.id == candidate.preheader)
        .ok_or_else(|| TransactionCheckError::compiler("vector preheader disappeared"))?;
    if preheader_after.id != preheader_before.id
        || preheader_after.label != preheader_before.label
        || preheader_after.params != preheader_before.params
        || preheader_after.memory_params != preheader_before.memory_params
    {
        return compiler("vector trial changed preheader parameters or identity");
    }
    if preheader_after
        .instructions
        .get(..preheader_before.instructions.len())
        != Some(preheader_before.instructions.as_slice())
        || !(preheader_before.instructions.len() + 3 + 2 * usize::from(persistent_add)
            ..=preheader_before.instructions.len()
                + 6
                + 2 * usize::from(persistent_add)
                + candidate.affine.as_ref().map_or(0, |affine| {
                    let endpoints = affine
                        .addresses
                        .values()
                        .filter_map(|address| match address {
                            CheckedWasmAddress::Contiguous(Some(value)) => Some(*value),
                            _ => None,
                        })
                        .collect::<BTreeSet<_>>()
                        .len();
                    8 + endpoints.saturating_sub(2)
                        + 2 * usize::from(affine.ranges.iter().any(|r| {
                            matches!(r.count, crate::WasmRangeCount::ScaledInvariant { .. })
                        }))
                }))
            .contains(&preheader_after.instructions.len())
    {
        return compiler("vector preheader predicate is not a closed append-only rewrite");
    }
    let KirTerminator::Jump {
        edge: original_entry,
    } = &preheader_before.terminator
    else {
        return compiler("vector original preheader is not a jump");
    };
    let KirTerminator::Branch {
        condition: entry_condition,
        then_edge: vector_entry,
        else_edge: scalar_entry,
    } = &preheader_after.terminator
    else {
        return compiler("vector preheader does not version the scalar loop");
    };
    if scalar_entry != original_entry {
        return compiler("vector short-trip fallback is not the original scalar edge");
    }
    let vector_header = transformed
        .blocks
        .iter()
        .find(|block| block.id == vector_entry.target && block.label == "loop_simd_header")
        .ok_or_else(|| TransactionCheckError::compiler("vector header block is missing"))?;
    let KirTerminator::Branch {
        condition: vector_condition,
        then_edge: vector_body_edge,
        else_edge: epilogue_edge,
    } = &vector_header.terminator
    else {
        return compiler("vector header does not branch to body and epilogue");
    };
    if vector_entry.args.get(..original_entry.args.len()) != Some(original_entry.args.as_slice())
        || vector_entry.args.len() != original_entry.args.len() + usize::from(persistent_add)
        || vector_entry.memory_args != original_entry.memory_args
    {
        return compiler("vector entry does not preserve the original loop state");
    }
    if !persistent_add && epilogue_edge.target != candidate.header {
        return compiler("vector epilogue does not enter the original scalar header");
    }
    let original_header = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .expect("reproduced vector header");
    // The fast loop skips source-header instructions until its scalar tail.
    // Even a pure integer divide can trap before the scalar loop's first
    // store, so effect metadata alone does not prove this reorder safe.
    if original_header.instructions.iter().any(|instruction| {
        instruction.memory.is_some()
            || instruction.effect.is_some()
            || !matches!(
                instruction.kind,
                KirInstructionKind::ConstInt { .. }
                    | KirInstructionKind::ConstFloat { .. }
                    | KirInstructionKind::ConstBool { .. }
                    | KirInstructionKind::Copy { .. }
                    | KirInstructionKind::Compare { .. }
                    | KirInstructionKind::SliceData { .. }
                    | KirInstructionKind::SliceLen { .. }
                    | KirInstructionKind::Binary {
                        op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
                        semantics: crate::KirArithmeticSemantics::Modular,
                        ..
                    }
            )
    }) {
        return compiler("vector source header may trap before vector stores");
    }
    let induction_index = original_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .ok_or_else(|| TransactionCheckError::compiler("vector header induction is missing"))?;
    let vector_induction = vector_header
        .params
        .get(induction_index)
        .map(|param| param.value)
        .ok_or_else(|| {
            TransactionCheckError::compiler("vector header induction parameter is missing")
        })?;
    let [vector_condition_instruction] = vector_header.instructions.as_slice() else {
        return compiler("vector header condition is not an exact single comparison");
    };
    let vector_limit = vector_condition_instruction
        .results
        .iter()
        .any(|result| result.value == *vector_condition)
        .then(|| match vector_condition_instruction.kind {
            KirInstructionKind::Compare {
                op: crate::MirCompareOp::Le,
                left,
                right,
            } if left == vector_induction => Some(right),
            _ => None,
        })
        .flatten()
        .ok_or_else(|| {
            TransactionCheckError::compiler("vector header limit comparison is false")
        })?;
    let chunk_width = u32::from(candidate.vf).saturating_mul(u32::from(candidate.uf));
    let limit_instruction = preheader_after
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == vector_limit)
        })
        .ok_or_else(|| TransactionCheckError::compiler("vector limit is not in the preheader"))?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Sub,
        left: limit_bound,
        right: limit_stride,
        semantics: crate::KirArithmeticSemantics::Modular,
    } = limit_instruction.kind
    else {
        return compiler("vector limit is not bound minus VF*UF");
    };
    if !entry_bound_matches(
        original,
        original_header,
        preheader_before,
        preheader_after,
        original_entry,
        candidate.bound,
        limit_bound,
    ) || integer_constant(transformed, limit_stride) != Some(i128::from(chunk_width))
    {
        return compiler("vector limit is not bound minus VF*UF");
    }
    let vector_body = transformed
        .blocks
        .iter()
        .find(|block| block.id == vector_body_edge.target && block.label == "loop_simd_body")
        .ok_or_else(|| TransactionCheckError::compiler("vector body block is missing"))?;
    if !matches!(
        vector_body.terminator,
        KirTerminator::Jump { ref edge } if edge.target == vector_header.id
    ) {
        return compiler("vector body does not cover the next VF chunk");
    }
    let original_body = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.body)
        .ok_or_else(|| TransactionCheckError::compiler("vector source body is missing"))?;
    let body_induction_index =
        original_header_body_induction_index(original, &candidate, original_body)
            .map_err(TransactionCheckError::compiler)?;
    let compact_interleaved_body =
        candidate.uf > 1 && candidate.diamond.is_none() && candidate.reduction.is_none();
    let (scalar_chunk_zero, header_induction) = if compact_interleaved_body {
        if !vector_body.params.is_empty() || !vector_body_edge.args.is_empty() {
            return compiler("compact vector body retained redundant block parameters");
        }
        (vector_induction, vector_induction)
    } else {
        let scalar_chunk_zero = vector_body
            .params
            .get(body_induction_index)
            .map(|param| param.value)
            .ok_or_else(|| TransactionCheckError::compiler("vector body induction is missing"))?;
        let header_induction = vector_body_edge
            .args
            .get(body_induction_index)
            .copied()
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector header induction edge is missing")
            })?;
        (scalar_chunk_zero, header_induction)
    };
    if header_induction != vector_induction {
        return compiler("vector body induction does not originate at the vector header");
    }
    let constant_value = |value| {
        vector_body.instructions.iter().find_map(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
                .then(|| match &instruction.kind {
                    KirInstructionKind::ConstInt { value } => value.parse::<u32>().ok(),
                    _ => None,
                })
                .flatten()
        })
    };
    let next_chunk_start = |start| {
        let starts = vector_body
            .instructions
            .iter()
            .filter_map(|instruction| {
                let result = instruction.results.first()?.value;
                let KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Add,
                    left,
                    right,
                    semantics: crate::KirArithmeticSemantics::Modular,
                } = instruction.kind
                else {
                    return None;
                };
                (left == start && constant_value(right) == Some(u32::from(candidate.vf)))
                    .then_some(result)
            })
            .collect::<Vec<_>>();
        let [start] = starts.as_slice() else {
            return Err(TransactionCheckError::compiler(
                "vector UF chunk stride is missing or ambiguous",
            ));
        };
        Ok(*start)
    };
    let mut chunk_starts = vec![scalar_chunk_zero];
    for _ in 1..candidate.uf {
        chunk_starts.push(next_chunk_start(
            *chunk_starts.last().expect("initial chunk"),
        )?);
    }
    let KirTerminator::Jump {
        edge: vector_backedge,
    } = &vector_body.terminator
    else {
        return compiler("vector body lost its backedge");
    };
    let next_induction = vector_backedge
        .args
        .get(induction_index)
        .copied()
        .ok_or_else(|| TransactionCheckError::compiler("vector backedge induction is missing"))?;
    if next_chunk_start(*chunk_starts.last().expect("initial chunk"))? != next_induction {
        return compiler("vector backedge does not advance by VF*UF");
    }
    let old_region_count = original.vector_regions.len();
    if transformed.vector_regions.get(..old_region_count)
        != Some(original.vector_regions.as_slice())
    {
        return compiler("vector trial changed a pre-existing vector region");
    }
    let [region] = &transformed.vector_regions[old_region_count..] else {
        return compiler("vector trial must create exactly one owned vector region");
    };
    if original
        .vector_regions
        .iter()
        .any(|original| original.id == region.id)
    {
        return compiler("vector trial reused a pre-existing vector region identity");
    }
    let expected_region_blocks = if persistent_add {
        vec![
            candidate.preheader,
            vector_header.id,
            vector_body.id,
            epilogue_edge.target,
        ]
    } else {
        vec![vector_body.id]
    };
    if region.blocks != expected_region_blocks {
        return compiler("vector region ownership is not exact");
    }

    let mut vectors = BTreeSet::new();
    let mut operation_identities = BTreeSet::new();
    for mapping in &plan.operations {
        let expected = candidate
            .operations
            .iter()
            .find(|expected| {
                expected.scalar == mapping.scalar && expected.operation == mapping.operation
            })
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector operation source identity is false")
            })?;
        if mapping.scalar != expected.scalar
            || mapping.operation != expected.operation
            || mapping.lane_type != expected.lane_type
            || mapping.semantics != expected.semantics
            || mapping.unroll_index >= candidate.uf
            || !operation_identities.insert((
                mapping.scalar,
                mapping.operation,
                mapping.unroll_index,
            ))
            || !vectors.insert(mapping.vector)
            || mapping.lanes.len() != usize::from(plan.vf)
            || mapping.lanes.iter().enumerate().any(|(index, lane)| {
                usize::from(lane.lane) != index
                    || lane.scalar_iteration
                        != u32::from(mapping.unroll_index)
                            .saturating_mul(u32::from(plan.vf))
                            .saturating_add(u32::try_from(index).unwrap_or(u32::MAX))
            })
        {
            return compiler("vector lane or operation mapping is false");
        }
        let instruction = vector_body
            .instructions
            .iter()
            .find(|instruction| instruction.id == mapping.vector)
            .ok_or_else(|| TransactionCheckError::compiler("mapped vector operation is missing"))?;
        let scalar = original
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == mapping.scalar)
            .ok_or_else(|| TransactionCheckError::compiler("mapped scalar operation is missing"))?;
        let operation_matches = matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Add,
                    semantics: scalar_semantics,
                    ..
                },
                KirInstructionKind::VectorBinary {
                    op: crate::KirVectorBinaryOp::Add,
                    semantics: vector_semantics,
                    no_failure_proof: None,
                    ..
                },
                crate::KirProfileOperation::Add
            ) if scalar_semantics == vector_semantics
        ) || matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Sub,
                    semantics: scalar_semantics,
                    ..
                },
                KirInstructionKind::VectorBinary {
                    op: crate::KirVectorBinaryOp::Subtract,
                    semantics: vector_semantics,
                    no_failure_proof: None,
                    ..
                },
                crate::KirProfileOperation::Subtract
            ) if scalar_semantics == vector_semantics
        ) || matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Mul,
                    semantics: scalar_semantics,
                    ..
                },
                KirInstructionKind::VectorBinary {
                    op: crate::KirVectorBinaryOp::Multiply,
                    semantics: vector_semantics,
                    no_failure_proof: None,
                    ..
                },
                crate::KirProfileOperation::Multiply
            ) if scalar_semantics == vector_semantics
        ) || matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Div,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    ..
                },
                KirInstructionKind::VectorBinary {
                    op: crate::KirVectorBinaryOp::Divide,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    no_failure_proof: None,
                    ..
                },
                crate::KirProfileOperation::Divide
            )
        ) || matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Unary {
                    op: crate::MirUnaryOp::Neg,
                    semantics: scalar_semantics,
                    ..
                },
                KirInstructionKind::VectorUnary {
                    op: crate::KirVectorUnaryOp::Negate,
                    semantics: vector_semantics,
                    no_failure_proof: None,
                    ..
                },
                crate::KirProfileOperation::Negate
            ) if scalar_semantics == vector_semantics
        ) || matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Cast {
                    op: crate::MirCastOp::I32ToF64,
                    ..
                },
                KirInstructionKind::VectorCast {
                    op: crate::KirVectorCastOp::I32ToF64,
                    ..
                },
                crate::KirProfileOperation::Cast
            ) | (
                KirInstructionKind::Cast {
                    op: crate::MirCastOp::U32ToF64,
                    ..
                },
                KirInstructionKind::VectorCast {
                    op: crate::KirVectorCastOp::U32ToF64,
                    ..
                },
                crate::KirProfileOperation::Cast
            )
        ) || matches!(
            (&scalar.kind, &instruction.kind, mapping.operation),
            (
                KirInstructionKind::Compare { op: scalar_op, .. },
                KirInstructionKind::VectorCompare { op: vector_op, .. },
                crate::KirProfileOperation::Compare
            ) if scalar_op == vector_op
        ) || matches!(
            (&instruction.kind, mapping.operation),
            (
                KirInstructionKind::VectorSelect { .. },
                crate::KirProfileOperation::Select
            ) | (
                KirInstructionKind::VectorReduce {
                    op: crate::KirVectorReductionOp::ModularAdd,
                    ..
                },
                crate::KirProfileOperation::ReduceAdd
            ) | (
                KirInstructionKind::VectorReduce {
                    op: crate::KirVectorReductionOp::ModularMultiply,
                    ..
                },
                crate::KirProfileOperation::ReduceMultiply
            )
        );
        let operation_matches = operation_matches
            || (persistent_add
                && mapping.operation == KirProfileOperation::ReduceAdd
                && matches!(
                    instruction.kind,
                    KirInstructionKind::VectorBinary {
                        op: crate::KirVectorBinaryOp::Add,
                        semantics: crate::KirArithmeticSemantics::Modular,
                        no_failure_proof: None,
                        ..
                    }
                ));
        if !operation_matches {
            return compiler("mapped vector instruction has the wrong operation family");
        }
    }
    if persistent_add {
        check_persistent_modular_add(
            original,
            transformed,
            &candidate,
            preheader_after,
            original_entry,
            vector_entry,
            vector_header,
            vector_body,
            vector_body_edge,
            epilogue_edge,
            plan,
        )?;
    } else if let Some(reduction) = &candidate.reduction {
        let mapping = plan
            .operations
            .iter()
            .find(|mapping| mapping.scalar == reduction.instruction)
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector reduction mapping is missing")
            })?;
        let reduce = vector_body
            .instructions
            .iter()
            .find(|instruction| instruction.id == mapping.vector)
            .ok_or_else(|| TransactionCheckError::compiler("vector reduction is missing"))?;
        let KirInstructionKind::VectorReduce {
            vector,
            semantics: crate::KirArithmeticSemantics::Modular,
            ..
        } = reduce.kind
        else {
            return compiler("vector reduction arithmetic semantics are false");
        };
        let reduced = reduce
            .results
            .first()
            .map(|result| result.value)
            .ok_or_else(|| TransactionCheckError::compiler("vector reduction result is missing"))?;
        let original_body = original
            .blocks
            .iter()
            .find(|block| block.id == candidate.body)
            .expect("reproduced vector body");
        let body_param_index = original_body
            .params
            .iter()
            .position(|param| param.value == reduction.body_value)
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector reduction body recurrence is missing")
            })?;
        let accumulator = vector_body
            .params
            .get(body_param_index)
            .map(|param| param.value)
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector reduction accumulator is missing")
            })?;
        let combine = vector_body
            .instructions
            .iter()
            .find(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::Binary {
                        op,
                        left,
                        right,
                        semantics: crate::KirArithmeticSemantics::Modular,
                    } if op == reduction.binary_op && left == accumulator && right == reduced
                )
            })
            .and_then(|instruction| instruction.results.first())
            .map(|result| result.value)
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector reduction scalar combine is false")
            })?;
        let original_header = original
            .blocks
            .iter()
            .find(|block| block.id == candidate.header)
            .expect("reproduced vector header");
        let header_param_index = original_header
            .params
            .iter()
            .position(|param| param.value == reduction.header_value)
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector reduction header recurrence is missing")
            })?;
        let KirTerminator::Jump { edge: backedge } = &vector_body.terminator else {
            return compiler("vector reduction body does not return to the vector header");
        };
        if backedge.args.get(header_param_index) != Some(&combine) {
            return compiler("vector reduction does not carry the combined value");
        }
        let vector_source_is_defined = vector_body.instructions.iter().any(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == vector)
                && matches!(
                    instruction.kind,
                    KirInstructionKind::VectorLoad { .. }
                        | KirInstructionKind::VectorBinary { .. }
                        | KirInstructionKind::VectorCast { .. }
                        | KirInstructionKind::VectorSelect { .. }
                )
        });
        if !vector_source_is_defined {
            return compiler("vector reduction lane source is not a vectorized operation");
        }
    }
    if let Some(diamond) = &candidate.diamond {
        let compare_mapping = plan
            .operations
            .iter()
            .find(|mapping| mapping.operation == crate::KirProfileOperation::Compare)
            .ok_or_else(|| TransactionCheckError::compiler("vector diamond compare is missing"))?;
        let select_mapping = plan
            .operations
            .iter()
            .find(|mapping| mapping.operation == crate::KirProfileOperation::Select)
            .ok_or_else(|| TransactionCheckError::compiler("vector diamond select is missing"))?;
        let compare = vector_body
            .instructions
            .iter()
            .find(|instruction| instruction.id == compare_mapping.vector)
            .and_then(|instruction| instruction.results.first())
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector diamond compare result is missing")
            })?;
        let select = vector_body
            .instructions
            .iter()
            .find(|instruction| instruction.id == select_mapping.vector)
            .ok_or_else(|| TransactionCheckError::compiler("vector diamond select is missing"))?;
        let KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } = &select.kind
        else {
            return compiler("vector diamond mapped select has the wrong kind");
        };
        let then_block = original
            .blocks
            .iter()
            .find(|block| block.id == diamond.then_block)
            .expect("reproduced diamond then block");
        let else_block = original
            .blocks
            .iter()
            .find(|block| block.id == diamond.else_block)
            .expect("reproduced diamond else block");
        let KirTerminator::Jump { edge: then_edge } = &then_block.terminator else {
            return compiler("vector diamond then arm no longer reconverges");
        };
        let KirTerminator::Jump { edge: else_edge } = &else_block.terminator else {
            return compiler("vector diamond else arm no longer reconverges");
        };
        let then_scalar = then_edge.args[diamond.selected_param_index];
        let else_scalar = else_edge.args[diamond.selected_param_index];
        let defining_instruction = |value| {
            original
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
        };
        let expected_arm = |scalar| {
            let root =
                independent_diamond_arm_root(original, &candidate.scalar_blocks, diamond, scalar);
            let vectorized_operation = defining_instruction(root).and_then(|instruction| {
                plan.operations
                    .iter()
                    .find(|mapping| mapping.scalar == instruction)
                    .and_then(|mapping| {
                        vector_body
                            .instructions
                            .iter()
                            .find(|instruction| instruction.id == mapping.vector)
                            .and_then(|instruction| instruction.results.first())
                            .map(|result| result.value)
                    })
                    .or_else(|| {
                        plan.memory_groups
                            .iter()
                            .find(|group| group.scalar_instructions.contains(&instruction))
                            .and_then(|group| {
                                vector_body
                                    .instructions
                                    .iter()
                                    .find(|instruction| instruction.id == group.vector_instruction)
                                    .and_then(|instruction| instruction.results.first())
                                    .map(|result| result.value)
                            })
                    })
            });
            vectorized_operation.or_else(|| {
                let scalar_source = independent_diamond_scalar_source(
                    original,
                    transformed,
                    original_header,
                    original_body,
                    vector_header,
                    vector_body,
                    root,
                )?;
                vector_body.instructions.iter().find_map(|instruction| {
                    let KirInstructionKind::VectorSplat { scalar, .. } = instruction.kind else {
                        return None;
                    };
                    let result = instruction.results.first()?;
                    (scalar == scalar_source
                        && result.type_node
                            == (KirValueType::FixedVector {
                                lane: select_mapping.lane_type,
                                lanes: candidate.vf,
                            }))
                    .then_some(result.value)
                })
            })
        };
        let expected_true = expected_arm(then_scalar);
        let expected_false = expected_arm(else_scalar);
        if *mask != compare.value
            || expected_true != Some(*when_true)
            || expected_false != Some(*when_false)
        {
            return compiler("vector diamond mask or selected arm mapping is false");
        }
    }
    let mut scalar_memory = BTreeSet::new();
    for group in &plan.memory_groups {
        let [scalar] = group.scalar_instructions.as_slice() else {
            return compiler("vector memory group is not one source access");
        };
        if group.unroll_index >= candidate.uf
            || !scalar_memory.insert((*scalar, group.unroll_index))
        {
            return compiler("vector memory scalar access is duplicated");
        }
        let expected = candidate
            .accesses
            .iter()
            .find(|access| access.instruction == *scalar)
            .ok_or_else(|| TransactionCheckError::compiler("vector memory source is false"))?;
        if group.region != expected.region
            || group.access
                != if expected.kind == CheckedMemoryAccessKind::Read {
                    VectorMemoryAccessKind::Read
                } else {
                    VectorMemoryAccessKind::Write
                }
        {
            return compiler("vector memory region or access kind is false");
        }
        let emitted = vector_body
            .instructions
            .iter()
            .find(|instruction| instruction.id == group.vector_instruction)
            .ok_or_else(|| {
                TransactionCheckError::compiler("vector memory instruction is missing")
            })?;
        let emitted_start = match &emitted.kind {
            KirInstructionKind::VectorLoad { access, .. }
            | KirInstructionKind::VectorStore { access, .. } => access.start,
            _ => return compiler("vector memory instruction kind is false"),
        };
        if candidate.affine.is_none()
            && chunk_starts.get(usize::from(group.unroll_index)) != Some(&emitted_start)
        {
            return compiler("vector memory group is mapped to the wrong UF chunk");
        }
        if !matches!(
            (&emitted.kind, group.access),
            (
                KirInstructionKind::VectorLoad { .. },
                VectorMemoryAccessKind::Read
            ) | (
                KirInstructionKind::VectorStore { .. },
                VectorMemoryAccessKind::Write
            )
        ) {
            return compiler("vector memory instruction kind is false");
        }
    }
    if let Some(affine) = &candidate.affine {
        if candidate.uf > 1 {
            check_wasm_direct_map_interleaved_emission(
                original,
                transformed,
                &candidate,
                affine,
                original_header,
                original_body,
                vector_header,
                vector_body,
                epilogue_edge,
                preheader_before,
                preheader_after,
                *entry_condition,
                limit_bound,
                plan,
            )?;
        } else {
            check_wasm_affine_emission(
                original,
                transformed,
                &candidate,
                affine,
                original_header,
                original_body,
                vector_header,
                vector_body,
                vector_body_edge,
                epilogue_edge,
                preheader_before,
                preheader_after,
                limit_bound,
                plan,
            )?;
        }
    }
    match plan.epilogue {
        VectorEpilogue::Scalar { start, end, .. }
            if start == candidate.induction && end == candidate.bound => {}
        _ => return compiler("vector scalar epilogue partition is false"),
    }
    validate_preheader_entry_predicate(
        &pre_state.module().profile,
        transformed,
        preheader_after,
        *entry_condition,
        limit_bound,
        &candidate,
        plan,
    )?;
    let roots = [
        plan.proofs.canonical_loop,
        plan.proofs.trip_partition,
        plan.proofs.lane_mapping,
        plan.proofs.operation_equivalence,
        plan.proofs.fallback_identity,
        plan.proofs.target_legality,
        plan.proofs.cost_and_budget,
    ];
    if roots.into_iter().collect::<BTreeSet<_>>().len() != roots.len()
        || roots.into_iter().any(|proof| {
            trial.proofs().get(proof).is_none_or(|certificate| {
                certificate.use_site.function != candidate.function
                    || certificate.generation != trial.evidence_generation()
            })
        })
    {
        return compiler("vector proof roots are missing, stale, or reused");
    }
    let before_module = pre_state
        .module()
        .functions
        .iter()
        .fold(0_u32, |total, function| {
            total.saturating_add(kir_function_units(function))
        });
    let after_module = trial
        .module()
        .functions
        .iter()
        .fold(0_u32, |total, function| {
            total.saturating_add(kir_function_units(function))
        });
    if plan.growth.original_units != kir_function_units(original)
        || plan.growth.transformed_units != kir_function_units(transformed)
        || plan.growth.module_before_units != before_module
        || plan.growth.module_after_units != after_module
    {
        return compiler("vector structural growth accounting is false");
    }
    if charge != &independently_recompute_vector_charge(plan) {
        return compiler("vector candidate budget charge is false");
    }
    let validation = validate_kir_module(trial.module());
    if !validation.errors.is_empty() {
        return Err(TransactionCheckError::compiler(format!(
            "vector trial KIR is invalid: {}",
            validation
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_persistent_modular_add(
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    candidate: &CheckedVectorSource,
    preheader: &crate::KirBlock,
    original_entry: &crate::KirEdge,
    entry: &crate::KirEdge,
    header: &crate::KirBlock,
    body: &crate::KirBlock,
    body_edge: &crate::KirEdge,
    finalizer_edge: &crate::KirEdge,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let reduction = candidate
        .reduction
        .as_ref()
        .ok_or_else(|| malformed("persistent reduction source is missing"))?;
    let source_header = source_block(original, candidate.header)
        .ok_or_else(|| malformed("persistent reduction source header is missing"))?;
    let source_body = source_block(original, candidate.body)
        .ok_or_else(|| malformed("persistent reduction source body is missing"))?;
    let seed_index = source_header
        .params
        .iter()
        .position(|param| param.value == reduction.header_value)
        .ok_or_else(|| malformed("persistent reduction seed is missing"))?;
    let body_seed_index = source_body
        .params
        .iter()
        .position(|param| param.value == reduction.body_value)
        .ok_or_else(|| malformed("persistent reduction body seed is missing"))?;
    let scalar_type = source_header.params[seed_index].type_node.clone();
    let lane = scalar_type
        .as_scalar()
        .and_then(lane_from_type)
        .filter(|lane| matches!(lane, KirLaneType::I32 | KirLaneType::U32))
        .ok_or_else(|| malformed("persistent reduction is not a modular 32-bit integer"))?;
    let vector_type = KirValueType::FixedVector { lane, lanes: 4 };
    if header.params.len() != source_header.params.len() + 1
        || body.params.len() != source_body.params.len() + 1
    {
        return Err(malformed(
            "persistent reduction parameter partition is false",
        ));
    }
    let header_acc = header
        .params
        .last()
        .ok_or_else(|| malformed("persistent vector header accumulator is missing"))?;
    let body_acc = body
        .params
        .last()
        .ok_or_else(|| malformed("persistent vector body accumulator is missing"))?;
    if header_acc.type_node != vector_type
        || body_acc.type_node != vector_type
        || body_edge.args.last() != Some(&header_acc.value)
        || body_edge.args.len() != body.params.len()
        || body_edge.args.get(body_seed_index) != Some(&header.params[seed_index].value)
    {
        return Err(malformed("persistent reduction accumulator edge is false"));
    }
    let initial = *entry
        .args
        .get(original_entry.args.len())
        .ok_or_else(|| malformed("persistent reduction initial vector is missing"))?;
    let splat = preheader
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == initial)
        })
        .ok_or_else(|| malformed("persistent reduction initialization is not in the preheader"))?;
    let KirInstructionKind::VectorSplat { scalar: zero, .. } = splat.kind else {
        return Err(malformed(
            "persistent reduction initialization is not a zero splat",
        ));
    };
    if splat.results.len() != 1 || splat.results[0].type_node != vector_type
        || !preheader.instructions.iter().any(|instruction| instruction.results.len() == 1 && instruction.results[0].value == zero && instruction.results[0].type_node == scalar_type && matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "0"))
    { return Err(malformed("persistent reduction initialization is not exact zero")); }
    let mapping = plan
        .operations
        .iter()
        .find(|mapping| mapping.scalar == reduction.instruction)
        .ok_or_else(|| malformed("persistent reduction mapping is missing"))?;
    let update = body
        .instructions
        .iter()
        .find(|instruction| instruction.id == mapping.vector)
        .ok_or_else(|| malformed("persistent reduction update is missing"))?;
    let KirInstructionKind::VectorBinary {
        op: crate::KirVectorBinaryOp::Add,
        left,
        right,
        semantics: crate::KirArithmeticSemantics::Modular,
        no_failure_proof: None,
        ..
    } = update.kind
    else {
        return Err(malformed(
            "persistent reduction update is not modular vector addition",
        ));
    };
    let scalar_update = defining_instruction_by_id(original, reduction.instruction)
        .ok_or_else(|| malformed("persistent reduction scalar update is missing"))?;
    let lane_source = match scalar_update.kind {
        KirInstructionKind::Binary { left, right, .. } if left == reduction.body_value => right,
        KirInstructionKind::Binary { left, right, .. } if right == reduction.body_value => left,
        _ => return Err(malformed("persistent reduction scalar recurrence is false")),
    };
    let source = defining_instruction(
        original,
        independent_scalar_copy_root(original, lane_source),
    )
    .ok_or_else(|| malformed("persistent reduction lane source is missing"))?;
    let source_mapping = plan
        .operations
        .iter()
        .find(|mapping| mapping.scalar == source.id)
        .map(|mapping| mapping.vector)
        .or_else(|| {
            plan.memory_groups
                .iter()
                .find(|mapping| mapping.scalar_instructions == [source.id])
                .map(|mapping| mapping.vector_instruction)
        });
    let vector_source = source_mapping
        .and_then(|id| {
            body.instructions
                .iter()
                .find(|instruction| instruction.id == id)
        })
        .and_then(|instruction| instruction.results.first())
        .map(|result| result.value);
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err(malformed("persistent reduction backedge is missing"));
    };
    if left != body_acc.value
        || Some(right) != vector_source
        || update.results.len() != 1
        || update.results[0].type_node != vector_type
        || backedge.args.last() != Some(&update.results[0].value)
        || backedge.args.len() != header.params.len()
        || backedge.args.get(seed_index) != Some(&body.params[body_seed_index].value)
        || body
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::VectorReduce { .. }))
    {
        return Err(malformed(
            "persistent reduction recurrence or scalar seed preservation is false",
        ));
    }
    let finalizer = source_block(transformed, finalizer_edge.target)
        .ok_or_else(|| malformed("persistent reduction finalizer is missing"))?;
    if finalizer.label != "loop_simd_finalize"
        || !finalizer.params.is_empty()
        || !finalizer.memory_params.is_empty()
        || !finalizer_edge.args.is_empty()
        || !finalizer_edge.memory_args.is_empty()
    {
        return Err(malformed("persistent reduction finalizer edge is false"));
    }
    let [fold, combine] = finalizer.instructions.as_slice() else {
        return Err(malformed(
            "persistent reduction finalizer is not one fold and seed combine",
        ));
    };
    if !matches!(fold.kind, KirInstructionKind::VectorReduce { op: crate::KirVectorReductionOp::ModularAdd, vector, semantics: crate::KirArithmeticSemantics::Modular, .. } if vector == header_acc.value)
        || fold.results.len() != 1
        || fold.results[0].type_node != scalar_type
        || !matches!(combine.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left, right, semantics: crate::KirArithmeticSemantics::Modular } if left == header.params[seed_index].value && right == fold.results[0].value)
        || combine.results.len() != 1
        || combine.results[0].type_node != scalar_type
        || fold.memory.is_some()
        || fold.effect.is_some()
        || combine.memory.is_some()
        || combine.effect.is_some()
    {
        return Err(malformed(
            "persistent reduction final fold or once-only seed combine is false",
        ));
    }
    let KirTerminator::Jump { edge: tail } = &finalizer.terminator else {
        return Err(malformed(
            "persistent reduction finalizer does not enter the scalar tail",
        ));
    };
    let mut expected = header.params[..source_header.params.len()]
        .iter()
        .map(|param| param.value)
        .collect::<Vec<_>>();
    expected[seed_index] = combine.results[0].value;
    if tail.target != candidate.header
        || tail.args != expected
        || tail.memory_args
            != header
                .memory_params
                .iter()
                .map(|param| param.version)
                .collect::<Vec<_>>()
    {
        return Err(malformed("persistent reduction scalar tail state is false"));
    }
    Ok(())
}

fn validate_preheader_entry_predicate(
    profile: &crate::KirTargetProfile,
    function: &crate::KirFunction,
    preheader: &crate::KirBlock,
    branch_condition: ValueId,
    entry_bound: ValueId,
    candidate: &CheckedVectorSource,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let compiler = |message: &str| Err(TransactionCheckError::compiler(message));
    if let Some(affine) = &candidate.affine {
        return check_wasm_affine_predicates(
            function,
            preheader,
            branch_condition,
            entry_bound,
            candidate,
            affine,
            plan,
        );
    }
    let expected_address_count = candidate
        .version_predicate
        .as_ref()
        .map_or(0, |predicate| predicate.conjuncts.len());
    let trip_thresholds = plan
        .predicates
        .iter()
        .filter_map(|predicate| match predicate {
            crate::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } => Some((*trip_count, *minimum)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let address_predicate_count = plan
        .predicates
        .iter()
        .filter(|predicate| matches!(predicate, crate::VectorPredicate::AddressNonOverlap { .. }))
        .count();
    if plan.predicates.len() != 1 + expected_address_count
        || trip_thresholds.as_slice() != [(candidate.bound, candidate.minimum_trip)]
        || address_predicate_count != expected_address_count
        || plan.predicates.iter().any(|predicate| {
            matches!(
                predicate,
                crate::VectorPredicate::AddressNonOverlap { bytes, .. }
                    if *bytes != candidate.bound
            )
        })
    {
        return compiler("vector plan predicates do not exactly describe threshold and aliases");
    }

    let emitted_version_predicates = preheader
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .collect::<Vec<_>>();
    let bool_type = MirType::Primitive(MirPrimitiveTypeName::Bool);
    match &candidate.version_predicate {
        None => {
            if !emitted_version_predicates.is_empty() {
                return compiler("scalar-only vector candidate emitted a runtime predicate");
            }
            let Some(condition) = preheader.instructions.iter().find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == branch_condition)
            }) else {
                return compiler("vector preheader branch condition has no local definition");
            };
            let exact_bool_result = matches!(
                condition.results.as_slice(),
                [result]
                    if result.value == branch_condition
                        && result.type_node.as_scalar() == Some(&bool_type)
            );
            let correct_threshold = matches!(
                condition.kind,
                KirInstructionKind::Compare {
                    op: crate::MirCompareOp::Ge,
                    left,
                    right,
                } if left == entry_bound
                    && value_type(function, right)
                        == Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
                    && integer_constant(function, right)
                        == Some(i128::from(candidate.minimum_trip))
            );
            if !exact_bool_result
                || condition.memory.is_some()
                || condition.effect.is_some()
                || !correct_threshold
            {
                return compiler(
                    "vector preheader branch does not use the exact scalar trip threshold",
                );
            }
        }
        Some(expected) => {
            let target_address_bits = match profile.layout() {
                crate::KirProfileLayout::Known {
                    pointer_width_bits: bits,
                    ..
                } if matches!(bits, 32 | 64) => bits as u8,
                _ => {
                    return compiler(
                        "vector runtime predicate requires a known 32- or 64-bit target layout",
                    );
                }
            };
            let [instruction] = emitted_version_predicates.as_slice() else {
                return compiler("vector preheader must emit exactly one runtime predicate");
            };
            let exact_bool_result = matches!(
                instruction.results.as_slice(),
                [result]
                    if result.value == branch_condition
                        && result.type_node.as_scalar() == Some(&bool_type)
            );
            if !exact_bool_result || instruction.memory.is_some() || instruction.effect.is_some() {
                return compiler(
                    "vector preheader branch does not use the pure runtime predicate result",
                );
            }
            let KirInstructionKind::VersionPredicate { predicate } = &instruction.kind else {
                unreachable!("filtered preheader instruction must be a version predicate");
            };
            if expected.address_bits != target_address_bits
                || predicate.address_bits != target_address_bits
            {
                return compiler("vector runtime predicate address width disagrees with target");
            }
            if predicate.conjuncts.len() != expected.conjuncts.len() + 1 {
                return compiler("vector runtime predicate conjunct count is false");
            }
            let mut threshold_count = 0_usize;
            let mut actual_intervals = Vec::new();
            for conjunct in &predicate.conjuncts {
                match conjunct {
                    crate::KirVersionPredicateConjunct::TripThreshold { value, minimum }
                        if *value == entry_bound && *minimum == candidate.minimum_trip =>
                    {
                        threshold_count = threshold_count.saturating_add(1);
                    }
                    crate::KirVersionPredicateConjunct::TripThreshold { .. } => {
                        return compiler("vector runtime trip threshold differs from its plan");
                    }
                    crate::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left,
                        left_count,
                        left_element_bytes,
                        right,
                        right_count,
                        right_element_bytes,
                    } => {
                        if *left_count != entry_bound || *right_count != entry_bound {
                            return compiler(
                                "vector runtime alias interval count differs from its trip bound",
                            );
                        }
                        actual_intervals.push(canonical_interval_key(
                            *left,
                            *left_element_bytes,
                            *right,
                            *right_element_bytes,
                        ));
                    }
                    crate::KirVersionPredicateConjunct::WasmSliceRange { .. } => {
                        return compiler(
                            "vector checker does not accept Wasm slice-range predicates yet",
                        );
                    }
                }
            }
            if threshold_count != 1 {
                return compiler("vector runtime predicate must contain one exact trip threshold");
            }
            let mut expected_intervals = expected
                .conjuncts
                .iter()
                .map(|conjunct| match conjunct {
                    CheckedVersionConjunct::AddressIntervalsDisjoint {
                        left,
                        left_element_bytes,
                        right,
                        right_element_bytes,
                    } => canonical_interval_key(
                        *left,
                        *left_element_bytes,
                        *right,
                        *right_element_bytes,
                    ),
                })
                .collect::<Vec<_>>();
            actual_intervals.sort_unstable();
            expected_intervals.sort_unstable();
            if actual_intervals != expected_intervals {
                return compiler(
                    "vector runtime alias intervals do not exactly match dependence legality",
                );
            }
        }
    }
    Ok(())
}

fn canonical_interval_key(
    left: ValueId,
    left_element_bytes: u32,
    right: ValueId,
    right_element_bytes: u32,
) -> (ValueId, u32, ValueId, u32) {
    if (left, left_element_bytes) <= (right, right_element_bytes) {
        (left, left_element_bytes, right, right_element_bytes)
    } else {
        (right, right_element_bytes, left, left_element_bytes)
    }
}

fn reconstruct_vector_source_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &VectorizationPlan,
) -> Result<CheckedVectorSource, TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
        .ok_or_else(|| malformed("vector source function is missing"))?;
    let transformed = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
        .ok_or_else(|| malformed("vector trial function is missing"))?;
    let changed = original
        .blocks
        .iter()
        .filter(|block| {
            transformed
                .blocks
                .iter()
                .find(|candidate| candidate.id == block.id)
                != Some(*block)
        })
        .collect::<Vec<_>>();
    let [preheader] = changed.as_slice() else {
        return Err(malformed(
            "vector trial must rewrite exactly one pre-existing block",
        ));
    };
    let KirTerminator::Jump { edge: entry } = &preheader.terminator else {
        return Err(malformed("vector source preheader is not a jump"));
    };
    let header = source_block(original, entry.target)
        .ok_or_else(|| malformed("vector source header is missing"))?;
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return Err(malformed("vector source header is not a branch"));
    };
    let VectorEpilogue::Scalar {
        start: induction,
        end: bound,
        ..
    } = plan.epilogue
    else {
        return Err(malformed("vector source requires a scalar epilogue"));
    };
    let induction_index = header
        .params
        .iter()
        .position(|param| param.value == induction)
        .ok_or_else(|| malformed("vector source induction is not a header parameter"))?;
    let entry_induction = entry
        .args
        .get(induction_index)
        .copied()
        .ok_or_else(|| malformed("vector source entry induction is missing"))?;
    if integer_constant(original, entry_induction) != Some(0) {
        return Err(malformed("vector source induction does not start at zero"));
    }
    let comparison = defining_instruction(original, *condition)
        .ok_or_else(|| malformed("vector source header condition is undefined"))?;
    let KirInstructionKind::Compare {
        op: crate::MirCompareOp::Lt,
        left,
        right,
    } = comparison.kind
    else {
        return Err(malformed("vector source is not a strict increasing loop"));
    };
    if left != induction || entry_value(header, entry, right) != Some(bound) {
        return Err(malformed(
            "vector source trip bound is not closed by the plan",
        ));
    }

    if !checked_loop_bound_is_invariant(original, preheader.id, right, bound) {
        return Err(malformed(
            "vector source comparison bound is not invariant across loop edges",
        ));
    }

    let (scalar_blocks, latch, diamond) =
        recognize_vector_shape(original, header.id, then_edge.target, else_edge.target)?;
    let body = source_block(original, then_edge.target)
        .ok_or_else(|| malformed("vector source body is missing"))?;
    let KirTerminator::Jump { edge: backedge } = &source_block(original, latch)
        .ok_or_else(|| malformed("vector source latch is missing"))?
        .terminator
    else {
        return Err(malformed("vector source latch is not a jump"));
    };
    if backedge.target != header.id {
        return Err(malformed(
            "vector source latch does not return to the header",
        ));
    }
    let next_induction = backedge
        .args
        .get(induction_index)
        .copied()
        .ok_or_else(|| malformed("vector source induction backedge is missing"))?;
    let induction_transfer = defining_instruction(original, next_induction)
        .ok_or_else(|| malformed("vector source induction transfer is missing"))?;
    let body_induction = body
        .params
        .get(
            then_edge
                .args
                .iter()
                .position(|value| *value == induction)
                .ok_or_else(|| malformed("vector source body induction edge is missing"))?,
        )
        .map(|param| param.value)
        .ok_or_else(|| malformed("vector source body induction parameter is missing"))?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left: step_left,
        right: step_right,
        semantics: crate::KirArithmeticSemantics::Modular,
    } = induction_transfer.kind
    else {
        return Err(malformed(
            "vector source induction is not a modular unit step",
        ));
    };
    let unit_step = (forwards_from(original, step_left, body_induction)
        && integer_constant(original, step_right) == Some(1))
        || (forwards_from(original, step_right, body_induction)
            && integer_constant(original, step_left) == Some(1));
    if !unit_step {
        return Err(malformed("vector source induction step is not one"));
    }

    let dominators = compute_kir_dominators(original);
    let mut loop_headers = original
        .blocks
        .iter()
        .flat_map(|block| {
            successor_ids(&block.terminator)
                .into_iter()
                .filter(|target| dominators.dominates(*target, block.id))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    loop_headers.sort_unstable();
    let expected_loop = loop_headers
        .iter()
        .position(|candidate| *candidate == header.id)
        .and_then(|index| u32::try_from(index).ok())
        .map(LoopId::from_index)
        .ok_or_else(|| malformed("vector source loop identity is not reproducible"))?;
    if expected_loop != plan.loop_id {
        return Err(malformed("vector source loop identity is false"));
    }

    let reduction = recognize_reduction(original, header, body, backedge, plan)?;
    if pre_state.module().profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128)
        && let Some(reduction) = reduction.as_ref()
    {
        let mut recurrent = BTreeSet::from([reduction.header_value, reduction.body_value]);
        let updated = defining_instruction_by_id(original, reduction.instruction)
            .and_then(|instruction| instruction.results.first())
            .map(|result| result.value)
            .ok_or_else(|| malformed("persistent reduction result is missing"))?;
        recurrent.insert(updated);
        recurrent.extend(
            body.params
                .iter()
                .zip(&then_edge.args)
                .filter(|(_, source)| **source == reduction.header_value)
                .map(|(param, _)| param.value),
        );
        let accumulator_index = header
            .params
            .iter()
            .position(|param| param.value == reduction.header_value)
            .ok_or_else(|| malformed("persistent reduction header parameter is missing"))?;
        for (index, value) in backedge.args.iter().enumerate() {
            let root = independent_scalar_copy_root(original, *value);
            if (index == accumulator_index && root != updated)
                || (index != accumulator_index && recurrent.contains(&root))
            {
                return Err(malformed(
                    "persistent reduction leaks an old or updated accumulator through another backedge parameter",
                ));
            }
        }
        for instruction in &body.instructions {
            let inputs = match instruction.kind {
                KirInstructionKind::Copy { value } => vec![value],
                _ => operation_inputs(instruction),
            };
            let uses = inputs
                .iter()
                .filter(|value| {
                    recurrent.contains(&independent_scalar_copy_root(original, **value))
                })
                .count();
            if (instruction.id == reduction.instruction && uses != 1)
                || (instruction.id != reduction.instruction && uses != 0)
                || matches!(instruction.kind, KirInstructionKind::Store { .. })
            {
                return Err(malformed(
                    "persistent reduction exposes a partial accumulator",
                ));
            }
        }
    }
    let affine = if is_wasm_affine_plan(plan) {
        if pre_state.module().profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
            || plan.vf != 2
            || !matches!(plan.uf, 1 | 2 | 4)
            || diamond.is_some()
            || reduction.is_some()
            || scalar_blocks.as_slice() != [body.id]
        {
            return Err(malformed(
                "WASM affine source is outside the checked f64x2 shape",
            ));
        }
        Some(reconstruct_wasm_affine_addresses(
            pre_state,
            original,
            preheader.id,
            header.id,
            body.id,
            body_induction,
            induction,
            bound,
            plan,
        )?)
    } else {
        None
    };
    let operations = independently_collect_operations(
        original,
        &scalar_blocks,
        induction_transfer.id,
        affine.as_ref(),
        reduction.as_ref(),
        diamond.as_ref(),
        plan,
    )?;
    let accesses = independently_collect_accesses(
        original,
        &scalar_blocks,
        body_induction,
        induction,
        affine.as_ref(),
        plan,
    )?;
    if pre_state.module().profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128)
        && plan.uf > 1
        && affine.as_ref().is_none_or(|affine| {
            !checked_wasm_direct_map_interleave(bound, plan, affine, &operations, &accesses)
                && !checked_wasm_matmul_interleave(CheckedMatmulInterleaveSource {
                    function: original,
                    header_id: header.id,
                    body_id: body.id,
                    induction,
                    scalar_blocks: &scalar_blocks,
                    has_diamond: diamond.is_some(),
                    has_reduction: reduction.is_some(),
                    bound,
                    plan,
                    affine,
                    operations: &operations,
                    accesses: &accesses,
                })
        })
    {
        return Err(malformed(
            "strict Wasm interleave is outside the proven direct contiguous map shape",
        ));
    }
    let required_pairs = independently_required_runtime_pairs(pre_state, original, &accesses)?;
    let planned_pairs = plan
        .predicates
        .iter()
        .filter_map(|predicate| match predicate {
            crate::VectorPredicate::AddressNonOverlap { left, right, .. } => {
                Some(ordered_region_pair(*left, *right))
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if planned_pairs != required_pairs {
        return Err(malformed(
            "vector runtime noalias predicates do not exactly close dependence legality",
        ));
    }
    let threshold_predicates = plan
        .predicates
        .iter()
        .filter_map(|predicate| match predicate {
            crate::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } => Some((*trip_count, *minimum)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if threshold_predicates.len() != 1 || threshold_predicates[0].0 != bound {
        return Err(malformed(
            "vector plan must contain exactly one threshold for the loop bound",
        ));
    }
    let minimum_trip = threshold_predicates[0].1;
    let version_predicate = if required_pairs.is_empty() {
        None
    } else {
        let address_bits = match pre_state.module().profile.layout() {
            crate::KirProfileLayout::Known {
                pointer_width_bits: bits,
                ..
            } if matches!(bits, 32 | 64) => bits as u8,
            _ => {
                return Err(malformed(
                    "vector runtime predicate requires a known 32- or 64-bit target layout",
                ));
            }
        };
        let mut conjuncts = Vec::with_capacity(required_pairs.len());
        for (left_region, right_region) in &required_pairs {
            let left = accesses
                .iter()
                .find(|access| access.region == *left_region)
                .ok_or_else(|| malformed("required left region has no scalar access"))?;
            let right = accesses
                .iter()
                .find(|access| access.region == *right_region)
                .ok_or_else(|| malformed("required right region has no scalar access"))?;
            conjuncts.push(CheckedVersionConjunct::AddressIntervalsDisjoint {
                left: left.base,
                left_element_bytes: left.element_bytes,
                right: right.base,
                right_element_bytes: right.element_bytes,
            });
        }
        Some(CheckedVersionPredicate {
            address_bits,
            conjuncts,
        })
    };
    let (predicted_cost, expected_minimum) = independently_price_vector_plan(
        pre_state,
        original,
        &scalar_blocks,
        &operations,
        &accesses,
        diamond.as_ref(),
        reduction.as_ref(),
        plan,
        version_predicate.is_some() || affine.is_some(),
        affine.as_ref(),
    )?;
    if minimum_trip != expected_minimum {
        return Err(malformed(
            "vector trip threshold is not independently optimal",
        ));
    }
    if integer_constant(original, bound).is_some_and(|trip| trip < i128::from(minimum_trip)) {
        return Err(TransactionCheckError::reject(
            "profitability-threshold-not-met",
        ));
    }
    Ok(CheckedVectorSource {
        function: original.id,
        preheader: preheader.id,
        header: header.id,
        body: body.id,
        exit: else_edge.target,
        scalar_blocks,
        diamond,
        reduction,
        induction,
        bound,
        vf: plan.vf,
        uf: plan.uf,
        minimum_trip,
        operations,
        accesses,
        version_predicate,
        predicted_cost,
        affine,
    })
}

fn recognize_vector_shape(
    function: &crate::KirFunction,
    header: BlockId,
    body: BlockId,
    exit: BlockId,
) -> Result<(Vec<BlockId>, BlockId, Option<CheckedVectorDiamond>), TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let body_block =
        source_block(function, body).ok_or_else(|| malformed("vector source body is missing"))?;
    if matches!(
        body_block.terminator,
        KirTerminator::Jump { ref edge } if edge.target == header
    ) {
        return Ok((vec![body], body, None));
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &body_block.terminator
    else {
        return Err(malformed("vector source control shape is unsupported"));
    };
    let then_block = source_block(function, then_edge.target)
        .ok_or_else(|| malformed("vector diamond then block is missing"))?;
    let else_block = source_block(function, else_edge.target)
        .ok_or_else(|| malformed("vector diamond else block is missing"))?;
    let (KirTerminator::Jump { edge: then_merge }, KirTerminator::Jump { edge: else_merge }) =
        (&then_block.terminator, &else_block.terminator)
    else {
        return Err(malformed("vector diamond arms do not reconverge"));
    };
    if then_merge.target != else_merge.target {
        return Err(malformed("vector diamond arms have different merge blocks"));
    }
    let merge = source_block(function, then_merge.target)
        .ok_or_else(|| malformed("vector diamond merge block is missing"))?;
    if !matches!(merge.terminator, KirTerminator::Jump { ref edge } if edge.target == header)
        || then_merge.args.len() != merge.params.len()
        || else_merge.args.len() != merge.params.len()
        || exit == header
    {
        return Err(malformed("vector diamond does not form a closed loop body"));
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
            incoming_source(then_block, then_edge, **then_value)
                != incoming_source(else_block, else_edge, **else_value)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let [selected_param_index] = varying.as_slice() else {
        return Err(malformed("vector diamond must select exactly one value"));
    };
    let condition_instruction = defining_instruction(function, *condition)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Compare { .. }))
        .map(|instruction| instruction.id)
        .ok_or_else(|| malformed("vector diamond condition is not a scalar compare"))?;
    Ok((
        vec![body, then_block.id, else_block.id, merge.id],
        merge.id,
        Some(CheckedVectorDiamond {
            then_block: then_block.id,
            else_block: else_block.id,
            merge_block: merge.id,
            condition_instruction,
            selected_param_index: *selected_param_index,
        }),
    ))
}

fn recognize_reduction(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    body: &crate::KirBlock,
    backedge: &crate::KirEdge,
    plan: &VectorizationPlan,
) -> Result<Option<CheckedVectorReduction>, TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let reductions = plan
        .operations
        .iter()
        .filter(|mapping| {
            mapping.unroll_index == 0
                && matches!(
                    mapping.operation,
                    KirProfileOperation::ReduceAdd | KirProfileOperation::ReduceMultiply
                )
        })
        .collect::<Vec<_>>();
    if reductions.is_empty() {
        return Ok(None);
    }
    let [mapping] = reductions.as_slice() else {
        return Err(malformed("vector source has more than one reduction"));
    };
    let instruction = defining_instruction_by_id(function, mapping.scalar)
        .ok_or_else(|| malformed("vector reduction source is missing"))?;
    let KirInstructionKind::Binary {
        op,
        left,
        right,
        semantics: crate::KirArithmeticSemantics::Modular,
    } = instruction.kind
    else {
        return Err(malformed(
            "vector reduction source is not modular arithmetic",
        ));
    };
    let expected_operation = match op {
        MirBinaryOp::Add => KirProfileOperation::ReduceAdd,
        MirBinaryOp::Mul => KirProfileOperation::ReduceMultiply,
        _ => {
            return Err(malformed(
                "vector reduction source operation is unsupported",
            ));
        }
    };
    if mapping.operation != expected_operation {
        return Err(malformed("vector reduction operation record is false"));
    }
    let mut recurrence = None;
    for (index, param) in body.params.iter().enumerate() {
        if (forwards_from(function, left, param.value)
            || forwards_from(function, right, param.value))
            && instruction.results.first().is_some_and(|result| {
                backedge
                    .args
                    .get(index)
                    .is_some_and(|value| forwards_from(function, *value, result.value))
            })
            && recurrence.replace(index).is_some()
        {
            return Err(malformed("vector reduction recurrence is ambiguous"));
        }
    }
    let index = recurrence.ok_or_else(|| malformed("vector reduction recurrence is missing"))?;
    let header_value = header
        .params
        .get(index)
        .map(|param| param.value)
        .ok_or_else(|| malformed("vector reduction header value is missing"))?;
    Ok(Some(CheckedVectorReduction {
        header_value,
        body_value: body.params[index].value,
        instruction: instruction.id,
        binary_op: op,
    }))
}

fn independently_collect_operations(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    induction_transfer: InstructionId,
    affine: Option<&CheckedWasmAffine>,
    reduction: Option<&CheckedVectorReduction>,
    diamond: Option<&CheckedVectorDiamond>,
    plan: &VectorizationPlan,
) -> Result<Vec<CheckedVectorOperation>, TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let memory = scalar_blocks
        .iter()
        .filter_map(|id| source_block(function, *id))
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Load { .. } | KirInstructionKind::Store { .. }
            )
        })
        .map(|instruction| instruction.id)
        .collect::<BTreeSet<_>>();
    let mut expected = Vec::new();
    for instruction in scalar_blocks
        .iter()
        .filter_map(|id| source_block(function, *id))
        .flat_map(|block| &block.instructions)
    {
        if instruction.id == induction_transfer
            || memory.contains(&instruction.id)
            || affine.is_some_and(|affine| affine.setup.contains(&instruction.id))
        {
            continue;
        }
        if reduction.is_some_and(|item| item.instruction == instruction.id) {
            let reduction = reduction.expect("matched reduction source");
            let lane_type = instruction
                .results
                .first()
                .and_then(|result| result.type_node.as_scalar())
                .and_then(lane_from_type)
                .ok_or_else(|| malformed("vector reduction lane is unsupported"))?;
            expected.push(CheckedVectorOperation {
                scalar: instruction.id,
                operation: if reduction.binary_op == MirBinaryOp::Add {
                    KirProfileOperation::ReduceAdd
                } else {
                    KirProfileOperation::ReduceMultiply
                },
                lane_type,
                semantics: KirCostSemantics::Modular,
            });
            continue;
        }
        if matches!(
            instruction.kind,
            KirInstructionKind::ConstInt { .. }
                | KirInstructionKind::ConstFloat { .. }
                | KirInstructionKind::Copy { .. }
        ) {
            continue;
        }
        expected.push(
            checked_scalar_operation(function, instruction).ok_or_else(|| {
                malformed("vector source contains an unsupported scalar operation")
            })?,
        );
    }
    if let Some(diamond) = diamond {
        let selected_lane = source_block(function, diamond.merge_block)
            .and_then(|block| block.params.get(diamond.selected_param_index))
            .and_then(|param| param.type_node.as_scalar())
            .and_then(lane_from_type)
            .ok_or_else(|| malformed("vector diamond selected lane is unsupported"))?;
        expected.push(CheckedVectorOperation {
            scalar: diamond.condition_instruction,
            operation: KirProfileOperation::Select,
            lane_type: selected_lane,
            semantics: KirCostSemantics::NotApplicable,
        });
    }
    expected.sort_by_key(|operation| (operation.scalar, operation.operation));
    let planned_base = plan
        .operations
        .iter()
        .filter(|mapping| mapping.unroll_index == 0)
        .collect::<Vec<_>>();
    if planned_base.len() != expected.len()
        || expected
            .iter()
            .zip(&planned_base)
            .any(|(expected, mapping)| {
                expected.scalar != mapping.scalar
                    || expected.operation != mapping.operation
                    || expected.lane_type != mapping.lane_type
                    || expected.semantics != mapping.semantics
            })
        || expected.iter().any(|expected| {
            (0..plan.uf).any(|unroll_index| {
                plan.operations
                    .iter()
                    .filter(|mapping| {
                        mapping.scalar == expected.scalar
                            && mapping.operation == expected.operation
                            && mapping.unroll_index == unroll_index
                            && mapping.lane_type == expected.lane_type
                            && mapping.semantics == expected.semantics
                    })
                    .count()
                    != 1
            })
        })
    {
        return Err(malformed(
            "vector operation record is not a complete independent source mapping",
        ));
    }
    Ok(expected)
}

fn independently_collect_accesses(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    body_induction: ValueId,
    header_induction: ValueId,
    affine: Option<&CheckedWasmAffine>,
    plan: &VectorizationPlan,
) -> Result<Vec<CheckedVectorAccess>, TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let mut accesses = Vec::new();
    for instruction in scalar_blocks
        .iter()
        .filter_map(|id| source_block(function, *id))
        .flat_map(|block| &block.instructions)
    {
        let (kind, place) = match &instruction.kind {
            KirInstructionKind::Load { place } => (CheckedMemoryAccessKind::Read, place.as_ref()),
            KirInstructionKind::Store { place, .. } => {
                (CheckedMemoryAccessKind::Write, place.as_ref())
            }
            _ => continue,
        };
        let crate::KirPlace::SliceIndex {
            slice,
            index,
            region,
            ..
        } = place
        else {
            return Err(malformed("vector memory source is not a slice index"));
        };
        if affine.is_none()
            && !forwards_from(function, *index, body_induction)
            && !forwards_from(function, *index, header_induction)
        {
            return Err(malformed(
                "vector memory source is not exact unit-stride induction",
            ));
        }
        if instruction.memory.is_none() {
            return Err(malformed("vector memory source lacks Memory SSA evidence"));
        }
        accesses.push(CheckedVectorAccess {
            instruction: instruction.id,
            kind,
            region: *region,
            base: invariant_root_value(function, *slice).unwrap_or(*slice),
            element_bytes: memory_lane_and_bytes(instruction)
                .map(|(_, bytes)| bytes)
                .ok_or_else(|| malformed("vector memory element width is unsupported"))?,
        });
    }
    accesses.sort_by_key(|access| access.instruction);
    if affine.is_some() {
        return Ok(accesses);
    }
    let base_groups = plan
        .memory_groups
        .iter()
        .filter(|group| group.unroll_index == 0)
        .collect::<Vec<_>>();
    if accesses.len() != base_groups.len()
        || accesses.iter().zip(&base_groups).any(|(access, group)| {
            group.scalar_instructions.as_slice() != [access.instruction]
                || group.region != access.region
                || group.access
                    != if access.kind == CheckedMemoryAccessKind::Read {
                        VectorMemoryAccessKind::Read
                    } else {
                        VectorMemoryAccessKind::Write
                    }
        })
    {
        return Err(malformed(
            "vector memory record does not cover the exact scalar footprint",
        ));
    }
    Ok(accesses)
}

fn independently_required_runtime_pairs(
    pre_state: &KirVerifiedProgramState,
    function: &crate::KirFunction,
    accesses: &[CheckedVectorAccess],
) -> Result<BTreeSet<(MemoryRegionId, MemoryRegionId)>, TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let mut required = BTreeSet::new();
    for (index, left) in accesses.iter().enumerate() {
        for right in accesses.iter().skip(index + 1) {
            if left.kind == CheckedMemoryAccessKind::Read
                && right.kind == CheckedMemoryAccessKind::Read
            {
                continue;
            }
            if left.region == right.region {
                if left.base != right.base {
                    return Err(malformed(
                        "same-region vector accesses have different invariant roots",
                    ));
                }
                continue;
            }
            if !has_noalias_fact(pre_state, function.id, left.base, right.base) {
                required.insert(ordered_region_pair(left.region, right.region));
            }
        }
    }
    Ok(required)
}

#[allow(clippy::too_many_arguments)]
fn independently_price_vector_plan(
    pre_state: &KirVerifiedProgramState,
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    operations: &[CheckedVectorOperation],
    accesses: &[CheckedVectorAccess],
    diamond: Option<&CheckedVectorDiamond>,
    reduction: Option<&CheckedVectorReduction>,
    plan: &VectorizationPlan,
    has_runtime_predicate: bool,
    affine: Option<&CheckedWasmAffine>,
) -> Result<(KirCostEstimate, u32), TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let profile = &pre_state.module().profile;
    let lanes = u8::try_from(plan.vf)
        .map_err(|_| malformed("vector VF is not representable in the target profile"))?;
    // The checked nested-matmul schema permits its invariant A scalar to be
    // loaded and splatted once for the entire UF bundle. Other affine
    // broadcasts retain one group per vector chunk.
    let shared_matmul_broadcast = plan.uf > 1
        && plan.broadcast_groups.len() == 1
        && plan.broadcast_groups[0].unroll_index == 0;
    let mut scalar_iteration = 0_u32;
    let mut vector_chunk = 0_u32;
    let mut reduction_setup_cost = 0_u32;
    for operation in operations {
        let scalar_operation = match operation.operation {
            KirProfileOperation::ReduceAdd => KirProfileOperation::Add,
            KirProfileOperation::ReduceMultiply => KirProfileOperation::Multiply,
            operation => operation,
        };
        scalar_iteration = scalar_iteration.saturating_add(independent_profile_cost(
            profile,
            KirCostKey {
                operation: scalar_operation,
                lane: operation.lane_type,
                lanes: 1,
                semantics: operation.semantics,
                alignment: KirAlignmentClass::NotApplicable,
            },
            false,
        )?);
    }
    for mapping in &plan.operations {
        let persistent_add = profile.wasm_features() == Some(crate::KirWasmFeatures::Simd128)
            && plan.vf == 4
            && plan.uf == 1
            && mapping.operation == KirProfileOperation::ReduceAdd;
        if persistent_add {
            reduction_setup_cost = reduction_setup_cost
                .saturating_add(independent_profile_cost(
                    profile,
                    KirCostKey {
                        operation: KirProfileOperation::ReduceAdd,
                        lane: mapping.lane_type,
                        lanes,
                        semantics: mapping.semantics,
                        alignment: mapping.alignment,
                    },
                    false,
                )?)
                .saturating_add(independent_profile_cost(
                    profile,
                    KirCostKey {
                        operation: KirProfileOperation::Splat,
                        lane: mapping.lane_type,
                        lanes,
                        semantics: KirCostSemantics::NotApplicable,
                        alignment: KirAlignmentClass::NotApplicable,
                    },
                    false,
                )?)
                .saturating_add(independent_profile_cost(
                    profile,
                    KirCostKey {
                        operation: KirProfileOperation::Branch,
                        lane: KirLaneType::U32,
                        lanes: 1,
                        semantics: KirCostSemantics::NotApplicable,
                        alignment: KirAlignmentClass::NotApplicable,
                    },
                    true,
                )?);
        }
        vector_chunk = vector_chunk.saturating_add(independent_profile_cost(
            profile,
            KirCostKey {
                operation: if persistent_add {
                    KirProfileOperation::Add
                } else {
                    mapping.operation
                },
                lane: mapping.lane_type,
                lanes,
                semantics: mapping.semantics,
                alignment: mapping.alignment,
            },
            false,
        )?);
    }
    for access in accesses {
        let instruction = defining_instruction_by_id(function, access.instruction)
            .ok_or_else(|| malformed("vector memory source disappeared during pricing"))?;
        let (lane, bytes) = memory_lane_and_bytes(instruction)
            .ok_or_else(|| malformed("vector memory lane is unavailable during pricing"))?;
        let operation = if access.kind == CheckedMemoryAccessKind::Read {
            KirProfileOperation::Load
        } else {
            KirProfileOperation::Store
        };
        let alignment = KirAlignmentClass::Bytes(
            u16::try_from(bytes)
                .map_err(|_| malformed("vector memory alignment is not representable"))?,
        );
        scalar_iteration = scalar_iteration.saturating_add(independent_profile_cost(
            profile,
            KirCostKey {
                operation,
                lane,
                lanes: 1,
                semantics: KirCostSemantics::NotApplicable,
                alignment,
            },
            false,
        )?);
        let broadcast = affine.is_some_and(|affine| {
            matches!(
                affine.addresses.get(&access.instruction),
                Some(CheckedWasmAddress::Broadcast(_))
            )
        });
        if broadcast {
            vector_chunk = vector_chunk.saturating_add(independent_profile_cost(
                profile,
                KirCostKey {
                    operation: KirProfileOperation::Splat,
                    lane,
                    lanes,
                    semantics: KirCostSemantics::NotApplicable,
                    alignment: KirAlignmentClass::NotApplicable,
                },
                false,
            )?);
        }
        let vector_load_repetitions = if broadcast && shared_matmul_broadcast {
            1
        } else {
            u32::from(plan.uf)
        };
        for _ in 0..vector_load_repetitions {
            vector_chunk = vector_chunk.saturating_add(independent_profile_cost(
                profile,
                KirCostKey {
                    operation,
                    lane,
                    lanes: if broadcast { 1 } else { lanes },
                    semantics: KirCostSemantics::NotApplicable,
                    alignment,
                },
                false,
            )?);
        }
    }

    if let Some(affine) = affine {
        let setup_cost = independent_profile_cost(
            profile,
            KirCostKey {
                operation: KirProfileOperation::Add,
                lane: KirLaneType::U32,
                lanes: 1,
                semantics: KirCostSemantics::Modular,
                alignment: KirAlignmentClass::NotApplicable,
            },
            false,
        )?
        .saturating_mul(u32::try_from(affine.setup.len()).unwrap_or(u32::MAX));
        scalar_iteration = scalar_iteration.saturating_add(setup_cost);
        vector_chunk = vector_chunk.saturating_add(setup_cost.saturating_mul(u32::from(plan.uf)));
    }
    let mut vectorized_values = accesses
        .iter()
        .filter(|access| access.kind == CheckedMemoryAccessKind::Read)
        .filter_map(|access| {
            defining_instruction_by_id(function, access.instruction)
                .and_then(|instruction| instruction.results.first())
                .map(|result| result.value)
        })
        .collect::<BTreeSet<_>>();
    for mapping in &plan.operations {
        if matches!(
            mapping.operation,
            KirProfileOperation::Select
                | KirProfileOperation::ReduceAdd
                | KirProfileOperation::ReduceMultiply
        ) {
            continue;
        }
        if let Some(instruction) = defining_instruction_by_id(function, mapping.scalar) {
            vectorized_values.extend(instruction.results.iter().map(|result| result.value));
        }
    }
    if let Some(diamond) = diamond
        && let Some(value) = source_block(function, diamond.merge_block)
            .and_then(|block| block.params.get(diamond.selected_param_index))
            .map(|param| param.value)
    {
        vectorized_values.insert(value);
    }

    let mut splat_inputs = BTreeSet::new();
    for mapping in &plan.operations {
        if mapping.operation == KirProfileOperation::Select {
            continue;
        }
        let Some(instruction) = defining_instruction_by_id(function, mapping.scalar) else {
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
            operation_inputs(instruction)
        };
        for operand in operands {
            let value = independent_scalar_copy_root(function, operand);
            if !vectorized_values.contains(&value) {
                splat_inputs.insert((value, mapping.lane_type));
            }
        }
    }
    if let Some(diamond) = diamond
        && let Some(lane) = source_block(function, diamond.merge_block)
            .and_then(|block| block.params.get(diamond.selected_param_index))
            .and_then(|param| param.type_node.as_scalar())
            .and_then(lane_from_type)
    {
        for operand in independent_diamond_select_values(function, scalar_blocks, diamond) {
            let root = independent_diamond_arm_root(function, scalar_blocks, diamond, operand);
            if !vectorized_values.contains(&root) {
                splat_inputs.insert((root, lane));
            }
        }
    }
    for (value, lane) in splat_inputs {
        let repetitions = if independent_loop_local_constant(function, scalar_blocks, value) {
            u32::from(plan.uf)
        } else {
            1
        };
        let cost = independent_profile_cost(
            profile,
            KirCostKey {
                operation: KirProfileOperation::Splat,
                lane,
                lanes,
                semantics: KirCostSemantics::NotApplicable,
                alignment: KirAlignmentClass::NotApplicable,
            },
            false,
        )?;
        vector_chunk = vector_chunk.saturating_add(cost.saturating_mul(repetitions));
    }
    let scalar_control = independent_profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Add,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::Modular,
            alignment: KirAlignmentClass::NotApplicable,
        },
        false,
    )?
    .saturating_add(independent_profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Compare,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
        true,
    )?)
    .saturating_add(independent_profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Branch,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
        true,
    )?);
    scalar_iteration = scalar_iteration.saturating_add(scalar_control);
    vector_chunk = vector_chunk.saturating_add(scalar_control);

    let predicate_base = independent_profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Compare,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
        true,
    )?
    .saturating_add(independent_profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Branch,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
        true,
    )?);
    let runtime_predicates = u32::try_from(
        plan.predicates
            .iter()
            .filter(|predicate| {
                matches!(
                    predicate,
                    crate::VectorPredicate::AddressNonOverlap { .. }
                        | crate::VectorPredicate::WasmSliceRange { .. }
                )
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    let predicate_cost = (if has_runtime_predicate {
        predicate_base.saturating_add(
            independent_profile_cost(
                profile,
                KirCostKey {
                    operation: KirProfileOperation::RuntimePredicate,
                    lane: KirLaneType::U32,
                    lanes,
                    semantics: KirCostSemantics::NotApplicable,
                    alignment: KirAlignmentClass::NotApplicable,
                },
                false,
            )?
            .saturating_mul(runtime_predicates),
        )
    } else {
        predicate_base
    })
    .saturating_add(reduction_setup_cost)
    .saturating_add(
        if affine.is_some_and(|a| {
            a.ranges
                .iter()
                .any(|r| matches!(r.count, crate::WasmRangeCount::ScaledInvariant { .. }))
        }) {
            independent_profile_cost(
                profile,
                KirCostKey {
                    operation: KirProfileOperation::Multiply,
                    lane: KirLaneType::U32,
                    lanes: 1,
                    semantics: KirCostSemantics::Modular,
                    alignment: KirAlignmentClass::NotApplicable,
                },
                false,
            )?
        } else {
            0
        },
    );
    let epilogue = independent_profile_cost(
        profile,
        KirCostKey {
            operation: KirProfileOperation::Branch,
            lane: KirLaneType::U32,
            lanes: 1,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        },
        true,
    )?;
    let chunk_width = u32::from(plan.vf).saturating_mul(u32::from(plan.uf));
    let scalar_chunk = scalar_iteration.saturating_mul(chunk_width);
    if u64::from(vector_chunk).saturating_mul(100) >= u64::from(scalar_chunk).saturating_mul(80) {
        return Err(TransactionCheckError::reject(
            "profitability-threshold-not-met",
        ));
    }
    let minimum_groups = match profile.target_identity() {
        KirTargetIdentity::Native { triple } if triple.starts_with("x86_64-") => {
            4_u32.div_ceil(u32::from(plan.uf))
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
        .ok_or_else(|| TransactionCheckError::reject("profitability-threshold-not-met"))?;
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

fn independent_profile_cost(
    profile: &crate::KirTargetProfile,
    key: KirCostKey,
    control: bool,
) -> Result<u32, TransactionCheckError> {
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Ok(cost.cost)
        }
        Some(KirOperationAvailability::Unavailable)
            if control && key.operation == KirProfileOperation::Branch =>
        {
            Ok(1)
        }
        _ => Err(TransactionCheckError::reject(
            "target-operation-unavailable",
        )),
    }
}

fn independently_recompute_vector_charge(plan: &VectorizationPlan) -> CandidateBudgetCharge {
    let lanes = plan.operations.iter().fold(0_u32, |total, operation| {
        total.saturating_add(u32::try_from(operation.lanes.len()).unwrap_or(u32::MAX))
    });
    let broadcasts = u32::try_from(plan.broadcast_groups.len()).unwrap_or(u32::MAX);
    let memory = plan.memory_groups.iter().fold(broadcasts, |total, group| {
        total.saturating_add(u32::try_from(group.scalar_instructions.len()).unwrap_or(u32::MAX))
    });
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
            .saturating_add(2)
            .saturating_add(broadcasts.saturating_mul(2)),
        16_u32
            .saturating_add(operations.saturating_mul(6))
            .saturating_add(lanes.saturating_mul(2))
            .saturating_add(groups.saturating_mul(6))
            .saturating_add(memory.saturating_mul(2))
            .saturating_add(predicates.saturating_mul(4))
            .saturating_add(7)
            .saturating_add(3)
            .saturating_add(broadcasts.saturating_mul(3)),
    )
}

fn checked_scalar_operation(
    function: &crate::KirFunction,
    instruction: &KirInstruction,
) -> Option<CheckedVectorOperation> {
    if let KirInstructionKind::Compare { left, right, .. } = instruction.kind {
        let lane = value_type(function, left).and_then(lane_from_type)?;
        if value_type(function, right).and_then(lane_from_type)? != lane {
            return None;
        }
        return Some(CheckedVectorOperation {
            scalar: instruction.id,
            operation: KirProfileOperation::Compare,
            lane_type: lane,
            semantics: KirCostSemantics::NotApplicable,
        });
    }
    let result_lane = instruction
        .results
        .first()
        .and_then(|result| result.type_node.as_scalar())
        .and_then(lane_from_type)?;
    let (operation, lane_type, semantics) = match instruction.kind {
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => KirProfileOperation::Add,
                MirBinaryOp::Sub => KirProfileOperation::Subtract,
                MirBinaryOp::Mul => KirProfileOperation::Multiply,
                MirBinaryOp::Div if semantics == crate::KirArithmeticSemantics::StrictFloat => {
                    KirProfileOperation::Divide
                }
                MirBinaryOp::Div | MirBinaryOp::Mod => return None,
            },
            result_lane,
            match semantics {
                crate::KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                crate::KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
                crate::KirArithmeticSemantics::Checked => return None,
            },
        ),
        KirInstructionKind::Unary {
            op: crate::MirUnaryOp::Neg,
            semantics,
            ..
        } => (
            KirProfileOperation::Negate,
            result_lane,
            match semantics {
                crate::KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                crate::KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
                crate::KirArithmeticSemantics::Checked => return None,
            },
        ),
        KirInstructionKind::Cast { value, .. } => (
            KirProfileOperation::Cast,
            value_type(function, value).and_then(lane_from_type)?,
            KirCostSemantics::NotApplicable,
        ),
        _ => return None,
    };
    Some(CheckedVectorOperation {
        scalar: instruction.id,
        operation,
        lane_type,
        semantics,
    })
}

fn memory_lane_and_bytes(instruction: &KirInstruction) -> Option<(KirLaneType, u32)> {
    let place = match &instruction.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
            place.as_ref()
        }
        _ => return None,
    };
    let type_node = match place {
        crate::KirPlace::SliceIndex { type_node, .. }
        | crate::KirPlace::Index { type_node, .. }
        | crate::KirPlace::Value { type_node, .. }
        | crate::KirPlace::Deref { type_node, .. } => type_node,
        crate::KirPlace::Field { .. } => return None,
    };
    let lane = lane_from_type(type_node)?;
    let bytes = match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => 4,
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => 8,
        _ => return None,
    };
    Some((lane, bytes))
}

fn lane_from_type(type_node: &MirType) -> Option<KirLaneType> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(KirLaneType::I64),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some(KirLaneType::U64),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn value_type(function: &crate::KirFunction, value: ValueId) -> Option<&MirType> {
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

fn operation_inputs(instruction: &KirInstruction) -> Vec<ValueId> {
    match instruction.kind {
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => vec![left, right],
        KirInstructionKind::Unary { operand, .. } => vec![operand],
        KirInstructionKind::Cast { value, .. } => vec![value],
        _ => Vec::new(),
    }
}

fn independent_scalar_copy_root(function: &crate::KirFunction, value: ValueId) -> ValueId {
    let mut value = value;
    let mut visited = BTreeSet::new();
    while visited.insert(value) {
        let source =
            defining_instruction(function, value).and_then(|instruction| match instruction.kind {
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

fn independent_diamond_select_values(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    diamond: &CheckedVectorDiamond,
) -> Vec<ValueId> {
    let Some(merge) = source_block(function, diamond.merge_block) else {
        return Vec::new();
    };
    let Some(body) = scalar_blocks
        .first()
        .and_then(|id| source_block(function, *id))
    else {
        return Vec::new();
    };
    let KirTerminator::Branch { .. } = &body.terminator else {
        return Vec::new();
    };
    let Some(then_block) = source_block(function, diamond.then_block) else {
        return Vec::new();
    };
    let Some(else_block) = source_block(function, diamond.else_block) else {
        return Vec::new();
    };
    let KirTerminator::Jump { edge: then_merge } = &then_block.terminator else {
        return Vec::new();
    };
    let KirTerminator::Jump { edge: else_merge } = &else_block.terminator else {
        return Vec::new();
    };
    if merge.params.get(diamond.selected_param_index).is_none() {
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

fn independent_diamond_arm_root(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    diamond: &CheckedVectorDiamond,
    value: ValueId,
) -> ValueId {
    let mut value = value;
    let mut visited = BTreeSet::new();
    while visited.insert(value) {
        if let Some(source) =
            defining_instruction(function, value).and_then(|instruction| match instruction.kind {
                KirInstructionKind::Copy { value } => Some(value),
                _ => None,
            })
        {
            value = source;
            continue;
        }
        let Some((arm, index)) = [diamond.then_block, diamond.else_block]
            .into_iter()
            .find_map(|arm_id| {
                source_block(function, arm_id)
                    .and_then(|block| block.params.iter().position(|param| param.value == value))
                    .map(|index| (arm_id, index))
            })
        else {
            break;
        };
        let Some(body) = scalar_blocks
            .first()
            .and_then(|id| source_block(function, *id))
        else {
            break;
        };
        let KirTerminator::Branch {
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

fn independent_diamond_scalar_source(
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    original_header: &crate::KirBlock,
    original_body: &crate::KirBlock,
    vector_header: &crate::KirBlock,
    vector_body: &crate::KirBlock,
    source: ValueId,
) -> Option<ValueId> {
    let matching_parameter = |source_params: &[crate::KirBlockParam],
                              target_params: &[crate::KirBlockParam]| {
        source_params
            .iter()
            .position(|param| param.value == source)
            .and_then(|index| {
                let source = source_params.get(index)?;
                let target = target_params.get(index)?;
                (source.type_node == target.type_node).then_some(target.value)
            })
    };
    if let Some(value) = matching_parameter(&original_body.params, &vector_body.params) {
        return Some(value);
    }
    if let Some(value) = matching_parameter(&original_header.params, &vector_header.params) {
        return Some(value);
    }
    if let Some(value) = original
        .params
        .iter()
        .position(|param| param.value == source)
        .and_then(|index| {
            let source = original.params.get(index)?;
            let target = transformed.params.get(index)?;
            (source.type_node == target.type_node).then_some(target.value)
        })
    {
        return Some(value);
    }

    let source_instruction = defining_instruction(original, source)?;
    let source_result = source_instruction
        .results
        .iter()
        .find(|result| result.value == source)?;
    vector_body.instructions.iter().find_map(|instruction| {
        let same_constant = match (&source_instruction.kind, &instruction.kind) {
            (
                KirInstructionKind::ConstInt { value: source },
                KirInstructionKind::ConstInt { value: target },
            )
            | (
                KirInstructionKind::ConstFloat { value: source },
                KirInstructionKind::ConstFloat { value: target },
            ) => source == target,
            _ => false,
        };
        if !same_constant {
            return None;
        }
        instruction
            .results
            .iter()
            .find(|result| result.type_node == source_result.type_node)
            .map(|result| result.value)
    })
}

fn independent_loop_local_constant(
    function: &crate::KirFunction,
    scalar_blocks: &[BlockId],
    value: ValueId,
) -> bool {
    function.blocks.iter().any(|block| {
        scalar_blocks.contains(&block.id)
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

fn source_block(function: &crate::KirFunction, id: BlockId) -> Option<&crate::KirBlock> {
    function.blocks.iter().find(|block| block.id == id)
}

fn defining_instruction(function: &crate::KirFunction, value: ValueId) -> Option<&KirInstruction> {
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
}

fn defining_instruction_by_id(
    function: &crate::KirFunction,
    id: InstructionId,
) -> Option<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
}

// A header parameter is not invariant merely because its first incoming value
// matches the plan. Every phi/copy path must retain that entry value. Stop at
// the preheader SSA value so a surrounding loop may legitimately change it
// between distinct invocations of this inner loop.
pub(crate) fn checked_loop_bound_is_invariant(
    function: &crate::KirFunction,
    preheader: BlockId,
    comparison_bound: ValueId,
    entry_bound: ValueId,
) -> bool {
    let dominators = compute_kir_dominators(function);
    let available_at_entry = function
        .params
        .iter()
        .any(|param| param.value == entry_bound)
        || function.blocks.iter().any(|block| {
            dominators.dominates(block.id, preheader)
                && (block.params.iter().any(|param| param.value == entry_bound)
                    || block.instructions.iter().any(|instruction| {
                        instruction
                            .results
                            .iter()
                            .any(|result| result.value == entry_bound)
                    }))
        });
    if available_at_entry {
        return forwards_from(function, comparison_bound, entry_bound);
    }
    if comparison_bound != entry_bound {
        return false;
    }
    let Some(instruction) = defining_instruction(function, comparison_bound) else {
        return false;
    };
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || !matches!(instruction.results.as_slice(), [result]
            if result.value == comparison_bound
                && result.type_node.as_scalar() == Some(&MirType::Primitive(MirPrimitiveTypeName::U32)))
    {
        return false;
    }
    match instruction.kind {
        KirInstructionKind::ConstInt { .. } => true,
        KirInstructionKind::SliceLen { slice } => {
            stable_invariant_descriptor_root(function, slice).is_some()
        }
        _ => false,
    }
}

fn entry_bound_matches(
    original: &crate::KirFunction,
    header: &crate::KirBlock,
    original_preheader: &crate::KirBlock,
    transformed_preheader: &crate::KirBlock,
    entry: &crate::KirEdge,
    source_bound: ValueId,
    trial_bound: ValueId,
) -> bool {
    if let Some(index) = header
        .params
        .iter()
        .position(|param| param.value == source_bound)
    {
        return entry.args.get(index) == Some(&trial_bound);
    }
    if original
        .params
        .iter()
        .any(|param| param.value == source_bound)
        || original_preheader
            .params
            .iter()
            .any(|param| param.value == source_bound)
        || original_preheader.instructions.iter().any(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == source_bound)
        })
    {
        return source_bound == trial_bound;
    }
    let Some(source) = defining_instruction(original, source_bound) else {
        return false;
    };
    if let KirInstructionKind::SliceLen {
        slice: source_slice,
    } = &source.kind
    {
        if source.results.len() != 1 || source.memory.is_some() || source.effect.is_some() {
            return false;
        }
        let Some(source_root) = stable_invariant_descriptor_root(original, *source_slice) else {
            return false;
        };
        return transformed_preheader
            .instructions
            .iter()
            .any(|instruction| {
                instruction.results.iter().any(|result| {
                    result.value == trial_bound && result.type_node == source.results[0].type_node
                }) && instruction.memory.is_none()
                    && instruction.effect.is_none()
                    && match instruction.kind {
                        KirInstructionKind::SliceLen { slice } => {
                            slice == source_root
                                && original
                                    .params
                                    .iter()
                                    .any(|parameter| parameter.value == slice)
                        }
                        _ => false,
                    }
            });
    }
    let KirInstructionKind::ConstInt {
        value: source_value,
    } = &source.kind
    else {
        return false;
    };
    transformed_preheader
        .instructions
        .iter()
        .any(|instruction| {
            instruction.results.iter().any(|result| {
                result.value == trial_bound && result.type_node == source.results[0].type_node
            }) && matches!(
                &instruction.kind,
                KirInstructionKind::ConstInt { value } if value == source_value
            )
        })
}

pub(crate) fn stable_invariant_descriptor_root(
    function: &crate::KirFunction,
    value: ValueId,
) -> Option<ValueId> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut roots = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if function.params.iter().any(|param| param.value == value) {
            roots.insert(value);
            continue;
        }
        if !visited.insert(value) {
            continue;
        }
        if let Some((block_id, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            let mut incoming_count = 0_usize;
            for predecessor in &function.blocks {
                let edges = match &predecessor.terminator {
                    KirTerminator::Jump { edge } => vec![edge],
                    KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => vec![then_edge, else_edge],
                    KirTerminator::Return { .. } => Vec::new(),
                };
                for edge in edges.into_iter().filter(|edge| edge.target == block_id) {
                    incoming_count = incoming_count.saturating_add(1);
                    pending.push(*edge.args.get(index)?);
                }
            }
            if incoming_count == 0 {
                return None;
            }
        } else if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = defining_instruction(function, value)
        {
            pending.push(*value);
        } else {
            return None;
        }
    }
    (roots.len() == 1).then(|| *roots.first().expect("one descriptor root"))
}

fn integer_constant(function: &crate::KirFunction, value: ValueId) -> Option<i128> {
    let KirInstructionKind::ConstInt { value } = &defining_instruction(function, value)?.kind
    else {
        return None;
    };
    value.parse().ok()
}

fn entry_value(
    header: &crate::KirBlock,
    entry: &crate::KirEdge,
    value: ValueId,
) -> Option<ValueId> {
    header
        .params
        .iter()
        .position(|param| param.value == value)
        .and_then(|index| entry.args.get(index).copied())
        .or(Some(value))
}

fn successor_ids(terminator: &KirTerminator) -> Vec<BlockId> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge.target],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge.target, else_edge.target],
    }
}

fn incoming_values(function: &crate::KirFunction, target: BlockId, index: usize) -> Vec<ValueId> {
    function
        .blocks
        .iter()
        .flat_map(|block| match &block.terminator {
            KirTerminator::Return { .. } => Vec::new(),
            KirTerminator::Jump { edge } => vec![edge],
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
        })
        .filter(|edge| edge.target == target)
        .filter_map(|edge| edge.args.get(index).copied())
        .collect()
}

pub(crate) fn forwards_from(
    function: &crate::KirFunction,
    value: ValueId,
    origin: ValueId,
) -> bool {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut leaves = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if value == origin {
            leaves.insert(value);
            continue;
        }
        if !visited.insert(value) {
            continue;
        }
        if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            let incoming = incoming_values(function, block, index);
            if incoming.is_empty() {
                return false;
            }
            pending.extend(incoming);
        } else if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = defining_instruction(function, value)
        {
            pending.push(*value);
        } else {
            leaves.insert(value);
        }
    }
    leaves == BTreeSet::from([origin])
}

fn invariant_root_value(function: &crate::KirFunction, value: ValueId) -> Option<ValueId> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut roots = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if function.params.iter().any(|param| param.value == value) {
            roots.insert(value);
            continue;
        }
        if !visited.insert(value) {
            continue;
        }
        if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            pending.extend(incoming_values(function, block, index));
        } else if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = defining_instruction(function, value)
        {
            pending.push(*value);
        }
    }
    (roots.len() == 1).then(|| *roots.first().expect("one root"))
}

fn ordered_region_pair(
    left: MemoryRegionId,
    right: MemoryRegionId,
) -> (MemoryRegionId, MemoryRegionId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn has_noalias_fact(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    left: ValueId,
    right: ValueId,
) -> bool {
    pre_state.contract_facts().is_some_and(|contracts| {
        contracts.facts().facts().iter().any(|fact| {
            let scope_matches = match &fact.scope {
                crate::FactScope::FunctionEntry(owner)
                | crate::FactScope::Block {
                    function: owner, ..
                } => *owner == function,
                crate::FactScope::CalleeInstance { callee, .. } => *callee == function,
                crate::FactScope::InlineClone {
                    function: owner, ..
                } => *owner == function,
            };
            scope_matches
                && matches!(
                    fact.predicate,
                    crate::FactPredicate::Contract(
                        crate::ContractFactPredicate::NoAlias {
                            left: fact_left,
                            right: fact_right,
                        }
                    ) if (fact_left == left && fact_right == right)
                        || (fact_left == right && fact_right == left)
                )
        })
    })
}

fn original_header_body_induction_index(
    original: &crate::KirFunction,
    candidate: &CheckedVectorSource,
    body: &crate::KirBlock,
) -> Result<usize, String> {
    let header = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .ok_or_else(|| "vector source header is missing".to_string())?;
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return Err("vector source header is not a branch".to_string());
    };
    let index = then_edge
        .args
        .iter()
        .position(|value| *value == candidate.induction)
        .ok_or_else(|| "vector source body induction edge is missing".to_string())?;
    if body.params.get(index).is_none() {
        return Err("vector source body induction parameter is missing".to_string());
    }
    Ok(index)
}

// This checker reconstructs address evidence from the original CFG. It never
// consumes the discovery pass's affine summary as a correctness premise.
#[allow(clippy::too_many_arguments)]
fn reconstruct_wasm_affine_addresses(
    state: &KirVerifiedProgramState,
    function: &crate::KirFunction,
    preheader: BlockId,
    header: BlockId,
    body: BlockId,
    body_induction: ValueId,
    induction: ValueId,
    bound: ValueId,
    plan: &VectorizationPlan,
) -> Result<CheckedWasmAffine, TransactionCheckError> {
    if plan.predicates.iter().any(|p|matches!(p,crate::VectorPredicate::WasmSliceRange{requirement,..}
        if matches!(requirement.count,crate::WasmRangeCount::Invariant(_)|crate::WasmRangeCount::ScaledInvariant{..}))) {
        let source=crate::optimizer::stencil_vector::reconstruct_wasm_stencil_source_independently(state,function.id,header)
            .map_err(TransactionCheckError::compiler)?;
        if source.preheader!=preheader||source.body!=body||source.induction!=induction||source.body_induction!=body_induction||source.bound!=bound {
            return Err(TransactionCheckError::compiler("stencil loop coordinates differ from independent source proof"));
        }
        return checked_stencil_affine(function,&source,plan);
    }
    let error = TransactionCheckError::compiler;
    let body_block = source_block(function, body).ok_or_else(|| error("affine body is missing"))?;
    let u32_type = MirType::Primitive(MirPrimitiveTypeName::U32);
    let f64_type = MirType::Primitive(MirPrimitiveTypeName::F64);
    if value_type(function, induction) != Some(&u32_type)
        || value_type(function, bound) != Some(&u32_type)
    {
        return Err(error("affine loop requires u32 induction and bound"));
    }
    let root = |value| checked_affine_invariant(function, preheader, header, body, value);
    let mut addresses = BTreeMap::new();
    let mut setup = BTreeSet::new();
    let mut ranges = BTreeSet::new();
    let mut accesses = Vec::new();
    for instruction in &body_block.instructions {
        let (place, write) = match &instruction.kind {
            KirInstructionKind::Load { place } => (place.as_ref(), false),
            KirInstructionKind::Store { place, .. } => (place.as_ref(), true),
            _ => continue,
        };
        let crate::KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } = place
        else {
            return Err(error("affine memory access must be a slice index"));
        };
        // Slice identities remain distinct even when readonly slices share a
        // MemorySSA alias partition. Check that mapping without conflating IDs.
        if *type_node != f64_type
            || instruction.memory.as_ref().is_none_or(|memory| {
                function
                    .regions
                    .iter()
                    .find(|descriptor| descriptor.id == *region)
                    .is_none_or(|descriptor| descriptor.partition != memory.region)
                    || !function.regions.iter().any(|descriptor| {
                        descriptor.id == memory.region && descriptor.partition == memory.region
                    })
            })
            || value_type(function, *index) != Some(&u32_type)
        {
            return Err(error(
                "affine memory access has a false type or MemorySSA region",
            ));
        }
        let slice =
            root(*slice).ok_or_else(|| error("affine slice does not dominate the preheader"))?;
        let is_induction = |value| {
            forwards_from(function, value, body_induction)
                || forwards_from(function, value, induction)
        };
        let address = if is_induction(*index) {
            CheckedWasmAddress::Contiguous(None)
        } else if let Some(index) = root(*index) {
            if write {
                return Err(error("affine broadcast store is unsupported"));
            }
            CheckedWasmAddress::Broadcast(index)
        } else {
            let definition = defining_instruction(function, *index)
                .filter(|definition| {
                    body_block
                        .instructions
                        .iter()
                        .any(|item| item.id == definition.id)
                })
                .ok_or_else(|| error("affine index is not a body address definition"))?;
            let KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: crate::KirArithmeticSemantics::Modular,
            } = definition.kind
            else {
                return Err(error(
                    "affine address must be unchecked offset plus induction",
                ));
            };
            if definition.memory.is_some()
                || definition.effect.is_some()
                || definition.results.len() != 1
            {
                return Err(error(
                    "affine address setup has effects or multiple results",
                ));
            }
            let offset = if is_induction(left) {
                root(right)
            } else if is_induction(right) {
                root(left)
            } else {
                None
            }
            .ok_or_else(|| error("affine address offset is not independently invariant"))?;
            if value_type(function, offset) != Some(&u32_type) {
                return Err(error("affine offset is not u32"));
            }
            setup.insert(definition.id);
            CheckedWasmAddress::Contiguous(Some(offset))
        };
        let (start, count) = match address {
            CheckedWasmAddress::Contiguous(offset) => {
                (offset, crate::WasmRangeCount::TripBound(bound))
            }
            CheckedWasmAddress::Broadcast(index) => (Some(index), crate::WasmRangeCount::One),
        };
        ranges.insert(crate::WasmSliceRangeRequirement {
            slice,
            start,
            count,
            element_bytes: 8,
        });
        addresses.insert(instruction.id, address);
        accesses.push((instruction.id, slice, *region, write, address));
    }
    let direct_map = accesses.len() == 2
        && setup.is_empty()
        && ranges.len() == 2
        && accesses
            .iter()
            .filter(|(_, _, _, write, _)| !*write)
            .count()
            == 1
        && accesses.iter().filter(|(_, _, _, write, _)| *write).count() == 1
        && accesses
            .iter()
            .all(|(_, _, _, _, address)| *address == CheckedWasmAddress::Contiguous(None))
        && accesses
            .iter()
            .map(|(_, slice, _, _, _)| *slice)
            .collect::<BTreeSet<_>>()
            .len()
            == 2
        && ranges.iter().all(|range| {
            range.element_bytes == 8
                && range.start.is_none()
                && range.count == crate::WasmRangeCount::TripBound(bound)
        });
    let legacy_broadcast = ranges.len() == 3
        && addresses
            .values()
            .any(|address| matches!(address, CheckedWasmAddress::Broadcast(_)));
    if setup.len() > 2
        || (!direct_map && !legacy_broadcast)
        || !accesses.iter().any(|(_, _, _, write, _)| *write)
    {
        return Err(error(
            "affine source range shape is outside the direct-map and readonly-broadcast schemas",
        ));
    }
    for (index, (_, left, left_region, left_write, left_address)) in accesses.iter().enumerate() {
        for (_, right, right_region, right_write, right_address) in accesses.iter().skip(index + 1)
        {
            if !left_write && !right_write {
                continue;
            }
            if left == right {
                if left_region != right_region
                    || left_address != right_address
                    || matches!(left_address, CheckedWasmAddress::Broadcast(_))
                {
                    return Err(error(
                        "affine output reads and writes have different complete addresses",
                    ));
                }
            } else if !checked_affine_noalias(state, function, preheader, *left, *right) {
                return Err(error(
                    "affine write alias freedom is not available at its preheader",
                ));
            }
        }
    }
    for setup_id in &setup {
        let value = defining_instruction_by_id(function, *setup_id)
            .and_then(|instruction| instruction.results.first())
            .map(|result| result.value)
            .ok_or_else(|| error("affine address setup has no result"))?;
        for block in &function.blocks {
            for instruction in &block.instructions {
                let mut uses = 0;
                super::analysis::visit_instruction_uses(instruction, &mut |operand| {
                    uses += usize::from(operand == value);
                });
                if uses == 0 {
                    continue;
                }
                let address_use = match &instruction.kind {
                    KirInstructionKind::Load { place }
                    | KirInstructionKind::Store { place, .. } => {
                        matches!(place.as_ref(), crate::KirPlace::SliceIndex { index, .. } if *index == value)
                    }
                    _ => false,
                };
                if uses != 1 || !address_use || !addresses.contains_key(&instruction.id) {
                    return Err(error(
                        "affine address setup escapes its covered memory indices",
                    ));
                }
            }
            let used = match &block.terminator {
                KirTerminator::Return { value: result, .. } => *result == Some(value),
                KirTerminator::Jump { edge } => edge.args.contains(&value),
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => {
                    *condition == value
                        || then_edge.args.contains(&value)
                        || else_edge.args.contains(&value)
                }
            };
            if used {
                return Err(error(
                    "affine address setup escapes through a control-flow edge",
                ));
            }
        }
    }
    let mut recorded = BTreeSet::new();
    for group in &plan.memory_groups {
        let [source] = group.scalar_instructions.as_slice() else {
            return Err(error(
                "affine vector group does not name exactly one source",
            ));
        };
        let Some((_, _, region, write, CheckedWasmAddress::Contiguous(_))) =
            accesses.iter().find(|access| access.0 == *source)
        else {
            return Err(error(
                "affine vector group does not cover a contiguous source access",
            ));
        };
        if group.unroll_index >= plan.uf
            || !recorded.insert((*source, group.unroll_index))
            || *region != group.region
            || (*write != (group.access == VectorMemoryAccessKind::Write))
        {
            return Err(error("affine vector footprint identity is false"));
        }
    }
    for group in &plan.broadcast_groups {
        let Some((_, _, region, false, CheckedWasmAddress::Broadcast(_))) = accesses
            .iter()
            .find(|access| access.0 == group.scalar_instruction)
        else {
            return Err(error(
                "affine broadcast group does not cover an invariant read",
            ));
        };
        if group.unroll_index >= plan.uf
            || !recorded.insert((group.scalar_instruction, group.unroll_index))
            || *region != group.region
        {
            return Err(error("affine broadcast footprint identity is false"));
        }
    }
    let strict_mul_add = body_block
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            KirInstructionKind::Binary {
                op,
                semantics: crate::KirArithmeticSemantics::StrictFloat,
                ..
            } if matches!(op, MirBinaryOp::Mul | MirBinaryOp::Add) => Some(op),
            _ => None,
        })
        .collect::<Vec<_>>();
    let shared_matmul_broadcast = plan.uf > 1
        && plan.vf == 2
        && accesses.len() == 4
        && addresses.len() == 4
        && setup.len() == 2
        && ranges.len() == 3
        && addresses
            .values()
            .filter(|address| matches!(address, CheckedWasmAddress::Broadcast(_)))
            .count()
            == 1
        && strict_mul_add == [MirBinaryOp::Mul, MirBinaryOp::Add]
        && plan.broadcast_groups.len() == 1
        && plan.broadcast_groups[0].unroll_index == 0
        && plan.memory_groups.len() == 3 * usize::from(plan.uf);
    let expected_recordings = accesses
        .iter()
        .flat_map(|(instruction, _, _, _, address)| match address {
            CheckedWasmAddress::Contiguous(_) => (0..plan.uf)
                .map(|unroll_index| (*instruction, unroll_index))
                .collect::<Vec<_>>(),
            CheckedWasmAddress::Broadcast(_) if shared_matmul_broadcast => {
                vec![(*instruction, 0)]
            }
            CheckedWasmAddress::Broadcast(_) => (0..plan.uf)
                .map(|unroll_index| (*instruction, unroll_index))
                .collect::<Vec<_>>(),
        })
        .collect::<BTreeSet<_>>();
    if recorded != expected_recordings {
        return Err(error(
            "affine memory groups do not cover every original read and write",
        ));
    }
    let planned = plan
        .predicates
        .iter()
        .filter_map(|predicate| match predicate {
            crate::VectorPredicate::WasmSliceRange { requirement, .. } => Some(*requirement),
            _ => None,
        })
        .collect::<Vec<_>>();
    if planned.len() != ranges.len()
        || planned.iter().copied().collect::<BTreeSet<_>>() != ranges
        || plan.predicates.len() != ranges.len() + 1
    {
        return Err(error(
            "affine ranges do not exactly close the original scalar footprints",
        ));
    }
    Ok(CheckedWasmAffine {
        addresses,
        setup,
        ranges,
    })
}

fn checked_stencil_affine(
    function: &crate::KirFunction,
    source: &crate::WasmStencilSource,
    plan: &VectorizationPlan,
) -> Result<CheckedWasmAffine, TransactionCheckError> {
    let error = TransactionCheckError::compiler;
    let mut addresses = source
        .loads
        .iter()
        .map(|load| {
            (
                load.instruction,
                CheckedWasmAddress::Contiguous(Some(load.origin)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    addresses.insert(
        source.store,
        CheckedWasmAddress::Contiguous(Some(source.store_origin)),
    );
    let ranges = source
        .ranges
        .iter()
        .map(|range| crate::WasmSliceRangeRequirement {
            slice: range.slice,
            start: range.start,
            element_bytes: 8,
            count: match range.count {
                crate::StencilRangeCount::Width => crate::WasmRangeCount::Invariant(source.width),
                crate::StencilRangeCount::ThreeWidths => crate::WasmRangeCount::ScaledInvariant {
                    value: source.width,
                    scale: 3,
                },
                crate::StencilRangeCount::InteriorTrip => {
                    crate::WasmRangeCount::TripBound(source.bound)
                }
            },
        })
        .collect::<BTreeSet<_>>();
    let planned = plan
        .predicates
        .iter()
        .filter_map(|p| match p {
            crate::VectorPredicate::WasmSliceRange { requirement, .. } => Some(*requirement),
            _ => None,
        })
        .collect::<Vec<_>>();
    if plan.predicates.len() != 4
        || planned.len() != 3
        || planned.iter().copied().collect::<BTreeSet<_>>() != ranges
        || !plan.broadcast_groups.is_empty()
    {
        return Err(error("stencil range recipe or broadcast partition differs"));
    }
    let mut recorded = BTreeSet::new();
    for group in &plan.memory_groups {
        let [instruction] = group.scalar_instructions.as_slice() else {
            return Err(error("stencil memory group must have exactly one source"));
        };
        let original = defining_instruction_by_id(function, *instruction)
            .ok_or_else(|| error("stencil group source is absent"))?;
        let place = match &original.kind {
            KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => place,
            _ => return Err(error("stencil group is not a memory access")),
        };
        let crate::KirPlace::SliceIndex { region, .. } = place.as_ref() else {
            return Err(error("stencil group is not a slice index"));
        };
        if !addresses.contains_key(instruction)
            || !recorded.insert(*instruction)
            || group.unroll_index != 0
            || group.region != *region
            || (group.access == VectorMemoryAccessKind::Write) != (*instruction == source.store)
        {
            return Err(error("stencil group identity, region or mode changed"));
        }
    }
    if recorded != addresses.keys().copied().collect() {
        return Err(error("stencil groups fail to cover all ten memory effects"));
    }
    Ok(CheckedWasmAffine {
        addresses,
        setup: source.scalar_address_setup.iter().copied().collect(),
        ranges,
    })
}

fn checked_wasm_direct_map_interleave(
    bound: ValueId,
    plan: &VectorizationPlan,
    affine: &CheckedWasmAffine,
    operations: &[CheckedVectorOperation],
    accesses: &[CheckedVectorAccess],
) -> bool {
    if !matches!(plan.uf, 2 | 4)
        || plan.vf != 2
        || affine.addresses.len() != 2
        || !affine.setup.is_empty()
        || affine.ranges.len() != 2
        || !plan.broadcast_groups.is_empty()
        || plan.predicates.len() != 3
        || plan.memory_groups.len() != 2 * usize::from(plan.uf)
        || operations.is_empty()
        || operations.iter().any(|operation| {
            operation.lane_type != KirLaneType::F64
                || operation.semantics != KirCostSemantics::StrictFloat
                || !matches!(
                    operation.operation,
                    KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
                        | KirProfileOperation::Divide
                        | KirProfileOperation::Negate
                )
        })
        || accesses.len() != 2
        || accesses
            .iter()
            .filter(|access| access.kind == CheckedMemoryAccessKind::Read)
            .count()
            != 1
        || accesses
            .iter()
            .filter(|access| access.kind == CheckedMemoryAccessKind::Write)
            .count()
            != 1
        || accesses.iter().any(|access| access.element_bytes != 8)
        || affine
            .addresses
            .values()
            .any(|address| *address != CheckedWasmAddress::Contiguous(None))
        || affine.ranges.iter().any(|range| {
            range.element_bytes != 8
                || range.start.is_some()
                || range.count != crate::WasmRangeCount::TripBound(bound)
        })
        || affine
            .ranges
            .iter()
            .map(|range| range.slice)
            .collect::<BTreeSet<_>>()
            .len()
            != 2
    {
        return false;
    }
    let expected = affine
        .addresses
        .keys()
        .flat_map(|instruction| (0..plan.uf).map(move |unroll_index| (*instruction, unroll_index)))
        .collect::<BTreeSet<_>>();
    let actual = plan
        .memory_groups
        .iter()
        .filter_map(|group| {
            let [instruction] = group.scalar_instructions.as_slice() else {
                return None;
            };
            Some((*instruction, group.unroll_index))
        })
        .collect::<BTreeSet<_>>();
    actual == expected
}

fn checked_wasm_matmul_interleave(source: CheckedMatmulInterleaveSource<'_>) -> bool {
    let CheckedMatmulInterleaveSource {
        function,
        header_id,
        body_id,
        induction,
        scalar_blocks,
        has_diamond,
        has_reduction,
        bound,
        plan,
        affine,
        operations,
        accesses,
    } = source;
    if !matches!(plan.uf, 2 | 4)
        || plan.vf != 2
        || has_diamond
        || has_reduction
        || scalar_blocks != [body_id]
        || plan.predicates.len() != 4
        || affine.addresses.len() != 4
        || affine.setup.len() != 2
        || affine.ranges.len() != 3
        || operations.len() != 2
        || operations
            .iter()
            .map(|operation| operation.operation)
            .collect::<Vec<_>>()
            != [KirProfileOperation::Multiply, KirProfileOperation::Add]
        || operations.iter().any(|operation| {
            operation.lane_type != KirLaneType::F64
                || operation.semantics != KirCostSemantics::StrictFloat
        })
        || accesses.len() != 4
        || accesses.iter().any(|access| access.element_bytes != 8)
        || accesses
            .iter()
            .filter(|access| access.kind == CheckedMemoryAccessKind::Read)
            .count()
            != 3
        || accesses
            .iter()
            .filter(|access| access.kind == CheckedMemoryAccessKind::Write)
            .count()
            != 1
    {
        return false;
    }
    let Some(body) = source_block(function, body_id) else {
        return false;
    };
    let source_arithmetic = body
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            KirInstructionKind::Binary {
                op,
                semantics: crate::KirArithmeticSemantics::StrictFloat,
                ..
            } if matches!(op, MirBinaryOp::Mul | MirBinaryOp::Add) => Some((instruction.id, *op)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if source_arithmetic.len() != 2
        || source_arithmetic
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
            != operations
                .iter()
                .map(|operation| operation.scalar)
                .collect::<Vec<_>>()
        || source_arithmetic
            .iter()
            .map(|(_, op)| *op)
            .collect::<Vec<_>>()
            != [MirBinaryOp::Mul, MirBinaryOp::Add]
    {
        return false;
    }
    let broadcast = affine
        .addresses
        .iter()
        .filter_map(|(instruction, address)| {
            matches!(address, CheckedWasmAddress::Broadcast(_)).then_some(*instruction)
        })
        .collect::<Vec<_>>();
    let [broadcast_instruction] = broadcast.as_slice() else {
        return false;
    };
    let Some(broadcast_access) = accesses
        .iter()
        .find(|access| access.instruction == *broadcast_instruction)
    else {
        return false;
    };
    if broadcast_access.kind != CheckedMemoryAccessKind::Read {
        return false;
    }
    let contiguous = accesses
        .iter()
        .filter(|access| access.instruction != *broadcast_instruction)
        .collect::<Vec<_>>();
    if contiguous.len() != 3
        || contiguous.iter().any(|access| {
            !matches!(
                affine.addresses.get(&access.instruction),
                Some(CheckedWasmAddress::Contiguous(Some(_)))
            )
        })
    {
        return false;
    }
    let mut contiguous_by_slice = BTreeMap::<ValueId, Vec<&CheckedVectorAccess>>::new();
    for access in &contiguous {
        contiguous_by_slice
            .entry(access.base)
            .or_default()
            .push(access);
    }
    if contiguous_by_slice.len() != 2 {
        return false;
    }
    let mut output_slice = None;
    let mut b_slice = None;
    for (slice, slice_accesses) in &contiguous_by_slice {
        match slice_accesses.as_slice() {
            [read, write]
                if read.kind == CheckedMemoryAccessKind::Read
                    && write.kind == CheckedMemoryAccessKind::Write =>
            {
                if affine.addresses.get(&read.instruction)
                    != affine.addresses.get(&write.instruction)
                    || read.region != write.region
                {
                    return false;
                }
                output_slice = Some(*slice);
            }
            [read] if read.kind == CheckedMemoryAccessKind::Read => {
                b_slice = Some(*slice);
            }
            _ => return false,
        }
    }
    let (Some(output_slice), Some(b_slice)) = (output_slice, b_slice) else {
        return false;
    };
    if output_slice == b_slice
        || output_slice == broadcast_access.base
        || b_slice == broadcast_access.base
    {
        return false;
    }
    let Some(output_read) = contiguous
        .iter()
        .find(|access| access.base == output_slice && access.kind == CheckedMemoryAccessKind::Read)
    else {
        return false;
    };
    let Some(output_write) = contiguous.iter().find(|access| {
        access.base == output_slice && access.kind == CheckedMemoryAccessKind::Write
    }) else {
        return false;
    };
    let Some(b_read) = contiguous
        .iter()
        .find(|access| access.base == b_slice && access.kind == CheckedMemoryAccessKind::Read)
    else {
        return false;
    };
    let Some(multiply_instruction) = operations
        .iter()
        .find(|operation| operation.operation == KirProfileOperation::Multiply)
        .map(|operation| operation.scalar)
    else {
        return false;
    };
    let Some(add_instruction) = operations
        .iter()
        .find(|operation| operation.operation == KirProfileOperation::Add)
        .map(|operation| operation.scalar)
    else {
        return false;
    };
    if !checked_matmul_source_dataflow_is_closed(
        function,
        header_id,
        body_id,
        induction,
        CheckedMatmulSourceInstructionIds {
            broadcast: *broadcast_instruction,
            output_read: output_read.instruction,
            b_read: b_read.instruction,
            output_write: output_write.instruction,
            multiply: multiply_instruction,
            add: add_instruction,
        },
    ) {
        return false;
    }
    let Some(CheckedWasmAddress::Broadcast(broadcast_index)) =
        affine.addresses.get(broadcast_instruction)
    else {
        return false;
    };
    let ranges = affine.ranges.iter().copied().collect::<Vec<_>>();
    if ranges
        .iter()
        .filter(|range| {
            (forwards_from(function, range.slice, broadcast_access.base)
                || forwards_from(function, broadcast_access.base, range.slice))
                && range.start == Some(*broadcast_index)
                && range.count == crate::WasmRangeCount::One
                && range.element_bytes == 8
        })
        .count()
        != 1
    {
        return false;
    }
    for slice in [b_slice, output_slice] {
        let slice_access = contiguous
            .iter()
            .find(|access| access.base == slice)
            .expect("slice came from contiguous access");
        let Some(CheckedWasmAddress::Contiguous(Some(offset))) =
            affine.addresses.get(&slice_access.instruction)
        else {
            return false;
        };
        if ranges
            .iter()
            .filter(|range| {
                forwards_from(function, range.slice, slice)
                    && range.start == Some(*offset)
                    && range.count == crate::WasmRangeCount::TripBound(bound)
                    && range.element_bytes == 8
            })
            .count()
            != 1
        {
            return false;
        }
    }
    if ranges.iter().any(|range| {
        range.element_bytes != 8
            || !(forwards_from(function, range.slice, broadcast_access.base)
                || forwards_from(function, broadcast_access.base, range.slice)
                || forwards_from(function, range.slice, b_slice)
                || forwards_from(function, range.slice, output_slice))
    }) {
        return false;
    }

    let expected_broadcasts = BTreeSet::from([(*broadcast_instruction, 0)]);
    let actual_broadcasts = plan
        .broadcast_groups
        .iter()
        .map(|group| {
            if group.region != broadcast_access.region
                || group.scalar_instruction != *broadcast_instruction
            {
                return None;
            }
            Some((group.scalar_instruction, group.unroll_index))
        })
        .collect::<Option<BTreeSet<_>>>();
    if actual_broadcasts != Some(expected_broadcasts) {
        return false;
    }
    let expected_memory = contiguous
        .iter()
        .flat_map(|access| {
            let kind = match access.kind {
                CheckedMemoryAccessKind::Read => VectorMemoryAccessKind::Read,
                CheckedMemoryAccessKind::Write => VectorMemoryAccessKind::Write,
            };
            (0..plan.uf)
                .map(move |unroll_index| (access.instruction, unroll_index, kind, access.region))
        })
        .collect::<BTreeSet<_>>();
    let actual_memory = plan
        .memory_groups
        .iter()
        .filter_map(|group| {
            let [instruction] = group.scalar_instructions.as_slice() else {
                return None;
            };
            Some((*instruction, group.unroll_index, group.access, group.region))
        })
        .collect::<BTreeSet<_>>();
    expected_memory == actual_memory
}

fn checked_matmul_source_dataflow_is_closed(
    function: &crate::KirFunction,
    header_id: BlockId,
    body_id: BlockId,
    induction: ValueId,
    instructions: CheckedMatmulSourceInstructionIds,
) -> bool {
    let CheckedMatmulSourceInstructionIds {
        broadcast,
        output_read,
        b_read,
        output_write,
        multiply,
        add,
    } = instructions;
    let Some(body) = source_block(function, body_id) else {
        return false;
    };
    let result = |instruction_id| {
        body.instructions
            .iter()
            .find(|instruction| instruction.id == instruction_id)
            .and_then(|instruction| match instruction.results.as_slice() {
                [result] => Some(result.value),
                _ => None,
            })
    };
    let (Some(a_value), Some(output_value), Some(b_value)) =
        (result(broadcast), result(output_read), result(b_read))
    else {
        return false;
    };
    let Some(multiply_source) = body
        .instructions
        .iter()
        .find(|instruction| instruction.id == multiply)
    else {
        return false;
    };
    let Some(add_source) = body
        .instructions
        .iter()
        .find(|instruction| instruction.id == add)
    else {
        return false;
    };
    let (
        KirInstructionKind::Binary {
            op: MirBinaryOp::Mul,
            left: multiply_left,
            right: multiply_right,
            semantics: crate::KirArithmeticSemantics::StrictFloat,
        },
        [multiply_result],
    ) = (&multiply_source.kind, multiply_source.results.as_slice())
    else {
        return false;
    };
    let (
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: add_left,
            right: add_right,
            semantics: crate::KirArithmeticSemantics::StrictFloat,
        },
        [add_result],
    ) = (&add_source.kind, add_source.results.as_slice())
    else {
        return false;
    };
    let multiplication_inputs = BTreeSet::from([
        independent_scalar_copy_root(function, *multiply_left),
        independent_scalar_copy_root(function, *multiply_right),
    ]);
    let expected_multiplication_inputs = BTreeSet::from([
        independent_scalar_copy_root(function, a_value),
        independent_scalar_copy_root(function, b_value),
    ]);
    let addition_inputs = BTreeSet::from([
        independent_scalar_copy_root(function, *add_left),
        independent_scalar_copy_root(function, *add_right),
    ]);
    let expected_addition_inputs = BTreeSet::from([
        independent_scalar_copy_root(function, output_value),
        independent_scalar_copy_root(function, multiply_result.value),
    ]);
    let Some(store_source) = body
        .instructions
        .iter()
        .find(|instruction| instruction.id == output_write)
    else {
        return false;
    };
    let KirInstructionKind::Store {
        value: stored_value,
        ..
    } = &store_source.kind
    else {
        return false;
    };
    if multiplication_inputs != expected_multiplication_inputs
        || addition_inputs != expected_addition_inputs
        || independent_scalar_copy_root(function, *stored_value)
            != independent_scalar_copy_root(function, add_result.value)
    {
        return false;
    }

    // The unroller rebuilds every chunk from the loop-header state and only
    // forwards the final chunk to the vector backedge. Therefore every
    // non-induction scalar loop-carried value must be transparent across the
    // source backedge; recurrences require a separate proof and stay scalar.
    let Some(header) = source_block(function, header_id) else {
        return false;
    };
    let Some(induction_index) = header
        .params
        .iter()
        .position(|parameter| parameter.value == induction)
    else {
        return false;
    };
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return false;
    };
    if then_edge.target != body_id {
        return false;
    }
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return false;
    };
    backedge.target == header_id
        && backedge.args.len() == header.params.len()
        && !header.params.iter().enumerate().any(|(index, parameter)| {
            index != induction_index
                && !backedge
                    .args
                    .get(index)
                    .is_some_and(|value| forwards_from(function, *value, parameter.value))
        })
}

fn checked_affine_invariant(
    function: &crate::KirFunction,
    preheader: BlockId,
    header: BlockId,
    body: BlockId,
    value: ValueId,
) -> Option<ValueId> {
    let dominators = compute_kir_dominators(function);
    let mut pending = vec![value];
    let mut seen = BTreeSet::new();
    let mut roots = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if !seen.insert(value) {
            continue;
        }
        if function.params.iter().any(|param| param.value == value) {
            roots.insert(value);
            continue;
        }
        let mut found = false;
        for block in &function.blocks {
            if let Some(instruction) = block.instructions.iter().find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            }) {
                found = true;
                if let KirInstructionKind::Copy { value } = instruction.kind {
                    pending.push(value);
                } else if block.id != header
                    && block.id != body
                    && dominators.dominates(block.id, preheader)
                {
                    roots.insert(value);
                } else {
                    return None;
                }
                break;
            }
            if let Some(index) = block.params.iter().position(|param| param.value == value) {
                found = true;
                if block.id == header || block.id == body {
                    let incoming = incoming_values(function, block.id, index);
                    if incoming.is_empty() {
                        return None;
                    }
                    pending.extend(incoming);
                } else if dominators.dominates(block.id, preheader) {
                    roots.insert(value);
                } else {
                    return None;
                }
                break;
            }
        }
        if !found {
            return None;
        }
    }
    if roots.len() == 1 {
        roots.into_iter().next()
    } else {
        None
    }
}

fn checked_affine_noalias(
    state: &KirVerifiedProgramState,
    function: &crate::KirFunction,
    preheader: BlockId,
    left: ValueId,
    right: ValueId,
) -> bool {
    let dominators = compute_kir_dominators(function);
    state.contract_facts().is_some_and(|contracts| contracts.facts().facts().iter().any(|fact| {
        let available = match &fact.scope {
            crate::FactScope::FunctionEntry(owner) => *owner == function.id,
            crate::FactScope::Block { function: owner, block } => *owner == function.id && dominators.dominates(*block, preheader),
            // Instance-local evidence requires a separate mapping proof.
            crate::FactScope::CalleeInstance { .. } | crate::FactScope::InlineClone { .. } => false,
        };
        available && matches!(fact.predicate, crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias { left: a, right: b })
            if (forwards_from(function, left, a) && forwards_from(function, right, b))
                || (forwards_from(function, left, b) && forwards_from(function, right, a)))
    }))
}

#[allow(clippy::too_many_arguments)]
fn check_wasm_affine_predicates(
    function: &crate::KirFunction,
    preheader: &crate::KirBlock,
    condition: ValueId,
    bound: ValueId,
    candidate: &CheckedVectorSource,
    affine: &CheckedWasmAffine,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let error = TransactionCheckError::compiler;
    let emitted = preheader
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .collect::<Vec<_>>();
    let [instruction] = emitted.as_slice() else {
        return Err(error("affine preheader requires one exact range predicate"));
    };
    let KirInstructionKind::VersionPredicate { predicate } = &instruction.kind else {
        unreachable!()
    };
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
        || instruction.results[0].value != condition
        || instruction.results[0].type_node.as_scalar()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::Bool))
        || predicate.address_bits != 32
        || predicate.conjuncts.len() != affine.ranges.len() + 1
    {
        return Err(error(
            "affine predicate is not a pure closed wasm32 range guard",
        ));
    }
    let mut actual = BTreeSet::new();
    let mut thresholds = 0;
    for conjunct in &predicate.conjuncts {
        match conjunct {
            crate::KirVersionPredicateConjunct::TripThreshold { value, minimum }
                if *value == bound && *minimum == candidate.minimum_trip =>
            {
                thresholds += 1
            }
            crate::KirVersionPredicateConjunct::WasmSliceRange {
                slice,
                start,
                count,
                element_bytes,
            } => {
                let matching = affine.ranges.iter().find(|range| {
                    range.slice == *slice
                        && range.element_bytes == *element_bytes
                        && range.start.map_or_else(
                            || integer_constant(function, *start) == Some(0),
                            |value| value == *start,
                        )
                        && match range.count {
                            crate::WasmRangeCount::TripBound(_) => *count == bound,
                            crate::WasmRangeCount::Invariant(value) => *count == value,
                            crate::WasmRangeCount::ScaledInvariant { value, scale } => {
                                scale==3 && preheader.instructions.iter().any(|setup|{
                                    setup.results.len()==1&&setup.results[0].value==*count
                                        &&setup.results[0].type_node.as_scalar()==Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
                                        &&setup.memory.is_none()&&setup.effect.is_none()
                                        &&matches!(setup.kind,KirInstructionKind::Binary{op:MirBinaryOp::Mul,left,right,semantics:crate::KirArithmeticSemantics::Modular}
                                            if left==value&&integer_constant(function,right)==Some(3))
                                })
                            },
                            crate::WasmRangeCount::One => {
                                integer_constant(function, *count) == Some(1)
                            }
                        }
                });
                if matching.is_none_or(|range| !actual.insert(*range)) {
                    return Err(error(
                        "affine emitted range does not match its source footprint",
                    ));
                }
            }
            _ => {
                return Err(error(
                    "affine predicate has an unsupported or false conjunct",
                ));
            }
        }
    }
    let thresholds_planned = plan
        .predicates
        .iter()
        .filter_map(|predicate| match predicate {
            crate::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } => Some((*trip_count, *minimum)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if thresholds != 1
        || actual != affine.ranges
        || thresholds_planned != [(candidate.bound, candidate.minimum_trip)]
    {
        return Err(error("affine trip threshold or range set is incomplete"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_wasm_affine_emission(
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    candidate: &CheckedVectorSource,
    affine: &CheckedWasmAffine,
    source_header: &crate::KirBlock,
    source_body: &crate::KirBlock,
    header: &crate::KirBlock,
    body: &crate::KirBlock,
    body_edge: &crate::KirEdge,
    epilogue: &crate::KirEdge,
    preheader_before: &crate::KirBlock,
    preheader: &crate::KirBlock,
    bound: ValueId,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let error = TransactionCheckError::compiler;
    let scalar_u32 = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32));
    let vector_type = KirValueType::FixedVector {
        lane: KirLaneType::F64,
        lanes: 2,
    };
    let KirTerminator::Branch {
        then_edge: source_body_edge,
        ..
    } = &source_header.terminator
    else {
        return Err(error("affine source header is not a branch"));
    };
    let KirTerminator::Jump {
        edge: source_backedge,
    } = &source_body.terminator
    else {
        return Err(error("affine source backedge is missing"));
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err(error("affine vector backedge is missing"));
    };
    if transformed.blocks.len() != original.blocks.len() + 2
        || header.params.len() != source_header.params.len()
        || body.params.len() != source_body.params.len()
        || header.memory_params.len() != source_header.memory_params.len()
        || !body.memory_params.is_empty()
        || !body_edge.memory_args.is_empty()
        || source_header
            .instructions
            .iter()
            .any(|instruction| instruction.memory.is_some() || instruction.effect.is_some())
    {
        return Err(error("affine loop block or parameter partition is false"));
    }
    for block in &original.blocks {
        for target in successor_ids(&block.terminator) {
            if (target == source_header.id
                && block.id != preheader_before.id
                && block.id != source_body.id)
                || (target == source_body.id && block.id != source_header.id)
            {
                return Err(error("affine scalar loop has an unaccounted entry edge"));
            }
        }
    }
    let mut values = BTreeMap::new();
    for (source, emitted) in source_header
        .params
        .iter()
        .zip(&header.params)
        .chain(source_body.params.iter().zip(&body.params))
    {
        if source.type_node != emitted.type_node {
            return Err(error("affine cloned scalar parameter type differs"));
        }
        values.insert(source.value, (emitted.value, false));
    }
    let mapped_scalar = |value, values: &BTreeMap<ValueId, (ValueId, bool)>| {
        values.get(&value).copied().unwrap_or((value, false))
    };
    if source_body_edge
        .args
        .iter()
        .map(|value| mapped_scalar(*value, &values).0)
        .collect::<Vec<_>>()
        != body_edge.args
        || epilogue.args
            != header
                .params
                .iter()
                .map(|param| param.value)
                .collect::<Vec<_>>()
        || epilogue.memory_args
            != header
                .memory_params
                .iter()
                .map(|param| param.version)
                .collect::<Vec<_>>()
    {
        return Err(error(
            "affine vector body or scalar tail edge changed carried state",
        ));
    }
    let mut memories = BTreeMap::new();
    for (source, emitted) in source_header
        .memory_params
        .iter()
        .zip(&header.memory_params)
    {
        if source.region != emitted.region {
            return Err(error("affine header MemorySSA region differs"));
        }
        memories.insert(source.version, emitted.version);
    }
    for (param, argument) in source_body
        .memory_params
        .iter()
        .zip(&source_body_edge.memory_args)
    {
        let version = memories
            .get(argument)
            .copied()
            .ok_or_else(|| error("affine body memory does not originate at header"))?;
        memories.insert(param.version, version);
    }
    let induction_index = source_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .ok_or_else(|| error("affine induction parameter is missing"))?;
    let updated = *source_backedge
        .args
        .get(induction_index)
        .ok_or_else(|| error("affine induction backedge is missing"))?;
    let update = defining_instruction(original, updated)
        .ok_or_else(|| error("affine induction update is missing"))?;
    let body_iv = *source_body
        .params
        .get(
            original_header_body_induction_index(original, candidate, source_body)
                .map_err(TransactionCheckError::compiler)?,
        )
        .map(|param| &param.value)
        .ok_or_else(|| error("affine body induction is missing"))?;
    let mut cursor = 0;
    let mut splats = BTreeMap::new();
    let mut previous_effect = None;
    let mut endpoints = BTreeSet::new();
    for source in &source_body.instructions {
        if let KirInstructionKind::Copy { value } = source.kind {
            let source_result = checked_single_result(source, None)?;
            values.insert(source_result, mapped_scalar(value, &values));
            continue;
        }
        if matches!(source.kind, KirInstructionKind::ConstInt { .. }) && source.results.len() == 1 {
            let value = source.results[0].value;
            let users = original
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| {
                    let mut used = false;
                    super::analysis::visit_instruction_uses(instruction, &mut |operand| {
                        used |= operand == value;
                    });
                    used
                })
                .map(|instruction| instruction.id)
                .collect::<BTreeSet<_>>();
            if users == BTreeSet::from([update.id]) {
                continue;
            }
        }
        if source.id == update.id {
            let stride = body
                .instructions
                .get(cursor)
                .ok_or_else(|| error("affine vector stride is missing"))?;
            cursor += 1;
            let stride_value = checked_single_result(stride, Some(&scalar_u32))?;
            if stride.memory.is_some()
                || stride.effect.is_some()
                || !matches!(&stride.kind, KirInstructionKind::ConstInt { value } if value == "2")
            {
                return Err(error("affine vector stride is not exact u32 two"));
            }
            let emitted = body
                .instructions
                .get(cursor)
                .ok_or_else(|| error("affine vector induction update is missing"))?;
            cursor += 1;
            if !matches!(emitted.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left, right, semantics: crate::KirArithmeticSemantics::Modular } if left == mapped_scalar(body_iv, &values).0 && right == stride_value)
                || emitted.memory.is_some()
                || emitted.effect.is_some()
            {
                return Err(error("affine vector induction update differs"));
            }
            values.insert(
                updated,
                (checked_single_result(emitted, Some(&scalar_u32))?, false),
            );
            continue;
        }
        if matches!(
            source.kind,
            KirInstructionKind::ConstInt { .. } | KirInstructionKind::ConstFloat { .. }
        ) || affine.setup.contains(&source.id)
        {
            let emitted = body
                .instructions
                .get(cursor)
                .ok_or_else(|| error("affine scalar setup is missing"))?;
            cursor += 1;
            let same = match (&source.kind, &emitted.kind) {
                (
                    KirInstructionKind::ConstInt { value: a },
                    KirInstructionKind::ConstInt { value: b },
                )
                | (
                    KirInstructionKind::ConstFloat { value: a },
                    KirInstructionKind::ConstFloat { value: b },
                ) => a == b,
                (
                    KirInstructionKind::Binary {
                        op: source_op,
                        left: a,
                        right: b,
                        semantics: crate::KirArithmeticSemantics::Modular,
                    },
                    KirInstructionKind::Binary {
                        op: emitted_op,
                        left: c,
                        right: d,
                        semantics: crate::KirArithmeticSemantics::Modular,
                    },
                ) => {
                    matches!(source_op, MirBinaryOp::Add | MirBinaryOp::Sub)
                        && source_op == emitted_op
                        && mapped_scalar(*a, &values) == (*c, false)
                        && mapped_scalar(*b, &values) == (*d, false)
                }
                (
                    KirInstructionKind::Compare {
                        op: a,
                        left: b,
                        right: c,
                    },
                    KirInstructionKind::Compare {
                        op: d,
                        left: e,
                        right: f,
                    },
                ) => {
                    a == d
                        && mapped_scalar(*b, &values) == (*e, false)
                        && mapped_scalar(*c, &values) == (*f, false)
                }
                _ => false,
            };
            if !same
                || emitted.memory.is_some()
                || emitted.effect.is_some()
                || source.results.len() != 1
            {
                return Err(error(
                    "affine scalar address setup or constant differs from source",
                ));
            }
            values.insert(
                source.results[0].value,
                (
                    checked_single_result(emitted, Some(&source.results[0].type_node))?,
                    false,
                ),
            );
            continue;
        }
        if let Some(address) = affine.addresses.get(&source.id) {
            let emitted = body
                .instructions
                .get(cursor)
                .ok_or_else(|| error("affine emitted memory instruction is missing"))?;
            cursor += 1;
            let (place, store) = match &source.kind {
                KirInstructionKind::Load { place } => (place.as_ref(), None),
                KirInstructionKind::Store { place, value } => (place.as_ref(), Some(*value)),
                _ => return Err(error("affine source memory kind differs")),
            };
            let crate::KirPlace::SliceIndex {
                slice,
                index,
                region,
                ..
            } = place
            else {
                return Err(error("affine source slice index is missing"));
            };
            let source_slice = *slice;
            let slice = checked_affine_invariant(
                original,
                candidate.preheader,
                candidate.header,
                candidate.body,
                source_slice,
            )
            .ok_or_else(|| error("affine slice root is missing"))?;
            let source_memory = source
                .memory
                .as_ref()
                .ok_or_else(|| error("affine source MemorySSA is missing"))?;
            let emitted_memory = emitted
                .memory
                .as_ref()
                .ok_or_else(|| error("affine emitted MemorySSA is missing"))?;
            if emitted_memory.region != source_memory.region
                || memories.get(&source_memory.input).copied() != Some(emitted_memory.input)
                || source_memory.output.is_some() != emitted_memory.output.is_some()
            {
                return Err(error(
                    "affine emitted memory chain differs from source order",
                ));
            }
            if let (Some(source), Some(emitted)) = (source_memory.output, emitted_memory.output) {
                memories.insert(source, emitted);
            }
            let expected_effect = if store.is_some() {
                crate::KirEffectKind::WriteMemory
            } else {
                crate::KirEffectKind::ReadMemory
            };
            let effect = emitted
                .effect
                .as_ref()
                .ok_or_else(|| error("affine emitted memory effect is missing"))?;
            if effect.kind != expected_effect
                || previous_effect.is_some_and(|previous| previous >= effect.order)
            {
                return Err(error("affine memory effects do not preserve scalar order"));
            }
            previous_effect = Some(effect.order);
            match address {
                CheckedWasmAddress::Broadcast(invariant_index) => {
                    let group = plan
                        .broadcast_groups
                        .iter()
                        .find(|group| group.scalar_instruction == source.id)
                        .ok_or_else(|| error("affine broadcast record is missing"))?;
                    if group.emitted_scalar_load != emitted.id
                        || !matches!(&emitted.kind, KirInstructionKind::Load { place } if matches!(place.as_ref(), crate::KirPlace::SliceIndex { slice: actual_slice, index: actual_index, type_node, region: actual_region } if (*actual_slice == slice || mapped_scalar(source_slice, &values) == (*actual_slice, false)) && (*actual_index == *invariant_index || mapped_scalar(*index, &values) == (*actual_index, false)) && *actual_region == *region && *type_node == MirType::Primitive(MirPrimitiveTypeName::F64)))
                    {
                        return Err(error("affine broadcast load address or identity differs"));
                    }
                    let scalar = checked_single_result(
                        emitted,
                        Some(&KirValueType::Scalar(MirType::Primitive(
                            MirPrimitiveTypeName::F64,
                        ))),
                    )?;
                    let splat = body
                        .instructions
                        .get(cursor)
                        .ok_or_else(|| error("affine broadcast splat is missing"))?;
                    cursor += 1;
                    if splat.id != group.emitted_splat
                        || splat.memory.is_some()
                        || splat.effect.is_some()
                        || !matches!(splat.kind, KirInstructionKind::VectorSplat { scalar: actual, .. } if actual == scalar)
                    {
                        return Err(error(
                            "affine broadcast splat is not adjacent to its own scalar load",
                        ));
                    }
                    values.insert(
                        checked_single_result(source, None)?,
                        (checked_single_result(splat, Some(&vector_type))?, true),
                    );
                }
                CheckedWasmAddress::Contiguous(offset) => {
                    let group = plan
                        .memory_groups
                        .iter()
                        .find(|group| group.scalar_instructions == [source.id])
                        .ok_or_else(|| error("affine vector memory record is missing"))?;
                    let (access, stored) = match &emitted.kind {
                        KirInstructionKind::VectorLoad { access, .. } => (access, None),
                        KirInstructionKind::VectorStore { access, value, .. } => {
                            (access, Some(*value))
                        }
                        _ => return Err(error("affine contiguous access is not vector memory")),
                    };
                    if group.vector_instruction != emitted.id {
                        return Err(error("affine vector memory instruction identity differs"));
                    }
                    let descriptor_matches = original.regions.iter().any(|descriptor| {
                        descriptor.partition == source_memory.region
                            && matches!(descriptor.origin,
                                crate::KirMemoryRegionOrigin::Parameter(value)
                                | crate::KirMemoryRegionOrigin::RawSlice(value)
                                | crate::KirMemoryRegionOrigin::Subslice(value)
                                if value == access.slice && forwards_from(original, source_slice, value))
                    });
                    if !descriptor_matches {
                        return Err(error(
                            "affine vector slice is not its proven descriptor-origin root",
                        ));
                    }
                    if mapped_scalar(*index, &values) != (access.start, false)
                        || access.lane != KirLaneType::F64
                        || access.lanes != 2
                        || access.byte_footprint != 16
                        || access.required_alignment != 8
                        || access.known_alignment != 8
                        || store.map(|value| mapped_scalar(value, &values))
                            != stored.map(|value| (value, true))
                    {
                        return Err(error(
                            "affine vector memory address, stored value, or footprint differs",
                        ));
                    }
                    if let Some(offset) = offset {
                        let endpoint = preheader
                            .instructions
                            .iter()
                            .find(|instruction| {
                                instruction
                                    .results
                                    .iter()
                                    .any(|result| result.value == access.end)
                            })
                            .ok_or_else(|| {
                                error("affine vector endpoint is not preheader-defined")
                            })?;
                        if !matches!(endpoint.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left, right, semantics: crate::KirArithmeticSemantics::Modular } if left == *offset && right == bound)
                            || checked_single_result(endpoint, Some(&scalar_u32))? != access.end
                        {
                            return Err(error(
                                "affine vector endpoint is not offset plus complete trip bound",
                            ));
                        }
                        endpoints.insert(endpoint.id);
                    } else if access.end != bound {
                        return Err(error("affine zero-offset endpoint differs from trip bound"));
                    }
                    if store.is_none() {
                        values.insert(
                            checked_single_result(source, None)?,
                            (checked_single_result(emitted, Some(&vector_type))?, true),
                        );
                    } else if !emitted.results.is_empty() {
                        return Err(error("affine vector store produces extra results"));
                    }
                }
            }
            continue;
        }
        let mapping = plan
            .operations
            .iter()
            .find(|mapping| mapping.scalar == source.id)
            .ok_or_else(|| error("affine source has an uncovered scalar operation"))?;
        if mapping.lane_type != KirLaneType::F64
            || mapping.semantics != KirCostSemantics::StrictFloat
            || source.memory.is_some()
            || source.effect.is_some()
        {
            return Err(error(
                "affine source operation is outside strict f64 arithmetic",
            ));
        }
        let operands = operation_inputs(source)
            .into_iter()
            .map(|operand| mapped_scalar(operand, &values))
            .collect::<Vec<_>>();
        let mut vector_operands = Vec::new();
        for (value, vector) in operands {
            if vector {
                vector_operands.push(value);
                continue;
            }
            let splat = if let Some(splat) = splats.get(&value) {
                *splat
            } else {
                let emitted = body
                    .instructions
                    .get(cursor)
                    .ok_or_else(|| error("affine arithmetic operand splat is missing"))?;
                cursor += 1;
                if emitted.memory.is_some()
                    || emitted.effect.is_some()
                    || !matches!(emitted.kind, KirInstructionKind::VectorSplat { scalar, .. } if scalar == value)
                {
                    return Err(error("affine arithmetic scalar operand was changed"));
                }
                let splat = checked_single_result(emitted, Some(&vector_type))?;
                splats.insert(value, splat);
                splat
            };
            vector_operands.push(splat);
        }
        let emitted = body
            .instructions
            .get(cursor)
            .ok_or_else(|| error("affine vector arithmetic operation is missing"))?;
        cursor += 1;
        let same = match (&source.kind, &emitted.kind) {
            (
                KirInstructionKind::Binary {
                    op,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    ..
                },
                KirInstructionKind::VectorBinary {
                    op: actual_op,
                    left,
                    right,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    no_failure_proof: None,
                    ..
                },
            ) => {
                let expected = match op {
                    MirBinaryOp::Add => Some(crate::KirVectorBinaryOp::Add),
                    MirBinaryOp::Sub => Some(crate::KirVectorBinaryOp::Subtract),
                    MirBinaryOp::Mul => Some(crate::KirVectorBinaryOp::Multiply),
                    MirBinaryOp::Div => Some(crate::KirVectorBinaryOp::Divide),
                    _ => None,
                };
                expected == Some(*actual_op) && vector_operands == [*left, *right]
            }
            (
                KirInstructionKind::Unary {
                    op: crate::MirUnaryOp::Neg,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    ..
                },
                KirInstructionKind::VectorUnary {
                    op: crate::KirVectorUnaryOp::Negate,
                    operand,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    no_failure_proof: None,
                    ..
                },
            ) => vector_operands == [*operand],
            _ => false,
        };
        if !same
            || mapping.vector != emitted.id
            || emitted.memory.is_some()
            || emitted.effect.is_some()
        {
            return Err(error(
                "affine strict arithmetic dataflow was changed or reassociated",
            ));
        }
        values.insert(
            checked_single_result(source, None)?,
            (checked_single_result(emitted, Some(&vector_type))?, true),
        );
    }
    if cursor != body.instructions.len() {
        return Err(error(
            "affine vector body contains unaccounted instructions",
        ));
    }
    if source_backedge
        .args
        .iter()
        .map(|value| mapped_scalar(*value, &values))
        .collect::<Vec<_>>()
        != backedge
            .args
            .iter()
            .map(|value| (*value, false))
            .collect::<Vec<_>>()
        || source_backedge
            .memory_args
            .iter()
            .map(|value| memories.get(value).copied())
            .collect::<Vec<_>>()
            != backedge
                .memory_args
                .iter()
                .map(|value| Some(*value))
                .collect::<Vec<_>>()
    {
        return Err(error(
            "affine vector backedge changed scalar or memory state",
        ));
    }
    for instruction in &preheader.instructions[preheader_before.instructions.len()..] {
        let permitted = match &instruction.kind {
            KirInstructionKind::ConstInt { value } => value.parse::<u32>().is_ok_and(|value| {
                [0, 1, 2, candidate.minimum_trip].contains(&value)
                    || value == 3
                        && affine.ranges.iter().any(|r| {
                            matches!(
                                r.count,
                                crate::WasmRangeCount::ScaledInvariant { scale: 3, .. }
                            )
                        })
            }),
            KirInstructionKind::VersionPredicate { .. } => true,
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                ..
            } => endpoints.contains(&instruction.id),
            KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                left,
                right,
                semantics: crate::KirArithmeticSemantics::Modular,
            } => *left == bound && integer_constant(transformed, *right) == Some(2),
            KirInstructionKind::Binary {
                op: MirBinaryOp::Mul,
                left,
                right,
                semantics: crate::KirArithmeticSemantics::Modular,
            } => {
                affine.ranges.iter().any(|r| {
                    matches!(r.count,crate::WasmRangeCount::ScaledInvariant{value,scale:3}
                    if value==*left&&integer_constant(transformed,*right)==Some(3))
                }) && instruction.results.len() == 1
                    && instruction.results[0].type_node == scalar_u32
            }
            KirInstructionKind::SliceLen { .. } => instruction
                .results
                .iter()
                .any(|result| result.value == bound),
            _ => false,
        };
        if !permitted || instruction.memory.is_some() || instruction.effect.is_some() {
            return Err(error(
                "affine preheader contains unaccounted computation or effects",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_wasm_direct_map_interleaved_emission(
    original: &crate::KirFunction,
    transformed: &crate::KirFunction,
    candidate: &CheckedVectorSource,
    affine: &CheckedWasmAffine,
    source_header: &crate::KirBlock,
    source_body: &crate::KirBlock,
    vector_header: &crate::KirBlock,
    vector_body: &crate::KirBlock,
    epilogue: &crate::KirEdge,
    preheader_before: &crate::KirBlock,
    preheader: &crate::KirBlock,
    entry_condition: ValueId,
    bound: ValueId,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let error = TransactionCheckError::compiler;
    let scalar_u32 = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32));
    let vector_f64 = KirValueType::FixedVector {
        lane: KirLaneType::F64,
        lanes: 2,
    };
    let direct_map = checked_wasm_direct_map_interleave(
        candidate.bound,
        plan,
        affine,
        &candidate.operations,
        &candidate.accesses,
    );
    let matmul = checked_wasm_matmul_interleave(CheckedMatmulInterleaveSource {
        function: original,
        header_id: candidate.header,
        body_id: candidate.body,
        induction: candidate.induction,
        scalar_blocks: &candidate.scalar_blocks,
        has_diamond: candidate.diamond.is_some(),
        has_reduction: candidate.reduction.is_some(),
        bound: candidate.bound,
        plan,
        affine,
        operations: &candidate.operations,
        accesses: &candidate.accesses,
    });
    if !direct_map && !matmul {
        return Err(error(
            "WASM interleave source is neither a strict direct map nor the closed matmul accumulator map",
        ));
    }
    if transformed.blocks.len() != original.blocks.len() + 2
        || vector_header.params.len() != source_header.params.len()
        || !vector_body.params.is_empty()
        || vector_header.memory_params.len() != source_header.memory_params.len()
        || !vector_body.memory_params.is_empty()
        || source_header
            .instructions
            .iter()
            .any(|instruction| instruction.memory.is_some() || instruction.effect.is_some())
    {
        return Err(error(
            "WASM interleave block or parameter partition differs",
        ));
    }
    if epilogue.args
        != vector_header
            .params
            .iter()
            .map(|param| param.value)
            .collect::<Vec<_>>()
        || epilogue.memory_args
            != vector_header
                .memory_params
                .iter()
                .map(|param| param.version)
                .collect::<Vec<_>>()
    {
        return Err(error("WASM interleave scalar tail changed carried state"));
    }
    // A trial may add only pure setup for the exact direct-map or matmul range
    // endpoints. Checking the guard result alone would miss unused arithmetic.
    let chunk_width = u32::from(candidate.vf) * u32::from(candidate.uf);
    let affine_offsets = affine
        .addresses
        .values()
        .filter_map(|address| match address {
            CheckedWasmAddress::Contiguous(Some(offset)) => Some(*offset),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut endpoint_ids = BTreeSet::new();
    let mut endpoint_values = BTreeMap::new();
    for offset in &affine_offsets {
        let endpoints = preheader.instructions[preheader_before.instructions.len()..]
            .iter()
            .filter_map(|instruction| {
                let KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left,
                    right,
                    semantics: crate::KirArithmeticSemantics::Modular,
                } = instruction.kind
                else {
                    return None;
                };
                (left == *offset
                    && right == bound
                    && instruction.memory.is_none()
                    && instruction.effect.is_none())
                .then(|| {
                    checked_single_result(instruction, Some(&scalar_u32))
                        .map(|value| (instruction.id, value))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if endpoints.len() != 1 {
            return Err(error(
                "WASM interleave affine end is not one exact offset+bound",
            ));
        }
        let (instruction, value) = endpoints[0];
        endpoint_ids.insert(instruction);
        endpoint_values.insert(Some(*offset), value);
    }
    endpoint_values.insert(None, bound);
    for instruction in &preheader.instructions[preheader_before.instructions.len()..] {
        if instruction.memory.is_some() || instruction.effect.is_some() {
            return Err(error("WASM interleave preheader gained effects"));
        }
        let permitted = match &instruction.kind {
            KirInstructionKind::ConstInt { value } => {
                matches!(instruction.results.as_slice(), [result]
                    if result.type_node == scalar_u32
                        && value.parse::<u32>().is_ok_and(|value|
                            [0, 1, 2, chunk_width, candidate.minimum_trip].contains(&value)))
            }
            KirInstructionKind::SliceLen { .. } => {
                matches!(instruction.results.as_slice(), [result]
                    if result.value == bound && result.type_node == scalar_u32)
            }
            KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                left,
                right,
                semantics: crate::KirArithmeticSemantics::Modular,
            } => {
                *left == bound
                    && integer_constant(transformed, *right) == Some(i128::from(chunk_width))
                    && matches!(instruction.results.as_slice(), [result]
                        if result.type_node == scalar_u32)
            }
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                ..
            } => endpoint_ids.contains(&instruction.id),
            KirInstructionKind::VersionPredicate { .. } => {
                matches!(instruction.results.as_slice(), [result]
                    if result.value == entry_condition
                        && result.type_node
                            == KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::Bool)))
            }
            _ => false,
        };
        if !permitted {
            return Err(error(
                "WASM interleave preheader contains unaccounted computation",
            ));
        }
    }
    let KirTerminator::Branch {
        then_edge: source_body_edge,
        ..
    } = &source_header.terminator
    else {
        return Err(error("WASM interleave source header is not a branch"));
    };
    let KirTerminator::Jump {
        edge: source_backedge,
    } = &source_body.terminator
    else {
        return Err(error("WASM interleave source backedge is missing"));
    };
    let KirTerminator::Jump {
        edge: vector_backedge,
    } = &vector_body.terminator
    else {
        return Err(error("WASM interleave vector backedge is missing"));
    };
    let induction_index = source_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .ok_or_else(|| error("WASM interleave induction parameter is missing"))?;
    let body_induction_index =
        original_header_body_induction_index(original, candidate, source_body)
            .map_err(TransactionCheckError::compiler)?;
    let mut values = BTreeMap::<ValueId, (ValueId, bool)>::new();
    for (source, emitted) in source_header.params.iter().zip(&vector_header.params) {
        if source.type_node != emitted.type_node {
            return Err(error("WASM interleave header parameter type differs"));
        }
        values.insert(source.value, (emitted.value, false));
    }
    for (index, source) in source_body.params.iter().enumerate() {
        let argument = *source_body_edge
            .args
            .get(index)
            .ok_or_else(|| error("WASM interleave source body edge is incomplete"))?;
        let mapped = values.get(&argument).copied().unwrap_or((argument, false));
        values.insert(source.value, mapped);
    }
    let mut memories = BTreeMap::new();
    for (source, emitted) in source_header
        .memory_params
        .iter()
        .zip(&vector_header.memory_params)
    {
        if source.region != emitted.region {
            return Err(error("WASM interleave header MemorySSA region differs"));
        }
        memories.insert(source.version, emitted.version);
    }
    for (param, incoming) in source_body
        .memory_params
        .iter()
        .zip(&source_body_edge.memory_args)
    {
        let mapped = memories
            .get(incoming)
            .copied()
            .ok_or_else(|| error("WASM interleave body memory is not a header value"))?;
        memories.insert(param.version, mapped);
    }
    let mut cursor = 0_usize;
    let mut shared_broadcasts = BTreeMap::<InstructionId, ValueId>::new();
    let mut current_chunk = values
        .get(&candidate.induction)
        .map(|(value, vector)| {
            if *vector {
                Err(error(
                    "WASM interleave induction unexpectedly became vector",
                ))
            } else {
                Ok(*value)
            }
        })
        .transpose()?
        .ok_or_else(|| error("WASM interleave induction mapping is missing"))?;
    let mut stride = None;
    let mut splats = BTreeMap::<ValueId, ValueId>::new();
    let mut previous_effect = None;
    let mut final_values = values.clone();

    for unroll_index in 0..candidate.uf {
        if unroll_index != 0 {
            if stride.is_none() {
                let step = vector_body
                    .instructions
                    .get(cursor)
                    .ok_or_else(|| error("WASM interleave chunk stride is missing"))?;
                cursor += 1;
                if step.memory.is_some()
                    || step.effect.is_some()
                    || !matches!(&step.kind, KirInstructionKind::ConstInt { value } if value == "2")
                {
                    return Err(error(
                        "WASM interleave chunk stride is not exactly two lanes",
                    ));
                }
                stride = Some(checked_single_result(step, Some(&scalar_u32))?);
            }
            let next = vector_body
                .instructions
                .get(cursor)
                .ok_or_else(|| error("WASM interleave chunk offset is missing"))?;
            cursor += 1;
            let result = checked_single_result(next, Some(&scalar_u32))?;
            if next.memory.is_some()
                || next.effect.is_some()
                || !matches!(next.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left, right, semantics: crate::KirArithmeticSemantics::Modular } if left == current_chunk && Some(right) == stride)
            {
                return Err(error("WASM interleave chunk offset changed"));
            }
            current_chunk = result;
        }
        let mut chunk_values = values.clone();
        let body_induction = source_body
            .params
            .get(body_induction_index)
            .ok_or_else(|| error("WASM interleave body induction is missing"))?
            .value;
        chunk_values.insert(candidate.induction, (current_chunk, false));
        chunk_values.insert(body_induction, (current_chunk, false));
        let source_update_value = source_backedge
            .args
            .get(induction_index)
            .copied()
            .ok_or_else(|| error("WASM interleave source induction update is missing"))?;
        let update = original
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == source_update_value)
            })
            .ok_or_else(|| error("WASM interleave induction update definition is missing"))?;
        let update_step = match update.kind {
            KirInstructionKind::Binary { right, .. } => right,
            _ => {
                return Err(error(
                    "WASM interleave source induction update is malformed",
                ));
            }
        };
        let mut chunk_effect_order = previous_effect;

        for source in &source_body.instructions {
            if source.id == update.id {
                if unroll_index + 1 != candidate.uf {
                    continue;
                }
                let step =
                    stride.ok_or_else(|| error("WASM interleave stride was not materialized"))?;
                let emitted = vector_body
                    .instructions
                    .get(cursor)
                    .ok_or_else(|| error("WASM interleave induction update is missing"))?;
                cursor += 1;
                let result = checked_single_result(emitted, Some(&scalar_u32))?;
                if emitted.memory.is_some()
                    || emitted.effect.is_some()
                    || !matches!(emitted.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left, right, semantics: crate::KirArithmeticSemantics::Modular } if left == current_chunk && right == step)
                {
                    return Err(error("WASM interleave loop-carried chunk update differs"));
                }
                chunk_values.insert(source_update_value, (result, false));
                continue;
            }
            if affine.setup.contains(&source.id) {
                let KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left,
                    right,
                    semantics: crate::KirArithmeticSemantics::Modular,
                } = &source.kind
                else {
                    return Err(error(
                        "WASM interleave affine setup is not a modular source addition",
                    ));
                };
                let emitted = vector_body
                    .instructions
                    .get(cursor)
                    .ok_or_else(|| error("WASM interleave affine setup is missing"))?;
                cursor += 1;
                let mapped_left = chunk_values.get(left).copied().unwrap_or((*left, false));
                let mapped_right = chunk_values.get(right).copied().unwrap_or((*right, false));
                if mapped_left.1
                    || mapped_right.1
                    || emitted.memory.is_some()
                    || emitted.effect.is_some()
                    || source.results.len() != 1
                    || !matches!(&emitted.kind, KirInstructionKind::Binary { op: MirBinaryOp::Add, left: actual_left, right: actual_right, semantics: crate::KirArithmeticSemantics::Modular } if *actual_left == mapped_left.0 && *actual_right == mapped_right.0)
                {
                    return Err(error(
                        "WASM interleave affine setup operands or effects differ",
                    ));
                }
                chunk_values.insert(
                    source.results[0].value,
                    (
                        checked_single_result(emitted, Some(&source.results[0].type_node))?,
                        false,
                    ),
                );
                continue;
            }
            match &source.kind {
                KirInstructionKind::Copy { value } => {
                    let result = checked_single_result(source, None)?;
                    chunk_values.insert(
                        result,
                        chunk_values.get(value).copied().unwrap_or((*value, false)),
                    );
                }
                KirInstructionKind::ConstInt { value } => {
                    let result = checked_single_result(source, None)?;
                    if result == update_step {
                        continue;
                    }
                    let emitted = vector_body.instructions.get(cursor).ok_or_else(|| {
                        error("WASM interleave scalar integer constant is missing")
                    })?;
                    cursor += 1;
                    if emitted.memory.is_some()
                        || emitted.effect.is_some()
                        || !matches!(&emitted.kind, KirInstructionKind::ConstInt { value: actual } if actual == value)
                    {
                        return Err(error("WASM interleave scalar integer constant differs"));
                    }
                    chunk_values.insert(result, (checked_single_result(emitted, None)?, false));
                }
                KirInstructionKind::ConstFloat { value } => {
                    let result = checked_single_result(source, None)?;
                    let emitted = vector_body
                        .instructions
                        .get(cursor)
                        .ok_or_else(|| error("WASM interleave scalar float constant is missing"))?;
                    cursor += 1;
                    if emitted.memory.is_some()
                        || emitted.effect.is_some()
                        || !matches!(&emitted.kind, KirInstructionKind::ConstFloat { value: actual } if actual == value)
                    {
                        return Err(error("WASM interleave scalar float constant differs"));
                    }
                    chunk_values.insert(result, (checked_single_result(emitted, None)?, false));
                }
                KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
                    let write = matches!(source.kind, KirInstructionKind::Store { .. });
                    let crate::KirPlace::SliceIndex {
                        slice: source_slice,
                        index: source_index,
                        region,
                        type_node,
                    } = place.as_ref()
                    else {
                        return Err(error("WASM interleave source is not a slice access"));
                    };
                    let source_memory = source
                        .memory
                        .as_ref()
                        .ok_or_else(|| error("WASM interleave source MemorySSA is missing"))?;
                    let source_partition = original
                        .regions
                        .iter()
                        .find(|descriptor| descriptor.id == *region)
                        .map(|descriptor| descriptor.partition);
                    if source_partition != Some(source_memory.region)
                        || !original.regions.iter().any(|descriptor| {
                            descriptor.id == source_memory.region
                                && descriptor.partition == source_memory.region
                        })
                        || *type_node != MirType::Primitive(MirPrimitiveTypeName::F64)
                    {
                        return Err(error(
                            "WASM interleave source type or MemorySSA region differs",
                        ));
                    }
                    if let Some(CheckedWasmAddress::Broadcast(_)) = affine.addresses.get(&source.id)
                    {
                        if write {
                            return Err(error("WASM interleave broadcast unexpectedly writes"));
                        }
                        let source_result = checked_single_result(source, None)?;
                        if matmul && unroll_index != 0 {
                            let splat =
                                shared_broadcasts.get(&source.id).copied().ok_or_else(|| {
                                    error("WASM interleave shared matmul broadcast is missing")
                                })?;
                            chunk_values.insert(source_result, (splat, true));
                            continue;
                        }
                        let broadcast_unroll_index = if matmul { 0 } else { unroll_index };
                        let group = plan
                            .broadcast_groups
                            .iter()
                            .find(|group| {
                                group.scalar_instruction == source.id
                                    && group.unroll_index == broadcast_unroll_index
                            })
                            .ok_or_else(|| error("WASM interleave broadcast mapping is missing"))?;
                        let emitted_load =
                            vector_body.instructions.get(cursor).ok_or_else(|| {
                                error("WASM interleave scalar broadcast load is missing")
                            })?;
                        cursor += 1;
                        let emitted_splat = vector_body
                            .instructions
                            .get(cursor)
                            .ok_or_else(|| error("WASM interleave broadcast splat is missing"))?;
                        cursor += 1;
                        let mapped_slice = chunk_values
                            .get(source_slice)
                            .copied()
                            .unwrap_or((*source_slice, false));
                        let mapped_index = chunk_values
                            .get(source_index)
                            .copied()
                            .unwrap_or((*source_index, false));
                        let KirInstructionKind::Load {
                            place: emitted_place,
                        } = &emitted_load.kind
                        else {
                            return Err(error(
                                "WASM interleave broadcast scalar load kind differs",
                            ));
                        };
                        let crate::KirPlace::SliceIndex {
                            slice: emitted_slice,
                            index: emitted_index,
                            region: emitted_region,
                            type_node: emitted_type,
                        } = emitted_place.as_ref()
                        else {
                            return Err(error(
                                "WASM interleave broadcast load is not a slice access",
                            ));
                        };
                        let emitted_memory = emitted_load.memory.as_ref().ok_or_else(|| {
                            error("WASM interleave broadcast MemorySSA is missing")
                        })?;
                        let effect = emitted_load
                            .effect
                            .as_ref()
                            .ok_or_else(|| error("WASM interleave broadcast effect is missing"))?;
                        let result = checked_single_result(
                            emitted_load,
                            Some(&KirValueType::Scalar(MirType::Primitive(
                                MirPrimitiveTypeName::F64,
                            ))),
                        )?;
                        let expected_vector_region = transformed
                            .vector_regions
                            .iter()
                            .find(|region| region.blocks.contains(&vector_body.id))
                            .map(|region| region.id)
                            .ok_or_else(|| error("WASM interleave vector region is missing"))?;
                        let splat_result = checked_single_result(emitted_splat, Some(&vector_f64))?;
                        if group.region != *region
                            || group.emitted_scalar_load != emitted_load.id
                            || group.emitted_splat != emitted_splat.id
                            || mapped_slice.1
                            || mapped_index.1
                            || *emitted_slice != mapped_slice.0
                            || *emitted_index != mapped_index.0
                            || *emitted_region != *region
                            || emitted_type != type_node
                            || emitted_memory.region != source_memory.region
                            || memories.get(&source_memory.input).copied()
                                != Some(emitted_memory.input)
                            || source_memory.output.is_some() != emitted_memory.output.is_some()
                            || effect.kind != crate::KirEffectKind::ReadMemory
                            || chunk_effect_order.is_some_and(|previous| previous >= effect.order)
                            || !matches!(
                                emitted_splat.kind,
                                KirInstructionKind::VectorSplat { scalar, region }
                                    if scalar == result && region == expected_vector_region
                            )
                            || emitted_splat.memory.is_some()
                            || emitted_splat.effect.is_some()
                        {
                            return Err(error(
                                "WASM interleave broadcast source, effect or splat differs",
                            ));
                        }
                        chunk_effect_order = Some(effect.order);
                        if matmul {
                            shared_broadcasts.insert(source.id, splat_result);
                        }
                        chunk_values.insert(source_result, (splat_result, true));
                        continue;
                    }
                    let group = plan
                        .memory_groups
                        .iter()
                        .find(|group| {
                            group.scalar_instructions == [source.id]
                                && group.unroll_index == unroll_index
                        })
                        .ok_or_else(|| error("WASM interleave memory mapping is missing"))?;
                    let emitted = vector_body.instructions.get(cursor).ok_or_else(|| {
                        error("WASM interleave vector memory operation is missing")
                    })?;
                    cursor += 1;
                    if emitted.id != group.vector_instruction {
                        return Err(error("WASM interleave memory identity differs"));
                    }
                    let source_root = checked_affine_invariant(
                        original,
                        candidate.preheader,
                        candidate.header,
                        candidate.body,
                        *source_slice,
                    )
                    .ok_or_else(|| error("WASM interleave source slice root is missing"))?;
                    let (access, stored) = match &emitted.kind {
                        KirInstructionKind::VectorLoad { access, .. } if !write => (access, None),
                        KirInstructionKind::VectorStore { access, value, .. } if write => {
                            (access, Some(*value))
                        }
                        _ => return Err(error("WASM interleave vector memory kind differs")),
                    };
                    let source_memory = source
                        .memory
                        .as_ref()
                        .ok_or_else(|| error("WASM interleave source MemorySSA is missing"))?;
                    let emitted_memory = emitted
                        .memory
                        .as_ref()
                        .ok_or_else(|| error("WASM interleave emitted MemorySSA is missing"))?;
                    let address = affine
                        .addresses
                        .get(&source.id)
                        .ok_or_else(|| error("WASM interleave source address is missing"))?;
                    let expected_end = match address {
                        CheckedWasmAddress::Contiguous(offset) => endpoint_values
                            .get(offset)
                            .copied()
                            .ok_or_else(|| error("WASM interleave source endpoint is missing"))?,
                        CheckedWasmAddress::Broadcast(_) => {
                            return Err(error("WASM interleave broadcast emitted vector memory"));
                        }
                    };
                    let mapped_start = chunk_values
                        .get(source_index)
                        .copied()
                        .unwrap_or((*source_index, false));
                    if emitted_memory.region != source_memory.region
                        || memories.get(&source_memory.input).copied() != Some(emitted_memory.input)
                        || source_memory.output.is_some() != emitted_memory.output.is_some()
                        || group.region != *region
                        || (access.slice != source_root
                            && !forwards_from(original, source_root, access.slice)
                            && !forwards_from(original, access.slice, source_root))
                        || mapped_start.1
                        || access.start != mapped_start.0
                        || access.end != expected_end
                        || access.lane != KirLaneType::F64
                        || access.lanes != 2
                        || access.byte_footprint != 16
                        || access.required_alignment != 8
                        || access.known_alignment != 8
                    {
                        return Err(error(
                            "WASM interleave vector memory range or chain differs",
                        ));
                    }
                    let expected_effect = if write {
                        crate::KirEffectKind::WriteMemory
                    } else {
                        crate::KirEffectKind::ReadMemory
                    };
                    let effect = emitted
                        .effect
                        .as_ref()
                        .ok_or_else(|| error("WASM interleave ordered memory effect is missing"))?;
                    if effect.kind != expected_effect
                        || chunk_effect_order.is_some_and(|previous| previous >= effect.order)
                    {
                        return Err(error("WASM interleave memory effect order changed"));
                    }
                    chunk_effect_order = Some(effect.order);
                    if let (Some(source_output), Some(emitted_output)) =
                        (source_memory.output, emitted_memory.output)
                    {
                        memories.insert(source_output, emitted_output);
                    }
                    if let KirInstructionKind::Load { .. } = source.kind {
                        chunk_values.insert(
                            checked_single_result(source, None)?,
                            (checked_single_result(emitted, Some(&vector_f64))?, true),
                        );
                    } else if let KirInstructionKind::Store { value, .. } = source.kind
                        && chunk_values.get(&value).copied() != stored.map(|value| (value, true))
                    {
                        return Err(error("WASM interleave stored vector value differs"));
                    }
                }
                KirInstructionKind::Binary {
                    op,
                    left,
                    right,
                    semantics,
                } => {
                    if *semantics != crate::KirArithmeticSemantics::StrictFloat {
                        return Err(error("WASM interleave changed strict scalar arithmetic"));
                    }
                    let mapping = plan
                        .operations
                        .iter()
                        .find(|mapping| {
                            mapping.scalar == source.id && mapping.unroll_index == unroll_index
                        })
                        .ok_or_else(|| error("WASM interleave operation mapping is missing"))?;
                    let mut operands = Vec::new();
                    for scalar in [*left, *right] {
                        let mapped = chunk_values
                            .get(&scalar)
                            .copied()
                            .unwrap_or((scalar, false));
                        if mapped.1 {
                            operands.push(mapped.0);
                        } else if let Some(splat) = splats.get(&mapped.0) {
                            operands.push(*splat);
                        } else {
                            let emitted_splat = vector_body
                                .instructions
                                .get(cursor)
                                .ok_or_else(|| error("WASM interleave scalar splat is missing"))?;
                            cursor += 1;
                            let result = checked_single_result(emitted_splat, Some(&vector_f64))?;
                            if emitted_splat.memory.is_some()
                                || emitted_splat.effect.is_some()
                                || !matches!(emitted_splat.kind, KirInstructionKind::VectorSplat { scalar: actual, .. } if actual == mapped.0)
                            {
                                return Err(error("WASM interleave scalar splat source differs"));
                            }
                            splats.insert(mapped.0, result);
                            operands.push(result);
                        }
                    }
                    let emitted = vector_body
                        .instructions
                        .get(cursor)
                        .ok_or_else(|| error("WASM interleave vector arithmetic is missing"))?;
                    cursor += 1;
                    let expected_op = match op {
                        MirBinaryOp::Add => crate::KirVectorBinaryOp::Add,
                        MirBinaryOp::Sub => crate::KirVectorBinaryOp::Subtract,
                        MirBinaryOp::Mul => crate::KirVectorBinaryOp::Multiply,
                        MirBinaryOp::Div => crate::KirVectorBinaryOp::Divide,
                        _ => return Err(error("WASM interleave scalar operator is unsupported")),
                    };
                    if emitted.id != mapping.vector
                        || emitted.memory.is_some()
                        || emitted.effect.is_some()
                        || !matches!(emitted.kind, KirInstructionKind::VectorBinary { op, left, right, semantics: crate::KirArithmeticSemantics::StrictFloat, no_failure_proof: None, .. } if op == expected_op && [left, right] == operands.as_slice())
                    {
                        return Err(error(
                            "WASM interleave strict arithmetic order or operands differ",
                        ));
                    }
                    chunk_values.insert(
                        checked_single_result(source, None)?,
                        (checked_single_result(emitted, Some(&vector_f64))?, true),
                    );
                }
                KirInstructionKind::Unary {
                    op: crate::MirUnaryOp::Neg,
                    operand,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                } => {
                    let mapping = plan
                        .operations
                        .iter()
                        .find(|mapping| {
                            mapping.scalar == source.id && mapping.unroll_index == unroll_index
                        })
                        .ok_or_else(|| error("WASM interleave unary mapping is missing"))?;
                    let mapped = chunk_values
                        .get(operand)
                        .copied()
                        .unwrap_or((*operand, false));
                    let operand = if mapped.1 {
                        mapped.0
                    } else if let Some(splat) = splats.get(&mapped.0) {
                        *splat
                    } else {
                        let splat = vector_body
                            .instructions
                            .get(cursor)
                            .ok_or_else(|| error("WASM interleave unary splat is missing"))?;
                        cursor += 1;
                        let result = checked_single_result(splat, Some(&vector_f64))?;
                        if !matches!(splat.kind, KirInstructionKind::VectorSplat { scalar, .. } if scalar == mapped.0)
                        {
                            return Err(error("WASM interleave unary splat source differs"));
                        }
                        splats.insert(mapped.0, result);
                        result
                    };
                    let emitted = vector_body
                        .instructions
                        .get(cursor)
                        .ok_or_else(|| error("WASM interleave vector negation is missing"))?;
                    cursor += 1;
                    if emitted.id != mapping.vector
                        || emitted.memory.is_some()
                        || emitted.effect.is_some()
                        || !matches!(emitted.kind, KirInstructionKind::VectorUnary { op: crate::KirVectorUnaryOp::Negate, operand: actual, semantics: crate::KirArithmeticSemantics::StrictFloat, no_failure_proof: None, .. } if actual == operand)
                    {
                        return Err(error("WASM interleave strict negation differs"));
                    }
                    chunk_values.insert(
                        checked_single_result(source, None)?,
                        (checked_single_result(emitted, Some(&vector_f64))?, true),
                    );
                }
                _ => {
                    return Err(error(
                        "WASM interleave body contains unsupported source shape",
                    ));
                }
            }
        }
        previous_effect = chunk_effect_order;
        for (body_param, header_memory) in source_body
            .memory_params
            .iter()
            .zip(&source_body_edge.memory_args)
        {
            let header_index = source_header
                .memory_params
                .iter()
                .position(|param| param.version == *header_memory)
                .ok_or_else(|| error("WASM interleave memory header identity is missing"))?;
            let latch_memory = *source_backedge
                .memory_args
                .get(header_index)
                .ok_or_else(|| error("WASM interleave memory latch is missing"))?;
            let carried = memories
                .get(&latch_memory)
                .copied()
                .ok_or_else(|| error("WASM interleave memory recurrence is unmapped"))?;
            memories.insert(body_param.version, carried);
            memories.insert(*header_memory, carried);
        }
        final_values = chunk_values;
    }
    if cursor != vector_body.instructions.len() {
        return Err(error(
            "WASM interleave body has unaccounted emitted instructions",
        ));
    }
    let expected_backedge = source_backedge
        .args
        .iter()
        .map(|value| {
            final_values
                .get(value)
                .map_or(*value, |(mapped, _)| *mapped)
        })
        .collect::<Vec<_>>();
    if expected_backedge != vector_backedge.args {
        return Err(error("WASM interleave scalar backedge state differs"));
    }
    let expected_memory = source_backedge
        .memory_args
        .iter()
        .map(|value| memories.get(value).copied())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| error("WASM interleave memory backedge mapping is incomplete"))?;
    if expected_memory != vector_backedge.memory_args {
        return Err(error("WASM interleave MemorySSA backedge differs"));
    }
    Ok(())
}

fn checked_single_result(
    instruction: &KirInstruction,
    expected: Option<&KirValueType>,
) -> Result<ValueId, TransactionCheckError> {
    match instruction.results.as_slice() {
        [result] if expected.is_none_or(|expected| *expected == result.type_node) => {
            Ok(result.value)
        }
        _ => Err(TransactionCheckError::compiler(
            "affine instruction result arity or type is false",
        )),
    }
}

#[cfg(test)]
mod matmul_source_dataflow_tests {
    use super::checked_matmul_source_dataflow_is_closed;

    const NORMAL_MATMUL: &str = r#"
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

    const CARRIED_MATMUL: &str = r#"
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
      let carried: f64 = 0.0;
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        let previous: f64 = out[out_index];
        let scalar: f64 = a[a_index];
        let varying: f64 = b[b_index];
        out[out_index] = carried + scalar * varying;
        carried = previous;
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

    const MATMUL_WITH_NONINDUCTION_BACKEDGE: &str = r#"
export unsafe fn matmul_column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> f64
contract {
  requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let carried: f64 = 0.0;
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
        carried = previous;
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
  return carried;
}
"#;

    fn matmul_source_is_closed(source: &str) -> bool {
        let checked = crate::check(&crate::SourceFile::new("matmul-source-check.ck", source));
        assert_eq!(checked.diagnostics, []);
        let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR lowering");
        let profile =
            crate::KirTargetProfile::webassembly_with_features(crate::KirWasmFeatures::Simd128);
        let module = crate::build_kir_module_with_profile(
            &mir,
            crate::KirBuildConfig {
                consumer: profile.consumer(),
                overflow_mode: crate::KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
            profile,
        )
        .expect("WASM KIR");
        let optimized = crate::run_kir_pass_pipeline(module, crate::KirOptimizationLevel::O3, None);
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        let module = optimized.artifact.expect("optimized KIR");
        let function = module
            .functions
            .iter()
            .find(|function| function.name == "matmul_column")
            .expect("matmul function");
        let descriptor = super::super::analysis::analyze_canonical_loops_for_discovery(function)
            .loops
            .into_iter()
            .find(|descriptor| descriptor.innermost && descriptor.induction.is_some())
            .expect("innermost column loop");
        let induction = descriptor.induction.as_ref().expect("loop induction").value;
        let header = function
            .blocks
            .iter()
            .find(|block| block.id == descriptor.header)
            .expect("loop header");
        let crate::KirTerminator::Branch { then_edge, .. } = &header.terminator else {
            panic!("column loop header");
        };
        let body = function
            .blocks
            .iter()
            .find(|block| block.id == then_edge.target)
            .expect("column loop body");
        let loads = body
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(instruction.kind, crate::KirInstructionKind::Load { .. })
            })
            .map(|instruction| instruction.id)
            .collect::<Vec<_>>();
        let [output_read, a_read, b_read] = loads.as_slice() else {
            panic!("expected output, A and B reads in the column body: {loads:?}");
        };
        let stores = body
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(instruction.kind, crate::KirInstructionKind::Store { .. })
            })
            .map(|instruction| instruction.id)
            .collect::<Vec<_>>();
        let [output_write] = stores.as_slice() else {
            panic!("expected one output store");
        };
        let arithmetic = body
            .instructions
            .iter()
            .filter_map(|instruction| match instruction.kind {
                crate::KirInstructionKind::Binary {
                    op,
                    semantics: crate::KirArithmeticSemantics::StrictFloat,
                    ..
                } if matches!(op, crate::MirBinaryOp::Mul | crate::MirBinaryOp::Add) => {
                    Some((instruction.id, op))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [
            (multiply, crate::MirBinaryOp::Mul),
            (add, crate::MirBinaryOp::Add),
        ] = arithmetic.as_slice()
        else {
            panic!("expected strict multiply and add: {arithmetic:?}");
        };
        checked_matmul_source_dataflow_is_closed(
            function,
            descriptor.header,
            body.id,
            induction,
            super::CheckedMatmulSourceInstructionIds {
                broadcast: *a_read,
                output_read: *output_read,
                b_read: *b_read,
                output_write: *output_write,
                multiply: *multiply,
                add: *add,
            },
        )
    }

    #[test]
    fn source_dataflow_proof_accepts_the_closed_strict_matmul_pattern() {
        assert!(matmul_source_is_closed(NORMAL_MATMUL));
    }

    #[test]
    fn source_dataflow_proof_rejects_a_carried_output_recurrence() {
        assert!(!matmul_source_is_closed(CARRIED_MATMUL));
    }

    #[test]
    fn source_dataflow_proof_rejects_noninduction_backedge_state() {
        // This source keeps the exact output-old + A*B arithmetic dataflow;
        // only an additional loop-carried scalar changes across columns.
        assert!(!matmul_source_is_closed(MATMUL_WITH_NONINDUCTION_BACKEDGE));
    }
}
