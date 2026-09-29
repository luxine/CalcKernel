use crate::{BlockId, InstructionId};

use super::VectorizationPlan;

/// Source branch to emitted select identity for one closed SIMD decision tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecisionTreeSelectMapping {
    pub source_branch: BlockId,
    pub vector_select: InstructionId,
    pub unroll_index: u8,
}

/// Closed vector plan plus the control decisions eliminated by if-conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionTreeVectorPlan {
    pub vector: VectorizationPlan,
    pub selects: Vec<DecisionTreeSelectMapping>,
}
