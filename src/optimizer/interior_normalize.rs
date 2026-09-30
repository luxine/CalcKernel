//! Scalar unit-offset loop rebasing and modular address-origin normalization.
//! New predicates and invariant arithmetic execute after the original preheader,
//! including any boundary store. The original loop is retained as a fallback.
use crate::{
    BlockId, CandidateBudgetCharge, CanonicalLoopDescriptor, FunctionId, InstructionId,
    KirArithmeticSemantics, KirBlock, KirEdge, KirFunction, KirInstruction, KirInstructionKind,
    KirPlace, KirResult, KirTerminator, KirValueType, KirVerifiedProgramState, LoopId,
    MemoryVersionId, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType,
    TransactionCheckError, ValueId, analyze_canonical_loops, kir_function_units,
};
use std::collections::{BTreeMap, BTreeSet};

type Values = BTreeMap<ValueId, ValueId>;
type Memories = BTreeMap<MemoryVersionId, MemoryVersionId>;
const MAX_BODY: usize = 128;
const MAX_GROWTH: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteriorNormalizeCandidate {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedInteriorAddress {
    pub source: InstructionId,
    pub source_index: ValueId,
    pub origin: ValueId,
    pub index: ValueId,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteriorNormalizePlan {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
    pub preheader: BlockId,
    pub source_body: BlockId,
    pub pre_state_digest: String,
    /// Dispatch, invariant setup, zero-based header, body and scalar exit adapter.
    pub blocks: [BlockId; 5],
    pub one: ValueId,
    pub zero: ValueId,
    pub trip_count: ValueId,
    pub body_column: ValueId,
    pub exit_column: ValueId,
    pub addresses: Vec<NormalizedInteriorAddress>,
    pub instruction_mapping: Vec<(InstructionId, InstructionId)>,
    pub removed_instructions: Vec<InstructionId>,
    pub before_units: u32,
    pub after_units: u32,
}
#[derive(Debug, Clone)]
pub struct PreparedInteriorNormalize {
    pub trial: KirVerifiedProgramState,
    pub plan: InteriorNormalizePlan,
    pub charge: CandidateBudgetCharge,
}

#[derive(Debug, Clone)]
struct SourceShape {
    descriptor: CanonicalLoopDescriptor,
    body: BlockId,
    induction_index: usize,
    bound: ValueId,
    incoming: KirEdge,
    body_edge: KirEdge,
    exit: KirEdge,
    backedge: KirEdge,
    step: InstructionId,
    invariant_values: Values,
    column_values: BTreeSet<ValueId>,
    address_nodes: BTreeSet<InstructionId>,
    addresses: Vec<(InstructionId, ValueId)>,
}

fn scalar(primitive: MirPrimitiveTypeName) -> KirValueType {
    MirType::Primitive(primitive).into()
}
fn block(f: &KirFunction, id: BlockId) -> Option<&KirBlock> {
    f.blocks.iter().find(|b| b.id == id)
}
fn definition(f: &KirFunction, v: ValueId) -> Option<&KirInstruction> {
    f.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .find(|i| i.results.iter().any(|r| r.value == v))
}
fn value_type(f: &KirFunction, v: ValueId) -> Option<KirValueType> {
    f.params
        .iter()
        .find(|p| p.value == v)
        .map(|p| p.type_node.clone().into())
        .or_else(|| {
            f.blocks.iter().find_map(|b| {
                b.params
                    .iter()
                    .find(|p| p.value == v)
                    .map(|p| p.type_node.clone())
                    .or_else(|| {
                        b.instructions
                            .iter()
                            .flat_map(|i| &i.results)
                            .find(|r| r.value == v)
                            .map(|r| r.type_node.clone())
                    })
            })
        })
}
fn mapped(v: ValueId, m: &Values) -> ValueId {
    m.get(&v).copied().unwrap_or(v)
}

#[must_use]
pub fn discover_interior_normalize_candidates(
    state: &KirVerifiedProgramState,
) -> Vec<InteriorNormalizeCandidate> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly {
        return Vec::new();
    }
    state
        .module()
        .functions
        .iter()
        .flat_map(|f| {
            analyze_canonical_loops(f)
                .loops
                .into_iter()
                .filter_map(|d| {
                    propose_shape(f, &d).map(|_| InteriorNormalizeCandidate {
                        function: f.id,
                        loop_id: d.id,
                        header: d.header,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn propose_shape(f: &KirFunction, d: &CanonicalLoopDescriptor) -> Option<SourceShape> {
    if !d.innermost || !d.lcssa || d.blocks.len() != 2 || d.exits.len() != 1 {
        return None;
    }
    let h = block(f, d.header)?;
    let p = block(f, d.preheader?)?;
    let KirTerminator::Jump { edge: incoming } = &p.terminator else {
        return None;
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &h.terminator
    else {
        return None;
    };
    if h.instructions.len() != 1
        || incoming.target != h.id
        || !d.blocks.contains(&then_edge.target)
        || d.blocks.contains(&else_edge.target)
    {
        return None;
    }
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: column,
        right: bound,
    } = h.instructions[0].kind
    else {
        return None;
    };
    if h.instructions[0].results.len() != 1
        || h.instructions[0].results[0].value != *condition
        || h.instructions[0].memory.is_some()
        || h.instructions[0].effect.is_some()
        || value_type(f, column) != Some(scalar(MirPrimitiveTypeName::U32))
        || value_type(f, bound) != Some(scalar(MirPrimitiveTypeName::U32))
    {
        return None;
    }
    let induction_index = h.params.iter().position(|p| p.value == column)?;
    if bound == column
        || proposer_constant(
            f,
            *incoming.args.get(induction_index)?,
            &mut BTreeSet::new(),
        )? != 1
    {
        return None;
    }
    let b = block(f, then_edge.target)?;
    let KirTerminator::Jump { edge: backedge } = &b.terminator else {
        return None;
    };
    if backedge.target != h.id
        || incoming.args.len() != h.params.len()
        || backedge.args.len() != h.params.len()
        || then_edge.args.len() != b.params.len()
        || b.instructions.len() > MAX_BODY
    {
        return None;
    }
    let mut roots = b
        .params
        .iter()
        .zip(&then_edge.args)
        .map(|(p, v)| (p.value, *v))
        .collect::<Values>();
    for i in &b.instructions {
        if let KirInstructionKind::Copy { value } = i.kind {
            for r in &i.results {
                roots.insert(r.value, mapped(value, &roots));
            }
        }
    }
    let _body_column = b
        .params
        .iter()
        .zip(&then_edge.args)
        .find_map(|(p, v)| (*v == column).then_some(p.value))?;
    for (index, (arg, param)) in backedge.args.iter().zip(&h.params).enumerate() {
        if index != induction_index && mapped(*arg, &roots) != param.value {
            return None;
        }
    }
    let step = definition(f, backedge.args[induction_index])?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = step.kind
    else {
        return None;
    };
    let constant_one = |v| matches!(definition(f,v).map(|i|&i.kind),Some(KirInstructionKind::ConstInt{value}) if value.parse::<u32>()==Ok(1));
    if !((mapped(left, &roots) == column && constant_one(right))
        || (mapped(right, &roots) == column && constant_one(left)))
        || step.effect.is_some()
        || step.memory.is_some()
    {
        return None;
    }
    let pre_values = h
        .params
        .iter()
        .zip(&incoming.args)
        .map(|(p, v)| (p.value, *v))
        .collect::<Values>();
    let column_values = roots
        .iter()
        .filter_map(|(value, root)| (*root == column).then_some(*value))
        .chain([column])
        .collect::<BTreeSet<_>>();
    let mut invariant_values = pre_values.clone();
    invariant_values.remove(&column);
    for (value, root) in &roots {
        if *root != column
            && !b
                .instructions
                .iter()
                .any(|i| i.results.iter().any(|r| r.value == *root))
        {
            invariant_values.insert(*value, mapped(*root, &pre_values));
        }
    }
    let mut nodes = BTreeSet::new();
    let mut addresses = Vec::new();
    for i in &b.instructions {
        if !supported(i) {
            return None;
        }
        let place = match &i.kind {
            KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => place,
            _ => continue,
        };
        let KirPlace::SliceIndex { index, .. } = place.as_ref() else {
            return None;
        };
        let mut visiting = BTreeSet::new();
        if address_coefficient(
            f,
            b,
            *index,
            &column_values,
            &invariant_values,
            &mut nodes,
            &mut visiting,
        )? != 1
        {
            return None;
        }
        addresses.push((i.id, *index));
    }
    if addresses.len() < 3
        || addresses
            .iter()
            .map(|(_, v)| *v)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        || !b
            .instructions
            .iter()
            .any(|i| matches!(i.kind, KirInstructionKind::Store { .. }))
    {
        return None;
    }
    let estimate = 12
        + 5 * (1 + h.params.len() + h.memory_params.len())
        + b.instructions.len()
        + nodes.len()
        + addresses.len();
    if estimate > MAX_GROWTH as usize {
        return None;
    }
    Some(SourceShape {
        descriptor: d.clone(),
        body: b.id,
        induction_index,
        bound,
        incoming: incoming.clone(),
        body_edge: then_edge.clone(),
        exit: else_edge.clone(),
        backedge: backedge.clone(),
        step: step.id,
        invariant_values,
        column_values,
        address_nodes: nodes,
        addresses,
    })
}

fn address_coefficient(
    f: &KirFunction,
    b: &KirBlock,
    v: ValueId,
    column: &BTreeSet<ValueId>,
    invariants: &Values,
    nodes: &mut BTreeSet<InstructionId>,
    visiting: &mut BTreeSet<ValueId>,
) -> Option<u32> {
    if value_type(f, v) != Some(scalar(MirPrimitiveTypeName::U32)) {
        return None;
    }
    if column.contains(&v) {
        return Some(1);
    }
    if invariants.contains_key(&v) {
        return Some(0);
    }
    let Some(i) = b
        .instructions
        .iter()
        .find(|i| i.results.iter().any(|r| r.value == v))
    else {
        return Some(0);
    };
    if !visiting.insert(v)
        || visiting.len() > MAX_BODY
        || i.results.len() != 1
        || i.effect.is_some()
        || i.memory.is_some()
    {
        return None;
    }
    let coefficient = match i.kind {
        KirInstructionKind::ConstInt { .. } => 0,
        KirInstructionKind::Copy { value } => {
            address_coefficient(f, b, value, column, invariants, nodes, visiting)?
        }
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
            let a = address_coefficient(f, b, left, column, invariants, nodes, visiting)?;
            let c = address_coefficient(f, b, right, column, invariants, nodes, visiting)?;
            if op == MirBinaryOp::Add {
                a.wrapping_add(c)
            } else {
                a.wrapping_sub(c)
            }
        }
        _ => return None,
    };
    visiting.remove(&v);
    nodes.insert(i.id);
    Some(coefficient)
}

pub fn prepare_interior_normalize_trial(
    state: &KirVerifiedProgramState,
    candidate: &InteriorNormalizeCandidate,
) -> Result<PreparedInteriorNormalize, String> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly {
        return Err("interior normalization currently requires WebAssembly".into());
    }
    let original = state
        .module()
        .functions
        .iter()
        .find(|f| f.id == candidate.function)
        .ok_or("missing normalization function")?;
    let d = analyze_canonical_loops(original)
        .loops
        .into_iter()
        .find(|d| d.id == candidate.loop_id && d.header == candidate.header)
        .ok_or("stale normalization loop")?;
    let shape = propose_shape(original, &d).ok_or("unsupported interior normalization")?;
    let h = block(original, d.header).ok_or("missing source header")?;
    let b = block(original, shape.body).ok_or("missing source body")?;
    let mut trial = state.clone();
    let ids = [
        trial.fresh_block()?,
        trial.fresh_block()?,
        trial.fresh_block()?,
        trial.fresh_block()?,
        trial.fresh_block()?,
    ];
    let mut dispatch = parameters(&mut trial, h, ids[0], "interior_normalize_guard")?;
    let one = emit(
        &mut trial,
        &mut dispatch.instructions,
        KirInstructionKind::ConstInt { value: "1".into() },
        MirPrimitiveTypeName::U32,
    )?;
    let dispatch_values = h
        .params
        .iter()
        .zip(&dispatch.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    let gate = emit(
        &mut trial,
        &mut dispatch.instructions,
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: mapped(shape.bound, &dispatch_values),
            right: one,
        },
        MirPrimitiveTypeName::Bool,
    )?;
    dispatch.terminator = KirTerminator::Branch {
        condition: gate,
        then_edge: block_edge(&dispatch, ids[1]),
        else_edge: block_edge(&dispatch, h.id),
    };
    let mut setup = parameters(&mut trial, h, ids[1], "interior_normalize_origins")?;
    let setup_header_values = h
        .params
        .iter()
        .zip(&setup.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    let trip_count = emit(
        &mut trial,
        &mut setup.instructions,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: mapped(shape.bound, &setup_header_values),
            right: one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirPrimitiveTypeName::U32,
    )?;
    let zero = emit(
        &mut trial,
        &mut setup.instructions,
        KirInstructionKind::ConstInt { value: "0".into() },
        MirPrimitiveTypeName::U32,
    )?;
    let mut origin_values = setup_header_values.clone();
    for (param, arg) in b.params.iter().zip(&shape.body_edge.args) {
        origin_values.insert(param.value, mapped(*arg, &setup_header_values));
    }
    for (value, root) in &shape.invariant_values {
        origin_values.entry(*value).or_insert(*root);
    }
    for value in &shape.column_values {
        origin_values.insert(*value, one);
    }
    for source in &b.instructions {
        if shape.address_nodes.contains(&source.id) {
            let mut copy = source.clone();
            copy.id = trial.fresh_instruction()?;
            remap_instruction(&mut copy, &origin_values)?;
            for result in &mut copy.results {
                let fresh = trial.fresh_value()?;
                origin_values.insert(result.value, fresh);
                result.value = fresh;
            }
            setup.instructions.push(copy);
        }
    }
    let origins = shape
        .addresses
        .iter()
        .map(|(_, index)| (*index, mapped(*index, &origin_values)))
        .collect::<Values>();
    let mut setup_edge = block_edge(&setup, ids[2]);
    setup_edge.args[shape.induction_index] = zero;
    setup.terminator = KirTerminator::Jump { edge: setup_edge };
    let mut header = parameters(&mut trial, h, ids[2], "interior_normalized_header")?;
    let test = emit(
        &mut trial,
        &mut header.instructions,
        KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: header.params[shape.induction_index].value,
            right: trip_count,
        },
        MirPrimitiveTypeName::Bool,
    )?;
    header.terminator = KirTerminator::Branch {
        condition: test,
        then_edge: block_edge(&header, ids[3]),
        else_edge: block_edge(&header, ids[4]),
    };
    let mut body = parameters(&mut trial, h, ids[3], "interior_normalized_body")?;
    let j = body.params[shape.induction_index].value;
    let body_column = emit(
        &mut trial,
        &mut body.instructions,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: j,
            right: one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirPrimitiveTypeName::U32,
    )?;
    let mut values = h
        .params
        .iter()
        .zip(&body.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    values.insert(h.params[shape.induction_index].value, body_column);
    let mut memories = h
        .memory_params
        .iter()
        .zip(&body.memory_params)
        .map(|(a, b)| (a.version, b.version))
        .collect::<Memories>();
    bind(b, &shape.body_edge, &mut values, &mut memories)?;
    let mut effect = next_effect(original)?;
    let mut mapping = Vec::new();
    let mut addresses = Vec::new();
    let mut index_cache = BTreeMap::new();
    for source in &b.instructions {
        let mut copy = source.clone();
        copy.id = trial.fresh_instruction()?;
        remap_instruction(&mut copy, &values)?;
        if let Some((_, source_index)) = shape.addresses.iter().find(|(id, _)| *id == source.id) {
            let origin = origins[source_index];
            let index = if let Some(index) = index_cache.get(source_index) {
                *index
            } else {
                let index = emit(
                    &mut trial,
                    &mut body.instructions,
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left: origin,
                        right: j,
                        semantics: KirArithmeticSemantics::Modular,
                    },
                    MirPrimitiveTypeName::U32,
                )?;
                index_cache.insert(*source_index, index);
                index
            };
            memory_index_mut(&mut copy)
                .ok_or("memory access disappeared")?
                .clone_from(&index);
            addresses.push(NormalizedInteriorAddress {
                source: source.id,
                source_index: *source_index,
                origin,
                index,
            });
        }
        for r in &mut copy.results {
            let fresh = trial.fresh_value()?;
            values.insert(r.value, fresh);
            r.value = fresh;
        }
        if let Some(m) = &mut copy.memory {
            m.input = memories.get(&m.input).copied().unwrap_or(m.input);
            if let Some(old) = m.output {
                let fresh = trial.fresh_memory_version()?;
                memories.insert(old, fresh);
                m.output = Some(fresh);
            }
        }
        if let Some(e) = &mut copy.effect {
            e.order = effect;
            effect = effect.checked_add(1).ok_or("effect identity exhausted")?;
        }
        mapping.push((source.id, copy.id));
        body.instructions.push(copy);
    }
    let mut backedge = map_edge(&shape.backedge, &values, &memories);
    backedge.target = ids[2];
    backedge.args[shape.induction_index] = body_column;
    body.terminator = KirTerminator::Jump { edge: backedge };
    // Remove only now-dead address DAG nodes and the replaced scalar increment.
    let erasable = shape
        .address_nodes
        .iter()
        .copied()
        .chain([shape.step])
        .collect::<BTreeSet<_>>();
    let mut removed = Vec::new();
    loop {
        let used = body
            .instructions
            .iter()
            .flat_map(instruction_uses)
            .chain(terminator_uses(&body.terminator))
            .collect::<BTreeSet<_>>();
        let dead = mapping
            .iter()
            .filter_map(|(source, target)| {
                let i = body.instructions.iter().find(|i| i.id == *target)?;
                (erasable.contains(source)
                    && i.effect.is_none()
                    && i.memory.is_none()
                    && i.results.iter().all(|r| !used.contains(&r.value)))
                .then_some((*source, *target))
            })
            .collect::<Vec<_>>();
        if dead.is_empty() {
            break;
        }
        for (source, target) in dead {
            removed.push(source);
            body.instructions.retain(|i| i.id != target);
            mapping.retain(|pair| *pair != (source, target));
        }
    }
    removed.sort();
    let mut exit = parameters(&mut trial, h, ids[4], "interior_normalize_exit")?;
    let exit_column = emit(
        &mut trial,
        &mut exit.instructions,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: exit.params[shape.induction_index].value,
            right: one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirPrimitiveTypeName::U32,
    )?;
    let mut exit_values = h
        .params
        .iter()
        .zip(&exit.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    exit_values.insert(h.params[shape.induction_index].value, exit_column);
    let exit_memories = h
        .memory_params
        .iter()
        .zip(&exit.memory_params)
        .map(|(a, b)| (a.version, b.version))
        .collect();
    exit.terminator = KirTerminator::Jump {
        edge: map_edge(&shape.exit, &exit_values, &exit_memories),
    };
    let mut function = original.clone();
    let preheader = shape.descriptor.preheader.ok_or("missing preheader")?;
    function
        .blocks
        .iter_mut()
        .find(|b| b.id == preheader)
        .ok_or("missing preheader")?
        .terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: ids[0],
            ..shape.incoming.clone()
        },
    };
    function
        .blocks
        .extend([dispatch, setup, header, body, exit]);
    let before_units = kir_function_units(original);
    let after_units = kir_function_units(&function);
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|f| f.id == original.id)
        .ok_or("missing trial function")? = function;
    let plan = InteriorNormalizePlan {
        function: original.id,
        loop_id: candidate.loop_id,
        header: candidate.header,
        preheader,
        source_body: shape.body,
        pre_state_digest: state.kir_digest(),
        blocks: ids,
        one,
        zero,
        trip_count,
        body_column,
        exit_column,
        addresses,
        instruction_mapping: mapping,
        removed_instructions: removed,
        before_units,
        after_units,
    };
    let charge = charge(&plan);
    Ok(PreparedInteriorNormalize {
        trial,
        plan,
        charge,
    })
}

fn parameters(
    state: &mut KirVerifiedProgramState,
    h: &KirBlock,
    id: BlockId,
    label: &str,
) -> Result<KirBlock, String> {
    let mut b = h.clone();
    b.id = id;
    b.label = label.into();
    b.instructions.clear();
    for p in &mut b.params {
        p.value = state.fresh_value()?;
    }
    for p in &mut b.memory_params {
        p.version = state.fresh_memory_version()?;
    }
    Ok(b)
}
fn emit(
    state: &mut KirVerifiedProgramState,
    out: &mut Vec<KirInstruction>,
    kind: KirInstructionKind,
    ty: MirPrimitiveTypeName,
) -> Result<ValueId, String> {
    let value = state.fresh_value()?;
    out.push(KirInstruction {
        id: state.fresh_instruction()?,
        results: vec![KirResult {
            value,
            type_node: scalar(ty),
        }],
        kind,
        memory: None,
        effect: None,
    });
    Ok(value)
}
fn next_effect(f: &KirFunction) -> Result<u32, String> {
    f.blocks
        .iter()
        .flat_map(|b| {
            b.instructions
                .iter()
                .filter_map(|i| i.effect.as_ref().map(|e| e.order))
                .chain(match b.terminator {
                    KirTerminator::Return { effect_order, .. } => Some(effect_order),
                    _ => None,
                })
        })
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "effect identity exhausted".into())
}
fn block_edge(b: &KirBlock, target: BlockId) -> KirEdge {
    KirEdge {
        target,
        args: b.params.iter().map(|p| p.value).collect(),
        memory_args: b.memory_params.iter().map(|p| p.version).collect(),
    }
}
fn bind(
    target: &KirBlock,
    edge: &KirEdge,
    values: &mut Values,
    memories: &mut Memories,
) -> Result<(), String> {
    if target.id != edge.target
        || target.params.len() != edge.args.len()
        || target.memory_params.len() != edge.memory_args.len()
    {
        return Err("invalid scalar edge".into());
    }
    let args = edge
        .args
        .iter()
        .map(|v| mapped(*v, values))
        .collect::<Vec<_>>();
    let mem = edge
        .memory_args
        .iter()
        .map(|v| memories.get(v).copied().unwrap_or(*v))
        .collect::<Vec<_>>();
    for (p, v) in target.params.iter().zip(args) {
        values.insert(p.value, v);
    }
    for (p, v) in target.memory_params.iter().zip(mem) {
        memories.insert(p.version, v);
    }
    Ok(())
}
fn map_edge(e: &KirEdge, v: &Values, m: &Memories) -> KirEdge {
    KirEdge {
        target: e.target,
        args: e.args.iter().map(|x| mapped(*x, v)).collect(),
        memory_args: e
            .memory_args
            .iter()
            .map(|x| m.get(x).copied().unwrap_or(*x))
            .collect(),
    }
}
fn supported(i: &KirInstruction) -> bool {
    matches!(
        i.kind,
        KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::ConstFloat { .. }
            | KirInstructionKind::ConstBool { .. }
            | KirInstructionKind::Copy { .. }
            | KirInstructionKind::Binary { .. }
            | KirInstructionKind::Unary { .. }
            | KirInstructionKind::Compare { .. }
            | KirInstructionKind::Cast { .. }
            | KirInstructionKind::CheckCondition { .. }
            | KirInstructionKind::Guard { .. }
            | KirInstructionKind::Load { .. }
            | KirInstructionKind::Store { .. }
            | KirInstructionKind::SliceLen { .. }
            | KirInstructionKind::SliceData { .. }
    )
}
fn memory_index_mut(i: &mut KirInstruction) -> Option<&mut ValueId> {
    match &mut i.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
            match place.as_mut() {
                KirPlace::SliceIndex { index, .. } => Some(index),
                _ => None,
            }
        }
        _ => None,
    }
}
fn remap_instruction(i: &mut KirInstruction, values: &Values) -> Result<(), String> {
    let remap = |v: &mut ValueId| *v = mapped(*v, values);
    match &mut i.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => {}
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => remap(value),
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => {
            remap(left);
            remap(right);
        }
        KirInstructionKind::Unary { operand, .. } => remap(operand),
        KirInstructionKind::CheckCondition { args, .. } => {
            for v in args {
                remap(v);
            }
        }
        KirInstructionKind::Guard { condition, .. } => remap(condition),
        KirInstructionKind::Load { place } => remap_place(place, values),
        KirInstructionKind::Store { place, value } => {
            remap_place(place, values);
            remap(value);
        }
        KirInstructionKind::SliceLen { slice } | KirInstructionKind::SliceData { slice } => {
            remap(slice)
        }
        _ => return Err("unsupported scalar operation".into()),
    }
    Ok(())
}
fn remap_place(p: &mut KirPlace, v: &Values) {
    match p {
        KirPlace::Value { value, .. } => *value = mapped(*value, v),
        KirPlace::Deref { pointer, .. } => *pointer = mapped(*pointer, v),
        KirPlace::Index { base, index, .. } => {
            remap_place(base, v);
            *index = mapped(*index, v);
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            *slice = mapped(*slice, v);
            *index = mapped(*index, v);
        }
        KirPlace::Field { base, .. } => remap_place(base, v),
    }
}
fn instruction_uses(i: &KirInstruction) -> Vec<ValueId> {
    let mut uses = Vec::new();
    match &i.kind {
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => {
            uses.push(*value)
        }
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => uses.extend([*left, *right]),
        KirInstructionKind::Unary { operand, .. } => uses.push(*operand),
        KirInstructionKind::CheckCondition { args, .. } => uses.extend(args),
        KirInstructionKind::Guard { condition, .. } => uses.push(*condition),
        KirInstructionKind::Load { place } => place_uses(place, &mut uses),
        KirInstructionKind::Store { place, value } => {
            place_uses(place, &mut uses);
            uses.push(*value);
        }
        KirInstructionKind::SliceLen { slice } | KirInstructionKind::SliceData { slice } => {
            uses.push(*slice)
        }
        _ => {}
    }
    uses
}
fn place_uses(p: &KirPlace, out: &mut Vec<ValueId>) {
    match p {
        KirPlace::Value { value, .. } => out.push(*value),
        KirPlace::Deref { pointer, .. } => out.push(*pointer),
        KirPlace::Index { base, index, .. } => {
            place_uses(base, out);
            out.push(*index);
        }
        KirPlace::SliceIndex { slice, index, .. } => out.extend([*slice, *index]),
        KirPlace::Field { base, .. } => place_uses(base, out),
    }
}
fn terminator_uses(t: &KirTerminator) -> Vec<ValueId> {
    match t {
        KirTerminator::Jump { edge } => edge.args.clone(),
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => std::iter::once(*condition)
            .chain(then_edge.args.iter().chain(&else_edge.args).copied())
            .collect(),
        KirTerminator::Return { value, .. } => value.iter().copied().collect(),
    }
}
fn charge(plan: &InteriorNormalizePlan) -> CandidateBudgetCharge {
    CandidateBudgetCharge::single(
        plan.function,
        plan.after_units
            .saturating_sub(plan.before_units)
            .saturating_add(24),
        plan.before_units
            .saturating_add(plan.after_units)
            .saturating_add(48),
    )
}

pub fn check_interior_normalize_independently(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &InteriorNormalizePlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_normalization(pre, trial, plan, charge).map_err(TransactionCheckError::compiler)
}

fn source_edges(t: &KirTerminator) -> Vec<&KirEdge> {
    match t {
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        KirTerminator::Return { .. } => Vec::new(),
    }
}

fn proposer_constant(
    f: &KirFunction,
    value: ValueId,
    visiting: &mut BTreeSet<ValueId>,
) -> Option<u32> {
    if !visiting.insert(value)
        || visiting.len() > 64
        || value_type(f, value) != Some(scalar(MirPrimitiveTypeName::U32))
    {
        return None;
    }
    let result = if let Some(i) = definition(f, value) {
        match &i.kind {
            KirInstructionKind::ConstInt { value } => value.parse::<u32>().ok(),
            KirInstructionKind::Copy { value } => proposer_constant(f, *value, visiting),
            KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                let a = proposer_constant(f, *left, visiting)?;
                let b = proposer_constant(f, *right, visiting)?;
                Some(if *op == MirBinaryOp::Add {
                    a.wrapping_add(b)
                } else {
                    a.wrapping_sub(b)
                })
            }
            _ => None,
        }
    } else if let Some((owner, index)) = f.blocks.iter().find_map(|b| {
        b.params
            .iter()
            .position(|p| p.value == value)
            .map(|i| (b.id, i))
    }) {
        let mut values = Vec::new();
        for edge in f
            .blocks
            .iter()
            .flat_map(|b| source_edges(&b.terminator))
            .filter(|e| e.target == owner)
        {
            values.push(proposer_constant(f, *edge.args.get(index)?, visiting)?);
        }
        values
            .first()
            .copied()
            .filter(|first| values.iter().all(|v| v == first))
    } else {
        None
    };
    visiting.remove(&value);
    result
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AddressAtom {
    Column,
    Invariant(ValueId),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ModularPolynomial {
    constant: u32,
    terms: BTreeMap<AddressAtom, u32>,
}
impl ModularPolynomial {
    fn constant(value: u32) -> Self {
        Self {
            constant: value,
            terms: BTreeMap::new(),
        }
    }
    fn atom(atom: AddressAtom) -> Self {
        Self {
            constant: 0,
            terms: BTreeMap::from([(atom, 1)]),
        }
    }
    fn combine(mut self, other: Self, subtract: bool) -> Self {
        self.constant = if subtract {
            self.constant.wrapping_sub(other.constant)
        } else {
            self.constant.wrapping_add(other.constant)
        };
        for (atom, value) in other.terms {
            let old = self.terms.get(&atom).copied().unwrap_or(0);
            let value = if subtract {
                old.wrapping_sub(value)
            } else {
                old.wrapping_add(value)
            };
            if value == 0 {
                self.terms.remove(&atom);
            } else {
                self.terms.insert(atom, value);
            }
        }
        self
    }
    fn at_first_column(mut self) -> Self {
        let coefficient = self.terms.remove(&AddressAtom::Column).unwrap_or(0);
        self.constant = self.constant.wrapping_add(coefficient);
        self
    }
}

struct CheckedInterior {
    header: BlockId,
    body: BlockId,
    preheader: BlockId,
    column_slot: usize,
    bound: ValueId,
    incoming: KirEdge,
    body_edge: KirEdge,
    backedge: KirEdge,
    exit: KirEdge,
    source_symbols: BTreeMap<ValueId, ModularPolynomial>,
    address_nodes: BTreeSet<InstructionId>,
    addresses: Vec<(InstructionId, ValueId, ModularPolynomial)>,
    step: InstructionId,
}

// Separate source proof from proposal discovery: reconstruct the entry value,
// every backedge slot and each address polynomial directly from the frozen CFG.
fn reconstruct_interior_source(
    f: &KirFunction,
    plan: &InteriorNormalizePlan,
) -> Result<CheckedInterior, String> {
    let analysis = analyze_canonical_loops(f);
    let descriptor = analysis
        .loops
        .iter()
        .find(|d| d.header == plan.header && d.id == plan.loop_id)
        .ok_or("checker source loop missing")?;
    if !descriptor.innermost
        || !descriptor.lcssa
        || descriptor.blocks.len() != 2
        || descriptor.exits.len() != 1
        || descriptor.preheader != Some(plan.preheader)
    {
        return Err("checker source loop is not a closed two-block loop".into());
    }
    let h = block(f, plan.header).ok_or("checker header missing")?;
    let p = block(f, plan.preheader).ok_or("checker preheader missing")?;
    let KirTerminator::Jump { edge: incoming } = &p.terminator else {
        return Err("checker source entry is conditional".into());
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &h.terminator
    else {
        return Err("checker source header is not a branch".into());
    };
    if h.instructions.len() != 1
        || incoming.target != h.id
        || incoming.args.len() != h.params.len()
        || incoming.memory_args.len() != h.memory_params.len()
        || then_edge.target != plan.source_body
        || !descriptor.blocks.contains(&then_edge.target)
        || descriptor.blocks.contains(&else_edge.target)
    {
        return Err("checker source loop edges are false".into());
    }
    let comparison = &h.instructions[0];
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: column,
        right: bound,
    } = comparison.kind
    else {
        return Err("checker requires strict unsigned less-than".into());
    };
    if comparison.results.len() != 1
        || comparison.results[0].value != *condition
        || comparison.memory.is_some()
        || comparison.effect.is_some()
        || value_type(f, column) != Some(scalar(MirPrimitiveTypeName::U32))
        || value_type(f, bound) != Some(scalar(MirPrimitiveTypeName::U32))
        || column == bound
    {
        return Err("checker source range type is false".into());
    }
    let column_slot = h
        .params
        .iter()
        .position(|p| p.value == column)
        .ok_or("checker column is not a header parameter")?;
    if checker_entry_constant(f, incoming.args[column_slot], &mut Vec::new()) != Some(1) {
        return Err("checker cannot prove entry column one on every incoming path".into());
    }
    let b = block(f, then_edge.target).ok_or("checker source body missing")?;
    let KirTerminator::Jump { edge: backedge } = &b.terminator else {
        return Err("checker source body has multiple successors".into());
    };
    if backedge.target != h.id
        || backedge.args.len() != h.params.len()
        || then_edge.args.len() != b.params.len()
        || b.instructions.len() > 128
    {
        return Err("checker source backedge shape is false".into());
    }
    let roots = b
        .params
        .iter()
        .zip(&then_edge.args)
        .map(|(p, v)| (p.value, *v))
        .collect::<Values>();
    let resolve = |value| checker_forwarded_value(b, value, &roots);
    for (slot, (arg, param)) in backedge.args.iter().zip(&h.params).enumerate() {
        if slot != column_slot && resolve(*arg) != Some(param.value) {
            return Err("checker source bound or non-column state changes on the backedge".into());
        }
    }
    let step =
        definition(f, backedge.args[column_slot]).ok_or("checker source increment missing")?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = step.kind
    else {
        return Err("checker increment is not modular addition".into());
    };
    let is_one = |v| checker_entry_constant(f, v, &mut Vec::new()) == Some(1);
    if !((resolve(left) == Some(column) && is_one(right))
        || (resolve(right) == Some(column) && is_one(left)))
        || step.memory.is_some()
        || step.effect.is_some()
        || step.results.len() != 1
        || step.results[0].type_node != scalar(MirPrimitiveTypeName::U32)
    {
        return Err("checker increment is not exactly one".into());
    }
    let dominators = crate::compute_kir_dominators(f);
    let mut symbols = BTreeMap::new();
    for (slot, (param, value)) in h.params.iter().zip(&incoming.args).enumerate() {
        if param.type_node == scalar(MirPrimitiveTypeName::U32) {
            symbols.insert(
                param.value,
                if slot == column_slot {
                    ModularPolynomial::atom(AddressAtom::Column)
                } else {
                    checker_invariant_leaf(f, *value)
                },
            );
        }
    }
    for (param, value) in b.params.iter().zip(&then_edge.args) {
        if param.type_node == scalar(MirPrimitiveTypeName::U32) {
            let expression = symbols
                .get(value)
                .cloned()
                .unwrap_or_else(|| checker_invariant_leaf(f, *value));
            symbols.insert(param.value, expression);
        }
    }
    let mut address_nodes = BTreeSet::new();
    let mut addresses = Vec::new();
    for instruction in &b.instructions {
        // This allowlist is independent of the proposal's scalar filter.
        match instruction.kind {
            KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::ConstFloat { .. }
            | KirInstructionKind::ConstBool { .. }
            | KirInstructionKind::Copy { .. }
            | KirInstructionKind::Binary { .. }
            | KirInstructionKind::Unary { .. }
            | KirInstructionKind::Compare { .. }
            | KirInstructionKind::Cast { .. }
            | KirInstructionKind::CheckCondition { .. }
            | KirInstructionKind::Guard { .. }
            | KirInstructionKind::Load { .. }
            | KirInstructionKind::Store { .. }
            | KirInstructionKind::SliceLen { .. }
            | KirInstructionKind::SliceData { .. } => {}
            _ => return Err("checker unsupported scalar instruction".into()),
        }
        let place = match &instruction.kind {
            KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => place,
            _ => continue,
        };
        let KirPlace::SliceIndex { index, .. } = place.as_ref() else {
            return Err("checker memory address is not a slice index".into());
        };
        let polynomial = checker_source_polynomial(
            f,
            b,
            *index,
            &symbols,
            &dominators,
            p.id,
            &mut address_nodes,
            &mut BTreeSet::new(),
        )?;
        if polynomial.terms.get(&AddressAtom::Column) != Some(&1) {
            return Err("checker address is not unit-stride modulo 2^32".into());
        }
        addresses.push((instruction.id, *index, polynomial));
    }
    if addresses.len() < 3
        || addresses
            .iter()
            .map(|(_, v, _)| *v)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        || !b
            .instructions
            .iter()
            .any(|i| matches!(i.kind, KirInstructionKind::Store { .. }))
    {
        return Err("checker address-origin coverage is insufficient".into());
    }
    Ok(CheckedInterior {
        header: h.id,
        body: b.id,
        preheader: p.id,
        column_slot,
        bound,
        incoming: incoming.clone(),
        body_edge: then_edge.clone(),
        backedge: backedge.clone(),
        exit: else_edge.clone(),
        source_symbols: symbols,
        address_nodes,
        addresses,
        step: step.id,
    })
}

fn checker_forwarded_value(
    body: &KirBlock,
    mut value: ValueId,
    parameters: &Values,
) -> Option<ValueId> {
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(value) || seen.len() > 128 {
            return None;
        }
        if let Some(root) = parameters.get(&value) {
            return Some(*root);
        }
        match body
            .instructions
            .iter()
            .find(|i| i.results.iter().any(|r| r.value == value))
            .map(|i| &i.kind)
        {
            Some(KirInstructionKind::Copy { value: v }) => value = *v,
            _ => return Some(value),
        }
    }
}
fn checker_entry_constant(
    f: &KirFunction,
    value: ValueId,
    stack: &mut Vec<ValueId>,
) -> Option<u32> {
    if stack.contains(&value)
        || stack.len() >= 64
        || value_type(f, value) != Some(scalar(MirPrimitiveTypeName::U32))
    {
        return None;
    }
    stack.push(value);
    let result = if let Some(i) = definition(f, value) {
        if i.memory.is_some() || i.effect.is_some() {
            return None;
        }
        match &i.kind {
            KirInstructionKind::ConstInt { value } => value.parse().ok(),
            KirInstructionKind::Copy { value } => checker_entry_constant(f, *value, stack),
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } => Some(
                checker_entry_constant(f, *left, stack)?
                    .wrapping_add(checker_entry_constant(f, *right, stack)?),
            ),
            KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } => Some(
                checker_entry_constant(f, *left, stack)?
                    .wrapping_sub(checker_entry_constant(f, *right, stack)?),
            ),
            _ => None,
        }
    } else {
        let (owner, position) = f.blocks.iter().find_map(|b| {
            b.params
                .iter()
                .position(|p| p.value == value)
                .map(|p| (b.id, p))
        })?;
        let mut expected = None;
        let mut predecessors = 0;
        for b in &f.blocks {
            let edges = match &b.terminator {
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
                KirTerminator::Return { .. } => Vec::new(),
            };
            for edge in edges {
                if edge.target == owner {
                    let constant = checker_entry_constant(f, *edge.args.get(position)?, stack)?;
                    if expected.is_some_and(|old| old != constant) {
                        return None;
                    }
                    expected = Some(constant);
                    predecessors += 1;
                }
            }
        }
        if predecessors == 0 { None } else { expected }
    };
    stack.pop();
    result
}
fn checker_invariant_leaf(f: &KirFunction, v: ValueId) -> ModularPolynomial {
    match definition(f, v).map(|i| &i.kind) {
        Some(KirInstructionKind::ConstInt { value }) => value
            .parse::<u32>()
            .ok()
            .map(ModularPolynomial::constant)
            .unwrap_or_else(|| ModularPolynomial::atom(AddressAtom::Invariant(v))),
        _ => ModularPolynomial::atom(AddressAtom::Invariant(v)),
    }
}
#[allow(clippy::too_many_arguments)]
fn checker_source_polynomial(
    f: &KirFunction,
    b: &KirBlock,
    v: ValueId,
    symbols: &BTreeMap<ValueId, ModularPolynomial>,
    dominators: &crate::KirDominators,
    preheader: BlockId,
    nodes: &mut BTreeSet<InstructionId>,
    visiting: &mut BTreeSet<ValueId>,
) -> Result<ModularPolynomial, String> {
    if value_type(f, v) != Some(scalar(MirPrimitiveTypeName::U32)) {
        return Err("checker address operand is not u32".into());
    }
    if let Some(expression) = symbols.get(&v) {
        return Ok(expression.clone());
    }
    let Some(i) = b
        .instructions
        .iter()
        .find(|i| i.results.iter().any(|r| r.value == v))
    else {
        if let Some(owner) = f.blocks.iter().find(|b| {
            b.params.iter().any(|p| p.value == v)
                || b.instructions
                    .iter()
                    .any(|i| i.results.iter().any(|r| r.value == v))
        }) && !dominators.dominates(owner.id, preheader)
        {
            return Err("checker invariant does not dominate setup".into());
        }
        return Ok(checker_invariant_leaf(f, v));
    };
    if !visiting.insert(v)
        || visiting.len() > 128
        || i.results.len() != 1
        || i.effect.is_some()
        || i.memory.is_some()
    {
        return Err("checker address DAG is cyclic or effectful".into());
    }
    let result = match &i.kind {
        KirInstructionKind::ConstInt { value } => ModularPolynomial::constant(
            value
                .parse::<u32>()
                .map_err(|_| "checker address constant is not u32")?,
        ),
        KirInstructionKind::Copy { value } => checker_source_polynomial(
            f, b, *value, symbols, dominators, preheader, nodes, visiting,
        )?,
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
            let a = checker_source_polynomial(
                f, b, *left, symbols, dominators, preheader, nodes, visiting,
            )?;
            let c = checker_source_polynomial(
                f, b, *right, symbols, dominators, preheader, nodes, visiting,
            )?;
            a.combine(c, *op == MirBinaryOp::Sub)
        }
        _ => return Err("checker address DAG is not pure modular add/sub".into()),
    };
    visiting.remove(&v);
    nodes.insert(i.id);
    Ok(result)
}

fn check_normalization(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &InteriorNormalizePlan,
    budget: &CandidateBudgetCharge,
) -> Result<(), String> {
    if pre.module().config.consumer != crate::KirConsumer::WebAssembly
        || plan.pre_state_digest != pre.kir_digest()
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
        || pre.evidence_generation() != trial.evidence_generation()
        || pre.optimization_entry_module_units() != trial.optimization_entry_module_units()
    {
        return Err("normalization source/evidence identity changed".into());
    }
    let original = pre
        .module()
        .functions
        .iter()
        .find(|f| f.id == plan.function)
        .ok_or("normalization source function missing")?;
    let after = trial
        .module()
        .functions
        .iter()
        .find(|f| f.id == plan.function)
        .ok_or("normalization trial function missing")?;
    let checked = reconstruct_interior_source(original, plan)?;
    let h = block(original, checked.header).ok_or("source header missing")?;
    let b = block(original, checked.body).ok_or("source body missing")?;
    let mut outside = trial.module().clone();
    *outside
        .functions
        .iter_mut()
        .find(|f| f.id == plan.function)
        .ok_or("function missing")? = original.clone();
    if &outside != pre.module() {
        return Err("normalization changed module metadata or another function".into());
    }
    let mut metadata = after.clone();
    metadata.blocks = original.blocks.clone();
    if &metadata != original {
        return Err("normalization changed function metadata".into());
    }
    if plan.blocks.iter().copied().collect::<BTreeSet<_>>().len() != 5
        || plan.blocks.iter().any(|id| block(original, *id).is_some())
        || after.blocks.len() != original.blocks.len() + 5
        || after.blocks[original.blocks.len()..]
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>()
            != plan.blocks
    {
        return Err("normalization added block coverage is false".into());
    }
    for (old, new) in original.blocks.iter().zip(&after.blocks) {
        let mut expected = old.clone();
        if old.id == checked.preheader {
            expected.terminator = KirTerminator::Jump {
                edge: KirEdge {
                    target: plan.blocks[0],
                    ..checked.incoming.clone()
                },
            };
        }
        if &expected != new {
            return Err(
                "normalization changed Left store, scalar fallback or original effects".into(),
            );
        }
    }
    let added = plan
        .blocks
        .iter()
        .map(|id| block(after, *id).ok_or("normalization block missing"))
        .collect::<Result<Vec<_>, _>>()?;
    for new in &added {
        check_parameters(h, new)?;
    }
    let (dispatch, setup, header, body, exit) = (added[0], added[1], added[2], added[3], added[4]);
    let dispatch_values = h
        .params
        .iter()
        .zip(&dispatch.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    if dispatch.instructions.len() != 2 {
        return Err("normalization dispatch contains effects".into());
    }
    exact_pure(
        &dispatch.instructions[0],
        plan.one,
        KirInstructionKind::ConstInt { value: "1".into() },
        MirPrimitiveTypeName::U32,
    )?;
    let gate = one_result(&dispatch.instructions[1], MirPrimitiveTypeName::Bool)?;
    exact_pure(
        &dispatch.instructions[1],
        gate,
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: mapped(checked.bound, &dispatch_values),
            right: plan.one,
        },
        MirPrimitiveTypeName::Bool,
    )?;
    if dispatch.terminator
        != (KirTerminator::Branch {
            condition: gate,
            then_edge: block_edge(dispatch, setup.id),
            else_edge: block_edge(dispatch, h.id),
        })
    {
        return Err("normalization guard or scalar fallback is false".into());
    }
    if setup.instructions.len() < 2 {
        return Err("normalization setup missing range".into());
    }
    let setup_values = h
        .params
        .iter()
        .zip(&setup.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    exact_pure(
        &setup.instructions[0],
        plan.trip_count,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: mapped(checked.bound, &setup_values),
            right: plan.one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirPrimitiveTypeName::U32,
    )?;
    exact_pure(
        &setup.instructions[1],
        plan.zero,
        KirInstructionKind::ConstInt { value: "0".into() },
        MirPrimitiveTypeName::U32,
    )?;
    let mut setup_edge = block_edge(setup, header.id);
    setup_edge.args[checked.column_slot] = plan.zero;
    if setup.terminator != (KirTerminator::Jump { edge: setup_edge }) {
        return Err("normalization zero-origin entry is false".into());
    }
    let mut setup_symbols = BTreeMap::from([(plan.one, ModularPolynomial::constant(1))]);
    for (slot, (new, old)) in setup.params.iter().zip(&h.params).enumerate() {
        if new.type_node == scalar(MirPrimitiveTypeName::U32) {
            let expression = if slot == checked.column_slot {
                ModularPolynomial::constant(1)
            } else {
                checked
                    .source_symbols
                    .get(&old.value)
                    .cloned()
                    .ok_or("checker setup invariant missing")?
            };
            setup_symbols.insert(new.value, expression);
        }
    }
    for i in &setup.instructions {
        if i.results.len() != 1
            || i.results[0].type_node != scalar(MirPrimitiveTypeName::U32)
            || i.memory.is_some()
            || i.effect.is_some()
        {
            return Err("normalization setup is not pure u32 arithmetic".into());
        }
        let lookup = |v| {
            setup_symbols
                .get(&v)
                .cloned()
                .or_else(|| {
                    value_type(original, v)
                        .filter(|ty| *ty == scalar(MirPrimitiveTypeName::U32))
                        .map(|_| checker_invariant_leaf(original, v))
                })
                .ok_or("checker setup refers to a non-invariant value")
        };
        let expression = match &i.kind {
            KirInstructionKind::ConstInt { value } => {
                ModularPolynomial::constant(value.parse::<u32>().map_err(|_| "bad setup integer")?)
            }
            KirInstructionKind::Copy { value } => lookup(*value)?,
            KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                lookup(*left)?.combine(lookup(*right)?, *op == MirBinaryOp::Sub)
            }
            _ => return Err("normalization setup speculates an unsupported operation".into()),
        };
        setup_symbols.insert(i.results[0].value, expression);
    }
    if header.instructions.len() != 1 {
        return Err("normalization header contains extra operations".into());
    }
    let condition = one_result(&header.instructions[0], MirPrimitiveTypeName::Bool)?;
    exact_pure(
        &header.instructions[0],
        condition,
        KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: header.params[checked.column_slot].value,
            right: plan.trip_count,
        },
        MirPrimitiveTypeName::Bool,
    )?;
    if header.terminator
        != (KirTerminator::Branch {
            condition,
            then_edge: block_edge(header, body.id),
            else_edge: block_edge(header, exit.id),
        })
    {
        return Err("normalization iteration partition is false".into());
    }
    if plan.addresses.len() != checked.addresses.len() {
        return Err("normalization access coverage is false".into());
    }
    for ((source, index, polynomial), address) in checked.addresses.iter().zip(&plan.addresses) {
        if address.source != *source
            || address.source_index != *index
            || setup_symbols.get(&address.origin) != Some(&polynomial.clone().at_first_column())
        {
            return Err("normalization modular origin equation is false".into());
        }
    }
    verify_normalized_body(original, &checked, h, b, body, plan)?;
    if exit.instructions.len() != 1 {
        return Err("normalization exit contains extra operations".into());
    }
    exact_pure(
        &exit.instructions[0],
        plan.exit_column,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: exit.params[checked.column_slot].value,
            right: plan.one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirPrimitiveTypeName::U32,
    )?;
    let mut exit_values = h
        .params
        .iter()
        .zip(&exit.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    exit_values.insert(h.params[checked.column_slot].value, plan.exit_column);
    let exit_memories = h
        .memory_params
        .iter()
        .zip(&exit.memory_params)
        .map(|(a, b)| (a.version, b.version))
        .collect();
    if exit.terminator
        != (KirTerminator::Jump {
            edge: map_edge(&checked.exit, &exit_values, &exit_memories),
        })
    {
        return Err("normalization Right column or exit memory state is false".into());
    }
    if plan.before_units != kir_function_units(original)
        || plan.after_units != kir_function_units(after)
        || plan.after_units.saturating_sub(plan.before_units) > 256
        || budget != &charge(plan)
    {
        return Err("normalization growth/budget is false".into());
    }
    let validation = crate::validate_kir_module(trial.module());
    if !validation.errors.is_empty() {
        return Err(format!(
            "normalization structural verification failed: {:?}",
            validation.errors
        ));
    }
    Ok(())
}

fn check_parameters(old: &KirBlock, new: &KirBlock) -> Result<(), String> {
    if old.params.len() != new.params.len()
        || old.memory_params.len() != new.memory_params.len()
        || old
            .params
            .iter()
            .zip(&new.params)
            .any(|(a, b)| a.type_node != b.type_node || a.slot != b.slot)
        || old
            .memory_params
            .iter()
            .zip(&new.memory_params)
            .any(|(a, b)| a.region != b.region)
    {
        return Err("normalization parameter or MemorySSA shape changed".into());
    }
    Ok(())
}
fn one_result(i: &KirInstruction, ty: MirPrimitiveTypeName) -> Result<ValueId, String> {
    if i.results.len() != 1 || i.results[0].type_node != scalar(ty) {
        return Err("normalization scalar result type is false".into());
    }
    Ok(i.results[0].value)
}
fn exact_pure(
    i: &KirInstruction,
    value: ValueId,
    kind: KirInstructionKind,
    ty: MirPrimitiveTypeName,
) -> Result<(), String> {
    if one_result(i, ty)? != value || i.kind != kind || i.memory.is_some() || i.effect.is_some() {
        return Err("normalization scalar range/column scaffold is false".into());
    }
    Ok(())
}

fn verify_normalized_body(
    f: &KirFunction,
    source: &CheckedInterior,
    h: &KirBlock,
    b: &KirBlock,
    actual: &KirBlock,
    plan: &InteriorNormalizePlan,
) -> Result<(), String> {
    let removed = plan
        .removed_instructions
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if removed.len() != plan.removed_instructions.len()
        || removed
            .iter()
            .any(|id| !source.address_nodes.contains(id) && *id != source.step)
    {
        return Err("normalization removed a non-address operation".into());
    }
    let removed_values = b
        .instructions
        .iter()
        .filter(|i| removed.contains(&i.id))
        .flat_map(|i| i.results.iter().map(|r| r.value))
        .collect::<BTreeSet<_>>();
    for i in &b.instructions {
        if removed.contains(&i.id) {
            continue;
        }
        let uses = match &i.kind {
            KirInstructionKind::Load { place } => match place.as_ref() {
                KirPlace::SliceIndex { slice, .. } => vec![*slice],
                _ => instruction_uses(i),
            },
            KirInstructionKind::Store { place, value } => match place.as_ref() {
                KirPlace::SliceIndex { slice, .. } => vec![*slice, *value],
                _ => instruction_uses(i),
            },
            _ => instruction_uses(i),
        };
        if uses.iter().any(|v| removed_values.contains(v)) {
            return Err("normalization erased an observed intermediate address".into());
        }
    }
    if source
        .backedge
        .args
        .iter()
        .enumerate()
        .any(|(slot, v)| slot != source.column_slot && removed_values.contains(v))
    {
        return Err("normalization erased an escaping address".into());
    }
    let retained = b
        .instructions
        .iter()
        .filter(|i| !removed.contains(&i.id))
        .collect::<Vec<_>>();
    if plan.instruction_mapping.len() != retained.len() {
        return Err("normalization source instruction coverage is false".into());
    }
    let j = actual.params[source.column_slot].value;
    exact_pure(
        actual
            .instructions
            .first()
            .ok_or("normalization body has no counter")?,
        plan.body_column,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: j,
            right: plan.one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirPrimitiveTypeName::U32,
    )?;
    let mut values = h
        .params
        .iter()
        .zip(&actual.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    values.insert(h.params[source.column_slot].value, plan.body_column);
    let mut memories = h
        .memory_params
        .iter()
        .zip(&actual.memory_params)
        .map(|(a, b)| (a.version, b.version))
        .collect::<Memories>();
    bind(b, &source.body_edge, &mut values, &mut memories)?;
    let mut position = 1;
    // An emitted SSA index may be shared by a load and store only when both
    // refer to the same independently checked source index and origin. Merely
    // remembering that the SSA builder was checked once loses that binding.
    let mut indices = BTreeMap::new();
    let mut effect = next_effect(f)?;
    for (source_instruction, mapping) in retained.iter().zip(&plan.instruction_mapping) {
        let address = plan
            .addresses
            .iter()
            .find(|a| a.source == source_instruction.id);
        if let Some(address) = address {
            let binding = (address.source_index, address.origin);
            if let Some(checked_binding) = indices.get(&address.index) {
                if *checked_binding != binding {
                    return Err(
                        "normalization reused an index for a different source index or origin"
                            .into(),
                    );
                }
            } else {
                let added = actual
                    .instructions
                    .get(position)
                    .ok_or("normalization index builder missing")?;
                exact_pure(
                    added,
                    address.index,
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left: address.origin,
                        right: j,
                        semantics: KirArithmeticSemantics::Modular,
                    },
                    MirPrimitiveTypeName::U32,
                )?;
                indices.insert(address.index, binding);
                position += 1;
            }
        }
        let after = actual
            .instructions
            .get(position)
            .ok_or("normalization cloned scalar instruction missing")?;
        if *mapping != (source_instruction.id, after.id)
            || source_instruction.results.len() != after.results.len()
        {
            return Err("normalization reordered scalar operations or load/trap order".into());
        }
        let mut expected = (*source_instruction).clone();
        expected.id = after.id;
        remap_instruction(&mut expected, &values)?;
        if let Some(address) = address {
            *memory_index_mut(&mut expected).ok_or("normalization source memory kind changed")? =
                address.index;
        }
        for (r, new) in expected.results.iter_mut().zip(&after.results) {
            if r.type_node != new.type_node {
                return Err("normalization cloned type changed".into());
            }
            values.insert(r.value, new.value);
            r.value = new.value;
        }
        if let Some(m) = &mut expected.memory {
            let actual_memory = after
                .memory
                .as_ref()
                .ok_or("normalization memory record missing")?;
            m.input = memories.get(&m.input).copied().unwrap_or(m.input);
            if let Some(old) = m.output {
                let new = actual_memory
                    .output
                    .ok_or("normalization memory output missing")?;
                memories.insert(old, new);
                m.output = Some(new);
            }
        }
        if let Some(e) = &mut expected.effect {
            e.order = effect;
            effect = effect.checked_add(1).ok_or("effect identity exhausted")?;
        }
        if &expected != after {
            return Err("normalization changed strict arithmetic, scalar value, memory state or effect order".into());
        }
        position += 1;
    }
    if position != actual.instructions.len() {
        return Err("normalization body contains unverified extra instructions".into());
    }
    let mut backedge = map_edge(&source.backedge, &values, &memories);
    backedge.target = plan.blocks[2];
    backedge.args[source.column_slot] = plan.body_column;
    if actual.terminator != (KirTerminator::Jump { edge: backedge }) {
        return Err("normalization counter increment or invariant backedge state is false".into());
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct InteriorNormalizeFrontierResult {
    pub accepted: u32,
    pub rejected: u32,
    pub fallbacks: Vec<crate::KirAnalysisFallback>,
}

/// Try each scalar interior at most once in this frontier; rejected trials leave
/// the entire verified state intact through the existing transaction mechanism.
pub fn run_interior_normalize_frontier(
    state: &mut KirVerifiedProgramState,
    audit: &mut crate::KirOptimizationAuditState,
) -> Result<InteriorNormalizeFrontierResult, String> {
    let mut result = InteriorNormalizeFrontierResult::default();
    let mut processed = BTreeSet::new();
    loop {
        let mut candidates = discover_interior_normalize_candidates(state);
        candidates.sort_by_key(|c| (c.function, c.header));
        let Some(candidate) = candidates
            .into_iter()
            .find(|c| processed.insert((c.function, c.header)))
        else {
            break;
        };
        let key = crate::CandidateKey::LoopFrontier {
            function: candidate.function,
            loop_id: candidate.loop_id,
            kind: crate::LoopCandidateKind::InteriorNormalize,
            variant: crate::LoopCandidateVariant::Scalar,
            vf: 1,
            uf: 1,
        };
        let prepared = match prepare_interior_normalize_trial(state, &candidate) {
            Ok(prepared) => prepared,
            Err(reason) => {
                audit.record_noncommitting_attempt(
                    key,
                    CandidateBudgetCharge::single(candidate.function, 24, 48),
                    crate::CandidateDisposition::Rejected,
                    &reason,
                )?;
                result.rejected = result.rejected.saturating_add(1);
                result.fallbacks.push(crate::KirAnalysisFallback {
                    function: candidate.function,
                    pass: "interior-normalize".into(),
                    reason,
                });
                continue;
            }
        };
        let plan = prepared.plan;
        let charge = prepared.charge;
        let proposed = prepared.trial;
        match crate::execute_verified_transaction(
            state,
            audit,
            key,
            charge.clone(),
            move |trial| {
                *trial = proposed;
                Ok(())
            },
            |before, after| check_interior_normalize_independently(before, after, &plan, &charge),
        ) {
            crate::TransactionOutcome::Committed => {
                result.accepted = result.accepted.saturating_add(1)
            }
            crate::TransactionOutcome::Rejected | crate::TransactionOutcome::BudgetExhausted => {
                result.rejected = result.rejected.saturating_add(1);
                result.fallbacks.push(crate::KirAnalysisFallback {
                    function: candidate.function,
                    pass: "interior-normalize".into(),
                    reason: "independent-check-or-budget-rejected".into(),
                });
            }
            crate::TransactionOutcome::CompilerError(error) => return Err(error),
        }
    }
    Ok(result)
}
