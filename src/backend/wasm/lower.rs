use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::*;

use super::{
    ir::{
        WasmLoweredBlock, WasmLoweredEdge, WasmLoweredFunction, WasmLoweredInstruction,
        WasmLoweredModule, WasmPhysicalType, WasmSourceType, WasmTypedValue,
    },
    kir::{
        adapt_instruction, block_label, edge_copy_instructions, local_name, validate_vector_kir,
        value_kir_types, value_types,
    },
    layout::WasmStructLayout,
    memory::checked_wasm_memory_plan_with_evidence,
};

const PIECEWISE_MAX_EXTRA_OPS: usize = 16;
const PIECEWISE_EXTRA_TO_MIN_PATH_RATIO: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WasmLoweringError {
    InvalidInput(String),
    UnsupportedValueType(MirType),
    InvariantFailure(String),
}

impl std::fmt::Display for WasmLoweringError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::InvariantFailure(message) => {
                formatter.write_str(message)
            }
            Self::UnsupportedValueType(type_node) => write!(
                formatter,
                "WebAssembly KIR backend cannot lower value type {type_node:?}"
            ),
        }
    }
}

impl std::error::Error for WasmLoweringError {}

pub(super) fn lower_wasm_module<'a>(
    module: &'a KirModule,
    contracts: Option<&ContractFactSet>,
    mir: &MirModule,
) -> Result<WasmLoweredModule<'a>, WasmLoweringError> {
    if module.config.overflow_mode != KirOverflowMode::Unchecked
        || module.config.bounds_mode != KirBoundsMode::Unchecked
    {
        return Err(WasmLoweringError::InvalidInput(
            "WebAssembly KIR backend accepts only unchecked KIR".to_string(),
        ));
    }
    let features = module.profile.wasm_features().ok_or_else(|| {
        WasmLoweringError::InvalidInput(
            "WebAssembly KIR backend requires a WebAssembly target profile".to_string(),
        )
    })?;
    validate_vector_kir(module, features).map_err(WasmLoweringError::InvalidInput)?;
    let layout = WasmStructLayout::new(mir);
    let artifact_functions = mir
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<BTreeSet<_>>();
    Ok(WasmLoweredModule {
        functions: module
            .functions
            .iter()
            .filter(|function| artifact_functions.contains(function.name.as_str()))
            .map(|function| lower_wasm_function(function, contracts, &layout))
            .collect::<Result<Vec<_>, WasmLoweringError>>()?,
    })
}

/// Rejects scalar KIR values that the legacy MIR emitter cannot represent before
/// optimization levels that intentionally skip typed lowering reach that emitter.
pub(super) fn validate_wasm_scalar_value_types(
    module: &KirModule,
    mir: &MirModule,
) -> Result<(), WasmLoweringError> {
    let artifact_functions = mir
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<BTreeSet<_>>();

    for function in module
        .functions
        .iter()
        .filter(|function| artifact_functions.contains(function.name.as_str()))
    {
        if !matches!(function.return_type, MirType::Void) {
            wasm_scalar_physical_type(&function.return_type)?;
        }
        for param in &function.params {
            wasm_scalar_physical_type(&param.type_node)?;
        }
        for value_type in function.blocks.iter().flat_map(|block| {
            block.params.iter().map(|param| &param.type_node).chain(
                block.instructions.iter().flat_map(|instruction| {
                    instruction.results.iter().map(|result| &result.type_node)
                }),
            )
        }) {
            if let KirValueType::Scalar(type_node) = value_type {
                wasm_scalar_physical_type(type_node)?;
            }
        }
    }

    Ok(())
}

fn lower_wasm_function<'a>(
    function: &'a KirFunction,
    contracts: Option<&ContractFactSet>,
    layout: &WasmStructLayout,
) -> Result<WasmLoweredFunction<'a>, WasmLoweringError> {
    let types = value_types(function);
    let kir_types = value_kir_types(function);
    let params = function
        .params
        .iter()
        .map(|param| (param.value, (param.name.clone(), param.type_node.clone())))
        .collect::<BTreeMap<_, _>>();
    let source_types = function
        .params
        .iter()
        .map(|param| (param.value, WasmSourceType::Mir(&param.type_node)))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, WasmSourceType::Kir(&param.type_node)))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, WasmSourceType::Kir(&result.type_node)))
                }))
        }))
        .collect::<BTreeMap<_, _>>();
    let values = source_types
        .into_iter()
        .map(|(id, source_type)| {
            let kir_type = kir_types.get(&id).ok_or_else(|| {
                WasmLoweringError::InvariantFailure(format!(
                    "WebAssembly KIR has no type for value {}",
                    id.index()
                ))
            })?;
            let operand = match source_type {
                WasmSourceType::Mir(_) | WasmSourceType::Kir(KirValueType::Scalar(_)) => {
                    Some(super::kir::mir_value(id, &types, &params))
                }
                WasmSourceType::Kir(KirValueType::FixedVector { .. }) => None,
                WasmSourceType::Kir(KirValueType::Mask { .. }) => None,
            };
            let physical = wasm_physical_type(source_type, kir_type)?;
            Ok((
                id,
                WasmTypedValue {
                    value: id,
                    source_type,
                    operand,
                    physical,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, WasmLoweringError>>()?;
    if !matches!(function.return_type, MirType::Void) {
        wasm_scalar_physical_type(&function.return_type)?;
    }

    let blocks_by_id = function
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<HashMap<_, _>>();
    let local_values = function
        .blocks
        .iter()
        .flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        })
        .collect::<BTreeSet<_>>();
    let locals = local_values
        .iter()
        .filter_map(|value| {
            types.get(value).map(|type_node| MirLocal {
                name: local_name(*value),
                type_node: type_node.clone(),
            })
        })
        .collect();

    let mut lowered_blocks = Vec::with_capacity(function.blocks.len());
    let mut view_blocks = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        let mut lowered_instructions = Vec::with_capacity(block.instructions.len());
        let mut view_instructions = Vec::new();
        for instruction in &block.instructions {
            let leaves = adapt_instruction(instruction, &types, &params)
                .map_err(WasmLoweringError::InvalidInput)?;
            view_instructions.extend(leaves.iter().cloned());
            lowered_instructions.push(WasmLoweredInstruction {
                source: instruction,
                leaves,
            });
        }
        let mut edges = Vec::new();
        match &block.terminator {
            KirTerminator::Return { .. } => {}
            KirTerminator::Jump { edge } => {
                edges.push(lower_edge(block, 0, edge, &blocks_by_id, &types, &params)?);
            }
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                edges.push(lower_edge(
                    block,
                    0,
                    then_edge,
                    &blocks_by_id,
                    &types,
                    &params,
                )?);
                edges.push(lower_edge(
                    block,
                    1,
                    else_edge,
                    &blocks_by_id,
                    &types,
                    &params,
                )?);
            }
        }
        for edge in &edges {
            view_instructions.extend(edge.copies.iter().cloned());
        }
        let view_terminator = match &block.terminator {
            KirTerminator::Return { value, .. } => MirTerminator::Return {
                value: value.map(|value| super::kir::mir_value(value, &types, &params)),
            },
            KirTerminator::Jump { edge } => MirTerminator::Jump {
                label: block_label(edge.target),
            },
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => MirTerminator::Branch {
                condition: super::kir::mir_value(*condition, &types, &params),
                then_label: block_label(then_edge.target),
                else_label: block_label(else_edge.target),
            },
        };
        view_blocks.push(MirBlock {
            label: block_label(block.id),
            instructions: view_instructions,
            terminator: view_terminator,
        });
        lowered_blocks.push(WasmLoweredBlock {
            source: block,
            instructions: lowered_instructions,
            edges,
        });
    }

    Ok(WasmLoweredFunction {
        source: function,
        vector_values: values
            .iter()
            .filter_map(|(value, typed)| {
                (typed.physical == WasmPhysicalType::V128).then_some(*value)
            })
            .collect(),
        memory_plan: checked_wasm_memory_plan_with_evidence(function, contracts, Some(layout)),
        values,
        blocks: lowered_blocks,
        local_view: MirFunction {
            name: function.name.clone(),
            exported: function.exported,
            params: function
                .params
                .iter()
                .map(|param| MirParam {
                    name: param.name.clone(),
                    type_node: param.type_node.clone(),
                })
                .collect(),
            return_type: function.return_type.clone(),
            locals,
            blocks: view_blocks,
        },
    })
}

fn lower_edge<'a>(
    source: &'a KirBlock,
    arm: u8,
    edge: &'a KirEdge,
    blocks: &HashMap<BlockId, &'a KirBlock>,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
) -> Result<WasmLoweredEdge<'a>, WasmLoweringError> {
    let target = blocks.get(&edge.target).ok_or_else(|| {
        WasmLoweringError::InvariantFailure(format!(
            "WebAssembly KIR edge targets unknown block {}",
            edge.target.index()
        ))
    })?;
    let label = super::kir::edge_label(source.id, edge.target, u32::from(arm));
    Ok(WasmLoweredEdge {
        arm,
        source: edge,
        copies: edge_copy_instructions(&label, target, edge, types, params),
    })
}

fn wasm_physical_type(
    source_type: WasmSourceType<'_>,
    kir_type: &KirValueType,
) -> Result<WasmPhysicalType, WasmLoweringError> {
    match (source_type, kir_type) {
        (WasmSourceType::Mir(type_node), KirValueType::Scalar(source))
        | (WasmSourceType::Kir(KirValueType::Scalar(type_node)), KirValueType::Scalar(source))
            if type_node == source =>
        {
            wasm_scalar_physical_type(type_node)
        }
        (
            WasmSourceType::Kir(KirValueType::FixedVector { .. }),
            KirValueType::FixedVector { .. },
        ) => Ok(WasmPhysicalType::V128),
        (WasmSourceType::Kir(KirValueType::Mask { .. }), KirValueType::Mask { .. }) => {
            Ok(WasmPhysicalType::V128)
        }
        _ => Err(WasmLoweringError::InvariantFailure(
            "WebAssembly KIR typed value source metadata is inconsistent".to_string(),
        )),
    }
}

/// Returns the v128 assignments on one selected edge. Scalar assignments live in
/// the MIR leaf view; vector and mask values retain their exact KIR types here.
pub(super) fn vector_edge_copies(
    function: &WasmLoweredFunction<'_>,
    edge: &WasmLoweredEdge<'_>,
) -> Result<Vec<(ValueId, ValueId)>, WasmLoweringError> {
    let invalid = |message: &str| WasmLoweringError::InvariantFailure(message.to_string());
    let target = function
        .source
        .blocks
        .iter()
        .find(|block| block.id == edge.source.target)
        .ok_or_else(|| invalid("WebAssembly vector edge targets an unknown block"))?;
    if target.params.len() != edge.source.args.len() {
        return Err(invalid(
            "WebAssembly vector edge argument arity is inconsistent",
        ));
    }
    let mut copies = Vec::new();
    for (param, argument) in target.params.iter().zip(&edge.source.args) {
        if param.type_node.as_scalar().is_some() {
            continue;
        }
        for value in [param.value, *argument] {
            let typed = function
                .values
                .get(&value)
                .ok_or_else(|| invalid("WebAssembly vector edge value has no typed metadata"))?;
            if typed.physical != WasmPhysicalType::V128
                || !matches!(typed.source_type, WasmSourceType::Kir(type_node) if type_node == &param.type_node)
            {
                return Err(invalid(
                    "WebAssembly vector edge value type is inconsistent",
                ));
            }
        }
        copies.push((param.value, *argument));
    }
    Ok(copies)
}

pub(super) struct WasmConditionalIncrement<'lowered, 'source> {
    pub direct_edge: &'lowered WasmLoweredEdge<'source>,
    pub condition: ValueId,
    pub base: ValueId,
    pub result: ValueId,
    pub copy_index: usize,
    pub increment_on_true: bool,
}

/// Recognizes a closed, effect-free triangle without changing its typed CFG.
/// The direct join edge supplies all parallel assignments except one modular
/// increment. Values defined in the bypassed arm cannot escape through other uses.
pub(super) fn checked_conditional_increment<'lowered, 'source>(
    function: &'lowered WasmLoweredFunction<'source>,
    source: &'lowered WasmLoweredBlock<'source>,
) -> Option<WasmConditionalIncrement<'lowered, 'source>> {
    let KirTerminator::Branch { condition, .. } = source.source.terminator else {
        return None;
    };
    for update_arm in 0..=1 {
        let update_edge = source.edges.iter().find(|edge| edge.arm == update_arm)?;
        let direct_edge = source.edges.iter().find(|edge| edge.arm != update_arm)?;
        let Some(arm) = function
            .blocks
            .iter()
            .find(|block| block.source.id == update_edge.source.target)
        else {
            continue;
        };
        let Some(candidate) =
            check_increment_arm(function, source, arm, update_edge, direct_edge, condition)
        else {
            continue;
        };
        return Some(candidate);
    }
    None
}

/// A closed, single-entry decision tree whose terminal blocks each perform one
/// same-address store and flow to a common join. The emitter uses this only as
/// a verified lowering pattern; the source KIR remains unchanged.
#[derive(Debug, Clone)]
pub(super) struct WasmPiecewiseClosedStoreTree {
    pub shape: WasmPiecewiseTreeShape,
    pub members: BTreeSet<BlockId>,
    pub join: BlockId,
    pub representative_leaf: BlockId,
    pub shared_increment: Option<InstructionId>,
    pub stores: BTreeSet<InstructionId>,
    pub increments: BTreeSet<InstructionId>,
}

#[derive(Debug, Clone)]
pub(super) enum WasmPiecewiseTreeShape {
    Branch {
        block: BlockId,
        condition: ValueId,
        then_node: Box<WasmPiecewiseTreeShape>,
        else_node: Box<WasmPiecewiseTreeShape>,
    },
    Leaf {
        block: BlockId,
        store: InstructionId,
        value: ValueId,
    },
}

struct PiecewiseLeaf<'a> {
    block: &'a KirBlock,
    edge: &'a KirEdge,
    store: &'a KirInstruction,
}

struct PiecewiseTreeState<'a> {
    root: BlockId,
    blocks: HashMap<BlockId, &'a KirBlock>,
    allowed: &'a BTreeSet<BlockId>,
    members: BTreeSet<BlockId>,
    parents: HashMap<BlockId, (BlockId, KirEdge)>,
    leaves: Vec<PiecewiseLeaf<'a>>,
    join: Option<BlockId>,
}

/// Finds a strictly closed piecewise-store tree rooted at `root`. Unknown or
/// effectful shapes retain their original structured control flow.
pub(super) fn piecewise_closed_store_tree(
    function: &WasmLoweredFunction<'_>,
    root: BlockId,
    allowed: &BTreeSet<BlockId>,
) -> Option<WasmPiecewiseClosedStoreTree> {
    if !allowed.contains(&root) {
        return None;
    }
    let source = function.source;
    let blocks = source
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<HashMap<_, _>>();
    let mut state = PiecewiseTreeState {
        root,
        blocks,
        allowed,
        members: BTreeSet::new(),
        parents: HashMap::new(),
        leaves: Vec::new(),
        join: None,
    };
    let shape = visit_piecewise_tree(function, root, None, &mut state, 0)?;
    if state.leaves.len() < 2 || state.members.len() > 32 {
        return None;
    }
    let join = state.join?;
    if state.members.contains(&join) {
        return None;
    }

    // Every tree block must have exactly its one structural predecessor. This
    // rejects shared subtrees, external entries, and implicit loop cycles.
    for member in &state.members {
        let predecessors = source
            .blocks
            .iter()
            .flat_map(|block| wasm_edges(&block.terminator).into_iter().flatten())
            .filter(|edge| edge.target == *member)
            .count();
        if predecessors != 1 {
            return None;
        }
    }
    if state.parents.len() + 1 != state.members.len() {
        return None;
    }
    for member in &state.members {
        let block = state.blocks.get(member)?;
        if block
            .params
            .iter()
            .any(|param| function.vector_values.contains(&param.value))
            || block
                .instructions
                .iter()
                .flat_map(|instruction| &instruction.results)
                .any(|result| function.vector_values.contains(&result.value))
        {
            return None;
        }
        if *member == root
            && block.instructions.iter().any(|instruction| {
                instruction
                    .memory
                    .as_ref()
                    .is_some_and(|memory| memory.output.is_some())
                    || instruction
                        .effect
                        .as_ref()
                        .is_some_and(|effect| effect.kind != KirEffectKind::ReadMemory)
            })
        {
            return None;
        }
    }

    let join_block = state.blocks.get(&join)?;
    let mut argument_columns =
        vec![Vec::with_capacity(state.leaves.len()); join_block.params.len()];
    for leaf in &state.leaves {
        if leaf.edge.args.len() != join_block.params.len()
            || leaf.edge.memory_args.len() != join_block.memory_params.len()
        {
            return None;
        }
        for (column, value) in argument_columns.iter_mut().zip(&leaf.edge.args) {
            column.push(resolve_piecewise_value(
                *value,
                leaf.block.id,
                root,
                &state.parents,
                &state.blocks,
            ));
        }
    }
    let varying = argument_columns
        .iter()
        .enumerate()
        .filter_map(|(index, values)| {
            values
                .first()
                .is_some_and(|first| values.iter().any(|value| value != first))
                .then_some(index)
        })
        .collect::<Vec<_>>();
    if varying.len() > 1 {
        return None;
    }

    let mut common_store = None;
    let mut common_result_type = None;
    for leaf in &state.leaves {
        let KirInstructionKind::Store { place, value } = &leaf.store.kind else {
            return None;
        };
        let KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } = place.as_ref()
        else {
            return None;
        };
        // Place regions identify source slices. MemorySSA may place writes in
        // the conservative partition instead when the source has no noalias
        // contract, so compare those namespaces independently below.
        let key = (
            resolve_piecewise_value(*slice, leaf.block.id, root, &state.parents, &state.blocks),
            resolve_piecewise_value(*index, leaf.block.id, root, &state.parents, &state.blocks),
            type_node.clone(),
            *region,
        );
        if common_store
            .as_ref()
            .is_some_and(|previous| previous != &key)
        {
            return None;
        }
        common_store = Some(key);
        let value_type = value_types(source).get(value)?.clone();
        if common_result_type
            .as_ref()
            .is_some_and(|previous| previous != &value_type)
            || &value_type != type_node
        {
            return None;
        }
        common_result_type = Some(value_type);
    }

    let mut shared_increment = None;
    let mut increments = BTreeSet::new();
    if let Some(index) = varying.first().copied() {
        let mut common_base = None;
        let mut update_values = Vec::new();
        for (leaf, resolved_edge_value) in state.leaves.iter().zip(&argument_columns[index]) {
            let edge_value = *leaf.edge.args.get(index)?;
            let instruction = leaf.block.instructions.iter().find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == edge_value)
            })?;
            let KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = &instruction.kind
            else {
                return None;
            };
            let left_root =
                resolve_piecewise_value(*left, leaf.block.id, root, &state.parents, &state.blocks);
            let right_root =
                resolve_piecewise_value(*right, leaf.block.id, root, &state.parents, &state.blocks);
            let (base, one) = if piecewise_const_int_is_one(source, *right) {
                (left_root, *right)
            } else if piecewise_const_int_is_one(source, *left) {
                (right_root, *left)
            } else {
                return None;
            };
            if *resolved_edge_value != edge_value
                || !leaf
                    .block
                    .params
                    .iter()
                    .any(|param| param.value == *left || param.value == *right)
                || !piecewise_root_induction_param(source, root, base)
                || one == base
                || common_base.is_some_and(|previous| previous != base)
            {
                return None;
            }
            common_base = Some(base);
            shared_increment.get_or_insert(instruction.id);
            increments.insert(instruction.id);
            update_values.push((leaf.block.id, edge_value));
        }
        if update_values.len() != state.leaves.len()
            || !piecewise_increment_results_are_local(source, &state.members, &update_values, index)
        {
            return None;
        }
    }

    if !piecewise_join_memory_is_closed(
        root,
        join_block,
        &state.leaves,
        &state.parents,
        &state.blocks,
    ) {
        return None;
    }
    if !piecewise_tree_values_are_closed(source, &state.members) {
        return None;
    }
    let cost = piecewise_tree_operation_cost(function, &shape, root, &increments)?;
    let extra = cost.total.saturating_sub(cost.minimum_path);
    // If-conversion computes every arm. Keep that speculative work bounded by
    // a small fixed allowance plus a multiple of the lightest original path;
    // this preserves compact piecewise trees while rejecting a very heavy arm
    // paired with a cheap one (which would multiply work for common inputs).
    let allowed_extra = PIECEWISE_MAX_EXTRA_OPS.saturating_add(
        cost.minimum_path
            .saturating_mul(PIECEWISE_EXTRA_TO_MIN_PATH_RATIO),
    );
    if extra > allowed_extra {
        return None;
    }

    Some(WasmPiecewiseClosedStoreTree {
        shape,
        members: state.members,
        join,
        representative_leaf: state.leaves.first()?.block.id,
        shared_increment,
        stores: state.leaves.iter().map(|leaf| leaf.store.id).collect(),
        increments,
    })
}

#[derive(Debug, Clone, Copy)]
struct PiecewiseOperationCost {
    total: usize,
    minimum_path: usize,
}

fn piecewise_tree_operation_cost(
    function: &WasmLoweredFunction<'_>,
    shape: &WasmPiecewiseTreeShape,
    root: BlockId,
    skipped: &BTreeSet<InstructionId>,
) -> Option<PiecewiseOperationCost> {
    let block_id = match shape {
        WasmPiecewiseTreeShape::Branch { block, .. }
        | WasmPiecewiseTreeShape::Leaf { block, .. } => *block,
    };
    let block = function
        .blocks
        .iter()
        .find(|block| block.source.id == block_id)?;
    let local_cost = if block_id == root {
        0
    } else {
        block
            .source
            .instructions
            .iter()
            .filter(|instruction| !skipped.contains(&instruction.id))
            .map(piecewise_instruction_cost)
            .fold(0usize, usize::saturating_add)
    };
    match shape {
        WasmPiecewiseTreeShape::Leaf { .. } => Some(PiecewiseOperationCost {
            total: local_cost,
            minimum_path: local_cost,
        }),
        WasmPiecewiseTreeShape::Branch {
            then_node,
            else_node,
            ..
        } => {
            let then_cost = piecewise_tree_operation_cost(function, then_node, root, skipped)?;
            let else_cost = piecewise_tree_operation_cost(function, else_node, root, skipped)?;
            Some(PiecewiseOperationCost {
                total: local_cost
                    .saturating_add(then_cost.total)
                    .saturating_add(else_cost.total),
                minimum_path: local_cost
                    .saturating_add(then_cost.minimum_path.min(else_cost.minimum_path)),
            })
        }
    }
}

fn piecewise_instruction_cost(instruction: &KirInstruction) -> usize {
    match &instruction.kind {
        KirInstructionKind::Binary {
            op: MirBinaryOp::Mul,
            ..
        } => 2,
        KirInstructionKind::Binary { .. }
        | KirInstructionKind::Unary { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. } => 1,
        _ => 0,
    }
}

fn visit_piecewise_tree<'a>(
    function: &WasmLoweredFunction<'_>,
    block_id: BlockId,
    parent: Option<(BlockId, &'a KirEdge)>,
    state: &mut PiecewiseTreeState<'a>,
    depth: usize,
) -> Option<WasmPiecewiseTreeShape> {
    if depth > 16 || !state.allowed.contains(&block_id) || !state.members.insert(block_id) {
        return None;
    }
    let block = *state.blocks.get(&block_id)?;
    if let Some((parent_id, edge)) = parent {
        if block.params.len() != edge.args.len()
            || block.memory_params.len() != edge.memory_args.len()
            || block
                .params
                .iter()
                .any(|param| function.vector_values.contains(&param.value))
            || block
                .instructions
                .iter()
                .flat_map(|inst| &inst.results)
                .any(|result| function.vector_values.contains(&result.value))
        {
            return None;
        }
        if state
            .parents
            .insert(block_id, (parent_id, edge.clone()))
            .is_some()
        {
            return None;
        }
    }
    let typed = function
        .blocks
        .iter()
        .find(|lowered| lowered.source.id == block_id)?;
    if typed.edges.iter().any(|edge| {
        function
            .memory_plan
            .edge_actions
            .get(&(block_id, edge.arm))
            .is_some_and(|actions| !actions.is_empty())
            || super::lower::vector_edge_copies(function, edge)
                .map_or(true, |copies| !copies.is_empty())
    }) {
        return None;
    }

    match &block.terminator {
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            if block_id != state.root
                && block
                    .instructions
                    .iter()
                    .any(|instruction| !piecewise_nontrapping_pure_instruction(instruction))
            {
                return None;
            }
            if [then_edge, else_edge].iter().any(|edge| {
                edge.memory_args.len() != block.memory_params.len()
                    || block
                        .memory_params
                        .iter()
                        .zip(&edge.memory_args)
                        .any(|(param, argument)| param.version != *argument)
            }) {
                return None;
            }
            if !matches!(
                value_types(function.source).get(condition),
                Some(MirType::Primitive(MirPrimitiveTypeName::Bool))
            ) || then_edge.target == else_edge.target
                || !piecewise_condition_is_safe(
                    function.source,
                    *condition,
                    state.root,
                    &mut BTreeSet::new(),
                )
            {
                return None;
            }
            let then_node = visit_piecewise_tree(
                function,
                then_edge.target,
                Some((block_id, then_edge)),
                state,
                depth + 1,
            )?;
            let else_node = visit_piecewise_tree(
                function,
                else_edge.target,
                Some((block_id, else_edge)),
                state,
                depth + 1,
            )?;
            Some(WasmPiecewiseTreeShape::Branch {
                block: block_id,
                condition: *condition,
                then_node: Box::new(then_node),
                else_node: Box::new(else_node),
            })
        }
        KirTerminator::Jump { edge } => {
            if block.instructions.iter().any(|instruction| {
                !matches!(instruction.kind, KirInstructionKind::Store { .. })
                    && !piecewise_nontrapping_pure_instruction(instruction)
            }) {
                return None;
            }
            let stores = block
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
                .collect::<Vec<_>>();
            if stores.len() != 1 {
                return None;
            }
            let store = stores[0];
            let KirInstructionKind::Store { place, value } = &store.kind else {
                unreachable!()
            };
            if !matches!(place.as_ref(), KirPlace::SliceIndex { .. })
                || !matches!(
                    &store.memory,
                    Some(KirMemoryAccess {
                        output: Some(_),
                        ..
                    })
                )
                || !matches!(
                    &store.effect,
                    Some(KirOrderedEffect {
                        kind: KirEffectKind::WriteMemory,
                        ..
                    })
                )
                || function
                    .memory_plan
                    .access_by_instruction
                    .contains_key(&store.id)
            {
                return None;
            }
            let join = state.join.get_or_insert(edge.target);
            if *join != edge.target {
                return None;
            }
            let shape = WasmPiecewiseTreeShape::Leaf {
                block: block_id,
                store: store.id,
                value: *value,
            };
            state.leaves.push(PiecewiseLeaf { block, edge, store });
            Some(shape)
        }
        KirTerminator::Return { .. } => None,
    }
}

fn piecewise_nontrapping_pure_instruction(instruction: &KirInstruction) -> bool {
    if instruction.memory.is_some() || instruction.effect.is_some() {
        return false;
    }
    match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. } => true,
        KirInstructionKind::Binary { op, semantics, .. } => {
            !matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod)
                && matches!(
                    semantics,
                    KirArithmeticSemantics::Modular | KirArithmeticSemantics::StrictFloat
                )
        }
        KirInstructionKind::Unary { semantics, .. } => {
            matches!(
                semantics,
                KirArithmeticSemantics::Modular | KirArithmeticSemantics::StrictFloat
            )
        }
        _ => false,
    }
}

fn piecewise_condition_is_safe(
    function: &KirFunction,
    value: ValueId,
    root: BlockId,
    visited: &mut BTreeSet<ValueId>,
) -> bool {
    if !visited.insert(value) {
        return true;
    }
    if function.params.iter().any(|param| param.value == value)
        || function
            .blocks
            .iter()
            .any(|block| block.params.iter().any(|param| param.value == value))
    {
        return true;
    }
    let Some((block, instruction)) = function.blocks.iter().find_map(|block| {
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
    }) else {
        return false;
    };
    let allowed = match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => true,
        KirInstructionKind::Copy { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. }
        | KirInstructionKind::Unary { .. } => true,
        KirInstructionKind::Binary { op, semantics, .. } => {
            !matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod)
                && matches!(
                    semantics,
                    KirArithmeticSemantics::Modular | KirArithmeticSemantics::StrictFloat
                )
        }
        // A common root input load executes at exactly its old point and is
        // shared by both the condition and leaf formulas. Speculating any
        // conditional load would change trap order, so it is not admitted.
        KirInstructionKind::Load { .. } => {
            block.id == root
                && instruction
                    .effect
                    .as_ref()
                    .is_some_and(|effect| effect.kind == KirEffectKind::ReadMemory)
                && instruction
                    .memory
                    .as_ref()
                    .is_some_and(|memory| memory.output.is_none())
        }
        _ => false,
    };
    allowed
        && kir_instruction_uses(instruction)
            .into_iter()
            .all(|operand| piecewise_condition_is_safe(function, operand, root, visited))
}

fn resolve_piecewise_value(
    mut value: ValueId,
    mut block_id: BlockId,
    root: BlockId,
    parents: &HashMap<BlockId, (BlockId, KirEdge)>,
    blocks: &HashMap<BlockId, &KirBlock>,
) -> ValueId {
    while block_id != root {
        let Some(block) = blocks.get(&block_id) else {
            break;
        };
        let Some(index) = block.params.iter().position(|param| param.value == value) else {
            break;
        };
        let Some((parent, edge)) = parents.get(&block_id) else {
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

fn resolve_piecewise_memory(
    mut version: MemoryVersionId,
    mut block_id: BlockId,
    root: BlockId,
    parents: &HashMap<BlockId, (BlockId, KirEdge)>,
    blocks: &HashMap<BlockId, &KirBlock>,
) -> MemoryVersionId {
    while block_id != root {
        let Some(block) = blocks.get(&block_id) else {
            break;
        };
        let Some(index) = block
            .memory_params
            .iter()
            .position(|param| param.version == version)
        else {
            break;
        };
        let Some((parent, edge)) = parents.get(&block_id) else {
            break;
        };
        let Some(argument) = edge.memory_args.get(index) else {
            break;
        };
        version = *argument;
        block_id = *parent;
    }
    version
}

fn piecewise_join_memory_is_closed(
    root: BlockId,
    join: &KirBlock,
    leaves: &[PiecewiseLeaf<'_>],
    parents: &HashMap<BlockId, (BlockId, KirEdge)>,
    blocks: &HashMap<BlockId, &KirBlock>,
) -> bool {
    let Some(first) = leaves.first() else {
        return false;
    };
    let Some(first_memory) = first.store.memory.as_ref() else {
        return false;
    };
    let Some(written_region) = first_memory.output.map(|_| first_memory.region) else {
        return false;
    };
    if join
        .memory_params
        .iter()
        .filter(|param| param.region == written_region)
        .count()
        != 1
    {
        return false;
    }
    for leaf in leaves {
        let Some(memory) = leaf.store.memory.as_ref() else {
            return false;
        };
        if memory.region != written_region || memory.output.is_none() {
            return false;
        }
        let Some(input_param) = leaf
            .block
            .memory_params
            .iter()
            .find(|param| param.region == written_region)
        else {
            return false;
        };
        if memory.input != input_param.version {
            return false;
        }
    }
    for (index, param) in join.memory_params.iter().enumerate() {
        let mut common = None;
        for leaf in leaves {
            let Some(argument) = leaf.edge.memory_args.get(index).copied() else {
                return false;
            };
            if param.region == written_region {
                if Some(argument) != leaf.store.memory.as_ref().and_then(|memory| memory.output) {
                    return false;
                }
            } else {
                let resolved =
                    resolve_piecewise_memory(argument, leaf.block.id, root, parents, blocks);
                if common.is_some_and(|previous| previous != resolved) {
                    return false;
                }
                common = Some(resolved);
            }
        }
    }
    true
}

fn piecewise_increment_results_are_local(
    function: &KirFunction,
    members: &BTreeSet<BlockId>,
    updates: &[(BlockId, ValueId)],
    join_arg_index: usize,
) -> bool {
    for (leaf_block, result) in updates {
        let mut terminator_uses = 0;
        for block in function
            .blocks
            .iter()
            .filter(|block| members.contains(&block.id))
        {
            if block
                .instructions
                .iter()
                .any(|instruction| kir_instruction_uses(instruction).contains(result))
            {
                return false;
            }
            if let KirTerminator::Jump { edge } = &block.terminator {
                for (index, argument) in edge.args.iter().enumerate() {
                    if argument == result {
                        terminator_uses += 1;
                        if block.id != *leaf_block || index != join_arg_index {
                            return false;
                        }
                    }
                }
            } else if kir_terminator_uses(&block.terminator).contains(result) {
                return false;
            }
        }
        if terminator_uses != 1 {
            return false;
        }
    }
    true
}

fn piecewise_const_int_is_one(function: &KirFunction, value: ValueId) -> bool {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .any(|instruction| {
            instruction.results.iter().any(|result| result.value == value)
                && matches!(
                    &instruction.kind,
                    KirInstructionKind::ConstInt { value } if value.replace('_', "").parse::<i128>().ok() == Some(1)
                )
        })
}

fn piecewise_root_induction_param(function: &KirFunction, root: BlockId, value: ValueId) -> bool {
    function
        .blocks
        .iter()
        .find(|block| block.id == root)
        .is_some_and(|block| {
            block.params.iter().any(|param| {
                param.value == value
                    && matches!(
                        param.type_node.as_scalar(),
                        Some(MirType::Primitive(
                            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32
                        ))
                    )
            })
        })
}

fn piecewise_tree_values_are_closed(function: &KirFunction, members: &BTreeSet<BlockId>) -> bool {
    let mut local_values = BTreeSet::new();
    for block in function
        .blocks
        .iter()
        .filter(|block| members.contains(&block.id))
    {
        local_values.extend(block.params.iter().map(|param| param.value));
        local_values.extend(
            block
                .instructions
                .iter()
                .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
        );
    }
    for block in function
        .blocks
        .iter()
        .filter(|block| !members.contains(&block.id))
    {
        if block.instructions.iter().any(|instruction| {
            kir_instruction_uses(instruction)
                .iter()
                .any(|value| local_values.contains(value))
        }) || kir_terminator_uses(&block.terminator)
            .iter()
            .any(|value| local_values.contains(value))
        {
            return false;
        }
    }
    true
}

fn kir_instruction_uses(instruction: &KirInstruction) -> Vec<ValueId> {
    let mut values = Vec::new();
    let mut place_values = |place: &KirPlace| collect_piecewise_place_values(place, &mut values);
    match &instruction.kind {
        KirInstructionKind::Copy { value }
        | KirInstructionKind::Unary { operand: value, .. }
        | KirInstructionKind::Cast { value, .. }
        | KirInstructionKind::SliceData { slice: value }
        | KirInstructionKind::SliceLen { slice: value }
        | KirInstructionKind::VectorSplat { scalar: value, .. }
        | KirInstructionKind::VectorCast { value, .. }
        | KirInstructionKind::VectorExtract { vector: value, .. }
        | KirInstructionKind::VectorReduce { vector: value, .. } => values.push(*value),
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. }
        | KirInstructionKind::MakeSlice {
            data: left,
            len: right,
        } => {
            values.extend([*left, *right]);
        }
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => values.extend(args.iter().copied()),
        KirInstructionKind::Guard { condition, .. } => values.push(*condition),
        KirInstructionKind::Address { place } | KirInstructionKind::Load { place } => {
            place_values(place);
        }
        KirInstructionKind::Store { place, value } => {
            place_values(place);
            values.push(*value);
        }
        KirInstructionKind::Subslice { slice, start, end } => {
            values.extend([*slice, *start, *end]);
        }
        KirInstructionKind::VectorLoad { access, .. } => {
            values.extend([access.slice, access.start, access.end]);
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            values.extend([access.slice, access.start, access.end, *value]);
        }
        KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => {
            values.extend([*left, *right]);
        }
        KirInstructionKind::VectorUnary { operand, .. } => values.push(*operand),
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => {
            values.extend([*mask, *when_true, *when_false]);
        }
        KirInstructionKind::VectorInsert { vector, scalar, .. } => {
            values.extend([*vector, *scalar]);
        }
        KirInstructionKind::VersionPredicate { predicate } => {
            for conjunct in &predicate.conjuncts {
                match conjunct {
                    KirVersionPredicateConjunct::TripThreshold { value, .. } => values.push(*value),
                    KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left,
                        left_count,
                        right,
                        right_count,
                        ..
                    } => {
                        values.extend([*left, *left_count, *right, *right_count]);
                    }
                    KirVersionPredicateConjunct::WasmSliceRange {
                        slice,
                        start,
                        count,
                        ..
                    } => {
                        values.extend([*slice, *start, *count]);
                    }
                }
            }
        }
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => {}
    }
    values
}

fn kir_terminator_uses(terminator: &KirTerminator) -> Vec<ValueId> {
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

fn collect_piecewise_place_values(place: &KirPlace, values: &mut Vec<ValueId>) {
    match place {
        KirPlace::Value { value, .. } => values.push(*value),
        KirPlace::Deref { pointer, .. } => values.push(*pointer),
        KirPlace::Index { base, index, .. } => {
            collect_piecewise_place_values(base, values);
            values.push(*index);
        }
        KirPlace::SliceIndex { slice, index, .. } => values.extend([*slice, *index]),
        KirPlace::Field { base, .. } => collect_piecewise_place_values(base, values),
    }
}

fn check_increment_arm<'lowered, 'source>(
    function: &'lowered WasmLoweredFunction<'source>,
    source: &WasmLoweredBlock<'source>,
    arm: &WasmLoweredBlock<'source>,
    update_edge: &WasmLoweredEdge<'source>,
    direct_edge: &'lowered WasmLoweredEdge<'source>,
    condition: ValueId,
) -> Option<WasmConditionalIncrement<'lowered, 'source>> {
    if arm.source.id == source.source.id
        || arm.source.id == direct_edge.source.target
        || direct_edge.source.target == source.source.id
        || function
            .source
            .blocks
            .iter()
            .flat_map(|block| wasm_edges(&block.terminator))
            .flatten()
            .filter(|edge| edge.target == arm.source.id)
            .count()
            != 1
        || [
            (source.source.id, update_edge.arm),
            (source.source.id, direct_edge.arm),
            (arm.source.id, 0),
        ]
        .iter()
        .any(|edge| {
            function
                .memory_plan
                .edge_actions
                .get(edge)
                .is_some_and(|actions| !actions.is_empty())
        })
    {
        return None;
    }
    let KirTerminator::Jump { edge: join_edge } = &arm.source.terminator else {
        return None;
    };
    if join_edge.target != direct_edge.source.target
        || arm.source.params.len() != update_edge.source.args.len()
        || arm.source.memory_params.len() != update_edge.source.memory_args.len()
        || join_edge.args.len() != direct_edge.source.args.len()
        || join_edge.memory_args.len() != direct_edge.source.memory_args.len()
    {
        return None;
    }
    let resolve = |value: ValueId| {
        arm.source
            .params
            .iter()
            .position(|param| param.value == value)
            .map_or(value, |index| update_edge.source.args[index])
    };
    for (carried, direct) in join_edge
        .memory_args
        .iter()
        .zip(&direct_edge.source.memory_args)
    {
        let resolved = arm
            .source
            .memory_params
            .iter()
            .position(|param| param.version == *carried)
            .map_or(*carried, |index| update_edge.source.memory_args[index]);
        if resolved != *direct {
            return None;
        }
    }
    let mut add = None;
    for instruction in &arm.source.instructions {
        if instruction.memory.is_some()
            || instruction.effect.is_some()
            || instruction.results.len() != 1
        {
            return None;
        }
        match &instruction.kind {
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                semantics: KirArithmeticSemantics::Modular,
                ..
            } => {
                if add.replace(instruction).is_some() {
                    return None;
                }
            }
            KirInstructionKind::ConstInt { value } if value.parse::<u32>().ok() == Some(1) => {}
            _ => return None,
        }
    }
    let add = add?;
    let KirInstructionKind::Binary { left, right, .. } = add.kind else {
        return None;
    };
    let result = &add.results[0];
    if !matches!(
        result.type_node,
        KirValueType::Scalar(MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32
        ))
    ) {
        return None;
    }
    let is_one = |value| {
        function.source.blocks.iter().flat_map(|block| &block.instructions)
        .any(|instruction| matches!(&instruction.kind, KirInstructionKind::ConstInt { value: literal } if literal.parse::<u32>().ok() == Some(1))
            && instruction.results.as_slice().iter().any(|item| item.value == resolve(value) && item.type_node == result.type_node))
    };
    let (base, one) = if is_one(right) {
        (resolve(left), resolve(right))
    } else if is_one(left) {
        (resolve(right), resolve(left))
    } else {
        return None;
    };
    // Any local constant must be precisely the unit operand, with no extra work.
    if arm
        .source
        .instructions
        .iter()
        .filter(|instruction| instruction.id != add.id)
        .any(|instruction| {
            instruction.results[0].value != one
                || instruction.results[0].type_node != result.type_node
        })
    {
        return None;
    }
    let mut changed_index = None;
    for (index, (carried, direct)) in join_edge
        .args
        .iter()
        .zip(&direct_edge.source.args)
        .enumerate()
    {
        if *carried == result.value {
            if *direct != base || changed_index.replace(index).is_some() {
                return None;
            }
        } else if resolve(*carried) != *direct {
            return None;
        }
    }
    let changed_index = changed_index?;
    let definitions = arm
        .source
        .params
        .iter()
        .map(|param| param.value)
        .chain(
            arm.source
                .instructions
                .iter()
                .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
        )
        .collect::<BTreeSet<_>>();
    for block in &function.source.blocks {
        if block.id == arm.source.id {
            continue;
        }
        let mut external_use = false;
        for instruction in &block.instructions {
            crate::visit_instruction_uses(instruction, &mut |value| {
                external_use |= definitions.contains(&value)
            });
        }
        let control_value = match block.terminator {
            KirTerminator::Return { value, .. } => value,
            KirTerminator::Branch { condition, .. } => Some(condition),
            KirTerminator::Jump { .. } => None,
        };
        if external_use
            || control_value.is_some_and(|value| definitions.contains(&value))
            || wasm_edges(&block.terminator)
                .into_iter()
                .flatten()
                .flat_map(|edge| &edge.args)
                .any(|value| definitions.contains(value))
        {
            return None;
        }
    }
    let join = function
        .source
        .blocks
        .iter()
        .find(|block| block.id == join_edge.target)?;
    if join.params.get(changed_index)?.type_node != result.type_node {
        return None;
    }
    let copy_index = join.params[..changed_index]
        .iter()
        .filter(|param| param.type_node.as_scalar().is_some())
        .count();
    let MirInstruction::Move { value, .. } = direct_edge.copies.get(copy_index)? else {
        return None;
    };
    if Some(value) != function.values.get(&base)?.operand.as_ref() {
        return None;
    }
    Some(WasmConditionalIncrement {
        direct_edge,
        condition,
        base,
        result: result.value,
        copy_index,
        increment_on_true: update_edge.arm == 0,
    })
}

fn wasm_edges(terminator: &KirTerminator) -> [Option<&KirEdge>; 2] {
    match terminator {
        KirTerminator::Return { .. } => [None, None],
        KirTerminator::Jump { edge } => [Some(edge), None],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
    }
}

fn wasm_scalar_physical_type(type_node: &MirType) -> Result<WasmPhysicalType, WasmLoweringError> {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => Ok(WasmPhysicalType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            Ok(WasmPhysicalType::I64)
        }
        MirType::Primitive(MirPrimitiveTypeName::F64) => Ok(WasmPhysicalType::F64),
        MirType::Slice(_) => Ok(WasmPhysicalType::I32Pair),
        MirType::Struct(_) | MirType::Void => {
            Err(WasmLoweringError::UnsupportedValueType(type_node.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
        KirValueType, MirType, SourceFile, build_kir_module, check, lower_to_mir,
    };

    use super::{WasmLoweringError, WasmPhysicalType, WasmSourceType, lower_wasm_module};

    fn test_kir() -> (crate::KirModule, crate::MirModule) {
        let checked = check(&SourceFile::new(
            "wasm-typed-lowering.ck",
            r#"
                export fn typed(out: ptr<i32>, i: i32, wide: i64, scale: f64, flag: bool,
                                values: slice<i32>) -> f64 {
                    let result: f64 = scale * 2.0;
                    if flag {
                        out[0] = i;
                    } else {
                        out[0] = i + 1;
                    }
                    return result;
                }
            "#,
        ));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let kir = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR");
        (kir, mir)
    }

    #[test]
    fn wasm_lowering_should_classify_invalid_input_and_cfg_invariants() {
        let (mut module, mir) = test_kir();
        module.profile = crate::KirTargetProfile::portable_c();
        assert!(matches!(
            lower_wasm_module(&module, None, &mir),
            Err(WasmLoweringError::InvalidInput(_))
        ));

        let (mut module, mir) = test_kir();
        let branch = module.functions[0]
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, crate::KirTerminator::Branch { .. }))
            .expect("conditional branch");
        let crate::KirTerminator::Branch { then_edge, .. } = &mut branch.terminator else {
            unreachable!();
        };
        then_edge.target = crate::BlockId::from_index(u32::MAX);
        let error = lower_wasm_module(&module, None, &mir).expect_err("unknown edge target");
        assert!(matches!(error, WasmLoweringError::InvariantFailure(_)));
        assert!(
            error
                .to_string()
                .contains("WebAssembly KIR edge targets unknown block"),
            "{error}"
        );
    }

    #[test]
    fn wasm_lowering_should_classify_unsupported_value_types_and_preserve_display() {
        let checked = check(&SourceFile::new(
            "wasm-typed-unsupported.ck",
            "struct Item { value: i32; } export fn read(item: Item) -> i32 { return item.value; }",
        ));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR");

        let error = lower_wasm_module(&module, None, &mir).expect_err("struct values unsupported");
        assert!(matches!(
            &error,
            WasmLoweringError::UnsupportedValueType(MirType::Struct(name)) if name == "Item"
        ));
        assert_eq!(
            error.to_string(),
            "WebAssembly KIR backend cannot lower value type Struct(\"Item\")"
        );
    }

    #[test]
    fn wasm_lowering_should_retain_typed_values_sources_effects_edges_and_order() {
        let (module, mir) = test_kir();
        let lowered = lower_wasm_module(&module, None, &mir).expect("typed lowering");

        let source = &module.functions[0];
        let function = &lowered.functions[0];
        assert!(std::ptr::eq(function.source, source));
        for param in &source.params {
            let typed = &function.values[&param.value];
            assert!(
                matches!(typed.source_type, WasmSourceType::Mir(source_type) if source_type == &param.type_node)
            );
        }
        let by_name = source
            .params
            .iter()
            .map(|param| (param.name.as_str(), param.value))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            function.values[&by_name["out"]].physical,
            WasmPhysicalType::I32
        );
        assert_eq!(
            function.values[&by_name["i"]].physical,
            WasmPhysicalType::I32
        );
        assert_eq!(
            function.values[&by_name["wide"]].physical,
            WasmPhysicalType::I64
        );
        assert_eq!(
            function.values[&by_name["scale"]].physical,
            WasmPhysicalType::F64
        );
        assert_eq!(
            function.values[&by_name["flag"]].physical,
            WasmPhysicalType::I32
        );
        assert_eq!(
            function.values[&by_name["values"]].physical,
            WasmPhysicalType::I32Pair
        );

        assert_eq!(
            function
                .blocks
                .iter()
                .map(|block| block.source.id)
                .collect::<Vec<_>>(),
            source
                .blocks
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>()
        );
        for (lowered_block, source_block) in function.blocks.iter().zip(&source.blocks) {
            assert!(std::ptr::eq(lowered_block.source, source_block));
            assert_eq!(
                lowered_block
                    .instructions
                    .iter()
                    .map(|instruction| instruction.source.id)
                    .collect::<Vec<_>>(),
                source_block
                    .instructions
                    .iter()
                    .map(|instruction| instruction.id)
                    .collect::<Vec<_>>()
            );
            for instruction in &lowered_block.instructions {
                assert!(
                    source_block
                        .instructions
                        .iter()
                        .any(|original| std::ptr::eq(instruction.source, original))
                );
            }
        }

        let mut saw_effectful_store = false;
        let mut saw_strict_float = false;
        for block in &function.blocks {
            for instruction in &block.instructions {
                if matches!(
                    instruction.source.kind,
                    crate::KirInstructionKind::Store { .. }
                ) {
                    assert!(instruction.source.memory.is_some());
                    assert!(instruction.source.effect.is_some());
                    saw_effectful_store = true;
                }
                if matches!(
                    instruction.source.kind,
                    crate::KirInstructionKind::Binary {
                        semantics: crate::KirArithmeticSemantics::StrictFloat,
                        ..
                    }
                ) {
                    saw_strict_float = true;
                }
            }
            for edge in &block.edges {
                let source_edge = match &block.source.terminator {
                    crate::KirTerminator::Jump { edge } => edge,
                    crate::KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => match edge.arm {
                        0 => then_edge,
                        1 => else_edge,
                        _ => panic!("invalid lowered edge arm"),
                    },
                    crate::KirTerminator::Return { .. } => panic!("return has no edges"),
                };
                assert!(std::ptr::eq(edge.source, source_edge));
                assert_eq!(edge.source.memory_args, source_edge.memory_args);
            }
        }
        assert!(saw_effectful_store);
        assert!(saw_strict_float);
        assert!(function.values.values().any(|value| matches!(
            value.source_type,
            WasmSourceType::Mir(MirType::Slice(_))
                | WasmSourceType::Kir(KirValueType::Scalar(MirType::Slice(_)))
        )));
        assert!(function.values.values().any(|value| matches!(
            &value.source_type,
            WasmSourceType::Kir(KirValueType::Scalar(_))
        )));
    }
}
