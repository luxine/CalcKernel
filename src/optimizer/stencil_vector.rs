//! Source-only proof of a strict nine-load affine stencil and its total range envelope.
use crate::{
    BlockId, CanonicalLoopDescriptor, FunctionId, InstructionId, KirArithmeticSemantics, KirBlock,
    KirEdge, KirFunction, KirInstruction, KirInstructionKind, KirPlace, KirTerminator,
    KirValueType, KirVerifiedProgramState, LoopId, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName,
    MirType, ValueId,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StencilRangeCount {
    Width,
    ThreeWidths,
    InteriorTrip,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StencilRangeRequirement {
    pub slice: ValueId,
    pub start: Option<ValueId>,
    pub count: StencilRangeCount,
    pub element_bytes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StencilLoadPosition {
    pub instruction: InstructionId,
    pub origin: ValueId,
    pub row: i8,
    pub column: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmStencilSource {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub preheader: BlockId,
    pub header: BlockId,
    pub body: BlockId,
    pub induction: ValueId,
    pub body_induction: ValueId,
    pub bound: ValueId,
    pub width: ValueId,
    pub row_base: ValueId,
    pub input: ValueId,
    pub output: ValueId,
    pub loads: Vec<StencilLoadPosition>,
    pub store: InstructionId,
    pub store_origin: ValueId,
    pub scalar_address_setup: Vec<InstructionId>,
    pub ranges: Vec<StencilRangeRequirement>,
    pub minimum_trip: u32,
    pub source_digest: String,
}

#[must_use]
pub fn discover_wasm_stencil_sources(state: &KirVerifiedProgramState) -> Vec<WasmStencilSource> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly
        || state.module().profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
    {
        return Vec::new();
    }
    state
        .module()
        .functions
        .iter()
        .flat_map(|function| {
            crate::analyze_canonical_loops(function)
                .loops
                .into_iter()
                .filter_map(|descriptor| propose(state, function, &descriptor))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn block(f: &KirFunction, id: BlockId) -> Option<&KirBlock> {
    f.blocks.iter().find(|b| b.id == id)
}
fn definition(f: &KirFunction, value: ValueId) -> Option<&KirInstruction> {
    f.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .find(|i| i.results.iter().any(|r| r.value == value))
}
fn edges(t: &KirTerminator) -> Vec<&KirEdge> {
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
fn ty(f: &KirFunction, value: ValueId) -> Option<KirValueType> {
    f.params
        .iter()
        .find(|p| p.value == value)
        .map(|p| p.type_node.clone().into())
        .or_else(|| {
            f.blocks.iter().find_map(|b| {
                b.params
                    .iter()
                    .find(|p| p.value == value)
                    .map(|p| p.type_node.clone())
                    .or_else(|| {
                        b.instructions
                            .iter()
                            .flat_map(|i| &i.results)
                            .find(|r| r.value == value)
                            .map(|r| r.type_node.clone())
                    })
            })
        })
}
fn u32_type() -> KirValueType {
    MirType::Primitive(MirPrimitiveTypeName::U32).into()
}

/// Follow *all* forwarding edges. A cycle is harmless only when its closure has
/// exactly one non-forwarding origin; changing phi inputs therefore never pass.
fn proposer_root(f: &KirFunction, value: ValueId) -> Option<ValueId> {
    let mut todo = vec![value];
    let mut seen = BTreeSet::new();
    let mut roots = BTreeSet::new();
    while let Some(value) = todo.pop() {
        if !seen.insert(value) {
            continue;
        }
        if seen.len() > 512 {
            return None;
        }
        if let Some(i) = definition(f, value)
            && let KirInstructionKind::Copy { value } = i.kind
        {
            if i.memory.is_some() || i.effect.is_some() {
                return None;
            }
            todo.push(value);
            continue;
        }
        if let Some((b, index)) = f.blocks.iter().find_map(|b| {
            b.params
                .iter()
                .position(|p| p.value == value)
                .map(|n| (b, n))
        }) {
            let incoming = f
                .blocks
                .iter()
                .flat_map(|p| edges(&p.terminator))
                .filter(|e| e.target == b.id)
                .collect::<Vec<_>>();
            if incoming.is_empty() {
                return None;
            }
            for edge in incoming {
                todo.push(*edge.args.get(index)?);
            }
        } else {
            roots.insert(value);
        }
    }
    (roots.len() == 1).then(|| *roots.first().unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Polynomial {
    constant: u32,
    terms: BTreeMap<ValueId, u32>,
}
impl Polynomial {
    fn atom(value: ValueId) -> Self {
        Self {
            constant: 0,
            terms: [(value, 1)].into(),
        }
    }
    fn add(mut self, rhs: Self, subtract: bool) -> Self {
        self.constant = if subtract {
            self.constant.wrapping_sub(rhs.constant)
        } else {
            self.constant.wrapping_add(rhs.constant)
        };
        for (v, c) in rhs.terms {
            let old = self.terms.get(&v).copied().unwrap_or(0);
            let new = if subtract {
                old.wrapping_sub(c)
            } else {
                old.wrapping_add(c)
            };
            if new == 0 {
                self.terms.remove(&v);
            } else {
                self.terms.insert(v, new);
            }
        }
        self
    }
}
fn proposer_polynomial(f: &KirFunction, value: ValueId, depth: usize) -> Option<Polynomial> {
    if depth > 64 || ty(f, value)? != u32_type() {
        return None;
    }
    let value = proposer_root(f, value)?;
    let Some(i) = definition(f, value) else {
        return Some(Polynomial::atom(value));
    };
    if i.memory.is_some() || i.effect.is_some() || i.results.len() != 1 {
        return None;
    }
    match &i.kind {
        KirInstructionKind::ConstInt { value } => Some(Polynomial {
            constant: value.parse().ok()?,
            terms: BTreeMap::new(),
        }),
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
            Some(proposer_polynomial(f, *left, depth + 1)?.add(
                proposer_polynomial(f, *right, depth + 1)?,
                *op == MirBinaryOp::Sub,
            ))
        }
        _ => Some(Polynomial::atom(value)),
    }
}
fn local_root(value: ValueId, roots: &BTreeMap<ValueId, ValueId>) -> ValueId {
    roots.get(&value).copied().unwrap_or(value)
}
fn invariant_dominates(f: &KirFunction, value: ValueId, preheader: BlockId) -> bool {
    if f.params.iter().any(|p| p.value == value) {
        return true;
    }
    let Some(owner) = f.blocks.iter().find(|b| {
        b.params.iter().any(|p| p.value == value)
            || b.instructions
                .iter()
                .any(|i| i.results.iter().any(|r| r.value == value))
    }) else {
        return false;
    };
    crate::compute_kir_dominators(f).dominates(owner.id, preheader)
}
fn proposer_noalias(
    state: &KirVerifiedProgramState,
    f: &KirFunction,
    preheader: BlockId,
    left: ValueId,
    right: ValueId,
) -> bool {
    let dom = crate::compute_kir_dominators(f);
    state.contract_facts().is_some_and(|c| {
        c.facts().facts().iter().any(|fact| {
            let available = match fact.scope {
                crate::FactScope::FunctionEntry(owner) => owner == f.id,
                crate::FactScope::Block { function, block } => {
                    function == f.id && dom.dominates(block, preheader)
                }
                _ => false,
            };
            available
                && matches!(fact.predicate,
            crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias{left:a,right:b})
            if (proposer_root(f,a)==Some(left)&&proposer_root(f,b)==Some(right))
            || (proposer_root(f,b)==Some(left)&&proposer_root(f,a)==Some(right)))
        })
    })
}
fn propose(
    state: &KirVerifiedProgramState,
    f: &KirFunction,
    d: &CanonicalLoopDescriptor,
) -> Option<WasmStencilSource> {
    if !d.innermost || !d.lcssa || d.blocks.len() != 2 || d.exits.len() != 1 {
        return None;
    }
    let preheader = d.preheader?;
    let p = block(f, preheader)?;
    let h = block(f, d.header)?;
    let KirTerminator::Jump { edge: entry } = &p.terminator else {
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
        || entry.target != h.id
        || !d.blocks.contains(&then_edge.target)
        || d.blocks.contains(&else_edge.target)
    {
        return None;
    }
    let compare = &h.instructions[0];
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: iv,
        right: bound,
    } = compare.kind
    else {
        return None;
    };
    if compare.results.len() != 1
        || compare.results[0].value != *condition
        || compare.effect.is_some()
        || compare.memory.is_some()
        || ty(f, iv)? != u32_type()
        || ty(f, bound)? != u32_type()
    {
        return None;
    }
    let iv_slot = h.params.iter().position(|p| p.value == iv)?;
    if proposer_polynomial(f, *entry.args.get(iv_slot)?, 0)? != Polynomial::default() {
        return None;
    }
    let b = block(f, then_edge.target)?;
    if f.blocks
        .iter()
        .flat_map(|owner| {
            edges(&owner.terminator)
                .into_iter()
                .map(move |edge| (owner.id, edge))
        })
        .any(|(owner, edge)| {
            (edge.target == h.id && owner != p.id && owner != b.id)
                || (edge.target == b.id && owner != h.id)
        })
    {
        return None;
    }
    let KirTerminator::Jump { edge: back } = &b.terminator else {
        return None;
    };
    if back.target != h.id
        || entry.args.len() != h.params.len()
        || back.args.len() != h.params.len()
        || then_edge.args.len() != b.params.len()
        || b.instructions.len() > 128
    {
        return None;
    }
    let mut roots = b
        .params
        .iter()
        .zip(&then_edge.args)
        .map(|(p, v)| (p.value, *v))
        .collect::<BTreeMap<_, _>>();
    for i in &b.instructions {
        if let KirInstructionKind::Copy { value } = i.kind {
            for r in &i.results {
                roots.insert(r.value, local_root(value, &roots));
            }
        }
    }
    let body_iv = b
        .params
        .iter()
        .zip(&then_edge.args)
        .find_map(|(p, v)| (*v == iv).then_some(p.value))?;
    for (n, (v, p)) in back.args.iter().zip(&h.params).enumerate() {
        if n != iv_slot && local_root(*v, &roots) != p.value {
            return None;
        }
    }
    let step = definition(f, back.args[iv_slot])?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = step.kind
    else {
        return None;
    };
    let one = |v| matches!(proposer_polynomial(f,v,0),Some(Polynomial{constant:1,terms}) if terms.is_empty());
    if step.memory.is_some()
        || step.effect.is_some()
        || !((local_root(left, &roots) == iv && one(right))
            || (local_root(right, &roots) == iv && one(left)))
    {
        return None;
    }
    let n = proposer_polynomial(f, bound, 0)?;
    if n.constant != u32::MAX - 1 || n.terms.len() != 1 {
        return None;
    }
    let (&width, &coefficient) = n.terms.first_key_value()?;
    if coefficient != 1 || !invariant_dominates(f, width, preheader) {
        return None;
    }
    let mut accesses = Vec::new();
    for i in &b.instructions {
        let (place, store) = match &i.kind {
            KirInstructionKind::Load { place } => (place, false),
            KirInstructionKind::Store { place, .. } => (place, true),
            KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::ConstFloat { .. }
            | KirInstructionKind::Copy { .. }
            | KirInstructionKind::Compare { .. }
                if i.memory.is_none() && i.effect.is_none() =>
            {
                continue;
            }
            KirInstructionKind::Binary { op, semantics, .. }
                if i.memory.is_none()
                    && i.effect.is_none()
                    && (matches!(semantics, KirArithmeticSemantics::StrictFloat)
                        && matches!(
                            op,
                            MirBinaryOp::Add
                                | MirBinaryOp::Sub
                                | MirBinaryOp::Mul
                                | MirBinaryOp::Div
                        )
                        || matches!(semantics, KirArithmeticSemantics::Modular)
                            && matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub)) =>
            {
                continue;
            }
            _ => return None,
        };
        let KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            ..
        } = place.as_ref()
        else {
            return None;
        };
        if *type_node != MirType::Primitive(MirPrimitiveTypeName::F64) {
            return None;
        }
        let index = definition(f, *index)?;
        let KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } = index.kind
        else {
            return None;
        };
        let origin = if local_root(left, &roots) == iv {
            right
        } else if local_root(right, &roots) == iv {
            left
        } else {
            return None;
        };
        if !invariant_dominates(f, origin, preheader) {
            return None;
        }
        accesses.push((i.id, proposer_root(f, *slice)?, origin, store));
    }
    if accesses.len() != 10 || accesses[..9].iter().any(|a| a.3) || !accesses[9].3 {
        return None;
    }
    let (store, output, store_origin, _) = accesses[9];
    let input = accesses[0].1;
    if accesses[..9].iter().any(|a| a.1 != input)
        || !proposer_noalias(state, f, preheader, input, output)
    {
        return None;
    }
    let output_index = proposer_polynomial(f, store_origin, 0)?;
    if output_index.constant != 1 || output_index.terms.len() != 1 {
        return None;
    }
    let (&row_base, &coefficient) = output_index.terms.first_key_value()?;
    if coefficient != 1 || row_base == width || !invariant_dominates(f, row_base, preheader) {
        return None;
    }
    let mut loads = Vec::new();
    let mut positions = BTreeSet::new();
    for (instruction, _, origin, _) in &accesses[..9] {
        let polynomial = proposer_polynomial(f, *origin, 0)?;
        let mut position = None;
        for row in -1_i8..=1 {
            for column in 0..=2 {
                let mut expected = Polynomial::atom(row_base);
                expected.constant = column;
                if row != 0 {
                    expected.terms.insert(width, row as u32);
                }
                if expected == polynomial {
                    position = Some((row, column as u8));
                }
            }
        }
        let (row, column) = position?;
        if !positions.insert((row, column)) {
            return None;
        }
        loads.push(StencilLoadPosition {
            instruction: *instruction,
            origin: *origin,
            row,
            column,
        });
    }
    let top = loads.iter().find(|p| p.row == -1 && p.column == 0)?.origin;
    let scalar_address_setup = b
        .instructions
        .iter()
        .filter(|i| {
            i.id != step.id
                && !matches!(i.kind, KirInstructionKind::Copy { .. })
                && !i.results.is_empty()
                && i.results.iter().all(|r| {
                    r.type_node == u32_type()
                        || r.type_node == MirType::Primitive(MirPrimitiveTypeName::Bool).into()
                })
        })
        .map(|i| i.id)
        .collect::<Vec<_>>();
    Some(WasmStencilSource {
        function: f.id,
        loop_id: d.id,
        preheader,
        header: h.id,
        body: b.id,
        induction: iv,
        body_induction: body_iv,
        bound,
        width,
        row_base,
        input,
        output,
        loads,
        store,
        store_origin,
        scalar_address_setup,
        ranges: vec![
            StencilRangeRequirement {
                slice: input,
                start: None,
                count: StencilRangeCount::Width,
                element_bytes: 8,
            },
            StencilRangeRequirement {
                slice: input,
                start: Some(top),
                count: StencilRangeCount::ThreeWidths,
                element_bytes: 8,
            },
            StencilRangeRequirement {
                slice: output,
                start: Some(store_origin),
                count: StencilRangeCount::InteriorTrip,
                element_bytes: 8,
            },
        ],
        minimum_trip: 4,
        source_digest: state.kir_digest(),
    })
}

/// Independently validate the source and the range recipe. This verifier does
/// not use the discovery recognizer, its forwarding solver, or its polynomial
/// evaluator. Emitted-vector verification must additionally preserve the exact
/// source instruction/effect order and the scalar fallback.
pub fn check_wasm_stencil_source_independently(
    state: &KirVerifiedProgramState,
    plan: &WasmStencilSource,
) -> Result<(), String> {
    if state.module().config.consumer != crate::KirConsumer::WebAssembly
        || state.module().profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
        || plan.source_digest != state.kir_digest()
        || plan.minimum_trip != 4
    {
        return Err("stencil source profile, snapshot or trip threshold changed".into());
    }
    let f = state
        .module()
        .functions
        .iter()
        .find(|f| f.id == plan.function)
        .ok_or("missing stencil function")?;
    let h = block(f, plan.header).ok_or("missing stencil header")?;
    let b = block(f, plan.body).ok_or("missing stencil body")?;
    let p = block(f, plan.preheader).ok_or("missing stencil preheader")?;
    let descriptor = crate::analyze_canonical_loops(f)
        .loops
        .into_iter()
        .find(|d| d.header == h.id)
        .ok_or("missing source loop")?;
    if descriptor.id != plan.loop_id
        || !descriptor.innermost
        || !descriptor.lcssa
        || descriptor.blocks.iter().copied().collect::<BTreeSet<_>>() != [h.id, b.id].into()
        || descriptor.preheader != Some(p.id)
        || descriptor.exits.len() != 1
    {
        return Err("source is not the claimed two-block closed loop".into());
    }
    let KirTerminator::Jump { edge: entry } = &p.terminator else {
        return Err("non-jump source entry".into());
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &h.terminator
    else {
        return Err("non-branch source header".into());
    };
    let KirTerminator::Jump { edge: back } = &b.terminator else {
        return Err("non-jump source latch".into());
    };
    if entry.target != h.id
        || back.target != h.id
        || then_edge.target != b.id
        || [h.id, b.id].contains(&else_edge.target)
        || entry.args.len() != h.params.len()
        || back.args.len() != h.params.len()
        || then_edge.args.len() != b.params.len()
        || h.instructions.len() != 1
        || b.instructions.len() > 128
    {
        return Err("source CFG or edge arity changed".into());
    }
    for owner in &f.blocks {
        for edge in edges(&owner.terminator) {
            if (edge.target == h.id && owner.id != p.id && owner.id != b.id)
                || (edge.target == b.id && owner.id != h.id)
            {
                return Err("source has an unproved incoming path".into());
            }
        }
    }
    let test = &h.instructions[0];
    if test.kind
        != (KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: plan.induction,
            right: plan.bound,
        })
        || test.results.len() != 1
        || test.results[0].value != *condition
        || test.effect.is_some()
        || test.memory.is_some()
        || ty(f, plan.induction) != Some(u32_type())
        || ty(f, plan.bound) != Some(u32_type())
    {
        return Err("source is not a pure strict u32 induction comparison".into());
    }
    let slot = h
        .params
        .iter()
        .position(|p| p.value == plan.induction)
        .ok_or("induction is not a header parameter")?;
    if !checker_equation(f, entry.args[slot], 0, &[]) {
        return Err("source induction does not start at zero on every path".into());
    }
    let mut aliases = BTreeMap::new();
    for (parameter, value) in b.params.iter().zip(&then_edge.args) {
        aliases.insert(parameter.value, *value);
    }
    for instruction in &b.instructions {
        if let KirInstructionKind::Copy { value } = instruction.kind {
            if instruction.results.len() != 1
                || instruction.effect.is_some()
                || instruction.memory.is_some()
            {
                return Err("invalid copy".into());
            }
            let root = aliases.get(&value).copied().unwrap_or(value);
            aliases.insert(instruction.results[0].value, root);
        }
    }
    let resolve = |v| aliases.get(&v).copied().unwrap_or(v);
    if resolve(plan.body_induction) != plan.induction
        || !b.params.iter().any(|p| p.value == plan.body_induction)
    {
        return Err("body induction is not the header induction".into());
    }
    for (index, parameter) in h.params.iter().enumerate() {
        if index != slot && resolve(back.args[index]) != parameter.value {
            return Err("non-induction source state changes on the backedge".into());
        }
    }
    let increment = definition(f, back.args[slot]).ok_or("missing induction increment")?;
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = increment.kind
    else {
        return Err("induction increment is not modular add".into());
    };
    if increment.effect.is_some()
        || increment.memory.is_some()
        || !((resolve(left) == plan.induction && checker_equation(f, right, 1, &[]))
            || (resolve(right) == plan.induction && checker_equation(f, left, 1, &[])))
    {
        return Err("induction step is not exactly one".into());
    }
    let dominators = crate::compute_kir_dominators(f);
    let available = |value| {
        f.params.iter().any(|p| p.value == value)
            || f.blocks.iter().any(|owner| {
                dominators.dominates(owner.id, p.id)
                    && (owner.params.iter().any(|p| p.value == value)
                        || owner
                            .instructions
                            .iter()
                            .any(|i| i.results.iter().any(|r| r.value == value)))
            })
    };
    if plan.width == plan.row_base
        || !available(plan.width)
        || !available(plan.row_base)
        || !checker_equation(f, plan.bound, u32::MAX - 1, &[(plan.width, 1)])
        || !checker_equation(f, plan.store_origin, 1, &[(plan.row_base, 1)])
    {
        return Err(
            "source trip count or store origin does not match the width/row equations".into(),
        );
    }
    let alias = state.contract_facts().is_some_and(|facts| {
        facts.facts().facts().iter().any(|fact| {
            let scope = match fact.scope {
                crate::FactScope::FunctionEntry(function) => function == f.id,
                crate::FactScope::Block { function, block } => {
                    function == f.id && dominators.dominates(block, p.id)
                }
                _ => false,
            };
            match fact.predicate {
                crate::FactPredicate::Contract(crate::ContractFactPredicate::NoAlias {
                    left,
                    right,
                }) if scope => {
                    (checker_origin(f, left) == Some(plan.input)
                        && checker_origin(f, right) == Some(plan.output))
                        || (checker_origin(f, right) == Some(plan.input)
                            && checker_origin(f, left) == Some(plan.output))
                }
                _ => false,
            }
        })
    });
    if !alias || plan.input == plan.output {
        return Err("stencil input/output lack available noalias evidence".into());
    }
    if plan.loads.len() != 9 {
        return Err("stencil requires nine loads".into());
    }
    let mut memory_position = 0;
    let mut positions = BTreeSet::new();
    for instruction in &b.instructions {
        let (place, is_store) = match &instruction.kind {
            KirInstructionKind::Load { place } => (place, false),
            KirInstructionKind::Store { place, .. } => (place, true),
            KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::ConstFloat { .. }
            | KirInstructionKind::Copy { .. }
            | KirInstructionKind::Compare { .. }
                if instruction.memory.is_none() && instruction.effect.is_none() =>
            {
                continue;
            }
            KirInstructionKind::Binary { op, semantics, .. }
                if instruction.memory.is_none()
                    && instruction.effect.is_none()
                    && ((*semantics == KirArithmeticSemantics::Modular
                        && matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub))
                        || (*semantics == KirArithmeticSemantics::StrictFloat
                            && matches!(
                                op,
                                MirBinaryOp::Add
                                    | MirBinaryOp::Sub
                                    | MirBinaryOp::Mul
                                    | MirBinaryOp::Div
                            ))) =>
            {
                continue;
            }
            _ => return Err("source body contains an unsupported effect or operation".into()),
        };
        let KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            ..
        } = place.as_ref()
        else {
            return Err("source access is not slice indexing".into());
        };
        if *type_node != MirType::Primitive(MirPrimitiveTypeName::F64)
            || instruction.memory.is_none()
            || instruction.effect.is_none()
        {
            return Err("source access type or memory/effect annotation changed".into());
        }
        let expected_origin;
        let expected_slice;
        if memory_position < 9 {
            let load = &plan.loads[memory_position];
            if is_store
                || instruction.id != load.instruction
                || !(-1..=1).contains(&load.row)
                || load.column > 2
                || !positions.insert((load.row, load.column))
            {
                return Err("source load order or grid position changed".into());
            }
            let mut equation = vec![(plan.row_base, 1)];
            if load.row != 0 {
                equation.push((plan.width, load.row as u32));
            }
            if !checker_equation(f, load.origin, u32::from(load.column), &equation) {
                return Err("source load origin is outside the proved grid".into());
            }
            expected_origin = load.origin;
            expected_slice = plan.input;
        } else if memory_position == 9 {
            if !is_store || instruction.id != plan.store {
                return Err("source final effect is not the claimed store".into());
            }
            expected_origin = plan.store_origin;
            expected_slice = plan.output;
        } else {
            return Err("source contains an extra memory effect".into());
        }
        if !available(expected_origin) || checker_origin(f, *slice) != Some(expected_slice) {
            return Err("source origin is not invariant or slice changed".into());
        }
        let address = definition(f, *index).ok_or("missing memory index definition")?;
        let KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } = address.kind
        else {
            return Err("source index is not modular origin plus induction".into());
        };
        if ty(f, *index) != Some(u32_type())
            || address.memory.is_some()
            || address.effect.is_some()
            || !((left == expected_origin && resolve(right) == plan.induction)
                || (right == expected_origin && resolve(left) == plan.induction))
        {
            return Err("source index is not the exact invariant origin plus induction".into());
        }
        memory_position += 1;
    }
    if memory_position != 10 || positions.len() != 9 {
        return Err("source memory envelope is incomplete".into());
    }
    let mut setup = BTreeSet::new();
    for instruction in &b.instructions {
        if instruction.id != increment.id
            && !matches!(instruction.kind, KirInstructionKind::Copy { .. })
            && !instruction.results.is_empty()
            && instruction.results.iter().all(|r| {
                matches!(
                    r.type_node.as_scalar(),
                    Some(MirType::Primitive(
                        MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool
                    ))
                )
            })
        {
            setup.insert(instruction.id);
        }
    }
    if plan
        .scalar_address_setup
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != setup
        || plan.scalar_address_setup.len() != setup.len()
    {
        return Err("scalar address setup coverage differs".into());
    }
    for instruction in b.instructions.iter().filter(|i| setup.contains(&i.id)) {
        if instruction.memory.is_some() || instruction.effect.is_some() {
            return Err("scalar address setup has effects".into());
        }
        for result in &instruction.results {
            for owner in &f.blocks {
                for user in &owner.instructions {
                    let mut uses = 0;
                    crate::optimizer::analysis::visit_instruction_uses(user, &mut |v| {
                        uses += usize::from(v == result.value)
                    });
                    if uses == 0 || setup.contains(&user.id) {
                        continue;
                    }
                    let is_index = matches!(&user.kind,KirInstructionKind::Load{place}|KirInstructionKind::Store{place,..}
                        if matches!(place.as_ref(),KirPlace::SliceIndex{index,..} if *index==result.value));
                    if !is_index || uses != 1 || owner.id != b.id {
                        return Err("scalar address setup has an observed intermediate".into());
                    }
                }
                let escaped = match &owner.terminator {
                    KirTerminator::Return { value, .. } => *value == Some(result.value),
                    KirTerminator::Jump { edge } => edge.args.contains(&result.value),
                    KirTerminator::Branch {
                        condition,
                        then_edge,
                        else_edge,
                    } => {
                        *condition == result.value
                            || then_edge.args.contains(&result.value)
                            || else_edge.args.contains(&result.value)
                    }
                };
                if escaped {
                    return Err("scalar address setup escapes through an edge".into());
                }
            }
        }
    }
    let top = plan
        .loads
        .iter()
        .find(|p| p.row == -1 && p.column == 0)
        .ok_or("missing top-left source origin")?
        .origin;
    let expected = vec![
        StencilRangeRequirement {
            slice: plan.input,
            start: None,
            count: StencilRangeCount::Width,
            element_bytes: 8,
        },
        StencilRangeRequirement {
            slice: plan.input,
            start: Some(top),
            count: StencilRangeCount::ThreeWidths,
            element_bytes: 8,
        },
        StencilRangeRequirement {
            slice: plan.output,
            start: Some(plan.store_origin),
            count: StencilRangeCount::InteriorTrip,
            element_bytes: 8,
        },
    ];
    if plan.ranges != expected {
        return Err("range recipe does not establish the three-row and output envelopes".into());
    }
    Ok(())
}

// The verifier uses recursive forwarding closure and a separate linear worklist
// evaluator. Cycles contribute no terminal; every noncyclic source must agree.
fn checker_origin(f: &KirFunction, value: ValueId) -> Option<ValueId> {
    fn visit(
        f: &KirFunction,
        value: ValueId,
        seen: &mut BTreeSet<ValueId>,
        terminals: &mut BTreeSet<ValueId>,
    ) -> Option<()> {
        if !seen.insert(value) {
            return Some(());
        }
        if seen.len() > 512 {
            return None;
        }
        if let Some(instruction) = definition(f, value)
            && let KirInstructionKind::Copy { value: input } = instruction.kind
        {
            if instruction.results.len() != 1
                || instruction.effect.is_some()
                || instruction.memory.is_some()
            {
                return None;
            }
            return visit(f, input, seen, terminals);
        }
        for owner in &f.blocks {
            for (position, parameter) in owner.params.iter().enumerate() {
                if parameter.value == value {
                    let mut count = 0;
                    for predecessor in &f.blocks {
                        for edge in edges(&predecessor.terminator) {
                            if edge.target == owner.id {
                                count += 1;
                                visit(f, *edge.args.get(position)?, seen, terminals)?;
                            }
                        }
                    }
                    return (count > 0).then_some(());
                }
            }
        }
        terminals.insert(value);
        Some(())
    }
    let mut terminals = BTreeSet::new();
    visit(f, value, &mut BTreeSet::new(), &mut terminals)?;
    if terminals.len() == 1 {
        terminals.into_iter().next()
    } else {
        None
    }
}
fn checker_equation(
    f: &KirFunction,
    value: ValueId,
    constant: u32,
    terms: &[(ValueId, u32)],
) -> bool {
    checker_expression(f, value) == Some((constant, terms.iter().copied().collect()))
}
fn checker_expression(f: &KirFunction, value: ValueId) -> Option<(u32, BTreeMap<ValueId, u32>)> {
    let mut work = vec![(value, 1_u32)];
    let mut actual = BTreeMap::<ValueId, u32>::new();
    let mut sum = 0_u32;
    let mut fuel = 512;
    while let Some((value, coefficient)) = work.pop() {
        if fuel == 0 || ty(f, value) != Some(u32_type()) {
            return None;
        }
        fuel -= 1;
        let value = checker_origin(f, value)?;
        if let Some(instruction) = definition(f, value) {
            if instruction.results.len() != 1
                || instruction.memory.is_some()
                || instruction.effect.is_some()
            {
                return None;
            }
            match &instruction.kind {
                KirInstructionKind::ConstInt { value } => {
                    let Ok(number) = value.parse::<u32>() else {
                        return None;
                    };
                    sum = sum.wrapping_add(coefficient.wrapping_mul(number));
                    continue;
                }
                KirInstructionKind::Binary {
                    op,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                    work.push((*left, coefficient));
                    work.push((
                        *right,
                        if *op == MirBinaryOp::Sub {
                            coefficient.wrapping_neg()
                        } else {
                            coefficient
                        },
                    ));
                    continue;
                }
                _ => {}
            }
        }
        let entry = actual.entry(value).or_default();
        *entry = entry.wrapping_add(coefficient);
    }
    actual.retain(|_, c| *c != 0);
    Some((sum, actual))
}

/// Reconstruct checker-owned source claims without consulting discovery. This
/// is used by vector verification, whose plan contains only concrete ranges.
pub(crate) fn reconstruct_wasm_stencil_source_independently(
    state: &KirVerifiedProgramState,
    function: FunctionId,
    header: BlockId,
) -> Result<WasmStencilSource, String> {
    fn reconstruct(
        state: &KirVerifiedProgramState,
        function: FunctionId,
        header: BlockId,
    ) -> Option<WasmStencilSource> {
        let f = state.module().functions.iter().find(|f| f.id == function)?;
        let descriptor = crate::analyze_canonical_loops(f)
            .loops
            .into_iter()
            .find(|d| d.header == header)?;
        let h = block(f, header)?;
        let KirTerminator::Branch { then_edge, .. } = &h.terminator else {
            return None;
        };
        let KirInstructionKind::Compare {
            left: induction,
            right: bound,
            op: MirCompareOp::Lt,
        } = h.instructions.first()?.kind
        else {
            return None;
        };
        let b = block(f, then_edge.target)?;
        let KirTerminator::Jump { edge: back } = &b.terminator else {
            return None;
        };
        let mut aliases = BTreeMap::new();
        for (param, arg) in b.params.iter().zip(&then_edge.args) {
            aliases.insert(param.value, *arg);
        }
        for i in &b.instructions {
            if let KirInstructionKind::Copy { value } = i.kind {
                let alias = aliases.get(&value).copied().unwrap_or(value);
                for result in &i.results {
                    aliases.insert(result.value, alias);
                }
            }
        }
        let body_induction = b
            .params
            .iter()
            .zip(&then_edge.args)
            .find_map(|(p, v)| (*v == induction).then_some(p.value))?;
        let slot = h.params.iter().position(|p| p.value == induction)?;
        let step = definition(f, *back.args.get(slot)?)?;
        let (constant, terms) = checker_expression(f, bound)?;
        if terms.len() != 1 {
            return None;
        }
        let (&width, &coefficient) = terms.first_key_value()?;
        if coefficient != 1 {
            return None;
        }
        if constant != u32::MAX - 1 {
            return None;
        }
        let mut accesses = Vec::new();
        for instruction in &b.instructions {
            let (place, store) = match &instruction.kind {
                KirInstructionKind::Load { place } => (place, false),
                KirInstructionKind::Store { place, .. } => (place, true),
                _ => continue,
            };
            let KirPlace::SliceIndex { slice, index, .. } = place.as_ref() else {
                return None;
            };
            let KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } = definition(f, *index)?.kind
            else {
                return None;
            };
            let origin = if aliases.get(&left).copied().unwrap_or(left) == induction {
                right
            } else if aliases.get(&right).copied().unwrap_or(right) == induction {
                left
            } else {
                return None;
            };
            accesses.push((instruction.id, checker_origin(f, *slice)?, origin, store));
        }
        if accesses.len() != 10 || accesses[..9].iter().any(|v| v.3) || !accesses[9].3 {
            return None;
        }
        let (store, output, store_origin, _) = accesses[9];
        let input = accesses[0].1;
        let (constant, terms) = checker_expression(f, store_origin)?;
        if terms.len() != 1 {
            return None;
        }
        let (&row_base, &coefficient) = terms.first_key_value()?;
        if coefficient != 1 {
            return None;
        }
        if constant != 1 {
            return None;
        }
        let mut loads = Vec::new();
        for (instruction, _, origin, _) in &accesses[..9] {
            let (column, terms) = checker_expression(f, *origin)?;
            if column > 2 || terms.get(&row_base) != Some(&1) {
                return None;
            }
            let row = match terms.get(&width).copied().unwrap_or(0) {
                0 => 0,
                1 => 1,
                u32::MAX => -1,
                _ => return None,
            };
            if terms.len() != if row == 0 { 1 } else { 2 } {
                return None;
            }
            loads.push(StencilLoadPosition {
                instruction: *instruction,
                origin: *origin,
                row,
                column: column as u8,
            });
        }
        let top = loads.iter().find(|l| l.row == -1 && l.column == 0)?.origin;
        let setup = b
            .instructions
            .iter()
            .filter(|i| {
                i.id != step.id
                    && !i.results.is_empty()
                    && i.results.iter().all(|r| {
                        matches!(
                            r.type_node.as_scalar(),
                            Some(MirType::Primitive(
                                MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool
                            ))
                        )
                    })
            })
            .map(|i| i.id)
            .collect();
        Some(WasmStencilSource {
            function,
            loop_id: descriptor.id,
            preheader: descriptor.preheader?,
            header,
            body: b.id,
            induction,
            body_induction,
            bound,
            width,
            row_base,
            input,
            output,
            loads,
            store,
            store_origin,
            scalar_address_setup: setup,
            ranges: vec![
                StencilRangeRequirement {
                    slice: input,
                    start: None,
                    count: StencilRangeCount::Width,
                    element_bytes: 8,
                },
                StencilRangeRequirement {
                    slice: input,
                    start: Some(top),
                    count: StencilRangeCount::ThreeWidths,
                    element_bytes: 8,
                },
                StencilRangeRequirement {
                    slice: output,
                    start: Some(store_origin),
                    count: StencilRangeCount::InteriorTrip,
                    element_bytes: 8,
                },
            ],
            minimum_trip: 4,
            source_digest: state.kir_digest(),
        })
    }
    let source = reconstruct(state, function, header)
        .ok_or("source does not reconstruct a nine-load stencil")?;
    check_wasm_stencil_source_independently(state, &source)?;
    Ok(source)
}
