use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    FactId, KirFunction, KirInstructionKind, KirMemoryRegionOrigin, MemoryRegionId, MirType,
    ValueId, print_mir_type,
};

use super::super::facts::{ContractFactPredicate, FactArena, FactPredicate, FactScope};

/// Symbolic `[start * sizeof(T), end * sizeof(T))` byte interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolicByteInterval {
    pub start: ValueId,
    pub end: ValueId,
    pub element_type: MirType,
    pub exact_start: Option<BigInt>,
    pub exact_end: Option<BigInt>,
}

impl SymbolicByteInterval {
    #[must_use]
    pub fn scale_description(&self) -> String {
        format!("sizeof({})", print_mir_type(&self.element_type))
    }

    #[must_use]
    pub fn is_proven_empty(&self) -> bool {
        self.exact_start.is_some() && self.exact_start == self.exact_end
    }

    fn is_proven_well_formed(&self) -> bool {
        matches!(
            (&self.exact_start, &self.exact_end),
            (Some(start), Some(end)) if start <= end
        )
    }
}

/// Canonical descriptor for one stable KIR memory region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionDescriptor {
    pub id: MemoryRegionId,
    pub root: MemoryRegionId,
    pub parent: Option<MemoryRegionId>,
    pub partition: MemoryRegionId,
    pub origin: KirMemoryRegionOrigin,
    pub byte_interval: Option<SymbolicByteInterval>,
}

/// Per-function region identity and pairwise contract evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionAnalysis {
    function: crate::FunctionId,
    descriptors: BTreeMap<MemoryRegionId, RegionDescriptor>,
    value_regions: BTreeMap<ValueId, MemoryRegionId>,
    noalias: BTreeMap<(MemoryRegionId, MemoryRegionId), FactId>,
}

impl RegionAnalysis {
    #[must_use]
    pub const fn function(&self) -> crate::FunctionId {
        self.function
    }

    #[must_use]
    pub fn descriptor(&self, region: MemoryRegionId) -> Option<&RegionDescriptor> {
        self.descriptors.get(&region)
    }

    #[must_use]
    pub fn root(&self, region: MemoryRegionId) -> Option<MemoryRegionId> {
        self.descriptor(region).map(|descriptor| descriptor.root)
    }

    #[must_use]
    pub fn partition(&self, region: MemoryRegionId) -> Option<MemoryRegionId> {
        self.descriptor(region)
            .map(|descriptor| descriptor.partition)
    }

    #[must_use]
    pub fn region_for_value(&self, value: ValueId) -> Option<MemoryRegionId> {
        self.value_regions.get(&value).copied()
    }

    pub(super) fn noalias_fact(
        &self,
        left: MemoryRegionId,
        right: MemoryRegionId,
    ) -> Option<FactId> {
        self.noalias.get(&ordered_pair(left, right)).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionAnalysisError {
    pub message: String,
}

/// Reconstructs region roots and symbolic interval evidence from verified KIR.
pub fn analyze_regions(
    function: &KirFunction,
    facts: Option<&FactArena>,
) -> Result<RegionAnalysis, RegionAnalysisError> {
    let constants = integer_constants(function);
    let parents = function
        .regions
        .iter()
        .map(|region| (region.id, region.parent))
        .collect::<BTreeMap<_, _>>();
    let mut descriptors = BTreeMap::new();
    let mut value_regions = BTreeMap::new();
    for region in &function.regions {
        let root = find_root(region.id, &parents)?;
        match region.origin {
            KirMemoryRegionOrigin::Parameter(value)
            | KirMemoryRegionOrigin::RawSlice(value)
            | KirMemoryRegionOrigin::Subslice(value) => {
                value_regions.insert(value, region.id);
            }
            KirMemoryRegionOrigin::Conservative => {}
        }
        descriptors.insert(
            region.id,
            RegionDescriptor {
                id: region.id,
                root,
                parent: region.parent,
                partition: region.partition,
                origin: region.origin.clone(),
                byte_interval: region
                    .byte_interval
                    .as_ref()
                    .map(|interval| SymbolicByteInterval {
                        start: interval.start,
                        end: interval.end,
                        element_type: interval.element_type.clone(),
                        exact_start: constants.get(&interval.start).cloned(),
                        exact_end: constants.get(&interval.end).cloned(),
                    }),
            },
        );
    }
    propagate_value_regions(function, &mut value_regions);
    let mut noalias = BTreeMap::new();
    if let Some(facts) = facts {
        for fact in facts.facts() {
            if !fact_applies_to_function(&fact.scope, function.id) {
                continue;
            }
            let FactPredicate::Contract(ContractFactPredicate::NoAlias { left, right }) =
                &fact.predicate
            else {
                continue;
            };
            let (Some(left), Some(right)) = (
                value_regions.get(left).copied(),
                value_regions.get(right).copied(),
            ) else {
                continue;
            };
            let (Some(left), Some(right)) = (
                descriptors.get(&left).map(|descriptor| descriptor.root),
                descriptors.get(&right).map(|descriptor| descriptor.root),
            ) else {
                continue;
            };
            noalias.insert(ordered_pair(left, right), fact.id);
        }
    }
    Ok(RegionAnalysis {
        function: function.id,
        descriptors,
        value_regions,
        noalias,
    })
}

fn integer_constants(function: &KirFunction) -> BTreeMap<ValueId, BigInt> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| {
            let KirInstructionKind::ConstInt { value } = &instruction.kind else {
                return None;
            };
            let result = instruction.results.first()?;
            value
                .parse::<BigInt>()
                .ok()
                .map(|value| (result.value, value))
        })
        .collect()
}

fn find_root(
    region: MemoryRegionId,
    parents: &BTreeMap<MemoryRegionId, Option<MemoryRegionId>>,
) -> Result<MemoryRegionId, RegionAnalysisError> {
    let mut current = region;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current) {
            return Err(RegionAnalysisError {
                message: format!("memory region r{} has a parent cycle", region.index()),
            });
        }
        match parents.get(&current) {
            Some(Some(parent)) => current = *parent,
            Some(None) => return Ok(current),
            None => {
                return Err(RegionAnalysisError {
                    message: format!(
                        "memory region r{} names missing parent r{}",
                        region.index(),
                        current.index()
                    ),
                });
            }
        }
    }
}

fn propagate_value_regions(
    function: &KirFunction,
    value_regions: &mut BTreeMap<ValueId, MemoryRegionId>,
) {
    let mut changed = true;
    while changed {
        changed = false;
        for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
            let Some(result) = instruction.results.first().map(|result| result.value) else {
                continue;
            };
            let source = match instruction.kind {
                KirInstructionKind::Copy { value } => Some(value),
                KirInstructionKind::SliceData { slice } => Some(slice),
                _ => None,
            };
            if let Some(region) = source.and_then(|source| value_regions.get(&source).copied())
                && value_regions.insert(result, region) != Some(region)
            {
                changed = true;
            }
        }
    }
}

fn fact_applies_to_function(scope: &FactScope, function: crate::FunctionId) -> bool {
    match scope {
        FactScope::FunctionEntry(owner)
        | FactScope::Block {
            function: owner, ..
        } => *owner == function,
        FactScope::CalleeInstance { callee, .. } => *callee == function,
        FactScope::InlineClone {
            function: owner, ..
        } => *owner == function,
    }
}

pub(super) fn intervals_are_disjoint(
    left: &SymbolicByteInterval,
    right: &SymbolicByteInterval,
) -> bool {
    if left.element_type != right.element_type
        || !left.is_proven_well_formed()
        || !right.is_proven_well_formed()
    {
        return false;
    }
    matches!(
        (
            &left.exact_start,
            &left.exact_end,
            &right.exact_start,
            &right.exact_end,
        ),
        (Some(left_start), Some(left_end), Some(right_start), Some(right_end))
            if left_end <= right_start || right_end <= left_start
    )
}

pub(super) fn ordered_pair(
    left: MemoryRegionId,
    right: MemoryRegionId,
) -> (MemoryRegionId, MemoryRegionId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}
