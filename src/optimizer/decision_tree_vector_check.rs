use std::collections::{BTreeMap, BTreeSet};

use super::vectorize_check::{
    checked_loop_bound_is_invariant, forwards_from, stable_invariant_descriptor_root,
};
use crate::{
    BlockId, CandidateBudgetCharge, DecisionTreeVectorPlan, InstructionId, KirArithmeticSemantics,
    KirBlock, KirEdge, KirFunction, KirInstruction, KirInstructionKind, KirTerminator,
    KirValueType, KirVerifiedProgramState, MemoryRegionId, MemoryVersionId, MirBinaryOp,
    MirPrimitiveTypeName, MirType, TransactionCheckError, ValueId,
};

type CheckResult<T> = Result<T, TransactionCheckError>;

fn error(message: &str) -> TransactionCheckError {
    TransactionCheckError::compiler(message)
}

/// Checks a closed decision-tree trial using only the immutable scalar KIR,
/// declared target profile, contract evidence, and the emitted typed KIR.
/// Speculative arithmetic is restricted to nontrapping strict f64 operations.
pub fn check_decision_tree_vector_trial_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &DecisionTreeVectorPlan,
    charge: &CandidateBudgetCharge,
) -> CheckResult<()> {
    let vector = &plan.vector;
    let before = pre_state.module();
    let after = trial.module();
    if before.config != after.config
        || before.profile != after.profile
        || before.entry != after.entry
        || before.structs != after.structs
        || !before
            .functions
            .iter()
            .map(|function| function.id)
            .eq(after.functions.iter().map(|function| function.id))
        || before.config.consumer != crate::KirConsumer::WebAssembly
        || before.profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
        || !before.profile.vector_operations_enabled()
        || before.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || before.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || before.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
        || vector.vf != 2
        || !matches!(vector.uf, 1 | 4)
        || !vector.broadcast_groups.is_empty()
    {
        return Err(error(
            "decision-tree target, mode, or module identity differs",
        ));
    }
    if vector.pre_state.kir_digest != pre_state.kir_digest()
        || vector.pre_state.profile_digest != before.profile.digest_hex()
        || vector.pre_state.evidence_generation != pre_state.evidence_generation()
        || trial.evidence_generation() != pre_state.evidence_generation()
        || trial.contract_facts() != pre_state.contract_facts()
        || trial.eliminated_guards() != pre_state.eliminated_guards()
        || trial.optimization_entry_module_units() != pre_state.optimization_entry_module_units()
        || trial.proofs().generation() != pre_state.proofs().generation()
        || trial
            .proofs()
            .proofs()
            .get(..pre_state.proofs().proofs().len())
            != Some(pre_state.proofs().proofs())
    {
        return Err(error("decision-tree pre-state identity is stale"));
    }
    let original = before
        .functions
        .iter()
        .find(|function| function.id == vector.pre_state.function)
        .ok_or_else(|| error("decision-tree source function is missing"))?;
    let transformed = after
        .functions
        .iter()
        .find(|function| function.id == original.id)
        .ok_or_else(|| error("decision-tree trial function is missing"))?;
    if original.name != transformed.name
        || original.exported != transformed.exported
        || original.params != transformed.params
        || original.return_type != transformed.return_type
        || original.regions != transformed.regions
        || original.initial_memory != transformed.initial_memory
        || before
            .functions
            .iter()
            .filter(|function| function.id != original.id)
            .any(|function| {
                after
                    .functions
                    .iter()
                    .find(|target| target.id == function.id)
                    != Some(function)
            })
    {
        return Err(error(
            "decision-tree trial changed ABI, memory metadata, or another function",
        ));
    }
    if vector.pre_state.frozen_kir_units != crate::kir_function_units(original) {
        return Err(error("decision-tree frozen function size is false"));
    }
    let source = reconstruct_source(pre_state, original, transformed, plan)?;
    check_source_plan(original, &source, plan)?;
    let frame = check_trial_frame(pre_state, original, transformed, &source, vector.uf)?;
    check_guard(transformed, &frame, &source, plan)?;
    let splats = check_emitted_tree(original, transformed, &source, &frame, plan)?;
    check_cost_and_charge(
        pre_state,
        trial,
        original,
        transformed,
        &source,
        plan,
        &splats,
        charge,
    )?;
    crate::validate_vectorization_plan(vector, &before.profile)
        .map_err(TransactionCheckError::compiler)?;
    let validation = crate::validate_kir_module(after);
    if !validation.errors.is_empty() {
        return Err(TransactionCheckError::compiler(format!(
            "decision-tree trial KIR is invalid: {:?}",
            validation.errors
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum Node {
    Branch {
        block: BlockId,
        condition: ValueId,
        then_node: Box<Node>,
        else_node: Box<Node>,
    },
    Leaf {
        block: BlockId,
        store: InstructionId,
        value: ValueId,
        edge: KirEdge,
    },
}

#[derive(Debug)]
struct Source {
    preheader: BlockId,
    header: BlockId,
    root: BlockId,
    join: BlockId,
    exit: BlockId,
    induction: ValueId,
    bound: ValueId,
    root_induction: ValueId,
    root_load: InstructionId,
    input: ValueId,
    output: ValueId,
    input_partition: MemoryRegionId,
    output_partition: MemoryRegionId,
    aliases: BTreeMap<ValueId, ValueId>,
    memories: BTreeMap<MemoryVersionId, MemoryVersionId>,
    members: BTreeSet<BlockId>,
    operations: BTreeSet<InstructionId>,
    tree: Node,
    backedge_values: Vec<ScalarState>,
    backedge_memories: Vec<MemoryState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarState {
    Value(ValueId),
    NextInduction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryState {
    Version(MemoryVersionId),
    Stored,
}

struct TreeBuild<'a> {
    function: &'a KirFunction,
    root: BlockId,
    aliases: BTreeMap<ValueId, ValueId>,
    memories: BTreeMap<MemoryVersionId, MemoryVersionId>,
    members: BTreeSet<BlockId>,
    branches: usize,
    leaves: usize,
    join: Option<BlockId>,
}

fn block(function: &KirFunction, id: BlockId) -> CheckResult<&KirBlock> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| error("decision-tree block is missing"))
}
fn instruction(function: &KirFunction, id: InstructionId) -> CheckResult<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
        .ok_or_else(|| error("decision-tree instruction is missing"))
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
fn value_type(function: &KirFunction, value: ValueId) -> Option<KirValueType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| param.type_node.clone().into())
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.params)
                .find(|param| param.value == value)
                .map(|param| param.type_node.clone())
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
fn single(instruction: &KirInstruction) -> CheckResult<ValueId> {
    match instruction.results.as_slice() {
        [result] => Ok(result.value),
        _ => Err(error("decision-tree result arity differs")),
    }
}
fn edges(terminator: &KirTerminator) -> Vec<&KirEdge> {
    match terminator {
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        KirTerminator::Return { .. } => vec![],
    }
}
fn resolved(aliases: &BTreeMap<ValueId, ValueId>, value: ValueId) -> ValueId {
    aliases.get(&value).copied().unwrap_or(value)
}
fn resolved_memory(
    aliases: &BTreeMap<MemoryVersionId, MemoryVersionId>,
    value: MemoryVersionId,
) -> MemoryVersionId {
    aliases.get(&value).copied().unwrap_or(value)
}
fn pure_f64_operation(function: &KirFunction, instruction: &KirInstruction) -> bool {
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
    {
        return false;
    }
    let float = Some(KirValueType::Scalar(MirType::Primitive(
        MirPrimitiveTypeName::F64,
    )));
    match instruction.kind {
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            left,
            right,
            semantics: KirArithmeticSemantics::StrictFloat,
        } => {
            value_type(function, left) == float
                && value_type(function, right) == float
                && Some(instruction.results[0].type_node.clone()) == float
        }
        KirInstructionKind::Unary {
            op: crate::MirUnaryOp::Neg,
            operand,
            semantics: KirArithmeticSemantics::StrictFloat,
        } => {
            value_type(function, operand) == float
                && Some(instruction.results[0].type_node.clone()) == float
        }
        KirInstructionKind::Compare { left, right, .. } => {
            value_type(function, left) == float
                && value_type(function, right) == float
                && instruction.results[0].type_node.as_scalar()
                    == Some(&MirType::Primitive(MirPrimitiveTypeName::Bool))
        }
        _ => false,
    }
}

fn reconstruct_source(
    state: &KirVerifiedProgramState,
    original: &KirFunction,
    transformed: &KirFunction,
    plan: &DecisionTreeVectorPlan,
) -> CheckResult<Source> {
    let changed = original
        .blocks
        .iter()
        .filter(|source| {
            transformed
                .blocks
                .iter()
                .find(|block| block.id == source.id)
                != Some(*source)
        })
        .collect::<Vec<_>>();
    let [preheader] = changed.as_slice() else {
        return Err(error(
            "decision-tree must change exactly its source preheader",
        ));
    };
    let KirTerminator::Jump { edge: entry } = &preheader.terminator else {
        return Err(error("decision-tree source preheader is not a jump"));
    };
    let header = block(original, entry.target)?;
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return Err(error("decision-tree source header is not a loop branch"));
    };
    let crate::VectorEpilogue::Scalar {
        start: induction,
        end: bound,
        ..
    } = plan.vector.epilogue
    else {
        return Err(error("decision-tree needs an exact scalar epilogue"));
    };
    let iv_index = header
        .params
        .iter()
        .position(|param| param.value == induction)
        .ok_or_else(|| error("decision-tree induction is not a header parameter"))?;
    let zero = entry
        .args
        .get(iv_index)
        .and_then(|value| definition(original, *value));
    if !matches!(zero.map(|instruction| &instruction.kind), Some(KirInstructionKind::ConstInt { value }) if value == "0")
    {
        return Err(error("decision-tree induction is not zero based"));
    }
    let comparison = definition(original, *condition)
        .ok_or_else(|| error("decision-tree loop comparison is missing"))?;
    let KirInstructionKind::Compare {
        op: crate::MirCompareOp::Lt,
        left,
        right,
    } = comparison.kind
    else {
        return Err(error(
            "decision-tree loop comparison is not strict increasing",
        ));
    };
    let entry_bound = header
        .params
        .iter()
        .position(|param| param.value == right)
        .and_then(|index| entry.args.get(index).copied())
        .unwrap_or(right);
    if left != induction
        || entry_bound != bound
        || !checked_loop_bound_is_invariant(original, preheader.id, right, bound)
    {
        return Err(error(
            "decision-tree comparison bound is not invariant across every loop edge",
        ));
    }
    check_header_work_is_nontrapping(original, preheader.id, header, entry, comparison, right)?;
    let root = block(original, then_edge.target)?;
    let root_iv_index = then_edge
        .args
        .iter()
        .position(|value| *value == induction)
        .ok_or_else(|| error("decision-tree root induction forwarding is missing"))?;
    let root_induction = root
        .params
        .get(root_iv_index)
        .ok_or_else(|| error("decision-tree root induction parameter is missing"))?
        .value;
    let incoming = original
        .blocks
        .iter()
        .flat_map(|block| {
            edges(&block.terminator)
                .into_iter()
                .map(move |edge| (block.id, edge))
        })
        .filter(|(_, edge)| edge.target == root.id)
        .collect::<Vec<_>>();
    if incoming.len() != 1 || incoming[0].0 != header.id {
        return Err(error(
            "decision-tree root has an external or duplicate entry",
        ));
    }
    let mut builder = TreeBuild {
        function: original,
        root: root.id,
        aliases: root
            .params
            .iter()
            .zip(&then_edge.args)
            .map(|(param, argument)| (param.value, *argument))
            .collect(),
        memories: root
            .memory_params
            .iter()
            .zip(&then_edge.memory_args)
            .map(|(param, argument)| (param.version, *argument))
            .collect(),
        members: BTreeSet::new(),
        branches: 0,
        leaves: 0,
        join: None,
    };
    let tree = builder.visit(root.id, None)?;
    if builder.branches == 0 || builder.branches > 3 || !(2..=4).contains(&builder.leaves) {
        return Err(error("decision-tree exceeds the closed branch/leaf shape"));
    }
    let join = builder
        .join
        .ok_or_else(|| error("decision-tree leaves have no join"))?;
    let join_block = block(original, join)?;
    if !matches!(join_block.terminator, KirTerminator::Jump { ref edge } if edge.target == header.id)
    {
        return Err(error("decision-tree join is not the loop latch"));
    }
    if header
        .instructions
        .iter()
        .any(|instruction| instruction.memory.is_some() || instruction.effect.is_some())
    {
        return Err(error("decision-tree header has observable work"));
    }
    let root_loads = root
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .collect::<Vec<_>>();
    let [load] = root_loads.as_slice() else {
        return Err(error("decision-tree requires exactly one root load"));
    };
    let (input, load_index, _input_region, input_partition) = checked_memory_place(original, load)?;
    if !forwards_from(original, load_index, root_induction) {
        return Err(error(
            "decision-tree root load address is not exact induction",
        ));
    }
    let input = stable_invariant_descriptor_root(original, input)
        .ok_or_else(|| error("decision-tree input slice has no stable descriptor origin"))?;
    let leaves = tree.leaves();
    let mut output = None;
    let mut output_region = None;
    let mut output_partition = None;
    for leaf in &leaves {
        let store = instruction(original, leaf.store)?;
        let (slice, index, region, partition) = checked_memory_place(original, store)?;
        let slice = stable_invariant_descriptor_root(original, slice)
            .ok_or_else(|| error("decision-tree output slice has no stable descriptor origin"))?;
        if resolved(&builder.aliases, index) != induction
            || output.is_some_and(|old| old != slice)
            || output_region.is_some_and(|old| old != region)
            || output_partition.is_some_and(|old| old != partition)
        {
            return Err(error(
                "decision-tree leaves do not store to one exact output address",
            ));
        }
        output = Some(slice);
        output_region = Some(region);
        output_partition = Some(partition);
    }
    let output = output.ok_or_else(|| error("decision-tree has no output"))?;
    if !noalias_available(state, original, preheader.id, input, output) {
        return Err(error(
            "decision-tree NoAlias evidence is not available at the preheader",
        ));
    }
    let mut operations = BTreeSet::new();
    let mut increments = BTreeSet::new();
    for id in builder.members.iter().copied().chain(std::iter::once(join)) {
        for item in &block(original, id)?.instructions {
            if item.id == load.id || leaves.iter().any(|leaf| leaf.store == item.id) {
                continue;
            }
            if pure_f64_operation(original, item) {
                operations.insert(item.id);
                continue;
            }
            match item.kind {
                KirInstructionKind::ConstFloat { .. }
                | KirInstructionKind::ConstInt { .. }
                | KirInstructionKind::Copy { .. }
                    if item.memory.is_none()
                        && item.effect.is_none()
                        && item.results.len() == 1 => {}
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } if item.memory.is_none()
                    && item.effect.is_none()
                    && item.results.len() == 1
                    && ((resolved(&builder.aliases, left) == induction
                        && is_one(original, right))
                        || (resolved(&builder.aliases, right) == induction
                            && is_one(original, left))) =>
                {
                    increments.insert(item.id);
                }
                _ => {
                    return Err(error(
                        "decision-tree includes memory, traps, effects, or unsupported arithmetic",
                    ));
                }
            }
        }
    }
    let (backedge_values, backedge_memories) = check_source_continuation(
        original,
        header,
        root,
        join_block,
        &tree,
        &builder.aliases,
        &builder.memories,
        &builder.members,
        &increments,
        induction,
        input_partition,
        output_partition.expect("leaf partition"),
        load.id,
    )?;
    check_scalar_select_subset(original, root.id, &tree, &builder.members, &increments)?;
    check_scalar_lowering_envelope(
        original,
        preheader.id,
        header.id,
        join,
        else_edge.target,
        &builder.members,
    )?;
    Ok(Source {
        preheader: preheader.id,
        header: header.id,
        root: root.id,
        join,
        exit: else_edge.target,
        induction,
        bound,
        root_induction,
        root_load: load.id,
        input,
        output,
        input_partition,
        output_partition: output_partition.expect("leaf partition"),
        aliases: builder.aliases,
        memories: builder.memories,
        members: builder.members,
        operations,
        tree,
        backedge_values,
        backedge_memories,
    })
}

#[derive(Clone, Copy)]
struct Leaf<'a> {
    block: BlockId,
    store: InstructionId,
    edge: &'a KirEdge,
}
impl Node {
    fn leaves(&self) -> Vec<Leaf<'_>> {
        match self {
            Self::Leaf {
                block, store, edge, ..
            } => vec![Leaf {
                block: *block,
                store: *store,
                edge,
            }],
            Self::Branch {
                then_node,
                else_node,
                ..
            } => then_node
                .leaves()
                .into_iter()
                .chain(else_node.leaves())
                .collect(),
        }
    }
    fn branches(&self) -> Vec<(BlockId, ValueId)> {
        match self {
            Self::Leaf { .. } => vec![],
            Self::Branch {
                block,
                condition,
                then_node,
                else_node,
            } => std::iter::once((*block, *condition))
                .chain(then_node.branches())
                .chain(else_node.branches())
                .collect(),
        }
    }
}
impl TreeBuild<'_> {
    fn visit(&mut self, id: BlockId, parent: Option<(BlockId, &KirEdge)>) -> CheckResult<Node> {
        if self.members.len() >= 7 || !self.members.insert(id) {
            return Err(error(
                "decision-tree has a shared subtree, cycle, or excess nodes",
            ));
        }
        let node = block(self.function, id)?.clone();
        if let Some((parent_id, edge)) = parent {
            let incoming = self
                .function
                .blocks
                .iter()
                .flat_map(|block| {
                    edges(&block.terminator)
                        .into_iter()
                        .map(move |edge| (block.id, edge))
                })
                .filter(|(_, edge)| edge.target == id)
                .collect::<Vec<_>>();
            if incoming.len() != 1
                || incoming[0].0 != parent_id
                || incoming[0].1 != edge
                || edge.args.len() != node.params.len()
                || edge.memory_args.len() != node.memory_params.len()
            {
                return Err(error(
                    "decision-tree child edge is not its unique exact predecessor",
                ));
            }
            for (param, argument) in node.params.iter().zip(&edge.args) {
                if value_type(self.function, *argument).as_ref() != Some(&param.type_node) {
                    return Err(error("decision-tree source edge parameter type differs"));
                }
                self.aliases
                    .insert(param.value, resolved(&self.aliases, *argument));
            }
            for (param, argument) in node.memory_params.iter().zip(&edge.memory_args) {
                self.memories
                    .insert(param.version, resolved_memory(&self.memories, *argument));
            }
        }
        for instruction in &node.instructions {
            if let KirInstructionKind::Copy { value } = instruction.kind {
                if instruction.memory.is_some() || instruction.effect.is_some() {
                    return Err(error("decision-tree copy has observable effects"));
                }
                self.aliases
                    .insert(single(instruction)?, resolved(&self.aliases, value));
            }
        }
        match &node.terminator {
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                self.branches += 1;
                if self.branches > 3
                    || then_edge.target == else_edge.target
                    || [then_edge, else_edge].iter().any(|edge| {
                        edge.memory_args.len() != node.memory_params.len()
                            || node
                                .memory_params
                                .iter()
                                .zip(&edge.memory_args)
                                .any(|(param, argument)| param.version != *argument)
                    })
                    || node.instructions.iter().any(|instruction| {
                        matches!(instruction.kind, KirInstructionKind::Store { .. })
                            || (id != self.root && instruction.memory.is_some())
                    })
                {
                    return Err(error(
                        "decision-tree branch has stores, additional reads, or excess decisions",
                    ));
                }
                let condition = resolved(&self.aliases, *condition);
                let compare = definition(self.function, condition)
                    .ok_or_else(|| error("decision-tree condition is not a comparison"))?;
                if !matches!(compare.kind, KirInstructionKind::Compare { .. })
                    || !pure_f64_operation(self.function, compare)
                {
                    return Err(error(
                        "decision-tree condition does not preserve strict f64 comparison semantics",
                    ));
                }
                let then_node = self.visit(then_edge.target, Some((id, then_edge)))?;
                let else_node = self.visit(else_edge.target, Some((id, else_edge)))?;
                Ok(Node::Branch {
                    block: id,
                    condition,
                    then_node: Box::new(then_node),
                    else_node: Box::new(else_node),
                })
            }
            KirTerminator::Jump { edge } => {
                self.leaves += 1;
                let stores = node
                    .instructions
                    .iter()
                    .filter(|instruction| {
                        matches!(instruction.kind, KirInstructionKind::Store { .. })
                    })
                    .collect::<Vec<_>>();
                let [store] = stores.as_slice() else {
                    return Err(error("decision-tree leaf must contain exactly one store"));
                };
                if self.leaves > 4
                    || self.join.is_some_and(|join| join != edge.target)
                    || node.instructions.iter().any(|instruction| {
                        matches!(instruction.kind, KirInstructionKind::Load { .. })
                    })
                {
                    return Err(error(
                        "decision-tree leaf has reads or a different continuation",
                    ));
                }
                self.join = Some(edge.target);
                let KirInstructionKind::Store { value, .. } = store.kind else {
                    unreachable!()
                };
                Ok(Node::Leaf {
                    block: id,
                    store: store.id,
                    value: resolved(&self.aliases, value),
                    edge: edge.clone(),
                })
            }
            KirTerminator::Return { .. } => Err(error(
                "decision-tree leaf exits instead of joining the loop",
            )),
        }
    }
}

fn checked_memory_place(
    function: &KirFunction,
    instruction: &KirInstruction,
) -> CheckResult<(ValueId, ValueId, MemoryRegionId, MemoryRegionId)> {
    let (place, store) = match &instruction.kind {
        KirInstructionKind::Load { place } => (place.as_ref(), false),
        KirInstructionKind::Store { place, value } => {
            if value_type(function, *value)
                != Some(KirValueType::Scalar(MirType::Primitive(
                    MirPrimitiveTypeName::F64,
                )))
            {
                return Err(error("decision-tree stored value is not f64"));
            }
            (place.as_ref(), true)
        }
        _ => return Err(error("decision-tree memory source is not load/store")),
    };
    let crate::KirPlace::SliceIndex {
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
        .ok_or_else(|| error("decision-tree source region is missing"))?;
    if *type_node != MirType::Primitive(MirPrimitiveTypeName::F64)
        || !matches!(descriptor.origin,
            crate::KirMemoryRegionOrigin::Parameter(origin)
                if stable_invariant_descriptor_root(function, *slice) == Some(origin))
        || value_type(function, *index)
            != Some(KirValueType::Scalar(MirType::Primitive(
                MirPrimitiveTypeName::U32,
            )))
        || descriptor.partition != memory.region
        || !function
            .regions
            .iter()
            .any(|region| region.id == memory.region && region.partition == memory.region)
        || memory.output.is_some() != store
        || instruction.effect.as_ref().is_none_or(|effect| {
            effect.kind
                != if store {
                    crate::KirEffectKind::WriteMemory
                } else {
                    crate::KirEffectKind::ReadMemory
                }
        })
        || if store {
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
    Ok((*slice, *index, *region, memory.region))
}
fn is_one(function: &KirFunction, value: ValueId) -> bool {
    matches!(definition(function, value).map(|instruction| &instruction.kind), Some(KirInstructionKind::ConstInt { value }) if value == "1")
}

fn check_header_work_is_nontrapping(
    function: &KirFunction,
    preheader: BlockId,
    header: &KirBlock,
    entry: &KirEdge,
    comparison: &KirInstruction,
    bound: ValueId,
) -> CheckResult<()> {
    fn bound_source(
        function: &KirFunction,
        preheader: BlockId,
        header: &KirBlock,
        entry: &KirEdge,
        value: ValueId,
        visiting: &mut BTreeSet<ValueId>,
        allowed: &mut BTreeSet<InstructionId>,
    ) -> bool {
        if function.params.iter().any(|param| param.value == value) {
            return true;
        }
        if !visiting.insert(value) {
            return false;
        }
        let result = if let Some(slot) = header.params.iter().position(|param| param.value == value)
        {
            entry.args.get(slot).is_some_and(|argument| {
                bound_source(
                    function, preheader, header, entry, *argument, visiting, allowed,
                )
            })
        } else if let Some(block) = function
            .blocks
            .iter()
            .find(|block| block.params.iter().any(|param| param.value == value))
        {
            crate::compute_kir_dominators(function).dominates(block.id, preheader)
        } else if let Some((block, instruction)) = function.blocks.iter().find_map(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == value)
                })
                .map(|instruction| (block, instruction))
        }) {
            if block.id != header.id {
                crate::compute_kir_dominators(function).dominates(block.id, preheader)
            } else if instruction.memory.is_some()
                || instruction.effect.is_some()
                || instruction.results.len() != 1
                || instruction.results[0].value != value
            {
                false
            } else {
                let safe = match instruction.kind {
                    KirInstructionKind::ConstInt { .. } => {
                        value_type(function, value)
                            == Some(KirValueType::Scalar(MirType::Primitive(
                                MirPrimitiveTypeName::U32,
                            )))
                    }
                    KirInstructionKind::Copy { value: source } => bound_source(
                        function, preheader, header, entry, source, visiting, allowed,
                    ),
                    KirInstructionKind::SliceLen { slice } => {
                        value_type(function, value)
                            == Some(KirValueType::Scalar(MirType::Primitive(
                                MirPrimitiveTypeName::U32,
                            )))
                            && bound_source(
                                function, preheader, header, entry, slice, visiting, allowed,
                            )
                    }
                    _ => false,
                };
                if safe {
                    allowed.insert(instruction.id);
                }
                safe
            }
        } else {
            false
        };
        visiting.remove(&value);
        result
    }

    if !header
        .instructions
        .iter()
        .any(|instruction| instruction.id == comparison.id)
        || comparison.memory.is_some()
        || comparison.effect.is_some()
        || comparison.results.len() != 1
    {
        return Err(error(
            "decision-tree loop comparison has observable or displaced work",
        ));
    }
    let mut allowed = BTreeSet::from([comparison.id]);
    if !bound_source(
        function,
        preheader,
        header,
        entry,
        bound,
        &mut BTreeSet::new(),
        &mut allowed,
    ) || header
        .instructions
        .iter()
        .any(|instruction| !allowed.contains(&instruction.id))
    {
        return Err(error(
            "decision-tree header contains skipped or trapping work",
        ));
    }
    Ok(())
}

fn noalias_available(
    state: &KirVerifiedProgramState,
    function: &KirFunction,
    preheader: BlockId,
    left: ValueId,
    right: ValueId,
) -> bool {
    let dominators = crate::compute_kir_dominators(function);
    state.contract_facts().is_some_and(|contracts| contracts.facts().facts().iter().any(|fact| {
        let available = match &fact.scope { crate::FactScope::FunctionEntry(owner) => *owner == function.id, crate::FactScope::Block { function: owner, block } => *owner == function.id && dominators.dominates(*block, preheader), _ => false };
        available && fact.generation == state.evidence_generation()
            && matches!(fact.origin, crate::FactOrigin::TrustedContract { .. })
            && fact.derivation == crate::FactDerivation::TrustedContractLeaf
            && matches!(fact.predicate, crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias { left: a, right: b }) if (forwards_from(function, left, a) && forwards_from(function, right, b)) || (forwards_from(function, left, b) && forwards_from(function, right, a)))
    }))
}
fn source_operation(
    instruction: &KirInstruction,
) -> CheckResult<(crate::KirProfileOperation, crate::KirCostSemantics)> {
    use crate::KirProfileOperation as Op;
    let operation = match instruction.kind {
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            ..
        } => Op::Add,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            ..
        } => Op::Subtract,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Mul,
            ..
        } => Op::Multiply,
        KirInstructionKind::Unary {
            op: crate::MirUnaryOp::Neg,
            ..
        } => Op::Negate,
        KirInstructionKind::Compare { .. } => {
            return Ok((Op::Compare, crate::KirCostSemantics::NotApplicable));
        }
        _ => return Err(error("decision-tree source operation is unsupported")),
    };
    Ok((operation, crate::KirCostSemantics::StrictFloat))
}
fn check_source_plan(
    function: &KirFunction,
    source: &Source,
    plan: &DecisionTreeVectorPlan,
) -> CheckResult<()> {
    let vector = &plan.vector;
    let uf = vector.uf;
    if !matches!(uf, 1 | 4)
        || vector.operations.len() != source.operations.len().saturating_mul(usize::from(uf))
    {
        return Err(error("decision-tree operation coverage is incomplete"));
    }
    let mut operation_ids = BTreeSet::new();
    let mut emitted_ids = BTreeSet::new();
    for mapping in &vector.operations {
        if !source.operations.contains(&mapping.scalar)
            || mapping.unroll_index >= uf
            || !operation_ids.insert((mapping.scalar, mapping.unroll_index))
            || !emitted_ids.insert(mapping.vector)
        {
            return Err(error("decision-tree operation identity is false"));
        }
        let (operation, semantics) = source_operation(instruction(function, mapping.scalar)?)?;
        if mapping.operation != operation
            || mapping.semantics != semantics
            || mapping.lane_type != crate::KirLaneType::F64
            || mapping.alignment != crate::KirAlignmentClass::NotApplicable
            || mapping.unroll_index >= uf
            || mapping.lanes.as_slice()
                != [
                    crate::VectorLaneMapping {
                        lane: 0,
                        scalar_iteration: u32::from(mapping.unroll_index) * 2,
                    },
                    crate::VectorLaneMapping {
                        lane: 1,
                        scalar_iteration: u32::from(mapping.unroll_index) * 2 + 1,
                    },
                ]
        {
            return Err(error(
                "decision-tree exact lane or strict operation mapping differs",
            ));
        }
    }
    let expected_operations = source
        .operations
        .iter()
        .flat_map(|scalar| (0..uf).map(move |unroll_index| (*scalar, unroll_index)))
        .collect::<BTreeSet<_>>();
    if operation_ids != expected_operations {
        return Err(error(
            "decision-tree unrolled operation coverage is not exact",
        ));
    }
    let branches = source
        .tree
        .branches()
        .into_iter()
        .map(|(block, _)| block)
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeSet::new();
    for mapping in &plan.selects {
        if !branches.contains(&mapping.source_branch)
            || mapping.unroll_index >= uf
            || !selected.insert((mapping.source_branch, mapping.unroll_index))
            || !emitted_ids.insert(mapping.vector_select)
        {
            return Err(error("decision-tree select mapping identity differs"));
        }
    }
    let expected_selects = branches
        .iter()
        .flat_map(|branch| (0..uf).map(move |unroll_index| (*branch, unroll_index)))
        .collect::<BTreeSet<_>>();
    if selected != expected_selects
        || plan.selects.windows(2).any(|pair| {
            (pair[0].source_branch, pair[0].unroll_index)
                >= (pair[1].source_branch, pair[1].unroll_index)
        })
    {
        return Err(error("decision-tree select coverage is not exact"));
    }
    let stores = source
        .tree
        .leaves()
        .iter()
        .map(|leaf| leaf.store)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if vector.memory_groups.len() != usize::from(uf) * 2 {
        return Err(error(
            "decision-tree requires exactly one load and store group",
        ));
    }
    let mut load_seen = BTreeSet::new();
    let mut store_seen = BTreeSet::new();
    for group in &vector.memory_groups {
        if group.unroll_index >= uf || !emitted_ids.insert(group.vector_instruction) {
            return Err(error("decision-tree vector memory identity differs"));
        }
        match group.access {
            crate::VectorMemoryAccessKind::Read
                if load_seen.insert(group.unroll_index)
                    && group.region == source.input_partition
                    && group.scalar_instructions == [source.root_load] => {}
            crate::VectorMemoryAccessKind::Write
                if store_seen.insert(group.unroll_index)
                    && group.region == source.output_partition
                    && group.scalar_instructions == stores => {}
            _ => {
                return Err(error(
                    "decision-tree memory groups do not cover the mutually exclusive scalar footprint",
                ));
            }
        }
    }
    if load_seen != (0..uf).collect() || store_seen != (0..uf).collect() {
        return Err(error("decision-tree load/store coverage is incomplete"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_source_continuation(
    function: &KirFunction,
    header: &KirBlock,
    root: &KirBlock,
    join: &KirBlock,
    tree: &Node,
    aliases: &BTreeMap<ValueId, ValueId>,
    memories: &BTreeMap<MemoryVersionId, MemoryVersionId>,
    members: &BTreeSet<BlockId>,
    increments: &BTreeSet<InstructionId>,
    induction: ValueId,
    _input_partition: MemoryRegionId,
    output_partition: MemoryRegionId,
    root_load: InstructionId,
) -> CheckResult<(Vec<ScalarState>, Vec<MemoryState>)> {
    if members.contains(&join.id) || join.id == header.id || !join.instructions.is_empty() {
        return Err(error(
            "decision-tree continuation is not an empty external canonical latch",
        ));
    }
    let KirTerminator::Jump { edge: latch } = &join.terminator else {
        return Err(error("decision-tree latch is not a jump"));
    };
    if latch.target != header.id
        || latch.args.len() != header.params.len()
        || latch.memory_args.len() != header.memory_params.len()
    {
        return Err(error("decision-tree latch state arity differs"));
    }
    let leaves = tree.leaves();
    let predecessors = function
        .blocks
        .iter()
        .flat_map(|block| {
            edges(&block.terminator)
                .into_iter()
                .filter(|edge| edge.target == join.id)
                .map(move |_| block.id)
        })
        .collect::<Vec<_>>();
    if predecessors.len() != leaves.len()
        || predecessors.into_iter().collect::<BTreeSet<_>>()
            != leaves.iter().map(|leaf| leaf.block).collect()
    {
        return Err(error("decision-tree latch has an external incoming path"));
    }
    let mut values = BTreeMap::new();
    let mut varying = 0;
    let increment_values = increments
        .iter()
        .map(|id| instruction(function, *id).and_then(single))
        .collect::<CheckResult<BTreeSet<_>>>()?;
    for (index, param) in join.params.iter().enumerate() {
        let column = leaves
            .iter()
            .map(|leaf| {
                leaf.edge
                    .args
                    .get(index)
                    .copied()
                    .map(|value| resolved(aliases, value))
                    .ok_or_else(|| error("decision-tree leaf omits a join argument"))
            })
            .collect::<CheckResult<Vec<_>>>()?;
        let first = *column
            .first()
            .ok_or_else(|| error("decision-tree has no leaves"))?;
        let next = if column.iter().all(|value| increment_values.contains(value)) {
            if column.iter().any(|value| *value != first) {
                varying += 1;
            }
            for (leaf, value) in leaves.iter().zip(&column) {
                let update = definition(function, *value)
                    .ok_or_else(|| error("decision-tree leaf induction update is undefined"))?;
                if !block(function, leaf.block)?
                    .instructions
                    .iter()
                    .any(|instruction| instruction.id == update.id)
                    || leaf.edge.args.get(index) != Some(value)
                {
                    return Err(error(
                        "decision-tree varying join value is not the local unit induction result",
                    ));
                }
                for source in &function.blocks {
                    for instruction in &source.instructions {
                        let mut used = false;
                        super::analysis::visit_instruction_uses(instruction, &mut |operand| {
                            used |= operand == *value;
                        });
                        if used {
                            return Err(error(
                                "decision-tree induction result escapes its sole latch column",
                            ));
                        }
                    }
                    for edge in edges(&source.terminator) {
                        for (position, argument) in edge.args.iter().enumerate() {
                            if *argument == *value
                                && (source.id != leaf.block
                                    || edge.target != join.id
                                    || position != index)
                            {
                                return Err(error(
                                    "decision-tree induction result has another edge use",
                                ));
                            }
                        }
                    }
                }
            }
            ScalarState::NextInduction
        } else {
            if column.iter().any(|value| *value != first) {
                return Err(error(
                    "decision-tree join carries path-dependent scalar state",
                ));
            }
            ScalarState::Value(first)
        };
        values.insert(param.value, next);
    }
    if varying > 1 || increments.len() != leaves.len() {
        return Err(error(
            "decision-tree does not have one equivalent unit update per leaf",
        ));
    }
    let scalar_state = latch
        .args
        .iter()
        .map(|value| {
            values
                .get(value)
                .copied()
                .unwrap_or(ScalarState::Value(resolved(aliases, *value)))
        })
        .collect::<Vec<_>>();
    for (param, value) in header.params.iter().zip(&scalar_state) {
        let expected = if param.value == induction {
            ScalarState::NextInduction
        } else {
            ScalarState::Value(param.value)
        };
        if *value != expected {
            return Err(error(
                "decision-tree backedge changes scalar state beyond the unit induction",
            ));
        }
    }
    let mut memory_state = BTreeMap::new();
    for (index, param) in join.memory_params.iter().enumerate() {
        let mut column = Vec::new();
        for leaf in &leaves {
            let version = *leaf
                .edge
                .memory_args
                .get(index)
                .ok_or_else(|| error("decision-tree leaf omits join memory"))?;
            let store = instruction(function, leaf.store)?
                .memory
                .as_ref()
                .ok_or_else(|| error("decision-tree leaf store memory is missing"))?;
            let leaf_state = if Some(version) == store.output {
                MemoryState::Stored
            } else {
                MemoryState::Version(resolved_memory(memories, version))
            };
            if (param.region == output_partition) != (leaf_state == MemoryState::Stored) {
                return Err(error(
                    "decision-tree join memory does not select exactly its leaf store output",
                ));
            }
            column.push(leaf_state);
        }
        let first = *column
            .first()
            .ok_or_else(|| error("decision-tree memory column is empty"))?;
        if column.iter().any(|value| *value != first) {
            return Err(error("decision-tree join memory differs by path"));
        }
        memory_state.insert(param.version, first);
    }
    let backedge_memory = latch
        .memory_args
        .iter()
        .map(|version| {
            memory_state
                .get(version)
                .copied()
                .unwrap_or(MemoryState::Version(resolved_memory(memories, *version)))
        })
        .collect::<Vec<_>>();
    for (param, state) in header.memory_params.iter().zip(&backedge_memory) {
        let expected = if param.region == output_partition {
            MemoryState::Stored
        } else {
            MemoryState::Version(param.version)
        };
        if *state != expected {
            return Err(error("decision-tree backedge MemorySSA state differs"));
        }
    }
    let load = instruction(function, root_load)?;
    for leaf in &leaves {
        let store = instruction(function, leaf.store)?;
        let memory = store.memory.as_ref().expect("checked store memory");
        let expected = header
            .memory_params
            .iter()
            .find(|param| param.region == output_partition)
            .ok_or_else(|| error("decision-tree output memory is absent from header"))?
            .version;
        if resolved_memory(memories, memory.input) != expected
            || store.effect.as_ref().expect("checked store effect").order
                <= load.effect.as_ref().expect("checked load effect").order
        {
            return Err(error(
                "decision-tree source load/store memory or effect order differs",
            ));
        }
    }
    let load_memory = load.memory.as_ref().expect("checked load memory");
    let source_input = header
        .memory_params
        .iter()
        .find(|param| param.region == load_memory.region)
        .ok_or_else(|| error("decision-tree input memory is absent from header"))?
        .version;
    if resolved_memory(memories, load_memory.input) != source_input || root.memory_params.is_empty()
    {
        return Err(error(
            "decision-tree root load does not use loop entry memory",
        ));
    }
    Ok((scalar_state, backedge_memory))
}

fn is_dead_pure_undef(function: &KirFunction, instruction: &KirInstruction) -> bool {
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
            super::analysis::visit_instruction_uses(candidate, &mut |operand| {
                used |= operand == value;
            });
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

fn check_scalar_lowering_envelope(
    function: &KirFunction,
    preheader: BlockId,
    header: BlockId,
    join: BlockId,
    exit: BlockId,
    members: &BTreeSet<BlockId>,
) -> CheckResult<()> {
    let mut allowed = members.clone();
    allowed.extend([preheader, header, join, exit]);
    if !function.exported
        || function.return_type != MirType::Void
        || !function.vector_regions.is_empty()
        || function.blocks.first().map(|block| block.id) != Some(preheader)
        || function.blocks.len() != allowed.len()
        || function
            .blocks
            .iter()
            .any(|block| !allowed.contains(&block.id))
    {
        return Err(error(
            "decision-tree scalar lowering cost is not proven for the whole function",
        ));
    }
    let safe_type = |type_node: &MirType| {
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
        .any(|param| !safe_type(&param.type_node))
        || function.blocks.iter().any(|block| {
            block.params.iter().any(|param| {
                param
                    .type_node
                    .as_scalar()
                    .is_none_or(|kind| !safe_type(kind))
            }) || block
                .instructions
                .iter()
                .flat_map(|instruction| &instruction.results)
                .any(|result| {
                    result
                        .type_node
                        .as_scalar()
                        .is_none_or(|kind| !safe_type(kind))
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
        return Err(error(
            "decision-tree scalar lowering cost has an unsupported typed action",
        ));
    }
    let entry = block(function, preheader)?;
    if !matches!(&entry.terminator, KirTerminator::Jump { edge } if edge.target == header)
        || entry.instructions.iter().any(|instruction| {
            instruction.memory.is_some()
                || instruction.effect.is_some()
                || !(matches!(
                    instruction.kind,
                    KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::ConstFloat { .. }
                        | KirInstructionKind::ConstBool { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::SliceLen { .. }
                ) || is_dead_pure_undef(function, instruction))
        })
    {
        return Err(error(
            "decision-tree scalar entry is not a simple nontrapping preheader",
        ));
    }
    let latch = block(function, join)?;
    let end = block(function, exit)?;
    if !latch.instructions.is_empty()
        || !matches!(&latch.terminator, KirTerminator::Jump { edge } if edge.target == header)
        || !end.instructions.is_empty()
        || !matches!(end.terminator, KirTerminator::Return { value: None, .. })
    {
        return Err(error(
            "decision-tree scalar latch or exit has extra lowering work",
        ));
    }
    Ok(())
}

fn check_scalar_select_subset(
    function: &KirFunction,
    root: BlockId,
    tree: &Node,
    members: &BTreeSet<BlockId>,
    increments: &BTreeSet<InstructionId>,
) -> CheckResult<()> {
    if function
        .blocks
        .iter()
        .filter(|block| members.contains(&block.id))
        .any(|block| {
            block
                .params
                .iter()
                .any(|param| param.type_node.as_scalar().is_none())
                || block
                    .instructions
                    .iter()
                    .flat_map(|instruction| &instruction.results)
                    .any(|result| result.type_node.as_scalar().is_none())
        })
    {
        return Err(error(
            "decision-tree scalar cost premise contains vector values",
        ));
    }
    // Scalar selection compares address and continuation identities at the
    // root block boundary. Global equivalence is insufficient for that cost
    // premise: two root parameters can happen to carry the same descriptor.
    fn root_value(
        function: &KirFunction,
        root: BlockId,
        mut owner: BlockId,
        mut value: ValueId,
    ) -> CheckResult<ValueId> {
        let mut visited = BTreeSet::new();
        while owner != root {
            if !visited.insert(owner) {
                return Err(error("decision-tree scalar boundary contains a cycle"));
            }
            let Some(index) = block(function, owner)?
                .params
                .iter()
                .position(|param| param.value == value)
            else {
                break;
            };
            let incoming = function
                .blocks
                .iter()
                .flat_map(|block| {
                    edges(&block.terminator)
                        .into_iter()
                        .map(move |edge| (block.id, edge))
                })
                .filter(|(_, edge)| edge.target == owner)
                .collect::<Vec<_>>();
            let [(parent, edge)] = incoming.as_slice() else {
                return Err(error("decision-tree scalar boundary has multiple entries"));
            };
            value = *edge
                .args
                .get(index)
                .ok_or_else(|| error("decision-tree scalar boundary argument is missing"))?;
            owner = *parent;
        }
        Ok(value)
    }
    let leaves = tree.leaves();
    let join = block(function, leaves[0].edge.target)?;
    let mut address = None;
    for leaf in &leaves {
        let store = instruction(function, leaf.store)?;
        let (slice, index, region, partition) = checked_memory_place(function, store)?;
        let key = (
            root_value(function, root, leaf.block, slice)?,
            root_value(function, root, leaf.block, index)?,
            region,
        );
        if address.replace(key).is_some_and(|previous| previous != key)
            || !block(function, leaf.block)?
                .memory_params
                .iter()
                .any(|param| {
                    param.region == partition
                        && Some(param.version) == store.memory.as_ref().map(|memory| memory.input)
                })
        {
            return Err(error(
                "decision-tree scalar select address or local memory identity differs",
            ));
        }
    }
    let mut varying = 0;
    for index in 0..join.params.len() {
        let column = leaves
            .iter()
            .map(|leaf| {
                leaf.edge
                    .args
                    .get(index)
                    .copied()
                    .ok_or_else(|| error("decision-tree scalar join column is missing"))
                    .and_then(|value| root_value(function, root, leaf.block, value))
            })
            .collect::<CheckResult<Vec<_>>>()?;
        if column.iter().all(|value| *value == column[0]) {
            continue;
        }
        varying += 1;
        let mut common_base = None;
        for (leaf, value) in leaves.iter().zip(column) {
            let leaf_block = block(function, leaf.block)?;
            let update = leaf_block
                .instructions
                .iter()
                .find(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == value)
                })
                .ok_or_else(|| {
                    error("decision-tree scalar varying column is not a local update")
                })?;
            let KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = update.kind
            else {
                return Err(error(
                    "decision-tree scalar varying column is not a modular increment",
                ));
            };
            let base = if is_one(function, right) {
                left
            } else if is_one(function, left) {
                right
            } else {
                return Err(error("decision-tree scalar increment does not add one"));
            };
            if !leaf_block.params.iter().any(|param| param.value == base) {
                return Err(error(
                    "decision-tree scalar increment base is not a leaf parameter",
                ));
            }
            let base = root_value(function, root, leaf.block, base)?;
            if !block(function, root)?
                .params
                .iter()
                .any(|param| param.value == base)
                || common_base
                    .replace(base)
                    .is_some_and(|previous| previous != base)
                || !increments.contains(&update.id)
            {
                return Err(error(
                    "decision-tree scalar increments do not share one root parameter",
                ));
            }
        }
    }
    if varying != 1 {
        return Err(error(
            "decision-tree scalar selection requires one shared induction column",
        ));
    }
    // Scalar all-arm pricing is valid only when the committed scalar lowering
    // can also evaluate every decision without moving a conditional trap.
    // Reconstruct that premise from definitions instead of trusting the plan.
    fn safe_condition(
        function: &KirFunction,
        root: BlockId,
        value: ValueId,
        visited: &mut BTreeSet<ValueId>,
    ) -> bool {
        if !visited.insert(value)
            || function.params.iter().any(|param| param.value == value)
            || function
                .blocks
                .iter()
                .any(|block| block.params.iter().any(|param| param.value == value))
        {
            return true;
        }
        let Some((owner, instruction)) = function.blocks.iter().find_map(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == value)
                })
                .map(|instruction| (block.id, instruction))
        }) else {
            return false;
        };
        let allowed = match instruction.kind {
            KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::ConstFloat { .. }
            | KirInstructionKind::ConstBool { .. }
            | KirInstructionKind::Copy { .. }
            | KirInstructionKind::Compare { .. }
            | KirInstructionKind::Cast { .. }
            | KirInstructionKind::Unary { .. } => {
                instruction.memory.is_none() && instruction.effect.is_none()
            }
            KirInstructionKind::Binary { op, semantics, .. } => {
                !matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod)
                    && matches!(
                        semantics,
                        KirArithmeticSemantics::Modular | KirArithmeticSemantics::StrictFloat
                    )
                    && instruction.memory.is_none()
                    && instruction.effect.is_none()
            }
            KirInstructionKind::Load { .. } => {
                owner == root
                    && instruction
                        .effect
                        .as_ref()
                        .is_some_and(|effect| effect.kind == crate::KirEffectKind::ReadMemory)
                    && instruction
                        .memory
                        .as_ref()
                        .is_some_and(|memory| memory.output.is_none())
            }
            _ => false,
        };
        let mut operands_safe = true;
        super::analysis::visit_instruction_uses(instruction, &mut |operand| {
            operands_safe &= safe_condition(function, root, operand, visited);
        });
        allowed && operands_safe
    }
    if tree
        .branches()
        .iter()
        .any(|(_, condition)| !safe_condition(function, root, *condition, &mut BTreeSet::new()))
    {
        return Err(error(
            "decision-tree scalar select condition can speculate a trap",
        ));
    }
    let local_values = function
        .blocks
        .iter()
        .filter(|block| members.contains(&block.id))
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
        .filter(|block| !members.contains(&block.id))
    {
        for instruction in &block.instructions {
            let mut escapes = false;
            super::analysis::visit_instruction_uses(instruction, &mut |value| {
                escapes |= local_values.contains(&value);
            });
            if escapes {
                return Err(error(
                    "decision-tree source value escapes the closed scalar select region",
                ));
            }
        }
        let mut uses = edges(&block.terminator)
            .into_iter()
            .flat_map(|edge| edge.args.iter().copied())
            .collect::<Vec<_>>();
        match &block.terminator {
            KirTerminator::Branch { condition, .. } => uses.push(*condition),
            KirTerminator::Return { value, .. } => uses.extend(value),
            _ => {}
        }
        if uses.iter().any(|value| local_values.contains(value)) {
            return Err(error(
                "decision-tree source value escapes through external control flow",
            ));
        }
    }
    fn cost(
        function: &KirFunction,
        root: BlockId,
        node: &Node,
        increments: &BTreeSet<InstructionId>,
    ) -> CheckResult<(u32, u32)> {
        let id = match node {
            Node::Branch { block, .. } | Node::Leaf { block, .. } => *block,
        };
        let own = if id == root {
            0
        } else {
            block(function, id)?
                .instructions
                .iter()
                .filter(|instruction| !increments.contains(&instruction.id))
                .map(|instruction| match instruction.kind {
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Mul,
                        ..
                    } => 2_u32,
                    KirInstructionKind::Binary { .. }
                    | KirInstructionKind::Unary { .. }
                    | KirInstructionKind::Compare { .. } => 1,
                    _ => 0,
                })
                .fold(0_u32, u32::saturating_add)
        };
        match node {
            Node::Leaf { .. } => Ok((own, own)),
            Node::Branch {
                then_node,
                else_node,
                ..
            } => {
                let left = cost(function, root, then_node, increments)?;
                let right = cost(function, root, else_node, increments)?;
                Ok((
                    own.saturating_add(left.0).saturating_add(right.0),
                    own.saturating_add(left.1.min(right.1)),
                ))
            }
        }
    }
    let (total, cheapest) = cost(function, root, tree, increments)?;
    if total.saturating_sub(cheapest) > 16_u32.saturating_add(cheapest.saturating_mul(4)) {
        return Err(TransactionCheckError::reject(
            "decision-tree-speculative-work-budget",
        ));
    }
    Ok(())
}

struct Frame<'a> {
    preheader: &'a KirBlock,
    header: &'a KirBlock,
    body: &'a KirBlock,
    entry_bound: ValueId,
    condition: ValueId,
    region: crate::VectorRegionId,
    scalar_map: BTreeMap<ValueId, ValueId>,
    scalar_aliases: BTreeMap<ValueId, ValueId>,
    memory_map: BTreeMap<MemoryVersionId, MemoryVersionId>,
    body_induction: ValueId,
    next_induction: ValueId,
    uf: u8,
    control: BTreeSet<InstructionId>,
}

fn check_trial_frame<'a>(
    state: &KirVerifiedProgramState,
    original: &KirFunction,
    transformed: &'a KirFunction,
    source: &Source,
    uf: u8,
) -> CheckResult<Frame<'a>> {
    if !matches!(uf, 1 | 4) {
        return Err(error("decision-tree unroll factor is unsupported"));
    }
    if transformed.blocks.len() != original.blocks.len() + 2
        || !original
            .blocks
            .iter()
            .map(|block| block.id)
            .eq(transformed.blocks[..original.blocks.len()]
                .iter()
                .map(|block| block.id))
        || source.exit == source.join
        || source.members.contains(&source.exit)
        || source.members.contains(&source.header)
    {
        return Err(error(
            "decision-tree original block partition or scalar exit differs",
        ));
    }
    let before = block(original, source.preheader)?;
    let preheader = block(transformed, source.preheader)?;
    if before.id != preheader.id
        || before.label != preheader.label
        || before.params != preheader.params
        || before.memory_params != preheader.memory_params
        || preheader.instructions.get(..before.instructions.len())
            != Some(before.instructions.as_slice())
        || !(3..=7).contains(
            &preheader
                .instructions
                .len()
                .saturating_sub(before.instructions.len()),
        )
    {
        return Err(error(
            "decision-tree preheader is not a closed append-only rewrite",
        ));
    }
    let KirTerminator::Jump {
        edge: original_entry,
    } = &before.terminator
    else {
        unreachable!()
    };
    let KirTerminator::Branch {
        condition,
        then_edge: entry,
        else_edge: fallback,
    } = &preheader.terminator
    else {
        return Err(error("decision-tree preheader is not a versioning branch"));
    };
    if fallback != original_entry
        || entry.args != original_entry.args
        || entry.memory_args != original_entry.memory_args
    {
        return Err(error(
            "decision-tree scalar fallback or vector entry state differs",
        ));
    }
    let old_header = block(original, source.header)?;
    let old_root = block(original, source.root)?;
    let header = block(transformed, entry.target)?;
    let KirTerminator::Branch {
        condition: loop_condition,
        then_edge: body_edge,
        else_edge: tail,
    } = &header.terminator
    else {
        return Err(error("decision-tree vector header is not a loop branch"));
    };
    let body = block(transformed, body_edge.target)?;
    if header.id.index() < state.ids().next_block
        || body.id.index() < state.ids().next_block
        || header.id == body.id
        || header.params.len() != old_header.params.len()
        || body.params.len() != old_root.params.len()
        || header.memory_params.len() != old_header.memory_params.len()
        || !body.memory_params.is_empty()
        || !body_edge.memory_args.is_empty()
    {
        return Err(error(
            "decision-tree vector block parameters or identities differ",
        ));
    }
    let mut scalar_map = BTreeMap::new();
    for (old, new) in old_header.params.iter().zip(&header.params) {
        if old.type_node != new.type_node {
            return Err(error("decision-tree vector header parameter type differs"));
        }
        scalar_map.insert(old.value, new.value);
    }
    let KirTerminator::Branch {
        then_edge: old_body_edge,
        ..
    } = &old_header.terminator
    else {
        unreachable!()
    };
    let expected_body_args = old_body_edge
        .args
        .iter()
        .map(|value| scalar_map.get(value).copied().unwrap_or(*value))
        .collect::<Vec<_>>();
    if expected_body_args != body_edge.args
        || tail.target != source.header
        || tail.args
            != header
                .params
                .iter()
                .map(|param| param.value)
                .collect::<Vec<_>>()
        || tail.memory_args
            != header
                .memory_params
                .iter()
                .map(|param| param.version)
                .collect::<Vec<_>>()
    {
        return Err(error(
            "decision-tree body or scalar tail edge state differs",
        ));
    }
    let mut scalar_aliases = BTreeMap::new();
    for ((old, new), argument) in old_root
        .params
        .iter()
        .zip(&body.params)
        .zip(&body_edge.args)
    {
        if old.type_node != new.type_node {
            return Err(error("decision-tree root parameter type differs"));
        }
        scalar_aliases.insert(new.value, *argument);
    }
    let mut memory_map = BTreeMap::new();
    for (old, new) in old_header.memory_params.iter().zip(&header.memory_params) {
        if old.region != new.region {
            return Err(error("decision-tree cloned MemorySSA partition differs"));
        }
        memory_map.insert(old.version, new.version);
    }
    let induction = *scalar_map
        .get(&source.induction)
        .ok_or_else(|| error("decision-tree vector induction is missing"))?;
    let source_root_iv = old_root
        .params
        .iter()
        .position(|param| param.value == source.root_induction)
        .ok_or_else(|| error("decision-tree source root induction is missing"))?;
    if scalar_aliases.get(&body.params[source_root_iv].value) != Some(&induction) {
        return Err(error(
            "decision-tree vector induction root forwarding differs",
        ));
    }
    let [comparison] = header.instructions.as_slice() else {
        return Err(error(
            "decision-tree vector header must have exactly one comparison",
        ));
    };
    let KirInstructionKind::Compare {
        op: crate::MirCompareOp::Le,
        left,
        right: limit,
    } = comparison.kind
    else {
        return Err(error("decision-tree vector bound comparison differs"));
    };
    if left != induction
        || single(comparison)? != *loop_condition
        || comparison.memory.is_some()
        || comparison.effect.is_some()
    {
        return Err(error(
            "decision-tree vector loop condition is not pure exact induction <= limit",
        ));
    }
    let limit_instruction = preheader
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == limit)
        })
        .ok_or_else(|| error("decision-tree vector limit is not preheader-defined"))?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Sub,
        left: entry_bound,
        right: stride,
        semantics: KirArithmeticSemantics::Modular,
    } = limit_instruction.kind
    else {
        return Err(error(
            "decision-tree vector limit is not bound minus chunk width",
        ));
    };
    let chunk_width = 2_u32.saturating_mul(u32::from(uf));
    if !int_constant(transformed, stride, &chunk_width.to_string())
        || !bound_matches(
            original,
            transformed,
            before,
            preheader,
            source.bound,
            entry_bound,
        )
    {
        return Err(error(
            "decision-tree emitted bound differs from the stable scalar bound",
        ));
    }
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return Err(error("decision-tree vector body has no backedge"));
    };
    if backedge.target != header.id
        || backedge.args.len() != header.params.len()
        || backedge.memory_args.len() != header.memory_params.len()
    {
        return Err(error("decision-tree vector backedge state arity differs"));
    }
    let induction_index = old_header
        .params
        .iter()
        .position(|param| param.value == source.induction)
        .expect("source induction");
    let next_induction = backedge.args[induction_index];
    let step = body
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == next_induction)
        })
        .ok_or_else(|| error("decision-tree vector induction update is missing"))?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = step.kind
    else {
        return Err(error(
            "decision-tree vector induction update is not modular addition",
        ));
    };
    if resolved(&scalar_aliases, left) != induction
        || !int_constant(transformed, right, &chunk_width.to_string())
        || step.memory.is_some()
        || step.effect.is_some()
    {
        return Err(error(
            "decision-tree vector induction does not advance by exact chunk width",
        ));
    }
    let mut control = BTreeSet::from([step.id]);
    if let Some(constant) = body.instructions.iter().find(|instruction| {
        instruction
            .results
            .iter()
            .any(|result| result.value == right)
    }) {
        control.insert(constant.id);
    }
    let old_regions = original.vector_regions.len();
    if transformed.vector_regions.get(..old_regions) != Some(original.vector_regions.as_slice()) {
        return Err(error("decision-tree changed a pre-existing vector region"));
    }
    let [region] = &transformed.vector_regions[old_regions..] else {
        return Err(error("decision-tree must add exactly one vector region"));
    };
    if region.blocks != [body.id] || region.id.index() < state.ids().next_vector_region {
        return Err(error("decision-tree vector ownership is not exact"));
    }
    Ok(Frame {
        preheader,
        header,
        body,
        entry_bound,
        condition: *condition,
        region: region.id,
        scalar_map,
        scalar_aliases,
        memory_map,
        body_induction: body.params[source_root_iv].value,
        next_induction,
        uf,
        control,
    })
}
fn int_constant(function: &KirFunction, value: ValueId, expected: &str) -> bool {
    definition(function, value).is_some_and(|instruction| instruction.memory.is_none() && instruction.effect.is_none()
        && instruction.results.len() == 1 && instruction.results[0].type_node.as_scalar() == Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        && matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == expected))
}
fn bound_matches(
    original: &KirFunction,
    transformed: &KirFunction,
    old_preheader: &KirBlock,
    preheader: &KirBlock,
    source: ValueId,
    emitted: ValueId,
) -> bool {
    if source == emitted
        && (original.params.iter().any(|param| param.value == source)
            || old_preheader
                .params
                .iter()
                .any(|param| param.value == source)
            || old_preheader.instructions.iter().any(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == source)
            }))
    {
        return true;
    }
    let Some(source) = definition(original, source) else {
        return false;
    };
    let Some(actual) = preheader.instructions.iter().find(|instruction| {
        instruction
            .results
            .iter()
            .any(|result| result.value == emitted)
    }) else {
        return false;
    };
    if actual.memory.is_some()
        || actual.effect.is_some()
        || actual.results.len() != 1
        || value_type(original, single(source).unwrap_or(emitted))
            != value_type(transformed, emitted)
    {
        return false;
    }
    match (&source.kind, &actual.kind) {
        (
            KirInstructionKind::ConstInt { value: left },
            KirInstructionKind::ConstInt { value: right },
        ) => left == right,
        (
            KirInstructionKind::SliceLen { slice },
            KirInstructionKind::SliceLen { slice: actual },
        ) => stable_invariant_descriptor_root(original, *slice) == Some(*actual),
        _ => false,
    }
}
fn check_guard(
    function: &KirFunction,
    frame: &Frame<'_>,
    source: &Source,
    plan: &DecisionTreeVectorPlan,
) -> CheckResult<()> {
    let vector = &plan.vector;
    let mut threshold = None;
    let mut ranges = BTreeSet::new();
    for predicate in &vector.predicates {
        match predicate {
            crate::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } if *trip_count == source.bound
                && *minimum >= 2 * u32::from(vector.uf)
                && minimum.is_multiple_of(2 * u32::from(vector.uf))
                && threshold.is_none() =>
            {
                threshold = Some(*minimum)
            }
            crate::VectorPredicate::WasmSliceRange { requirement, proof }
                if requirement.start.is_none()
                    && requirement.count == crate::WasmRangeCount::TripBound(source.bound)
                    && requirement.element_bytes == 8
                    && *proof == vector.proofs.target_legality =>
            {
                if !ranges.insert(requirement.slice) {
                    return Err(error("decision-tree plan duplicates a memory range"));
                }
            }
            _ => {
                return Err(error(
                    "decision-tree plan guard is not an exact threshold and two footprints",
                ));
            }
        }
    }
    if vector.predicates.len() != 3 || ranges != BTreeSet::from([source.input, source.output]) {
        return Err(error(
            "decision-tree plan memory extent closure is incomplete",
        ));
    }
    let minimum = threshold.ok_or_else(|| error("decision-tree trip threshold is missing"))?;
    let predicates = frame
        .preheader
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .collect::<Vec<_>>();
    let [guard] = predicates.as_slice() else {
        return Err(error("decision-tree needs one total entry predicate"));
    };
    if single(guard)? != frame.condition
        || guard.memory.is_some()
        || guard.effect.is_some()
        || guard.results[0].type_node.as_scalar()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::Bool))
    {
        return Err(error("decision-tree entry guard is not a pure bool"));
    }
    let KirInstructionKind::VersionPredicate { predicate } = &guard.kind else {
        unreachable!()
    };
    if predicate.address_bits != 32 || predicate.conjuncts.len() != 3 {
        return Err(error(
            "decision-tree memory extent guard has the wrong width or arity",
        ));
    }
    let mut actual = BTreeSet::new();
    let mut trip = false;
    for conjunct in &predicate.conjuncts {
        match conjunct {
            crate::KirVersionPredicateConjunct::TripThreshold {
                value,
                minimum: bound,
            } if !trip && *value == frame.entry_bound && *bound == minimum => trip = true,
            crate::KirVersionPredicateConjunct::WasmSliceRange {
                slice,
                start,
                count,
                element_bytes: 8,
            } if ranges.contains(slice)
                && int_constant(function, *start, "0")
                && *count == frame.entry_bound
                && actual.insert(*slice) => {}
            _ => {
                return Err(error(
                    "decision-tree emitted guard changed a trip or memory extent",
                ));
            }
        }
    }
    if !trip || actual != ranges {
        return Err(error("decision-tree emitted guard is incomplete"));
    }
    Ok(())
}

struct DagCheck<'a, 'b> {
    original: &'a KirFunction,
    source: &'a Source,
    frame: &'a Frame<'b>,
    vectors: BTreeMap<ValueId, ValueId>,
    used: BTreeSet<InstructionId>,
    splats: BTreeMap<(ValueId, u8), ValueId>,
    unroll_index: u8,
}
impl DagCheck<'_, '_> {
    fn operand(&mut self, scalar: ValueId, vector: ValueId) -> CheckResult<()> {
        let scalar = resolved(&self.source.aliases, scalar);
        if let Some(expected) = self.vectors.get(&scalar) {
            return if *expected == vector {
                Ok(())
            } else {
                Err(error(
                    "decision-tree strict vector operand differs from its scalar DAG",
                ))
            };
        }
        if value_type(self.original, scalar)
            != Some(KirValueType::Scalar(MirType::Primitive(
                MirPrimitiveTypeName::F64,
            )))
        {
            return Err(error("decision-tree splat source is not scalar f64"));
        }
        let splat = self
            .frame
            .body
            .instructions
            .iter()
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == vector)
            })
            .ok_or_else(|| error("decision-tree scalar operand has no vector splat"))?;
        let KirInstructionKind::VectorSplat {
            scalar: actual,
            region,
        } = splat.kind
        else {
            return Err(error("decision-tree scalar operand is not an exact splat"));
        };
        if region != self.frame.region
            || splat.memory.is_some()
            || splat.effect.is_some()
            || !vector_result(splat, false)
        {
            return Err(error("decision-tree splat type or effects differ"));
        }
        let actual = resolved(&self.frame.scalar_aliases, actual);
        let expected = self
            .frame
            .scalar_map
            .get(&scalar)
            .copied()
            .unwrap_or(scalar);
        if actual != expected {
            let old = definition(self.original, scalar)
                .ok_or_else(|| error("decision-tree invariant scalar operand changed"))?;
            let new = self
                .frame
                .body
                .instructions
                .iter()
                .find(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == actual)
                })
                .ok_or_else(|| error("decision-tree constant clone is missing"))?;
            if !matches!((&old.kind, &new.kind), (KirInstructionKind::ConstFloat { value: left }, KirInstructionKind::ConstFloat { value: right }) if left == right)
                || new.memory.is_some()
                || new.effect.is_some()
                || old.results.len() != 1
                || new.results.len() != 1
                || old.results[0].type_node != new.results[0].type_node
            {
                return Err(error(
                    "decision-tree scalar constant bits or invariant binding changed",
                ));
            }
            self.used.insert(new.id);
        }
        if self
            .splats
            .insert((scalar, self.unroll_index), vector)
            .is_some_and(|previous| previous != vector)
        {
            return Err(error(
                "decision-tree duplicates a source splat without accounting",
            ));
        }
        self.used.insert(splat.id);
        Ok(())
    }
    fn selection(
        &mut self,
        node: &Node,
        selected: ValueId,
        plan: &DecisionTreeVectorPlan,
    ) -> CheckResult<()> {
        match node {
            Node::Leaf { value, .. } => self.operand(*value, selected),
            Node::Branch {
                block,
                condition,
                then_node,
                else_node,
            } => {
                let mapping = plan
                    .selects
                    .iter()
                    .find(|mapping| {
                        mapping.source_branch == *block && mapping.unroll_index == self.unroll_index
                    })
                    .ok_or_else(|| error("decision-tree branch select mapping is missing"))?;
                let instruction = self
                    .frame
                    .body
                    .instructions
                    .iter()
                    .find(|instruction| instruction.id == mapping.vector_select)
                    .ok_or_else(|| error("decision-tree emitted select is missing"))?;
                let KirInstructionKind::VectorSelect {
                    mask,
                    when_true,
                    when_false,
                    region,
                } = instruction.kind
                else {
                    return Err(error(
                        "decision-tree branch is not lowered to an exact vector select",
                    ));
                };
                if region != self.frame.region
                    || single(instruction)? != selected
                    || !vector_result(instruction, false)
                    || instruction.memory.is_some()
                    || instruction.effect.is_some()
                    || self
                        .vectors
                        .get(&resolved(&self.source.aliases, *condition))
                        != Some(&mask)
                {
                    return Err(error(
                        "decision-tree select mask, result, or region differs",
                    ));
                }
                self.used.insert(instruction.id);
                self.selection(then_node, when_true, plan)?;
                self.selection(else_node, when_false, plan)
            }
        }
    }
}
fn vector_result(instruction: &KirInstruction, mask: bool) -> bool {
    instruction.results.len() == 1
        && instruction.results[0].type_node
            == if mask {
                KirValueType::Mask { lanes: 2 }
            } else {
                KirValueType::FixedVector {
                    lane: crate::KirLaneType::F64,
                    lanes: 2,
                }
            }
}
fn emitted_memory<'a>(frame: &Frame<'a>, id: InstructionId) -> CheckResult<&'a KirInstruction> {
    frame
        .body
        .instructions
        .iter()
        .find(|instruction| instruction.id == id)
        .ok_or_else(|| error("decision-tree mapped vector memory instruction is missing"))
}
#[expect(
    clippy::too_many_arguments,
    reason = "The independent chunk proof binds source, plan, frame, and emitted chunk identities explicitly."
)]
fn check_decision_tree_chunk_dag(
    original: &KirFunction,
    source: &Source,
    frame: &Frame<'_>,
    plan: &DecisionTreeVectorPlan,
    unroll_index: u8,
    load: &KirInstruction,
    store: &KirInstruction,
    stored: ValueId,
) -> CheckResult<(BTreeSet<InstructionId>, BTreeSet<ValueId>)> {
    let mut dag = DagCheck {
        original,
        source,
        frame,
        unroll_index,
        vectors: BTreeMap::from([(
            single(instruction(original, source.root_load)?)?,
            single(load)?,
        )]),
        used: BTreeSet::from([load.id, store.id]),
        splats: BTreeMap::new(),
    };
    let mappings = plan
        .vector
        .operations
        .iter()
        .filter(|mapping| mapping.unroll_index == unroll_index)
        .collect::<Vec<_>>();
    for mapping in &mappings {
        let source_instruction = instruction(original, mapping.scalar)?;
        let emitted = frame
            .body
            .instructions
            .iter()
            .find(|instruction| instruction.id == mapping.vector)
            .ok_or_else(|| error("decision-tree mapped arithmetic is missing"))?;
        if !vector_result(
            emitted,
            mapping.operation == crate::KirProfileOperation::Compare,
        ) || emitted.memory.is_some()
            || emitted.effect.is_some()
        {
            return Err(error(
                "decision-tree vector arithmetic has a false type or effects",
            ));
        }
        dag.vectors
            .insert(single(source_instruction)?, single(emitted)?);
    }
    for mapping in mappings {
        let scalar = instruction(original, mapping.scalar)?;
        let emitted = frame
            .body
            .instructions
            .iter()
            .find(|instruction| instruction.id == mapping.vector)
            .expect("mapped instruction checked");
        match (&scalar.kind, &emitted.kind) {
            (
                KirInstructionKind::Binary {
                    op,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::StrictFloat,
                },
                KirInstructionKind::VectorBinary {
                    op: actual_op,
                    left: actual_left,
                    right: actual_right,
                    semantics: KirArithmeticSemantics::StrictFloat,
                    no_failure_proof: None,
                    region,
                },
            ) => {
                let expected = match op {
                    MirBinaryOp::Add => crate::KirVectorBinaryOp::Add,
                    MirBinaryOp::Sub => crate::KirVectorBinaryOp::Subtract,
                    MirBinaryOp::Mul => crate::KirVectorBinaryOp::Multiply,
                    _ => {
                        return Err(error(
                            "decision-tree source FP operation is outside the scalar select subset",
                        ));
                    }
                };
                if *actual_op != expected || *region != frame.region {
                    return Err(error("decision-tree strict FP operation was changed"));
                }
                dag.operand(*left, *actual_left)?;
                dag.operand(*right, *actual_right)?;
            }
            (
                KirInstructionKind::Unary {
                    op: crate::MirUnaryOp::Neg,
                    operand,
                    semantics: KirArithmeticSemantics::StrictFloat,
                },
                KirInstructionKind::VectorUnary {
                    op: crate::KirVectorUnaryOp::Negate,
                    operand: actual,
                    semantics: KirArithmeticSemantics::StrictFloat,
                    no_failure_proof: None,
                    region,
                },
            ) if *region == frame.region => dag.operand(*operand, *actual)?,
            (
                KirInstructionKind::Compare { op, left, right },
                KirInstructionKind::VectorCompare {
                    op: actual_op,
                    left: actual_left,
                    right: actual_right,
                    region,
                },
            ) if op == actual_op && *region == frame.region => {
                dag.operand(*left, *actual_left)?;
                dag.operand(*right, *actual_right)?;
            }
            _ => {
                return Err(error(
                    "decision-tree arithmetic/comparison changed strict rounding, NaN, or signed-zero semantics",
                ));
            }
        }
        dag.used.insert(emitted.id);
    }
    dag.selection(&source.tree, stored, plan)?;
    let splats = dag.splats.values().copied().collect();
    Ok((dag.used, splats))
}

fn check_emitted_tree(
    original: &KirFunction,
    transformed: &KirFunction,
    source: &Source,
    frame: &Frame<'_>,
    plan: &DecisionTreeVectorPlan,
) -> CheckResult<BTreeSet<ValueId>> {
    let mut used = frame.control.clone();
    let mut splats = BTreeSet::new();
    let old_load_memory = instruction(original, source.root_load)?
        .memory
        .as_ref()
        .expect("source memory checked");
    let old_load_input = resolved_memory(&source.memories, old_load_memory.input);
    let input_memory = frame
        .memory_map
        .get(&old_load_input)
        .copied()
        .ok_or_else(|| error("decision-tree input MemorySSA does not map to vector header"))?;
    let output_header_memory = block(original, source.header)?
        .memory_params
        .iter()
        .find(|param| param.region == source.output_partition)
        .ok_or_else(|| error("decision-tree source output memory parameter is missing"))?
        .version;
    let mut output_memory = frame
        .memory_map
        .get(&output_header_memory)
        .copied()
        .ok_or_else(|| error("decision-tree output MemorySSA does not map to vector header"))?;
    let mut previous_store_effect = None;
    let mut expected_start = frame.body_induction;

    for unroll_index in 0..frame.uf {
        let load_group = plan
            .vector
            .memory_groups
            .iter()
            .find(|group| {
                group.access == crate::VectorMemoryAccessKind::Read
                    && group.unroll_index == unroll_index
            })
            .expect("load group coverage checked");
        let store_group = plan
            .vector
            .memory_groups
            .iter()
            .find(|group| {
                group.access == crate::VectorMemoryAccessKind::Write
                    && group.unroll_index == unroll_index
            })
            .expect("store group coverage checked");
        let load = emitted_memory(frame, load_group.vector_instruction)?;
        let store = emitted_memory(frame, store_group.vector_instruction)?;
        let KirInstructionKind::VectorLoad {
            access: load_access,
            region: load_region,
        } = &load.kind
        else {
            return Err(error("decision-tree root read is not a vector load"));
        };
        let KirInstructionKind::VectorStore {
            access: store_access,
            value: stored,
            region: store_region,
        } = &store.kind
        else {
            return Err(error("decision-tree terminal write is not a vector store"));
        };
        if *load_region != frame.region
            || *store_region != frame.region
            || !vector_result(load, false)
            || !store.results.is_empty()
        {
            return Err(error("decision-tree vector memory type or owner differs"));
        }
        if unroll_index == 0 {
            if load_access.start != expected_start {
                return Err(error("first vector chunk does not start at source IV"));
            }
        } else {
            let address = definition(transformed, load_access.start)
                .ok_or_else(|| error("decision-tree chunk offset has no definition"))?;
            let KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = address.kind
            else {
                return Err(error("decision-tree chunk offset is not modular addition"));
            };
            if left != expected_start
                || !int_constant(transformed, right, "2")
                || address.memory.is_some()
                || address.effect.is_some()
                || address.results.len() != 1
                || single(address)? != load_access.start
            {
                return Err(error(
                    "decision-tree chunk offset does not advance by exactly two",
                ));
            }
            used.insert(address.id);
            let constant = definition(transformed, right)
                .ok_or_else(|| error("decision-tree chunk offset constant is missing"))?;
            used.insert(constant.id);
        }
        if store_access.start != load_access.start {
            return Err(error("decision-tree load/store chunk starts differ"));
        }
        for (access, slice) in [(load_access, source.input), (store_access, source.output)] {
            if access.slice != slice
                || access.end != frame.entry_bound
                || access.lane != crate::KirLaneType::F64
                || access.lanes != 2
                || access.byte_footprint != 16
                || access.known_alignment != 8
                || access.required_alignment != 8
            {
                return Err(error(
                    "decision-tree vector address or complete footprint differs",
                ));
            }
        }
        let load_memory = load
            .memory
            .as_ref()
            .ok_or_else(|| error("decision-tree vector load MemorySSA is missing"))?;
        let store_memory = store
            .memory
            .as_ref()
            .ok_or_else(|| error("decision-tree vector store MemorySSA is missing"))?;
        if load_memory.region != source.input_partition
            || load_memory.input != input_memory
            || load_memory.output.is_some()
            || store_memory.region != source.output_partition
            || store_memory.input != output_memory
            || store_memory.output.is_none()
        {
            return Err(error(
                "decision-tree emitted MemorySSA changed a source partition or version",
            ));
        }
        let load_effect = load
            .effect
            .as_ref()
            .ok_or_else(|| error("decision-tree root read effect is missing"))?;
        let store_effect = store
            .effect
            .as_ref()
            .ok_or_else(|| error("decision-tree selected write effect is missing"))?;
        if load_effect.kind != crate::KirEffectKind::ReadMemory
            || store_effect.kind != crate::KirEffectKind::WriteMemory
            || load_effect.order >= store_effect.order
            || previous_store_effect.is_some_and(|previous| previous >= load_effect.order)
        {
            return Err(error("decision-tree read/write effect order differs"));
        }
        let load_position = frame
            .body
            .instructions
            .iter()
            .position(|instruction| instruction.id == load.id)
            .expect("load in body");
        let store_position = frame
            .body
            .instructions
            .iter()
            .position(|instruction| instruction.id == store.id)
            .expect("store in body");
        if load_position >= store_position {
            return Err(error("decision-tree store executes before its root read"));
        }
        previous_store_effect = Some(store_effect.order);
        output_memory = store_memory.output.expect("store output checked");
        expected_start = load_access.start;
        let (chunk_used, chunk_splats) = check_decision_tree_chunk_dag(
            original,
            source,
            frame,
            plan,
            unroll_index,
            load,
            store,
            *stored,
        )?;
        used.extend(chunk_used);
        splats.extend(chunk_splats);
    }

    if frame
        .body
        .instructions
        .iter()
        .any(|instruction| !used.contains(&instruction.id))
    {
        return Err(error(
            "decision-tree vector body contains unaccounted computation, effects, or stores",
        ));
    }
    let KirTerminator::Jump { edge: backedge } = &frame.body.terminator else {
        unreachable!()
    };
    let expected_values = source
        .backedge_values
        .iter()
        .map(|state| match state {
            ScalarState::Value(value) => frame.scalar_map.get(value).copied().unwrap_or(*value),
            ScalarState::NextInduction => frame.next_induction,
        })
        .collect::<Vec<_>>();
    if expected_values
        != backedge
            .args
            .iter()
            .map(|value| resolved(&frame.scalar_aliases, *value))
            .collect::<Vec<_>>()
    {
        return Err(error(
            "decision-tree vector backedge changed scalar state or lane coverage",
        ));
    }
    let expected_memories = source
        .backedge_memories
        .iter()
        .map(|state| match state {
            MemoryState::Version(version) => {
                frame.memory_map.get(version).copied().unwrap_or(*version)
            }
            MemoryState::Stored => output_memory,
        })
        .collect::<Vec<_>>();
    if expected_memories != backedge.memory_args {
        return Err(error(
            "decision-tree vector backedge does not retain the complete unrolled write chain",
        ));
    }
    let before = block(original, source.preheader)?;
    let limit = match frame.header.instructions[0].kind {
        KirInstructionKind::Compare { right, .. } => right,
        _ => unreachable!(),
    };
    for emitted in &frame.preheader.instructions[before.instructions.len()..] {
        let allowed = match &emitted.kind {
            KirInstructionKind::ConstInt { value } => value.parse::<u32>().is_ok(),
            KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                ..
            } => single(emitted)? == limit,
            KirInstructionKind::VersionPredicate { .. } => single(emitted)? == frame.condition,
            KirInstructionKind::SliceLen { .. } => single(emitted)? == frame.entry_bound,
            _ => false,
        };
        if !allowed || emitted.memory.is_some() || emitted.effect.is_some() {
            return Err(error(
                "decision-tree entry adds unaccounted effects or computation",
            ));
        }
    }
    for id in &frame.control {
        let emitted = instruction(transformed, *id)?;
        if emitted.memory.is_some() || emitted.effect.is_some() {
            return Err(error("decision-tree vector control has effects"));
        }
    }
    Ok(splats)
}

fn priced(
    profile: &crate::KirTargetProfile,
    operation: crate::KirProfileOperation,
    lane: crate::KirLaneType,
    lanes: u8,
    semantics: crate::KirCostSemantics,
    alignment: crate::KirAlignmentClass,
) -> CheckResult<u32> {
    match profile.operation_availability(&crate::KirCostKey {
        operation,
        lane,
        lanes,
        semantics,
        alignment,
    }) {
        Some(crate::KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Ok(cost.cost)
        }
        Some(crate::KirOperationAvailability::Unavailable)
            if operation == crate::KirProfileOperation::Branch && lanes == 1 =>
        {
            Ok(1)
        }
        _ => Err(TransactionCheckError::reject(
            "decision-tree-target-operation-unavailable",
        )),
    }
}
#[allow(clippy::too_many_arguments)]
fn check_cost_and_charge(
    state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    original: &KirFunction,
    transformed: &KirFunction,
    source: &Source,
    plan: &DecisionTreeVectorPlan,
    splats: &BTreeSet<ValueId>,
    charge: &CandidateBudgetCharge,
) -> CheckResult<()> {
    use crate::{
        KirAlignmentClass as Align, KirCostSemantics as Sem, KirLaneType as Lane,
        KirProfileOperation as Op,
    };
    let profile = &state.module().profile;
    let uf = u32::from(plan.vector.uf);
    let chunk_width = uf.saturating_mul(2);
    let select_count = u32::try_from(plan.selects.len())
        .map_err(|_| error("decision-tree select count overflows"))?;
    let decision_count = select_count / uf;
    let mut scalar_iteration = 0_u32;
    let mut vector_per_chunk = 0_u32;
    // The independent source checks above prove the committed O3 scalar
    // closed-store lowering executes every pure arm and one select per branch.
    // This prices actual lowering, not a branch-frequency prior or scalar sum
    // that would be valid for a branch-preserving implementation.
    for source_id in &source.operations {
        let (operation, semantics) = source_operation(instruction(original, *source_id)?)?;
        scalar_iteration = scalar_iteration.saturating_add(priced(
            profile,
            operation,
            Lane::F64,
            1,
            semantics,
            Align::NotApplicable,
        )?);
        vector_per_chunk = vector_per_chunk.saturating_add(priced(
            profile,
            operation,
            Lane::F64,
            2,
            semantics,
            Align::NotApplicable,
        )?);
    }
    scalar_iteration = scalar_iteration.saturating_add(
        priced(
            profile,
            Op::Select,
            Lane::F64,
            1,
            Sem::NotApplicable,
            Align::NotApplicable,
        )?
        .saturating_mul(decision_count),
    );
    vector_per_chunk = vector_per_chunk.saturating_add(
        priced(
            profile,
            Op::Select,
            Lane::F64,
            2,
            Sem::NotApplicable,
            Align::NotApplicable,
        )?
        .saturating_mul(decision_count),
    );
    let mut vector_memory = 0_u32;
    for operation in [Op::Load, Op::Store] {
        scalar_iteration = scalar_iteration.saturating_add(priced(
            profile,
            operation,
            Lane::F64,
            1,
            Sem::NotApplicable,
            Align::Bytes(8),
        )?);
        vector_memory = vector_memory.saturating_add(priced(
            profile,
            operation,
            Lane::F64,
            2,
            Sem::NotApplicable,
            Align::Bytes(8),
        )?);
    }
    let splat_cost = priced(
        profile,
        Op::Splat,
        Lane::F64,
        2,
        Sem::NotApplicable,
        Align::NotApplicable,
    )?
    .saturating_mul(u32::try_from(splats.len()).unwrap_or(u32::MAX));
    let add = priced(
        profile,
        Op::Add,
        Lane::U32,
        1,
        Sem::Modular,
        Align::NotApplicable,
    )?;
    let compare = priced(
        profile,
        Op::Compare,
        Lane::U32,
        1,
        Sem::NotApplicable,
        Align::NotApplicable,
    )?;
    let branch = priced(
        profile,
        Op::Branch,
        Lane::U32,
        1,
        Sem::NotApplicable,
        Align::NotApplicable,
    )?;
    let control = add.saturating_add(compare).saturating_add(branch);
    scalar_iteration = scalar_iteration.saturating_add(control);
    let vector_chunk = vector_per_chunk
        .saturating_mul(uf)
        .saturating_add(vector_memory.saturating_mul(uf))
        .saturating_add(splat_cost)
        .saturating_add(add.saturating_mul(uf.saturating_sub(1)))
        .saturating_add(control);
    let predicates = compare.saturating_add(branch).saturating_add(
        priced(
            profile,
            Op::RuntimePredicate,
            Lane::U32,
            2,
            Sem::NotApplicable,
            Align::NotApplicable,
        )?
        .saturating_mul(2),
    );
    if u64::from(vector_chunk).saturating_mul(100)
        >= u64::from(scalar_iteration)
            .saturating_mul(u64::from(chunk_width))
            .saturating_mul(80)
    {
        return Err(TransactionCheckError::reject(
            "profitability-threshold-not-met",
        ));
    }
    let minimum = (2_u32..=1024)
        .map(|groups| groups * chunk_width)
        .find(|trip| {
            (0..chunk_width).all(|tail| {
                let scalar = scalar_iteration.saturating_mul(trip.saturating_add(tail));
                let vector = vector_chunk
                    .saturating_mul(*trip / chunk_width)
                    .saturating_add(scalar_iteration.saturating_mul(tail))
                    .saturating_add(predicates)
                    .saturating_add(branch.saturating_mul(u32::from(tail != 0)));
                u64::from(vector).saturating_mul(100) <= u64::from(scalar).saturating_mul(80)
            })
        })
        .ok_or_else(|| TransactionCheckError::reject("profitability-threshold-not-met"))?;
    let expected = crate::KirCostEstimate::new(
        scalar_iteration.saturating_mul(minimum.saturating_add(chunk_width - 1)),
        vector_chunk.saturating_mul(minimum / chunk_width),
        predicates,
        scalar_iteration
            .saturating_mul(chunk_width - 1)
            .saturating_add(branch),
    );
    if plan.vector.cost != expected
        || plan
            .vector
            .predicates
            .iter()
            .filter_map(|predicate| match predicate {
                crate::VectorPredicate::TripThreshold { minimum, .. } => Some(*minimum),
                _ => None,
            })
            .collect::<Vec<_>>()
            != [minimum]
    {
        return Err(error(
            "decision-tree cost or minimum trip was not independently reproduced",
        ));
    }
    let dominators = crate::compute_kir_dominators(original);
    let headers = original
        .blocks
        .iter()
        .flat_map(|block| {
            edges(&block.terminator)
                .into_iter()
                .filter(|edge| dominators.dominates(edge.target, block.id))
                .map(|edge| edge.target)
        })
        .collect::<BTreeSet<_>>();
    if headers
        .iter()
        .position(|header| *header == source.header)
        .and_then(|index| u32::try_from(index).ok())
        .map(crate::LoopId::from_index)
        != Some(plan.vector.loop_id)
    {
        return Err(error("decision-tree source loop identity differs"));
    }
    let module_units = |module: &crate::KirModule| {
        module
            .functions
            .iter()
            .map(crate::kir_function_units)
            .fold(0_u32, u32::saturating_add)
    };
    let growth = crate::VectorPlanGrowth::new(
        crate::kir_function_units(original),
        crate::kir_function_units(transformed),
        module_units(state.module()),
        module_units(trial.module()),
    );
    if growth != plan.vector.growth {
        return Err(error("decision-tree growth accounting is false"));
    }
    let roots = &plan.vector.proofs;
    let proofs = [
        roots.canonical_loop,
        roots.trip_partition,
        roots.lane_mapping,
        roots.operation_equivalence,
        roots.fallback_identity,
        roots.target_legality,
        roots.cost_and_budget,
    ];
    if proofs.iter().copied().collect::<BTreeSet<_>>().len() != 7
        || trial.proofs().proofs().len() != state.proofs().proofs().len() + 7
        || trial.proofs().proofs()[state.proofs().proofs().len()..]
            .iter()
            .map(|certificate| certificate.id)
            .collect::<BTreeSet<_>>()
            != proofs.iter().copied().collect()
        || proofs.iter().any(|proof| {
            trial.proofs().get(*proof).is_none_or(|certificate| {
                certificate.use_site.function != original.id
                    || certificate.generation != trial.evidence_generation()
            })
        })
    {
        return Err(error(
            "decision-tree proof roots are missing, stale, or reused",
        ));
    }
    if plan
        .vector
        .memory_groups
        .iter()
        .any(|group| group.footprint_proof != roots.operation_equivalence)
    {
        return Err(error("decision-tree footprint proof identity differs"));
    }
    let operations = u32::try_from(plan.vector.operations.len()).unwrap_or(u32::MAX);
    let lanes = plan
        .vector
        .operations
        .iter()
        .map(|operation| u32::try_from(operation.lanes.len()).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add);
    let memory = plan
        .vector
        .memory_groups
        .iter()
        .map(|group| u32::try_from(group.scalar_instructions.len()).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add);
    let groups = u32::try_from(plan.vector.memory_groups.len()).unwrap_or(u32::MAX);
    let predicates = u32::try_from(plan.vector.predicates.len()).unwrap_or(u32::MAX);
    let proposal = 8_u32
        .saturating_add(operations.saturating_mul(4))
        .saturating_add(lanes)
        .saturating_add(groups.saturating_mul(4))
        .saturating_add(memory)
        .saturating_add(predicates.saturating_mul(3))
        .saturating_add(2)
        .saturating_add(select_count.saturating_mul(6));
    let checker = 16_u32
        .saturating_add(operations.saturating_mul(6))
        .saturating_add(lanes.saturating_mul(2))
        .saturating_add(groups.saturating_mul(6))
        .saturating_add(memory.saturating_mul(2))
        .saturating_add(predicates.saturating_mul(4))
        .saturating_add(7)
        .saturating_add(3)
        .saturating_add(select_count.saturating_mul(10));
    if *charge != CandidateBudgetCharge::single(original.id, proposal, checker) {
        return Err(error(
            "decision-tree charge does not cover every operation, leaf store, and select",
        ));
    }
    Ok(())
}
