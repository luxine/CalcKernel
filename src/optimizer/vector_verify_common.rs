use num_bigint::BigInt;

use super::vectorize_check::{checked_loop_bound_is_invariant, forwards_from};
use crate::{
    AffineArithmetic, AffineForm, AffineMemoryAccess, AliasKind, BlockId, DependenceQueryBudget,
    DependenceRelation, DependenceWrapFacts, FactScope, IndexWrapProof, InstructionId,
    KirAlignmentClass, KirBlock, KirCostKey, KirCostSemantics, KirEffectKind, KirFunction,
    KirInstruction, KirInstructionKind, KirLaneType, KirOperationAvailability, KirPlace,
    KirProfileOperation, KirTargetProfile, KirTerminator, KirValueType, KirVerifiedProgramState,
    LoopMemoryAccessKind, MemoryRegionId, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType,
    TransactionCheckError, ValueId,
};

type CheckResult<T> = Result<T, TransactionCheckError>;

fn error(message: &str) -> TransactionCheckError {
    TransactionCheckError::compiler(message)
}

/// Purpose-specific fallback allowed by the existing profile cost policy.
/// Only scalar loop-control branches may use the unavailable-operation cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CostFallback {
    None,
    ScalarBranchControl,
}

/// Looks up a target operation cost without accepting a proposed cost value.
/// The caller remains responsible for its established error classification.
pub(super) fn target_operation_cost(
    profile: &KirTargetProfile,
    key: &KirCostKey,
    fallback: CostFallback,
) -> Option<u32> {
    match profile.operation_availability(key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        Some(KirOperationAvailability::Unavailable)
            if fallback == CostFallback::ScalarBranchControl
                && key.operation == KirProfileOperation::Branch
                && key.lane == KirLaneType::U32
                && key.lanes == 1
                && key.semantics == KirCostSemantics::NotApplicable
                && key.alignment == KirAlignmentClass::NotApplicable =>
        {
            Some(1)
        }
        _ => None,
    }
}

/// Shared bounded dependence query for independently reconstructed source
/// access records. Callers must establish the supplied alias and wrap facts
/// from their own source proof before invoking it.
pub(super) fn query_memory_access_pair(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
    alias: AliasKind,
    wrap: DependenceWrapFacts,
) -> crate::MemoryDependenceResult {
    crate::query_memory_dependence(
        left,
        right,
        alias,
        wrap,
        DependenceQueryBudget::with_max_steps(128),
    )
}

pub(super) fn memory_dependence_is_safe(result: &crate::MemoryDependenceResult) -> bool {
    !result.budget_exhausted
        && result.dependences.iter().all(|dependence| {
            !matches!(
                dependence.relation,
                DependenceRelation::LoopCarried | DependenceRelation::Unknown
            )
        })
}

#[derive(Debug, Clone, Copy)]
struct ReconstructedAccess {
    instruction: InstructionId,
    block: BlockId,
    kind: LoopMemoryAccessKind,
    base: ValueId,
    index: ValueId,
    place_region: MemoryRegionId,
    partition: MemoryRegionId,
    instruction_order: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CheckedDecisionTreeMemoryExposure {
    pub(super) input_slice: ValueId,
    pub(super) output_slice: ValueId,
    pub(super) input_region: MemoryRegionId,
    pub(super) input_partition: MemoryRegionId,
    pub(super) output_region: MemoryRegionId,
    pub(super) output_partition: MemoryRegionId,
}

/// Reconstructs a tree's memory effects after its independent CFG traversal
/// has certified that `leaf_stores` are the complete, mutually exclusive leaf
/// set. This routine verifies the common access address, canonical trip domain,
/// contract availability, and alias/dependence facts from source KIR.
#[allow(clippy::too_many_arguments)]
pub(super) fn verify_decision_tree_memory_exposure(
    state: &KirVerifiedProgramState,
    function: &KirFunction,
    exposure: &super::decision_tree_vector_check::VerifiedTreeExposure,
) -> CheckResult<CheckedDecisionTreeMemoryExposure> {
    if exposure.function() != function.id {
        return Err(error(
            "decision-tree exposure belongs to a different function",
        ));
    }
    let (preheader_id, _, root_id, _, induction, bound, _) = exposure.loop_parts();
    let root_load = exposure.root_load();
    let leaf_stores = exposure.leaf_stores();
    if !(2..=4).contains(&leaf_stores.len()) {
        return Err(error(
            "decision-tree memory exposure has an invalid leaf count",
        ));
    }
    check_canonical_unit_stride_loop(function, exposure)?;

    let root = block(function, root_id)?;
    let input = reconstruct_access(function, root_load, LoopMemoryAccessKind::Read)?;
    if input.block != root_id || !forwards_from(function, input.index, induction) {
        return Err(error(
            "decision-tree memory access is not the canonical unit-stride source address",
        ));
    }
    let root_loads = root
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .count();
    if root_loads != 1 {
        return Err(error("decision-tree root memory exposure is not unique"));
    }

    let mut outputs = Vec::with_capacity(leaf_stores.len());
    let mut seen_stores = std::collections::BTreeSet::new();
    for (leaf_id, store_id) in leaf_stores {
        if !seen_stores.insert(*store_id) {
            return Err(error("decision-tree leaf store identity is duplicated"));
        }
        let leaf = block(function, *leaf_id)?;
        let stores = leaf
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
            .collect::<Vec<_>>();
        if stores.len() != 1 || stores[0].id != *store_id {
            return Err(error(
                "decision-tree logical write omits a source leaf store",
            ));
        }
        let output = reconstruct_access(function, *store_id, LoopMemoryAccessKind::Write)?;
        if output.block != *leaf_id || !forwards_from(function, output.index, induction) {
            return Err(error(
                "decision-tree memory access is not the canonical unit-stride source address",
            ));
        }
        outputs.push(output);
    }

    let output = outputs
        .first()
        .copied()
        .ok_or_else(|| error("decision-tree logical write is missing"))?;
    if outputs.iter().any(|access| {
        access.base != output.base
            || access.place_region != output.place_region
            || access.partition != output.partition
    }) {
        return Err(error(
            "decision-tree leaves do not share one exact logical write address",
        ));
    }

    let alias_analysis = crate::optimizer::analysis::analyze_regions(
        function,
        state.contract_facts().map(crate::ContractFactSet::facts),
    )
    .map_err(|failure| error(&failure.message))?;
    let dominators = crate::compute_kir_dominators(function);
    for write in &outputs {
        let alias = crate::optimizer::analysis::query_alias(
            &alias_analysis,
            input.place_region,
            write.place_region,
        );
        if alias.kind != AliasKind::NoAlias
            || !noalias_available_at(
                state,
                function,
                preheader_id,
                input.base,
                write.base,
                &dominators,
            )
        {
            return Err(error(
                "decision-tree NoAlias evidence is not available at the preheader",
            ));
        }
    }

    let read_access = dependence_access(function, input, induction, bound)?;
    let write_accesses = outputs
        .iter()
        .copied()
        .map(|access| dependence_access(function, access, induction, bound))
        .collect::<CheckResult<Vec<_>>>()?;
    for write in &write_accesses {
        let self_dependence = query_memory_access_pair(
            write,
            write,
            AliasKind::MustAlias,
            no_wrap_facts(write, write),
        );
        if !memory_dependence_is_safe(&self_dependence) {
            return Err(error(
                "decision-tree output address is not injective per trip",
            ));
        }
    }
    for (source, write) in outputs.iter().zip(&write_accesses) {
        let alias = crate::optimizer::analysis::query_alias(
            &alias_analysis,
            input.place_region,
            source.place_region,
        )
        .kind;
        let dependence = query_memory_access_pair(
            &read_access,
            write,
            alias,
            no_wrap_facts(&read_access, write),
        );
        if !memory_dependence_is_safe(&dependence) {
            return Err(error(
                "decision-tree source read/write dependence is not proven safe",
            ));
        }
    }

    Ok(CheckedDecisionTreeMemoryExposure {
        input_slice: input.base,
        output_slice: output.base,
        input_region: input.place_region,
        input_partition: input.partition,
        output_region: output.place_region,
        output_partition: output.partition,
    })
}

#[allow(clippy::too_many_arguments)]
fn check_canonical_unit_stride_loop(
    function: &KirFunction,
    exposure: &super::decision_tree_vector_check::VerifiedTreeExposure,
) -> CheckResult<()> {
    let (preheader_id, header_id, root_id, join_id, induction, bound, root_induction) =
        exposure.loop_parts();
    let leaf_stores = exposure.leaf_stores();
    let preheader = block(function, preheader_id)?;
    let header = block(function, header_id)?;
    let _root = block(function, root_id)?;
    let join = block(function, join_id)?;
    let KirTerminator::Jump { edge: entry } = &preheader.terminator else {
        return Err(error("decision-tree source preheader is not a jump"));
    };
    if entry.target != header_id {
        return Err(error(
            "decision-tree source preheader does not enter its header",
        ));
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        ..
    } = &header.terminator
    else {
        return Err(error("decision-tree source header is not a loop branch"));
    };
    let induction_slot = header
        .params
        .iter()
        .position(|parameter| parameter.value == induction)
        .ok_or_else(|| error("decision-tree canonical induction is not a header parameter"))?;
    if !is_u32(function, induction) || !is_u32(function, bound) {
        return Err(error("decision-tree canonical loop values are not u32"));
    }
    let entry_induction = entry
        .args
        .get(induction_slot)
        .copied()
        .ok_or_else(|| error("decision-tree canonical induction entry is missing"))?;
    if !int_constant(function, entry_induction, "0") {
        return Err(error(
            "decision-tree canonical induction does not start at zero",
        ));
    }
    let compare = definition(function, *condition)
        .ok_or_else(|| error("decision-tree canonical loop comparison is missing"))?;
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left,
        right,
    } = compare.kind
    else {
        return Err(error(
            "decision-tree canonical loop test is not strict increasing",
        ));
    };
    let entry_bound = header
        .params
        .iter()
        .position(|parameter| parameter.value == right)
        .and_then(|index| entry.args.get(index).copied())
        .unwrap_or(right);
    if left != induction
        || entry_bound != bound
        || !checked_loop_bound_is_invariant(function, preheader_id, right, bound)
        || then_edge.target != root_id
        || !forwards_from(function, root_induction, induction)
    {
        return Err(error(
            "decision-tree canonical unit-stride trip domain is not proven",
        ));
    }

    if !join.instructions.is_empty() {
        return Err(error("decision-tree canonical latch contains work"));
    }
    let KirTerminator::Jump { edge: latch } = &join.terminator else {
        return Err(error("decision-tree canonical latch is not a jump"));
    };
    if latch.target != header_id || latch.args.len() != header.params.len() {
        return Err(error("decision-tree canonical latch state differs"));
    }
    let latch_iv = *latch
        .args
        .get(induction_slot)
        .ok_or_else(|| error("decision-tree canonical latch omits its induction"))?;
    let join_iv_slot = join
        .params
        .iter()
        .position(|parameter| parameter.value == latch_iv)
        .ok_or_else(|| error("decision-tree canonical latch does not join leaf updates"))?;

    let mut leaf_blocks = std::collections::BTreeSet::new();
    for (leaf_id, store_id) in leaf_stores {
        if !leaf_blocks.insert(*leaf_id) {
            return Err(error("decision-tree leaf block is duplicated"));
        }
        let leaf = block(function, *leaf_id)?;
        let KirTerminator::Jump { edge } = &leaf.terminator else {
            return Err(error(
                "decision-tree leaf does not reach the canonical latch",
            ));
        };
        if edge.target != join_id || edge.args.len() != join.params.len() {
            return Err(error(
                "decision-tree leaf continuation does not reach the latch",
            ));
        }
        if !leaf.instructions.iter().any(|item| item.id == *store_id) {
            return Err(error("decision-tree leaf store is outside its leaf block"));
        }
        let updates = leaf
            .instructions
            .iter()
            .filter(|item| is_unit_increment(function, item, induction))
            .collect::<Vec<_>>();
        let [update] = updates.as_slice() else {
            return Err(error(
                "decision-tree leaf does not have one proven no-wrap induction update",
            ));
        };
        let update_result = single_result(update)?;
        let leaf_iv = edge
            .args
            .get(join_iv_slot)
            .copied()
            .ok_or_else(|| error("decision-tree leaf omits its induction update"))?;
        if !forwards_from(function, leaf_iv, update_result) {
            return Err(error(
                "decision-tree leaf induction update does not reach the canonical latch",
            ));
        }
    }
    Ok(())
}

fn is_unit_increment(
    function: &KirFunction,
    instruction: &KirInstruction,
    induction: ValueId,
) -> bool {
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
    {
        return false;
    }
    matches!(
        instruction.kind,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: crate::KirArithmeticSemantics::Modular,
        } if (forwards_from(function, left, induction) && int_constant(function, right, "1"))
            || (forwards_from(function, right, induction) && int_constant(function, left, "1"))
    ) && is_u32(function, instruction.results[0].value)
}

fn reconstruct_access(
    function: &KirFunction,
    id: InstructionId,
    kind: LoopMemoryAccessKind,
) -> CheckResult<ReconstructedAccess> {
    let instruction = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
        .ok_or_else(|| error("decision-tree source memory instruction is missing"))?;
    let place = match (&instruction.kind, kind) {
        (KirInstructionKind::Load { place }, LoopMemoryAccessKind::Read) => place.as_ref(),
        (KirInstructionKind::Store { place, value }, LoopMemoryAccessKind::Write) => {
            if value_type(function, *value)
                != Some(KirValueType::Scalar(MirType::Primitive(
                    MirPrimitiveTypeName::F64,
                )))
            {
                return Err(error("decision-tree stored value is not f64"));
            }
            place.as_ref()
        }
        _ => return Err(error("decision-tree source memory kind differs")),
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        region,
    } = place
    else {
        return Err(error("decision-tree memory source is not a slice index"));
    };
    let memory = instruction
        .memory
        .as_ref()
        .ok_or_else(|| error("decision-tree source MemorySSA is missing"))?;
    let descriptor = function
        .regions
        .iter()
        .find(|descriptor| descriptor.id == *region)
        .ok_or_else(|| error("decision-tree source memory region is missing"))?;
    let write = kind == LoopMemoryAccessKind::Write;
    if *type_node != MirType::Primitive(MirPrimitiveTypeName::F64)
        || !matches!(descriptor.origin,
            crate::KirMemoryRegionOrigin::Parameter(origin)
                if stable_invariant_descriptor_root(function, *slice) == Some(origin))
        || !is_u32(function, *index)
        || descriptor.partition != memory.region
        || !function
            .regions
            .iter()
            .any(|candidate| candidate.id == memory.region && candidate.partition == memory.region)
        || memory.output.is_some() != write
        || instruction.effect.as_ref().is_none_or(|effect| {
            effect.kind
                != if write {
                    KirEffectKind::WriteMemory
                } else {
                    KirEffectKind::ReadMemory
                }
        })
        || if write {
            !instruction.results.is_empty()
        } else {
            instruction.results.len() != 1
                || instruction.results[0].type_node.as_scalar() != Some(type_node)
        }
    {
        return Err(error(
            "decision-tree source memory type, partition, or effect is false",
        ));
    }
    let block = function
        .blocks
        .iter()
        .find(|block| block.instructions.iter().any(|item| item.id == id))
        .ok_or_else(|| error("decision-tree source memory block is missing"))?;
    let instruction_order = u32::try_from(
        block
            .instructions
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| error("decision-tree source memory order is missing"))?,
    )
    .map_err(|_| error("decision-tree source memory order overflows"))?;
    let base = stable_invariant_descriptor_root(function, *slice)
        .ok_or_else(|| error("decision-tree slice has no stable descriptor origin"))?;
    Ok(ReconstructedAccess {
        instruction: id,
        block: block.id,
        kind,
        base,
        index: *index,
        place_region: *region,
        partition: memory.region,
        instruction_order,
    })
}

fn dependence_access(
    function: &KirFunction,
    access: ReconstructedAccess,
    induction: ValueId,
    bound: ValueId,
) -> CheckResult<AffineMemoryAccess> {
    let block = block(function, access.block)?;
    let instruction = block
        .instructions
        .iter()
        .find(|instruction| instruction.id == access.instruction)
        .ok_or_else(|| error("decision-tree checked memory instruction disappeared"))?;
    let (element_type, element_bytes) = match &instruction.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
            let KirPlace::SliceIndex { type_node, .. } = place.as_ref() else {
                return Err(error("decision-tree checked memory place changed"));
            };
            (type_node.clone(), 8)
        }
        _ => return Err(error("decision-tree checked memory kind changed")),
    };
    Ok(AffineMemoryAccess {
        instruction: access.instruction,
        block: access.block,
        instruction_order: access.instruction_order,
        kind: access.kind,
        source_region: access.place_region,
        region: access.place_region,
        base: access.base,
        slice_base: true,
        induction,
        trip_start: BigInt::from(0_u8),
        induction_step: BigInt::from(1_u8),
        trip_bound: bound,
        affine_index: AffineForm::variable(induction),
        arithmetic: AffineArithmetic::Mathematical,
        // This fact is reconstructed from the direct canonical IV, invariant
        // u32 bound, zero entry, and exact per-leaf unit updates above.
        index_no_wrap: true,
        legacy_projection_exact: true,
        coefficient: BigInt::from(1_u8),
        invariant_offset: None,
        bias: BigInt::from(0_u8),
        element_type,
        element_bytes,
        base_alignment: 8,
        known_alignment: 8,
        slice_interval: None,
        unit_stride: true,
        vector_group_eligible: true,
    })
}

fn no_wrap_facts(left: &AffineMemoryAccess, right: &AffineMemoryAccess) -> DependenceWrapFacts {
    let mut facts = DependenceWrapFacts::from_accesses(left, right);
    facts.left = IndexWrapProof::ProvenNoWrap;
    facts.right = IndexWrapProof::ProvenNoWrap;
    facts
}

pub(super) fn noalias_available_at(
    state: &KirVerifiedProgramState,
    function: &KirFunction,
    preheader: BlockId,
    left: ValueId,
    right: ValueId,
    dominators: &crate::KirDominators,
) -> bool {
    state.contract_facts().is_some_and(|contracts| {
        contracts.facts().facts().iter().any(|fact| {
            let scope_available = match &fact.scope {
                FactScope::FunctionEntry(owner) => *owner == function.id,
                FactScope::Block {
                    function: owner,
                    block,
                } => *owner == function.id && dominators.dominates(*block, preheader),
                _ => false,
            };
            scope_available
                && fact.generation == state.evidence_generation()
                && matches!(fact.origin, crate::FactOrigin::TrustedContract { .. })
                && fact.derivation == crate::FactDerivation::TrustedContractLeaf
                && matches!(
                    fact.predicate,
                    crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias {
                        left: fact_left,
                        right: fact_right,
                    }) if (forwards_from(function, left, fact_left)
                        && forwards_from(function, right, fact_right))
                        || (forwards_from(function, left, fact_right)
                            && forwards_from(function, right, fact_left))
                )
        })
    })
}

fn block(function: &KirFunction, id: BlockId) -> CheckResult<&KirBlock> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| error("decision-tree source block is missing"))
}

fn definition(function: &KirFunction, value: ValueId) -> Option<&KirInstruction> {
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

fn single_result(instruction: &KirInstruction) -> CheckResult<ValueId> {
    match instruction.results.as_slice() {
        [result] => Ok(result.value),
        _ => Err(error("decision-tree induction update result arity differs")),
    }
}

fn value_type(function: &KirFunction, value: ValueId) -> Option<KirValueType> {
    function
        .params
        .iter()
        .find(|parameter| parameter.value == value)
        .map(|parameter| parameter.type_node.clone().into())
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.params)
                .find(|parameter| parameter.value == value)
                .map(|parameter| parameter.type_node.clone())
        })
        .or_else(|| {
            definition(function, value).and_then(|instruction| {
                instruction
                    .results
                    .iter()
                    .find(|result| result.value == value)
                    .map(|result| result.type_node.clone())
            })
        })
}

fn is_u32(function: &KirFunction, value: ValueId) -> bool {
    value_type(function, value)
        == Some(KirValueType::Scalar(MirType::Primitive(
            MirPrimitiveTypeName::U32,
        )))
}

fn int_constant(function: &KirFunction, value: ValueId, expected: &str) -> bool {
    is_u32(function, value)
        && matches!(definition(function, value).map(|instruction| &instruction.kind),
            Some(KirInstructionKind::ConstInt { value }) if value == expected)
}

fn stable_invariant_descriptor_root(function: &KirFunction, value: ValueId) -> Option<ValueId> {
    super::vectorize_check::stable_invariant_descriptor_root(function, value)
}
