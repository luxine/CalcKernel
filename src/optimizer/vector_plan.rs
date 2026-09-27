use crate::{
    BlockId, FunctionId, InstructionId, KirAlignmentClass, KirCostKey, KirCostSemantics,
    KirLaneType, KirOperationAvailability, KirProfileOperation, KirTargetProfile, LoopId,
    MemoryRegionId, ProofId, ValueId,
};

use super::SpecializationFact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirPreStateIdentity {
    pub function: FunctionId,
    pub kir_digest: String,
    pub profile_digest: String,
    pub evidence_generation: u32,
    pub frozen_kir_units: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorLaneMapping {
    pub lane: u16,
    pub scalar_iteration: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorOperationMapping {
    pub scalar: InstructionId,
    pub vector: InstructionId,
    pub unroll_index: u8,
    pub operation: KirProfileOperation,
    pub lane_type: KirLaneType,
    pub semantics: KirCostSemantics,
    pub alignment: KirAlignmentClass,
    pub lanes: Vec<VectorLaneMapping>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VectorMemoryAccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorMemoryGroup {
    pub region: MemoryRegionId,
    pub access: VectorMemoryAccessKind,
    pub scalar_instructions: Vec<InstructionId>,
    pub vector_instruction: InstructionId,
    pub unroll_index: u8,
    pub footprint_proof: ProofId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorPredicate {
    TripThreshold {
        trip_count: ValueId,
        minimum: u32,
        proof: ProofId,
    },
    Divisibility {
        value: ValueId,
        divisor: u32,
        proof: ProofId,
    },
    AddressNonOverlap {
        left: MemoryRegionId,
        right: MemoryRegionId,
        bytes: ValueId,
        proof: ProofId,
    },
    PowerOfTwoAlignment {
        value: ValueId,
        alignment: u16,
        proof: ProofId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorEpilogue {
    None,
    Scalar {
        start: ValueId,
        end: ValueId,
        coverage_proof: ProofId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KirCostEstimate {
    pub scalar: u32,
    pub transformed_body: u32,
    pub predicates: u32,
    pub epilogue: u32,
    pub total: u32,
}

impl KirCostEstimate {
    #[must_use]
    pub const fn new(scalar: u32, transformed_body: u32, predicates: u32, epilogue: u32) -> Self {
        Self {
            scalar,
            transformed_body,
            predicates,
            epilogue,
            total: transformed_body
                .saturating_add(predicates)
                .saturating_add(epilogue),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorPlanGrowth {
    pub original_units: u32,
    pub transformed_units: u32,
    pub module_before_units: u32,
    pub module_after_units: u32,
}

impl VectorPlanGrowth {
    #[must_use]
    pub const fn new(
        original_units: u32,
        transformed_units: u32,
        module_before_units: u32,
        module_after_units: u32,
    ) -> Self {
        Self {
            original_units,
            transformed_units,
            module_before_units,
            module_after_units,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorProofRoots {
    pub canonical_loop: ProofId,
    pub trip_partition: ProofId,
    pub lane_mapping: ProofId,
    pub operation_equivalence: ProofId,
    pub fallback_identity: ProofId,
    pub target_legality: ProofId,
    pub cost_and_budget: ProofId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorizationPlan {
    pub pre_state: KirPreStateIdentity,
    pub loop_id: LoopId,
    pub vf: u16,
    pub uf: u8,
    pub operations: Vec<VectorOperationMapping>,
    pub memory_groups: Vec<VectorMemoryGroup>,
    pub predicates: Vec<VectorPredicate>,
    pub epilogue: VectorEpilogue,
    pub cost: KirCostEstimate,
    pub growth: VectorPlanGrowth,
    pub proofs: VectorProofRoots,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpPlan {
    pub pre_state: KirPreStateIdentity,
    pub block: BlockId,
    pub root: InstructionId,
    pub lanes: u16,
    pub lane_type: KirLaneType,
    pub semantics: KirCostSemantics,
    pub scalar_instructions: Vec<InstructionId>,
    pub setup_instructions: Vec<InstructionId>,
    pub vector_instructions: Vec<InstructionId>,
    pub extracts: Vec<InstructionId>,
    pub operations: Vec<KirProfileOperation>,
    pub memory: Option<SlpMemoryPlan>,
    pub cost: KirCostEstimate,
    pub growth: VectorPlanGrowth,
    pub proof: SlpProofRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpMemoryPlan {
    pub left_loads: Vec<InstructionId>,
    pub right_loads: Vec<InstructionId>,
    pub stores: Vec<InstructionId>,
    pub vector_loads: Vec<InstructionId>,
    pub vector_store: InstructionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrollPlan {
    pub pre_state: KirPreStateIdentity,
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
    pub factor: u8,
    pub full: bool,
    pub trip_count: u32,
    pub remainder: u8,
    pub body_units: u32,
    pub o3_entry_module_units: u32,
    pub instruction_mapping: Vec<UnrollInstructionMapping>,
    pub cost: KirCostEstimate,
    pub growth: VectorPlanGrowth,
    pub proof: UnrollProofRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnrollInstructionMapping {
    pub scalar_iteration: u32,
    pub source: InstructionId,
    pub transformed: InstructionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrollProofRecord {
    pub cfg_digest: String,
    pub source_order: Vec<InstructionId>,
    pub iterations: u32,
    pub factor: u8,
    pub remainder: u8,
    pub dedicated_exits: bool,
    pub lcssa: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpProofRecord {
    pub block: BlockId,
    pub source_order: Vec<InstructionId>,
    pub identity_lanes: Vec<u16>,
    pub barrier_free: bool,
    pub exact_memory_footprint: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecializationPlan {
    pub pre_state: KirPreStateIdentity,
    pub caller: FunctionId,
    pub call: InstructionId,
    pub callee: FunctionId,
    pub fact_set_digest: String,
    pub clone_ordinal: u8,
    pub clone: FunctionId,
    pub clone_name: String,
    pub reused: bool,
    pub o3_entry_module_units: u32,
    pub facts: Vec<SpecializationFact>,
    pub mapping: SpecializationIdMapping,
    pub cost: KirCostEstimate,
    pub growth: VectorPlanGrowth,
    pub argument_mapping_proof: ProofId,
    pub fact_scope_proof: ProofId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecializationIdMapping {
    pub parameters: Vec<(ValueId, ValueId)>,
    pub blocks: Vec<(BlockId, BlockId)>,
    pub instructions: Vec<(InstructionId, InstructionId)>,
    pub values: Vec<(ValueId, ValueId)>,
    pub memory_regions: Vec<(MemoryRegionId, MemoryRegionId)>,
    pub memory_versions: Vec<(crate::MemoryVersionId, crate::MemoryVersionId)>,
    pub vector_regions: Vec<(crate::VectorRegionId, crate::VectorRegionId)>,
}

pub fn validate_vectorization_plan(
    plan: &VectorizationPlan,
    profile: &KirTargetProfile,
) -> Result<(), String> {
    if plan.pre_state.profile_digest != profile.digest_hex() {
        return Err("vector plan target profile identity is stale".to_string());
    }
    if !is_sha256(&plan.pre_state.kir_digest) {
        return Err("vector plan pre-state KIR digest is malformed".to_string());
    }
    if !matches!(plan.vf, 2 | 4 | 8 | 16) || !(1..=4).contains(&plan.uf) {
        return Err("vector plan VF/UF is outside the closed schema".to_string());
    }
    if plan.operations.is_empty() {
        return Err("vector plan contains no mapped operations".to_string());
    }
    let mut previous_operation = None;
    for operation in &plan.operations {
        let identity = (
            operation.scalar,
            operation.operation,
            operation.unroll_index,
        );
        if previous_operation.is_some_and(|previous| previous >= identity) {
            return Err("vector plan operation mappings are not strictly ordered".to_string());
        }
        previous_operation = Some(identity);
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
            return Err(
                "vector plan lane mapping is not complete source-order identity".to_string(),
            );
        }
        let lanes = u8::try_from(plan.vf)
            .map_err(|_| "vector plan VF is not representable by the target schema")?;
        let key = KirCostKey {
            operation: operation.operation,
            lane: operation.lane_type,
            lanes,
            semantics: operation.semantics,
            alignment: operation.alignment,
        };
        if !matches!(
            profile.operation_availability(&key),
            Some(KirOperationAvailability::Legal(_))
        ) {
            return Err("vector plan target operation is unavailable".to_string());
        }
    }
    if plan
        .memory_groups
        .iter()
        .any(|group| group.unroll_index >= plan.uf)
    {
        return Err("vector plan memory group UF identity is outside the plan".to_string());
    }
    if plan.predicates.len() > 4 {
        return Err("vector plan has more than four runtime predicates".to_string());
    }
    validate_cost(plan.cost, 20)?;
    validate_growth(plan.pre_state.frozen_kir_units, plan.growth)?;
    Ok(())
}

fn validate_cost(cost: KirCostEstimate, minimum_reduction_percent: u32) -> Result<(), String> {
    if cost.total
        != cost
            .transformed_body
            .saturating_add(cost.predicates)
            .saturating_add(cost.epilogue)
    {
        return Err("plan cost total does not match its closed decomposition".to_string());
    }
    let maximum = cost.scalar.saturating_mul(100 - minimum_reduction_percent) / 100;
    if cost.total > maximum {
        return Err("plan cost does not meet the profitability threshold".to_string());
    }
    Ok(())
}

fn validate_growth(frozen_units: u32, growth: VectorPlanGrowth) -> Result<(), String> {
    if growth.original_units != frozen_units
        || growth.transformed_units > growth.original_units.saturating_mul(3).saturating_add(32)
        || growth.module_after_units > growth.module_before_units.saturating_mul(2)
    {
        return Err("vector plan growth exceeds its frozen structural budget".to_string());
    }
    Ok(())
}

#[must_use]
pub fn print_vectorization_plan(plan: &VectorizationPlan) -> String {
    let mut text = format!(
        "vector-plan function={} loop={} vf={} uf={} kir={} profile={} generation={} frozen={} cost={}/{}/{}/{}/{} growth={}/{}/{}/{}\n",
        plan.pre_state.function.index(),
        plan.loop_id.index(),
        plan.vf,
        plan.uf,
        plan.pre_state.kir_digest,
        plan.pre_state.profile_digest,
        plan.pre_state.evidence_generation,
        plan.pre_state.frozen_kir_units,
        plan.cost.scalar,
        plan.cost.transformed_body,
        plan.cost.predicates,
        plan.cost.epilogue,
        plan.cost.total,
        plan.growth.original_units,
        plan.growth.transformed_units,
        plan.growth.module_before_units,
        plan.growth.module_after_units,
    );
    for operation in &plan.operations {
        text.push_str(&format!(
            "op-map scalar=i{} vector=i{} uf-index={} op={} type={:?} semantics={:?} alignment={:?} lanes={}\n",
            operation.scalar.index(),
            operation.vector.index(),
            operation.unroll_index,
            operation_name(operation.operation),
            operation.lane_type,
            operation.semantics,
            operation.alignment,
            operation
                .lanes
                .iter()
                .map(|lane| format!("{}:{}", lane.lane, lane.scalar_iteration))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    for group in &plan.memory_groups {
        text.push_str(&format!(
            "memory r{} {:?} scalar={} vector=i{} uf-index={} proof=p{}\n",
            group.region.index(),
            group.access,
            group
                .scalar_instructions
                .iter()
                .map(|instruction| format!("i{}", instruction.index()))
                .collect::<Vec<_>>()
                .join(","),
            group.vector_instruction.index(),
            group.unroll_index,
            group.footprint_proof.index()
        ));
    }
    text
}

const fn operation_name(operation: KirProfileOperation) -> &'static str {
    match operation {
        KirProfileOperation::Splat => "splat",
        KirProfileOperation::Add => "add",
        KirProfileOperation::Subtract => "subtract",
        KirProfileOperation::Multiply => "multiply",
        KirProfileOperation::Divide => "divide",
        KirProfileOperation::Remainder => "remainder",
        KirProfileOperation::Negate => "negate",
        KirProfileOperation::MaskNot => "mask-not",
        KirProfileOperation::BitAnd => "bit-and",
        KirProfileOperation::BitOr => "bit-or",
        KirProfileOperation::BitXor => "bit-xor",
        KirProfileOperation::ShiftLeft => "shift-left",
        KirProfileOperation::ShiftRight => "shift-right",
        KirProfileOperation::Compare => "compare",
        KirProfileOperation::Select => "select",
        KirProfileOperation::Cast => "cast",
        KirProfileOperation::Insert => "insert",
        KirProfileOperation::Extract => "extract",
        KirProfileOperation::Load => "load",
        KirProfileOperation::Store => "store",
        KirProfileOperation::ReduceAdd => "reduce-add",
        KirProfileOperation::ReduceMin => "reduce-min",
        KirProfileOperation::ReduceMax => "reduce-max",
        KirProfileOperation::Branch => "branch",
        KirProfileOperation::RuntimePredicate => "runtime-predicate",
        KirProfileOperation::ReduceMultiply => "reduce-multiply",
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
