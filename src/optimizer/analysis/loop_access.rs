use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, ContractFactPointer, ContractFactPredicate, FactArena, FactPredicate, InstructionId,
    KirFunction, KirInstructionKind, KirPlace, MemoryRegionId, MirBinaryOp, MirPrimitiveTypeName,
    MirType, ValueId, compute_kir_dominators,
};

use super::{
    AffineForm as SharedAffineForm, CanonicalLoopDescriptor, SymbolicByteInterval, analyze_regions,
};

const MAX_AFFINE_TERMS: usize = 32;
const MAX_AFFINE_DEPTH: u32 = 128;
const MAX_AFFINE_STEPS: u32 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMemoryAccessKind {
    Read,
    Write,
}

/// Arithmetic model attached to a normalized memory index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AffineArithmetic {
    /// Every represented operation has mathematical-integer semantics.
    Mathematical,
    /// At least one represented operation may wrap in its source integer type.
    MayWrap,
}

impl AffineArithmetic {
    const fn combine(self, other: Self) -> Self {
        if matches!(self, Self::MayWrap) || matches!(other, Self::MayWrap) {
            Self::MayWrap
        } else {
            Self::Mathematical
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffineMemoryAccess {
    pub instruction: InstructionId,
    pub block: BlockId,
    pub instruction_order: u32,
    pub kind: LoopMemoryAccessKind,
    pub source_region: MemoryRegionId,
    pub region: MemoryRegionId,
    pub base: ValueId,
    pub slice_base: bool,
    pub induction: ValueId,
    pub trip_start: BigInt,
    pub induction_step: BigInt,
    pub trip_bound: ValueId,
    /// Complete deterministic affine index over SSA leaves, including induction terms.
    pub affine_index: SharedAffineForm,
    pub arithmetic: AffineArithmetic,
    /// Whether every arithmetic operation in the index is non-wrapping over this loop domain.
    pub index_no_wrap: bool,
    /// Whether the compatibility fields below exactly express `affine_index`.
    pub legacy_projection_exact: bool,
    pub coefficient: BigInt,
    pub invariant_offset: Option<ValueId>,
    pub bias: BigInt,
    pub element_type: MirType,
    pub element_bytes: u32,
    pub base_alignment: u32,
    pub known_alignment: u32,
    pub slice_interval: Option<SymbolicByteInterval>,
    pub unit_stride: bool,
    pub vector_group_eligible: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopAccessAnalysis {
    pub accesses: Vec<AffineMemoryAccess>,
    pub rejected_instructions: Vec<InstructionId>,
    pub budget_exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedAffineForm {
    form: SharedAffineForm,
    arithmetic: AffineArithmetic,
    index_no_wrap: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AffineAnalysisBudget {
    pub(super) remaining_steps: u32,
    pub(super) exhausted: bool,
}

impl AffineAnalysisBudget {
    pub(super) fn for_function(function: &KirFunction) -> Self {
        let units = function
            .blocks
            .iter()
            .fold(function.params.len(), |count, block| {
                count
                    .saturating_add(block.params.len())
                    .saturating_add(block.instructions.len())
                    .saturating_add(1)
            });
        let units = u32::try_from(units).unwrap_or(u32::MAX);
        Self {
            remaining_steps: units
                .saturating_mul(16)
                .saturating_add(64)
                .min(MAX_AFFINE_STEPS),
            exhausted: false,
        }
    }

    pub(super) fn spend(&mut self) -> bool {
        if self.remaining_steps == 0 {
            self.exhausted = true;
            return false;
        }
        self.remaining_steps -= 1;
        true
    }
}

/// Indexes SSA definitions and incoming edges once so forwarding queries do not
/// rescan all of KIR for every value reached by a worklist.
pub(super) struct ForwardingIndex<'a> {
    parameters: BTreeMap<ValueId, (BlockId, usize)>,
    owners: BTreeMap<ValueId, BlockId>,
    definitions: BTreeMap<ValueId, &'a crate::KirInstruction>,
    incoming: BTreeMap<BlockId, Vec<(BlockId, &'a crate::KirEdge)>>,
}

impl<'a> ForwardingIndex<'a> {
    pub(super) fn new(
        function: &'a KirFunction,
        budget: &mut AffineAnalysisBudget,
    ) -> Option<Self> {
        let mut index = Self {
            parameters: BTreeMap::new(),
            owners: BTreeMap::new(),
            definitions: BTreeMap::new(),
            incoming: BTreeMap::new(),
        };
        for block in &function.blocks {
            if !budget.spend() {
                return None;
            }
            for (param_index, param) in block.params.iter().enumerate() {
                if !budget.spend() {
                    return None;
                }
                index
                    .parameters
                    .insert(param.value, (block.id, param_index));
                index.owners.insert(param.value, block.id);
            }
            for instruction in &block.instructions {
                if !budget.spend() {
                    return None;
                }
                for result in &instruction.results {
                    if !budget.spend() {
                        return None;
                    }
                    index.definitions.insert(result.value, instruction);
                    index.owners.insert(result.value, block.id);
                }
            }
            for edge in terminator_edges(&block.terminator) {
                if !budget.spend() {
                    return None;
                }
                index
                    .incoming
                    .entry(edge.target)
                    .or_default()
                    .push((block.id, edge));
            }
        }
        Some(index)
    }

    fn incoming_values_with_sources(
        &self,
        target: BlockId,
        param_index: usize,
    ) -> Vec<(BlockId, ValueId)> {
        self.incoming
            .get(&target)
            .into_iter()
            .flatten()
            .filter_map(|(source, edge)| {
                edge.args
                    .get(param_index)
                    .copied()
                    .map(|value| (*source, value))
            })
            .collect()
    }

    fn incoming_values(&self, target: BlockId, param_index: usize) -> Vec<ValueId> {
        self.incoming_values_with_sources(target, param_index)
            .into_iter()
            .map(|(_, value)| value)
            .collect()
    }

    pub(super) fn definition(&self, value: ValueId) -> Option<&'a crate::KirInstruction> {
        self.definitions.get(&value).copied()
    }
}

fn terminator_edges(terminator: &crate::KirTerminator) -> Vec<&crate::KirEdge> {
    match terminator {
        crate::KirTerminator::Return { .. } => Vec::new(),
        crate::KirTerminator::Jump { edge } => vec![edge],
        crate::KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
    }
}

/// Returns all terminal SSA values reachable through phi and Copy forwarding.
/// Empty incoming-phi lists are malformed and return `Ok(None)`; budget or
/// depth exhaustion returns `Err(())` and marks the shared budget exhausted.
pub(super) fn forwarding_leaves(
    index: &ForwardingIndex<'_>,
    value: ValueId,
    budget: &mut AffineAnalysisBudget,
) -> Result<Option<BTreeSet<ValueId>>, ()> {
    forwarding_leaves_inner(index, value, None, budget)
}

pub(super) fn forwarding_leaves_until(
    index: &ForwardingIndex<'_>,
    value: ValueId,
    origin: ValueId,
    budget: &mut AffineAnalysisBudget,
) -> Result<Option<BTreeSet<ValueId>>, ()> {
    forwarding_leaves_inner(index, value, Some(origin), budget)
}

fn forwarding_leaves_inner(
    index: &ForwardingIndex<'_>,
    value: ValueId,
    stop_at: Option<ValueId>,
    budget: &mut AffineAnalysisBudget,
) -> Result<Option<BTreeSet<ValueId>>, ()> {
    let mut pending = vec![(value, 0_u32)];
    let mut visited = BTreeSet::new();
    let mut leaves = BTreeSet::new();
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_AFFINE_DEPTH {
            budget.exhausted = true;
            return Err(());
        }
        if !visited.insert(value) {
            continue;
        }
        if !budget.spend() {
            return Err(());
        }
        if stop_at == Some(value) {
            leaves.insert(value);
        } else if let Some(&(block, param_index)) = index.parameters.get(&value) {
            let Some(incoming) = index.incoming.get(&block) else {
                return Ok(None);
            };
            if incoming.is_empty() {
                return Ok(None);
            }
            for (_, edge) in incoming {
                if !budget.spend() {
                    return Err(());
                }
                let Some(value) = edge.args.get(param_index).copied() else {
                    return Ok(None);
                };
                pending.push((value, depth + 1));
            }
        } else if let Some(crate::KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = index.definitions.get(&value).copied()
        {
            if !budget.spend() {
                return Err(());
            }
            pending.push((*value, depth + 1));
        } else {
            leaves.insert(value);
        }
    }
    Ok(Some(leaves))
}

pub fn analyze_affine_loop_accesses(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
) -> Result<LoopAccessAnalysis, String> {
    if descriptor.function != function.id
        || !descriptor.lcssa
        || descriptor.preheader.is_none()
        || descriptor.latch.is_none()
    {
        return Err("affine access analysis requires a fresh canonical loop".to_string());
    }
    let induction = descriptor
        .induction
        .as_ref()
        .ok_or_else(|| "affine access analysis requires a canonical induction".to_string())?;
    let regions = analyze_regions(function, facts).map_err(|error| error.message)?;
    let dominators = compute_kir_dominators(function);
    let loop_blocks = descriptor.blocks.iter().copied().collect::<BTreeSet<_>>();
    let mut budget = AffineAnalysisBudget::for_function(function);
    let Some(forwarding_index) = ForwardingIndex::new(function, &mut budget) else {
        let rejected_instructions = function
            .blocks
            .iter()
            .filter(|block| loop_blocks.contains(&block.id))
            .flat_map(|block| &block.instructions)
            .filter(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::Load { .. } | KirInstructionKind::Store { .. }
                )
            })
            .map(|instruction| instruction.id)
            .collect();
        return Ok(LoopAccessAnalysis {
            accesses: Vec::new(),
            rejected_instructions,
            budget_exhausted: true,
        });
    };
    let mut accesses = Vec::new();
    let mut rejected = Vec::new();
    for block in function
        .blocks
        .iter()
        .filter(|block| loop_blocks.contains(&block.id))
    {
        for (instruction_order, instruction) in block.instructions.iter().enumerate() {
            let (kind, place) = match &instruction.kind {
                KirInstructionKind::Load { place } => (LoopMemoryAccessKind::Read, place.as_ref()),
                KirInstructionKind::Store { place, .. } => {
                    (LoopMemoryAccessKind::Write, place.as_ref())
                }
                _ => continue,
            };
            let Some((base, index, element_type, source_region, slice_base)) = indexed_place(place)
            else {
                rejected.push(instruction.id);
                continue;
            };
            let mut visiting = BTreeSet::new();
            let Some(form) = affine_form(
                descriptor,
                &dominators,
                &forwarding_index,
                induction.value,
                index,
                &mut visiting,
                &mut budget,
                0,
                &induction.step,
                induction.wrap_safe_for_strict_bound,
            ) else {
                rejected.push(instruction.id);
                continue;
            };
            let Some(element_bytes) = primitive_bytes(&element_type) else {
                rejected.push(instruction.id);
                continue;
            };
            let region_descriptor = regions.descriptor(source_region);
            let region = region_descriptor.map_or(source_region, |descriptor| descriptor.root);
            let base_alignment = match fact_alignment(&forwarding_index, facts, base, &mut budget) {
                Ok(alignment) => alignment.unwrap_or(element_bytes).max(element_bytes),
                Err(()) => {
                    rejected.push(instruction.id);
                    continue;
                }
            };
            let affine_index = form.form;
            let (coefficient, invariant_offset, bias, legacy_projection_exact) =
                legacy_projection(&affine_index, induction.value);
            let arithmetic = if induction.wrap_safe_for_strict_bound {
                form.arithmetic
            } else {
                form.arithmetic.combine(AffineArithmetic::MayWrap)
            };
            let known_alignment = if legacy_projection_exact {
                access_alignment(
                    base_alignment,
                    element_bytes,
                    &induction.start,
                    &coefficient,
                    invariant_offset,
                    &bias,
                )
            } else {
                element_bytes
            };
            let unit_stride = induction.step == BigInt::from(1) && coefficient == BigInt::from(1);
            accesses.push(AffineMemoryAccess {
                instruction: instruction.id,
                block: block.id,
                instruction_order: u32::try_from(instruction_order).unwrap_or(u32::MAX),
                kind,
                source_region,
                region,
                base,
                slice_base,
                induction: induction.value,
                trip_start: induction.start.clone(),
                induction_step: induction.step.clone(),
                trip_bound: induction.bound,
                affine_index,
                arithmetic,
                index_no_wrap: form.index_no_wrap,
                legacy_projection_exact,
                coefficient,
                invariant_offset,
                bias,
                element_type,
                element_bytes,
                base_alignment,
                known_alignment,
                slice_interval: region_descriptor
                    .and_then(|descriptor| descriptor.byte_interval.clone()),
                unit_stride,
                vector_group_eligible: unit_stride && legacy_projection_exact,
            });
        }
    }
    accesses.sort_by_key(|access| access.instruction);
    rejected.sort_unstable();
    Ok(LoopAccessAnalysis {
        accesses,
        rejected_instructions: rejected,
        budget_exhausted: budget.exhausted,
    })
}

fn indexed_place(place: &KirPlace) -> Option<(ValueId, ValueId, MirType, MemoryRegionId, bool)> {
    match place {
        KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } => Some((*slice, *index, type_node.clone(), *region, true)),
        KirPlace::Index {
            base,
            index,
            type_node,
            region,
        } => root_value(base).map(|base| (base, *index, type_node.clone(), *region, false)),
        _ => None,
    }
}

fn root_value(place: &KirPlace) -> Option<ValueId> {
    match place {
        KirPlace::Value { value, .. } => Some(*value),
        KirPlace::Deref { pointer, .. } => Some(*pointer),
        KirPlace::Index { base, .. } | KirPlace::Field { base, .. } => root_value(base),
        KirPlace::SliceIndex { slice, .. } => Some(*slice),
    }
}

#[expect(clippy::too_many_arguments)]
fn affine_form(
    descriptor: &CanonicalLoopDescriptor,
    dominators: &crate::KirDominators,
    forwarding_index: &ForwardingIndex<'_>,
    induction: ValueId,
    value: ValueId,
    visiting: &mut BTreeSet<ValueId>,
    budget: &mut AffineAnalysisBudget,
    depth: u32,
    induction_step: &BigInt,
    induction_wrap_safe: bool,
) -> Option<NormalizedAffineForm> {
    if depth > MAX_AFFINE_DEPTH {
        budget.exhausted = true;
        return None;
    }
    if !budget.spend() {
        return None;
    }
    if value == induction {
        return Some(NormalizedAffineForm {
            form: SharedAffineForm::variable(value),
            arithmetic: AffineArithmetic::Mathematical,
            index_no_wrap: induction_wrap_safe,
        });
    }
    if let Some(constant) = integer_constant(forwarding_index, value) {
        return Some(NormalizedAffineForm {
            form: SharedAffineForm::integer(constant),
            arithmetic: AffineArithmetic::Mathematical,
            index_no_wrap: true,
        });
    }
    // Keep pre-loop values opaque. Their SSA identities remain distinct terms,
    // including outer-loop values used as invariants in a nested loop.
    if is_invariant_value(forwarding_index, descriptor, dominators, value) {
        return Some(NormalizedAffineForm {
            form: SharedAffineForm::variable(value),
            arithmetic: AffineArithmetic::Mathematical,
            index_no_wrap: true,
        });
    }
    if !visiting.insert(value) {
        return None;
    }
    let result = (|| {
        if let Some(&(block, index)) = forwarding_index.parameters.get(&value) {
            if block == descriptor.header {
                let incoming = forwarding_index.incoming_values_with_sources(block, index);
                let entry = incoming
                    .iter()
                    .find(|(source, _)| descriptor.blocks.binary_search(source).is_err())
                    .map(|(_, value)| *value)?;
                for (_, incoming_value) in &incoming {
                    if !budget.spend() {
                        return None;
                    }
                    let leaves =
                        forwarding_leaves_until(forwarding_index, *incoming_value, entry, budget)
                            .ok()??;
                    if leaves.iter().any(|leaf| *leaf != entry) {
                        return None;
                    }
                }
                affine_form(
                    descriptor,
                    dominators,
                    forwarding_index,
                    induction,
                    entry,
                    visiting,
                    budget,
                    depth + 1,
                    induction_step,
                    induction_wrap_safe,
                )
            } else {
                let incoming = forwarding_index.incoming_values(block, index);
                let mut forms = Vec::with_capacity(incoming.len());
                for incoming_value in incoming {
                    if !budget.spend() {
                        return None;
                    }
                    forms.push(affine_form(
                        descriptor,
                        dominators,
                        forwarding_index,
                        induction,
                        incoming_value,
                        visiting,
                        budget,
                        depth + 1,
                        induction_step,
                        induction_wrap_safe,
                    )?);
                }
                let first = forms.pop()?;
                forms.into_iter().all(|form| form == first).then_some(first)
            }
        } else if let Some(instruction) = forwarding_index.definitions.get(&value).copied() {
            match &instruction.kind {
                KirInstructionKind::Copy { value } => affine_form(
                    descriptor,
                    dominators,
                    forwarding_index,
                    induction,
                    *value,
                    visiting,
                    budget,
                    depth + 1,
                    induction_step,
                    induction_wrap_safe,
                ),
                KirInstructionKind::Binary {
                    op,
                    left,
                    right,
                    semantics,
                } => {
                    let left = affine_form(
                        descriptor,
                        dominators,
                        forwarding_index,
                        induction,
                        *left,
                        visiting,
                        budget,
                        depth + 1,
                        induction_step,
                        induction_wrap_safe,
                    )?;
                    let right = affine_form(
                        descriptor,
                        dominators,
                        forwarding_index,
                        induction,
                        *right,
                        visiting,
                        budget,
                        depth + 1,
                        induction_step,
                        induction_wrap_safe,
                    )?;
                    combine_affine(
                        *op,
                        *semantics,
                        left,
                        right,
                        budget,
                        induction,
                        induction_step,
                        induction_wrap_safe,
                    )
                }
                _ => None,
            }
        } else {
            None
        }
    })();
    visiting.remove(&value);
    result
}

#[expect(clippy::too_many_arguments)]
fn combine_affine(
    op: MirBinaryOp,
    semantics: crate::KirArithmeticSemantics,
    left: NormalizedAffineForm,
    right: NormalizedAffineForm,
    budget: &mut AffineAnalysisBudget,
    induction: ValueId,
    induction_step: &BigInt,
    induction_wrap_safe: bool,
) -> Option<NormalizedAffineForm> {
    let index_no_wrap = if semantics == crate::KirArithmeticSemantics::Modular {
        induction_wrap_safe
            && match op {
                MirBinaryOp::Add => {
                    (is_direct_induction_form(&left.form, induction)
                        && right.form.is_constant()
                        && right.form.constant() == induction_step)
                        || (is_direct_induction_form(&right.form, induction)
                            && left.form.is_constant()
                            && left.form.constant() == induction_step)
                }
                MirBinaryOp::Sub => {
                    is_direct_induction_form(&left.form, induction)
                        && right.form.is_constant()
                        && right.form.constant() == &-induction_step
                }
                MirBinaryOp::Mul | MirBinaryOp::Div | MirBinaryOp::Mod => false,
            }
    } else {
        left.index_no_wrap && right.index_no_wrap
    };
    let arithmetic = left.arithmetic.combine(right.arithmetic).combine(
        if semantics == crate::KirArithmeticSemantics::Modular {
            AffineArithmetic::MayWrap
        } else {
            AffineArithmetic::Mathematical
        },
    );
    let form = match op {
        MirBinaryOp::Add => left.form.add(&right.form),
        MirBinaryOp::Sub => left.form.sub(&right.form),
        MirBinaryOp::Mul if left.form.is_constant() => right.form.scale(left.form.constant()),
        MirBinaryOp::Mul if right.form.is_constant() => left.form.scale(right.form.constant()),
        MirBinaryOp::Mul | MirBinaryOp::Div | MirBinaryOp::Mod => return None,
    };
    if form.terms().len() > MAX_AFFINE_TERMS {
        budget.exhausted = true;
        return None;
    }
    Some(NormalizedAffineForm {
        form,
        arithmetic,
        index_no_wrap,
    })
}

fn is_direct_induction_form(form: &SharedAffineForm, induction: ValueId) -> bool {
    form.terms().len() == 1
        && form.coefficient(induction) == BigInt::from(1)
        && form.constant() == &BigInt::from(0)
}

fn legacy_projection(
    form: &SharedAffineForm,
    induction: ValueId,
) -> (BigInt, Option<ValueId>, BigInt, bool) {
    let coefficient = form.coefficient(induction);
    let mut invariant_terms = form
        .terms()
        .iter()
        .filter(|(value, _)| **value != induction);
    let first = invariant_terms.next();
    let exact = match (first, invariant_terms.next()) {
        (None, None) => true,
        (Some((_, coefficient)), None) => coefficient == &BigInt::from(1),
        _ => false,
    };
    let invariant_offset = if exact {
        first
            .filter(|(_, coefficient)| **coefficient == BigInt::from(1))
            .map(|(value, _)| *value)
    } else {
        None
    };
    (
        coefficient,
        invariant_offset,
        form.constant().clone(),
        exact,
    )
}

fn integer_constant(index: &ForwardingIndex<'_>, value: ValueId) -> Option<BigInt> {
    let instruction = index.definitions.get(&value)?;
    let KirInstructionKind::ConstInt { value } = &instruction.kind else {
        return None;
    };
    value.parse().ok()
}

fn is_invariant_value(
    index: &ForwardingIndex<'_>,
    descriptor: &CanonicalLoopDescriptor,
    dominators: &crate::KirDominators,
    value: ValueId,
) -> bool {
    index.owners.get(&value).is_none_or(|block| {
        descriptor.blocks.binary_search(block).is_err()
            && descriptor
                .preheader
                .is_some_and(|preheader| dominators.dominates(*block, preheader))
    })
}

fn primitive_bytes(type_node: &MirType) -> Option<u32> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => Some(8),
        MirType::Primitive(MirPrimitiveTypeName::Bool) => Some(1),
        MirType::Pointer(_) | MirType::Slice(_) | MirType::Struct(_) | MirType::Void => None,
    }
}

fn access_alignment(
    base_alignment: u32,
    element_bytes: u32,
    start: &BigInt,
    coefficient: &BigInt,
    invariant: Option<ValueId>,
    bias: &BigInt,
) -> u32 {
    if invariant.is_some() {
        return element_bytes;
    }
    let first = (start * coefficient + bias) * BigInt::from(element_bytes);
    let stride = coefficient * BigInt::from(element_bytes);
    gcd_u32(
        gcd_u32(base_alignment, bigint_alignment_component(&first)),
        bigint_alignment_component(&stride),
    )
    .max(1)
}

fn bigint_alignment_component(value: &BigInt) -> u32 {
    if value == &BigInt::from(0) {
        return 0;
    }
    value
        .to_string()
        .trim_start_matches('-')
        .parse::<u64>()
        .ok()
        .map_or(1, |value| u32::try_from(value).unwrap_or(1))
}

const fn gcd_u32(mut left: u32, mut right: u32) -> u32 {
    if left == 0 {
        return right;
    }
    if right == 0 {
        return left;
    }
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn fact_alignment(
    index: &ForwardingIndex<'_>,
    facts: Option<&FactArena>,
    base: ValueId,
    budget: &mut AffineAnalysisBudget,
) -> Result<Option<u32>, ()> {
    let Some(facts) = facts else {
        return Ok(None);
    };
    let mut aligned_facts = Vec::new();
    for fact in facts.facts() {
        if !budget.spend() {
            return Err(());
        }
        let FactPredicate::Contract(ContractFactPredicate::Aligned { pointer, alignment }) =
            &fact.predicate
        else {
            continue;
        };
        let origin = match pointer {
            ContractFactPointer::Value(value) | ContractFactPointer::SliceData(value) => *value,
        };
        aligned_facts.push((origin, *alignment));
    }
    if aligned_facts.is_empty() {
        return Ok(None);
    }
    let mut alignment = None;
    for (origin, fact_alignment) in aligned_facts {
        let Some(leaves) = forwarding_leaves_until(index, base, origin, budget)? else {
            continue;
        };
        if leaves.iter().all(|leaf| *leaf == origin) {
            alignment = Some(alignment.unwrap_or(0_u32).max(fact_alignment));
        }
    }
    Ok(alignment)
}

#[cfg(test)]
#[path = "loop_access_tests.rs"]
mod tests;
