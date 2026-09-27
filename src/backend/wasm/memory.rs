use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, ContractFactPointer, ContractFactPredicate, ContractFactSet, ContractInstanceSource,
    FactDerivation, FactId, FactOrigin, FactPredicate, FactScope, FactUseSite, InstructionId,
    KirBlock, KirEdge, KirFunction, KirInstruction, KirInstructionKind, KirPlace, KirTerminator,
    KirValueType, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, ValueId,
    contract_fact_dominates_at,
};

use super::layout::WasmStructLayout;

const MAX_FUNCTION_BLOCKS: usize = 1024;
const MAX_FUNCTION_INSTRUCTIONS: usize = 65_536;
const MAX_MEMORY_CURSORS: usize = 128;
const MAX_PLANNED_ACCESSES: usize = 256;
const MAX_PLAN_SCAN_WORK: usize = 1_000_000;
const MAX_PROVENANCE_HOPS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct CursorId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct WasmMemoryPlan {
    pub cursors: Vec<WasmMemoryCursor>,
    pub access_by_instruction: BTreeMap<InstructionId, CursorId>,
    pub memarg_offset_by_instruction: BTreeMap<InstructionId, WasmMemargOffset>,
    pub edge_actions: BTreeMap<(BlockId, u8), Vec<WasmMemoryEdgeAction>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WasmMemargOffset {
    pub base: ValueId,
    pub index: ValueId,
    pub stride_bytes: u32,
    pub offset_bytes: u32,
    pub access_bytes: u32,
    pub base_alignment: u32,
    pub memarg_alignment: u32,
    pub alignment_fact: FactId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WasmMemoryCursor {
    pub id: CursorId,
    pub element_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WasmMemoryEdgeAction {
    Initialize {
        cursor: CursorId,
        base: CursorOperand,
        induction_arg_index: usize,
        bias_bytes: u32,
    },
    Advance {
        cursor: CursorId,
        delta_bytes: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CursorOperand {
    HeaderArgument(usize),
    Value(ValueId),
}

/// Proposes loop-carried Wasm32 cursors only after proving each indexed access
/// has one stable base and follows the loop's actual induction recurrence.
/// Unsupported or ambiguous shapes retain their original indexed addresses.
#[cfg(test)]
pub(super) fn checked_wasm_memory_plan(function: &KirFunction) -> WasmMemoryPlan {
    checked_wasm_memory_plan_with_evidence(function, None, None)
}

pub(super) fn checked_wasm_memory_plan_with_evidence(
    function: &KirFunction,
    contracts: Option<&ContractFactSet>,
    layout: Option<&WasmStructLayout>,
) -> WasmMemoryPlan {
    if function.blocks.is_empty() || function.blocks.len() > MAX_FUNCTION_BLOCKS {
        return WasmMemoryPlan::default();
    }
    let instruction_count = function.blocks.iter().fold(0_usize, |count, block| {
        count.saturating_add(block.instructions.len())
    });
    if instruction_count > MAX_FUNCTION_INSTRUCTIONS {
        return WasmMemoryPlan::default();
    }
    let Some(definitions) = ValueDefinitions::new(function) else {
        return WasmMemoryPlan::default();
    };
    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<BTreeMap<_, _>>();
    if blocks.len() != function.blocks.len() {
        return WasmMemoryPlan::default();
    }
    let incoming = incoming_edges(function);

    let mut cursors = BTreeMap::<CursorKey, CursorDraft>::new();
    let mut proposer_scan_work = 0_usize;
    let mut planned_access_count = 0_usize;
    for shape in simple_loop_shapes(function, &blocks, &incoming) {
        proposer_scan_work = proposer_scan_work.saturating_add(shape.body.instructions.len());
        if proposer_scan_work > MAX_PLAN_SCAN_WORK {
            return WasmMemoryPlan::default();
        }
        let Some(induction) = checked_induction(&shape, &definitions) else {
            continue;
        };
        if !header_state_is_invariant_except_induction(
            &shape,
            induction.argument_index,
            &definitions,
        ) {
            continue;
        }
        let mut loop_cursors = BTreeMap::<CursorKey, CursorDraft>::new();
        let mut loop_access_count = 0_usize;
        let mut invalid_loop = false;
        for instruction in &shape.body.instructions {
            if instruction.memory.is_none() {
                continue;
            }
            let Some(access) = indexed_access(instruction, &definitions) else {
                invalid_loop = true;
                break;
            };
            if resolve_origin(access.index, &shape, &definitions)
                != Some(CursorOperand::HeaderArgument(induction.argument_index))
            {
                continue;
            }
            if access.index_type != induction.type_node {
                invalid_loop = true;
                break;
            }
            let Some(base) = resolve_origin(access.base, &shape, &definitions) else {
                invalid_loop = true;
                break;
            };
            if !base_type_matches(base, access, &shape, &definitions)
                || !base_is_stable(base, &shape, &definitions)
            {
                invalid_loop = true;
                break;
            }
            let key = CursorKey {
                header: shape.header.id,
                base,
                induction_argument_index: induction.argument_index,
                element_bytes: access.element_bytes,
            };
            let draft = loop_cursors.entry(key).or_insert_with(|| CursorDraft {
                entry_edge: EdgeKey {
                    source: shape.entry.source.id,
                    arm: shape.entry.arm,
                },
                backedge: EdgeKey {
                    source: shape.backedge.source.id,
                    arm: shape.backedge.arm,
                },
                delta_bytes: induction.delta_bytes(access.element_bytes),
                accesses: Vec::new(),
            });
            if draft.entry_edge
                != (EdgeKey {
                    source: shape.entry.source.id,
                    arm: shape.entry.arm,
                })
                || draft.backedge
                    != (EdgeKey {
                        source: shape.backedge.source.id,
                        arm: shape.backedge.arm,
                    })
                || draft.delta_bytes != induction.delta_bytes(access.element_bytes)
            {
                continue;
            }
            draft.accesses.push(instruction.id);
            loop_access_count = loop_access_count.saturating_add(1);
            if loop_access_count > MAX_PLANNED_ACCESSES {
                invalid_loop = true;
                break;
            }
        }
        if !invalid_loop {
            planned_access_count = planned_access_count.saturating_add(loop_access_count);
            if planned_access_count > MAX_PLANNED_ACCESSES {
                return WasmMemoryPlan::default();
            }
            for (key, draft) in loop_cursors {
                cursors.insert(key, draft);
            }
        }
    }

    if cursors.len() > MAX_MEMORY_CURSORS {
        return WasmMemoryPlan::default();
    }
    let access_count = cursors
        .values()
        .map(|draft| draft.accesses.len())
        .fold(0_usize, usize::saturating_add);
    if access_count > MAX_PLANNED_ACCESSES {
        return WasmMemoryPlan::default();
    }
    let mut plan = WasmMemoryPlan::default();
    for (index, (key, mut draft)) in cursors.into_iter().enumerate() {
        draft.accesses.sort_unstable();
        draft.accesses.dedup();
        if draft.accesses.is_empty() {
            continue;
        }
        let Ok(raw_id) = u32::try_from(index) else {
            return WasmMemoryPlan::default();
        };
        let cursor = CursorId(raw_id);
        plan.cursors.push(WasmMemoryCursor {
            id: cursor,
            element_bytes: key.element_bytes,
        });
        for instruction in draft.accesses {
            if plan
                .access_by_instruction
                .insert(instruction, cursor)
                .is_some()
            {
                return WasmMemoryPlan::default();
            }
        }
        plan.edge_actions
            .entry((draft.entry_edge.source, draft.entry_edge.arm))
            .or_default()
            .push(WasmMemoryEdgeAction::Initialize {
                cursor,
                base: key.base,
                induction_arg_index: key.induction_argument_index,
                bias_bytes: 0,
            });
        plan.edge_actions
            .entry((draft.backedge.source, draft.backedge.arm))
            .or_default()
            .push(WasmMemoryEdgeAction::Advance {
                cursor,
                delta_bytes: draft.delta_bytes,
            });
    }
    for actions in plan.edge_actions.values_mut() {
        actions.sort_by_key(|action| match action {
            WasmMemoryEdgeAction::Initialize { cursor, .. }
            | WasmMemoryEdgeAction::Advance { cursor, .. } => *cursor,
        });
    }
    if let (Some(contracts), Some(layout)) = (contracts, layout)
        && function.blocks.len() == 1
        && matches!(function.blocks[0].terminator, KirTerminator::Return { .. })
    {
        let mut scan_work = 0_usize;
        let _ = propose_memarg_offset_folds(
            &mut plan,
            function,
            contracts,
            layout,
            &definitions,
            &mut scan_work,
        );
    }
    if independently_validate_plan_with_evidence(function, contracts, layout, &plan) {
        plan
    } else {
        WasmMemoryPlan::default()
    }
}

fn propose_memarg_offset_folds(
    plan: &mut WasmMemoryPlan,
    function: &KirFunction,
    contracts: &ContractFactSet,
    layout: &WasmStructLayout,
    definitions: &ValueDefinitions<'_>,
    scan_work: &mut usize,
) -> bool {
    let Some(alignment_facts) = proposer_alignment_fact_index(function, contracts, scan_work)
    else {
        plan.memarg_offset_by_instruction.clear();
        return false;
    };
    if alignment_facts.is_empty() {
        plan.memarg_offset_by_instruction.clear();
        return true;
    }
    for block in &function.blocks {
        for instruction in &block.instructions {
            if !consume_scan_work(scan_work, 1) {
                plan.memarg_offset_by_instruction.clear();
                return false;
            }
            let Some(fold) = propose_memarg_offset_fold(
                function,
                block,
                instruction,
                contracts,
                &alignment_facts,
                layout,
                definitions,
            ) else {
                continue;
            };
            plan.memarg_offset_by_instruction
                .insert(instruction.id, fold);
            if plan.memarg_offset_by_instruction.len() > MAX_PLANNED_ACCESSES {
                plan.memarg_offset_by_instruction.clear();
                return false;
            }
        }
    }
    true
}

fn propose_memarg_offset_fold(
    function: &KirFunction,
    block: &KirBlock,
    instruction: &KirInstruction,
    contracts: &ContractFactSet,
    alignment_facts: &BTreeMap<ValueId, FactId>,
    layout: &WasmStructLayout,
    definitions: &ValueDefinitions<'_>,
) -> Option<WasmMemargOffset> {
    if function.blocks.len() != 1
        || !matches!(block.terminator, KirTerminator::Return { .. })
        || instruction.memory.is_none()
    {
        return None;
    }
    let KirInstructionKind::Load { place } = &instruction.kind else {
        return None;
    };
    let (base, index, struct_type, field_name, access_type) =
        simple_indexed_field_load(place, definitions)?;
    let MirType::Struct(struct_name) = struct_type else {
        return None;
    };
    let stride = u32::try_from(layout.size_of(struct_type)).ok()?;
    let offset = u32::try_from(layout.field_offset(struct_name, field_name)).ok()?;
    let access_bytes = u32::try_from(layout.size_of(access_type)).ok()?;
    let natural_alignment = u32::try_from(layout.align_of(access_type)).ok()?;
    let base_alignment = 16;
    let guaranteed_alignment = power_of_two_gcd(base_alignment, offset);
    let memarg_alignment = guaranteed_alignment
        .min(natural_alignment)
        .min(access_bytes);
    if stride == 0
        || stride % base_alignment != 0
        || offset == 0
        || offset >= base_alignment
        || offset >= stride
        || access_bytes == 0
        || offset.checked_add(access_bytes)? > stride
        || memarg_alignment == 0
        || !memarg_alignment.is_power_of_two()
    {
        return None;
    }
    let alignment_fact = proposer_entry_alignment_fact(
        contracts,
        alignment_facts,
        function,
        block.id,
        instruction.id,
        base,
        base_alignment,
    )?;
    Some(WasmMemargOffset {
        base,
        index,
        stride_bytes: stride,
        offset_bytes: offset,
        access_bytes,
        base_alignment,
        memarg_alignment,
        alignment_fact,
    })
}

fn simple_indexed_field_load<'a>(
    place: &'a KirPlace,
    definitions: &ValueDefinitions<'_>,
) -> Option<(ValueId, ValueId, &'a MirType, &'a str, &'a MirType)> {
    let KirPlace::Field {
        base: indexed,
        field_name,
        type_node: access_type,
        ..
    } = place
    else {
        return None;
    };
    let KirPlace::Index {
        base,
        index,
        type_node: struct_type,
        ..
    } = indexed.as_ref()
    else {
        return None;
    };
    let KirPlace::Value {
        value: base,
        type_node: base_type,
        ..
    } = base.as_ref()
    else {
        return None;
    };
    let MirType::Pointer(pointee) = base_type else {
        return None;
    };
    if pointee.as_ref() != struct_type
        || definitions.scalar_type(*base)? != base_type
        || !matches!(
            definitions.scalar_type(*index)?,
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
        )
        || primitive_element_bytes(access_type).is_none()
    {
        return None;
    }
    Some((*base, *index, struct_type, field_name, access_type))
}

fn proposer_entry_alignment_fact(
    contracts: &ContractFactSet,
    alignment_facts: &BTreeMap<ValueId, FactId>,
    function: &KirFunction,
    block: BlockId,
    instruction: InstructionId,
    pointer: ValueId,
    alignment: u32,
) -> Option<FactId> {
    let fact_id = *alignment_facts.get(&pointer)?;
    let fact = contracts.facts().get(fact_id)?;
    let FactOrigin::TrustedContract { instance } = fact.origin else {
        return None;
    };
    if fact.id == fact_id
        && fact.predicate
            == FactPredicate::Contract(ContractFactPredicate::Aligned {
                pointer: ContractFactPointer::Value(pointer),
                alignment,
            })
        && fact.origin == (FactOrigin::TrustedContract { instance })
        && fact.scope == FactScope::FunctionEntry(function.id)
        && fact.derivation == FactDerivation::TrustedContractLeaf
        && fact.generation == contracts.facts().generation()
        && contract_fact_dominates_at(
            contracts,
            fact.id,
            FactUseSite {
                function: function.id,
                block,
                instruction: Some(instruction),
                contract_instance: Some(instance),
            },
        )
    {
        Some(fact_id)
    } else {
        None
    }
}

fn proposer_alignment_fact_index(
    function: &KirFunction,
    contracts: &ContractFactSet,
    scan_work: &mut usize,
) -> Option<BTreeMap<ValueId, FactId>> {
    let entry = function.blocks.first()?;
    let mut params = BTreeMap::new();
    for param in &function.params {
        if !consume_scan_work(scan_work, 1) {
            return None;
        }
        if matches!(param.type_node, MirType::Pointer(_))
            && params.insert(param.value, param).is_some()
        {
            return None;
        }
    }

    let mut result = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for instance in contracts.instances() {
        if !consume_scan_work(scan_work, 1) {
            return None;
        }
        if instance.callee != function.id
            || instance.source != ContractInstanceSource::FunctionEntry
        {
            continue;
        }
        let mut bindings = BTreeMap::new();
        let mut bindings_valid = true;
        for binding in &instance.bindings {
            if !consume_scan_work(scan_work, 1) {
                return None;
            }
            if bindings
                .insert(binding.parameter.as_str(), binding.value)
                .is_some()
            {
                bindings_valid = false;
            }
        }
        if !bindings_valid {
            continue;
        }
        for fact_id in &instance.facts {
            if !consume_scan_work(scan_work, 1) {
                return None;
            }
            let fact = contracts.facts().get(*fact_id)?;
            let FactPredicate::Contract(ContractFactPredicate::Aligned {
                pointer: ContractFactPointer::Value(pointer),
                alignment: 16,
            }) = &fact.predicate
            else {
                continue;
            };
            let Some(param) = params.get(pointer) else {
                continue;
            };
            if bindings.get(param.name.as_str()) != Some(pointer)
                || fact.id != *fact_id
                || fact.origin
                    != (FactOrigin::TrustedContract {
                        instance: instance.id,
                    })
                || fact.scope != FactScope::FunctionEntry(function.id)
                || fact.derivation != FactDerivation::TrustedContractLeaf
                || fact.generation != contracts.facts().generation()
                || !contract_fact_dominates_at(
                    contracts,
                    fact.id,
                    FactUseSite {
                        function: function.id,
                        block: entry.id,
                        instruction: None,
                        contract_instance: Some(instance.id),
                    },
                )
            {
                continue;
            }
            if result
                .insert(*pointer, *fact_id)
                .is_some_and(|previous| previous != *fact_id)
            {
                ambiguous.insert(*pointer);
            }
        }
    }
    for pointer in ambiguous {
        result.remove(&pointer);
    }
    Some(result)
}

fn consume_scan_work(work: &mut usize, amount: usize) -> bool {
    *work = work.saturating_add(amount);
    *work <= MAX_PLAN_SCAN_WORK
}

fn power_of_two_gcd(left: u32, right: u32) -> u32 {
    if right == 0 {
        left
    } else {
        1_u32 << left.trailing_zeros().min(right.trailing_zeros())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct EdgeKey {
    source: BlockId,
    arm: u8,
}

#[derive(Debug, Clone, Copy)]
struct LocatedEdge<'a> {
    source: &'a KirBlock,
    arm: u8,
    edge: &'a KirEdge,
}

#[derive(Debug)]
struct SimpleLoop<'a> {
    header: &'a KirBlock,
    body: &'a KirBlock,
    entry: LocatedEdge<'a>,
    body_entry: LocatedEdge<'a>,
    backedge: LocatedEdge<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CursorKey {
    header: BlockId,
    base: CursorOperand,
    induction_argument_index: usize,
    element_bytes: u32,
}

#[derive(Debug)]
struct CursorDraft {
    entry_edge: EdgeKey,
    backedge: EdgeKey,
    delta_bytes: u32,
    accesses: Vec<InstructionId>,
}

#[derive(Debug, Clone, Copy)]
enum ValueDefinition<'a> {
    FunctionParam,
    BlockParam {
        block: BlockId,
        index: usize,
    },
    Instruction {
        block: BlockId,
        instruction: &'a KirInstruction,
    },
}

struct ValueDefinitions<'a> {
    definitions: BTreeMap<ValueId, ValueDefinition<'a>>,
    scalar_types: BTreeMap<ValueId, &'a MirType>,
}

impl<'a> ValueDefinitions<'a> {
    fn new(function: &'a KirFunction) -> Option<Self> {
        let mut definitions = BTreeMap::new();
        let mut scalar_types = BTreeMap::new();
        for param in &function.params {
            definitions.insert(param.value, ValueDefinition::FunctionParam);
            scalar_types.insert(param.value, &param.type_node);
        }
        for block in &function.blocks {
            for (index, param) in block.params.iter().enumerate() {
                if definitions
                    .insert(
                        param.value,
                        ValueDefinition::BlockParam {
                            block: block.id,
                            index,
                        },
                    )
                    .is_some()
                {
                    return None;
                }
                if let KirValueType::Scalar(type_node) = &param.type_node {
                    scalar_types.insert(param.value, type_node);
                }
            }
            for instruction in &block.instructions {
                for result in &instruction.results {
                    if definitions
                        .insert(
                            result.value,
                            ValueDefinition::Instruction {
                                block: block.id,
                                instruction,
                            },
                        )
                        .is_some()
                    {
                        return None;
                    }
                    if let KirValueType::Scalar(type_node) = &result.type_node {
                        scalar_types.insert(result.value, type_node);
                    }
                }
            }
        }
        Some(Self {
            definitions,
            scalar_types,
        })
    }

    fn definition(&self, value: ValueId) -> Option<ValueDefinition<'a>> {
        self.definitions.get(&value).copied()
    }

    fn scalar_type(&self, value: ValueId) -> Option<&'a MirType> {
        self.scalar_types.get(&value).copied()
    }
}

fn outgoing_edges(block: &KirBlock) -> Vec<LocatedEdge<'_>> {
    match &block.terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![LocatedEdge {
            source: block,
            arm: 0,
            edge,
        }],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![
            LocatedEdge {
                source: block,
                arm: 0,
                edge: then_edge,
            },
            LocatedEdge {
                source: block,
                arm: 1,
                edge: else_edge,
            },
        ],
    }
}

fn incoming_edges<'a>(function: &'a KirFunction) -> BTreeMap<BlockId, Vec<LocatedEdge<'a>>> {
    let mut incoming = BTreeMap::<BlockId, Vec<LocatedEdge<'a>>>::new();
    for block in &function.blocks {
        for edge in outgoing_edges(block) {
            incoming.entry(edge.edge.target).or_default().push(edge);
        }
    }
    for edges in incoming.values_mut() {
        edges.sort_by_key(|edge| (edge.source.id, edge.arm));
    }
    incoming
}

fn simple_loop_shapes<'a>(
    function: &'a KirFunction,
    blocks: &BTreeMap<BlockId, &'a KirBlock>,
    incoming: &BTreeMap<BlockId, Vec<LocatedEdge<'a>>>,
) -> Vec<SimpleLoop<'a>> {
    let mut loops = Vec::new();
    let Some(entry_block) = function.blocks.first().map(|block| block.id) else {
        return loops;
    };
    for header in &function.blocks {
        if header.id == entry_block {
            continue;
        }
        let header_edges = outgoing_edges(header);
        if header_edges.len() != 2 {
            continue;
        }
        for body_arm in 0_u8..2 {
            let body_entry = header_edges[usize::from(body_arm)];
            let exit = header_edges[usize::from(1 - body_arm)];
            if body_entry.edge.target == header.id || body_entry.edge.target == exit.edge.target {
                continue;
            }
            let Some(body) = blocks.get(&body_entry.edge.target).copied() else {
                continue;
            };
            // The first block has an implicit function-entry edge that does
            // not appear in the explicit incoming-edge map. A cursor init on
            // a different preheader could therefore be skipped entirely.
            if body.id == entry_block {
                continue;
            }
            let body_edges = outgoing_edges(body);
            let [backedge] = body_edges.as_slice() else {
                continue;
            };
            if backedge.edge.target != header.id || exit.edge.target == body.id {
                continue;
            }
            if body_entry.edge.args.len() != body.params.len()
                || backedge.edge.args.len() != header.params.len()
            {
                continue;
            }
            let Some(body_incoming) = incoming.get(&body.id) else {
                continue;
            };
            if body_incoming.len() != 1
                || body_incoming[0].source.id != header.id
                || body_incoming[0].arm != body_arm
            {
                continue;
            }
            let Some(header_incoming) = incoming.get(&header.id) else {
                continue;
            };
            if header_incoming.len() != 2 {
                continue;
            }
            let mut entry = None;
            let mut unique_backedge = false;
            for edge in header_incoming {
                if edge.source.id == body.id {
                    if edge.arm != 0 || unique_backedge {
                        unique_backedge = false;
                        break;
                    }
                    unique_backedge = true;
                } else if entry.replace(*edge).is_some() {
                    entry = None;
                    break;
                }
            }
            let Some(entry) = entry else {
                continue;
            };
            if !unique_backedge
                || entry.edge.args.len() != header.params.len()
                || entry.source.id == header.id
                || entry.source.id == body.id
                || exit.edge.target == header.id
                || exit.edge.target == body.id
            {
                continue;
            }
            loops.push(SimpleLoop {
                header,
                body,
                entry,
                body_entry,
                backedge: *backedge,
            });
        }
    }
    loops
}

#[derive(Debug, Clone, Copy)]
struct InductionProof<'a> {
    argument_index: usize,
    type_node: &'a MirType,
    step: u32,
}

impl InductionProof<'_> {
    fn delta_bytes(self, element_bytes: u32) -> u32 {
        self.step.wrapping_mul(element_bytes)
    }
}

fn checked_induction<'a>(
    shape: &SimpleLoop<'a>,
    definitions: &ValueDefinitions<'a>,
) -> Option<InductionProof<'a>> {
    let mut candidates = Vec::new();
    for (index, param) in shape.header.params.iter().enumerate() {
        let Some(type_node) = scalar_integer32_type(&param.type_node) else {
            continue;
        };
        let Some(step) = induction_step(index, type_node, shape, definitions) else {
            continue;
        };
        if !condition_compares_induction(index, shape, definitions) {
            continue;
        }
        candidates.push(InductionProof {
            argument_index: index,
            type_node,
            step,
        });
    }
    (candidates.len() == 1).then(|| candidates[0])
}

fn header_state_is_invariant_except_induction(
    shape: &SimpleLoop<'_>,
    induction_argument_index: usize,
    definitions: &ValueDefinitions<'_>,
) -> bool {
    shape
        .header
        .params
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != induction_argument_index)
        .all(|(index, _)| {
            shape
                .backedge
                .edge
                .args
                .get(index)
                .and_then(|value| resolve_origin(*value, shape, definitions))
                == Some(CursorOperand::HeaderArgument(index))
        })
}

fn induction_step(
    argument_index: usize,
    induction_type: &MirType,
    shape: &SimpleLoop<'_>,
    definitions: &ValueDefinitions<'_>,
) -> Option<u32> {
    let value = *shape.backedge.edge.args.get(argument_index)?;
    let instruction = match definitions.definition(value)? {
        ValueDefinition::Instruction { block, instruction } if block == shape.body.id => {
            instruction
        }
        _ => return None,
    };
    let KirInstructionKind::Binary {
        op,
        left,
        right,
        semantics: crate::KirArithmeticSemantics::Modular,
    } = instruction.kind
    else {
        return None;
    };
    if instruction.results.first()?.type_node.as_scalar()? != induction_type {
        return None;
    }
    let left_is_induction = resolve_origin(left, shape, definitions)
        == Some(CursorOperand::HeaderArgument(argument_index));
    let right_is_induction = resolve_origin(right, shape, definitions)
        == Some(CursorOperand::HeaderArgument(argument_index));
    let step = match (op, left_is_induction, right_is_induction) {
        (MirBinaryOp::Add, true, false) => constant_i32(right, induction_type, definitions)?,
        (MirBinaryOp::Add, false, true) => constant_i32(left, induction_type, definitions)?,
        (MirBinaryOp::Sub, true, false) => -constant_i32(right, induction_type, definitions)?,
        _ => return None,
    };
    Some(step.rem_euclid(1_i128 << 32) as u32)
}

fn constant_i32(
    value: ValueId,
    expected_type: &MirType,
    definitions: &ValueDefinitions<'_>,
) -> Option<i128> {
    if definitions.scalar_type(value)? != expected_type {
        return None;
    }
    let ValueDefinition::Instruction { instruction, .. } = definitions.definition(value)? else {
        return None;
    };
    match &instruction.kind {
        KirInstructionKind::ConstInt { value } => value.parse().ok(),
        _ => None,
    }
}

fn condition_compares_induction(
    argument_index: usize,
    shape: &SimpleLoop<'_>,
    definitions: &ValueDefinitions<'_>,
) -> bool {
    let KirTerminator::Branch { condition, .. } = shape.header.terminator else {
        return false;
    };
    let Some(ValueDefinition::Instruction { block, instruction }) =
        definitions.definition(condition)
    else {
        return false;
    };
    if block != shape.header.id {
        return false;
    }
    let KirInstructionKind::Compare { op, left, right } = instruction.kind else {
        return false;
    };
    if !matches!(
        op,
        MirCompareOp::Eq
            | MirCompareOp::Ne
            | MirCompareOp::Lt
            | MirCompareOp::Le
            | MirCompareOp::Gt
            | MirCompareOp::Ge
    ) {
        return false;
    }
    let induction = Some(CursorOperand::HeaderArgument(argument_index));
    let left_induction = resolve_origin(left, shape, definitions) == induction;
    let right_induction = resolve_origin(right, shape, definitions) == induction;
    left_induction ^ right_induction
}

#[derive(Debug, Clone, Copy)]
struct IndexedAccess<'a> {
    base: ValueId,
    index: ValueId,
    index_type: &'a MirType,
    element_type: &'a MirType,
    element_bytes: u32,
    slice_base: bool,
}

fn indexed_access<'a>(
    instruction: &'a KirInstruction,
    definitions: &ValueDefinitions<'a>,
) -> Option<IndexedAccess<'a>> {
    match &instruction.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
            indexed_place(place, definitions)
        }
        _ => None,
    }
}

fn indexed_place<'a>(
    place: &'a KirPlace,
    definitions: &ValueDefinitions<'a>,
) -> Option<IndexedAccess<'a>> {
    let (base, index, element_type, slice_base) = match place {
        KirPlace::Index {
            base,
            index,
            type_node,
            ..
        } => {
            let KirPlace::Value {
                value,
                type_node: base_type,
                ..
            } = base.as_ref()
            else {
                return None;
            };
            let MirType::Pointer(pointee) = base_type else {
                return None;
            };
            if pointee.as_ref() != type_node || definitions.scalar_type(*value)? != base_type {
                return None;
            }
            (*value, *index, type_node, false)
        }
        // Slice indexing is intentionally left to the existing structured
        // loop path. Cursor initialization currently runs on explicit CFG
        // edges, which would force conditional-backedge loops through the
        // typed dispatcher and regress their established Wasm shape.
        KirPlace::SliceIndex { .. } => return None,
        _ => return None,
    };
    let element_bytes = primitive_element_bytes(element_type)?;
    Some(IndexedAccess {
        base,
        index,
        index_type: definitions.scalar_type(index)?,
        element_type,
        element_bytes,
        slice_base,
    })
}

fn scalar_integer32_type(type_node: &KirValueType) -> Option<&MirType> {
    match type_node {
        KirValueType::Scalar(MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32,
        )) => type_node.as_scalar(),
        _ => None,
    }
}

fn primitive_element_bytes(type_node: &MirType) -> Option<u32> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => Some(8),
        _ => None,
    }
}

fn base_type_matches(
    base: CursorOperand,
    access: IndexedAccess<'_>,
    shape: &SimpleLoop<'_>,
    definitions: &ValueDefinitions<'_>,
) -> bool {
    let base_type = match base {
        CursorOperand::HeaderArgument(index) => shape
            .header
            .params
            .get(index)
            .and_then(|param| param.type_node.as_scalar()),
        CursorOperand::Value(value) => definitions.scalar_type(value),
    };
    match (access.slice_base, base_type) {
        (true, Some(MirType::Slice(element))) => element.as_ref() == access.element_type,
        (false, Some(MirType::Pointer(element))) => element.as_ref() == access.element_type,
        _ => false,
    }
}

fn base_is_stable(
    base: CursorOperand,
    shape: &SimpleLoop<'_>,
    definitions: &ValueDefinitions<'_>,
) -> bool {
    match base {
        CursorOperand::Value(value) => !matches!(
            definitions.definition(value),
            Some(ValueDefinition::BlockParam { block, .. } | ValueDefinition::Instruction { block, .. })
                if block == shape.header.id || block == shape.body.id
        ),
        CursorOperand::HeaderArgument(index) => {
            let Some(backedge_value) = shape.backedge.edge.args.get(index).copied() else {
                return false;
            };
            resolve_origin(backedge_value, shape, definitions)
                == Some(CursorOperand::HeaderArgument(index))
        }
    }
}

fn resolve_origin(
    value: ValueId,
    shape: &SimpleLoop<'_>,
    definitions: &ValueDefinitions<'_>,
) -> Option<CursorOperand> {
    let mut current = value;
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..MAX_PROVENANCE_HOPS {
        if !visited.insert(current) {
            return None;
        }
        if let Some((index, _)) = shape
            .header
            .params
            .iter()
            .enumerate()
            .find(|(_, param)| param.value == current)
        {
            return Some(CursorOperand::HeaderArgument(index));
        }
        match definitions.definition(current)? {
            ValueDefinition::BlockParam { block, index } if block == shape.body.id => {
                current = *shape.body_entry.edge.args.get(index)?;
            }
            ValueDefinition::BlockParam { block, .. }
                if block == shape.header.id || block == shape.body.id =>
            {
                return None;
            }
            ValueDefinition::Instruction { block, instruction }
                if block == shape.header.id || block == shape.body.id =>
            {
                let KirInstructionKind::Copy { value } = instruction.kind else {
                    return None;
                };
                current = value;
            }
            ValueDefinition::FunctionParam | ValueDefinition::BlockParam { .. } => {
                return Some(CursorOperand::Value(current));
            }
            ValueDefinition::Instruction { .. } => {
                return Some(CursorOperand::Value(current));
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckedCursorPath {
    entry_edge: EdgeKey,
    backedge: EdgeKey,
    base: CursorOperand,
    induction_argument_index: usize,
    element_bytes: u32,
    delta_bytes: u32,
}

#[derive(Debug, Clone)]
struct CheckedIndexedAccess {
    base: ValueId,
    index: ValueId,
    index_type: MirType,
    element_type: MirType,
    element_bytes: u32,
    slice_base: bool,
}

#[cfg(test)]
fn independently_validate_plan(function: &KirFunction, plan: &WasmMemoryPlan) -> bool {
    independently_validate_plan_with_evidence(function, None, None, plan)
}

fn independently_validate_plan_with_evidence(
    function: &KirFunction,
    contracts: Option<&ContractFactSet>,
    layout: Option<&WasmStructLayout>,
    plan: &WasmMemoryPlan,
) -> bool {
    let instruction_count = function.blocks.iter().fold(0_usize, |count, block| {
        count.saturating_add(block.instructions.len())
    });
    let scan_count = instruction_count.saturating_add(function.blocks.len());
    let estimated_scan_work = scan_count.saturating_mul(
        1_usize
            .saturating_add(plan.access_by_instruction.len())
            .saturating_add(plan.memarg_offset_by_instruction.len())
            .saturating_add(plan.cursors.len()),
    );
    if plan.cursors.len() > MAX_MEMORY_CURSORS
        || plan.access_by_instruction.len() > MAX_PLANNED_ACCESSES
        || plan.memarg_offset_by_instruction.len() > MAX_PLANNED_ACCESSES
        || function.blocks.len() > MAX_FUNCTION_BLOCKS
        || instruction_count > MAX_FUNCTION_INSTRUCTIONS
        || estimated_scan_work > MAX_PLAN_SCAN_WORK
        || plan
            .cursors
            .iter()
            .any(|cursor| !matches!(cursor.element_bytes, 4 | 8))
    {
        return false;
    }
    let Some(definitions) = ValueDefinitions::new(function) else {
        return false;
    };
    let alignment_facts = if plan.memarg_offset_by_instruction.is_empty() {
        BTreeMap::new()
    } else {
        let Some(contracts) = contracts else {
            return false;
        };
        let mut fact_scan_work = 0_usize;
        let Some(facts) = checker_alignment_fact_index(function, contracts, &mut fact_scan_work)
        else {
            return false;
        };
        facts
    };
    let mut cursors = BTreeMap::new();
    for cursor in &plan.cursors {
        if cursors.insert(cursor.id, cursor.element_bytes).is_some() {
            return false;
        }
    }
    let mut instruction_map = BTreeMap::<InstructionId, (&KirBlock, &KirInstruction)>::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if instruction_map
                .insert(instruction.id, (block, instruction))
                .is_some()
            {
                return false;
            }
        }
    }
    let action_count = plan
        .edge_actions
        .values()
        .map(Vec::len)
        .fold(0_usize, usize::saturating_add);
    if action_count > MAX_MEMORY_CURSORS.saturating_mul(2) {
        return false;
    }
    let mut cursor_paths = BTreeMap::<CursorId, CheckedCursorPath>::new();
    let mut cursor_access_counts = BTreeMap::<CursorId, usize>::new();
    for (&instruction_id, &cursor_id) in &plan.access_by_instruction {
        let Some((owner, instruction)) = instruction_map.get(&instruction_id).copied() else {
            return false;
        };
        if !cursors.contains_key(&cursor_id) {
            return false;
        }
        let Some(access) = checker_indexed_access(instruction, &definitions) else {
            return false;
        };
        let Some(path) = checker_rederive_access_path(function, owner, &access, &definitions)
        else {
            return false;
        };
        if cursors.get(&cursor_id) != Some(&path.element_bytes) {
            return false;
        }
        if cursor_paths
            .get(&cursor_id)
            .is_some_and(|previous| previous != &path)
        {
            return false;
        }
        cursor_paths.insert(cursor_id, path.clone());
        *cursor_access_counts.entry(cursor_id).or_default() += 1;
    }
    if cursor_paths.len() != cursors.len() {
        return false;
    }

    for (&cursor, &element_bytes) in &cursors {
        let Some(path) = cursor_paths.get(&cursor) else {
            return false;
        };
        if cursor_access_counts.get(&cursor).copied().unwrap_or(0) == 0 {
            return false;
        }
        let mut init_count = 0;
        let mut advance_count = 0;
        for (&edge, actions) in &plan.edge_actions {
            for action in actions {
                match action {
                    WasmMemoryEdgeAction::Initialize {
                        cursor: action_cursor,
                        base,
                        induction_arg_index,
                        bias_bytes,
                    } if *action_cursor == cursor => {
                        init_count += 1;
                        if edge != (path.entry_edge.source, path.entry_edge.arm)
                            || *base != path.base
                            || *induction_arg_index != path.induction_argument_index
                            || *bias_bytes != 0
                        {
                            return false;
                        }
                    }
                    WasmMemoryEdgeAction::Advance {
                        cursor: action_cursor,
                        delta_bytes,
                    } if *action_cursor == cursor => {
                        advance_count += 1;
                        if edge != (path.backedge.source, path.backedge.arm)
                            || *delta_bytes != path.delta_bytes
                        {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
        }
        if init_count != 1 || advance_count != 1 || element_bytes != path.element_bytes {
            return false;
        }

        // A loop is transformed as one unit: every direct indexed memory
        // access using this induction must have a matching proven cursor.
        let Some(body) = function
            .blocks
            .iter()
            .find(|block| block.id == path.backedge.source)
        else {
            return false;
        };
        for instruction in &body.instructions {
            if instruction.memory.is_none() {
                continue;
            }
            if checker_indexed_access(instruction, &definitions).is_none() {
                return false;
            }
            if checker_indexed_memory_index(instruction).is_some_and(|index| {
                checker_index_header_argument(function, body, index, &definitions)
                    == Some(path.induction_argument_index)
            }) {
                let Some(mapped) = plan.access_by_instruction.get(&instruction.id) else {
                    return false;
                };
                let Some(mapped_path) = cursor_paths.get(mapped) else {
                    return false;
                };
                if mapped_path.entry_edge != path.entry_edge
                    || mapped_path.backedge != path.backedge
                    || mapped_path.induction_argument_index != path.induction_argument_index
                {
                    return false;
                }
            }
        }
    }
    for (&instruction_id, &fold) in &plan.memarg_offset_by_instruction {
        let (Some(contracts), Some(layout)) = (contracts, layout) else {
            return false;
        };
        if plan.access_by_instruction.contains_key(&instruction_id) {
            return false;
        }
        let Some((block, instruction)) = instruction_map.get(&instruction_id).copied() else {
            return false;
        };
        let Some(expected) = checker_rederive_memarg_offset(
            function,
            block,
            instruction,
            contracts,
            &alignment_facts,
            layout,
            &definitions,
        ) else {
            return false;
        };
        if expected != fold {
            return false;
        }
    }
    plan.edge_actions.values().all(|actions| {
        !actions.is_empty()
            && actions.iter().all(|action| match action {
                WasmMemoryEdgeAction::Initialize { cursor, .. }
                | WasmMemoryEdgeAction::Advance { cursor, .. } => cursors.contains_key(cursor),
            })
    })
}

fn checker_rederive_memarg_offset(
    function: &KirFunction,
    block: &KirBlock,
    instruction: &KirInstruction,
    contracts: &ContractFactSet,
    alignment_facts: &BTreeMap<ValueId, FactId>,
    layout: &WasmStructLayout,
    definitions: &ValueDefinitions<'_>,
) -> Option<WasmMemargOffset> {
    if function.blocks.len() != 1
        || !matches!(block.terminator, KirTerminator::Return { .. })
        || instruction.memory.is_none()
    {
        return None;
    }
    let KirInstructionKind::Load { place } = &instruction.kind else {
        return None;
    };
    let KirPlace::Field {
        base: struct_element,
        field_name,
        type_node: load_type,
        ..
    } = place.as_ref()
    else {
        return None;
    };
    let KirPlace::Index {
        base: pointer_place,
        index,
        type_node: element_type,
        ..
    } = struct_element.as_ref()
    else {
        return None;
    };
    let KirPlace::Value {
        value: pointer_value,
        type_node: pointer_type,
        ..
    } = pointer_place.as_ref()
    else {
        return None;
    };
    let MirType::Pointer(element_pointee) = pointer_type else {
        return None;
    };
    let MirType::Struct(struct_name) = element_type else {
        return None;
    };
    if element_pointee.as_ref() != element_type
        || definitions.scalar_type(*pointer_value)? != pointer_type
        || !matches!(
            definitions.scalar_type(*index)?,
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
        )
        || primitive_element_bytes(load_type).is_none()
    {
        return None;
    }
    let stride_bytes = u32::try_from(layout.size_of(element_type)).ok()?;
    let offset_bytes = u32::try_from(layout.field_offset(struct_name, field_name)).ok()?;
    let access_bytes = u32::try_from(layout.size_of(load_type)).ok()?;
    let access_alignment = u32::try_from(layout.align_of(load_type)).ok()?;
    let base_alignment = 16;
    if stride_bytes % base_alignment != 0
        || offset_bytes == 0
        || offset_bytes >= base_alignment
        || offset_bytes >= stride_bytes
        || offset_bytes.checked_add(access_bytes)? > stride_bytes
    {
        return None;
    }
    let memarg_alignment = power_of_two_gcd(base_alignment, offset_bytes)
        .min(access_alignment)
        .min(access_bytes);
    if memarg_alignment == 0 || !memarg_alignment.is_power_of_two() {
        return None;
    }

    let alignment_fact = *alignment_facts.get(pointer_value)?;
    let fact = contracts.facts().get(alignment_fact)?;
    let FactOrigin::TrustedContract { instance } = fact.origin else {
        return None;
    };
    if fact.id != alignment_fact
        || fact.origin != (FactOrigin::TrustedContract { instance })
        || fact.scope != FactScope::FunctionEntry(function.id)
        || fact.derivation != FactDerivation::TrustedContractLeaf
        || fact.generation != contracts.facts().generation()
        || fact.predicate
            != FactPredicate::Contract(ContractFactPredicate::Aligned {
                pointer: ContractFactPointer::Value(*pointer_value),
                alignment: base_alignment,
            })
        || !contract_fact_dominates_at(
            contracts,
            fact.id,
            FactUseSite {
                function: function.id,
                block: block.id,
                instruction: Some(instruction.id),
                contract_instance: Some(instance),
            },
        )
    {
        return None;
    }
    Some(WasmMemargOffset {
        base: *pointer_value,
        index: *index,
        stride_bytes,
        offset_bytes,
        access_bytes,
        base_alignment,
        memarg_alignment,
        alignment_fact,
    })
}

fn checker_alignment_fact_index(
    function: &KirFunction,
    contracts: &ContractFactSet,
    scan_work: &mut usize,
) -> Option<BTreeMap<ValueId, FactId>> {
    let entry_block = function.blocks.first()?;
    let mut pointer_parameters = BTreeMap::new();
    for parameter in &function.params {
        if !consume_scan_work(scan_work, 1) {
            return None;
        }
        if matches!(parameter.type_node, MirType::Pointer(_))
            && pointer_parameters
                .insert(parameter.value, parameter.name.as_str())
                .is_some()
        {
            return None;
        }
    }

    let mut checked = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    for contract_instance in contracts.instances() {
        if !consume_scan_work(scan_work, 1) {
            return None;
        }
        if contract_instance.callee != function.id
            || !matches!(
                contract_instance.source,
                ContractInstanceSource::FunctionEntry
            )
        {
            continue;
        }
        let mut parameter_values = BTreeMap::new();
        let mut unique_bindings = true;
        for binding in &contract_instance.bindings {
            if !consume_scan_work(scan_work, 1) {
                return None;
            }
            if parameter_values
                .insert(binding.parameter.as_str(), binding.value)
                .is_some()
            {
                unique_bindings = false;
            }
        }
        if !unique_bindings {
            continue;
        }
        for fact_id in &contract_instance.facts {
            if !consume_scan_work(scan_work, 1) {
                return None;
            }
            let fact = contracts.facts().get(*fact_id)?;
            let FactPredicate::Contract(ContractFactPredicate::Aligned {
                pointer: ContractFactPointer::Value(pointer),
                alignment: 16,
            }) = &fact.predicate
            else {
                continue;
            };
            let Some(parameter_name) = pointer_parameters.get(pointer) else {
                continue;
            };
            if parameter_values.get(*parameter_name) != Some(pointer)
                || fact.id != *fact_id
                || fact.origin
                    != (FactOrigin::TrustedContract {
                        instance: contract_instance.id,
                    })
                || fact.scope != FactScope::FunctionEntry(function.id)
                || fact.derivation != FactDerivation::TrustedContractLeaf
                || fact.generation != contracts.facts().generation()
                || !contract_fact_dominates_at(
                    contracts,
                    fact.id,
                    FactUseSite {
                        function: function.id,
                        block: entry_block.id,
                        instruction: None,
                        contract_instance: Some(contract_instance.id),
                    },
                )
            {
                continue;
            }
            if checked
                .insert(*pointer, *fact_id)
                .is_some_and(|previous| previous != *fact_id)
            {
                conflicts.insert(*pointer);
            }
        }
    }
    for pointer in conflicts {
        checked.remove(&pointer);
    }
    Some(checked)
}

fn checker_indexed_memory_index(instruction: &KirInstruction) -> Option<ValueId> {
    let place = match &instruction.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => place,
        _ => return None,
    };
    match place.as_ref() {
        KirPlace::Index { index, .. } | KirPlace::SliceIndex { index, .. } => Some(*index),
        _ => None,
    }
}

fn checker_index_header_argument(
    function: &KirFunction,
    body: &KirBlock,
    index: ValueId,
    definitions: &ValueDefinitions<'_>,
) -> Option<usize> {
    let body_index = body.params.iter().position(|param| param.value == index)?;
    let body_entry_edge = function
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                if then_edge.target == body.id {
                    Some(then_edge)
                } else if else_edge.target == body.id {
                    Some(else_edge)
                } else {
                    None
                }
            }
            KirTerminator::Jump { edge } if edge.target == body.id => Some(edge),
            _ => None,
        })?;
    let forwarded = *body_entry_edge.args.get(body_index)?;
    let header = function
        .blocks
        .iter()
        .find(|block| block.params.iter().any(|param| param.value == forwarded))?;
    let argument_index = header
        .params
        .iter()
        .position(|param| param.value == forwarded)?;
    let scalar_type = definitions.scalar_type(index)?;
    (definitions.scalar_type(forwarded)? == scalar_type).then_some(argument_index)
}

fn checker_indexed_access(
    instruction: &KirInstruction,
    definitions: &ValueDefinitions<'_>,
) -> Option<CheckedIndexedAccess> {
    instruction.memory.as_ref()?;
    let place = match &instruction.kind {
        KirInstructionKind::Load { place } => place.as_ref(),
        KirInstructionKind::Store { place, value } => {
            if definitions.scalar_type(*value)? != place_element_type(place.as_ref())? {
                return None;
            }
            place.as_ref()
        }
        _ => return None,
    };
    let (base, index, element_type, slice_base) = match place {
        KirPlace::Index {
            base,
            index,
            type_node,
            ..
        } => {
            let KirPlace::Value {
                value,
                type_node: pointer_type,
                ..
            } = base.as_ref()
            else {
                return None;
            };
            let MirType::Pointer(pointee) = pointer_type else {
                return None;
            };
            if pointee.as_ref() != type_node || definitions.scalar_type(*value)? != pointer_type {
                return None;
            }
            (*value, *index, type_node.clone(), false)
        }
        KirPlace::SliceIndex { .. } => return None,
        _ => return None,
    };
    let element_bytes = checker_element_width(&element_type)?;
    let index_type = definitions.scalar_type(index)?.clone();
    if let KirInstructionKind::Load { .. } = instruction.kind
        && instruction.results.first()?.type_node.as_scalar()? != &element_type
    {
        return None;
    }
    Some(CheckedIndexedAccess {
        base,
        index,
        index_type,
        element_type,
        element_bytes,
        slice_base,
    })
}

fn place_element_type(place: &KirPlace) -> Option<&MirType> {
    match place {
        KirPlace::Index { type_node, .. }
        | KirPlace::SliceIndex { type_node, .. }
        | KirPlace::Deref { type_node, .. }
        | KirPlace::Field { type_node, .. } => Some(type_node),
        KirPlace::Value { .. } => None,
    }
}

fn checker_element_width(type_node: &MirType) -> Option<u32> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => Some(8),
        _ => None,
    }
}

fn checker_rederive_access_path(
    function: &KirFunction,
    body: &KirBlock,
    access: &CheckedIndexedAccess,
    definitions: &ValueDefinitions<'_>,
) -> Option<CheckedCursorPath> {
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return None;
    };
    if function.blocks.first()?.id == body.id {
        return None;
    }
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == backedge.target)?;
    if function.blocks.first()?.id == header.id {
        return None;
    }
    let header_edges = outgoing_edges(header);
    if header_edges.len() != 2 {
        return None;
    }
    let body_edges = header_edges
        .iter()
        .filter(|edge| edge.edge.target == body.id)
        .copied()
        .collect::<Vec<_>>();
    if body_edges.len() != 1 {
        return None;
    }
    let body_entry = body_edges[0];
    let exit = header_edges
        .iter()
        .find(|edge| edge.arm != body_entry.arm)
        .copied()?;
    if exit.edge.target == header.id || exit.edge.target == body.id {
        return None;
    }
    if body_entry.edge.args.len() != body.params.len() || backedge.args.len() != header.params.len()
    {
        return None;
    }

    let mut header_incoming = Vec::new();
    let mut body_incoming = Vec::new();
    for source in &function.blocks {
        for edge in outgoing_edges(source) {
            if edge.edge.target == header.id {
                header_incoming.push(edge);
            }
            if edge.edge.target == body.id {
                body_incoming.push(edge);
            }
        }
    }
    if header_incoming.len() != 2
        || !header_incoming.iter().any(|edge| {
            edge.source.id == body.id && edge.arm == 0 && edge.edge.args == backedge.args
        })
        || body_incoming.len() != 1
        || body_incoming[0].source.id != header.id
        || body_incoming[0].arm != body_entry.arm
    {
        return None;
    }
    let entry = header_incoming
        .iter()
        .find(|edge| edge.source.id != body.id)
        .copied()?;
    if entry.source.id == header.id
        || entry.edge.args.len() != header.params.len()
        || entry.edge.args.len() != backedge.args.len()
    {
        return None;
    }

    let body_induction_index = body
        .params
        .iter()
        .position(|param| param.value == access.index)?;
    let transported_induction = *body_entry.edge.args.get(body_induction_index)?;
    let induction_argument_index = header
        .params
        .iter()
        .position(|param| param.value == transported_induction)?;
    let induction_type = header.params[induction_argument_index]
        .type_node
        .as_scalar()?;
    if !matches!(
        induction_type,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
    ) || access.index_type != *induction_type
        || definitions.scalar_type(transported_induction)? != induction_type
    {
        return None;
    }
    if !(0..header.params.len())
        .filter(|index| *index != induction_argument_index)
        .all(|index| {
            checker_header_argument_is_stable(header, body, body_entry.edge, backedge, index)
        })
    {
        return None;
    }

    let update_value = *backedge.args.get(induction_argument_index)?;
    let ValueDefinition::Instruction {
        block: update_block,
        instruction: update,
    } = definitions.definition(update_value)?
    else {
        return None;
    };
    if update_block != body.id || update.results.first()?.type_node.as_scalar()? != induction_type {
        return None;
    }
    let KirInstructionKind::Binary {
        op,
        left,
        right,
        semantics: crate::KirArithmeticSemantics::Modular,
    } = update.kind
    else {
        return None;
    };
    let body_induction = body.params[body_induction_index].value;
    let left_is_induction = left == body_induction;
    let right_is_induction = right == body_induction;
    let step = match (op, left_is_induction, right_is_induction) {
        (MirBinaryOp::Add, true, false) => {
            checker_integer_constant(right, induction_type, definitions)?
        }
        (MirBinaryOp::Add, false, true) => {
            checker_integer_constant(left, induction_type, definitions)?
        }
        (MirBinaryOp::Sub, true, false) => {
            -checker_integer_constant(right, induction_type, definitions)?
        }
        _ => return None,
    };
    let delta = step
        .rem_euclid(1_i128 << 32)
        .wrapping_mul(i128::from(access.element_bytes))
        .rem_euclid(1_i128 << 32) as u32;

    let KirTerminator::Branch { condition, .. } = &header.terminator else {
        return None;
    };
    let ValueDefinition::Instruction {
        block: compare_block,
        instruction: compare,
    } = definitions.definition(*condition)?
    else {
        return None;
    };
    if compare_block != header.id {
        return None;
    }
    let KirInstructionKind::Compare { left, right, .. } = compare.kind else {
        return None;
    };
    let induction_value = header.params[induction_argument_index].value;
    let other = if left == induction_value {
        right
    } else if right == induction_value {
        left
    } else {
        return None;
    };
    if !checker_header_value_is_stable(header, body, body_entry.edge, backedge, other) {
        return None;
    }

    let (base_operand, base_argument_index) = checker_base_operand(
        function,
        header,
        body,
        body_entry.edge,
        backedge,
        access,
        definitions,
    )?;
    let base_type = match base_operand {
        CursorOperand::HeaderArgument(index) => header.params.get(index)?.type_node.as_scalar()?,
        CursorOperand::Value(value) => definitions.scalar_type(value)?,
    };
    let expected_base_type = if access.slice_base {
        MirType::Slice(Box::new(access.element_type.clone()))
    } else {
        MirType::Pointer(Box::new(access.element_type.clone()))
    };
    if base_type != &expected_base_type {
        return None;
    }
    if let Some(index) = base_argument_index
        && !checker_header_argument_is_stable(header, body, body_entry.edge, backedge, index)
    {
        return None;
    }

    Some(CheckedCursorPath {
        entry_edge: EdgeKey {
            source: entry.source.id,
            arm: entry.arm,
        },
        backedge: EdgeKey {
            source: body.id,
            arm: 0,
        },
        base: base_operand,
        induction_argument_index,
        element_bytes: access.element_bytes,
        delta_bytes: delta,
    })
}

fn checker_integer_constant(
    value: ValueId,
    expected_type: &MirType,
    definitions: &ValueDefinitions<'_>,
) -> Option<i128> {
    if definitions.scalar_type(value)? != expected_type {
        return None;
    }
    let ValueDefinition::Instruction { instruction, .. } = definitions.definition(value)? else {
        return None;
    };
    let KirInstructionKind::ConstInt { value } = &instruction.kind else {
        return None;
    };
    value.parse().ok()
}

fn checker_header_argument_is_stable(
    header: &KirBlock,
    body: &KirBlock,
    body_entry: &KirEdge,
    backedge: &KirEdge,
    argument_index: usize,
) -> bool {
    let Some(header_param) = header.params.get(argument_index) else {
        return false;
    };
    let Some(backedge_value) = backedge.args.get(argument_index) else {
        return false;
    };
    if backedge_value == &header_param.value {
        return true;
    }
    body.params
        .iter()
        .enumerate()
        .any(|(body_index, body_param)| {
            body_entry.args.get(body_index) == Some(&header_param.value)
                && backedge_value == &body_param.value
        })
}

fn checker_header_value_is_stable(
    header: &KirBlock,
    body: &KirBlock,
    body_entry: &KirEdge,
    backedge: &KirEdge,
    value: ValueId,
) -> bool {
    if let Some(argument_index) = header.params.iter().position(|param| param.value == value) {
        return checker_header_argument_is_stable(
            header,
            body,
            body_entry,
            backedge,
            argument_index,
        );
    }
    true
}

fn checker_base_operand(
    function: &KirFunction,
    header: &KirBlock,
    body: &KirBlock,
    body_entry: &KirEdge,
    backedge: &KirEdge,
    access: &CheckedIndexedAccess,
    definitions: &ValueDefinitions<'_>,
) -> Option<(CursorOperand, Option<usize>)> {
    if let Some(body_index) = body
        .params
        .iter()
        .position(|param| param.value == access.base)
    {
        let forwarded = *body_entry.args.get(body_index)?;
        if let Some(argument_index) = header
            .params
            .iter()
            .position(|param| param.value == forwarded)
        {
            let header_param = &header.params[argument_index];
            let index_is_stable = body.params.iter().enumerate().any(|(index, param)| {
                body_entry.args.get(index) == Some(&header_param.value)
                    && backedge.args.get(argument_index) == Some(&param.value)
            });
            if !index_is_stable {
                return None;
            }
            return Some((
                CursorOperand::HeaderArgument(argument_index),
                Some(argument_index),
            ));
        }
        let is_loop_defined = match definitions.definition(forwarded)? {
            ValueDefinition::BlockParam { block, .. }
            | ValueDefinition::Instruction { block, .. } => block == header.id || block == body.id,
            ValueDefinition::FunctionParam => false,
        };
        if is_loop_defined {
            return None;
        }
        return Some((CursorOperand::Value(forwarded), None));
    }
    if let Some(argument_index) = header
        .params
        .iter()
        .position(|param| param.value == access.base)
    {
        return checker_header_argument_is_stable(
            header,
            body,
            body_entry,
            backedge,
            argument_index,
        )
        .then_some((
            CursorOperand::HeaderArgument(argument_index),
            Some(argument_index),
        ));
    }
    let is_loop_defined = match definitions.definition(access.base)? {
        ValueDefinition::BlockParam { block, .. } | ValueDefinition::Instruction { block, .. } => {
            block == header.id || block == body.id
        }
        ValueDefinition::FunctionParam => false,
    };
    if is_loop_defined {
        return None;
    }
    let _ = function;
    Some((CursorOperand::Value(access.base), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirModule, KirOptimizationLevel,
        KirOverflowMode, KirSanitizerMode, SourceFile, build_kir_module, check,
        import_contract_facts, lower_to_mir, run_kir_pass_pipeline,
    };

    const COPY_U32: &str = r#"
export unsafe fn copy_u32(dst: ptr<u32>, src: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i];
    i = i + 1;
  }
}
"#;

    const SUM_F64: &str = r#"
export fn sum_f64(values: ptr<f64>, len: i32) -> f64 {
  let i: i32 = 0;
  let total: f64 = 0.0;
  while i < len {
    total = total + values[i];
    i = i + 1;
  }
  return total;
}
"#;

    const ALIGNED_SLOT_LOAD: &str = r#"
struct Slot {
  head: u32;
  value: u32;
  tail: u64;
}
export unsafe fn load_aligned(items: ptr<Slot>, index: u32) -> u32 contract {
  requires aligned(items, 16);
} {
  return items[index].value;
}
"#;

    fn optimized_module_with_contracts(source: &str) -> (KirModule, ContractFactSet) {
        let checked = check(&SourceFile::new("wasm-memory-plan.ck", source));
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
        .expect("valid WebAssembly KIR");
        let contracts = import_contract_facts(&module, &checked.checked_program, 0)
            .expect("valid contract facts");
        let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        (
            optimized.artifact.expect("verified optimized KIR"),
            optimized.contract_facts.expect("verified contract facts"),
        )
    }

    fn valid_memarg_plan() -> (
        KirFunction,
        ContractFactSet,
        WasmStructLayout,
        WasmMemoryPlan,
    ) {
        let (module, contracts) = optimized_module_with_contracts(ALIGNED_SLOT_LOAD);
        let function = module
            .functions
            .iter()
            .find(|function| function.name == "load_aligned")
            .expect("aligned slot function")
            .clone();
        let mir = crate::MirModule {
            entry: module.entry.clone(),
            structs: module.structs.clone(),
            functions: Vec::new(),
        };
        let layout = WasmStructLayout::new(&mir);
        let plan =
            checked_wasm_memory_plan_with_evidence(&function, Some(&contracts), Some(&layout));
        assert_eq!(plan.memarg_offset_by_instruction.len(), 1);
        assert!(independently_validate_plan_with_evidence(
            &function,
            Some(&contracts),
            Some(&layout),
            &plan,
        ));
        (function, contracts, layout, plan)
    }

    fn optimized_function(source: &str) -> KirFunction {
        let checked = check(&SourceFile::new("wasm-memory-plan.ck", source));
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
        .expect("valid WebAssembly KIR");
        let contracts = import_contract_facts(&module, &checked.checked_program, 0)
            .expect("valid contract facts");
        let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        optimized
            .artifact
            .expect("verified optimized KIR")
            .functions
            .into_iter()
            .next()
            .expect("compiled function")
    }

    fn optimized_copy_function() -> KirFunction {
        optimized_function(COPY_U32)
    }

    fn valid_plan_and_function() -> (KirFunction, WasmMemoryPlan) {
        let function = optimized_copy_function();
        let plan = checked_wasm_memory_plan(&function);
        assert_eq!(plan.cursors.len(), 2, "source and destination cursors");
        assert_eq!(
            plan.access_by_instruction.len(),
            2,
            "load and store mappings"
        );
        assert!(independently_validate_plan(&function, &plan));
        (function, plan)
    }

    #[test]
    fn independent_checker_should_reject_wrong_init_edge() {
        let (function, mut plan) = valid_plan_and_function();
        let (entry_edge, action_index, initialize) = plan
            .edge_actions
            .iter()
            .find_map(|(edge, actions)| {
                actions
                    .iter()
                    .position(|action| matches!(action, WasmMemoryEdgeAction::Initialize { .. }))
                    .map(|index| (*edge, index, actions[index]))
            })
            .expect("cursor initialization");
        let backedge = plan
            .edge_actions
            .iter()
            .find_map(|(edge, actions)| {
                actions
                    .iter()
                    .any(|action| matches!(action, WasmMemoryEdgeAction::Advance { .. }))
                    .then_some(*edge)
            })
            .expect("cursor advance");
        assert_ne!(entry_edge, backedge);
        let entry_actions = plan
            .edge_actions
            .get_mut(&entry_edge)
            .expect("entry edge actions");
        assert_eq!(entry_actions.remove(action_index), initialize);
        plan.edge_actions
            .entry(backedge)
            .or_default()
            .push(initialize);

        assert!(!independently_validate_plan(&function, &plan));
    }

    #[test]
    fn independent_checker_should_reject_wrong_induction_argument() {
        let (function, mut plan) = valid_plan_and_function();
        let initialize = plan
            .edge_actions
            .values_mut()
            .flatten()
            .find(|action| matches!(action, WasmMemoryEdgeAction::Initialize { .. }))
            .expect("cursor initialization");
        let WasmMemoryEdgeAction::Initialize {
            induction_arg_index,
            ..
        } = initialize
        else {
            unreachable!();
        };
        *induction_arg_index = induction_arg_index.saturating_add(1);

        assert!(!independently_validate_plan(&function, &plan));
    }

    #[test]
    fn independent_checker_should_reject_wrong_cursor_delta() {
        let (function, mut plan) = valid_plan_and_function();
        let advance = plan
            .edge_actions
            .values_mut()
            .flatten()
            .find(|action| matches!(action, WasmMemoryEdgeAction::Advance { .. }))
            .expect("cursor advance");
        let WasmMemoryEdgeAction::Advance { delta_bytes, .. } = advance else {
            unreachable!();
        };
        *delta_bytes = delta_bytes.wrapping_add(1);

        assert!(!independently_validate_plan(&function, &plan));
    }

    #[test]
    fn independent_checker_should_reject_wrong_access_instruction_id() {
        let (function, mut plan) = valid_plan_and_function();
        let (&instruction, &cursor) = plan
            .access_by_instruction
            .iter()
            .next()
            .expect("memory access mapping");
        plan.access_by_instruction.remove(&instruction);
        plan.access_by_instruction
            .insert(InstructionId::from_index(u32::MAX), cursor);

        assert!(!independently_validate_plan(&function, &plan));
    }

    #[test]
    fn independent_checker_should_reject_mutated_memarg_layout_and_provenance() {
        let (function, contracts, layout, plan) = valid_memarg_plan();
        let (&instruction, &fold) = plan
            .memarg_offset_by_instruction
            .iter()
            .next()
            .expect("checked field load");

        let mut wrong_offset = plan.clone();
        wrong_offset
            .memarg_offset_by_instruction
            .get_mut(&instruction)
            .expect("field fold")
            .offset_bytes += 1;
        assert!(!independently_validate_plan_with_evidence(
            &function,
            Some(&contracts),
            Some(&layout),
            &wrong_offset,
        ));

        let mut wrong_base = plan.clone();
        wrong_base
            .memarg_offset_by_instruction
            .get_mut(&instruction)
            .expect("field fold")
            .base = function.params[1].value;
        assert!(!independently_validate_plan_with_evidence(
            &function,
            Some(&contracts),
            Some(&layout),
            &wrong_base,
        ));

        let mut wrong_index = plan.clone();
        wrong_index
            .memarg_offset_by_instruction
            .get_mut(&instruction)
            .expect("field fold")
            .index = function.params[0].value;
        assert!(!independently_validate_plan_with_evidence(
            &function,
            Some(&contracts),
            Some(&layout),
            &wrong_index,
        ));

        let mut wrong_fact = plan.clone();
        wrong_fact
            .memarg_offset_by_instruction
            .get_mut(&instruction)
            .expect("field fold")
            .alignment_fact = FactId::from_index(u32::MAX);
        assert!(!independently_validate_plan_with_evidence(
            &function,
            Some(&contracts),
            Some(&layout),
            &wrong_fact,
        ));

        let mut wrong_instruction = plan;
        wrong_instruction
            .memarg_offset_by_instruction
            .remove(&instruction);
        wrong_instruction
            .memarg_offset_by_instruction
            .insert(InstructionId::from_index(u32::MAX), fold);
        assert!(!independently_validate_plan_with_evidence(
            &function,
            Some(&contracts),
            Some(&layout),
            &wrong_instruction,
        ));
    }

    #[test]
    fn memarg_fact_indexes_should_fail_closed_at_the_scan_budget() {
        let (_, contracts, layout, _) = valid_memarg_plan();
        let cursor_function = optimized_copy_function();
        let definitions = ValueDefinitions::new(&cursor_function).expect("value definitions");
        let mut cursor_plan = checked_wasm_memory_plan(&cursor_function);
        assert_eq!(cursor_plan.cursors.len(), 2, "P8a cursor plan is present");
        let original_actions = cursor_plan.edge_actions.clone();

        let mut proposer_work = MAX_PLAN_SCAN_WORK;
        assert!(!propose_memarg_offset_folds(
            &mut cursor_plan,
            &cursor_function,
            &contracts,
            &layout,
            &definitions,
            &mut proposer_work,
        ));
        assert!(cursor_plan.memarg_offset_by_instruction.is_empty());
        assert_eq!(cursor_plan.cursors.len(), 2);
        assert_eq!(cursor_plan.edge_actions, original_actions);
        assert!(independently_validate_plan(&cursor_function, &cursor_plan));

        let mut checker_work = MAX_PLAN_SCAN_WORK;
        assert!(
            checker_alignment_fact_index(&cursor_function, &contracts, &mut checker_work,)
                .is_none()
        );
    }

    #[test]
    fn planner_should_reject_function_entry_as_loop_body() {
        let mut function = optimized_copy_function();
        let plan = checked_wasm_memory_plan(&function);
        let backedge_source = plan
            .edge_actions
            .iter()
            .find_map(|(edge, actions)| {
                actions
                    .iter()
                    .any(|action| matches!(action, WasmMemoryEdgeAction::Advance { .. }))
                    .then_some(edge.0)
            })
            .expect("cursor backedge");
        let body_index = function
            .blocks
            .iter()
            .position(|block| block.id == backedge_source)
            .expect("loop body block");
        let body = function.blocks.remove(body_index);
        function.blocks.insert(0, body);

        assert!(checked_wasm_memory_plan(&function).cursors.is_empty());
        assert!(!independently_validate_plan(&function, &plan));
    }

    #[test]
    fn planner_should_reject_loop_with_reduction_state() {
        let function = optimized_function(SUM_F64);

        assert!(checked_wasm_memory_plan(&function).cursors.is_empty());
    }
}
