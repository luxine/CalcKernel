use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, CandidateKey, FactId, FunctionId, InstructionId, KirAlignmentClass,
    KirArithmeticSemantics, KirCostEstimate, KirCostKey, KirCostSemantics, KirInstruction,
    KirInstructionKind, KirLaneType, KirOperationAvailability, KirProfileOperation, KirTerminator,
    LoopCandidateKind, LoopCandidateVariant, LoopId, MirBinaryOp, MirPrimitiveTypeName, MirType,
    ValueId,
};

use super::analyze_canonical_loops_for_discovery;
use crate::optimizer::vectorize_check::{
    checked_loop_bound_is_invariant, stable_invariant_descriptor_root,
};

const MAX_INDEX_DAG_INSTRUCTIONS: usize = 16;
const MAX_COST_SEARCH_TRIP: u32 = 4096;
const MINIMUM_COST_REDUCTION_PERCENT: u32 = 10;
const BASELINE_RANGE_PREDICATE_COST: u32 = 8;
const BASELINE_GUARDED_UNROLL_FACTOR: u32 = 8;
const BASELINE_LOOP_BRANCH_COST: u32 = 1;

/// A source-proven scalar load that is invariant across one Wasm32 loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInvariantLoadCandidate {
    pub key: CandidateKey,
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub preheader: BlockId,
    pub header: BlockId,
    pub body: BlockId,
    pub exit: BlockId,
    pub induction: ValueId,
    pub bound: ValueId,
    pub input_slice: ValueId,
    pub input_region: crate::MemoryRegionId,
    pub input_partition: crate::MemoryRegionId,
    pub output_slice: ValueId,
    /// Source loop output index. Re-materializing this with the column IV
    /// replaced by zero yields the base of every write in the loop interval.
    pub output_index: ValueId,
    pub output_range_dag: Vec<InstructionId>,
    pub output_region: crate::MemoryRegionId,
    pub output_partition: crate::MemoryRegionId,
    pub load: InstructionId,
    pub load_value: ValueId,
    pub induction_update: InstructionId,
    pub index: ValueId,
    /// Pure source instructions needed to reconstruct `index` at the preheader.
    pub index_dag: Vec<InstructionId>,
    /// Index instructions that become dead in the cloned loop after replacing
    /// the source load with the loop-invariant value.
    pub elidable_index_dag: Vec<InstructionId>,
    pub noalias_fact: FactId,
    pub minimum_trip: u32,
    pub predicted_cost: KirCostEstimate,
}

/// Candidate discovery results, including source-derived rejection reasons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasmInvariantLoadDiscovery {
    pub candidates: Vec<WasmInvariantLoadCandidate>,
    pub fallbacks: Vec<WasmInvariantLoadFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInvariantLoadFallback {
    pub function: FunctionId,
    pub loop_id: Option<LoopId>,
    pub reason: String,
}

#[must_use]
pub fn discover_wasm_invariant_load_candidates(
    state: &crate::KirVerifiedProgramState,
) -> WasmInvariantLoadDiscovery {
    let module = state.module();
    if module.config.consumer != crate::KirConsumer::WebAssembly
        || module.profile.wasm_features() != Some(crate::KirWasmFeatures::Baseline)
        || module.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || module.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || module.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
    {
        return WasmInvariantLoadDiscovery::default();
    }

    let mut result = WasmInvariantLoadDiscovery::default();
    for function in &module.functions {
        let loops = analyze_canonical_loops_for_discovery(function);
        for descriptor in loops.loops.iter().filter(|loop_| loop_.innermost) {
            match discover_one(state, function, descriptor) {
                Ok(Some(candidate)) => result.candidates.push(candidate),
                Ok(None) => {}
                Err(reason) => result.fallbacks.push(WasmInvariantLoadFallback {
                    function: function.id,
                    loop_id: Some(descriptor.id),
                    reason,
                }),
            }
        }
    }
    result
        .candidates
        .sort_by(|left, right| left.key.cmp(&right.key));
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.loop_id, left.reason.as_str()).cmp(&(
            right.function,
            right.loop_id,
            right.reason.as_str(),
        ))
    });
    result
}

#[derive(Debug, Clone, Copy)]
struct Shape<'a> {
    preheader: &'a crate::KirBlock,
    header: &'a crate::KirBlock,
    body: &'a crate::KirBlock,
    exit: &'a crate::KirBlock,
    incoming: &'a crate::KirEdge,
    then_edge: &'a crate::KirEdge,
    backedge: &'a crate::KirEdge,
}

fn discover_one(
    state: &crate::KirVerifiedProgramState,
    function: &crate::KirFunction,
    descriptor: &crate::CanonicalLoopDescriptor,
) -> Result<Option<WasmInvariantLoadCandidate>, String> {
    let Some(shape) = simple_shape(function, descriptor) else {
        return Ok(None);
    };
    let induction = descriptor
        .induction
        .as_ref()
        .ok_or_else(|| "loop has no canonical induction".to_string())?;
    if induction.start != 0.into()
        || induction.step != 1.into()
        || induction.comparison != crate::MirCompareOp::Lt
        || value_type(function, induction.value)
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
    {
        return Ok(None);
    }
    let Some(induction_parameter) = shape
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == induction.value)
    else {
        return Ok(None);
    };
    if shape
        .header
        .params
        .iter()
        .enumerate()
        .any(|(index, parameter)| {
            if index == induction_parameter {
                return false;
            }
            let Some(backedge_value) = shape.backedge.args.get(index) else {
                return true;
            };
            *backedge_value != parameter.value
                && !shape.body.params.get(index).is_some_and(|body_parameter| {
                    *backedge_value == body_parameter.value
                        && shape.then_edge.args.get(index) == Some(&parameter.value)
                })
        })
    {
        return Ok(None);
    }
    let Some(bound) = loop_bound(shape.header, induction.value) else {
        return Ok(None);
    };
    let Some(induction_update) = unit_induction_update(function, shape, induction.value) else {
        return Ok(None);
    };
    let entry_bound = shape
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == bound)
        .and_then(|index| shape.incoming.args.get(index).copied())
        .unwrap_or(bound);
    if !checked_loop_bound_is_invariant(function, shape.preheader.id, bound, entry_bound) {
        return Ok(None);
    }

    let loads = shape
        .body
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .collect::<Vec<_>>();
    let stores = shape
        .body
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
        .collect::<Vec<_>>();
    let [store] = stores.as_slice() else {
        return Ok(None);
    };
    let (output_slice, output_index, output_region) = match &store.kind {
        KirInstructionKind::Store { place, .. } => match place.as_ref() {
            crate::KirPlace::SliceIndex {
                slice,
                index,
                type_node,
                region,
            } if *type_node == MirType::Primitive(MirPrimitiveTypeName::F64) => {
                (*slice, *index, *region)
            }
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    if !value_depends_on_induction(function, shape, induction.value, output_index) {
        return Ok(None);
    }
    let Some(output_range_dag) =
        output_range_dag(function, shape, induction.value, output_index, store.id)
    else {
        return Ok(None);
    };
    let Some(output_root) = stable_invariant_descriptor_root(function, output_slice) else {
        return Ok(None);
    };
    let Some(output_partition) = region_partition(function, output_region) else {
        return Ok(None);
    };

    let mut accepted_load = None;
    for load in loads {
        let KirInstructionKind::Load { place } = &load.kind else {
            continue;
        };
        let crate::KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } = place.as_ref()
        else {
            continue;
        };
        if *type_node != MirType::Primitive(MirPrimitiveTypeName::F64)
            || *slice == output_slice
            || load.results.len() != 1
        {
            continue;
        }
        let Some(input_root) = stable_invariant_descriptor_root(function, *slice) else {
            continue;
        };
        let Some(input_partition) = region_partition(function, *region) else {
            continue;
        };
        if !load_is_used_by_store(function, shape.body, load.results[0].value, store.id) {
            continue;
        }
        let Some((index_dag, elidable_index_dag)) =
            invariant_index_dag(function, shape, induction.value, *index, load.id)
        else {
            continue;
        };
        let Some(noalias_fact) =
            available_noalias_fact(state, function, shape.preheader.id, input_root, output_root)
        else {
            continue;
        };
        accepted_load = Some((
            load.id,
            load.results[0].value,
            *index,
            input_root,
            *region,
            input_partition,
            index_dag,
            elidable_index_dag,
            noalias_fact,
        ));
        break;
    }
    let Some((
        load,
        load_value,
        index,
        input_slice,
        input_region,
        input_partition,
        index_dag,
        elidable_index_dag,
        noalias_fact,
    )) = accepted_load
    else {
        return Ok(None);
    };
    if !matmul_scalar_body(function, shape, load, store.id, output_root, input_slice) {
        return Ok(None);
    }
    let Some((predicted_cost, minimum_trip)) = profitability(ProfitabilityInput {
        profile: &state.module().profile,
        body: shape.body,
        function,
        load,
        elidable_index_dag: &elidable_index_dag,
        index_dag: &index_dag,
        output_range_dag: &output_range_dag,
    }) else {
        return Ok(None);
    };
    Ok(Some(WasmInvariantLoadCandidate {
        key: CandidateKey::LoopFrontier {
            function: function.id,
            loop_id: descriptor.id,
            kind: LoopCandidateKind::WasmInvariantLoad,
            variant: LoopCandidateVariant::Scalar,
            vf: 1,
            uf: BASELINE_GUARDED_UNROLL_FACTOR as u8,
        },
        function: function.id,
        loop_id: descriptor.id,
        preheader: shape.preheader.id,
        header: shape.header.id,
        body: shape.body.id,
        exit: shape.exit.id,
        induction: induction.value,
        bound,
        input_slice,
        input_region,
        input_partition,
        output_slice: output_root,
        output_index,
        output_range_dag,
        output_region,
        output_partition,
        load,
        load_value,
        induction_update,
        index,
        index_dag,
        elidable_index_dag,
        noalias_fact,
        minimum_trip,
        predicted_cost,
    }))
}

fn simple_shape<'a>(
    function: &'a crate::KirFunction,
    descriptor: &crate::CanonicalLoopDescriptor,
) -> Option<Shape<'a>> {
    if !descriptor.innermost
        || !descriptor.lcssa
        || !descriptor.dedicated_exits
        || descriptor.blocks.len() != 2
        || descriptor.exits.len() != 1
    {
        return None;
    }
    let preheader = block(function, descriptor.preheader?)?;
    let header = block(function, descriptor.header)?;
    let body = block(function, descriptor.latch?)?;
    let exit = block(function, descriptor.exits[0])?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return None;
    };
    let KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &header.terminator
    else {
        return None;
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return None;
    };
    if incoming.target != header.id
        || then_edge.target != body.id
        || else_edge.target != exit.id
        || backedge.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || then_edge.args.len() != body.params.len()
        || then_edge.memory_args.len() != body.memory_params.len()
        || backedge.args.len() != header.params.len()
        || backedge.memory_args.len() != header.memory_params.len()
        || else_edge.args.len() != exit.params.len()
        || else_edge.memory_args.len() != exit.memory_params.len()
    {
        return None;
    }
    Some(Shape {
        preheader,
        header,
        body,
        exit,
        incoming,
        then_edge,
        backedge,
    })
}

fn unit_induction_update(
    function: &crate::KirFunction,
    shape: Shape<'_>,
    induction: ValueId,
) -> Option<InstructionId> {
    let index = shape
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == induction)?;
    let body_induction = shape.body.params.get(index)?.value;
    if shape.then_edge.args.get(index) != Some(&induction) {
        return None;
    }
    let next = *shape.backedge.args.get(index)?;
    let (update_block, update) = definition(function, next)?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &update.kind
    else {
        return None;
    };
    if update_block != shape.body.id
        || *left != body_induction
        || update.results.len() != 1
        || update.results.as_slice().first().map(|result| result.value) != Some(next)
        || update.results[0].type_node.as_scalar()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || update.memory.is_some()
        || update.effect.is_some()
        || function.blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|user| user.id != update.id && uses_value(user, next))
                || (block.id != shape.body.id && terminator_uses_value(&block.terminator, next))
        })
        || shape
            .backedge
            .args
            .iter()
            .enumerate()
            .any(|(argument_index, argument)| argument_index != index && *argument == next)
    {
        return None;
    }
    let (_, step) = definition(function, *right)?;
    (matches!(&step.kind, KirInstructionKind::ConstInt { value } if value == "1")
        && step.memory.is_none()
        && step.effect.is_none()
        && step.results.as_slice().first().is_some_and(|result| {
            result.value == *right
                && result.type_node.as_scalar()
                    == Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        }))
    .then_some(update.id)
}

fn loop_bound(header: &crate::KirBlock, induction: ValueId) -> Option<ValueId> {
    if header.instructions.len() != 1 {
        return None;
    }
    let KirTerminator::Branch { condition, .. } = &header.terminator else {
        return None;
    };
    let compare = header.instructions.iter().find(|instruction| {
        instruction
            .results
            .first()
            .is_some_and(|r| r.value == *condition)
    })?;
    let KirInstructionKind::Compare {
        op: crate::MirCompareOp::Lt,
        left,
        right,
    } = &compare.kind
    else {
        return None;
    };
    (*left == induction).then_some(*right)
}

fn invariant_index_dag(
    function: &crate::KirFunction,
    shape: Shape<'_>,
    induction: ValueId,
    index: ValueId,
    load: InstructionId,
) -> Option<(Vec<InstructionId>, Vec<InstructionId>)> {
    struct Walker<'a> {
        function: &'a crate::KirFunction,
        shape: Shape<'a>,
        body_param_indices: BTreeMap<ValueId, usize>,
        header_param_indices: BTreeMap<ValueId, usize>,
        induction: ValueId,
        dominators: crate::KirDominators,
        visiting: BTreeSet<ValueId>,
        visited: BTreeSet<ValueId>,
        ordered: Vec<InstructionId>,
    }

    impl Walker<'_> {
        fn visit(&mut self, value: ValueId) -> Option<()> {
            if self.visited.contains(&value) {
                return Some(());
            }
            if let Some(body_index) = self.body_param_indices.get(&value).copied() {
                let source_header_value = *self.shape.then_edge.args.get(body_index)?;
                if source_header_value == self.induction {
                    return None;
                }
                let header_index = *self.header_param_indices.get(&source_header_value)?;
                // A source value used in this loop is invariant only when every
                // backedge forwards the corresponding body parameter unchanged.
                if self.shape.backedge.args.get(header_index) != Some(&value) {
                    return None;
                }
                self.visited.insert(value);
                return Some(());
            }
            let Some((definition_block, instruction)) = definition(self.function, value) else {
                if self
                    .function
                    .params
                    .iter()
                    .any(|parameter| parameter.value == value)
                    || self.function.blocks.iter().any(|block| {
                        block.id != self.shape.header.id
                            && block.id != self.shape.body.id
                            && block
                                .params
                                .iter()
                                .any(|parameter| parameter.value == value)
                            && self.dominators.dominates(block.id, self.shape.preheader.id)
                    })
                {
                    self.visited.insert(value);
                    return Some(());
                }
                return None;
            };
            if definition_block != self.shape.body.id {
                if self
                    .dominators
                    .dominates(definition_block, self.shape.preheader.id)
                {
                    self.visited.insert(value);
                    return Some(());
                }
                return None;
            }
            if !self.visiting.insert(value) || instruction.results.len() != 1 {
                return None;
            }
            if instruction.results[0].value != value
                || instruction.memory.is_some()
                || instruction.effect.is_some()
                || !matches!(
                    instruction.kind,
                    KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::Binary {
                            semantics: KirArithmeticSemantics::Modular,
                            ..
                        }
                )
                || instruction.results[0].type_node.as_scalar()
                    != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
            {
                return None;
            }
            for operand in integer_operands(&instruction.kind)? {
                self.visit(operand)?;
            }
            self.visiting.remove(&value);
            self.visited.insert(value);
            self.ordered.push(instruction.id);
            if self.ordered.len() > MAX_INDEX_DAG_INSTRUCTIONS {
                return None;
            }
            Some(())
        }
    }

    let mut walker = Walker {
        function,
        shape,
        body_param_indices: shape
            .body
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| (parameter.value, index))
            .collect(),
        header_param_indices: shape
            .header
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| (parameter.value, index))
            .collect(),
        induction,
        dominators: crate::compute_kir_dominators(function),
        visiting: BTreeSet::new(),
        visited: BTreeSet::new(),
        ordered: Vec::new(),
    };
    walker.visit(index)?;
    let ordered = walker.ordered;

    let dag_ids = ordered.iter().copied().collect::<BTreeSet<_>>();
    let mut elidable = dag_ids.clone();
    loop {
        let mut changed = false;
        for id in dag_ids.iter().copied() {
            if !elidable.contains(&id) {
                continue;
            }
            let instruction = instruction(function, id)?;
            let result = instruction.results.first()?.value;
            let has_external_use = shape
                .body
                .instructions
                .iter()
                .filter(|user| user.id != id)
                .any(|user| {
                    uses_value(user, result) && !elidable.contains(&user.id) && user.id != load
                });
            let has_external_edge_use = terminator_uses_value(&shape.body.terminator, result);
            if has_external_use || has_external_edge_use {
                elidable.remove(&id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Some((ordered, elidable.into_iter().collect()))
}

/// Proves a modular U32 output subscript is `base + iv` with coefficient one.
/// The returned source-ordered DAG is cloned into the preheader with the body
/// induction parameter mapped to zero; its result is then a range-checkable
/// base for all writes in the loop.
fn output_range_dag(
    function: &crate::KirFunction,
    shape: Shape<'_>,
    induction: ValueId,
    output_index: ValueId,
    store: InstructionId,
) -> Option<Vec<InstructionId>> {
    let body_induction = shape
        .body
        .params
        .iter()
        .zip(&shape.then_edge.args)
        .find_map(|(parameter, argument)| (*argument == induction).then_some(parameter.value))?;
    struct Walker<'a> {
        function: &'a crate::KirFunction,
        shape: Shape<'a>,
        induction: ValueId,
        body_induction: ValueId,
        store: InstructionId,
        dominators: crate::KirDominators,
        dag: BTreeSet<InstructionId>,
        visiting: BTreeSet<ValueId>,
    }

    impl Walker<'_> {
        fn coefficient(&mut self, value: ValueId) -> Option<i32> {
            if value == self.body_induction {
                return Some(1);
            }
            if self
                .function
                .params
                .iter()
                .any(|parameter| parameter.value == value)
            {
                return Some(0);
            }
            if let Some((invariant_dag, _)) =
                invariant_index_dag(self.function, self.shape, self.induction, value, self.store)
            {
                self.dag.extend(invariant_dag);
                return Some(0);
            }
            if let Some((definition_block, _)) = definition(self.function, value)
                && definition_block != self.shape.body.id
                && self
                    .dominators
                    .dominates(definition_block, self.shape.preheader.id)
            {
                return Some(0);
            }
            if !self.visiting.insert(value) {
                return None;
            }
            let (definition_block, instruction) = definition(self.function, value)?;
            if definition_block != self.shape.body.id
                || instruction.results.len() != 1
                || instruction.results[0].value != value
                || instruction.memory.is_some()
                || instruction.effect.is_some()
                || instruction.results[0].type_node.as_scalar()
                    != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
            {
                return None;
            }
            let instruction_kind = instruction.kind.clone();
            let instruction_id = instruction.id;
            let coefficient = match &instruction_kind {
                KirInstructionKind::Copy { value: operand } => self.coefficient(*operand)?,
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add | MirBinaryOp::Sub,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } => {
                    let left_coefficient = self.coefficient(*left)?;
                    let right_coefficient = self.coefficient(*right)?;
                    if matches!(
                        &instruction_kind,
                        KirInstructionKind::Binary {
                            op: MirBinaryOp::Sub,
                            ..
                        }
                    ) {
                        left_coefficient.checked_sub(right_coefficient)?
                    } else {
                        left_coefficient.checked_add(right_coefficient)?
                    }
                }
                _ => return None,
            };
            self.visiting.remove(&value);
            self.dag.insert(instruction_id);
            Some(coefficient)
        }
    }

    let mut walker = Walker {
        function,
        shape,
        induction,
        body_induction,
        store,
        dominators: crate::compute_kir_dominators(function),
        dag: BTreeSet::new(),
        visiting: BTreeSet::new(),
    };
    if walker.coefficient(output_index)? != 1 || walker.dag.len() > MAX_INDEX_DAG_INSTRUCTIONS {
        return None;
    }
    Some(
        shape
            .body
            .instructions
            .iter()
            .filter(|instruction| walker.dag.contains(&instruction.id))
            .map(|instruction| instruction.id)
            .collect(),
    )
}

fn matmul_scalar_body(
    function: &crate::KirFunction,
    shape: Shape<'_>,
    invariant_load: InstructionId,
    output_store: InstructionId,
    output_slice: ValueId,
    input_slice: ValueId,
) -> bool {
    let loads = shape
        .body
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .collect::<Vec<_>>();
    if loads.len() != 3
        || !loads.iter().any(|load| load.id == invariant_load)
        || shape
            .body
            .instructions
            .iter()
            .filter(|i| matches!(i.kind, KirInstructionKind::Store { .. }))
            .count()
            != 1
        || !shape.body.instructions.iter().any(|i| i.id == output_store)
    {
        return false;
    }
    let invariant_position = shape
        .body
        .instructions
        .iter()
        .position(|i| i.id == invariant_load);
    let store_position = shape
        .body
        .instructions
        .iter()
        .position(|i| i.id == output_store);
    if invariant_position
        .zip(store_position)
        .is_none_or(|(load, store)| load >= store)
    {
        return false;
    }
    if shape.body.instructions.iter().any(|instruction| {
        matches!(
            instruction.kind,
            KirInstructionKind::Call { .. }
                | KirInstructionKind::RuntimeCall { .. }
                | KirInstructionKind::Guard { .. }
                | KirInstructionKind::CheckCondition { .. }
                | KirInstructionKind::VersionPredicate { .. }
                | KirInstructionKind::Subslice { .. }
                | KirInstructionKind::MakeSlice { .. }
                | KirInstructionKind::Address { .. }
        )
    }) {
        return false;
    }
    let Some(output_root) = stable_invariant_descriptor_root(function, output_slice) else {
        return false;
    };
    let Some(input_root) = stable_invariant_descriptor_root(function, input_slice) else {
        return false;
    };
    // Any loop write must be the unique output store. The noalias fact then
    // proves that hoisting this input read cannot observe changed memory.
    shape.body.instructions.iter().all(|instruction| {
        if instruction.id == output_store {
            let KirInstructionKind::Store { place, .. } = &instruction.kind else {
                return false;
            };
            return matches!(place.as_ref(), crate::KirPlace::SliceIndex { slice, .. }
                if stable_invariant_descriptor_root(function, *slice) == Some(output_root));
        }
        instruction
            .memory
            .as_ref()
            .is_none_or(|memory| memory.output.is_none())
    }) && input_root != output_root
}

fn available_noalias_fact(
    state: &crate::KirVerifiedProgramState,
    function: &crate::KirFunction,
    preheader: BlockId,
    input: ValueId,
    output: ValueId,
) -> Option<FactId> {
    let dominators = crate::compute_kir_dominators(function);
    state
        .contract_facts()?
        .facts()
        .facts()
        .iter()
        .find_map(|fact| {
            let scope_matches = match &fact.scope {
                crate::FactScope::FunctionEntry(owner) => *owner == function.id,
                crate::FactScope::Block {
                    function: owner,
                    block,
                } => *owner == function.id && dominators.dominates(*block, preheader),
                crate::FactScope::CalleeInstance { .. } | crate::FactScope::InlineClone { .. } => {
                    false
                }
            };
            (scope_matches
                && fact.generation == state.evidence_generation()
                && matches!(fact.origin, crate::FactOrigin::TrustedContract { .. })
                && fact.derivation == crate::FactDerivation::TrustedContractLeaf
                && matches!(
                    fact.predicate,
                    crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias {
                        left,
                        right,
                    }) if stable_invariant_descriptor_root(function, left) == Some(input)
                        && stable_invariant_descriptor_root(function, right) == Some(output)
                        || stable_invariant_descriptor_root(function, right) == Some(input)
                        && stable_invariant_descriptor_root(function, left) == Some(output)
                ))
            .then_some(fact.id)
        })
}

struct ProfitabilityInput<'a> {
    profile: &'a crate::KirTargetProfile,
    body: &'a crate::KirBlock,
    function: &'a crate::KirFunction,
    load: InstructionId,
    elidable_index_dag: &'a [InstructionId],
    index_dag: &'a [InstructionId],
    output_range_dag: &'a [InstructionId],
}

fn profitability(input: ProfitabilityInput<'_>) -> Option<(KirCostEstimate, u32)> {
    let body_cost = input
        .body
        .instructions
        .iter()
        .map(|instruction| instruction_cost(input.profile, instruction, input.function))
        .try_fold(0_u32, |sum, cost| sum.checked_add(cost?))?;
    let load_cost = instruction_cost(
        input.profile,
        instruction(input.function, input.load)?,
        input.function,
    )?;
    let per_trip_saving = input
        .elidable_index_dag
        .iter()
        .try_fold(load_cost, |sum, id| {
            sum.checked_add(instruction_cost(
                input.profile,
                instruction(input.function, *id)?,
                input.function,
            )?)
        })?;
    let index_cost = input.index_dag.iter().try_fold(0_u32, |sum, id| {
        sum.checked_add(instruction_cost(
            input.profile,
            instruction(input.function, *id)?,
            input.function,
        )?)
    })?;
    let output_cost = input.output_range_dag.iter().try_fold(0_u32, |sum, id| {
        sum.checked_add(instruction_cost(
            input.profile,
            instruction(input.function, *id)?,
            input.function,
        )?)
    })?;
    let compare_cost = operation_cost(
        input.profile,
        KirProfileOperation::Compare,
        KirLaneType::U32,
        KirCostSemantics::NotApplicable,
    )?;
    let bulk_limit_cost = operation_cost(
        input.profile,
        KirProfileOperation::Subtract,
        KirLaneType::U32,
        KirCostSemantics::Modular,
    )?;
    if body_cost == 0 || per_trip_saving == 0 || per_trip_saving >= body_cost {
        return None;
    }
    // The Wasm32 range predicate expands to several i64 operations and three
    // checks. Charge that predicate and the fast-entry bulk-limit arithmetic
    // once per loop. This model also charges scalar and strip-mined loop tests,
    // so a candidate cannot win merely by changing its audit factor.
    let predicate_cost = BASELINE_RANGE_PREDICATE_COST
        .checked_mul(3)?
        .checked_add(1)?;
    let setup_cost = index_cost
        .checked_add(output_cost)?
        .checked_add(load_cost)?
        .checked_add(bulk_limit_cost)?
        .checked_add(predicate_cost)?;
    let loop_control_cost = compare_cost.checked_add(BASELINE_LOOP_BRANCH_COST)?;
    for trip in BASELINE_GUARDED_UNROLL_FACTOR..=MAX_COST_SEARCH_TRIP {
        let chunks = trip / BASELINE_GUARDED_UNROLL_FACTOR;
        let tail = trip % BASELINE_GUARDED_UNROLL_FACTOR;
        let bulk_trip_count = chunks.checked_mul(BASELINE_GUARDED_UNROLL_FACTOR)?;
        let scalar = body_cost
            .checked_mul(trip)?
            .checked_add(loop_control_cost.checked_mul(trip)?)?
            .checked_add(compare_cost)?;
        let transformed_body = body_cost
            .checked_sub(per_trip_saving)?
            .checked_mul(bulk_trip_count)?
            .checked_add(body_cost.checked_mul(tail)?)?;
        let transformed_control = loop_control_cost
            .checked_mul(chunks.checked_add(tail)?)?
            .checked_add(compare_cost.checked_mul(2)?)?
            .checked_add(BASELINE_LOOP_BRANCH_COST)?;
        let total = transformed_body
            .checked_add(setup_cost)?
            .checked_add(transformed_control)?;
        if u64::from(total).saturating_mul(100)
            <= u64::from(scalar).saturating_mul(u64::from(100 - MINIMUM_COST_REDUCTION_PERCENT))
        {
            return Some((
                KirCostEstimate::new(scalar, transformed_body, setup_cost, transformed_control),
                trip,
            ));
        }
    }
    None
}

fn instruction_cost(
    profile: &crate::KirTargetProfile,
    instruction: &KirInstruction,
    function: &crate::KirFunction,
) -> Option<u32> {
    use KirProfileOperation as Op;
    let (operation, lane, semantics, alignment) = match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. } => return Some(0),
        KirInstructionKind::Load { place } => (
            Op::Load,
            lane_for_type(place_type(place)?)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::Bytes(8),
        ),
        KirInstructionKind::Store { place, .. } => (
            Op::Store,
            lane_for_type(place_type(place)?)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::Bytes(8),
        ),
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => Op::Add,
                MirBinaryOp::Sub => Op::Subtract,
                MirBinaryOp::Mul => Op::Multiply,
                MirBinaryOp::Div => Op::Divide,
                MirBinaryOp::Mod => Op::Remainder,
            },
            lane_for_result(instruction, function)?,
            match semantics {
                KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                KirArithmeticSemantics::Checked => KirCostSemantics::Checked,
                KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
            },
            KirAlignmentClass::NotApplicable,
        ),
        KirInstructionKind::Compare { .. } => (
            Op::Compare,
            lane_for_result(instruction, function)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::NotApplicable,
        ),
        KirInstructionKind::SliceLen { .. } | KirInstructionKind::SliceData { .. } => {
            return Some(1);
        }
        _ => return None,
    };
    let key = KirCostKey {
        operation,
        lane,
        lanes: 1,
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

fn operation_cost(
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
        _ => None,
    }
}

fn lane_for_result(
    instruction: &KirInstruction,
    function: &crate::KirFunction,
) -> Option<KirLaneType> {
    let [result] = instruction.results.as_slice() else {
        return None;
    };
    lane_for_type(value_type(function, result.value)?)
}

fn lane_for_type(type_node: &MirType) -> Option<KirLaneType> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn place_type(place: &crate::KirPlace) -> Option<&MirType> {
    Some(match place {
        crate::KirPlace::SliceIndex { type_node, .. }
        | crate::KirPlace::Index { type_node, .. }
        | crate::KirPlace::Value { type_node, .. }
        | crate::KirPlace::Deref { type_node, .. } => type_node,
        crate::KirPlace::Field { .. } => return None,
    })
}

fn value_type(function: &crate::KirFunction, value: ValueId) -> Option<&MirType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| &param.type_node)
        .or_else(|| {
            function.blocks.iter().find_map(|block| {
                block
                    .params
                    .iter()
                    .find(|param| param.value == value)
                    .and_then(|param| param.type_node.as_scalar())
                    .or_else(|| {
                        block.instructions.iter().find_map(|instruction| {
                            instruction
                                .results
                                .iter()
                                .find(|result| result.value == value)
                                .and_then(|result| result.type_node.as_scalar())
                        })
                    })
            })
        })
}

fn load_is_used_by_store(
    function: &crate::KirFunction,
    body: &crate::KirBlock,
    value: ValueId,
    store: InstructionId,
) -> bool {
    fn reaches_store(
        function: &crate::KirFunction,
        body: &crate::KirBlock,
        value: ValueId,
        store: InstructionId,
        visited: &mut BTreeSet<ValueId>,
    ) -> bool {
        if !visited.insert(value) {
            return false;
        }
        let mut reaches = false;
        for user in &body.instructions {
            if !uses_value(user, value) {
                continue;
            }
            if user.id == store {
                reaches = true;
            } else if user.results.len() == 1
                && matches!(
                    user.kind,
                    KirInstructionKind::Binary { .. }
                        | KirInstructionKind::Unary { .. }
                        | KirInstructionKind::Copy { .. }
                )
            {
                reaches |= reaches_store(function, body, user.results[0].value, store, visited);
            } else {
                return false;
            }
        }
        if function.blocks.iter().any(|block| {
            block.id != body.id
                && block
                    .instructions
                    .iter()
                    .any(|user| uses_value(user, value))
        }) {
            return false;
        }
        reaches
    }
    reaches_store(function, body, value, store, &mut BTreeSet::new())
}

fn value_depends_on_induction(
    function: &crate::KirFunction,
    shape: Shape<'_>,
    induction: ValueId,
    value: ValueId,
) -> bool {
    fn visit(
        function: &crate::KirFunction,
        shape: Shape<'_>,
        induction: ValueId,
        value: ValueId,
        visited: &mut BTreeSet<ValueId>,
    ) -> bool {
        if !visited.insert(value) {
            return false;
        }
        if let Some((index, _)) = shape
            .body
            .params
            .iter()
            .enumerate()
            .find(|(_, parameter)| parameter.value == value)
        {
            return shape.then_edge.args.get(index) == Some(&induction);
        }
        let Some((definition_block, instruction)) = definition(function, value) else {
            return false;
        };
        if definition_block != shape.body.id {
            return false;
        }
        integer_operands(&instruction.kind).is_some_and(|operands| {
            operands
                .into_iter()
                .any(|operand| visit(function, shape, induction, operand, visited))
        })
    }
    visit(function, shape, induction, value, &mut BTreeSet::new())
}

fn uses_value(instruction: &KirInstruction, value: ValueId) -> bool {
    match &instruction.kind {
        KirInstructionKind::Copy { value: operand }
        | KirInstructionKind::Cast { value: operand, .. }
        | KirInstructionKind::VectorSplat {
            scalar: operand, ..
        } => *operand == value,
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. }
        | KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => {
            *left == value || *right == value
        }
        KirInstructionKind::Unary { operand, .. }
        | KirInstructionKind::VectorUnary { operand, .. } => *operand == value,
        KirInstructionKind::Load { place } | KirInstructionKind::Address { place } => {
            place_uses_value(place, value)
        }
        KirInstructionKind::Store {
            place,
            value: stored,
        } => *stored == value || place_uses_value(place, value),
        KirInstructionKind::SliceLen { slice }
        | KirInstructionKind::SliceData { slice }
        | KirInstructionKind::VectorLoad {
            access: crate::KirVectorMemoryAccess { slice, .. },
            ..
        } => *slice == value,
        KirInstructionKind::Subslice { slice, start, end } => {
            *slice == value || *start == value || *end == value
        }
        KirInstructionKind::MakeSlice { data, len } => *data == value || *len == value,
        KirInstructionKind::Guard { condition, .. } => *condition == value,
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.contains(&value),
        KirInstructionKind::VersionPredicate { predicate } => {
            predicate.conjuncts.iter().any(|c| match c {
                crate::KirVersionPredicateConjunct::TripThreshold { value: v, .. } => *v == value,
                crate::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                    left,
                    left_count,
                    right,
                    right_count,
                    ..
                } => [left, left_count, right, right_count].contains(&&value),
                crate::KirVersionPredicateConjunct::WasmSliceRange {
                    slice,
                    start,
                    count,
                    ..
                } => [slice, start, count].contains(&&value),
            })
        }
        KirInstructionKind::VectorStore {
            access,
            value: stored,
            ..
        } => {
            *stored == value
                || access.slice == value
                || access.start == value
                || access.end == value
        }
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => [mask, when_true, when_false].contains(&&value),
        KirInstructionKind::VectorCast { value: operand, .. }
        | KirInstructionKind::VectorExtract {
            vector: operand, ..
        } => *operand == value,
        KirInstructionKind::VectorInsert { vector, scalar, .. } => {
            *vector == value || *scalar == value
        }
        KirInstructionKind::VectorReduce { vector, .. } => *vector == value,
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => false,
    }
}

fn place_uses_value(place: &crate::KirPlace, value: ValueId) -> bool {
    match place {
        crate::KirPlace::SliceIndex { slice, index, .. } => *slice == value || *index == value,
        crate::KirPlace::Index { base, index, .. } => {
            place_uses_value(base, value) || *index == value
        }
        crate::KirPlace::Field { base, .. } => place_uses_value(base, value),
        crate::KirPlace::Value { value: actual, .. } => *actual == value,
        crate::KirPlace::Deref { pointer, .. } => *pointer == value,
    }
}

fn terminator_uses_value(terminator: &KirTerminator, value: ValueId) -> bool {
    match terminator {
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
        } => *returned == Some(value),
    }
}

fn integer_operands(kind: &KirInstructionKind) -> Option<Vec<ValueId>> {
    match kind {
        KirInstructionKind::ConstInt { .. } => Some(Vec::new()),
        KirInstructionKind::Copy { value } => Some(vec![*value]),
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } => Some(vec![*left, *right]),
        _ => None,
    }
}

fn definition(function: &crate::KirFunction, value: ValueId) -> Option<(BlockId, &KirInstruction)> {
    function.blocks.iter().find_map(|block| {
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
    })
}

fn instruction(function: &crate::KirFunction, id: InstructionId) -> Option<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
}

fn block(function: &crate::KirFunction, id: BlockId) -> Option<&crate::KirBlock> {
    function.blocks.iter().find(|block| block.id == id)
}

fn region_partition(
    function: &crate::KirFunction,
    id: crate::MemoryRegionId,
) -> Option<crate::MemoryRegionId> {
    function
        .regions
        .iter()
        .find(|region| region.id == id)
        .map(|region| region.partition)
}
