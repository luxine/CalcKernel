use std::collections::{BTreeMap, BTreeSet};

use crate::{
    FactArena, InstructionId, KirArithmeticSemantics, KirEffectKind, KirFunction,
    KirInstructionKind, MemoryRegionId, MirBinaryOp, MirPrimitiveTypeName, MirType, ValueId,
};

use super::loop_access::{
    AffineAnalysisBudget, ForwardingIndex, forwarding_leaves, forwarding_leaves_until,
};
use super::{
    AffineMemoryAccess, AliasKind, CanonicalLoopDescriptor, LoopFallbackReason,
    LoopMemoryAccessKind, LoopTripCount, analyze_affine_loop_accesses, analyze_regions,
    query_alias,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopDependenceKind {
    ReadRead,
    Independent,
    SameIteration,
    ModularReduction,
    Dependent,
    RuntimeGuarded,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependenceDirection {
    Raw,
    War,
    Waw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependenceRelation {
    Independent,
    SameIteration,
    LoopCarried,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryDependence {
    pub direction: DependenceDirection,
    pub relation: DependenceRelation,
    /// Signed `right_iteration - left_iteration` when a fixed distance is known.
    pub distance: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryDependenceResult {
    pub dependences: Vec<MemoryDependence>,
    pub budget_exhausted: bool,
}

/// Caller-supplied premise for reasoning about source-level modular index arithmetic.
/// The caller must independently establish each non-wrap fact before supplying it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexWrapProof {
    Unknown,
    ProvenNoWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependenceWrapFacts {
    pub left: IndexWrapProof,
    pub right: IndexWrapProof,
}

impl DependenceWrapFacts {
    #[must_use]
    pub const fn from_accesses(left: &AffineMemoryAccess, right: &AffineMemoryAccess) -> Self {
        Self {
            left: if left.index_no_wrap {
                IndexWrapProof::ProvenNoWrap
            } else {
                IndexWrapProof::Unknown
            },
            right: if right.index_no_wrap {
                IndexWrapProof::ProvenNoWrap
            } else {
                IndexWrapProof::Unknown
            },
        }
    }
}

/// Bounded work supplied to the pure access-pair query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependenceQueryBudget {
    remaining_steps: u32,
}

impl DependenceQueryBudget {
    #[must_use]
    pub const fn with_max_steps(max_steps: u32) -> Self {
        Self {
            remaining_steps: max_steps,
        }
    }

    fn spend(&mut self) -> bool {
        if self.remaining_steps == 0 {
            return false;
        }
        self.remaining_steps -= 1;
        true
    }
}

impl Default for DependenceQueryBudget {
    fn default() -> Self {
        Self::with_max_steps(128)
    }
}

/// Compares two normalized affine accesses using only caller-supplied alias
/// and wrap premises. It never reconstructs either premise from profile data.
#[must_use]
pub fn query_memory_dependence(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
    alias: AliasKind,
    wrap_facts: DependenceWrapFacts,
    mut budget: DependenceQueryBudget,
) -> MemoryDependenceResult {
    let Some(direction) = default_direction(left, right) else {
        return MemoryDependenceResult::default();
    };
    if !budget.spend() {
        return unknown_dependence(direction, true);
    }
    if alias == AliasKind::NoAlias {
        return dependence_result(direction, DependenceRelation::Independent, None);
    }
    if alias == AliasKind::MayAlias {
        return unknown_dependence(direction, false);
    }
    if left.base != right.base
        || left.element_bytes != right.element_bytes
        || left.induction != right.induction
        || left.induction_step != right.induction_step
        || left.affine_index.terms().len() > MAX_DEPENDENCE_TERMS
        || right.affine_index.terms().len() > MAX_DEPENDENCE_TERMS
    {
        return unknown_dependence(
            direction,
            left.affine_index.terms().len() > MAX_DEPENDENCE_TERMS
                || right.affine_index.terms().len() > MAX_DEPENDENCE_TERMS,
        );
    }

    let left_coefficient = left.affine_index.coefficient(left.induction);
    let right_coefficient = right.affine_index.coefficient(right.induction);
    if left_coefficient != right_coefficient {
        return unknown_dependence(direction, false);
    }
    let mut left_terms = left
        .affine_index
        .terms()
        .iter()
        .filter(|(value, _)| **value != left.induction);
    let mut right_terms = right
        .affine_index
        .terms()
        .iter()
        .filter(|(value, _)| **value != right.induction);
    loop {
        if !budget.spend() {
            return unknown_dependence(direction, true);
        }
        match (left_terms.next(), right_terms.next()) {
            (None, None) => break,
            (Some((left_value, left_coefficient)), Some((right_value, right_coefficient)))
                if left_value == right_value && left_coefficient == right_coefficient => {}
            _ => return unknown_dependence(direction, false),
        }
    }

    let wrap_is_proven = matches!(wrap_facts.left, IndexWrapProof::ProvenNoWrap)
        && matches!(wrap_facts.right, IndexWrapProof::ProvenNoWrap);
    let displacement = left.affine_index.constant() - right.affine_index.constant();
    let stride = left_coefficient * &left.induction_step;
    if stride == 0.into() {
        if displacement == 0.into() {
            return dependence_result(direction, DependenceRelation::LoopCarried, Some(1));
        }
        return if wrap_is_proven {
            dependence_result(direction, DependenceRelation::Independent, None)
        } else {
            unknown_dependence(direction, false)
        };
    }
    if !wrap_is_proven {
        return unknown_dependence(direction, false);
    }
    let remainder = &displacement % &stride;
    if remainder != 0.into() {
        return dependence_result(direction, DependenceRelation::Independent, None);
    }
    let distance = displacement / stride;
    let Some(distance) = distance.to_string().parse::<i64>().ok() else {
        return unknown_dependence(direction, false);
    };
    if distance == 0 {
        if left.block != right.block {
            return unknown_dependence(direction, false);
        }
        let direction = direction_at_distance(left, right, distance).unwrap_or(direction);
        dependence_result(direction, DependenceRelation::SameIteration, Some(0))
    } else {
        let direction = direction_at_distance(left, right, distance).unwrap_or(direction);
        dependence_result(direction, DependenceRelation::LoopCarried, Some(distance))
    }
}

const MAX_DEPENDENCE_TERMS: usize = 32;

fn default_direction(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
) -> Option<DependenceDirection> {
    match (left.kind, right.kind) {
        (LoopMemoryAccessKind::Read, LoopMemoryAccessKind::Read) => None,
        (LoopMemoryAccessKind::Write, LoopMemoryAccessKind::Write) => {
            Some(DependenceDirection::Waw)
        }
        _ => {
            let (first, second) =
                if left.block == right.block && left.instruction_order > right.instruction_order {
                    (right.kind, left.kind)
                } else {
                    (left.kind, right.kind)
                };
            match (first, second) {
                (LoopMemoryAccessKind::Write, LoopMemoryAccessKind::Read) => {
                    Some(DependenceDirection::Raw)
                }
                (LoopMemoryAccessKind::Read, LoopMemoryAccessKind::Write) => {
                    Some(DependenceDirection::War)
                }
                _ => None,
            }
        }
    }
}

fn direction_at_distance(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
    distance: i64,
) -> Option<DependenceDirection> {
    if left.kind == LoopMemoryAccessKind::Write && right.kind == LoopMemoryAccessKind::Write {
        return Some(DependenceDirection::Waw);
    }
    let left_precedes_right = if distance == 0 {
        if left.block != right.block {
            return None;
        }
        left.instruction_order <= right.instruction_order
    } else {
        distance > 0
    };
    let (first, second) = if left_precedes_right {
        (left.kind, right.kind)
    } else {
        (right.kind, left.kind)
    };
    match (first, second) {
        (LoopMemoryAccessKind::Write, LoopMemoryAccessKind::Read) => Some(DependenceDirection::Raw),
        (LoopMemoryAccessKind::Read, LoopMemoryAccessKind::Write) => Some(DependenceDirection::War),
        _ => None,
    }
}

fn dependence_result(
    direction: DependenceDirection,
    relation: DependenceRelation,
    distance: Option<i64>,
) -> MemoryDependenceResult {
    MemoryDependenceResult {
        dependences: vec![MemoryDependence {
            direction,
            relation,
            distance,
        }],
        budget_exhausted: false,
    }
}

fn unknown_dependence(
    direction: DependenceDirection,
    budget_exhausted: bool,
) -> MemoryDependenceResult {
    MemoryDependenceResult {
        dependences: vec![MemoryDependence {
            direction,
            relation: DependenceRelation::Unknown,
            distance: None,
        }],
        budget_exhausted,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopDependencePair {
    pub left: crate::InstructionId,
    pub right: crate::InstructionId,
    pub kind: LoopDependenceKind,
    pub distance: Option<i64>,
    pub predicate: Option<TotalVersionPredicate>,
    pub relations: Vec<MemoryDependence>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopDependenceAnalysis {
    pub pairs: Vec<LoopDependencePair>,
    pub reductions: Vec<LoopReduction>,
    pub all_writes_classified: bool,
    pub budget_exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopReduction {
    pub value: ValueId,
    pub operation: MirBinaryOp,
    pub updates: Vec<InstructionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopLegalityAnalysis {
    pub eligible: bool,
    pub fallback_reasons: Vec<LoopFallbackReason>,
    pub dependences: LoopDependenceAnalysis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalVersionPredicate {
    pub address_bits: u8,
    pub conjuncts: Vec<VersionPredicateConjunct>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionPredicateConjunct {
    TripThreshold {
        trip_count: ValueId,
        minimum: u32,
    },
    Divisible {
        value: ValueId,
        divisor: u32,
    },
    PowerOfTwoAlignment {
        address: ValueId,
        alignment: u32,
    },
    AddressIntervalsDisjoint {
        left: ValueId,
        left_count: ValueId,
        left_element_bytes: u32,
        right: ValueId,
        right_count: ValueId,
        right_element_bytes: u32,
    },
}

impl TotalVersionPredicate {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.address_bits, 32 | 64) {
            return Err("version predicate address width must be 32 or 64".to_string());
        }
        if self.conjuncts.is_empty() || self.conjuncts.len() > 4 {
            return Err("version predicate must contain one to four conjuncts".to_string());
        }
        for conjunct in &self.conjuncts {
            match conjunct {
                VersionPredicateConjunct::TripThreshold { minimum, .. } if *minimum == 0 => {
                    return Err("trip threshold must be positive".to_string());
                }
                VersionPredicateConjunct::Divisible { divisor, .. } if *divisor == 0 => {
                    return Err("divisibility predicate divisor must be positive".to_string());
                }
                VersionPredicateConjunct::PowerOfTwoAlignment { alignment, .. }
                    if !alignment.is_power_of_two() =>
                {
                    return Err("alignment predicate must name a power of two".to_string());
                }
                VersionPredicateConjunct::AddressIntervalsDisjoint {
                    left_element_bytes,
                    right_element_bytes,
                    ..
                } if *left_element_bytes == 0 || *right_element_bytes == 0 => {
                    return Err("address interval element width must be positive".to_string());
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn evaluate(&self, values: &BTreeMap<ValueId, u64>) -> bool {
        if self.validate().is_err() {
            return false;
        }
        self.conjuncts
            .iter()
            .all(|conjunct| evaluate_conjunct(conjunct, values, self.address_bits))
    }
}

pub fn analyze_loop_dependences(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
) -> Result<LoopDependenceAnalysis, String> {
    analyze_loop_dependences_for_address_bits(
        function,
        descriptor,
        facts,
        Some(usize::BITS as u8),
        &BTreeSet::new(),
    )
}

fn analyze_loop_dependences_for_address_bits(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
    address_bits: Option<u8>,
    no_wrap_accesses: &BTreeSet<InstructionId>,
) -> Result<LoopDependenceAnalysis, String> {
    let accesses = analyze_affine_loop_accesses(function, descriptor, facts)?;
    let regions = analyze_regions(function, facts).map_err(|error| error.message)?;
    let units = function
        .blocks
        .iter()
        .fold(function.params.len(), |count, block| {
            count + block.params.len() + block.instructions.len() + 1
        });
    let units = u32::try_from(units).unwrap_or(u32::MAX);
    let max_pairs = units
        .saturating_mul(32)
        .saturating_add(128)
        .min(MAX_DEPENDENCE_PAIRS);
    let source_writes = function
        .blocks
        .iter()
        .filter(|block| descriptor.blocks.binary_search(&block.id).is_ok())
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
        .map(|instruction| instruction.id)
        .collect::<BTreeSet<_>>();
    let access_writes = accesses
        .accesses
        .iter()
        .filter(|access| access.kind == LoopMemoryAccessKind::Write)
        .map(|access| access.instruction)
        .collect::<BTreeSet<_>>();
    let mut pairs = Vec::new();
    let mut classified_self_writes = BTreeSet::new();
    let mut budget_exhausted = accesses.budget_exhausted;
    let mut examined_pairs = 0_u32;

    'all_pairs: {
        for access in accesses
            .accesses
            .iter()
            .filter(|access| access.kind == LoopMemoryAccessKind::Write)
        {
            if examined_pairs >= max_pairs {
                budget_exhausted = true;
                break 'all_pairs;
            }
            examined_pairs += 1;
            let wrap = wrap_facts_for(access, access, no_wrap_accesses);
            let result = query_memory_dependence(
                access,
                access,
                AliasKind::MustAlias,
                wrap,
                DependenceQueryBudget::default(),
            );
            let kind = aggregate_dependence_kind(&result, None);
            if kind != LoopDependenceKind::Unknown {
                classified_self_writes.insert(access.instruction);
            }
            pairs.push(LoopDependencePair {
                left: access.instruction,
                right: access.instruction,
                kind,
                distance: result
                    .dependences
                    .first()
                    .and_then(|dependence| dependence.distance),
                predicate: None,
                relations: result.dependences,
            });
        }
        for (left_index, left) in accesses.accesses.iter().enumerate() {
            for right in accesses.accesses.iter().skip(left_index + 1) {
                if examined_pairs >= max_pairs {
                    budget_exhausted = true;
                    break 'all_pairs;
                }
                examined_pairs += 1;
                if left.kind == LoopMemoryAccessKind::Read
                    && right.kind == LoopMemoryAccessKind::Read
                {
                    pairs.push(LoopDependencePair {
                        left: left.instruction,
                        right: right.instruction,
                        kind: LoopDependenceKind::ReadRead,
                        distance: None,
                        predicate: None,
                        relations: Vec::new(),
                    });
                    continue;
                }
                let alias = if left.region == right.region {
                    AliasKind::MustAlias
                } else {
                    query_alias(&regions, left.region, right.region).kind
                };
                let result = query_memory_dependence(
                    left,
                    right,
                    alias,
                    wrap_facts_for(left, right, no_wrap_accesses),
                    DependenceQueryBudget::default(),
                );
                let mut predicate = None;
                let mut kind = aggregate_dependence_kind(&result, None);
                if alias == AliasKind::MayAlias {
                    predicate = runtime_disambiguation(left, right, address_bits);
                    if predicate.is_some() {
                        kind = LoopDependenceKind::RuntimeGuarded;
                    }
                }
                let distance = result
                    .dependences
                    .first()
                    .and_then(|dependence| dependence.distance);
                pairs.push(LoopDependencePair {
                    left: left.instruction,
                    right: right.instruction,
                    kind,
                    distance,
                    predicate,
                    relations: result.dependences,
                });
            }
        }
    }
    pairs.sort_by_key(|pair| (pair.left, pair.right));
    let pairs_classified = pairs
        .iter()
        .all(|pair| !matches!(pair.kind, LoopDependenceKind::Unknown));
    let reductions = if budget_exhausted {
        Vec::new()
    } else {
        match find_modular_reductions(function, descriptor) {
            Ok(reductions) => reductions,
            Err(()) => {
                budget_exhausted = true;
                Vec::new()
            }
        }
    };
    Ok(LoopDependenceAnalysis {
        all_writes_classified: !budget_exhausted
            && source_writes == access_writes
            && source_writes == classified_self_writes
            && (source_writes.is_empty() || accesses.rejected_instructions.is_empty())
            && pairs_classified,
        pairs,
        reductions,
        budget_exhausted,
    })
}

const MAX_DEPENDENCE_PAIRS: u32 = 8192;

fn wrap_facts_for(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
    no_wrap_accesses: &BTreeSet<InstructionId>,
) -> DependenceWrapFacts {
    let mut facts = DependenceWrapFacts::from_accesses(left, right);
    if no_wrap_accesses.contains(&left.instruction) {
        facts.left = IndexWrapProof::ProvenNoWrap;
    }
    if no_wrap_accesses.contains(&right.instruction) {
        facts.right = IndexWrapProof::ProvenNoWrap;
    }
    facts
}

fn aggregate_dependence_kind(
    result: &MemoryDependenceResult,
    predicate: Option<&TotalVersionPredicate>,
) -> LoopDependenceKind {
    if predicate.is_some() {
        return LoopDependenceKind::RuntimeGuarded;
    }
    if result.budget_exhausted
        || result
            .dependences
            .iter()
            .any(|dependence| dependence.relation == DependenceRelation::Unknown)
    {
        return LoopDependenceKind::Unknown;
    }
    if result
        .dependences
        .iter()
        .any(|dependence| dependence.relation == DependenceRelation::LoopCarried)
    {
        LoopDependenceKind::Dependent
    } else if result
        .dependences
        .iter()
        .any(|dependence| dependence.relation == DependenceRelation::SameIteration)
    {
        LoopDependenceKind::SameIteration
    } else {
        LoopDependenceKind::Independent
    }
}

pub fn analyze_loop_legality(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
) -> Result<LoopLegalityAnalysis, String> {
    analyze_loop_legality_for_address_bits(
        function,
        descriptor,
        facts,
        Some(usize::BITS as u8),
        &BTreeSet::new(),
    )
}

pub(crate) fn analyze_loop_legality_for_profile(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
    profile: &crate::KirTargetProfile,
) -> Result<LoopLegalityAnalysis, String> {
    let address_bits = match profile.layout() {
        crate::KirProfileLayout::Known {
            pointer_width_bits, ..
        } => u8::try_from(pointer_width_bits)
            .ok()
            .filter(|bits| matches!(bits, 32 | 64)),
        crate::KirProfileLayout::PortableUnknown => None,
    };
    analyze_loop_legality_for_address_bits(
        function,
        descriptor,
        facts,
        address_bits,
        &BTreeSet::new(),
    )
}

pub(crate) fn analyze_loop_legality_for_profile_with_index_facts(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
    profile: &crate::KirTargetProfile,
    no_wrap_accesses: &BTreeSet<InstructionId>,
) -> Result<LoopLegalityAnalysis, String> {
    let address_bits = match profile.layout() {
        crate::KirProfileLayout::Known {
            pointer_width_bits, ..
        } => u8::try_from(pointer_width_bits)
            .ok()
            .filter(|bits| matches!(bits, 32 | 64)),
        crate::KirProfileLayout::PortableUnknown => None,
    };
    analyze_loop_legality_for_address_bits(
        function,
        descriptor,
        facts,
        address_bits,
        no_wrap_accesses,
    )
}

fn analyze_loop_legality_for_address_bits(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
    address_bits: Option<u8>,
    no_wrap_accesses: &BTreeSet<InstructionId>,
) -> Result<LoopLegalityAnalysis, String> {
    let dependences = analyze_loop_dependences_for_address_bits(
        function,
        descriptor,
        facts,
        address_bits,
        no_wrap_accesses,
    )?;
    let loop_blocks = descriptor.blocks.iter().copied().collect::<BTreeSet<_>>();
    let has_ordered_effect = function
        .blocks
        .iter()
        .filter(|block| loop_blocks.contains(&block.id))
        .flat_map(|block| &block.instructions)
        .any(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Guard { .. }
                    | KirInstructionKind::Call { .. }
                    | KirInstructionKind::RuntimeCall { .. }
            ) || instruction.effect.as_ref().is_some_and(|effect| {
                matches!(
                    effect.kind,
                    KirEffectKind::MayFail | KirEffectKind::Runtime | KirEffectKind::Call
                )
            })
        });
    let mut fallback_reasons = Vec::new();
    if !descriptor.innermost {
        fallback_reasons.push(LoopFallbackReason::NotInnermost);
    }
    if matches!(descriptor.trip_count, LoopTripCount::Unknown) {
        fallback_reasons.push(LoopFallbackReason::NonCountableTrip);
    }
    if has_ordered_effect {
        fallback_reasons.push(LoopFallbackReason::OrderedEffect);
    }
    if dependences.budget_exhausted {
        fallback_reasons.push(LoopFallbackReason::BudgetExhausted);
    }
    if dependences
        .pairs
        .iter()
        .any(|pair| pair.kind == LoopDependenceKind::Unknown)
        || !dependences.all_writes_classified
    {
        fallback_reasons.push(LoopFallbackReason::UnknownMemoryDependence);
    }
    if dependences
        .pairs
        .iter()
        .any(|pair| pair.kind == LoopDependenceKind::Dependent)
    {
        fallback_reasons.push(LoopFallbackReason::LoopCarriedDependence);
    }
    fallback_reasons.sort_unstable();
    fallback_reasons.dedup();
    Ok(LoopLegalityAnalysis {
        eligible: fallback_reasons.is_empty(),
        fallback_reasons,
        dependences,
    })
}

fn find_modular_reductions(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
) -> Result<Vec<LoopReduction>, ()> {
    let mut budget = AffineAnalysisBudget::for_function(function);
    let forwarding_index = ForwardingIndex::new(function, &mut budget).ok_or(())?;
    let (Some(header), Some(latch)) = (
        function
            .blocks
            .iter()
            .find(|block| block.id == descriptor.header),
        function
            .blocks
            .iter()
            .find(|block| Some(block.id) == descriptor.latch),
    ) else {
        return Ok(Vec::new());
    };
    let crate::KirTerminator::Jump { edge } = &latch.terminator else {
        return Ok(Vec::new());
    };
    let mut reductions = Vec::new();
    for (param_index, param) in header.params.iter().enumerate() {
        if !budget.spend() {
            return Err(());
        }
        if descriptor
            .induction
            .as_ref()
            .is_some_and(|induction| induction.value == param.value)
            || !matches!(
                param.type_node.as_scalar(),
                Some(MirType::Primitive(
                    MirPrimitiveTypeName::I32
                        | MirPrimitiveTypeName::I64
                        | MirPrimitiveTypeName::U32
                        | MirPrimitiveTypeName::U64
                ))
            )
        {
            continue;
        }
        let Some(backedge) = edge.args.get(param_index).copied() else {
            continue;
        };
        let Some(leaves) = forwarding_leaves(&forwarding_index, backedge, &mut budget)? else {
            continue;
        };
        let mut operation = None;
        let mut updates = Vec::new();
        let mut valid = true;
        for value in &leaves {
            if !budget.spend() {
                return Err(());
            }
            let Some(instruction) = forwarding_index.definition(*value) else {
                valid = false;
                break;
            };
            let KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = instruction.kind
            else {
                valid = false;
                break;
            };
            let left_is_origin =
                forwarding_leaves_until(&forwarding_index, left, param.value, &mut budget)?
                    .is_some_and(|left_leaves| left_leaves == BTreeSet::from([param.value]));
            let right_is_origin = if left_is_origin {
                false
            } else {
                forwarding_leaves_until(&forwarding_index, right, param.value, &mut budget)?
                    .is_some_and(|right_leaves| right_leaves == BTreeSet::from([param.value]))
            };
            if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Mul)
                || !(left_is_origin || right_is_origin)
                || operation.is_some_and(|expected| expected != op)
            {
                valid = false;
                break;
            }
            operation = Some(op);
            updates.push(instruction.id);
        }
        if valid && !updates.is_empty() {
            updates.sort_unstable();
            reductions.push(LoopReduction {
                value: param.value,
                operation: operation.expect("nonempty reduction updates set an operation"),
                updates,
            });
        }
    }
    reductions.sort_by_key(|reduction| reduction.value);
    Ok(reductions)
}

fn runtime_disambiguation(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
    address_bits: Option<u8>,
) -> Option<TotalVersionPredicate> {
    if !left.unit_stride
        || !right.unit_stride
        || !left.legacy_projection_exact
        || !right.legacy_projection_exact
        || !left.index_no_wrap
        || !right.index_no_wrap
        || !left.slice_base
        || !right.slice_base
        || left.invariant_offset.is_some()
        || right.invariant_offset.is_some()
        || left.bias != 0.into()
        || right.bias != 0.into()
        || left.induction != right.induction
        || left.trip_start != 0.into()
        || right.trip_start != 0.into()
        || left.trip_bound != right.trip_bound
    {
        return None;
    }
    let address_bits = address_bits.filter(|bits| matches!(bits, 32 | 64))?;
    let predicate = TotalVersionPredicate {
        address_bits,
        conjuncts: vec![VersionPredicateConjunct::AddressIntervalsDisjoint {
            left: left.base,
            left_count: loop_bound_value(left)?,
            left_element_bytes: left.element_bytes,
            right: right.base,
            right_count: loop_bound_value(right)?,
            right_element_bytes: right.element_bytes,
        }],
    };
    predicate.validate().ok()?;
    Some(predicate)
}

fn loop_bound_value(access: &AffineMemoryAccess) -> Option<ValueId> {
    Some(access.trip_bound)
}

fn evaluate_conjunct(
    conjunct: &VersionPredicateConjunct,
    values: &BTreeMap<ValueId, u64>,
    address_bits: u8,
) -> bool {
    let maximum = if address_bits == 32 {
        u64::from(u32::MAX)
    } else {
        u64::MAX
    };
    let value = |id: ValueId| values.get(&id).copied().filter(|value| *value <= maximum);
    match conjunct {
        VersionPredicateConjunct::TripThreshold {
            trip_count,
            minimum,
        } => value(*trip_count).is_some_and(|value| value >= u64::from(*minimum)),
        VersionPredicateConjunct::Divisible {
            value: input,
            divisor,
        } => value(*input).is_some_and(|value| value % u64::from(*divisor) == 0),
        VersionPredicateConjunct::PowerOfTwoAlignment { address, alignment } => {
            value(*address).is_some_and(|value| value & (u64::from(*alignment) - 1) == 0)
        }
        VersionPredicateConjunct::AddressIntervalsDisjoint {
            left,
            left_count,
            left_element_bytes,
            right,
            right_count,
            right_element_bytes,
        } => {
            let (Some(left_count), Some(right_count)) = (value(*left_count), value(*right_count))
            else {
                return false;
            };
            if left_count == 0 || right_count == 0 {
                return true;
            }
            let (Some(left), Some(right)) = (value(*left), value(*right)) else {
                return false;
            };
            let Some(left_bytes) = left_count.checked_mul(u64::from(*left_element_bytes)) else {
                return false;
            };
            let Some(right_bytes) = right_count.checked_mul(u64::from(*right_element_bytes)) else {
                return false;
            };
            let Some(left_end) = left.checked_add(left_bytes).filter(|end| *end <= maximum) else {
                return false;
            };
            let Some(right_end) = right.checked_add(right_bytes).filter(|end| *end <= maximum)
            else {
                return false;
            };
            left_end <= right || right_end <= left
        }
    }
}

#[must_use]
pub fn regions_in_dependence_pairs(accesses: &[AffineMemoryAccess]) -> BTreeSet<MemoryRegionId> {
    accesses.iter().map(|access| access.region).collect()
}
