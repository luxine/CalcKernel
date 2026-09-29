use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, CandidateKey, FactId, FunctionId, InstructionId, KirAlignmentClass,
    KirArithmeticSemantics, KirCostEstimate, KirCostKey, KirCostSemantics, KirEdge, KirInstruction,
    KirInstructionKind, KirLaneType, KirOperationAvailability, KirPlace, KirProfileOperation,
    KirTerminator, LoopCandidateKind, LoopCandidateVariant, LoopId, LoopTripCount, MemoryRegionId,
    MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, MirUnaryOp, ValueId, WasmRangeCount,
    WasmSliceRangeRequirement,
};

use super::{
    AliasKind, IntegerType, analyze_canonical_loops_for_discovery, analyze_regions, query_alias,
};

/// Ordered source shape retained for one closed, strict SIMD decision tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmDecisionTreeNode {
    Branch {
        block: BlockId,
        condition: ValueId,
        /// Pure source instructions in dependency order that form `condition`.
        condition_dag: Vec<InstructionId>,
        then_edge: KirEdge,
        else_edge: KirEdge,
        then_node: Box<Self>,
        else_node: Box<Self>,
    },
    Leaf {
        block: BlockId,
        /// Strict, pure source instructions in source order that compute `value`.
        computation_dag: Vec<InstructionId>,
        value: ValueId,
        store: InstructionId,
        store_slice: ValueId,
        store_index: ValueId,
        store_region: MemoryRegionId,
        store_partition: MemoryRegionId,
        induction_update: InstructionId,
        induction_result: ValueId,
        join_edge: KirEdge,
    },
}

/// A source-proven, exact SIMD128 VF2/UF1 candidate for a piecewise loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmDecisionTreeCandidate {
    pub key: CandidateKey,
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub preheader: BlockId,
    pub header: BlockId,
    pub root: BlockId,
    pub join: BlockId,
    pub exit: BlockId,
    /// The canonical loop-header induction phi.
    pub induction: ValueId,
    /// The corresponding induction parameter at `root`.
    pub root_induction: ValueId,
    /// The exact loop bound from the canonical induction proof.
    pub bound: ValueId,
    pub root_load: InstructionId,
    pub root_load_value: ValueId,
    pub input_slice: ValueId,
    pub output_slice: ValueId,
    /// Source place region and MemorySSA partition are intentionally separate.
    pub input_region: MemoryRegionId,
    pub input_partition: MemoryRegionId,
    pub output_region: MemoryRegionId,
    pub output_partition: MemoryRegionId,
    /// Contract evidence required to version the vector path safely.
    pub noalias_fact: FactId,
    /// Unique preorder node blocks; excludes preheader, loop header, join/latch, and exit.
    pub blocks: Vec<BlockId>,
    /// Every supported pure computation in deterministic block/source order,
    /// including shared ancestors, listed exactly once.
    pub ordered_tree_dag: Vec<InstructionId>,
    pub tree: WasmDecisionTreeNode,
    pub range_requirements: Vec<WasmSliceRangeRequirement>,
    pub vf: u16,
    pub uf: u8,
    pub minimum_trip: u32,
    pub predicted_cost: KirCostEstimate,
}

/// Candidate list and explainable discovery rejections for a KIR state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionTreeDiscovery {
    pub candidates: Vec<WasmDecisionTreeCandidate>,
    pub fallbacks: Vec<DecisionTreeFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionTreeFallback {
    pub function: FunctionId,
    pub loop_id: Option<LoopId>,
    pub reason: String,
}

/// Finds strict, closed piecewise-store trees eligible for SIMD128 if-conversion.
///
/// Discovery is advisory: every accepted candidate is independently reconstructed
/// and checked against the immutable source KIR before materialization can commit.
#[must_use]
pub fn discover_wasm_decision_tree_candidates(
    state: &crate::KirVerifiedProgramState,
) -> DecisionTreeDiscovery {
    let module = state.module();
    if module.config.consumer != crate::KirConsumer::WebAssembly
        || module.profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
        || !module.profile.vector_operations_enabled()
        || module.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || module.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || module.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
    {
        return DecisionTreeDiscovery::default();
    }

    let mut discovery = DecisionTreeDiscovery::default();
    for function in &module.functions {
        let loops = analyze_canonical_loops_for_discovery(function);
        for descriptor in loops.loops.iter().filter(|loop_| loop_.innermost) {
            match discover_one(state, function, descriptor) {
                Ok(candidates) => discovery.candidates.extend(candidates),
                Err(reason) => discovery.fallbacks.push(DecisionTreeFallback {
                    function: function.id,
                    loop_id: Some(descriptor.id),
                    reason,
                }),
            }
        }
    }
    discovery.candidates.sort_by(|left, right| {
        right
            .uf
            .cmp(&left.uf)
            .then_with(|| left.key.cmp(&right.key))
    });
    discovery.fallbacks.sort_by(|left, right| {
        (left.function, left.loop_id, left.reason.as_str()).cmp(&(
            right.function,
            right.loop_id,
            right.reason.as_str(),
        ))
    });
    discovery
}

const MAX_TREE_BRANCHES: usize = 3;
const MAX_TREE_LEAVES: usize = 4;
const MAX_TREE_DEPTH: usize = 16;
const MAX_TREE_BLOCKS: usize = 32;
const MAX_SPECULATIVE_EXTRA_OPS: usize = 16;
const SPECULATIVE_EXTRA_TO_MIN_PATH_RATIO: usize = 4;

fn dead_pure_undef(function: &crate::KirFunction, instruction: &KirInstruction) -> bool {
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || !matches!(instruction.kind, KirInstructionKind::Undef { .. })
    {
        return false;
    }
    let [result] = instruction.results.as_slice() else {
        return false;
    };
    let value = result.value;
    !function.blocks.iter().any(|block| {
        block.instructions.iter().any(|candidate| {
            let mut used = false;
            super::visit_instruction_uses(candidate, &mut |operand| used |= operand == value);
            used
        }) || match &block.terminator {
            KirTerminator::Jump { edge } => edge.args.contains(&value),
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                *condition == value
                    || then_edge.args.contains(&value)
                    || else_edge.args.contains(&value)
            }
            KirTerminator::Return {
                value: returned, ..
            } => returned == &Some(value),
        }
    })
}

fn scalar_lowering_envelope(
    function: &crate::KirFunction,
    preheader: BlockId,
    header: BlockId,
    join: BlockId,
    exit: BlockId,
    members: &BTreeSet<BlockId>,
) -> bool {
    let mut shape = members.clone();
    shape.extend([preheader, header, join, exit]);
    if !function.exported
        || function.return_type != MirType::Void
        || !function.vector_regions.is_empty()
        || function.blocks.first().map(|block| block.id) != Some(preheader)
        || function.blocks.len() != shape.len()
        || function
            .blocks
            .iter()
            .any(|block| !shape.contains(&block.id))
    {
        return false;
    }
    let scalar_type = |type_node: &MirType| {
        matches!(
            type_node,
            MirType::Primitive(
                MirPrimitiveTypeName::F64 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool
            )
        ) || *type_node == MirType::Slice(Box::new(MirType::Primitive(MirPrimitiveTypeName::F64)))
    };
    if function
        .params
        .iter()
        .any(|param| !scalar_type(&param.type_node))
        || function.blocks.iter().any(|block| {
            block.params.iter().any(|param| {
                param
                    .type_node
                    .as_scalar()
                    .is_none_or(|kind| !scalar_type(kind))
            }) || block
                .instructions
                .iter()
                .flat_map(|instruction| &instruction.results)
                .any(|result| {
                    result
                        .type_node
                        .as_scalar()
                        .is_none_or(|kind| !scalar_type(kind))
                })
                || block.instructions.iter().any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::VersionPredicate { .. }
                            | KirInstructionKind::Call { .. }
                            | KirInstructionKind::RuntimeCall { .. }
                            | KirInstructionKind::Guard { .. }
                    )
                })
        })
    {
        return false;
    }
    let (Some(entry), Some(latch), Some(exit_block)) = (
        block(function, preheader),
        block(function, join),
        block(function, exit),
    ) else {
        return false;
    };
    matches!(&entry.terminator, KirTerminator::Jump { edge } if edge.target == header)
        && entry.instructions.iter().all(|instruction| {
            instruction.memory.is_none()
                && instruction.effect.is_none()
                && (matches!(
                    instruction.kind,
                    KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::ConstFloat { .. }
                        | KirInstructionKind::ConstBool { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::SliceLen { .. }
                ) || dead_pure_undef(function, instruction))
        })
        && latch.instructions.is_empty()
        && matches!(&latch.terminator, KirTerminator::Jump { edge } if edge.target == header)
        && exit_block.instructions.is_empty()
        && matches!(
            exit_block.terminator,
            KirTerminator::Return { value: None, .. }
        )
}

#[derive(Debug)]
struct TreeBuildState<'a> {
    function: &'a crate::KirFunction,
    loop_blocks: &'a BTreeSet<BlockId>,
    root: BlockId,
    root_induction: ValueId,
    join: Option<BlockId>,
    branches: usize,
    leaves: usize,
    members: BTreeSet<BlockId>,
    preorder: Vec<BlockId>,
    parents: BTreeMap<BlockId, (BlockId, crate::KirEdge)>,
    node_dags: BTreeMap<BlockId, Vec<InstructionId>>,
    leaf_store_data: BTreeMap<BlockId, LeafStoreData>,
}

#[derive(Debug, Clone, Copy)]
struct LeafStoreData {
    value: ValueId,
    store: InstructionId,
    slice: ValueId,
    index: ValueId,
    place_region: MemoryRegionId,
    memory_region: MemoryRegionId,
    increment: InstructionId,
    increment_result: ValueId,
}

fn discover_one(
    state: &crate::KirVerifiedProgramState,
    function: &crate::KirFunction,
    descriptor: &super::CanonicalLoopDescriptor,
) -> Result<Vec<WasmDecisionTreeCandidate>, String> {
    if !descriptor.lcssa || !descriptor.dedicated_exits {
        return Ok(Vec::new());
    }
    let induction = descriptor.induction.as_ref().filter(|induction| {
        induction.type_node == IntegerType::U32
            && induction.start == BigInt::from(0)
            && induction.step == BigInt::from(1)
            && induction.comparison == MirCompareOp::Lt
            && induction.wrap_safe_for_strict_bound
    });
    let Some(induction) = induction else {
        return Ok(Vec::new());
    };
    if !matches!(
        descriptor.trip_count,
        LoopTripCount::Runtime { .. } | LoopTripCount::Exact { .. }
    ) {
        return Ok(Vec::new());
    }
    let (Some(preheader_id), Some(latch_id)) = (descriptor.preheader, descriptor.latch) else {
        return Ok(Vec::new());
    };
    let Some(preheader) = block(function, preheader_id) else {
        return Ok(Vec::new());
    };
    let Some(header) = block(function, descriptor.header) else {
        return Ok(Vec::new());
    };
    let Some(latch) = block(function, latch_id) else {
        return Ok(Vec::new());
    };
    let KirTerminator::Jump { edge: entry_edge } = &preheader.terminator else {
        return Ok(Vec::new());
    };
    if entry_edge.target != header.id {
        return Ok(Vec::new());
    }
    let KirTerminator::Branch {
        condition: loop_condition,
        then_edge: body_edge,
        else_edge: exit_edge,
    } = &header.terminator
    else {
        return Ok(Vec::new());
    };
    let Some(induction_index) = header
        .params
        .iter()
        .position(|parameter| parameter.value == induction.value)
    else {
        return Ok(Vec::new());
    };
    if body_edge.args.len() != block(function, body_edge.target).map_or(0, |body| body.params.len())
        || body_edge.memory_args.len()
            != block(function, body_edge.target).map_or(usize::MAX, |body| body.memory_params.len())
        || body_edge.target == exit_edge.target
        || !descriptor.blocks.contains(&body_edge.target)
        || descriptor.blocks.contains(&exit_edge.target)
        || !is_strict_loop_test(
            function,
            header,
            entry_edge,
            *loop_condition,
            induction.value,
            induction.bound,
        )
        || !stable_loop_bound(function, header, preheader, entry_edge, induction.bound)
        || !header_loop_condition_is_safe(
            function,
            header,
            preheader,
            entry_edge,
            *loop_condition,
            induction.bound,
        )
    {
        return Ok(Vec::new());
    }
    let Some(root) = block(function, body_edge.target) else {
        return Ok(Vec::new());
    };
    let Some(root_induction_index) = body_edge
        .args
        .iter()
        .position(|argument| *argument == induction.value)
    else {
        return Ok(Vec::new());
    };
    let Some(root_induction) = root
        .params
        .get(root_induction_index)
        .map(|parameter| parameter.value)
    else {
        return Ok(Vec::new());
    };
    if !is_u32_value(function, root_induction) {
        return Ok(Vec::new());
    }

    let loop_blocks = descriptor.blocks.iter().copied().collect::<BTreeSet<_>>();
    let mut build = TreeBuildState {
        function,
        loop_blocks: &loop_blocks,
        root: root.id,
        root_induction,
        join: None,
        branches: 0,
        leaves: 0,
        members: BTreeSet::new(),
        preorder: Vec::new(),
        parents: BTreeMap::new(),
        node_dags: BTreeMap::new(),
        leaf_store_data: BTreeMap::new(),
    };
    let tree = visit_tree(&mut build, root.id, descriptor.header, body_edge, 0);
    let Some(tree) = tree else {
        return Err("decision-tree-source-tree-shape-or-effects-not-proven".to_string());
    };
    if build.branches == 0
        || build.branches > MAX_TREE_BRANCHES
        || !(2..=MAX_TREE_LEAVES).contains(&build.leaves)
        || build.preorder.len() > MAX_TREE_BLOCKS
    {
        return Ok(Vec::new());
    }
    let Some(join_id) = build.join else {
        return Ok(Vec::new());
    };
    if join_id != latch.id || !valid_latch(function, header, latch, induction_index) {
        return Err("decision-tree-common-latch-not-proven".to_string());
    }
    let root_loads = root
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .collect::<Vec<_>>();
    let [root_load] = root_loads.as_slice() else {
        return Err("decision-tree-requires-one-root-input-load".to_string());
    };
    let Some((input_slice, root_load_index, input_region, input_partition)) =
        slice_memory_access(function, root_load, false)
    else {
        return Ok(Vec::new());
    };
    if resolve_value(&build, root.id, root_load_index) != root_induction
        || root_load.results.len() != 1
        || !is_f64_value(function, root_load.results[0].value)
        || root.instructions.first().map(|instruction| instruction.id) != Some(root_load.id)
        || !root_load
            .effect
            .as_ref()
            .is_some_and(|effect| effect.kind == crate::KirEffectKind::ReadMemory)
    {
        return Ok(Vec::new());
    }
    let root_load_value = root_load.results[0].value;

    let leaf_data = build
        .preorder
        .iter()
        .filter_map(|block| build.leaf_store_data.get(block).map(|leaf| (*block, *leaf)))
        .collect::<Vec<_>>();
    if leaf_data.len() != build.leaves {
        return Ok(Vec::new());
    }
    let first_leaf = leaf_data.first().map(|(_, leaf)| *leaf);
    let Some(first_leaf) = first_leaf else {
        return Ok(Vec::new());
    };
    let output_slice = resolve_value(&build, leaf_data[0].0, first_leaf.slice);
    let output_region = first_leaf.place_region;
    let output_partition = first_leaf.memory_region;
    for (leaf_block, leaf) in &leaf_data {
        if resolve_value(&build, *leaf_block, leaf.slice) != output_slice
            || resolve_value(&build, *leaf_block, leaf.index) != root_induction
            || leaf.place_region != output_region
            || leaf.memory_region != output_partition
            || !is_f64_value(function, leaf.value)
        {
            return Ok(Vec::new());
        }
    }
    if !is_root_parameter(root, input_slice) || !is_root_parameter(root, output_slice) {
        return Ok(Vec::new());
    }
    let regions = analyze_regions(
        function,
        state.contract_facts().map(crate::ContractFactSet::facts),
    )
    .map_err(|error| error.message)?;
    let alias = query_alias(&regions, input_region, output_region);
    let (AliasKind::NoAlias, Some(noalias_fact)) = (alias.kind, alias.fact) else {
        return Err("decision-tree-source-noalias-contract-not-proven".to_string());
    };
    if !noalias_fact_available(state, function, preheader_id, noalias_fact) {
        return Err("decision-tree-source-noalias-fact-not-available-at-preheader".to_string());
    }

    let ordered_tree_dag = build
        .preorder
        .iter()
        .flat_map(|block_id| {
            block(function, *block_id)
                .into_iter()
                .flat_map(|block| block.instructions.iter())
        })
        .filter(|instruction| {
            instruction.id != root_load.id
                && !build
                    .leaf_store_data
                    .values()
                    .any(|leaf| leaf.store == instruction.id || leaf.increment == instruction.id)
        })
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>();
    if ordered_tree_dag
        .iter()
        .any(|id| !tree_instruction_is_pure(function, *id))
    {
        return Err("decision-tree-pure-dag-not-proven".to_string());
    }
    // This is a strict source-shape subset of the committed WASM closed-store
    // select lowering. All-arm scalar pricing below is valid only after these
    // independent structural conditions, including memory and escape closure,
    // match that emitter's admission conditions.
    if !scalar_tree_values_are_scalar(function, &build) {
        return Err("decision-tree-scalar-values-not-proven".to_string());
    }
    if !closed_tree_values(function, &build) {
        return Err("decision-tree-tree-values-escape-closed-region".to_string());
    }
    if !closed_join_values(
        function,
        &build,
        latch,
        induction_index,
        induction.value,
        root_induction,
    ) {
        return Err("decision-tree-single-induction-join-not-proven".to_string());
    }
    if !closed_memory(
        function,
        root_load,
        &build,
        latch,
        input_partition,
        output_partition,
    ) {
        return Err("decision-tree-memory-ssa-join-not-proven".to_string());
    }
    if !scalar_lowering_envelope(
        function,
        preheader_id,
        header.id,
        latch_id,
        exit_edge.target,
        &build.members,
    ) {
        return Err("decision-tree-scalar-lowering-cost-not-proven".to_string());
    }
    // All-arm scalar pricing is valid only after this exact source shape has
    // been shown to satisfy the closed-store select emitter's source matcher
    // and its frozen speculative-work budget.
    if !speculation_within_budget(function, &tree, &build) {
        return Err("decision-tree-speculative-work-budget-exceeded".to_string());
    }
    let Some(operations) = operation_counts(function, &ordered_tree_dag, build.branches) else {
        return Err("decision-tree-operation-profile-not-proven".to_string());
    };
    let splat_inputs = scalar_splat_inputs(function, &build, root_load_value, &ordered_tree_dag);
    let range_requirements = vec![
        WasmSliceRangeRequirement {
            slice: input_slice,
            start: None,
            count: WasmRangeCount::TripBound(induction.bound),
            element_bytes: 8,
        },
        WasmSliceRangeRequirement {
            slice: output_slice,
            start: None,
            count: WasmRangeCount::TripBound(induction.bound),
            element_bytes: 8,
        },
    ];
    let mut candidates = Vec::new();
    for uf in [4, 1] {
        let Some((predicted_cost, minimum_trip)) = estimate_cost(
            &state.module().profile,
            function,
            &ordered_tree_dag,
            operations,
            build.branches,
            &splat_inputs,
            uf,
        ) else {
            continue;
        };
        if matches!(descriptor.trip_count, LoopTripCount::Exact { iterations } if iterations < u64::from(minimum_trip) || iterations > u64::from(u32::MAX))
        {
            continue;
        }
        let key = CandidateKey::LoopFrontier {
            function: function.id,
            loop_id: descriptor.id,
            kind: LoopCandidateKind::DecisionTreeVector,
            variant: LoopCandidateVariant::Scalar,
            vf: 2,
            uf,
        };
        candidates.push(WasmDecisionTreeCandidate {
            key,
            function: function.id,
            loop_id: descriptor.id,
            preheader: preheader_id,
            header: header.id,
            root: root.id,
            join: join_id,
            exit: exit_edge.target,
            induction: induction.value,
            root_induction,
            bound: induction.bound,
            root_load: root_load.id,
            root_load_value,
            input_slice,
            output_slice,
            input_region,
            input_partition,
            output_region,
            output_partition,
            noalias_fact,
            blocks: build.preorder.clone(),
            ordered_tree_dag: ordered_tree_dag.clone(),
            tree: tree.clone(),
            range_requirements: range_requirements.clone(),
            vf: 2,
            uf,
            minimum_trip,
            predicted_cost,
        });
    }
    if candidates.is_empty() {
        return Err("decision-tree-vector-cost-threshold-not-met".to_string());
    }
    Ok(candidates)
}

fn visit_tree(
    state: &mut TreeBuildState<'_>,
    block_id: BlockId,
    parent_id: BlockId,
    incoming: &crate::KirEdge,
    depth: usize,
) -> Option<WasmDecisionTreeNode> {
    if depth > MAX_TREE_DEPTH
        || !state.loop_blocks.contains(&block_id)
        || !state.members.insert(block_id)
    {
        return None;
    }
    let current = block(state.function, block_id)?;
    let parent = block(state.function, parent_id)?;
    if incoming.target != block_id
        || current.params.len() != incoming.args.len()
        || current.memory_params.len() != incoming.memory_args.len()
        || !edge_preserves_memory(parent, incoming)
        || parent
            .memory_params
            .iter()
            .zip(&current.memory_params)
            .any(|(parent, child)| parent.region != child.region)
        || current
            .params
            .iter()
            .zip(&incoming.args)
            .any(|(param, value)| {
                value_type(state.function, *value).as_ref() != Some(&param.type_node)
            })
    {
        return None;
    }
    if block_predecessor_edges(state.function, block_id).len() != 1
        || block_predecessor_edges(state.function, block_id)[0].0 != parent_id
    {
        return None;
    }
    if block_id != state.root {
        state
            .parents
            .insert(block_id, (parent_id, incoming.clone()));
    }
    state.preorder.push(block_id);
    if state.preorder.len() > MAX_TREE_BLOCKS {
        return None;
    }

    match &current.terminator {
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            if state.branches >= MAX_TREE_BRANCHES
                || !is_bool_value(state.function, *condition)
                || then_edge.target == else_edge.target
                || !branch_block_is_safe(state.function, current, block_id == state.root)
                || !edge_preserves_memory(current, then_edge)
                || !edge_preserves_memory(current, else_edge)
            {
                return None;
            }
            state.branches += 1;
            let condition_dag =
                local_dependency_dag(state.function, block_id, *condition, state.root);
            let then_node = visit_tree(
                state,
                then_edge.target,
                block_id,
                then_edge,
                depth.saturating_add(1),
            )?;
            let else_node = visit_tree(
                state,
                else_edge.target,
                block_id,
                else_edge,
                depth.saturating_add(1),
            )?;
            state.node_dags.insert(block_id, condition_dag.clone());
            Some(WasmDecisionTreeNode::Branch {
                block: block_id,
                condition: *condition,
                condition_dag,
                then_edge: then_edge.clone(),
                else_edge: else_edge.clone(),
                then_node: Box::new(then_node),
                else_node: Box::new(else_node),
            })
        }
        KirTerminator::Jump { edge } => {
            if state.leaves >= MAX_TREE_LEAVES
                || edge.target == block_id
                || !edge_shape_matches(state.function, edge)
            {
                return None;
            }
            let stores = current
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
                .collect::<Vec<_>>();
            let [store] = stores.as_slice() else {
                return None;
            };
            let increments = current
                .instructions
                .iter()
                .filter(|instruction| is_unit_induction_increment(state, block_id, instruction))
                .collect::<Vec<_>>();
            let [increment] = increments.as_slice() else {
                return None;
            };
            if current.instructions.iter().any(|instruction| {
                instruction.id != store.id
                    && instruction.id != increment.id
                    && !strict_tree_pure_instruction(state.function, instruction)
            }) {
                return None;
            }
            let (Some((slice, index, place_region, memory_region)), Some(value)) = (
                slice_memory_access(state.function, store, true),
                store_value(&store.kind),
            ) else {
                return None;
            };
            let increment_result = increment.results.first().map(|result| result.value)?;
            if increment.results.len() != 1
                || !is_u32_value(state.function, increment_result)
                || !is_f64_value(state.function, value)
                || !store
                    .effect
                    .as_ref()
                    .is_some_and(|effect| effect.kind == crate::KirEffectKind::WriteMemory)
            {
                return None;
            }
            let join = state.join.get_or_insert(edge.target);
            if *join != edge.target {
                return None;
            }
            if edge
                .args
                .iter()
                .filter(|arg| **arg == increment_result)
                .count()
                != 1
            {
                return None;
            }
            let computation_dag = local_dependency_dag(state.function, block_id, value, state.root);
            state.node_dags.insert(block_id, computation_dag.clone());
            state.leaf_store_data.insert(
                block_id,
                LeafStoreData {
                    value,
                    store: store.id,
                    slice,
                    index,
                    place_region,
                    memory_region,
                    increment: increment.id,
                    increment_result,
                },
            );
            state.leaves += 1;
            Some(WasmDecisionTreeNode::Leaf {
                block: block_id,
                computation_dag,
                value,
                store: store.id,
                store_slice: slice,
                store_index: index,
                store_region: place_region,
                store_partition: memory_region,
                induction_update: increment.id,
                induction_result: increment_result,
                join_edge: edge.clone(),
            })
        }
        KirTerminator::Return { .. } => None,
    }
}

fn block(function: &crate::KirFunction, id: BlockId) -> Option<&crate::KirBlock> {
    function.blocks.iter().find(|block| block.id == id)
}

fn block_predecessor_edges(
    function: &crate::KirFunction,
    target: BlockId,
) -> Vec<(BlockId, &crate::KirEdge)> {
    function
        .blocks
        .iter()
        .flat_map(|block| {
            let edges = match &block.terminator {
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
                KirTerminator::Return { .. } => Vec::new(),
            };
            edges
                .into_iter()
                .filter(move |edge| edge.target == target)
                .map(move |edge| (block.id, edge))
        })
        .collect()
}

fn edge_shape_matches(function: &crate::KirFunction, edge: &crate::KirEdge) -> bool {
    block(function, edge.target).is_some_and(|target| {
        edge.args.len() == target.params.len()
            && edge.memory_args.len() == target.memory_params.len()
    })
}

fn edge_preserves_memory(block: &crate::KirBlock, edge: &crate::KirEdge) -> bool {
    edge.memory_args.len() == block.memory_params.len()
        && block
            .memory_params
            .iter()
            .zip(&edge.memory_args)
            .all(|(param, argument)| param.version == *argument)
}

fn value_type(function: &crate::KirFunction, value: ValueId) -> Option<crate::KirValueType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| crate::KirValueType::Scalar(param.type_node.clone()))
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.params)
                .find(|param| param.value == value)
                .map(|param| param.type_node.clone())
        })
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .flat_map(|instruction| &instruction.results)
                .find(|result| result.value == value)
                .map(|result| result.type_node.clone())
        })
}

fn is_bool_value(function: &crate::KirFunction, value: ValueId) -> bool {
    value_type(function, value).and_then(|type_node| type_node.as_scalar().cloned())
        == Some(MirType::Primitive(MirPrimitiveTypeName::Bool))
}

fn is_u32_value(function: &crate::KirFunction, value: ValueId) -> bool {
    value_type(function, value).and_then(|type_node| type_node.as_scalar().cloned())
        == Some(MirType::Primitive(MirPrimitiveTypeName::U32))
}

fn is_f64_value(function: &crate::KirFunction, value: ValueId) -> bool {
    value_type(function, value).and_then(|type_node| type_node.as_scalar().cloned())
        == Some(f64_type())
}

fn f64_type() -> MirType {
    MirType::Primitive(MirPrimitiveTypeName::F64)
}

fn is_strict_loop_test(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    entry: &crate::KirEdge,
    condition: ValueId,
    induction: ValueId,
    bound: ValueId,
) -> bool {
    let Some(definition) = instruction_def(function, condition) else {
        return false;
    };
    matches!(
        definition.kind,
        KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left,
            right,
        } if left == induction && resolves_to_entry_value(function, header, entry, right, bound)
    ) && definition.memory.is_none()
        && definition.effect.is_none()
}

fn resolves_to_entry_value(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    entry: &crate::KirEdge,
    mut value: ValueId,
    expected: ValueId,
) -> bool {
    let mut visited = BTreeSet::new();
    while visited.insert(value) {
        if value == expected {
            return true;
        }
        if let Some(index) = header.params.iter().position(|param| param.value == value) {
            let Some(argument) = entry.args.get(index) else {
                return false;
            };
            value = *argument;
            continue;
        }
        let Some(definition) = instruction_def(function, value) else {
            return false;
        };
        if let KirInstructionKind::Copy { value: source } = definition.kind {
            value = source;
        } else {
            return false;
        }
    }
    false
}

fn stable_loop_bound(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    preheader: &crate::KirBlock,
    entry: &crate::KirEdge,
    bound: ValueId,
) -> bool {
    fn root_value(
        function: &crate::KirFunction,
        header: &crate::KirBlock,
        entry: &crate::KirEdge,
        value: ValueId,
        visiting: &mut BTreeSet<ValueId>,
    ) -> bool {
        if function.params.iter().any(|param| param.value == value) {
            return true;
        }
        if !visiting.insert(value) {
            return false;
        }
        let result =
            if let Some(index) = header.params.iter().position(|param| param.value == value) {
                entry.args.get(index).is_some_and(|argument| {
                    root_value(function, header, entry, *argument, visiting)
                })
            } else if let Some(definition) = instruction_def(function, value) {
                match definition.kind {
                    KirInstructionKind::Copy { value: source } => {
                        root_value(function, header, entry, source, visiting)
                    }
                    KirInstructionKind::ConstInt { .. } => true,
                    KirInstructionKind::SliceLen { slice } => {
                        root_value(function, header, entry, slice, visiting)
                    }
                    _ => false,
                }
            } else {
                false
            };
        visiting.remove(&value);
        result
    }
    is_u32_value(function, bound)
        && root_value(function, header, entry, bound, &mut BTreeSet::new())
        && header.id != preheader.id
}

fn header_loop_condition_is_safe(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    preheader: &crate::KirBlock,
    entry: &crate::KirEdge,
    condition: ValueId,
    bound: ValueId,
) -> bool {
    fn collect_bound_definition(
        function: &crate::KirFunction,
        header: &crate::KirBlock,
        preheader: &crate::KirBlock,
        entry: &crate::KirEdge,
        value: ValueId,
        visiting: &mut BTreeSet<ValueId>,
        header_instructions: &mut BTreeSet<InstructionId>,
    ) -> bool {
        if function
            .params
            .iter()
            .any(|parameter| parameter.value == value)
        {
            return true;
        }
        if !visiting.insert(value) {
            return false;
        }
        let result = if let Some(index) = header
            .params
            .iter()
            .position(|parameter| parameter.value == value)
        {
            entry.args.get(index).is_some_and(|argument| {
                collect_bound_definition(
                    function,
                    header,
                    preheader,
                    entry,
                    *argument,
                    visiting,
                    header_instructions,
                )
            })
        } else if let Some(definition) = instruction_def(function, value) {
            let definition_block = defining_block(function, definition.id);
            let dominates_preheader = definition_block.is_some_and(|block_id| {
                block_id == header.id
                    || crate::compute_kir_dominators(function).dominates(block_id, preheader.id)
            });
            let pure_value = definition.memory.is_none()
                && definition.effect.is_none()
                && definition.results.len() == 1
                && definition.results[0].value == value;
            if !dominates_preheader || !pure_value {
                false
            } else {
                match definition.kind {
                    KirInstructionKind::Copy { value: source } => {
                        header_instructions.insert(definition.id);
                        collect_bound_definition(
                            function,
                            header,
                            preheader,
                            entry,
                            source,
                            visiting,
                            header_instructions,
                        )
                    }
                    KirInstructionKind::ConstInt { .. } if is_u32_value(function, value) => {
                        header_instructions.insert(definition.id);
                        true
                    }
                    KirInstructionKind::SliceLen { slice } if is_u32_value(function, value) => {
                        header_instructions.insert(definition.id);
                        collect_bound_definition(
                            function,
                            header,
                            preheader,
                            entry,
                            slice,
                            visiting,
                            header_instructions,
                        )
                    }
                    _ => false,
                }
            }
        } else {
            false
        };
        visiting.remove(&value);
        result
    }

    let Some(compare) = instruction_def(function, condition) else {
        return false;
    };
    let Some(compare_block) = defining_block(function, compare.id) else {
        return false;
    };
    if compare_block != header.id {
        return false;
    }
    let mut allowed = BTreeSet::from([compare.id]);
    if !collect_bound_definition(
        function,
        header,
        preheader,
        entry,
        bound,
        &mut BTreeSet::new(),
        &mut allowed,
    ) {
        return false;
    }
    header
        .instructions
        .iter()
        .all(|instruction| allowed.contains(&instruction.id))
}

fn valid_latch(
    function: &crate::KirFunction,
    header: &crate::KirBlock,
    latch: &crate::KirBlock,
    induction_index: usize,
) -> bool {
    if !latch.instructions.is_empty() {
        return false;
    }
    let KirTerminator::Jump { edge } = &latch.terminator else {
        return false;
    };
    edge.target == header.id
        && edge_shape_matches(function, edge)
        && edge.memory_args.len() == latch.memory_params.len()
        && latch
            .memory_params
            .iter()
            .zip(&edge.memory_args)
            .zip(&header.memory_params)
            .all(|((latch_param, argument), header_param)| {
                latch_param.version == *argument && latch_param.region == header_param.region
            })
        && edge.args.get(induction_index).is_some_and(|value| {
            latch
                .params
                .iter()
                .any(|parameter| parameter.value == *value && is_u32_value(function, *value))
        })
}

fn is_root_parameter(root: &crate::KirBlock, value: ValueId) -> bool {
    root.params.iter().any(|param| param.value == value)
}

fn instruction_def(function: &crate::KirFunction, value: ValueId) -> Option<&KirInstruction> {
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

fn defining_block(function: &crate::KirFunction, instruction_id: InstructionId) -> Option<BlockId> {
    function.blocks.iter().find_map(|block| {
        block
            .instructions
            .iter()
            .any(|instruction| instruction.id == instruction_id)
            .then_some(block.id)
    })
}

fn noalias_fact_available(
    state: &crate::KirVerifiedProgramState,
    function: &crate::KirFunction,
    preheader: BlockId,
    noalias_fact: FactId,
) -> bool {
    let dominators = crate::compute_kir_dominators(function);
    state
        .contract_facts()
        .and_then(|contracts| contracts.facts().get(noalias_fact))
        .is_some_and(|fact| {
            fact.generation == state.evidence_generation()
                && matches!(fact.origin, crate::FactOrigin::TrustedContract { .. })
                && matches!(
                    &fact.predicate,
                    crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias { .. })
                )
                && match &fact.scope {
                    crate::FactScope::FunctionEntry(owner) => *owner == function.id,
                    crate::FactScope::Block {
                        function: owner,
                        block,
                    } => *owner == function.id && dominators.dominates(*block, preheader),
                    _ => false,
                }
        })
}

fn branch_block_is_safe(
    function: &crate::KirFunction,
    block: &crate::KirBlock,
    root: bool,
) -> bool {
    let loads = block
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .collect::<Vec<_>>();
    if root {
        if loads.len() != 1 {
            return false;
        }
        let load = loads[0];
        if !matches!(&load.memory, Some(access) if access.output.is_none())
            || !load
                .effect
                .as_ref()
                .is_some_and(|effect| effect.kind == crate::KirEffectKind::ReadMemory)
        {
            return false;
        }
    } else if !loads.is_empty() {
        return false;
    }
    block.instructions.iter().all(|instruction| {
        (root && loads.first().is_some_and(|load| load.id == instruction.id))
            || strict_tree_pure_instruction(function, instruction)
    })
}

fn strict_tree_pure_instruction(
    function: &crate::KirFunction,
    instruction: &KirInstruction,
) -> bool {
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
    {
        return false;
    }
    let result = instruction.results[0].value;
    match instruction.kind {
        KirInstructionKind::ConstFloat { .. } => is_f64_value(function, result),
        KirInstructionKind::ConstBool { .. } => is_bool_value(function, result),
        KirInstructionKind::Copy { value } => {
            (is_f64_value(function, value) || is_bool_value(function, value))
                && value_type(function, value) == value_type(function, result)
        }
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            left,
            right,
            semantics: KirArithmeticSemantics::StrictFloat,
        } => {
            is_f64_value(function, left)
                && is_f64_value(function, right)
                && is_f64_value(function, result)
        }
        KirInstructionKind::Unary {
            op: MirUnaryOp::Neg,
            operand,
            semantics: KirArithmeticSemantics::StrictFloat,
        } => is_f64_value(function, operand) && is_f64_value(function, result),
        KirInstructionKind::Compare { left, right, .. } => {
            is_f64_value(function, left)
                && is_f64_value(function, right)
                && is_bool_value(function, result)
        }
        _ => false,
    }
}

fn tree_instruction_is_pure(function: &crate::KirFunction, instruction_id: InstructionId) -> bool {
    instruction_def_by_id(function, instruction_id)
        .is_some_and(|instruction| strict_tree_pure_instruction(function, instruction))
}

fn instruction_def_by_id(
    function: &crate::KirFunction,
    id: InstructionId,
) -> Option<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
}

fn is_unit_induction_increment(
    state: &TreeBuildState<'_>,
    block_id: BlockId,
    instruction: &KirInstruction,
) -> bool {
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = instruction.kind
    else {
        return false;
    };
    instruction.memory.is_none()
        && instruction.effect.is_none()
        && instruction.results.len() == 1
        && is_u32_value(state.function, instruction.results[0].value)
        && ((resolve_value(state, block_id, left) == state.root_induction
            && is_one(state.function, right))
            || (resolve_value(state, block_id, right) == state.root_induction
                && is_one(state.function, left)))
}

fn is_one(function: &crate::KirFunction, value: ValueId) -> bool {
    instruction_def(function, value).is_some_and(|instruction| {
        matches!(
            &instruction.kind,
            KirInstructionKind::ConstInt { value }
                if value.replace('_', "").parse::<i128>().ok() == Some(1)
        )
    })
}

fn store_value(kind: &KirInstructionKind) -> Option<ValueId> {
    if let KirInstructionKind::Store { value, .. } = kind {
        Some(*value)
    } else {
        None
    }
}

fn slice_memory_access(
    function: &crate::KirFunction,
    instruction: &KirInstruction,
    write: bool,
) -> Option<(ValueId, ValueId, MemoryRegionId, MemoryRegionId)> {
    let place = match &instruction.kind {
        KirInstructionKind::Load { place } if !write => place.as_ref(),
        KirInstructionKind::Store { place, .. } if write => place.as_ref(),
        _ => return None,
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        region,
    } = place
    else {
        return None;
    };
    if type_node != &f64_type() || !is_u32_value(function, *index) {
        return None;
    }
    let memory = instruction.memory.as_ref()?;
    let source_region = function
        .regions
        .iter()
        .find(|candidate| candidate.id == *region)?;
    if write != memory.output.is_some()
        || source_region.partition != memory.region
        || !function
            .regions
            .iter()
            .any(|candidate| candidate.id == memory.region && candidate.partition == memory.region)
    {
        return None;
    }
    Some((*slice, *index, *region, memory.region))
}

fn resolve_value(state: &TreeBuildState<'_>, mut block_id: BlockId, mut value: ValueId) -> ValueId {
    while block_id != state.root {
        let Some(block) = block(state.function, block_id) else {
            break;
        };
        let Some(index) = block
            .params
            .iter()
            .position(|parameter| parameter.value == value)
        else {
            break;
        };
        let Some((parent, edge)) = state.parents.get(&block_id) else {
            break;
        };
        let Some(argument) = edge.args.get(index) else {
            break;
        };
        value = *argument;
        block_id = *parent;
    }
    value
}

fn resolve_memory(
    state: &TreeBuildState<'_>,
    mut block_id: BlockId,
    mut value: crate::MemoryVersionId,
) -> crate::MemoryVersionId {
    while block_id != state.root {
        let Some(block) = block(state.function, block_id) else {
            break;
        };
        let Some(index) = block
            .memory_params
            .iter()
            .position(|parameter| parameter.version == value)
        else {
            break;
        };
        let Some((parent, edge)) = state.parents.get(&block_id) else {
            break;
        };
        let Some(argument) = edge.memory_args.get(index) else {
            break;
        };
        value = *argument;
        block_id = *parent;
    }
    value
}

fn closed_tree_values(function: &crate::KirFunction, state: &TreeBuildState<'_>) -> bool {
    let local_values = function
        .blocks
        .iter()
        .filter(|block| state.members.contains(&block.id))
        .flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        })
        .collect::<BTreeSet<_>>();
    for block in function
        .blocks
        .iter()
        .filter(|block| !state.members.contains(&block.id))
    {
        if block.instructions.iter().any(|instruction| {
            instruction_uses(instruction)
                .iter()
                .any(|value| local_values.contains(value))
        }) || terminator_uses(&block.terminator)
            .iter()
            .any(|value| local_values.contains(value))
        {
            return false;
        }
    }
    let Some(join_id) = state.join else {
        return false;
    };
    for (block_id, leaf) in &state.leaf_store_data {
        let mut edge_uses = 0_usize;
        for block in function
            .blocks
            .iter()
            .filter(|block| state.members.contains(&block.id))
        {
            if block
                .instructions
                .iter()
                .any(|instruction| instruction_uses(instruction).contains(&leaf.increment_result))
            {
                return false;
            }
            match &block.terminator {
                KirTerminator::Jump { edge } => {
                    for argument in &edge.args {
                        if *argument == leaf.increment_result {
                            if block.id != *block_id || edge.target != join_id {
                                return false;
                            }
                            edge_uses = edge_uses.saturating_add(1);
                        }
                    }
                }
                _ if terminator_uses(&block.terminator).contains(&leaf.increment_result) => {
                    return false;
                }
                _ => {}
            }
        }
        if edge_uses != 1 {
            return false;
        }
    }
    true
}

fn scalar_tree_values_are_scalar(
    function: &crate::KirFunction,
    state: &TreeBuildState<'_>,
) -> bool {
    function
        .blocks
        .iter()
        .filter(|block| state.members.contains(&block.id))
        .all(|block| {
            block
                .params
                .iter()
                .all(|parameter| parameter.type_node.as_scalar().is_some())
                && block.instructions.iter().all(|instruction| {
                    instruction
                        .results
                        .iter()
                        .all(|result| result.type_node.as_scalar().is_some())
                })
        })
}

fn closed_join_values(
    function: &crate::KirFunction,
    state: &TreeBuildState<'_>,
    join: &crate::KirBlock,
    induction_index: usize,
    header_induction: ValueId,
    root_induction: ValueId,
) -> bool {
    let Some(header) = function.blocks.iter().find(|block| {
        block
            .params
            .iter()
            .any(|param| param.value == header_induction)
            && matches!(block.terminator, KirTerminator::Branch { .. })
    }) else {
        return false;
    };
    if header
        .params
        .get(induction_index)
        .map(|parameter| parameter.value)
        != Some(header_induction)
    {
        return false;
    }
    let KirTerminator::Jump { edge: latch_edge } = &join.terminator else {
        return false;
    };
    let Some(iv_join_index) = join.params.iter().position(|parameter| {
        Some(parameter.value) == latch_edge.args.get(induction_index).copied()
    }) else {
        return false;
    };
    let leaves = state
        .leaf_store_data
        .iter()
        .filter_map(|(block_id, leaf)| {
            let block = block(function, *block_id)?;
            let KirTerminator::Jump { edge } = &block.terminator else {
                return None;
            };
            Some((block_id, leaf, edge))
        })
        .collect::<Vec<_>>();
    if leaves.len() != state.leaf_store_data.len() || join.params.is_empty() {
        return false;
    }
    for (block_id, leaf, edge) in &leaves {
        if edge.target != join.id
            || edge.args.len() != join.params.len()
            || edge.memory_args.len() != join.memory_params.len()
            || edge.args.get(iv_join_index) != Some(&leaf.increment_result)
        {
            return false;
        }
        let Some(update) = instruction_def_by_id(function, leaf.increment) else {
            return false;
        };
        let KirInstructionKind::Binary { left, right, .. } = update.kind else {
            return false;
        };
        let base = if is_one(function, right) {
            left
        } else if is_one(function, left) {
            right
        } else {
            return false;
        };
        if resolve_value(state, **block_id, base) != root_induction {
            return false;
        }
    }

    let incoming = block_predecessor_edges(function, join.id);
    if incoming.len() != leaves.len()
        || incoming
            .iter()
            .any(|(predecessor, _)| !state.leaf_store_data.contains_key(predecessor))
    {
        return false;
    }

    for index in 0..join.params.len() {
        if index == iv_join_index {
            continue;
        }
        let mut common = None;
        for (block_id, _, edge) in &leaves {
            let Some(argument) = edge.args.get(index).copied() else {
                return false;
            };
            let resolved = resolve_value(state, **block_id, argument);
            if common.is_some_and(|previous| previous != resolved) {
                return false;
            }
            common = Some(resolved);
        }
    }
    if latch_edge.target != header.id
        || latch_edge.args.len() != header.params.len()
        || latch_edge.args.get(induction_index) != Some(&join.params[iv_join_index].value)
    {
        return false;
    }
    true
}

fn closed_memory(
    function: &crate::KirFunction,
    root_load: &KirInstruction,
    state: &TreeBuildState<'_>,
    join: &crate::KirBlock,
    input_partition: MemoryRegionId,
    output_partition: MemoryRegionId,
) -> bool {
    let Some(load_memory) = root_load.memory.as_ref() else {
        return false;
    };
    let Some(root) = block(function, state.root) else {
        return false;
    };
    if !root
        .memory_params
        .iter()
        .any(|param| param.region == input_partition && param.version == load_memory.input)
    {
        return false;
    }
    let output_indices = join
        .memory_params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| (param.region == output_partition).then_some(index))
        .collect::<Vec<_>>();
    let [output_index] = output_indices.as_slice() else {
        return false;
    };
    if join
        .memory_params
        .iter()
        .filter(|parameter| parameter.region == output_partition)
        .count()
        != 1
    {
        return false;
    }
    let leaves = state
        .leaf_store_data
        .iter()
        .filter_map(|(block_id, leaf)| {
            let block = block(function, *block_id)?;
            let KirTerminator::Jump { edge } = &block.terminator else {
                return None;
            };
            let store = instruction_def_by_id(function, leaf.store)?;
            Some((*block_id, block, edge, store))
        })
        .collect::<Vec<_>>();
    for (_, block, edge, store) in &leaves {
        let Some(memory) = store.memory.as_ref() else {
            return false;
        };
        let Some(output) = memory.output else {
            return false;
        };
        if memory.region != output_partition
            || !block
                .memory_params
                .iter()
                .any(|param| param.region == output_partition && param.version == memory.input)
            || edge.memory_args.get(*output_index) != Some(&output)
        {
            return false;
        }
    }
    for index in 0..join.memory_params.len() {
        if index == *output_index {
            continue;
        }
        let mut common = None;
        for (block_id, _, edge, _) in &leaves {
            let Some(argument) = edge.memory_args.get(index).copied() else {
                return false;
            };
            let resolved = resolve_memory(state, *block_id, argument);
            if common.is_some_and(|previous| previous != resolved) {
                return false;
            }
            common = Some(resolved);
        }
    }
    true
}

fn local_dependency_dag(
    function: &crate::KirFunction,
    block_id: BlockId,
    root_value: ValueId,
    tree_root: BlockId,
) -> Vec<InstructionId> {
    fn visit(
        function: &crate::KirFunction,
        current: BlockId,
        value: ValueId,
        tree_root: BlockId,
        found: &mut BTreeSet<InstructionId>,
        active: &mut BTreeSet<ValueId>,
    ) {
        if !active.insert(value) {
            return;
        }
        let Some(block) = block(function, current) else {
            active.remove(&value);
            return;
        };
        if block
            .params
            .iter()
            .any(|parameter| parameter.value == value)
        {
            active.remove(&value);
            return;
        }
        let Some(instruction) = block.instructions.iter().find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        }) else {
            active.remove(&value);
            return;
        };
        if !strict_tree_pure_instruction(function, instruction) {
            active.remove(&value);
            return;
        }
        found.insert(instruction.id);
        for operand in instruction_value_uses(instruction) {
            let operand_block = function
                .blocks
                .iter()
                .find(|block| block.params.iter().any(|param| param.value == operand))
                .map_or_else(
                    || {
                        function
                            .blocks
                            .iter()
                            .find(|block| {
                                block.instructions.iter().any(|item| {
                                    item.results.iter().any(|result| result.value == operand)
                                })
                            })
                            .map_or(tree_root, |block| block.id)
                    },
                    |block| block.id,
                );
            if operand_block == current {
                visit(function, current, operand, tree_root, found, active);
            }
        }
        active.remove(&value);
    }
    let mut found = BTreeSet::new();
    visit(
        function,
        block_id,
        root_value,
        tree_root,
        &mut found,
        &mut BTreeSet::new(),
    );
    block(function, block_id)
        .into_iter()
        .flat_map(|block| block.instructions.iter())
        .filter(|instruction| found.contains(&instruction.id))
        .map(|instruction| instruction.id)
        .collect()
}

fn instruction_value_uses(instruction: &KirInstruction) -> Vec<ValueId> {
    match &instruction.kind {
        KirInstructionKind::Copy { value }
        | KirInstructionKind::Unary { operand: value, .. }
        | KirInstructionKind::Cast { value, .. }
        | KirInstructionKind::SliceLen { slice: value }
        | KirInstructionKind::SliceData { slice: value } => vec![*value],
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. }
        | KirInstructionKind::MakeSlice {
            data: left,
            len: right,
        } => vec![*left, *right],
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.clone(),
        KirInstructionKind::Guard { condition, .. } => vec![*condition],
        KirInstructionKind::Subslice { slice, start, end } => vec![*slice, *start, *end],
        KirInstructionKind::VersionPredicate { predicate } => predicate
            .conjuncts
            .iter()
            .flat_map(|conjunct| match conjunct {
                crate::KirVersionPredicateConjunct::TripThreshold { value, .. } => vec![*value],
                crate::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                    left,
                    left_count,
                    right,
                    right_count,
                    ..
                } => vec![*left, *left_count, *right, *right_count],
                crate::KirVersionPredicateConjunct::WasmSliceRange {
                    slice,
                    start,
                    count,
                    ..
                } => {
                    vec![*slice, *start, *count]
                }
            })
            .collect(),
        KirInstructionKind::VectorSplat { scalar, .. } => vec![*scalar],
        KirInstructionKind::VectorLoad { access, .. } => {
            vec![access.slice, access.start, access.end]
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            vec![access.slice, access.start, access.end, *value]
        }
        KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => vec![*left, *right],
        KirInstructionKind::VectorUnary { operand, .. }
        | KirInstructionKind::VectorCast { value: operand, .. }
        | KirInstructionKind::VectorExtract {
            vector: operand, ..
        }
        | KirInstructionKind::VectorReduce {
            vector: operand, ..
        } => vec![*operand],
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => vec![*mask, *when_true, *when_false],
        KirInstructionKind::VectorInsert { vector, scalar, .. } => vec![*vector, *scalar],
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Address { .. }
        | KirInstructionKind::Load { .. }
        | KirInstructionKind::Store { .. } => Vec::new(),
    }
}

fn instruction_uses(instruction: &KirInstruction) -> Vec<ValueId> {
    let mut values = instruction_value_uses(instruction);
    match &instruction.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Address { place } => {
            place_values(place, &mut values);
        }
        KirInstructionKind::Store { place, value } => {
            place_values(place, &mut values);
            values.push(*value);
        }
        _ => {}
    }
    values
}

fn place_values(place: &KirPlace, values: &mut Vec<ValueId>) {
    match place {
        KirPlace::Value { value, .. } => values.push(*value),
        KirPlace::Deref { pointer, .. } => values.push(*pointer),
        KirPlace::Index { base, index, .. } => {
            place_values(base, values);
            values.push(*index);
        }
        KirPlace::SliceIndex { slice, index, .. } => values.extend([*slice, *index]),
        KirPlace::Field { base, .. } => place_values(base, values),
    }
}

fn terminator_uses(terminator: &KirTerminator) -> Vec<ValueId> {
    let mut values = Vec::new();
    match terminator {
        KirTerminator::Return { value, .. } => values.extend(value.iter().copied()),
        KirTerminator::Jump { edge } => values.extend(edge.args.iter().copied()),
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            values.push(*condition);
            values.extend(then_edge.args.iter().copied());
            values.extend(else_edge.args.iter().copied());
        }
    }
    values
}

#[derive(Debug, Clone, Copy, Default)]
struct OperationCounts {
    adds: u32,
    subs: u32,
    multiplies: u32,
    negates: u32,
    compares: u32,
}

fn operation_counts(
    function: &crate::KirFunction,
    dag: &[InstructionId],
    branches: usize,
) -> Option<OperationCounts> {
    let mut counts = OperationCounts::default();
    for id in dag {
        let instruction = instruction_def_by_id(function, *id)?;
        match instruction.kind {
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                ..
            } => counts.adds += 1,
            KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                ..
            } => counts.subs += 1,
            KirInstructionKind::Binary {
                op: MirBinaryOp::Mul,
                ..
            } => counts.multiplies += 1,
            KirInstructionKind::Unary {
                op: MirUnaryOp::Neg,
                ..
            } => counts.negates += 1,
            KirInstructionKind::Compare { .. } => counts.compares += 1,
            KirInstructionKind::ConstFloat { .. } => {}
            KirInstructionKind::ConstBool { .. } | KirInstructionKind::Copy { .. } => {}
            _ => return None,
        }
    }
    if counts.compares != u32::try_from(branches).ok()? {
        return None;
    }
    Some(counts)
}

fn scalar_splat_inputs(
    function: &crate::KirFunction,
    state: &TreeBuildState<'_>,
    root_load_value: ValueId,
    dag: &[InstructionId],
) -> BTreeSet<ValueId> {
    let dag_ids = dag.iter().copied().collect::<BTreeSet<_>>();
    let mut inputs = BTreeSet::new();
    for id in dag {
        let Some(instruction) = instruction_def_by_id(function, *id) else {
            continue;
        };
        let Some(block_id) = defining_block(function, instruction.id) else {
            continue;
        };
        for operand in instruction_value_uses(instruction) {
            if !is_f64_value(function, operand)
                || instruction_def(function, operand)
                    .is_some_and(|definition| dag_ids.contains(&definition.id))
            {
                continue;
            }
            let root_value = resolve_value(state, block_id, operand);
            if root_value != root_load_value && is_f64_value(function, root_value) {
                inputs.insert(root_value);
            }
        }
    }
    inputs
}

fn speculation_within_budget(
    function: &crate::KirFunction,
    tree: &WasmDecisionTreeNode,
    state: &TreeBuildState<'_>,
) -> bool {
    fn cost(
        function: &crate::KirFunction,
        node: &WasmDecisionTreeNode,
        root: BlockId,
        state: &TreeBuildState<'_>,
    ) -> Option<(usize, usize)> {
        let block_id = match node {
            WasmDecisionTreeNode::Branch { block, .. }
            | WasmDecisionTreeNode::Leaf { block, .. } => *block,
        };
        let local = if block_id == root {
            0
        } else {
            let block = block(function, block_id)?;
            block
                .instructions
                .iter()
                .filter(|instruction| {
                    state
                        .leaf_store_data
                        .get(&block_id)
                        .is_none_or(|leaf| leaf.increment != instruction.id)
                        && !matches!(instruction.kind, KirInstructionKind::Store { .. })
                })
                .map(|instruction| match instruction.kind {
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Mul,
                        ..
                    } => 2,
                    KirInstructionKind::Binary { .. }
                    | KirInstructionKind::Unary { .. }
                    | KirInstructionKind::Compare { .. }
                    | KirInstructionKind::Cast { .. } => 1,
                    _ => 0,
                })
                .sum()
        };
        match node {
            WasmDecisionTreeNode::Leaf { .. } => Some((local, local)),
            WasmDecisionTreeNode::Branch {
                then_node,
                else_node,
                ..
            } => {
                let (then_total, then_min) = cost(function, then_node, root, state)?;
                let (else_total, else_min) = cost(function, else_node, root, state)?;
                Some((
                    local + then_total + else_total,
                    local + then_min.min(else_min),
                ))
            }
        }
    }
    let Some((total, minimum_path)) = cost(function, tree, state.root, state) else {
        return false;
    };
    let extra = total.saturating_sub(minimum_path);
    let allowed = MAX_SPECULATIVE_EXTRA_OPS
        .saturating_add(minimum_path.saturating_mul(SPECULATIVE_EXTRA_TO_MIN_PATH_RATIO));
    extra <= allowed
}

fn estimate_cost(
    profile: &crate::KirTargetProfile,
    function: &crate::KirFunction,
    dag: &[InstructionId],
    operations: OperationCounts,
    branches: usize,
    splat_inputs: &BTreeSet<ValueId>,
    uf: u8,
) -> Option<(KirCostEstimate, u32)> {
    if !matches!(uf, 1 | 4) {
        return None;
    }
    let f64_lane = KirLaneType::F64;
    let u32_lane = KirLaneType::U32;
    let mut scalar_body = 0_u32;
    let mut vector_chunk = 0_u32;
    for (operation, count) in [
        (KirProfileOperation::Add, operations.adds),
        (KirProfileOperation::Subtract, operations.subs),
        (KirProfileOperation::Multiply, operations.multiplies),
        (KirProfileOperation::Negate, operations.negates),
        (KirProfileOperation::Compare, operations.compares),
    ] {
        let semantics = if operation == KirProfileOperation::Compare {
            KirCostSemantics::NotApplicable
        } else {
            KirCostSemantics::StrictFloat
        };
        scalar_body = scalar_body.checked_add(
            profile_cost(
                profile,
                operation,
                f64_lane,
                1,
                semantics,
                KirAlignmentClass::NotApplicable,
            )?
            .checked_mul(count)?,
        )?;
        vector_chunk = vector_chunk.checked_add(
            profile_cost(
                profile,
                operation,
                f64_lane,
                2,
                semantics,
                KirAlignmentClass::NotApplicable,
            )?
            .checked_mul(count)?,
        )?;
    }
    let branch_count = u32::try_from(branches).ok()?;
    scalar_body = scalar_body.checked_add(
        profile_cost(
            profile,
            KirProfileOperation::Select,
            f64_lane,
            1,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::NotApplicable,
        )?
        .checked_mul(branch_count)?,
    )?;
    vector_chunk = vector_chunk.checked_add(
        profile_cost(
            profile,
            KirProfileOperation::Select,
            f64_lane,
            2,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::NotApplicable,
        )?
        .checked_mul(branch_count)?,
    )?;
    let mut splats = splat_inputs.clone();
    for id in dag {
        if let Some(instruction) = instruction_def_by_id(function, *id)
            && matches!(instruction.kind, KirInstructionKind::ConstFloat { .. })
        {
            splats.extend(instruction.results.iter().map(|result| result.value));
        }
    }
    let splat_cost = profile_cost(
        profile,
        KirProfileOperation::Splat,
        f64_lane,
        2,
        KirCostSemantics::NotApplicable,
        KirAlignmentClass::NotApplicable,
    )?
    .checked_mul(u32::try_from(splats.len()).ok()?)?;
    vector_chunk = vector_chunk.checked_add(splat_cost)?;

    let memory_alignment = KirAlignmentClass::Bytes(8);
    scalar_body = scalar_body
        .checked_add(profile_cost(
            profile,
            KirProfileOperation::Load,
            f64_lane,
            1,
            KirCostSemantics::NotApplicable,
            memory_alignment,
        )?)?
        .checked_add(profile_cost(
            profile,
            KirProfileOperation::Store,
            f64_lane,
            1,
            KirCostSemantics::NotApplicable,
            memory_alignment,
        )?)?;
    vector_chunk = vector_chunk
        .checked_add(profile_cost(
            profile,
            KirProfileOperation::Load,
            f64_lane,
            2,
            KirCostSemantics::NotApplicable,
            memory_alignment,
        )?)?
        .checked_add(profile_cost(
            profile,
            KirProfileOperation::Store,
            f64_lane,
            2,
            KirCostSemantics::NotApplicable,
            memory_alignment,
        )?)?;
    let control_add = profile_cost(
        profile,
        KirProfileOperation::Add,
        u32_lane,
        1,
        KirCostSemantics::Modular,
        KirAlignmentClass::NotApplicable,
    )?;
    let control_compare = profile_control_cost(
        profile,
        KirProfileOperation::Compare,
        u32_lane,
        KirCostSemantics::NotApplicable,
    )?;
    let branch = profile_control_cost(
        profile,
        KirProfileOperation::Branch,
        u32_lane,
        KirCostSemantics::NotApplicable,
    )?;
    let scalar_control = control_add
        .checked_add(control_compare)?
        .checked_add(branch)?;
    scalar_body = scalar_body.checked_add(scalar_control)?;
    let chunk_width = 2_u32.checked_mul(u32::from(uf))?;
    let address_steps = u32::from(uf.checked_sub(1)?);
    vector_chunk = vector_chunk
        .checked_mul(u32::from(uf))?
        .checked_add(control_add.checked_mul(address_steps)?)?
        .checked_add(scalar_control)?;

    let predicate_base = control_compare.checked_add(branch)?;
    let range_predicates = profile_cost(
        profile,
        KirProfileOperation::RuntimePredicate,
        u32_lane,
        2,
        KirCostSemantics::NotApplicable,
        KirAlignmentClass::NotApplicable,
    )?
    .checked_mul(2)?;
    let predicates = predicate_base.checked_add(range_predicates)?;
    let epilogue = branch;
    if u64::from(vector_chunk).saturating_mul(100)
        >= u64::from(scalar_body)
            .saturating_mul(u64::from(chunk_width))
            .saturating_mul(80)
    {
        return None;
    }
    let minimum_trip = (2_u32..=1024)
        .map(|groups| groups * chunk_width)
        .find(|trip| {
            (0..chunk_width).all(|tail| {
                let iterations = trip.saturating_add(tail);
                let scalar = scalar_body.saturating_mul(iterations);
                let transformed = vector_chunk
                    .saturating_mul(*trip / chunk_width)
                    .saturating_add(scalar_body.saturating_mul(tail))
                    .saturating_add(predicates)
                    .saturating_add(epilogue.saturating_mul(u32::from(tail != 0)));
                u64::from(transformed).saturating_mul(100) <= u64::from(scalar).saturating_mul(80)
            })
        })?;
    let priced_tail = chunk_width - 1;
    let priced_trip = minimum_trip.saturating_add(priced_tail);
    let priced_chunks = minimum_trip / chunk_width;
    Some((
        KirCostEstimate::new(
            scalar_body.saturating_mul(priced_trip),
            vector_chunk.saturating_mul(priced_chunks),
            predicates,
            scalar_body
                .saturating_mul(priced_tail)
                .saturating_add(epilogue),
        ),
        minimum_trip,
    ))
}

fn profile_cost(
    profile: &crate::KirTargetProfile,
    operation: KirProfileOperation,
    lane: KirLaneType,
    lanes: u8,
    semantics: KirCostSemantics,
    alignment: KirAlignmentClass,
) -> Option<u32> {
    let key = KirCostKey {
        operation,
        lane,
        lanes,
        semantics,
        alignment,
    };
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        _ => None,
    }
}

fn profile_control_cost(
    profile: &crate::KirTargetProfile,
    operation: KirProfileOperation,
    lane: KirLaneType,
    semantics: KirCostSemantics,
) -> Option<u32> {
    let key = KirCostKey {
        operation,
        lane,
        lanes: 1,
        semantics,
        alignment: KirAlignmentClass::NotApplicable,
    };
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        Some(KirOperationAvailability::Unavailable) if operation == KirProfileOperation::Branch => {
            Some(1)
        }
        _ => None,
    }
}
