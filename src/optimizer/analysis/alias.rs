use crate::{FactId, MemoryRegionId};

use super::{RegionAnalysis, intervals_are_disjoint};

/// Conservative three-valued alias result shared by every memory consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    NoAlias,
    MayAlias,
    MustAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasQueryResult {
    pub kind: AliasKind,
    pub fact: Option<FactId>,
}

#[must_use]
pub fn query_alias(
    analysis: &RegionAnalysis,
    left: MemoryRegionId,
    right: MemoryRegionId,
) -> AliasQueryResult {
    if left == right {
        return AliasQueryResult {
            kind: AliasKind::MustAlias,
            fact: None,
        };
    }
    let (Some(left_descriptor), Some(right_descriptor)) =
        (analysis.descriptor(left), analysis.descriptor(right))
    else {
        return AliasQueryResult {
            kind: AliasKind::MayAlias,
            fact: None,
        };
    };
    if left_descriptor
        .byte_interval
        .as_ref()
        .is_some_and(super::SymbolicByteInterval::is_proven_empty)
        || right_descriptor
            .byte_interval
            .as_ref()
            .is_some_and(super::SymbolicByteInterval::is_proven_empty)
    {
        return AliasQueryResult {
            kind: AliasKind::NoAlias,
            fact: None,
        };
    }
    if left_descriptor.root == right_descriptor.root
        && left_descriptor
            .byte_interval
            .as_ref()
            .zip(right_descriptor.byte_interval.as_ref())
            .is_some_and(|(left, right)| intervals_are_disjoint(left, right))
    {
        return AliasQueryResult {
            kind: AliasKind::NoAlias,
            fact: None,
        };
    }
    if let Some(fact) = analysis.noalias_fact(left_descriptor.root, right_descriptor.root) {
        return AliasQueryResult {
            kind: AliasKind::NoAlias,
            fact: Some(fact),
        };
    }
    AliasQueryResult {
        kind: AliasKind::MayAlias,
        fact: None,
    }
}
