//! Scalar boundary-column peeling. The original loop remains the fallback.
//!
//! Only a unit-step `u32` loop with two boundary tests and a statically selectable
//! scalar path is accepted. Peeling changes control flow, never memory or floating
//! point evaluation order. No relation between dimensions and slice lengths is
//! assumed, and input/output aliasing is allowed.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, CandidateBudgetCharge, CanonicalLoopDescriptor, FunctionId, InstructionId,
    KirArithmeticSemantics, KirBlock, KirEdge, KirFunction, KirInstruction, KirInstructionKind,
    KirPlace, KirResult, KirTerminator, KirValueType, KirVerifiedProgramState, LoopId,
    MemoryVersionId, MirBinaryOp, MirCompareOp, MirType, TransactionCheckError, ValueId,
    analyze_canonical_loops, kir_function_units,
};

const MAX_LOOP_BLOCKS: usize = 20;
const MAX_PATH_INSTRUCTIONS: usize = 96;
type Values = BTreeMap<ValueId, ValueId>;
type Memories = BTreeMap<MemoryVersionId, MemoryVersionId>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StencilPeelCandidate {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StencilPeelPhase {
    pub block: BlockId,
    pub source_blocks: Vec<BlockId>,
    pub instruction_mapping: Vec<(InstructionId, InstructionId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StencilPeelPlan {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
    pub pre_state_digest: String,
    pub preheader: BlockId,
    pub guards: Vec<BlockId>,
    pub interior_header: BlockId,
    pub phases: [StencilPeelPhase; 3],
    pub added_instructions: [InstructionId; 4],
    pub before_units: u32,
    pub after_units: u32,
}

#[derive(Debug, Clone)]
pub struct PreparedStencilPeel {
    pub trial: KirVerifiedProgramState,
    pub plan: StencilPeelPlan,
    pub charge: CandidateBudgetCharge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Symbol {
    Column,
    NextColumn,
    Width,
    Integer(u32),
    Boolean(bool),
    Left,
    Right,
    Root(ValueId),
    Unknown,
}

#[derive(Debug, Clone)]
struct Path {
    blocks: Vec<BlockId>,
    edges: Vec<KirEdge>,
}

#[derive(Debug, Clone)]
struct Shape {
    descriptor: CanonicalLoopDescriptor,
    induction_index: usize,
    width: ValueId,
    incoming: KirEdge,
    outgoing: KirEdge,
    invariant_conditions: Vec<ValueId>,
    paths: [Path; 3],
}

#[must_use]
pub fn discover_stencil_peel_candidates(
    state: &KirVerifiedProgramState,
) -> Vec<StencilPeelCandidate> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly {
        return Vec::new();
    }
    state
        .module()
        .functions
        .iter()
        .flat_map(|function| {
            analyze_canonical_loops(function)
                .loops
                .into_iter()
                .filter_map(|descriptor| {
                    recognize(function, &descriptor).map(|_| StencilPeelCandidate {
                        function: function.id,
                        loop_id: descriptor.id,
                        header: descriptor.header,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn block(function: &KirFunction, id: BlockId) -> Option<&KirBlock> {
    function.blocks.iter().find(|block| block.id == id)
}

fn recognize(function: &KirFunction, descriptor: &CanonicalLoopDescriptor) -> Option<Shape> {
    if !descriptor.innermost
        || !descriptor.lcssa
        || descriptor.exits.len() != 1
        || descriptor.blocks.len() > MAX_LOOP_BLOCKS
        || descriptor.blocks.len() < 4
        || !function.vector_regions.is_empty()
    {
        return None;
    }
    let induction = descriptor.induction.as_ref()?;
    if induction.start.to_string() != "0"
        || induction.step.to_string() != "1"
        || induction.comparison != MirCompareOp::Lt
    {
        return None;
    }
    let header = block(function, descriptor.header)?;
    let induction_index = header
        .params
        .iter()
        .position(|p| p.value == induction.value)?;
    if header.params[induction_index].type_node
        != KirValueType::Scalar(MirType::Primitive(crate::MirPrimitiveTypeName::U32))
    {
        return None;
    }
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: compared,
        right: width,
    } = header.instructions.first()?.kind
    else {
        return None;
    };
    if compared != induction.value {
        return None;
    }
    if !header.params.iter().any(|p| {
        p.value == width
            && p.type_node
                == KirValueType::Scalar(MirType::Primitive(crate::MirPrimitiveTypeName::U32))
    }) {
        return None;
    }
    // The fast loop supplies its own equivalent range test. No ordered stop point
    // or computation other than this comparison may be omitted from the header.
    if header.instructions.len() != 1 {
        return None;
    }
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    let comparison = &header.instructions[0];
    if comparison.kind
        != (KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: induction.value,
            right: width,
        })
        || comparison.results.len() != 1
        || comparison.results[0].value != *condition
        || comparison.effect.is_some()
        || comparison.memory.is_some()
    {
        return None;
    }
    if descriptor.blocks.contains(&else_edge.target)
        || !descriptor.blocks.contains(&then_edge.target)
    {
        return None;
    }
    let preheader = block(function, descriptor.preheader?)?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return None;
    };
    if incoming.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
    {
        return None;
    }
    let mut invariants = BTreeSet::new();
    let paths = [
        trace(
            function,
            descriptor,
            induction_index,
            width,
            true,
            false,
            &mut invariants,
        )?,
        trace(
            function,
            descriptor,
            induction_index,
            width,
            false,
            false,
            &mut invariants,
        )?,
        trace(
            function,
            descriptor,
            induction_index,
            width,
            false,
            true,
            &mut invariants,
        )?,
    ];
    if invariants.len() > 2 {
        return None;
    }
    let effects = |path: &Path| -> Option<(Vec<InstructionId>, usize)> {
        let mut stores = Vec::new();
        let mut loads = 0;
        for id in &path.blocks {
            for i in &block(function, *id)?.instructions {
                match i.kind {
                    KirInstructionKind::Store { .. } => stores.push(i.id),
                    KirInstructionKind::Load { .. } => loads += 1,
                    _ => {}
                }
            }
        }
        Some((stores, loads))
    };
    let left = effects(&paths[0])?;
    let interior = effects(&paths[1])?;
    let right = effects(&paths[2])?;
    if left.0.len() != 1
        || left.1 != 0
        || right != left
        || interior.0.len() != 1
        || interior.0 == left.0
        || interior.1 == 0
    {
        return None;
    }
    // Keep growth bounded even when the scalar body itself is sizeable.
    if paths.iter().any(|p| {
        p.blocks
            .iter()
            .filter_map(|id| block(function, *id))
            .map(|b| b.instructions.len())
            .sum::<usize>()
            > MAX_PATH_INSTRUCTIONS
    }) {
        return None;
    }
    let path_units = paths
        .iter()
        .flat_map(|p| &p.blocks)
        .filter_map(|id| block(function, *id))
        .map(|b| b.instructions.len())
        .sum::<usize>();
    let added_units = 5
        + invariants.len()
        + 4 * (1 + header.params.len() + header.memory_params.len())
        + path_units;
    if added_units > 192 {
        return None;
    }
    Some(Shape {
        descriptor: descriptor.clone(),
        induction_index,
        width,
        incoming: incoming.clone(),
        outgoing: else_edge.clone(),
        invariant_conditions: invariants.into_iter().collect(),
        paths,
    })
}

fn trace(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    induction_index: usize,
    width: ValueId,
    left: bool,
    right: bool,
    invariants: &mut BTreeSet<ValueId>,
) -> Option<Path> {
    let header = block(function, descriptor.header)?;
    let mut symbols = BTreeMap::new();
    let mut local_definitions = BTreeSet::new();
    let mut bool_values = BTreeSet::new();
    for b in &function.blocks {
        for p in &b.params {
            if descriptor.blocks.contains(&b.id) {
                local_definitions.insert(p.value);
            }
            if p.type_node
                == KirValueType::Scalar(MirType::Primitive(crate::MirPrimitiveTypeName::Bool))
            {
                bool_values.insert(p.value);
            }
        }
        for i in &b.instructions {
            for r in &i.results {
                if descriptor.blocks.contains(&b.id) {
                    local_definitions.insert(r.value);
                }
                if r.type_node
                    == KirValueType::Scalar(MirType::Primitive(crate::MirPrimitiveTypeName::Bool))
                {
                    bool_values.insert(r.value);
                }
                match &i.kind {
                    KirInstructionKind::ConstInt { value } => {
                        if let Ok(n) = value.parse::<u32>() {
                            symbols.insert(r.value, Symbol::Integer(n));
                        }
                    }
                    KirInstructionKind::ConstBool { value } => {
                        symbols.insert(r.value, Symbol::Boolean(*value));
                    }
                    _ => {}
                }
            }
        }
    }
    for p in &function.params {
        if p.type_node == MirType::Primitive(crate::MirPrimitiveTypeName::Bool) {
            bool_values.insert(p.value);
        }
    }
    for p in &header.params {
        symbols.insert(
            p.value,
            if p.value == width {
                Symbol::Width
            } else if p.value == header.params[induction_index].value {
                Symbol::Column
            } else {
                Symbol::Root(p.value)
            },
        );
    }
    let sym = |v: ValueId, symbols: &BTreeMap<ValueId, Symbol>| {
        symbols.get(&v).cloned().unwrap_or(Symbol::Root(v))
    };
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        return None;
    };
    let mut edge = then_edge.clone();
    let mut path = Path {
        blocks: Vec::new(),
        edges: vec![edge.clone()],
    };
    loop {
        if edge.target == header.id {
            if edge.args.len() != header.params.len() {
                return None;
            }
            for (index, (argument, param)) in edge.args.iter().zip(&header.params).enumerate() {
                let expected = if index == induction_index {
                    Symbol::NextColumn
                } else if param.value == width {
                    Symbol::Width
                } else {
                    Symbol::Root(param.value)
                };
                if sym(*argument, &symbols) != expected {
                    return None;
                }
            }
            return Some(path);
        }
        if !descriptor.blocks.contains(&edge.target)
            || path.blocks.contains(&edge.target)
            || path.blocks.len() >= MAX_LOOP_BLOCKS
        {
            return None;
        }
        let current = block(function, edge.target)?;
        if current.params.len() != edge.args.len()
            || current.memory_params.len() != edge.memory_args.len()
        {
            return None;
        }
        let incoming = edge
            .args
            .iter()
            .map(|v| sym(*v, &symbols))
            .collect::<Vec<_>>();
        for (param, value) in current.params.iter().zip(incoming) {
            symbols.insert(param.value, value);
        }
        path.blocks.push(current.id);
        for instruction in &current.instructions {
            if !supported_instruction(instruction) {
                return None;
            }
            let value =
                match &instruction.kind {
                    KirInstructionKind::ConstInt { value } => value
                        .parse::<u32>()
                        .ok()
                        .map(Symbol::Integer)
                        .unwrap_or(Symbol::Unknown),
                    KirInstructionKind::ConstBool { value } => Symbol::Boolean(*value),
                    KirInstructionKind::Copy { value } => sym(*value, &symbols),
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left: a,
                        right: b,
                        semantics: KirArithmeticSemantics::Modular,
                    } => match (sym(*a, &symbols), sym(*b, &symbols)) {
                        (Symbol::Column, Symbol::Integer(1))
                        | (Symbol::Integer(1), Symbol::Column) => Symbol::NextColumn,
                        _ => Symbol::Unknown,
                    },
                    KirInstructionKind::Compare {
                        op: MirCompareOp::Eq,
                        left: a,
                        right: b,
                    } => match (sym(*a, &symbols), sym(*b, &symbols)) {
                        (Symbol::Column, Symbol::Integer(0))
                        | (Symbol::Integer(0), Symbol::Column) => Symbol::Left,
                        (Symbol::NextColumn, Symbol::Width)
                        | (Symbol::Width, Symbol::NextColumn) => Symbol::Right,
                        _ => Symbol::Unknown,
                    },
                    _ => Symbol::Unknown,
                };
            for r in &instruction.results {
                symbols.insert(r.value, value.clone());
            }
        }
        edge = match &current.terminator {
            KirTerminator::Jump { edge } => edge.clone(),
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let taken = match sym(*condition, &symbols) {
                    Symbol::Boolean(v) => v,
                    Symbol::Left => left,
                    Symbol::Right => right,
                    Symbol::Root(v)
                        if bool_values.contains(&v) && !local_definitions.contains(&v) =>
                    {
                        invariants.insert(v);
                        false
                    }
                    Symbol::Root(v)
                        if header.params.iter().any(|p| {
                            p.value == v
                                && p.type_node
                                    == KirValueType::Scalar(MirType::Primitive(
                                        crate::MirPrimitiveTypeName::Bool,
                                    ))
                        }) =>
                    {
                        invariants.insert(v);
                        false
                    }
                    _ => return None,
                };
                if taken {
                    then_edge.clone()
                } else {
                    else_edge.clone()
                }
            }
            KirTerminator::Return { .. } => return None,
        };
        path.edges.push(edge.clone());
    }
}

fn supported_instruction(i: &KirInstruction) -> bool {
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

pub fn prepare_stencil_peel_trial(
    state: &KirVerifiedProgramState,
    candidate: &StencilPeelCandidate,
) -> Result<PreparedStencilPeel, String> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly {
        return Err("scalar boundary peeling currently requires WebAssembly".into());
    }
    let original = state
        .module()
        .functions
        .iter()
        .find(|f| f.id == candidate.function)
        .ok_or("missing peel function")?;
    let descriptor = analyze_canonical_loops(original)
        .loops
        .into_iter()
        .find(|l| l.id == candidate.loop_id && l.header == candidate.header)
        .ok_or("stale peel loop")?;
    let shape = recognize(original, &descriptor).ok_or("unsupported boundary peel")?;
    let mut trial = state.clone();
    let mut function = original.clone();
    let header = block(original, candidate.header).ok_or("missing header")?;
    let preheader_id = shape.descriptor.preheader.ok_or("missing preheader")?;
    let pre_values = header
        .params
        .iter()
        .zip(&shape.incoming.args)
        .map(|(p, v)| (p.value, *v))
        .collect::<Values>();
    let width = mapped(shape.width, &pre_values);
    let mut extra = Vec::new();
    let one = emit_value(
        &mut trial,
        &mut extra,
        KirInstructionKind::ConstInt { value: "1".into() },
        MirType::Primitive(crate::MirPrimitiveTypeName::U32),
    )?;
    let three = emit_value(
        &mut trial,
        &mut extra,
        KirInstructionKind::ConstInt { value: "3".into() },
        MirType::Primitive(crate::MirPrimitiveTypeName::U32),
    )?;
    let last = emit_value(
        &mut trial,
        &mut extra,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: width,
            right: one,
            semantics: KirArithmeticSemantics::Modular,
        },
        MirType::Primitive(crate::MirPrimitiveTypeName::U32),
    )?;
    let gate = emit_value(
        &mut trial,
        &mut extra,
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: width,
            right: three,
        },
        MirType::Primitive(crate::MirPrimitiveTypeName::Bool),
    )?;
    let added_instructions = std::array::from_fn(|i| extra[i].id);
    let guards = shape
        .invariant_conditions
        .iter()
        .map(|_| trial.fresh_block())
        .collect::<Result<Vec<_>, _>>()?;
    let left_id = trial.fresh_block()?;
    let inner_header_id = trial.fresh_block()?;
    let body_id = trial.fresh_block()?;
    let right_id = trial.fresh_block()?;
    let mut next_effect = next_effect(original)?;
    let (mut left, left_edge, left_phase) = clone_path(
        &mut trial,
        original,
        header,
        &shape.paths[0],
        left_id,
        "peel_left",
        &mut next_effect,
    )?;
    let (mut body, body_edge, body_phase) = clone_path(
        &mut trial,
        original,
        header,
        &shape.paths[1],
        body_id,
        "peel_interior",
        &mut next_effect,
    )?;
    let (mut right, right_edge, right_phase) = clone_path(
        &mut trial,
        original,
        header,
        &shape.paths[2],
        right_id,
        "peel_right",
        &mut next_effect,
    )?;
    left.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: inner_header_id,
            ..left_edge
        },
    };
    body.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: inner_header_id,
            ..body_edge
        },
    };
    let right_values = header
        .params
        .iter()
        .zip(&right_edge.args)
        .map(|(p, v)| (p.value, *v))
        .collect();
    let right_memories = header
        .memory_params
        .iter()
        .zip(&right_edge.memory_args)
        .map(|(p, v)| (p.version, *v))
        .collect();
    right.terminator = KirTerminator::Jump {
        edge: map_edge(&shape.outgoing, &right_values, &right_memories),
    };
    let mut inner_header =
        clone_parameters(&mut trial, header, inner_header_id, "peel_interior_header")?;
    let column = inner_header.params[shape.induction_index].value;
    let cond = emit_value(
        &mut trial,
        &mut inner_header.instructions,
        KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: column,
            right: last,
        },
        MirType::Primitive(crate::MirPrimitiveTypeName::Bool),
    )?;
    inner_header.terminator = KirTerminator::Branch {
        condition: cond,
        then_edge: block_edge(&inner_header, body_id),
        else_edge: block_edge(&inner_header, right_id),
    };
    let mut additions = Vec::new();
    for (i, id) in guards.iter().enumerate() {
        additions.push(KirBlock {
            id: *id,
            label: "peel_invariant_guard".into(),
            params: Vec::new(),
            memory_params: Vec::new(),
            instructions: Vec::new(),
            terminator: KirTerminator::Branch {
                condition: mapped(shape.invariant_conditions[i], &pre_values),
                then_edge: shape.incoming.clone(),
                else_edge: if let Some(next) = guards.get(i + 1) {
                    KirEdge {
                        target: *next,
                        args: Vec::new(),
                        memory_args: Vec::new(),
                    }
                } else {
                    KirEdge {
                        target: left_id,
                        ..shape.incoming.clone()
                    }
                },
            },
        });
    }
    let entry = if let Some(first) = guards.first() {
        KirEdge {
            target: *first,
            args: Vec::new(),
            memory_args: Vec::new(),
        }
    } else {
        KirEdge {
            target: left_id,
            ..shape.incoming.clone()
        }
    };
    let preheader = function
        .blocks
        .iter_mut()
        .find(|b| b.id == preheader_id)
        .ok_or("missing preheader")?;
    preheader.instructions.extend(extra);
    preheader.terminator = KirTerminator::Branch {
        condition: gate,
        then_edge: entry,
        else_edge: shape.incoming.clone(),
    };
    additions.extend([left, inner_header, body, right]);
    function.blocks.extend(additions);
    let before_units = kir_function_units(original);
    let after_units = kir_function_units(&function);
    *trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|f| f.id == original.id)
        .ok_or("missing trial function")? = function;
    let plan = StencilPeelPlan {
        function: original.id,
        loop_id: candidate.loop_id,
        header: candidate.header,
        pre_state_digest: state.kir_digest(),
        preheader: preheader_id,
        guards,
        interior_header: inner_header_id,
        phases: [left_phase, body_phase, right_phase],
        added_instructions,
        before_units,
        after_units,
    };
    let charge = peel_charge(&plan);
    Ok(PreparedStencilPeel {
        trial,
        plan,
        charge,
    })
}

fn next_effect(function: &KirFunction) -> Result<u32, String> {
    function
        .blocks
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

fn emit_value(
    state: &mut KirVerifiedProgramState,
    out: &mut Vec<KirInstruction>,
    kind: KirInstructionKind,
    ty: MirType,
) -> Result<ValueId, String> {
    let value = state.fresh_value()?;
    out.push(KirInstruction {
        id: state.fresh_instruction()?,
        results: vec![KirResult {
            value,
            type_node: ty.into(),
        }],
        kind,
        memory: None,
        effect: None,
    });
    Ok(value)
}

fn clone_parameters(
    state: &mut KirVerifiedProgramState,
    source: &KirBlock,
    id: BlockId,
    label: &str,
) -> Result<KirBlock, String> {
    let mut result = source.clone();
    result.id = id;
    result.label = label.into();
    result.instructions.clear();
    for p in &mut result.params {
        p.value = state.fresh_value()?;
    }
    for p in &mut result.memory_params {
        p.version = state.fresh_memory_version()?;
    }
    Ok(result)
}

fn clone_path(
    state: &mut KirVerifiedProgramState,
    function: &KirFunction,
    header: &KirBlock,
    path: &Path,
    id: BlockId,
    label: &str,
    effect: &mut u32,
) -> Result<(KirBlock, KirEdge, StencilPeelPhase), String> {
    let mut result = clone_parameters(state, header, id, label)?;
    let mut values = header
        .params
        .iter()
        .zip(&result.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    let mut memories = header
        .memory_params
        .iter()
        .zip(&result.memory_params)
        .map(|(a, b)| (a.version, b.version))
        .collect::<Memories>();
    let mut mapping = Vec::new();
    for (index, block_id) in path.blocks.iter().enumerate() {
        let source = block(function, *block_id).ok_or("source path block missing")?;
        bind_edge(source, &path.edges[index], &mut values, &mut memories)?;
        for instruction in &source.instructions {
            let mut copy = instruction.clone();
            copy.id = state.fresh_instruction()?;
            remap_instruction(&mut copy, &values)?;
            for r in &mut copy.results {
                let fresh = state.fresh_value()?;
                values.insert(r.value, fresh);
                r.value = fresh;
            }
            if let Some(m) = &mut copy.memory {
                m.input = memories.get(&m.input).copied().unwrap_or(m.input);
                if let Some(old) = m.output {
                    let fresh = state.fresh_memory_version()?;
                    memories.insert(old, fresh);
                    m.output = Some(fresh);
                }
            }
            if let Some(e) = &mut copy.effect {
                e.order = *effect;
                *effect = effect.checked_add(1).ok_or("effect identity exhausted")?;
            }
            mapping.push((instruction.id, copy.id));
            result.instructions.push(copy);
        }
    }
    let edge = map_edge(
        path.edges.last().ok_or("empty source path")?,
        &values,
        &memories,
    );
    Ok((
        result,
        edge,
        StencilPeelPhase {
            block: id,
            source_blocks: path.blocks.clone(),
            instruction_mapping: mapping,
        },
    ))
}

fn bind_edge(
    target: &KirBlock,
    edge: &KirEdge,
    values: &mut Values,
    memories: &mut Memories,
) -> Result<(), String> {
    if edge.target != target.id
        || edge.args.len() != target.params.len()
        || edge.memory_args.len() != target.memory_params.len()
    {
        return Err("invalid path edge".into());
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
fn mapped(v: ValueId, values: &Values) -> ValueId {
    values.get(&v).copied().unwrap_or(v)
}
fn map_edge(edge: &KirEdge, values: &Values, memories: &Memories) -> KirEdge {
    KirEdge {
        target: edge.target,
        args: edge.args.iter().map(|v| mapped(*v, values)).collect(),
        memory_args: edge
            .memory_args
            .iter()
            .map(|v| memories.get(v).copied().unwrap_or(*v))
            .collect(),
    }
}
fn block_edge(block: &KirBlock, target: BlockId) -> KirEdge {
    KirEdge {
        target,
        args: block.params.iter().map(|p| p.value).collect(),
        memory_args: block.memory_params.iter().map(|p| p.version).collect(),
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
        KirInstructionKind::SliceData { slice } | KirInstructionKind::SliceLen { slice } => {
            remap(slice)
        }
        _ => return Err("unsupported scalar peel instruction".into()),
    }
    Ok(())
}
fn remap_place(place: &mut KirPlace, values: &Values) {
    match place {
        KirPlace::Value { value, .. } => *value = mapped(*value, values),
        KirPlace::Deref { pointer, .. } => *pointer = mapped(*pointer, values),
        KirPlace::Index { base, index, .. } => {
            remap_place(base, values);
            *index = mapped(*index, values);
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            *slice = mapped(*slice, values);
            *index = mapped(*index, values);
        }
        KirPlace::Field { base, .. } => remap_place(base, values),
    }
}
fn peel_charge(plan: &StencilPeelPlan) -> CandidateBudgetCharge {
    CandidateBudgetCharge::single(
        plan.function,
        plan.after_units
            .saturating_sub(plan.before_units)
            .saturating_add(16),
        plan.before_units
            .saturating_add(plan.after_units)
            .saturating_add(32),
    )
}

pub fn check_stencil_peel_independently(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &StencilPeelPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_peel(pre, trial, plan, charge).map_err(TransactionCheckError::compiler)
}

fn check_peel(
    pre: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &StencilPeelPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), String> {
    if pre.module().config.consumer != crate::KirConsumer::WebAssembly {
        return Err("checker consumer is not WebAssembly".into());
    }
    if plan.pre_state_digest != pre.kir_digest()
        || pre.contract_facts() != trial.contract_facts()
        || pre.proofs() != trial.proofs()
        || pre.eliminated_guards() != trial.eliminated_guards()
        || pre.evidence_generation() != trial.evidence_generation()
        || pre.optimization_entry_module_units() != trial.optimization_entry_module_units()
    {
        return Err("peel source/evidence identity changed".into());
    }
    let original = pre
        .module()
        .functions
        .iter()
        .find(|f| f.id == plan.function)
        .ok_or("peel source function missing")?;
    let transformed = trial
        .module()
        .functions
        .iter()
        .find(|f| f.id == plan.function)
        .ok_or("peel trial function missing")?;
    let mut unchanged_module = trial.module().clone();
    *unchanged_module
        .functions
        .iter_mut()
        .find(|f| f.id == plan.function)
        .ok_or("missing function")? = original.clone();
    if &unchanged_module != pre.module() {
        return Err("peel changed another function or module metadata".into());
    }
    let mut unchanged_function = transformed.clone();
    unchanged_function.blocks = original.blocks.clone();
    if &unchanged_function != original {
        return Err("peel changed function metadata".into());
    }
    let descriptor = analyze_canonical_loops(original)
        .loops
        .into_iter()
        .find(|l| l.id == plan.loop_id && l.header == plan.header)
        .ok_or("peel source loop is stale")?;
    let shape = reconstruct_partition_for_checker(original, &descriptor, plan)?;
    let header = block(original, plan.header).ok_or("peel header missing")?;
    if shape.descriptor.preheader != Some(plan.preheader)
        || plan.guards.len() != shape.invariant_conditions.len()
    {
        return Err("peel preheader/invariant coverage is false".into());
    }
    let added_ids = plan
        .guards
        .iter()
        .copied()
        .chain([
            plan.phases[0].block,
            plan.interior_header,
            plan.phases[1].block,
            plan.phases[2].block,
        ])
        .collect::<Vec<_>>();
    if added_ids.iter().copied().collect::<BTreeSet<_>>().len() != added_ids.len()
        || added_ids.iter().any(|id| block(original, *id).is_some())
        || transformed.blocks.len() != original.blocks.len() + added_ids.len()
        || transformed.blocks[original.blocks.len()..]
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>()
            != added_ids
    {
        return Err("peel new-block coverage is false".into());
    }
    for (before, after) in original.blocks.iter().zip(&transformed.blocks) {
        if before.id != plan.preheader && before != after {
            return Err("peel changed original scalar fallback".into());
        }
        if before.id != after.id {
            return Err("peel reordered original blocks".into());
        }
    }
    let preheader = block(original, plan.preheader).ok_or("source preheader missing")?;
    let dispatch = block(transformed, plan.preheader).ok_or("trial preheader missing")?;
    if preheader.params != dispatch.params
        || preheader.memory_params != dispatch.memory_params
        || preheader.label != dispatch.label
        || dispatch.instructions.len() != preheader.instructions.len() + 4
        || dispatch.instructions[..preheader.instructions.len()] != preheader.instructions
    {
        return Err("peel changed original preheader evaluation".into());
    }
    let extra = &dispatch.instructions[preheader.instructions.len()..];
    let pre_values = header
        .params
        .iter()
        .zip(&shape.incoming.args)
        .map(|(p, v)| (p.value, *v))
        .collect::<Values>();
    let width = mapped(shape.width, &pre_values);
    let one = checked_result(&extra[0], &u32_type())?;
    let three = checked_result(&extra[1], &u32_type())?;
    let last = checked_result(&extra[2], &u32_type())?;
    let gate = checked_result(&extra[3], &bool_type())?;
    let kinds = [
        KirInstructionKind::ConstInt { value: "1".into() },
        KirInstructionKind::ConstInt { value: "3".into() },
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: width,
            right: one,
            semantics: KirArithmeticSemantics::Modular,
        },
        KirInstructionKind::Compare {
            op: MirCompareOp::Ge,
            left: width,
            right: three,
        },
    ];
    for (index, instruction) in extra.iter().enumerate() {
        if instruction.id != plan.added_instructions[index]
            || instruction.kind != kinds[index]
            || instruction.memory.is_some()
            || instruction.effect.is_some()
        {
            return Err("peel dimension gate or last-column calculation is false".into());
        }
    }
    let fast_entry = if let Some(first) = plan.guards.first() {
        KirEdge {
            target: *first,
            args: Vec::new(),
            memory_args: Vec::new(),
        }
    } else {
        KirEdge {
            target: plan.phases[0].block,
            ..shape.incoming.clone()
        }
    };
    if dispatch.terminator
        != (KirTerminator::Branch {
            condition: gate,
            then_edge: fast_entry,
            else_edge: shape.incoming.clone(),
        })
    {
        return Err("peel small-width fallback is false".into());
    }
    for (index, id) in plan.guards.iter().enumerate() {
        let guard = block(transformed, *id).ok_or("peel invariant guard missing")?;
        let next = if let Some(next) = plan.guards.get(index + 1) {
            KirEdge {
                target: *next,
                args: Vec::new(),
                memory_args: Vec::new(),
            }
        } else {
            KirEdge {
                target: plan.phases[0].block,
                ..shape.incoming.clone()
            }
        };
        if !guard.params.is_empty()
            || !guard.memory_params.is_empty()
            || !guard.instructions.is_empty()
            || guard.terminator
                != (KirTerminator::Branch {
                    condition: mapped(shape.invariant_conditions[index], &pre_values),
                    then_edge: shape.incoming.clone(),
                    else_edge: next,
                })
        {
            return Err("peel invariant fallback is false".into());
        }
    }
    let mut effect = next_effect(original)?;
    for phase in 0..3 {
        let actual = block(transformed, plan.phases[phase].block).ok_or("peel phase missing")?;
        let edge = verify_phase(
            original,
            header,
            &shape.paths[phase],
            actual,
            &plan.phases[phase],
            &mut effect,
        )?;
        let expected = if phase < 2 {
            KirEdge {
                target: plan.interior_header,
                ..edge
            }
        } else {
            let values = header
                .params
                .iter()
                .zip(&edge.args)
                .map(|(p, v)| (p.value, *v))
                .collect();
            let memories = header
                .memory_params
                .iter()
                .zip(&edge.memory_args)
                .map(|(p, v)| (p.version, *v))
                .collect();
            map_edge(&shape.outgoing, &values, &memories)
        };
        if actual.terminator != (KirTerminator::Jump { edge: expected }) {
            return Err("peel phase continuation or exit state is false".into());
        }
    }
    let inner = block(transformed, plan.interior_header).ok_or("peel interior header missing")?;
    check_parameter_shape(header, inner)?;
    if inner.instructions.len() != 1 {
        return Err("peel interior header has extra operations".into());
    }
    let condition = checked_result(&inner.instructions[0], &bool_type())?;
    if inner.instructions[0].kind
        != (KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: inner.params[shape.induction_index].value,
            right: last,
        })
        || inner.instructions[0].effect.is_some()
        || inner.instructions[0].memory.is_some()
        || inner.terminator
            != (KirTerminator::Branch {
                condition,
                then_edge: block_edge(inner, plan.phases[1].block),
                else_edge: block_edge(inner, plan.phases[2].block),
            })
    {
        return Err("peel interior partition overlaps or omits a column".into());
    }
    if plan.before_units != kir_function_units(original)
        || plan.after_units != kir_function_units(transformed)
        || plan.after_units.saturating_sub(plan.before_units) > 192
        || charge != &peel_charge(plan)
    {
        return Err("peel growth/budget evidence is false".into());
    }
    let validation = crate::validate_kir_module(trial.module());
    if !validation.errors.is_empty() {
        return Err(format!(
            "peel structural verification failed: {:?}",
            validation.errors
        ));
    }
    Ok(())
}

fn u32_type() -> KirValueType {
    MirType::Primitive(crate::MirPrimitiveTypeName::U32).into()
}
fn bool_type() -> KirValueType {
    MirType::Primitive(crate::MirPrimitiveTypeName::Bool).into()
}
fn checked_result(instruction: &KirInstruction, ty: &KirValueType) -> Result<ValueId, String> {
    if instruction.results.len() != 1 || &instruction.results[0].type_node != ty {
        return Err("peel scalar result type is false".into());
    }
    Ok(instruction.results[0].value)
}
fn check_parameter_shape(header: &KirBlock, actual: &KirBlock) -> Result<(), String> {
    if header.params.len() != actual.params.len()
        || header.memory_params.len() != actual.memory_params.len()
        || header
            .params
            .iter()
            .zip(&actual.params)
            .any(|(a, b)| a.type_node != b.type_node || a.slot != b.slot)
        || header
            .memory_params
            .iter()
            .zip(&actual.memory_params)
            .any(|(a, b)| a.region != b.region)
    {
        return Err("peel scalar/memory parameter shape is false".into());
    }
    Ok(())
}

// Verify each source instruction against the supplied trial, using edge arguments
// as simultaneous substitutions. This does not invoke the materializer, allocate
// identities or trust the proposed path/instruction mapping.
fn verify_phase(
    function: &KirFunction,
    header: &KirBlock,
    path: &Path,
    actual: &KirBlock,
    plan: &StencilPeelPhase,
    effect: &mut u32,
) -> Result<KirEdge, String> {
    check_parameter_shape(header, actual)?;
    if plan.source_blocks != path.blocks {
        return Err("peel source path mapping is false".into());
    }
    let source_count = path
        .blocks
        .iter()
        .filter_map(|id| block(function, *id))
        .map(|b| b.instructions.len())
        .sum::<usize>();
    if actual.instructions.len() != source_count || plan.instruction_mapping.len() != source_count {
        return Err("peel source instruction coverage is false".into());
    }
    let mut values = header
        .params
        .iter()
        .zip(&actual.params)
        .map(|(a, b)| (a.value, b.value))
        .collect::<Values>();
    let mut memories = header
        .memory_params
        .iter()
        .zip(&actual.memory_params)
        .map(|(a, b)| (a.version, b.version))
        .collect::<Memories>();
    let mut position = 0;
    for (step, block_id) in path.blocks.iter().enumerate() {
        let source = block(function, *block_id).ok_or("peel source path disappeared")?;
        bind_edge(source, &path.edges[step], &mut values, &mut memories)?;
        for before in &source.instructions {
            let after = &actual.instructions[position];
            if plan.instruction_mapping[position] != (before.id, after.id)
                || before.results.len() != after.results.len()
            {
                return Err("peel source instruction order is false".into());
            }
            let mut expected = before.clone();
            expected.id = after.id;
            remap_instruction(&mut expected, &values)?;
            for (result, replacement) in expected.results.iter_mut().zip(&after.results) {
                if result.type_node != replacement.type_node {
                    return Err("peel cloned value type changed".into());
                }
                values.insert(result.value, replacement.value);
                result.value = replacement.value;
            }
            if let Some(memory) = &mut expected.memory {
                let actual_memory = after.memory.as_ref().ok_or("peel cloned memory missing")?;
                memory.input = memories.get(&memory.input).copied().unwrap_or(memory.input);
                if let Some(source_output) = memory.output {
                    let output = actual_memory
                        .output
                        .ok_or("peel cloned memory output missing")?;
                    memories.insert(source_output, output);
                    memory.output = Some(output);
                }
            }
            if let Some(ordered) = &mut expected.effect {
                ordered.order = *effect;
                *effect = effect.checked_add(1).ok_or("effect identity exhausted")?;
            }
            if &expected != after {
                return Err(
                    "peel changed scalar operation, trap order, memory state or floating semantics"
                        .into(),
                );
            }
            position += 1;
        }
    }
    Ok(map_edge(
        path.edges.last().ok_or("peel path has no backedge")?,
        &values,
        &memories,
    ))
}

// This source checker deliberately has its own abstract evaluation and CFG walk.
// The proposer may suggest block IDs, but cannot supply truth values, induction
// facts, invariant assumptions or chosen edges to this proof.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CheckedExpression {
    Input(ValueId),
    Column(u32),
    Width,
    Integer(u32),
    Boolean(bool),
    Unavailable,
}

fn reconstruct_partition_for_checker(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    plan: &StencilPeelPlan,
) -> Result<Shape, String> {
    if !descriptor.innermost
        || !descriptor.lcssa
        || descriptor.exits.len() != 1
        || descriptor.blocks.len() < 4
        || descriptor.blocks.len() > 20
        || !function.vector_regions.is_empty()
    {
        return Err("checker cannot prove a closed scalar inner loop".into());
    }
    let header = block(function, plan.header).ok_or("checker source header missing")?;
    let preheader = block(function, plan.preheader).ok_or("checker source preheader missing")?;
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return Err("checker requires one scalar entry edge".into());
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return Err("checker requires a scalar range branch".into());
    };
    if incoming.target != header.id
        || incoming.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || header.instructions.len() != 1
        || descriptor.blocks.contains(&else_edge.target)
        || !descriptor.blocks.contains(&then_edge.target)
    {
        return Err("checker source loop entry/header/exit shape is false".into());
    }
    let comparison = &header.instructions[0];
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: column,
        right: width,
    } = comparison.kind
    else {
        return Err("checker requires a strict unsigned column bound".into());
    };
    let induction_index = header
        .params
        .iter()
        .position(|p| p.value == column && p.type_node == u32_type())
        .ok_or("checker column is not u32")?;
    if !header
        .params
        .iter()
        .any(|p| p.value == width && p.type_node == u32_type())
        || comparison.results.len() != 1
        || comparison.results[0].value != *condition
        || comparison.effect.is_some()
        || comparison.memory.is_some()
    {
        return Err("checker source comparison has extra semantics".into());
    }
    let definitions = function
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .flat_map(|i| i.results.iter().map(move |r| (r.value, i)))
        .collect::<BTreeMap<_, _>>();
    // Prove the incoming zero directly, independently of the induction analysis.
    let mut start = incoming.args[induction_index];
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(start) || visited.len() > 16 {
            return Err("checker entry column is not a finite zero expression".into());
        }
        match definitions.get(&start).map(|i| &i.kind) {
            Some(KirInstructionKind::Copy { value }) => start = *value,
            Some(KirInstructionKind::ConstInt { value }) if value.parse::<u32>() == Ok(0) => break,
            _ => return Err("checker entry column is not zero".into()),
        }
    }
    let mut loop_values = BTreeSet::new();
    let mut bool_values = BTreeSet::new();
    for param in &function.params {
        if param.type_node == MirType::Primitive(crate::MirPrimitiveTypeName::Bool) {
            bool_values.insert(param.value);
        }
    }
    for b in &function.blocks {
        for p in &b.params {
            if descriptor.blocks.contains(&b.id) {
                loop_values.insert(p.value);
            }
            if p.type_node == bool_type() {
                bool_values.insert(p.value);
            }
        }
        for i in &b.instructions {
            for r in &i.results {
                if descriptor.blocks.contains(&b.id) {
                    loop_values.insert(r.value);
                }
                if r.type_node == bool_type() {
                    bool_values.insert(r.value);
                }
            }
        }
    }
    let mut assumptions = BTreeSet::new();
    let mut verified_paths = Vec::new();
    let mut accesses = Vec::new();
    for (phase, mapping) in plan.phases.iter().enumerate() {
        if mapping.source_blocks.is_empty() || mapping.source_blocks.len() > 20 {
            return Err("checker source path budget exceeded".into());
        }
        let mut expressions = header
            .params
            .iter()
            .map(|p| {
                (
                    p.value,
                    if p.value == column {
                        CheckedExpression::Column(0)
                    } else if p.value == width {
                        CheckedExpression::Width
                    } else {
                        CheckedExpression::Input(p.value)
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut edge = then_edge.clone();
        let mut edges = vec![edge.clone()];
        let mut seen = BTreeSet::new();
        let mut stores = Vec::new();
        let mut loads = 0;
        let mut count = 0;
        for id in &mapping.source_blocks {
            if edge.target != *id
                || !descriptor.blocks.contains(id)
                || *id == header.id
                || !seen.insert(*id)
            {
                return Err("checker path does not follow the original CFG".into());
            }
            let current = block(function, *id).ok_or("checker source block absent")?;
            if current.params.len() != edge.args.len() {
                return Err("checker source phi arity differs".into());
            }
            let incoming_values = edge
                .args
                .iter()
                .map(|v| checker_expression(*v, &expressions, &definitions, &loop_values))
                .collect::<Vec<_>>();
            for (p, e) in current.params.iter().zip(incoming_values) {
                expressions.insert(p.value, e);
            }
            for instruction in &current.instructions {
                count += 1;
                if count > 96 {
                    return Err("checker source instruction budget exceeded".into());
                }
                let get = |v| checker_expression(v, &expressions, &definitions, &loop_values);
                let result = match &instruction.kind {
                    KirInstructionKind::ConstInt { value } => value
                        .parse::<u32>()
                        .ok()
                        .map(CheckedExpression::Integer)
                        .unwrap_or(CheckedExpression::Unavailable),
                    KirInstructionKind::ConstBool { value } => CheckedExpression::Boolean(*value),
                    KirInstructionKind::Copy { value } => get(*value),
                    KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left,
                        right,
                        semantics: KirArithmeticSemantics::Modular,
                    } => match (get(*left), get(*right)) {
                        (CheckedExpression::Column(offset), CheckedExpression::Integer(n))
                        | (CheckedExpression::Integer(n), CheckedExpression::Column(offset)) => {
                            CheckedExpression::Column(offset.wrapping_add(n))
                        }
                        _ => CheckedExpression::Unavailable,
                    },
                    KirInstructionKind::Compare {
                        op: MirCompareOp::Eq,
                        left,
                        right,
                    } => {
                        let a = get(*left);
                        let b = get(*right);
                        let truth = match (&a, &b) {
                            (CheckedExpression::Column(0), CheckedExpression::Integer(0))
                            | (CheckedExpression::Integer(0), CheckedExpression::Column(0)) => {
                                Some(phase == 0)
                            }
                            (CheckedExpression::Column(1), CheckedExpression::Width)
                            | (CheckedExpression::Width, CheckedExpression::Column(1)) => {
                                Some(phase == 2)
                            }
                            _ => None,
                        };
                        truth
                            .map(CheckedExpression::Boolean)
                            .unwrap_or(CheckedExpression::Unavailable)
                    }
                    KirInstructionKind::Load { .. } => {
                        loads += 1;
                        CheckedExpression::Unavailable
                    }
                    KirInstructionKind::Store { .. } => {
                        stores.push(instruction.id);
                        CheckedExpression::Unavailable
                    }
                    KirInstructionKind::ConstFloat { .. }
                    | KirInstructionKind::Binary { .. }
                    | KirInstructionKind::Unary { .. }
                    | KirInstructionKind::Compare { .. }
                    | KirInstructionKind::Cast { .. }
                    | KirInstructionKind::CheckCondition { .. }
                    | KirInstructionKind::Guard { .. }
                    | KirInstructionKind::SliceLen { .. }
                    | KirInstructionKind::SliceData { .. } => CheckedExpression::Unavailable,
                    _ => return Err("checker refuses unsupported scalar effects".into()),
                };
                for output in &instruction.results {
                    expressions.insert(output.value, result.clone());
                }
            }
            edge = match &current.terminator {
                KirTerminator::Jump { edge } => edge.clone(),
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => {
                    let truth = match checker_expression(
                        *condition,
                        &expressions,
                        &definitions,
                        &loop_values,
                    ) {
                        CheckedExpression::Boolean(value) => value,
                        CheckedExpression::Input(value)
                            if bool_values.contains(&value)
                                && (!loop_values.contains(&value)
                                    || header.params.iter().any(|p| p.value == value)) =>
                        {
                            assumptions.insert(value);
                            false
                        }
                        _ => return Err(
                            "checker cannot prove a branch constant throughout this column domain"
                                .into(),
                        ),
                    };
                    if truth {
                        then_edge.clone()
                    } else {
                        else_edge.clone()
                    }
                }
                KirTerminator::Return { .. } => {
                    return Err("checker source path has an early return".into());
                }
            };
            edges.push(edge.clone());
        }
        if edge.target != header.id || edge.args.len() != header.params.len() {
            return Err("checker path has no complete scalar iteration".into());
        }
        for (param, argument) in header.params.iter().zip(&edge.args) {
            let expected = if param.value == column {
                CheckedExpression::Column(1)
            } else if param.value == width {
                CheckedExpression::Width
            } else {
                CheckedExpression::Input(param.value)
            };
            if checker_expression(*argument, &expressions, &definitions, &loop_values) != expected {
                return Err("checker source induction or invariant state changes".into());
            }
        }
        accesses.push((stores, loads));
        verified_paths.push(Path {
            blocks: mapping.source_blocks.clone(),
            edges,
        });
    }
    if assumptions.len() > 2
        || accesses[0].0.len() != 1
        || accesses[0].1 != 0
        || accesses[2] != accesses[0]
        || accesses[1].0.len() != 1
        || accesses[1].0 == accesses[0].0
        || accesses[1].1 == 0
    {
        return Err("checker source boundary/interior coverage is false".into());
    }
    let paths: [Path; 3] = verified_paths
        .try_into()
        .map_err(|_| "checker needs three column domains")?;
    Ok(Shape {
        descriptor: descriptor.clone(),
        induction_index,
        width,
        incoming: incoming.clone(),
        outgoing: else_edge.clone(),
        invariant_conditions: assumptions.into_iter().collect(),
        paths,
    })
}

fn checker_expression(
    value: ValueId,
    expressions: &BTreeMap<ValueId, CheckedExpression>,
    definitions: &BTreeMap<ValueId, &KirInstruction>,
    loop_values: &BTreeSet<ValueId>,
) -> CheckedExpression {
    if let Some(expression) = expressions.get(&value) {
        return expression.clone();
    }
    match definitions.get(&value).map(|i| &i.kind) {
        Some(KirInstructionKind::ConstInt { value }) => value
            .parse::<u32>()
            .ok()
            .map(CheckedExpression::Integer)
            .unwrap_or(CheckedExpression::Unavailable),
        Some(KirInstructionKind::ConstBool { value }) => CheckedExpression::Boolean(*value),
        _ if !loop_values.contains(&value) => CheckedExpression::Input(value),
        _ => CheckedExpression::Unavailable,
    }
}
