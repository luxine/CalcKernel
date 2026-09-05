use std::collections::BTreeSet;

use crate::{
    BlockId, CandidateBudgetCharge, FunctionId, InstructionId, KirAlignmentClass, KirCostEstimate,
    KirCostKey, KirCostSemantics, KirInstruction, KirInstructionKind, KirLaneType,
    KirOperationAvailability, KirProfileOperation, KirTargetIdentity, KirTerminator, LoopId,
    MemoryRegionId, MirBinaryOp, MirPrimitiveTypeName, MirType, TransactionCheckError, ValueId,
    VectorEpilogue, VectorMemoryAccessKind, VectorizationPlan, compute_kir_dominators,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckedMemoryAccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone)]
struct CheckedVersionPredicate {
    conjuncts: Vec<CheckedVersionConjunct>,
}

#[derive(Debug, Clone)]
enum CheckedVersionConjunct {
    AddressIntervalsDisjoint { left: ValueId, right: ValueId },
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
    if plan.cost != candidate.predicted_cost
        || plan.operations.len()
            != candidate
                .operations
                .len()
                .saturating_mul(usize::from(candidate.uf))
        || plan.memory_groups.len()
            != candidate
                .accesses
                .len()
                .saturating_mul(usize::from(candidate.uf))
    {
        return compiler("vector operation, memory, or cost closure is false");
    }
    let transformed = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| TransactionCheckError::compiler("vector trial function is missing"))?;
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
    if preheader_after
        .instructions
        .get(..preheader_before.instructions.len())
        != Some(preheader_before.instructions.as_slice())
        || !(preheader_before.instructions.len() + 4..=preheader_before.instructions.len() + 6)
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
        then_edge: vector_entry,
        else_edge: scalar_entry,
        ..
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
    if vector_entry.args != original_entry.args
        || vector_entry.memory_args != original_entry.memory_args
    {
        return compiler("vector entry does not preserve the original loop state");
    }
    if epilogue_edge.target != candidate.header {
        return compiler("vector epilogue does not enter the original scalar header");
    }
    let original_header = original
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .expect("reproduced vector header");
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
    let mut chunk_starts = vec![scalar_chunk_zero];
    for unroll_index in 1..candidate.uf {
        let expected_offset = u32::from(unroll_index).saturating_mul(u32::from(candidate.vf));
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
                (left == header_induction && constant_value(right) == Some(expected_offset))
                    .then_some(result)
            })
            .collect::<Vec<_>>();
        let [start] = starts.as_slice() else {
            return compiler("vector UF chunk offset is missing or ambiguous");
        };
        chunk_starts.push(*start);
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
    let advances_full_chunk = vector_body.instructions.iter().any(|instruction| {
        instruction
            .results
            .iter()
            .any(|result| result.value == next_induction)
            && matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    op: crate::MirBinaryOp::Add,
                    left,
                    right,
                    semantics: crate::KirArithmeticSemantics::Modular,
                } if left == header_induction && constant_value(right) == Some(chunk_width)
            )
    });
    if !advances_full_chunk {
        return compiler("vector backedge does not advance by VF*UF");
    }
    let [region] = transformed.vector_regions.as_slice() else {
        return compiler("vector trial must create exactly one owned vector region");
    };
    if region.blocks != [vector_body.id] {
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
        if !operation_matches {
            return compiler("mapped vector instruction has the wrong operation family");
        }
    }
    if let Some(reduction) = &candidate.reduction {
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
        let expected_true = defining_instruction(then_scalar).and_then(|scalar| {
            plan.operations
                .iter()
                .find(|mapping| mapping.scalar == scalar)
                .and_then(|mapping| {
                    vector_body
                        .instructions
                        .iter()
                        .find(|instruction| instruction.id == mapping.vector)
                        .and_then(|instruction| instruction.results.first())
                        .map(|result| result.value)
                })
        });
        let expected_false = defining_instruction(else_scalar).and_then(|scalar| {
            plan.operations
                .iter()
                .find(|mapping| mapping.scalar == scalar)
                .and_then(|mapping| {
                    vector_body
                        .instructions
                        .iter()
                        .find(|instruction| instruction.id == mapping.vector)
                        .and_then(|instruction| instruction.results.first())
                        .map(|result| result.value)
                })
        });
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
        if chunk_starts.get(usize::from(group.unroll_index)) != Some(&emitted_start) {
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
    match plan.epilogue {
        VectorEpilogue::Scalar { start, end, .. }
            if start == candidate.induction && end == candidate.bound => {}
        _ => return compiler("vector scalar epilogue partition is false"),
    }
    let expected_predicates = 1_usize.saturating_add(
        candidate
            .version_predicate
            .as_ref()
            .map_or(0, |predicate| predicate.conjuncts.len()),
    );
    if plan.predicates.len() != expected_predicates
        || !matches!(
            plan.predicates[0],
            crate::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } if trip_count == candidate.bound && minimum == candidate.minimum_trip
        )
    {
        return compiler("vector trip threshold predicate is incomplete");
    }
    let emitted_version_predicates = preheader_after
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .count();
    if emitted_version_predicates != usize::from(candidate.version_predicate.is_some()) {
        return compiler("vector runtime version predicate presence is false");
    }
    if let Some(predicate) = &candidate.version_predicate {
        for conjunct in &predicate.conjuncts {
            let CheckedVersionConjunct::AddressIntervalsDisjoint { left, right } = conjunct;
            let left_region = candidate
                .accesses
                .iter()
                .find(|access| access.base == *left)
                .map(|access| access.region);
            let right_region = candidate
                .accesses
                .iter()
                .find(|access| access.base == *right)
                .map(|access| access.region);
            if !plan.predicates.iter().any(|planned| {
                matches!(
                    planned,
                    crate::VectorPredicate::AddressNonOverlap { left, right, bytes, .. }
                        if Some(*left) == left_region
                            && Some(*right) == right_region
                            && *bytes == candidate.bound
                )
            }) {
                return compiler("vector runtime noalias predicate is incomplete");
            }
        }
    }
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
    let operations = independently_collect_operations(
        original,
        &scalar_blocks,
        induction_transfer.id,
        reduction.as_ref(),
        diamond.as_ref(),
        plan,
    )?;
    let accesses =
        independently_collect_accesses(original, &scalar_blocks, body_induction, induction, plan)?;
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
    let minimum_trip = plan
        .predicates
        .iter()
        .find_map(|predicate| match predicate {
            crate::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } if *trip_count == bound => Some(*minimum),
            _ => None,
        })
        .ok_or_else(|| malformed("vector trip threshold predicate is missing"))?;
    let version_predicate = (!required_pairs.is_empty()).then(|| CheckedVersionPredicate {
        conjuncts: required_pairs
            .iter()
            .map(|(left, right)| {
                let left = accesses
                    .iter()
                    .find(|access| access.region == *left)
                    .map(|access| access.base)
                    .expect("required region has an access");
                let right = accesses
                    .iter()
                    .find(|access| access.region == *right)
                    .map(|access| access.base)
                    .expect("required region has an access");
                CheckedVersionConjunct::AddressIntervalsDisjoint { left, right }
            })
            .collect(),
    });
    let (predicted_cost, expected_minimum) = independently_price_vector_plan(
        pre_state,
        original,
        &scalar_blocks,
        &operations,
        &accesses,
        plan,
        version_predicate.is_some(),
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
        if instruction.id == induction_transfer || memory.contains(&instruction.id) {
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
            KirInstructionKind::ConstInt { .. } | KirInstructionKind::Copy { .. }
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
        if !forwards_from(function, *index, body_induction)
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
        });
    }
    accesses.sort_by_key(|access| access.instruction);
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
    plan: &VectorizationPlan,
    has_runtime_predicate: bool,
) -> Result<(KirCostEstimate, u32), TransactionCheckError> {
    let malformed = |message: &str| TransactionCheckError::compiler(message);
    let profile = &pre_state.module().profile;
    let lanes = u8::try_from(plan.vf)
        .map_err(|_| malformed("vector VF is not representable in the target profile"))?;
    let mut scalar_iteration = 0_u32;
    let mut vector_chunk = 0_u32;
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
        vector_chunk = vector_chunk.saturating_add(independent_profile_cost(
            profile,
            KirCostKey {
                operation: mapping.operation,
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
        for _ in 0..plan.uf {
            vector_chunk = vector_chunk.saturating_add(independent_profile_cost(
                profile,
                KirCostKey {
                    operation,
                    lane,
                    lanes,
                    semantics: KirCostSemantics::NotApplicable,
                    alignment,
                },
                false,
            )?);
        }
    }

    let mut loop_values = scalar_blocks
        .iter()
        .filter_map(|id| source_block(function, *id))
        .flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        })
        .collect::<BTreeSet<_>>();
    for block in &function.blocks {
        if successor_ids(&block.terminator)
            .iter()
            .any(|target| scalar_blocks.first() == Some(target))
        {
            loop_values.extend(block.params.iter().map(|param| param.value));
        }
    }
    let needs_splat = operations.iter().any(|operation| {
        if matches!(
            operation.operation,
            KirProfileOperation::ReduceAdd
                | KirProfileOperation::ReduceMultiply
                | KirProfileOperation::Select
        ) {
            return false;
        }
        defining_instruction_by_id(function, operation.scalar).is_some_and(|instruction| {
            operation_inputs(instruction)
                .into_iter()
                .any(|value| !loop_values.contains(&value))
        })
    });
    if needs_splat {
        for lane in operations
            .iter()
            .map(|operation| operation.lane_type)
            .collect::<BTreeSet<_>>()
        {
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
                matches!(predicate, crate::VectorPredicate::AddressNonOverlap { .. })
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    let predicate_cost = if has_runtime_predicate {
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
    };
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

fn forwards_from(function: &crate::KirFunction, value: ValueId, origin: ValueId) -> bool {
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
