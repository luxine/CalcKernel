use std::collections::BTreeSet;

use crate::{
    FunctionId, InstructionId, KirAlignmentClass, KirArithmeticSemantics, KirCostKey,
    KirCostSemantics, KirInstruction, KirInstructionKind, KirLaneType, KirOperationAvailability,
    KirProfileOperation, MemoryRegionId, MirBinaryOp, MirCastOp, MirPrimitiveTypeName, MirType,
    MirUnaryOp, ProofId, ValueId,
};

use super::{
    CandidateBudgetCharge, KirVerifiedProgramState, TransactionCheckError, VectorEpilogue,
    VectorPredicate, VectorizationPlan, kir_function_units,
};

/// Independently checks a closed vector plan against its immutable pre-state.
///
/// This module deliberately consumes only KIR, the closed plan, target profile,
/// proof arena, and fixed accounting rules. It has no dependency on candidate
/// proposal, alias/dependence analysis, or a proposal-side cost implementation.
pub fn check_vectorization_plan_independently(
    pre_state: &KirVerifiedProgramState,
    plan: &VectorizationPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    let module = pre_state.module();
    module
        .profile
        .validate()
        .map_err(TransactionCheckError::compiler)?;
    if plan.pre_state.kir_digest != pre_state.kir_digest() {
        return malformed("vector plan pre-state KIR identity is stale");
    }
    if plan.pre_state.profile_digest != module.profile.digest_hex() {
        return malformed("vector plan target profile identity is stale");
    }
    if plan.pre_state.evidence_generation != pre_state.evidence_generation() {
        return malformed("vector plan evidence generation is stale");
    }
    let Some(function) = module
        .functions
        .iter()
        .find(|function| function.id == plan.pre_state.function)
    else {
        return malformed("vector plan input function is missing");
    };
    let function_units = kir_function_units(function);
    if plan.pre_state.frozen_kir_units != function_units {
        return malformed("vector plan frozen function size is false");
    }
    if !matches!(plan.vf, 2 | 4 | 8 | 16) || !(1..=4).contains(&plan.uf) {
        return malformed("vector plan VF/UF is outside the closed schema");
    }
    if plan.operations.is_empty() {
        return malformed("vector plan contains no mapped operations");
    }

    let mut vector_ids = BTreeSet::new();
    let mut operation_identities = BTreeSet::new();
    let mut previous_identity = None;
    for operation in &plan.operations {
        let identity = (
            operation.scalar,
            operation.operation,
            operation.unroll_index,
        );
        if previous_identity.is_some_and(|previous| previous >= identity)
            || !operation_identities.insert(identity)
        {
            return malformed("vector plan scalar mappings are not a strict total order");
        }
        previous_identity = Some(identity);
        if !vector_ids.insert(operation.vector)
            || instruction(function, operation.vector).is_some()
            || operation.vector.index() < pre_state.ids().next_instruction
        {
            return malformed("vector plan vector identity is not fresh and unique");
        }
        let Some(scalar) = instruction(function, operation.scalar) else {
            return malformed("vector plan scalar instruction is missing from the pre-state");
        };
        let Some((expected_operation, expected_semantics, expected_lane, expected_alignment)) =
            scalar_operation(scalar)
        else {
            return malformed("vector plan scalar instruction is not vector-mappable");
        };
        if operation.operation != expected_operation
            || operation.semantics != expected_semantics
            || operation.lane_type != expected_lane
            || operation.alignment != expected_alignment
        {
            return malformed("vector plan operation mapping contradicts scalar KIR");
        }
        if operation.unroll_index >= plan.uf
            || operation.lanes.len() != usize::from(plan.vf)
            || operation.lanes.iter().enumerate().any(|(index, lane)| {
                usize::from(lane.lane) != index
                    || lane.scalar_iteration
                        != u32::from(operation.unroll_index)
                            .saturating_mul(u32::from(plan.vf))
                            .saturating_add(u32::try_from(index).unwrap_or(u32::MAX))
            })
        {
            return malformed("vector plan lane mapping is not complete source-order identity");
        }
        let lanes = u8::try_from(plan.vf)
            .map_err(|_| TransactionCheckError::compiler("vector VF is not representable"))?;
        let key = KirCostKey {
            operation: operation.operation,
            lane: operation.lane_type,
            lanes,
            semantics: operation.semantics,
            alignment: operation.alignment,
        };
        if !matches!(
            module.profile.operation_availability(&key),
            Some(KirOperationAvailability::Legal(_))
        ) {
            return Err(TransactionCheckError::reject(
                "target-operation-unavailable",
            ));
        }
    }

    if plan.predicates.len() > 4 {
        return malformed("vector plan has more than four runtime predicates");
    }
    verify_memory_groups(pre_state, function.id, plan)?;
    verify_predicates(pre_state, function.id, plan)?;
    verify_epilogue(pre_state, function.id, plan)?;
    verify_proof_roots(pre_state, function.id, plan)?;
    verify_cost(plan)?;
    verify_growth(module, function_units, plan)?;

    let expected_charge = independently_recompute_charge(plan);
    if charge != &expected_charge {
        return malformed("vector plan budget consumption is false");
    }
    Ok(())
}

fn verify_memory_groups(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    let function_data = pre_state
        .module()
        .functions
        .iter()
        .find(|item| item.id == function)
        .expect("caller established the function");
    let mut identities = BTreeSet::new();
    let mut vector_ids = plan
        .operations
        .iter()
        .map(|operation| operation.vector)
        .collect::<BTreeSet<_>>();
    for group in &plan.memory_groups {
        if group.unroll_index >= plan.uf
            || !identities.insert((
                group.region,
                group.access,
                group.scalar_instructions.clone(),
                group.unroll_index,
            ))
        {
            return malformed("vector plan contains a duplicate memory group");
        }
        if !function_data
            .regions
            .iter()
            .any(|region| region.id == group.region)
        {
            return malformed("vector plan memory region is missing");
        }
        if group.scalar_instructions.is_empty()
            || group
                .scalar_instructions
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || group
                .scalar_instructions
                .iter()
                .any(|id| instruction(function_data, *id).is_none())
        {
            return malformed("vector plan memory group has an invalid scalar footprint");
        }
        if !vector_ids.insert(group.vector_instruction)
            || instruction(function_data, group.vector_instruction).is_some()
            || group.vector_instruction.index() < pre_state.ids().next_instruction
        {
            return malformed("vector plan memory vector identity is not fresh and unique");
        }
        for scalar in &group.scalar_instructions {
            let source = instruction(function_data, *scalar)
                .expect("the scalar footprint was checked above");
            let expected_access = match source.kind {
                KirInstructionKind::Load { .. } => crate::VectorMemoryAccessKind::Read,
                KirInstructionKind::Store { .. } => crate::VectorMemoryAccessKind::Write,
                _ => return malformed("vector memory footprint is not load/store KIR"),
            };
            if group.access != expected_access
                || source
                    .memory
                    .as_ref()
                    .is_none_or(|memory| memory.region != group.region)
            {
                return malformed("vector memory footprint contradicts scalar KIR");
            }
        }
        verify_proof(pre_state, function, group.footprint_proof)?;
    }
    Ok(())
}

fn verify_predicates(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    for predicate in &plan.predicates {
        let (values, regions, proof, scalar_is_valid) = match predicate {
            VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                proof,
            } => (vec![*trip_count], Vec::new(), *proof, *minimum > 0),
            VectorPredicate::Divisibility {
                value,
                divisor,
                proof,
            } => (vec![*value], Vec::new(), *proof, *divisor > 1),
            VectorPredicate::AddressNonOverlap {
                left,
                right,
                bytes,
                proof,
            } => (vec![*bytes], vec![*left, *right], *proof, left != right),
            VectorPredicate::PowerOfTwoAlignment {
                value,
                alignment,
                proof,
            } => (
                vec![*value],
                Vec::new(),
                *proof,
                alignment.is_power_of_two(),
            ),
        };
        if !scalar_is_valid
            || values
                .into_iter()
                .any(|value| !value_exists(pre_state, function, value))
            || regions
                .into_iter()
                .any(|region| !region_exists(pre_state, function, region))
        {
            return malformed("vector runtime predicate is structurally false");
        }
        verify_proof(pre_state, function, proof)?;
    }
    Ok(())
}

fn verify_epilogue(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    if let VectorEpilogue::Scalar {
        start,
        end,
        coverage_proof,
    } = plan.epilogue
    {
        if !value_exists(pre_state, function, start) || !value_exists(pre_state, function, end) {
            return Err(TransactionCheckError::compiler(
                "vector epilogue range is missing from the pre-state",
            ));
        }
        verify_proof(pre_state, function, coverage_proof)?;
    }
    Ok(())
}

fn verify_proof_roots(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let roots = [
        plan.proofs.canonical_loop,
        plan.proofs.trip_partition,
        plan.proofs.lane_mapping,
        plan.proofs.operation_equivalence,
        plan.proofs.fallback_identity,
        plan.proofs.target_legality,
        plan.proofs.cost_and_budget,
    ];
    if roots.iter().copied().collect::<BTreeSet<_>>().len() != roots.len() {
        return Err(TransactionCheckError::compiler(
            "vector plan proof roots, including fallback identity, are not distinct",
        ));
    }
    for proof in roots {
        verify_proof(pre_state, function, proof)?;
    }
    Ok(())
}

fn verify_proof(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    proof: ProofId,
) -> Result<(), TransactionCheckError> {
    let Some(certificate) = pre_state.proofs().get(proof) else {
        return Err(TransactionCheckError::compiler(
            "vector plan names a missing proof root",
        ));
    };
    if certificate.use_site.function != function
        || certificate.generation != pre_state.evidence_generation()
    {
        return Err(TransactionCheckError::compiler(
            "vector plan proof root belongs to a different pre-state",
        ));
    }
    Ok(())
}

fn verify_cost(plan: &VectorizationPlan) -> Result<(), TransactionCheckError> {
    let expected_total = plan
        .cost
        .transformed_body
        .saturating_add(plan.cost.predicates)
        .saturating_add(plan.cost.epilogue);
    if plan.cost.scalar == 0 || plan.cost.total != expected_total {
        return Err(TransactionCheckError::compiler(
            "vector plan cost decomposition is false",
        ));
    }
    if u64::from(plan.cost.total).saturating_mul(100)
        > u64::from(plan.cost.scalar).saturating_mul(80)
    {
        return Err(TransactionCheckError::reject(
            "profitability-threshold-not-met",
        ));
    }
    Ok(())
}

fn verify_growth(
    module: &crate::KirModule,
    function_units: u32,
    plan: &VectorizationPlan,
) -> Result<(), TransactionCheckError> {
    let module_units = module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    });
    let expected_after = module_units
        .saturating_sub(function_units)
        .saturating_add(plan.growth.transformed_units);
    if plan.growth.original_units != function_units
        || plan.growth.module_before_units != module_units
        || plan.growth.module_after_units != expected_after
        || plan.growth.transformed_units > function_units.saturating_mul(3).saturating_add(32)
        || plan.growth.module_after_units > module_units.saturating_mul(2)
    {
        return Err(TransactionCheckError::compiler(
            "vector plan structural growth record is false",
        ));
    }
    Ok(())
}

fn independently_recompute_charge(plan: &VectorizationPlan) -> CandidateBudgetCharge {
    let lane_steps = plan.operations.iter().fold(0_u32, |total, operation| {
        total.saturating_add(u32::try_from(operation.lanes.len()).unwrap_or(u32::MAX))
    });
    let memory_steps = plan.memory_groups.iter().fold(0_u32, |total, group| {
        total.saturating_add(u32::try_from(group.scalar_instructions.len()).unwrap_or(u32::MAX))
    });
    let operations = u32::try_from(plan.operations.len()).unwrap_or(u32::MAX);
    let groups = u32::try_from(plan.memory_groups.len()).unwrap_or(u32::MAX);
    let predicates = u32::try_from(plan.predicates.len()).unwrap_or(u32::MAX);
    let epilogue = u32::from(matches!(plan.epilogue, VectorEpilogue::Scalar { .. }));
    let proposal_units = 8_u32
        .saturating_add(operations.saturating_mul(4))
        .saturating_add(lane_steps)
        .saturating_add(groups.saturating_mul(4))
        .saturating_add(memory_steps)
        .saturating_add(predicates.saturating_mul(3))
        .saturating_add(epilogue.saturating_mul(2));
    let checker = 16_u32
        .saturating_add(operations.saturating_mul(6))
        .saturating_add(lane_steps.saturating_mul(2))
        .saturating_add(groups.saturating_mul(6))
        .saturating_add(memory_steps.saturating_mul(2))
        .saturating_add(predicates.saturating_mul(4))
        .saturating_add(7)
        .saturating_add(epilogue.saturating_mul(3));
    CandidateBudgetCharge::single(plan.pre_state.function, proposal_units, checker)
}

fn instruction(function: &crate::KirFunction, id: InstructionId) -> Option<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
}

fn scalar_operation(
    instruction: &KirInstruction,
) -> Option<(
    KirProfileOperation,
    KirCostSemantics,
    KirLaneType,
    KirAlignmentClass,
)> {
    let lane = instruction
        .results
        .first()
        .and_then(|result| result.type_node.as_scalar())
        .and_then(lane_type)?;
    let (operation, semantics) = match instruction.kind {
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => KirProfileOperation::Add,
                MirBinaryOp::Sub => KirProfileOperation::Subtract,
                MirBinaryOp::Mul => KirProfileOperation::Multiply,
                MirBinaryOp::Div => KirProfileOperation::Divide,
                MirBinaryOp::Mod => KirProfileOperation::Remainder,
            },
            cost_semantics(semantics),
        ),
        KirInstructionKind::Unary { op, semantics, .. } => (
            match op {
                MirUnaryOp::Neg => KirProfileOperation::Negate,
                MirUnaryOp::Not => KirProfileOperation::MaskNot,
            },
            cost_semantics(semantics),
        ),
        KirInstructionKind::Compare { .. } => (
            KirProfileOperation::Compare,
            KirCostSemantics::NotApplicable,
        ),
        KirInstructionKind::Cast { op, .. } => (
            match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => KirProfileOperation::Cast,
            },
            KirCostSemantics::NotApplicable,
        ),
        _ => return None,
    };
    Some((operation, semantics, lane, KirAlignmentClass::NotApplicable))
}

const fn cost_semantics(semantics: KirArithmeticSemantics) -> KirCostSemantics {
    match semantics {
        KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
        KirArithmeticSemantics::Checked => KirCostSemantics::Checked,
        KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
    }
}

fn lane_type(ty: &MirType) -> Option<KirLaneType> {
    match ty {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(KirLaneType::I64),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some(KirLaneType::U64),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn value_exists(pre_state: &KirVerifiedProgramState, function: FunctionId, value: ValueId) -> bool {
    pre_state
        .module()
        .functions
        .iter()
        .find(|item| item.id == function)
        .is_some_and(|function| {
            function.params.iter().any(|param| param.value == value)
                || function.blocks.iter().any(|block| {
                    block.params.iter().any(|param| param.value == value)
                        || block.instructions.iter().any(|instruction| {
                            instruction
                                .results
                                .iter()
                                .any(|result| result.value == value)
                        })
                })
        })
}

fn region_exists(
    pre_state: &KirVerifiedProgramState,
    function: FunctionId,
    region: MemoryRegionId,
) -> bool {
    pre_state
        .module()
        .functions
        .iter()
        .find(|item| item.id == function)
        .is_some_and(|function| function.regions.iter().any(|item| item.id == region))
}
