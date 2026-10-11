//! Independent proof of source access coverage by Wasm slice-range guards.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use num_bigint::BigInt;

use crate::{
    AffineForm, BlockId, InstructionId, KirArithmeticSemantics, KirFunction, KirInstruction,
    KirInstructionKind, KirPlace, KirTerminator, KirValueType, MemoryRegionId, MirBinaryOp,
    MirCompareOp, MirPrimitiveTypeName, MirType, ValueId, VectorMemoryAccessKind, WasmRangeCount,
    WasmSliceRangeRequirement, analyze_canonical_loops, compute_kir_dominators,
};

const U32_MODULUS: u64 = 1_u64 << 32;
const MAX_SOURCE_NODES: usize = 512;
const MAX_SOURCE_ACCESSES: usize = 128;
const MAX_AFFINE_TERMS: usize = 32;
const MAX_REFINEMENT_PASSES: usize = 32;
const MAX_RANGE_RECIPES: usize = 3;

/// Source memory access reconstructed by the guarded-span checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedSpanAccess {
    pub instruction: InstructionId,
    pub index: ValueId,
    pub slice: ValueId,
    pub region: MemoryRegionId,
    pub access: VectorMemoryAccessKind,
    pub contiguous: bool,
    /// `None` denotes the zero origin; otherwise this is the independently
    /// reconstructed source value added to the canonical induction variable.
    pub origin: Option<ValueId>,
    pub origin_expression: AffineForm,
}

/// A source access proved to lie in one exact range recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedSpanAccessProof {
    pub range_ordinal: usize,
    /// The non-negative mathematical offset from the selected guarded start.
    pub offset: AffineForm,
}

/// Independently reconstructed source addresses and their guarded coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedSpanCoverage {
    pub accesses: BTreeMap<InstructionId, GuardedSpanAccess>,
    pub setup: BTreeSet<InstructionId>,
    pub ranges: BTreeSet<WasmSliceRangeRequirement>,
    pub coverage: BTreeMap<InstructionId, GuardedSpanAccessProof>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Interval {
    lower: BigInt,
    upper: BigInt,
}

type RefinedGuardAssumptions = (BTreeMap<ValueId, Interval>, BTreeMap<ValueId, AffineForm>);

impl Interval {
    fn u32() -> Self {
        Self {
            lower: BigInt::from(0_u8),
            upper: modulus() - BigInt::from(1_u8),
        }
    }

    fn intersect(&mut self, other: &Self) -> Result<bool, String> {
        let lower = self.lower.clone().max(other.lower.clone());
        let upper = self.upper.clone().min(other.upper.clone());
        if lower > upper {
            return Err("guard assumptions have no possible u32 value".into());
        }
        let changed = lower != self.lower || upper != self.upper;
        self.lower = lower;
        self.upper = upper;
        Ok(changed)
    }
}

#[derive(Debug, Clone)]
struct ModularRelation {
    value: ValueId,
    expression: AffineForm,
}

#[derive(Debug, Clone)]
struct ExactRelation {
    difference: AffineForm,
    multiple: BigInt,
}

#[derive(Debug, Clone)]
struct RangeFacts {
    recipe: WasmSliceRangeRequirement,
    ordinal: usize,
    slice_root: ValueId,
    start_expression: AffineForm,
    count_exact_expression: AffineForm,
    max_elements: BigInt,
}

#[derive(Debug, Clone)]
struct RawAccess {
    access: GuardedSpanAccess,
    element_bytes: u32,
    contiguous: bool,
}

struct SourceExpressions<'a> {
    function: &'a KirFunction,
    preheader: BlockId,
    induction: ValueId,
    loop_blocks: BTreeSet<BlockId>,
    definitions: BTreeMap<ValueId, (BlockId, &'a KirInstruction)>,
    block_params: BTreeMap<ValueId, BlockId>,
    incoming: BTreeMap<ValueId, Vec<ValueId>>,
    dominators: crate::KirDominators,
    roots: HashMap<ValueId, ValueId>,
    setup_ids: BTreeSet<InstructionId>,
    bounds: BTreeMap<ValueId, Interval>,
    forms: HashMap<ValueId, AffineForm>,
    relations: Vec<ModularRelation>,
    visiting: BTreeSet<ValueId>,
    node_visits: usize,
}

impl<'a> SourceExpressions<'a> {
    fn new(
        function: &'a KirFunction,
        preheader: BlockId,
        induction: ValueId,
        loop_blocks: &[BlockId],
    ) -> Self {
        let mut definitions = BTreeMap::new();
        let mut block_params = BTreeMap::new();
        let block_by_id = function
            .blocks
            .iter()
            .map(|block| (block.id, block))
            .collect::<BTreeMap<_, _>>();
        for block in &function.blocks {
            for parameter in &block.params {
                block_params.insert(parameter.value, block.id);
            }
            for instruction in &block.instructions {
                for result in &instruction.results {
                    definitions.insert(result.value, (block.id, instruction));
                }
            }
        }
        let mut incoming = BTreeMap::<ValueId, Vec<ValueId>>::new();
        for block in &function.blocks {
            for edge in outgoing_edges(&block.terminator) {
                let Some(target) = block_by_id.get(&edge.target) else {
                    continue;
                };
                for (parameter, argument) in target.params.iter().zip(&edge.args) {
                    incoming.entry(parameter.value).or_default().push(*argument);
                }
            }
        }
        Self {
            function,
            preheader,
            induction,
            loop_blocks: loop_blocks.iter().copied().collect(),
            definitions,
            block_params,
            incoming,
            dominators: compute_kir_dominators(function),
            roots: HashMap::new(),
            setup_ids: BTreeSet::new(),
            bounds: BTreeMap::new(),
            forms: HashMap::new(),
            relations: Vec::new(),
            visiting: BTreeSet::new(),
            node_visits: 0,
        }
    }

    fn value_type(&self, value: ValueId) -> Option<KirValueType> {
        self.function
            .params
            .iter()
            .find(|parameter| parameter.value == value)
            .map(|parameter| KirValueType::Scalar(parameter.type_node.clone()))
            .or_else(|| {
                self.function.blocks.iter().find_map(|block| {
                    block
                        .params
                        .iter()
                        .find(|parameter| parameter.value == value)
                        .map(|parameter| parameter.type_node.clone())
                        .or_else(|| {
                            block
                                .instructions
                                .iter()
                                .flat_map(|instruction| &instruction.results)
                                .find(|result| result.value == value)
                                .map(|result| result.type_node.clone())
                        })
                })
            })
    }

    fn is_u32(&self, value: ValueId) -> bool {
        self.value_type(value)
            .and_then(|type_node| type_node.as_scalar().cloned())
            == Some(MirType::Primitive(MirPrimitiveTypeName::U32))
    }

    fn root(&mut self, value: ValueId) -> Result<ValueId, String> {
        if let Some(root) = self.roots.get(&value) {
            return Ok(*root);
        }
        let expected_type = self
            .value_type(value)
            .ok_or_else(|| "guarded address references an unknown SSA value".to_string())?;
        let mut pending = vec![value];
        let mut visited = BTreeSet::new();
        let mut terminals = BTreeSet::new();
        while let Some(current) = pending.pop() {
            if current == self.induction {
                terminals.insert(current);
                continue;
            }
            if !visited.insert(current) {
                continue;
            }
            self.node_visits = self.node_visits.saturating_add(1);
            if self.node_visits > MAX_SOURCE_NODES {
                return Err("guarded source reconstruction exceeded its node budget".into());
            }
            if self.value_type(current).as_ref() != Some(&expected_type) {
                return Err("copy or phi forwarding changes a source value type".into());
            }
            if let Some((block, instruction)) = self.definitions.get(&current).copied()
                && let KirInstructionKind::Copy { value: source } = instruction.kind
            {
                if instruction.results.len() != 1
                    || instruction.memory.is_some()
                    || instruction.effect.is_some()
                    || self.value_type(source).as_ref() != Some(&expected_type)
                {
                    return Err("copy forwarding is effectful or ill-typed".into());
                }
                if self.loop_blocks.contains(&block) {
                    self.setup_ids.insert(instruction.id);
                }
                pending.push(source);
                continue;
            }
            if let Some(arguments) = self.incoming.get(&current).cloned() {
                if !arguments.is_empty() {
                    // Values from an enclosing loop can vary between
                    // invocations of this loop while remaining invariant
                    // throughout the selected loop. Keep such a dominating
                    // block parameter as an atom; only peel phis owned by the
                    // loop whose induction and accesses are being proved.
                    let enclosing_value = self
                        .block_params
                        .get(&current)
                        .is_some_and(|block| !self.loop_blocks.contains(block))
                        && self.is_available_at_preheader(current);
                    if enclosing_value {
                        let (forwarded_roots, forwarded_values) =
                            self.forwarded_sources(&arguments, &expected_type)?;
                        if forwarded_roots.len() == 1 {
                            terminals.insert(*forwarded_roots.first().expect("one root"));
                            for forwarded in forwarded_values {
                                if let Some((block, instruction)) =
                                    self.definitions.get(&forwarded).copied()
                                    && self.loop_blocks.contains(&block)
                                    && matches!(instruction.kind, KirInstructionKind::Copy { .. })
                                {
                                    self.setup_ids.insert(instruction.id);
                                }
                                visited.insert(forwarded);
                            }
                        } else {
                            // An outer-loop value with changing incoming roots
                            // is still one stable SSA atom during this loop.
                            terminals.insert(current);
                        }
                        continue;
                    }
                    pending.extend(arguments);
                    continue;
                }
                // The entry preheader's block parameters are available inputs.
                if self.block_params.get(&current) == Some(&self.preheader) {
                    terminals.insert(current);
                    continue;
                }
                return Err("phi value has no incoming source".into());
            }
            terminals.insert(current);
        }
        if terminals.len() != 1 {
            return Err("copy or phi inputs have conflicting source roots".into());
        }
        let root = *terminals.first().expect("one terminal root");
        for visited_value in visited {
            self.roots.insert(visited_value, root);
        }
        self.roots.insert(value, root);
        Ok(root)
    }

    fn forwarded_sources(
        &mut self,
        values: &[ValueId],
        expected_type: &KirValueType,
    ) -> Result<(BTreeSet<ValueId>, BTreeSet<ValueId>), String> {
        let mut pending = values.to_vec();
        let mut visited = BTreeSet::new();
        let mut terminals = BTreeSet::new();
        while let Some(current) = pending.pop() {
            if current == self.induction {
                terminals.insert(current);
                continue;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(root) = self.roots.get(&current) {
                terminals.insert(*root);
                continue;
            }
            self.node_visits = self.node_visits.saturating_add(1);
            if self.node_visits > MAX_SOURCE_NODES {
                return Err("guarded source reconstruction exceeded its node budget".into());
            }
            if self.value_type(current).as_ref() != Some(expected_type) {
                return Err("copy or phi forwarding changes a source value type".into());
            }
            if let Some((_, instruction)) = self.definitions.get(&current).copied()
                && let KirInstructionKind::Copy { value: source } = instruction.kind
            {
                if instruction.results.len() != 1
                    || instruction.memory.is_some()
                    || instruction.effect.is_some()
                    || self.value_type(source).as_ref() != Some(expected_type)
                {
                    return Err("copy forwarding is effectful or ill-typed".into());
                }
                pending.push(source);
                continue;
            }
            if let Some(arguments) = self.incoming.get(&current)
                && !arguments.is_empty()
            {
                pending.extend(arguments.iter().copied());
                continue;
            }
            if self.block_params.contains_key(&current)
                && self.block_params.get(&current) != Some(&self.preheader)
            {
                return Err("phi value has no incoming source".into());
            }
            terminals.insert(current);
        }
        Ok((terminals, visited))
    }

    fn is_available_at_preheader(&self, value: ValueId) -> bool {
        if self
            .function
            .params
            .iter()
            .any(|parameter| parameter.value == value)
        {
            return true;
        }
        if let Some(block) = self.block_params.get(&value)
            && self.dominators.dominates(*block, self.preheader)
        {
            return true;
        }
        self.definitions
            .get(&value)
            .is_some_and(|(block, _)| self.dominators.dominates(*block, self.preheader))
    }

    fn is_bound_slice_length(
        &mut self,
        value: ValueId,
        bound_root: ValueId,
    ) -> Result<bool, String> {
        if value != bound_root {
            return Ok(false);
        }
        let Some((block, instruction)) = self.definitions.get(&value).copied() else {
            return Ok(false);
        };
        if !self.loop_blocks.contains(&block)
            || instruction.results.len() != 1
            || instruction.memory.is_some()
            || instruction.effect.is_some()
            || !self.is_u32(value)
        {
            return Ok(false);
        }
        let KirInstructionKind::SliceLen { slice } = &instruction.kind else {
            return Ok(false);
        };
        let slice_root = self.root(*slice)?;
        let is_slice = self
            .value_type(slice_root)
            .and_then(|type_node| type_node.as_scalar().cloned())
            .is_some_and(|type_node| matches!(type_node, MirType::Slice(_)));
        Ok(is_slice && self.is_available_at_preheader(slice_root))
    }

    fn atom(&mut self, value: ValueId) -> Result<AffineForm, String> {
        if !self.is_u32(value) {
            return Err("guarded affine expressions require u32 values".into());
        }
        let root = self.root(value)?;
        self.bounds.entry(root).or_insert_with(Interval::u32);
        Ok(AffineForm::variable(root))
    }

    fn expression(&mut self, value: ValueId) -> Result<AffineForm, String> {
        if !self.is_u32(value) {
            return Err("guarded affine expressions require u32 values".into());
        }
        let root = self.root(value)?;
        if let Some(form) = self.forms.get(&root) {
            return Ok(form.clone());
        }
        if root == self.induction {
            return self.atom(root);
        }
        if !self.visiting.insert(root) {
            return Err("cyclic non-induction source expression".into());
        }
        self.node_visits = self.node_visits.saturating_add(1);
        if self.node_visits > MAX_SOURCE_NODES {
            self.visiting.remove(&root);
            return Err("guarded source reconstruction exceeded its node budget".into());
        }
        let form = if let Some((block_id, instruction)) = self.definitions.get(&root).copied() {
            if instruction.results.len() != 1
                || instruction.memory.is_some()
                || instruction.effect.is_some()
                || !self.is_u32(root)
            {
                self.visiting.remove(&root);
                return Err("source address expression is effectful or ill-typed".into());
            }
            match &instruction.kind {
                KirInstructionKind::ConstInt { value } => {
                    let number = value
                        .parse::<u32>()
                        .map_err(|_| "source integer constant is outside u32".to_string())?;
                    self.relation(root, AffineForm::integer(BigInt::from(number)))?;
                    self.note_setup(block_id, instruction);
                    AffineForm::integer(BigInt::from(number))
                }
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add | MirBinaryOp::Sub,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } => {
                    let left_form = self.expression(*left)?;
                    let right_form = self.expression(*right)?;
                    let expression = match instruction.kind {
                        KirInstructionKind::Binary {
                            op: MirBinaryOp::Sub,
                            ..
                        } => left_form.sub(&right_form),
                        _ => left_form.add(&right_form),
                    };
                    let expression = ring_normalize(expression)?;
                    self.relation(root, expression.clone())?;
                    self.note_setup(block_id, instruction);
                    expression
                }
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Mul,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } => {
                    let left_constant = self.constant_expression(*left, &mut BTreeSet::new())?;
                    let right_constant = self.constant_expression(*right, &mut BTreeSet::new())?;
                    let (variable, factor) = match (left_constant, right_constant) {
                        (Some(factor), _) => (*right, factor),
                        (_, Some(factor)) => (*left, factor),
                        _ => {
                            if self.opaque_allowed(block_id, root) {
                                self.bounds.entry(root).or_insert_with(Interval::u32);
                                self.visiting.remove(&root);
                                let atom = AffineForm::variable(root);
                                self.forms.insert(root, atom.clone());
                                return Ok(atom);
                            }
                            self.visiting.remove(&root);
                            return Err("loop-local multiplication is not affine".into());
                        }
                    };
                    let variable_form = self.expression(variable)?;
                    let expression = ring_normalize(variable_form.scale(&factor))?;
                    self.relation(root, expression.clone())?;
                    self.note_setup(block_id, instruction);
                    expression
                }
                _ if self.opaque_allowed(block_id, root) => {
                    self.bounds.entry(root).or_insert_with(Interval::u32);
                    let atom = AffineForm::variable(root);
                    self.forms.insert(root, atom.clone());
                    self.visiting.remove(&root);
                    return Ok(atom);
                }
                _ => {
                    self.visiting.remove(&root);
                    return Err("loop-local source expression is unsupported".into());
                }
            }
        } else {
            self.bounds.entry(root).or_insert_with(Interval::u32);
            AffineForm::variable(root)
        };
        self.visiting.remove(&root);
        self.forms.insert(root, form.clone());
        Ok(form)
    }

    fn constant_expression(
        &mut self,
        value: ValueId,
        seen: &mut BTreeSet<ValueId>,
    ) -> Result<Option<BigInt>, String> {
        let root = match self.root(value) {
            Ok(root) => root,
            Err(error) if error == "copy or phi inputs have conflicting source roots" => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if !seen.insert(root) {
            return Ok(None);
        }
        self.node_visits = self.node_visits.saturating_add(1);
        if self.node_visits > MAX_SOURCE_NODES {
            seen.remove(&root);
            return Err("guarded constant expression exceeded its node budget".into());
        }
        let constant = match self.definitions.get(&root).copied() {
            Some((_, instruction))
                if instruction.results.len() == 1
                    && instruction.memory.is_none()
                    && instruction.effect.is_none() =>
            {
                match &instruction.kind {
                    KirInstructionKind::ConstInt { value } => {
                        value.parse::<u32>().ok().map(BigInt::from)
                    }
                    KirInstructionKind::Binary {
                        op,
                        left,
                        right,
                        semantics: KirArithmeticSemantics::Modular,
                    } => {
                        let Some(left) = self.constant_expression(*left, seen)? else {
                            seen.remove(&root);
                            return Ok(None);
                        };
                        let Some(right) = self.constant_expression(*right, seen)? else {
                            seen.remove(&root);
                            return Ok(None);
                        };
                        let result = match op {
                            MirBinaryOp::Add => left + right,
                            MirBinaryOp::Sub => left - right,
                            MirBinaryOp::Mul => left * right,
                            _ => {
                                seen.remove(&root);
                                return Ok(None);
                            }
                        };
                        Some(unsigned_modulo(result))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if constant.is_some()
            && let Some((block, instruction)) = self.definitions.get(&root).copied()
        {
            self.note_setup(block, instruction);
        }
        seen.remove(&root);
        Ok(constant)
    }

    fn opaque_allowed(&self, block: BlockId, value: ValueId) -> bool {
        !self.loop_blocks.contains(&block)
            && self.is_available_at_preheader(value)
            && self
                .definitions
                .get(&value)
                .is_some_and(|(_, instruction)| {
                    instruction.results.len() == 1
                        && instruction.memory.is_none()
                        && instruction.effect.is_none()
                })
    }

    fn note_setup(&mut self, block: BlockId, instruction: &KirInstruction) {
        if self.loop_blocks.contains(&block) {
            self.setup_ids.insert(instruction.id);
        }
    }

    fn relation(&mut self, value: ValueId, expression: AffineForm) -> Result<(), String> {
        self.bounds.entry(value).or_insert_with(Interval::u32);
        if expression.terms().len() > MAX_AFFINE_TERMS {
            return Err("guarded affine expression exceeded its term budget".into());
        }
        if !self
            .relations
            .iter()
            .any(|relation| relation.value == value && relation.expression == expression)
        {
            self.relations.push(ModularRelation { value, expression });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct LoopContext {
    body_induction_root: ValueId,
    bound_root: ValueId,
    header: BlockId,
    loop_blocks: Vec<BlockId>,
}

fn outgoing_edges(terminator: &KirTerminator) -> Vec<&crate::KirEdge> {
    match terminator {
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        KirTerminator::Return { .. } => Vec::new(),
    }
}

fn modulus() -> BigInt {
    BigInt::from(U32_MODULUS)
}

fn centered_modulo(value: BigInt) -> BigInt {
    let modulus = modulus();
    let mut residue = value % &modulus;
    if residue < BigInt::from(0_u8) {
        residue += &modulus;
    }
    let half = &modulus / BigInt::from(2_u8);
    if residue >= half {
        residue -= modulus;
    }
    residue
}

fn unsigned_modulo(value: BigInt) -> BigInt {
    let modulus = modulus();
    let mut residue = value % &modulus;
    if residue < BigInt::from(0_u8) {
        residue += modulus;
    }
    residue
}

fn ring_normalize(form: AffineForm) -> Result<AffineForm, String> {
    let mut result = AffineForm::integer(centered_modulo(form.constant().clone()));
    for (value, coefficient) in form.terms() {
        let coefficient = centered_modulo(coefficient.clone());
        if coefficient != BigInt::from(0_u8) {
            result = result.add(&AffineForm::variable(*value).scale(&coefficient));
        }
    }
    if result.terms().len() > MAX_AFFINE_TERMS {
        return Err("guarded affine expression exceeded its term budget".into());
    }
    Ok(result)
}

fn interval_of(
    form: &AffineForm,
    bounds: &BTreeMap<ValueId, Interval>,
) -> Result<Interval, String> {
    if form.terms().len() > MAX_AFFINE_TERMS {
        return Err("guarded interval expression exceeded its term budget".into());
    }
    let mut lower = form.constant().clone();
    let mut upper = form.constant().clone();
    for (value, coefficient) in form.terms() {
        let interval = bounds.get(value).cloned().unwrap_or_else(Interval::u32);
        if coefficient >= &BigInt::from(0_u8) {
            lower += coefficient * &interval.lower;
            upper += coefficient * &interval.upper;
        } else {
            lower += coefficient * &interval.upper;
            upper += coefficient * &interval.lower;
        }
    }
    Ok(Interval { lower, upper })
}

fn ceil_div(value: &BigInt, divisor: &BigInt) -> BigInt {
    -floor_div(&(-value), divisor)
}

fn floor_div(value: &BigInt, divisor: &BigInt) -> BigInt {
    let quotient = value / divisor;
    let remainder = value % divisor;
    if remainder != BigInt::from(0_u8) && value < &BigInt::from(0_u8) {
        quotient - BigInt::from(1_u8)
    } else {
        quotient
    }
}

fn unique_modular_multiple(interval: &Interval) -> Option<BigInt> {
    let modulus = modulus();
    let low = ceil_div(&interval.lower, &modulus);
    let high = floor_div(&interval.upper, &modulus);
    (low == high).then_some(low)
}

fn exact_substitutions(
    equations: &[ExactRelation],
) -> Result<BTreeMap<ValueId, AffineForm>, String> {
    let mut result = BTreeMap::new();
    for equation in equations {
        let mut unit = None;
        for (value, coefficient) in equation.difference.terms() {
            if coefficient == &BigInt::from(1_u8) {
                if unit.is_some() {
                    unit = None;
                    break;
                }
                unit = Some(*value);
            }
        }
        let Some(value) = unit else {
            continue;
        };
        let rest = equation.difference.sub(&AffineForm::variable(value));
        if rest.terms().contains_key(&value) {
            continue;
        }
        let replacement = AffineForm::integer(&equation.multiple * modulus()).sub(&rest);
        if replacement.terms().len() > MAX_AFFINE_TERMS {
            return Err("guarded equality substitution exceeded its term budget".into());
        }
        if let Some(existing) = result.get(&value) {
            if existing != &replacement {
                return Err("guarded source value has conflicting exact lifts".into());
            }
        } else {
            result.insert(value, replacement);
        }
    }
    Ok(result)
}

fn substitute_exact(
    form: &AffineForm,
    substitutions: &BTreeMap<ValueId, AffineForm>,
    visiting: &mut BTreeSet<ValueId>,
) -> Result<AffineForm, String> {
    let mut result = AffineForm::integer(form.constant().clone());
    for (value, coefficient) in form.terms() {
        if let Some(replacement) = substitutions.get(value) {
            if !visiting.insert(*value) {
                return Err("cyclic exact source equalities".into());
            }
            let expanded = substitute_exact(replacement, substitutions, visiting)?;
            visiting.remove(value);
            result = result.add(&expanded.scale(coefficient));
        } else {
            result = result.add(&AffineForm::variable(*value).scale(coefficient));
        }
    }
    if result.terms().len() > MAX_AFFINE_TERMS {
        return Err("guarded equality substitution exceeded its term budget".into());
    }
    Ok(result)
}

fn propagate_relation_bounds(
    equation: &ExactRelation,
    bounds: &mut BTreeMap<ValueId, Interval>,
) -> Result<bool, String> {
    let mut changed = false;
    let right = &equation.multiple * modulus();
    let terms = equation
        .difference
        .terms()
        .iter()
        .map(|(value, coefficient)| (*value, coefficient.clone()))
        .collect::<Vec<_>>();
    for (value, coefficient) in &terms {
        if coefficient != &BigInt::from(1_u8) && coefficient != &BigInt::from(-1_i8) {
            continue;
        }
        let rest = equation
            .difference
            .sub(&AffineForm::variable(*value).scale(coefficient));
        let rest_interval = interval_of(&rest, bounds)?;
        let inferred = if coefficient == &BigInt::from(1_u8) {
            Interval {
                lower: &right - &rest_interval.upper,
                upper: &right - &rest_interval.lower,
            }
        } else {
            Interval {
                lower: &rest_interval.lower - &right,
                upper: &rest_interval.upper - &right,
            }
        };
        let entry = bounds.entry(*value).or_insert_with(Interval::u32);
        changed |= entry.intersect(&inferred)?;
    }
    Ok(changed)
}

// These coordinates are deliberately explicit because this verifier checks
// each caller-supplied identity against the canonical loop descriptor.
#[allow(clippy::too_many_arguments)]
fn validate_loop_context(
    function: &KirFunction,
    preheader: BlockId,
    header: BlockId,
    body: BlockId,
    induction: ValueId,
    body_induction: ValueId,
    bound: ValueId,
    expressions: &mut SourceExpressions<'_>,
) -> Result<LoopContext, String> {
    let descriptor = analyze_canonical_loops(function)
        .loops
        .into_iter()
        .find(|descriptor| descriptor.header == header)
        .ok_or_else(|| "guarded source is not a canonical loop".to_string())?;
    let loop_induction = descriptor
        .induction
        .as_ref()
        .ok_or_else(|| "guarded loop has no induction descriptor".to_string())?;
    if descriptor.preheader != Some(preheader)
        || !descriptor.innermost
        || !descriptor.blocks.contains(&body)
        || loop_induction.value != induction
        || loop_induction.bound != bound
        || loop_induction.comparison != MirCompareOp::Lt
        || loop_induction.start != BigInt::from(0_u8)
        || loop_induction.step != BigInt::from(1_u8)
        || !loop_induction.wrap_safe_for_strict_bound
        || !expressions.is_u32(induction)
        || !expressions.is_u32(bound)
    {
        return Err("guarded source loop coordinates are not canonical u32 values".into());
    }
    if expressions.root(body_induction)? != induction {
        return Err("loop body induction does not forward to its canonical value".into());
    }
    let header_block = function
        .blocks
        .iter()
        .find(|block| block.id == header)
        .ok_or_else(|| "guarded loop header is missing".to_string())?;
    let KirTerminator::Branch {
        condition,
        then_edge,
        ..
    } = &header_block.terminator
    else {
        return Err("guarded loop header is not a conditional branch".into());
    };
    if then_edge.target != body {
        return Err("guarded loop body differs from its source branch".into());
    }
    if header_block
        .instructions
        .iter()
        .any(|instruction| instruction.memory.is_some() || instruction.effect.is_some())
    {
        return Err("guarded loop condition block has an effect".into());
    }
    let compare_value = expressions.root(*condition)?;
    let (_, compare) = expressions
        .definitions
        .get(&compare_value)
        .copied()
        .ok_or_else(|| "guarded loop condition is not a source comparison".to_string())?;
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left,
        right,
    } = compare.kind
    else {
        return Err("guarded loop condition is not unsigned less-than".into());
    };
    let left_matches_induction = expressions.root(left)? == induction;
    let (header_uses_bound, bound_root) = if left_matches_induction {
        let right_root = expressions.root(right)?;
        if right == bound || right_root == bound {
            (true, right_root)
        } else {
            let bound_root = expressions.root(bound)?;
            (right_root == bound_root, bound_root)
        }
    } else {
        (false, bound)
    };
    if !left_matches_induction || !header_uses_bound {
        return Err("guarded loop comparison uses different coordinates".into());
    }
    let body_block = function
        .blocks
        .iter()
        .find(|block| block.id == body)
        .ok_or_else(|| "guarded loop body is missing".to_string())?;
    if !body_block
        .params
        .iter()
        .any(|parameter| parameter.value == body_induction)
    {
        return Err("body induction is not a body block parameter".into());
    }
    let entry_induction = then_edge
        .args
        .iter()
        .zip(&body_block.params)
        .find_map(|(argument, parameter)| (parameter.value == body_induction).then_some(*argument))
        .ok_or_else(|| "body induction has no header incoming value".to_string())?;
    if expressions.root(entry_induction)? != induction {
        return Err("body induction has a conflicting incoming edge".into());
    }
    Ok(LoopContext {
        body_induction_root: induction,
        bound_root,
        header,
        loop_blocks: descriptor.blocks,
    })
}

fn primitive_element_bytes(type_node: &MirType) -> Option<u32> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::Bool) => Some(1),
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => Some(8),
        _ => None,
    }
}

fn range_count_form(
    expressions: &mut SourceExpressions<'_>,
    count: WasmRangeCount,
    bound_root: ValueId,
) -> Result<(AffineForm, AffineForm), String> {
    match count {
        WasmRangeCount::TripBound(value) | WasmRangeCount::Invariant(value) => {
            let root = expressions.root(value)?;
            let available = expressions.is_available_at_preheader(root);
            let is_bound_length =
                !available && expressions.is_bound_slice_length(root, bound_root)?;
            if !expressions.is_u32(value) || (!available && !is_bound_length) {
                return Err("range count is not an available invariant u32".into());
            }
            let exact = expressions.atom(root)?;
            let modular = if available {
                expressions.expression(root)?
            } else {
                exact.clone()
            };
            Ok((modular, exact))
        }
        WasmRangeCount::ScaledInvariant { value, scale } => {
            let root = expressions.root(value)?;
            let available = expressions.is_available_at_preheader(root);
            let is_bound_length =
                !available && expressions.is_bound_slice_length(root, bound_root)?;
            if scale == 0 || !expressions.is_u32(value) || (!available && !is_bound_length) {
                return Err("scaled range count is zero or not an available u32".into());
            }
            let atom = expressions.atom(root)?;
            let modular = if available {
                ring_normalize(atom.scale(&BigInt::from(scale)))?
            } else {
                atom.scale(&BigInt::from(scale))
            };
            let exact = atom.scale(&BigInt::from(scale));
            Ok((modular, exact))
        }
        WasmRangeCount::One => {
            let one = AffineForm::integer(BigInt::from(1_u8));
            Ok((one.clone(), one))
        }
    }
}

fn collect_ranges(
    expressions: &mut SourceExpressions<'_>,
    ranges: &[WasmSliceRangeRequirement],
    bound_root: ValueId,
) -> Result<Vec<RangeFacts>, String> {
    if ranges.is_empty() || ranges.len() > MAX_RANGE_RECIPES {
        return Err("guarded span requires one to three range recipes".into());
    }
    let mut unique = BTreeSet::new();
    let mut facts = Vec::with_capacity(ranges.len());
    for (ordinal, recipe) in ranges.iter().copied().enumerate() {
        if !unique.insert(recipe) || recipe.element_bytes == 0 {
            return Err("guarded range recipe is duplicate or has zero element width".into());
        }
        let slice_root = expressions.root(recipe.slice)?;
        let slice_type = expressions
            .value_type(slice_root)
            .and_then(|type_node| type_node.as_scalar().cloned());
        if !matches!(slice_type, Some(MirType::Slice(_)))
            || !expressions.is_available_at_preheader(slice_root)
        {
            return Err("guarded range slice is not an available source slice".into());
        }
        let (_, start_expression) = match recipe.start {
            Some(start) => {
                let root = expressions.root(start)?;
                if !expressions.is_u32(start) || !expressions.is_available_at_preheader(root) {
                    return Err("guarded range start is not an available invariant u32".into());
                }
                let form = expressions.expression(root)?;
                (Some(root), form)
            }
            None => (None, AffineForm::integer(BigInt::from(0_u8))),
        };
        let (_, count_exact_expression) = range_count_form(expressions, recipe.count, bound_root)?;
        facts.push(RangeFacts {
            recipe,
            ordinal,
            slice_root,
            start_expression,
            count_exact_expression,
            max_elements: BigInt::from(U32_MODULUS / u64::from(recipe.element_bytes)),
        });
    }
    Ok(facts)
}

fn source_accesses(
    function: &KirFunction,
    expressions: &mut SourceExpressions<'_>,
    context: &LoopContext,
) -> Result<BTreeMap<InstructionId, RawAccess>, String> {
    let mut accesses = BTreeMap::new();
    let loop_blocks = context
        .loop_blocks
        .iter()
        .filter(|block| **block != context.header)
        .copied()
        .collect::<BTreeSet<_>>();
    for block in function
        .blocks
        .iter()
        .filter(|block| loop_blocks.contains(&block.id))
    {
        for instruction in &block.instructions {
            let (place, access) = match &instruction.kind {
                KirInstructionKind::Load { place } => {
                    (place.as_ref(), VectorMemoryAccessKind::Read)
                }
                KirInstructionKind::Store { place, .. } => {
                    (place.as_ref(), VectorMemoryAccessKind::Write)
                }
                _ => {
                    if instruction.memory.is_some() || instruction.effect.is_some() {
                        return Err(
                            "loop contains an unsupported memory or effect instruction".into()
                        );
                    }
                    continue;
                }
            };
            let KirPlace::SliceIndex {
                slice,
                index,
                type_node,
                region,
            } = place
            else {
                return Err("guarded source memory access is not a slice index".into());
            };
            let element_bytes = primitive_element_bytes(type_node).ok_or_else(|| {
                "guarded source element type has no scalar byte width".to_string()
            })?;
            let memory = instruction
                .memory
                .as_ref()
                .ok_or_else(|| "guarded source access lacks MemorySSA identity".to_string())?;
            if !function
                .regions
                .iter()
                .any(|descriptor| descriptor.id == *region && descriptor.partition == memory.region)
                || !function.regions.iter().any(|descriptor| {
                    descriptor.id == memory.region && descriptor.partition == memory.region
                })
            {
                return Err("guarded source access has a false MemorySSA region".into());
            }
            let slice = expressions.root(*slice)?;
            let index_root = expressions.root(*index)?;
            if !expressions.is_u32(*index) {
                return Err("guarded source index is not u32".into());
            }
            let index_expression = expressions.expression(index_root)?;
            let iv_coefficient = index_expression.coefficient(context.body_induction_root);
            let (origin, origin_expression, contiguous) = if iv_coefficient == BigInt::from(1_u8) {
                if index_root == context.body_induction_root {
                    (None, AffineForm::integer(BigInt::from(0_u8)), true)
                } else {
                    let definition = expressions
                        .definitions
                        .get(&index_root)
                        .map(|(_, instruction)| *instruction)
                        .ok_or_else(|| {
                            "contiguous index is not a source address expression".to_string()
                        })?;
                    let KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left,
                        right,
                        semantics: KirArithmeticSemantics::Modular,
                    } = definition.kind
                    else {
                        return Err("contiguous source index is not origin plus induction".into());
                    };
                    let (origin, induction_side) =
                        if expressions.root(left)? == context.body_induction_root {
                            (right, left)
                        } else if expressions.root(right)? == context.body_induction_root {
                            (left, right)
                        } else {
                            return Err(
                                "contiguous source index has no direct induction operand".into()
                            );
                        };
                    if expressions.root(induction_side)? != context.body_induction_root {
                        return Err(
                            "contiguous source index uses a conflicting induction value".into()
                        );
                    }
                    let origin = expressions.root(origin)?;
                    if !expressions.is_available_at_preheader(origin) {
                        return Err(
                            "nonzero source origin is not available at the loop preheader".into(),
                        );
                    }
                    let origin_expression = expressions.expression(origin)?;
                    if origin_expression.coefficient(context.body_induction_root)
                        != BigInt::from(0_u8)
                    {
                        return Err(
                            "contiguous source origin still depends on its induction".into()
                        );
                    }
                    let rebuilt = ring_normalize(
                        origin_expression.add(&AffineForm::variable(context.body_induction_root)),
                    )?;
                    if ring_normalize(index_expression.clone())? != rebuilt {
                        return Err(
                            "source index differs from its reconstructed origin plus induction"
                                .into(),
                        );
                    }
                    (Some(origin), origin_expression, true)
                }
            } else if iv_coefficient == BigInt::from(0_u8) {
                if !expressions.is_available_at_preheader(index_root) {
                    return Err(
                        "broadcast source index is not available at the loop preheader".into(),
                    );
                }
                (Some(index_root), index_expression.clone(), false)
            } else {
                return Err("source index has a non-unit induction coefficient".into());
            };
            let access_record = GuardedSpanAccess {
                instruction: instruction.id,
                index: *index,
                slice,
                region: *region,
                access,
                contiguous,
                origin,
                origin_expression,
            };
            if accesses
                .insert(
                    instruction.id,
                    RawAccess {
                        access: access_record,
                        element_bytes,
                        contiguous,
                    },
                )
                .is_some()
            {
                return Err("source memory instruction identity is duplicated".into());
            }
            if accesses.len() > MAX_SOURCE_ACCESSES {
                return Err("guarded source exceeds the access budget".into());
            }
        }
    }
    if accesses.is_empty() {
        return Err("guarded source loop has no slice accesses".into());
    }
    Ok(accesses)
}

fn intersect_bound(
    bounds: &mut BTreeMap<ValueId, Interval>,
    value: ValueId,
    interval: Interval,
) -> Result<bool, String> {
    bounds
        .entry(value)
        .or_insert_with(Interval::u32)
        .intersect(&interval)
}

fn refine_relations_and_scaled_counts(
    expressions: &SourceExpressions<'_>,
    ranges: &[RangeFacts],
    bound: ValueId,
    minimum_trip: u32,
) -> Result<RefinedGuardAssumptions, String> {
    if minimum_trip == 0 {
        return Err("guarded vector trip threshold must be positive".into());
    }
    let bound = expressions.roots.get(&bound).copied().unwrap_or(bound);
    let mut bounds = expressions.bounds.clone();
    let mut exact_equations = Vec::<ExactRelation>::new();
    let threshold = Interval {
        lower: BigInt::from(minimum_trip),
        upper: modulus() - BigInt::from(1_u8),
    };
    intersect_bound(&mut bounds, bound, threshold)?;

    for range in ranges {
        let upper = range.max_elements.clone();
        if upper < BigInt::from(0_u8) {
            return Err("range element width exceeds the wasm address space".into());
        }
        match range.recipe.count {
            WasmRangeCount::TripBound(value) => {
                let value = expressions.roots.get(&value).copied().unwrap_or(value);
                if value != bound {
                    return Err("trip range count does not match the loop bound".into());
                }
                intersect_bound(
                    &mut bounds,
                    value,
                    Interval {
                        lower: BigInt::from(minimum_trip),
                        upper,
                    },
                )?;
            }
            WasmRangeCount::Invariant(value) => {
                let value = expressions.roots.get(&value).copied().unwrap_or(value);
                intersect_bound(
                    &mut bounds,
                    value,
                    Interval {
                        lower: BigInt::from(0_u8),
                        upper,
                    },
                )?;
            }
            WasmRangeCount::ScaledInvariant { .. } | WasmRangeCount::One => {}
        }
    }

    for range in ranges {
        let WasmRangeCount::ScaledInvariant { value, .. } = range.recipe.count else {
            continue;
        };
        let value = expressions.roots.get(&value).copied().unwrap_or(value);
        let independently_bounded = ranges.iter().any(|support| {
            let supporting_value = match support.recipe.count {
                WasmRangeCount::Invariant(value) | WasmRangeCount::TripBound(value) => value,
                WasmRangeCount::ScaledInvariant { .. } | WasmRangeCount::One => return false,
            };
            expressions
                .roots
                .get(&supporting_value)
                .copied()
                .unwrap_or(supporting_value)
                == value
        });
        if !independently_bounded {
            return Err("scaled range lacks a direct count guard for its source value".into());
        }
    }

    let mut scaled_exact = BTreeSet::new();
    for _ in 0..MAX_REFINEMENT_PASSES {
        let mut changed = false;
        for relation in &expressions.relations {
            let difference =
                ring_normalize(AffineForm::variable(relation.value).sub(&relation.expression))?;
            let interval = interval_of(&difference, &bounds)?;
            let Some(multiple) = unique_modular_multiple(&interval) else {
                let low = ceil_div(&interval.lower, &modulus());
                let high = floor_div(&interval.upper, &modulus());
                if low > high {
                    return Err("source modular relation is unreachable under the guards".into());
                }
                continue;
            };
            if !exact_equations
                .iter()
                .any(|existing| existing.difference == difference && existing.multiple == multiple)
            {
                exact_equations.push(ExactRelation {
                    difference: difference.clone(),
                    multiple,
                });
                changed = true;
            }
        }
        if exact_equations.len() > MAX_SOURCE_NODES {
            return Err("guarded equality set exceeded its work budget".into());
        }
        for equation in &exact_equations {
            changed |= propagate_relation_bounds(equation, &mut bounds)?;
        }
        for (ordinal, range) in ranges.iter().enumerate() {
            let WasmRangeCount::ScaledInvariant { value, scale } = range.recipe.count else {
                continue;
            };
            let value = expressions.roots.get(&value).copied().unwrap_or(value);
            if scale == 0 {
                return Err("scaled range count has zero scale".into());
            }
            let value_interval = bounds.get(&value).cloned().unwrap_or_else(Interval::u32);
            let mathematical_upper = &value_interval.upper * BigInt::from(scale);
            if mathematical_upper >= modulus() {
                continue;
            }
            scaled_exact.insert(ordinal);
            let guard_upper = floor_div(&range.max_elements, &BigInt::from(scale));
            changed |= intersect_bound(
                &mut bounds,
                value,
                Interval {
                    lower: value_interval.lower,
                    upper: guard_upper,
                },
            )?;
        }
        if !changed {
            break;
        }
    }
    for (ordinal, range) in ranges.iter().enumerate() {
        if matches!(range.recipe.count, WasmRangeCount::ScaledInvariant { .. })
            && !scaled_exact.contains(&ordinal)
        {
            return Err("scaled range count is not proved to avoid u32 wrap".into());
        }
    }
    let substitutions = exact_substitutions(&exact_equations)?;
    Ok((bounds, substitutions))
}

fn count_is_positive(
    range: &RangeFacts,
    bounds: &BTreeMap<ValueId, Interval>,
    substitutions: &BTreeMap<ValueId, AffineForm>,
) -> Result<bool, String> {
    let form = substitute_exact(
        &range.count_exact_expression,
        substitutions,
        &mut BTreeSet::new(),
    )?;
    Ok(interval_of(&form, bounds)?.lower > BigInt::from(0_u8))
}

fn source_setup_is_closed(
    function: &KirFunction,
    setup_ids: &BTreeSet<InstructionId>,
    accesses: &BTreeMap<InstructionId, RawAccess>,
) -> Result<(), String> {
    if setup_ids.len() > MAX_SOURCE_NODES {
        return Err("guarded address setup exceeds its closure budget".into());
    }
    let access_ids = accesses.keys().copied().collect::<BTreeSet<_>>();
    let definitions = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| (instruction.id, instruction))
        .collect::<BTreeMap<_, _>>();
    let values = setup_ids
        .iter()
        .flat_map(|instruction| {
            definitions
                .get(instruction)
                .into_iter()
                .flat_map(|instruction| instruction.results.iter().map(|result| result.value))
        })
        .collect::<BTreeSet<_>>();
    for value in values {
        for block in &function.blocks {
            for instruction in &block.instructions {
                let mut total_uses = 0_usize;
                super::analysis::visit_instruction_uses(instruction, &mut |operand| {
                    total_uses += usize::from(operand == value);
                });
                if total_uses == 0 {
                    continue;
                }
                if setup_ids.contains(&instruction.id) {
                    continue;
                }
                if access_ids.contains(&instruction.id) {
                    let index_uses = match &instruction.kind {
                        KirInstructionKind::Load { place }
                        | KirInstructionKind::Store { place, .. } => {
                            matches!(place.as_ref(), KirPlace::SliceIndex { index, slice, .. }
                                if *index == value && *slice != value)
                            .then_some(1_usize)
                            .unwrap_or(0)
                        }
                        _ => 0,
                    };
                    if total_uses == index_uses {
                        continue;
                    }
                }
                return Err(format!(
                    "source address setup value {value:?} escapes at instruction {:?}",
                    instruction.id
                ));
            }
            let terminator_uses = match &block.terminator {
                KirTerminator::Return { value: result, .. } => *result == Some(value),
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
            };
            if terminator_uses {
                return Err(format!(
                    "source address setup value {value:?} escapes through block {:?}",
                    block.id
                ));
            }
        }
    }
    Ok(())
}

fn prove_access_coverage(
    access: &RawAccess,
    ranges: &[RangeFacts],
    trip: ValueId,
    bounds: &BTreeMap<ValueId, Interval>,
    substitutions: &BTreeMap<ValueId, AffineForm>,
) -> Result<GuardedSpanAccessProof, String> {
    let trip = AffineForm::variable(trip);
    for range in ranges {
        if range.slice_root != access.access.slice {
            continue;
        }
        if range.recipe.element_bytes != access.element_bytes {
            continue;
        }
        let count = substitute_exact(
            &range.count_exact_expression,
            substitutions,
            &mut BTreeSet::new(),
        )?;
        let start = &range.start_expression;
        let offset = ring_normalize(access.access.origin_expression.sub(start))?;
        let offset = substitute_exact(&offset, substitutions, &mut BTreeSet::new())?;
        let offset = ring_normalize(offset)?;
        if !access.contiguous {
            let start_and_offset = ring_normalize(start.add(&offset))?;
            if start_and_offset != ring_normalize(access.access.origin_expression.clone())? {
                continue;
            }
            if !count_is_positive(range, bounds, substitutions)?
                || interval_of(&offset, bounds)?.lower < BigInt::from(0_u8)
            {
                continue;
            }
            let point_margin = substitute_exact(
                &count
                    .sub(&offset)
                    .sub(&AffineForm::integer(BigInt::from(1_u8))),
                substitutions,
                &mut BTreeSet::new(),
            )?;
            if interval_of(&point_margin, bounds)?.lower < BigInt::from(0_u8) {
                continue;
            }
            return Ok(GuardedSpanAccessProof {
                range_ordinal: range.ordinal,
                offset,
            });
        }
        if !count_is_positive(range, bounds, substitutions)? {
            continue;
        }
        let offset_interval = interval_of(&offset, bounds)?;
        if offset_interval.lower < BigInt::from(0_u8) {
            continue;
        }
        let end_margin = substitute_exact(
            &count.sub(&trip).sub(&offset),
            substitutions,
            &mut BTreeSet::new(),
        )?;
        if interval_of(&end_margin, bounds)?.lower < BigInt::from(0_u8) {
            continue;
        }
        // The source origin and the guarded start + offset are congruent. The
        // range predicate proves start + count is below 2^32/element_bytes;
        // the inequalities above place start + offset inside that nonempty
        // range, so this congruence has one unsigned lift and cannot wrap.
        return Ok(GuardedSpanAccessProof {
            range_ordinal: range.ordinal,
            offset,
        });
    }
    Err("no exact guarded range proves this source access".into())
}

/// Reconstructs modular affine source indices and proves that every source
/// slice access fits one of the supplied range recipes. `ranges` and
/// `minimum_trip` are conditional premises: callers must independently prove
/// that the plan and emitted predicate implement those exact checks before
/// accepting the vector trial.
#[allow(clippy::too_many_arguments)]
pub fn verify_guarded_span_coverage(
    function: &KirFunction,
    preheader: BlockId,
    header: BlockId,
    body: BlockId,
    induction: ValueId,
    body_induction: ValueId,
    bound: ValueId,
    minimum_trip: u32,
    ranges: &[WasmSliceRangeRequirement],
) -> Result<GuardedSpanCoverage, String> {
    if minimum_trip == 0 || ranges.len() > MAX_RANGE_RECIPES {
        return Err("guarded span exceeds its predicate budget".into());
    }
    let descriptor = analyze_canonical_loops(function)
        .loops
        .into_iter()
        .find(|descriptor| descriptor.header == header)
        .ok_or_else(|| "guarded source is not a canonical loop".to_string())?;
    let mut expressions =
        SourceExpressions::new(function, preheader, induction, &descriptor.blocks);
    let context = validate_loop_context(
        function,
        preheader,
        header,
        body,
        induction,
        body_induction,
        bound,
        &mut expressions,
    )?;
    let ranges = collect_ranges(&mut expressions, ranges, context.bound_root)?;
    let raw_accesses = source_accesses(function, &mut expressions, &context)?;
    source_setup_is_closed(function, &expressions.setup_ids, &raw_accesses)?;
    let (bounds, substitutions) =
        refine_relations_and_scaled_counts(&expressions, &ranges, bound, minimum_trip)?;

    let mut coverage = BTreeMap::new();
    let mut accesses = BTreeMap::new();
    for (instruction, access) in &raw_accesses {
        let proof = prove_access_coverage(
            access,
            &ranges,
            expressions.roots.get(&bound).copied().unwrap_or(bound),
            &bounds,
            &substitutions,
        )?;
        accesses.insert(*instruction, access.access.clone());
        coverage.insert(*instruction, proof);
    }
    Ok(GuardedSpanCoverage {
        accesses,
        setup: expressions.setup_ids,
        ranges: ranges.iter().map(|range| range.recipe).collect(),
        coverage,
    })
}
