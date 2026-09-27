use std::collections::{BTreeMap, BTreeSet};

use crate::{
    FactArena, InstructionId, KirArithmeticSemantics, KirEffectKind, KirFunction,
    KirInstructionKind, MemoryRegionId, MirBinaryOp, MirPrimitiveTypeName, MirType, ValueId,
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
    ModularReduction,
    Dependent,
    RuntimeGuarded,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopDependencePair {
    pub left: crate::InstructionId,
    pub right: crate::InstructionId,
    pub kind: LoopDependenceKind,
    pub distance: Option<i64>,
    pub predicate: Option<TotalVersionPredicate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopDependenceAnalysis {
    pub pairs: Vec<LoopDependencePair>,
    pub reductions: Vec<LoopReduction>,
    pub all_writes_classified: bool,
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
    let accesses = analyze_affine_loop_accesses(function, descriptor, facts)?;
    let regions = analyze_regions(function, facts).map_err(|error| error.message)?;
    let mut pairs = Vec::new();
    for (left_index, left) in accesses.accesses.iter().enumerate() {
        for right in accesses.accesses.iter().skip(left_index + 1) {
            let has_write = left.kind == LoopMemoryAccessKind::Write
                || right.kind == LoopMemoryAccessKind::Write;
            let (kind, distance, predicate) = if !has_write {
                (LoopDependenceKind::ReadRead, None, None)
            } else if left.region == right.region {
                same_region_dependence(left, right)
            } else {
                match query_alias(&regions, left.region, right.region).kind {
                    AliasKind::NoAlias => (LoopDependenceKind::Independent, None, None),
                    AliasKind::MustAlias => same_region_dependence(left, right),
                    AliasKind::MayAlias => runtime_disambiguation(left, right)
                        .map_or((LoopDependenceKind::Unknown, None, None), |predicate| {
                            (LoopDependenceKind::RuntimeGuarded, None, Some(predicate))
                        }),
                }
            };
            pairs.push(LoopDependencePair {
                left: left.instruction,
                right: right.instruction,
                kind,
                distance,
                predicate,
            });
        }
    }
    pairs.sort_by_key(|pair| (pair.left, pair.right));
    let writes = accesses
        .accesses
        .iter()
        .filter(|access| access.kind == LoopMemoryAccessKind::Write)
        .map(|access| access.instruction)
        .collect::<BTreeSet<_>>();
    let classified_writes = pairs
        .iter()
        .filter(|pair| {
            !matches!(
                pair.kind,
                LoopDependenceKind::Unknown | LoopDependenceKind::ReadRead
            )
        })
        .flat_map(|pair| [pair.left, pair.right])
        .filter(|instruction| writes.contains(instruction))
        .collect::<BTreeSet<_>>();
    let reductions = find_modular_reductions(function, descriptor);
    Ok(LoopDependenceAnalysis {
        all_writes_classified: writes.is_subset(&classified_writes)
            || (writes.len() == 1 && accesses.accesses.len() == 1),
        pairs,
        reductions,
    })
}

pub fn analyze_loop_legality(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
) -> Result<LoopLegalityAnalysis, String> {
    let dependences = analyze_loop_dependences(function, descriptor, facts)?;
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
) -> Vec<LoopReduction> {
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
        return Vec::new();
    };
    let crate::KirTerminator::Jump { edge } = &latch.terminator else {
        return Vec::new();
    };
    let mut reductions = Vec::new();
    for (index, param) in header.params.iter().enumerate() {
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
        let Some(backedge) = edge.args.get(index).copied() else {
            continue;
        };
        let Some(leaves) = forwarding_leaves(function, backedge) else {
            continue;
        };
        let mut operation = None;
        let mut updates = Vec::new();
        let valid = leaves.iter().all(|value| {
            let Some(instruction) = defining_instruction(function, *value) else {
                return false;
            };
            let KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = instruction.kind
            else {
                return false;
            };
            if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Mul)
                || !(forwarded_from(function, left, param.value)
                    || forwarded_from(function, right, param.value))
                || operation.is_some_and(|expected| expected != op)
            {
                return false;
            }
            operation = Some(op);
            updates.push(instruction.id);
            true
        });
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
    reductions
}

fn forwarding_leaves(function: &KirFunction, value: ValueId) -> Option<BTreeSet<ValueId>> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut leaves = BTreeSet::new();
    while let Some(value) = pending.pop() {
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
                return None;
            }
            pending.extend(incoming);
        } else if let Some(crate::KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = defining_instruction(function, value)
        {
            pending.push(*value);
        } else {
            leaves.insert(value);
        }
    }
    Some(leaves)
}

fn forwarded_from(function: &KirFunction, value: ValueId, origin: ValueId) -> bool {
    forwarding_leaves_until(function, value, origin)
        .is_some_and(|leaves| leaves == BTreeSet::from([origin]))
}

fn forwarding_leaves_until(
    function: &KirFunction,
    value: ValueId,
    origin: ValueId,
) -> Option<BTreeSet<ValueId>> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut leaves = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if !visited.insert(value) {
            continue;
        }
        if value == origin {
            leaves.insert(value);
        } else if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            let incoming = incoming_values(function, block, index);
            if incoming.is_empty() {
                return None;
            }
            pending.extend(incoming);
        } else if let Some(crate::KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = defining_instruction(function, value)
        {
            pending.push(*value);
        } else {
            leaves.insert(value);
        }
    }
    Some(leaves)
}

fn incoming_values(function: &KirFunction, target: crate::BlockId, index: usize) -> Vec<ValueId> {
    function
        .blocks
        .iter()
        .flat_map(|block| match &block.terminator {
            crate::KirTerminator::Return { .. } => Vec::new(),
            crate::KirTerminator::Jump { edge } => vec![edge],
            crate::KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
        })
        .filter(|edge| edge.target == target)
        .filter_map(|edge| edge.args.get(index).copied())
        .collect()
}

fn defining_instruction(function: &KirFunction, value: ValueId) -> Option<&crate::KirInstruction> {
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

fn same_region_dependence(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
) -> (
    LoopDependenceKind,
    Option<i64>,
    Option<TotalVersionPredicate>,
) {
    if left.base != right.base
        || left.coefficient != right.coefficient
        || left.invariant_offset != right.invariant_offset
        || left.element_bytes != right.element_bytes
    {
        return (LoopDependenceKind::Unknown, None, None);
    }
    let distance = (&right.bias - &left.bias).to_string().parse::<i64>().ok();
    if distance == Some(0) && left.unit_stride && right.unit_stride {
        (LoopDependenceKind::Independent, distance, None)
    } else {
        (LoopDependenceKind::Dependent, distance, None)
    }
}

fn runtime_disambiguation(
    left: &AffineMemoryAccess,
    right: &AffineMemoryAccess,
) -> Option<TotalVersionPredicate> {
    if !left.unit_stride
        || !right.unit_stride
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
    let predicate = TotalVersionPredicate {
        address_bits: usize::BITS as u8,
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
