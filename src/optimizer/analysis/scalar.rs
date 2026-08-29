use std::{collections::BTreeMap, error::Error, fmt};

use num_bigint::BigInt;

use crate::{
    BlockId, KirArithmeticSemantics, KirFunction, KirInstructionKind, KirTerminator, MirBinaryOp,
    MirCompareOp, MirPrimitiveTypeName, MirType, ValueId,
};

use super::super::facts::{
    FactArena, FactArenaError, FactDerivation, FactOrigin, FactPredicate, FactScope,
};
use super::{
    AffineForm, ScalarAnalysisBudget, ScalarAnalysisConfig, ScalarCongruence, ScalarKnownBits,
    mathematical_mod,
};

/// CK integer type identity used by the target-neutral scalar domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntegerType {
    I32,
    I64,
    U32,
    U64,
}

impl IntegerType {
    #[must_use]
    pub const fn from_mir(type_node: &MirType) -> Option<Self> {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => Some(Self::I32),
            MirType::Primitive(MirPrimitiveTypeName::I64) => Some(Self::I64),
            MirType::Primitive(MirPrimitiveTypeName::U32) => Some(Self::U32),
            MirType::Primitive(MirPrimitiveTypeName::U64) => Some(Self::U64),
            _ => None,
        }
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::U64 => 64,
        }
    }

    #[must_use]
    pub const fn is_signed(self) -> bool {
        matches!(self, Self::I32 | Self::I64)
    }

    #[must_use]
    pub const fn minimum_i128(self) -> i128 {
        match self {
            Self::I32 => i32::MIN as i128,
            Self::I64 => i64::MIN as i128,
            Self::U32 | Self::U64 => 0,
        }
    }

    #[must_use]
    pub const fn maximum_i128(self) -> i128 {
        match self {
            Self::I32 => i32::MAX as i128,
            Self::I64 => i64::MAX as i128,
            Self::U32 => u32::MAX as i128,
            Self::U64 => u64::MAX as i128,
        }
    }

    #[must_use]
    pub const fn contains_i128(self, value: i128) -> bool {
        value >= self.minimum_i128() && value <= self.maximum_i128()
    }

    #[must_use]
    pub const fn bit_mask(self) -> u64 {
        if self.bits() == 64 {
            u64::MAX
        } else {
            (1_u64 << self.bits()) - 1
        }
    }

    fn minimum(self) -> BigInt {
        BigInt::from(self.minimum_i128())
    }

    fn maximum(self) -> BigInt {
        BigInt::from(self.maximum_i128())
    }

    fn modulus(self) -> BigInt {
        BigInt::from(1_u8) << self.bits()
    }
}

/// Closed integer interval used by KIR scalar facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarInterval {
    lower: BigInt,
    upper: BigInt,
}

impl ScalarInterval {
    pub fn new(lower: BigInt, upper: BigInt) -> Result<Self, ScalarDomainError> {
        if lower > upper {
            return Err(ScalarDomainError::new(
                "interval lower bound exceeds upper bound",
            ));
        }
        Ok(Self { lower, upper })
    }

    #[must_use]
    pub const fn lower(&self) -> &BigInt {
        &self.lower
    }

    #[must_use]
    pub const fn upper(&self) -> &BigInt {
        &self.upper
    }

    fn exact(value: BigInt) -> Self {
        Self {
            lower: value.clone(),
            upper: value,
        }
    }

    fn for_type(type_node: IntegerType) -> Self {
        Self {
            lower: type_node.minimum(),
            upper: type_node.maximum(),
        }
    }

    fn intersection(&self, other: &Self) -> Option<Self> {
        let lower = self.lower.clone().max(other.lower.clone());
        let upper = self.upper.clone().min(other.upper.clone());
        (lower <= upper).then_some(Self { lower, upper })
    }

    fn hull(&self, other: &Self) -> Self {
        Self {
            lower: self.lower.clone().min(other.lower.clone()),
            upper: self.upper.clone().max(other.upper.clone()),
        }
    }

    fn is_exact(&self) -> bool {
        self.lower == self.upper
    }
}

/// Whether evaluating a scalar operation can trigger its ordered failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScalarFailure {
    None,
    May,
    Always,
}

/// Product domain used for integer SSA values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarValue {
    type_node: IntegerType,
    interval: ScalarInterval,
    congruence: ScalarCongruence,
    known_bits: ScalarKnownBits,
    affine: Option<AffineForm>,
    failure: ScalarFailure,
    unknown: bool,
}

impl ScalarValue {
    #[must_use]
    pub fn unknown(type_node: IntegerType) -> Self {
        Self {
            type_node,
            interval: ScalarInterval::for_type(type_node),
            congruence: ScalarCongruence::top(),
            known_bits: ScalarKnownBits::unknown(),
            affine: None,
            failure: ScalarFailure::None,
            unknown: true,
        }
    }

    pub fn constant(type_node: IntegerType, value: BigInt) -> Result<Self, ScalarDomainError> {
        ensure_in_type(type_node, &value)?;
        Ok(Self {
            type_node,
            interval: ScalarInterval::exact(value.clone()),
            congruence: ScalarCongruence::exact(value.clone()),
            known_bits: ScalarKnownBits::exact(&value, type_node),
            affine: Some(AffineForm::integer(value)),
            failure: ScalarFailure::None,
            unknown: false,
        })
    }

    pub fn from_interval(
        type_node: IntegerType,
        interval: ScalarInterval,
    ) -> Result<Self, ScalarDomainError> {
        ensure_in_type(type_node, interval.lower())?;
        ensure_in_type(type_node, interval.upper())?;
        let top = interval == ScalarInterval::for_type(type_node);
        let exact = interval.is_exact().then(|| interval.lower().clone());
        Ok(Self {
            type_node,
            congruence: exact.as_ref().map_or_else(ScalarCongruence::top, |value| {
                ScalarCongruence::exact(value.clone())
            }),
            known_bits: exact
                .as_ref()
                .map_or_else(ScalarKnownBits::unknown, |value| {
                    ScalarKnownBits::exact(value, type_node)
                }),
            affine: exact.map(AffineForm::integer),
            interval,
            failure: ScalarFailure::None,
            unknown: top,
        })
    }

    #[must_use]
    pub const fn type_node(&self) -> IntegerType {
        self.type_node
    }

    #[must_use]
    pub const fn interval(&self) -> &ScalarInterval {
        &self.interval
    }

    #[must_use]
    pub const fn congruence(&self) -> &ScalarCongruence {
        &self.congruence
    }

    #[must_use]
    pub const fn known_bits(&self) -> ScalarKnownBits {
        self.known_bits
    }

    #[must_use]
    pub const fn affine(&self) -> Option<&AffineForm> {
        self.affine.as_ref()
    }

    #[must_use]
    pub const fn failure(&self) -> ScalarFailure {
        self.failure
    }

    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        self.unknown
    }

    #[must_use]
    pub fn exact_value(&self) -> Option<&BigInt> {
        self.interval.is_exact().then_some(&self.interval.lower)
    }

    pub(crate) fn with_failure(mut self, failure: ScalarFailure) -> Self {
        self.failure = failure;
        self
    }
}

/// Three-valued result for an integer comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolLattice {
    AlwaysFalse,
    AlwaysTrue,
    Unknown,
}

/// Computes a local transfer without consulting the optimizing analyzer.
pub fn scalar_binary(
    op: MirBinaryOp,
    semantics: KirArithmeticSemantics,
    left: &ScalarValue,
    right: &ScalarValue,
) -> Result<ScalarValue, ScalarDomainError> {
    if left.type_node != right.type_node {
        return Err(ScalarDomainError::new(
            "binary operands have different integer types",
        ));
    }
    if semantics == KirArithmeticSemantics::StrictFloat {
        return Err(ScalarDomainError::new(
            "strict-float semantics cannot be applied to integer scalar values",
        ));
    }
    let type_node = left.type_node;
    let mathematical = mathematical_interval(op, &left.interval, &right.interval)?;
    let division_failure = division_failure(op, type_node, left, right);
    match semantics {
        KirArithmeticSemantics::Checked => {
            let range_failure = range_failure(type_node, &mathematical);
            let failure = division_failure.max(range_failure);
            let interval = mathematical
                .intersection(&ScalarInterval::for_type(type_node))
                .unwrap_or_else(|| ScalarInterval::for_type(type_node));
            let mut result = from_transfer_parts(op, left, right, interval, false)?;
            result.failure = failure;
            if failure == ScalarFailure::Always {
                result.unknown = true;
                result.affine = None;
            }
            Ok(result)
        }
        KirArithmeticSemantics::Modular => {
            if division_failure != ScalarFailure::None {
                return Ok(ScalarValue::unknown(type_node).with_failure(division_failure));
            }
            if range_failure(type_node, &mathematical) == ScalarFailure::None {
                return from_transfer_parts(op, left, right, mathematical, false);
            }
            if let (Some(left), Some(right)) = (left.exact_value(), right.exact_value()) {
                let raw = exact_binary(op, left, right)?;
                let wrapped = wrap_integer(type_node, raw);
                let mut result = ScalarValue::constant(type_node, wrapped)?;
                result.affine = None;
                return Ok(result);
            }
            Ok(ScalarValue::unknown(type_node))
        }
        KirArithmeticSemantics::StrictFloat => unreachable!("handled above"),
    }
}

pub fn scalar_compare(
    op: MirCompareOp,
    left: &ScalarValue,
    right: &ScalarValue,
) -> Result<BoolLattice, ScalarDomainError> {
    if left.type_node != right.type_node {
        return Err(ScalarDomainError::new(
            "comparison operands have different integer types",
        ));
    }
    let result = match op {
        MirCompareOp::Eq => {
            if left.interval.upper < right.interval.lower
                || right.interval.upper < left.interval.lower
            {
                BoolLattice::AlwaysFalse
            } else if left.exact_value().is_some() && left.exact_value() == right.exact_value() {
                BoolLattice::AlwaysTrue
            } else {
                BoolLattice::Unknown
            }
        }
        MirCompareOp::Ne => match scalar_compare(MirCompareOp::Eq, left, right)? {
            BoolLattice::AlwaysFalse => BoolLattice::AlwaysTrue,
            BoolLattice::AlwaysTrue => BoolLattice::AlwaysFalse,
            BoolLattice::Unknown => BoolLattice::Unknown,
        },
        MirCompareOp::Lt => compare_ordered(
            &left.interval.upper,
            &right.interval.lower,
            &left.interval.lower,
            &right.interval.upper,
            false,
        ),
        MirCompareOp::Le => compare_ordered(
            &left.interval.upper,
            &right.interval.lower,
            &left.interval.lower,
            &right.interval.upper,
            true,
        ),
        MirCompareOp::Gt => scalar_compare(MirCompareOp::Lt, right, left)?,
        MirCompareOp::Ge => scalar_compare(MirCompareOp::Le, right, left)?,
    };
    Ok(result)
}

pub type ScalarBranchRefinement = ((ScalarValue, ScalarValue), (ScalarValue, ScalarValue));

pub fn refine_scalar_comparison(
    op: MirCompareOp,
    left: &ScalarValue,
    right: &ScalarValue,
) -> Result<ScalarBranchRefinement, ScalarDomainError> {
    if left.type_node != right.type_node {
        return Err(ScalarDomainError::new(
            "comparison operands have different integer types",
        ));
    }
    let mut taken_left = left.clone();
    let mut taken_right = right.clone();
    let mut other_left = left.clone();
    let mut other_right = right.clone();
    match op {
        MirCompareOp::Lt => {
            refine_upper(&mut taken_left, right.interval.upper.clone() - 1)?;
            refine_lower(&mut taken_right, left.interval.lower.clone() + 1)?;
            refine_lower(&mut other_left, right.interval.lower.clone())?;
            refine_upper(&mut other_right, left.interval.upper.clone())?;
        }
        MirCompareOp::Le => {
            refine_upper(&mut taken_left, right.interval.upper.clone())?;
            refine_lower(&mut taken_right, left.interval.lower.clone())?;
            refine_lower(&mut other_left, right.interval.lower.clone() + 1)?;
            refine_upper(&mut other_right, left.interval.upper.clone() - 1)?;
        }
        MirCompareOp::Gt => {
            return refine_scalar_comparison(MirCompareOp::Lt, right, left).map(
                |((taken_right, taken_left), (other_right, other_left))| {
                    ((taken_left, taken_right), (other_left, other_right))
                },
            );
        }
        MirCompareOp::Ge => {
            return refine_scalar_comparison(MirCompareOp::Le, right, left).map(
                |((taken_right, taken_left), (other_right, other_left))| {
                    ((taken_left, taken_right), (other_left, other_right))
                },
            );
        }
        MirCompareOp::Eq => {
            if let Some(value) = right.exact_value() {
                taken_left = ScalarValue::constant(left.type_node, value.clone())?;
            }
            if let Some(value) = left.exact_value() {
                taken_right = ScalarValue::constant(right.type_node, value.clone())?;
            }
        }
        MirCompareOp::Ne => {
            let equality = refine_scalar_comparison(MirCompareOp::Eq, left, right)?;
            return Ok((equality.1, equality.0));
        }
    }
    Ok(((taken_left, taken_right), (other_left, other_right)))
}

pub fn widen_scalar(
    previous: &ScalarValue,
    next: &ScalarValue,
) -> Result<ScalarValue, ScalarDomainError> {
    ensure_same_type(previous, next)?;
    let lower = if next.interval.lower < previous.interval.lower {
        previous.type_node.minimum()
    } else {
        previous.interval.lower.clone()
    };
    let upper = if next.interval.upper > previous.interval.upper {
        previous.type_node.maximum()
    } else {
        previous.interval.upper.clone()
    };
    let mut result =
        ScalarValue::from_interval(previous.type_node, ScalarInterval { lower, upper })?;
    result.failure = previous.failure.max(next.failure);
    Ok(result)
}

pub fn narrow_scalar(
    widened: &ScalarValue,
    candidate: &ScalarValue,
) -> Result<ScalarValue, ScalarDomainError> {
    ensure_same_type(widened, candidate)?;
    let interval = widened
        .interval
        .intersection(&candidate.interval)
        .ok_or_else(|| ScalarDomainError::new("narrowing produced an empty interval"))?;
    let mut result = ScalarValue::from_interval(widened.type_node, interval)?;
    result.failure = widened.failure.max(candidate.failure);
    Ok(result)
}

/// Deterministic scalar-analysis output for one KIR function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarAnalysisResult {
    function: crate::FunctionId,
    config: ScalarAnalysisConfig,
    budget: ScalarAnalysisBudget,
    steps: u32,
    narrowing_iterations_run: u32,
    exhausted: bool,
    values: BTreeMap<ValueId, ScalarValue>,
    edge_values: ScalarEdgeValues,
}

pub type ScalarEdgeValues = BTreeMap<(BlockId, BlockId), BTreeMap<ValueId, ScalarValue>>;
type ScalarState = Vec<Option<ScalarValue>>;

impl ScalarAnalysisResult {
    #[must_use]
    pub const fn function(&self) -> crate::FunctionId {
        self.function
    }
    #[must_use]
    pub const fn budget(&self) -> ScalarAnalysisBudget {
        self.budget
    }

    #[must_use]
    pub const fn steps(&self) -> u32 {
        self.steps
    }

    #[must_use]
    pub const fn narrowing_iterations_run(&self) -> u32 {
        self.narrowing_iterations_run
    }

    #[must_use]
    pub const fn exhausted(&self) -> bool {
        self.exhausted
    }

    #[must_use]
    pub const fn values(&self) -> &BTreeMap<ValueId, ScalarValue> {
        &self.values
    }

    #[must_use]
    pub const fn edge_values(&self) -> &ScalarEdgeValues {
        &self.edge_values
    }

    #[must_use]
    pub const fn config(&self) -> ScalarAnalysisConfig {
        self.config
    }
}

pub fn analyze_scalar_function(
    function: &KirFunction,
    config: ScalarAnalysisConfig,
) -> Result<ScalarAnalysisResult, ScalarDomainError> {
    let budget = ScalarAnalysisBudget::for_function(function, config);
    let value_types = collect_integer_types(function);
    let incoming_edges = collect_incoming_edges(function);
    let unknown_values = || {
        value_types
            .iter()
            .map(|(value, type_node)| (*value, ScalarValue::unknown(*type_node)))
            .collect::<BTreeMap<_, _>>()
    };
    if budget.max_steps() == 0 {
        return Ok(ScalarAnalysisResult {
            function: function.id,
            config,
            budget,
            steps: 0,
            narrowing_iterations_run: 0,
            exhausted: true,
            values: unknown_values(),
            edge_values: BTreeMap::new(),
        });
    }

    let state_len = value_types
        .keys()
        .map(|value| value.index() as usize + 1)
        .max()
        .unwrap_or(0);
    let mut values = vec![None; state_len];
    for param in &function.params {
        if let Some(type_node) = IntegerType::from_mir(&param.type_node) {
            values[param.value.index() as usize] = Some(ScalarValue::unknown(type_node));
        }
    }
    let mut edge_values;
    let mut steps = 0_u32;
    let mut iteration = 0_u32;
    loop {
        let mut changed = false;
        let refinements = compute_edge_refinements(function, &values)?;
        edge_values = refinements.clone();
        for block in &function.blocks {
            for (index, param) in block.params.iter().enumerate() {
                let Some(type_node) = IntegerType::from_mir(&param.type_node) else {
                    continue;
                };
                let incoming = incoming_values(&incoming_edges, block.id, index)
                    .into_iter()
                    .map(|(predecessor, value)| {
                        refinements
                            .get(&(predecessor, block.id))
                            .and_then(|state| state.get(&value))
                            .or_else(|| scalar_value(&values, value))
                            .cloned()
                            .unwrap_or_else(|| ScalarValue::unknown(type_node))
                    })
                    .reduce(|left, right| join_scalar(&left, &right))
                    .unwrap_or_else(|| ScalarValue::unknown(type_node));
                let next = if iteration >= budget.widening_after() {
                    scalar_value(&values, param.value)
                        .map(|old| widen_scalar(old, &incoming))
                        .transpose()?
                        .unwrap_or(incoming)
                } else {
                    incoming
                };
                changed |= update_value(&mut values, param.value, next);
            }
            for instruction in &block.instructions {
                steps = steps.saturating_add(1);
                if steps > budget.max_steps() {
                    return Ok(ScalarAnalysisResult {
                        function: function.id,
                        config,
                        budget,
                        steps: budget.max_steps(),
                        narrowing_iterations_run: 0,
                        exhausted: true,
                        values: unknown_values(),
                        edge_values: BTreeMap::new(),
                    });
                }
                let Some(result) = instruction.results.first() else {
                    continue;
                };
                let Some(type_node) = IntegerType::from_mir(&result.type_node) else {
                    continue;
                };
                if !has_scalar_transfer(&instruction.kind) {
                    if scalar_value(&values, result.value).is_none() {
                        changed |= update_value(
                            &mut values,
                            result.value,
                            ScalarValue::unknown(type_node),
                        );
                    }
                    continue;
                }
                let next = transfer_instruction(&instruction.kind, type_node, &values)?;
                changed |= update_value(&mut values, result.value, next);
            }
        }
        iteration = iteration.saturating_add(1);
        if !changed {
            break;
        }
    }
    let mut narrowing_iterations_run = 0;
    for _ in 0..budget.narrowing_iterations() {
        let refinements = compute_edge_refinements(function, &values)?;
        edge_values = refinements.clone();
        for block in &function.blocks {
            for (index, param) in block.params.iter().enumerate() {
                let Some(type_node) = IntegerType::from_mir(&param.type_node) else {
                    continue;
                };
                let incoming = incoming_values(&incoming_edges, block.id, index)
                    .into_iter()
                    .map(|(predecessor, value)| {
                        refinements
                            .get(&(predecessor, block.id))
                            .and_then(|state| state.get(&value))
                            .or_else(|| scalar_value(&values, value))
                            .cloned()
                            .unwrap_or_else(|| ScalarValue::unknown(type_node))
                    })
                    .reduce(|left, right| join_scalar(&left, &right))
                    .unwrap_or_else(|| ScalarValue::unknown(type_node));
                let next = scalar_value(&values, param.value)
                    .map(|old| narrow_scalar(old, &incoming))
                    .transpose()?
                    .unwrap_or(incoming);
                update_value(&mut values, param.value, next);
            }
            for instruction in &block.instructions {
                steps = steps.saturating_add(1);
                if steps > budget.max_steps() {
                    return Ok(ScalarAnalysisResult {
                        function: function.id,
                        config,
                        budget,
                        steps: budget.max_steps(),
                        narrowing_iterations_run,
                        exhausted: true,
                        values: unknown_values(),
                        edge_values: BTreeMap::new(),
                    });
                }
                let Some(result) = instruction.results.first() else {
                    continue;
                };
                let Some(type_node) = IntegerType::from_mir(&result.type_node) else {
                    continue;
                };
                if !has_scalar_transfer(&instruction.kind) {
                    continue;
                }
                let next = transfer_instruction(&instruction.kind, type_node, &values)?;
                update_value(&mut values, result.value, next);
            }
        }
        narrowing_iterations_run += 1;
    }
    let values = value_types
        .iter()
        .map(|(value, type_node)| {
            (
                *value,
                scalar_value(&values, *value)
                    .cloned()
                    .unwrap_or_else(|| ScalarValue::unknown(*type_node)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(ScalarAnalysisResult {
        function: function.id,
        config,
        budget,
        steps,
        narrowing_iterations_run,
        exhausted: false,
        values,
        edge_values,
    })
}

/// Converts independently reproducible scalar conclusions into Proven facts.
pub fn materialize_scalar_facts(
    function: &KirFunction,
    analysis: &ScalarAnalysisResult,
    generation: u32,
) -> Result<FactArena, FactArenaError> {
    let mut facts = FactArena::new(generation);
    if analysis.function != function.id || analysis.exhausted {
        return Ok(facts);
    }
    let mut value_facts = BTreeMap::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            let Some(result) = instruction.results.first() else {
                continue;
            };
            let Some(value) = analysis.values.get(&result.value) else {
                continue;
            };
            let derivation = match &instruction.kind {
                KirInstructionKind::ConstInt { .. } => FactDerivation::Constant {
                    instruction: instruction.id,
                },
                KirInstructionKind::Binary { left, right, .. }
                    if value.failure == ScalarFailure::None =>
                {
                    let (Some(left), Some(right)) = (
                        value_facts.get(left).copied(),
                        value_facts.get(right).copied(),
                    ) else {
                        continue;
                    };
                    FactDerivation::BinaryTransfer {
                        instruction: instruction.id,
                        inputs: vec![left, right],
                    }
                }
                _ => continue,
            };
            let fact = facts.try_insert(
                FactOrigin::Proven,
                FactScope::Block {
                    function: function.id,
                    block: block.id,
                },
                FactPredicate::ValueInterval {
                    value: result.value,
                    interval: value.interval.clone(),
                },
                derivation,
            )?;
            value_facts.insert(result.value, fact);
        }
    }
    Ok(facts)
}

fn mathematical_interval(
    op: MirBinaryOp,
    left: &ScalarInterval,
    right: &ScalarInterval,
) -> Result<ScalarInterval, ScalarDomainError> {
    match op {
        MirBinaryOp::Add => {
            ScalarInterval::new(&left.lower + &right.lower, &left.upper + &right.upper)
        }
        MirBinaryOp::Sub => {
            ScalarInterval::new(&left.lower - &right.upper, &left.upper - &right.lower)
        }
        MirBinaryOp::Mul => {
            let products = [
                &left.lower * &right.lower,
                &left.lower * &right.upper,
                &left.upper * &right.lower,
                &left.upper * &right.upper,
            ];
            let lower = products.iter().min().cloned().unwrap_or_default();
            let upper = products.iter().max().cloned().unwrap_or_default();
            ScalarInterval::new(lower, upper)
        }
        MirBinaryOp::Div => {
            if right.lower <= BigInt::from(0_u8) && right.upper >= BigInt::from(0_u8) {
                return Ok(ScalarInterval {
                    lower: left.lower.clone().min(left.upper.clone()),
                    upper: left.lower.clone().max(left.upper.clone()),
                });
            }
            let quotients = [
                &left.lower / &right.lower,
                &left.lower / &right.upper,
                &left.upper / &right.lower,
                &left.upper / &right.upper,
            ];
            ScalarInterval::new(
                quotients.iter().min().cloned().unwrap_or_default(),
                quotients.iter().max().cloned().unwrap_or_default(),
            )
        }
        MirBinaryOp::Mod => {
            let maximum = right
                .lower
                .clone()
                .max(-right.lower.clone())
                .max(right.upper.clone())
                .max(-right.upper.clone());
            ScalarInterval::new(-maximum.clone() + 1, maximum - 1)
        }
    }
}

fn exact_binary(
    op: MirBinaryOp,
    left: &BigInt,
    right: &BigInt,
) -> Result<BigInt, ScalarDomainError> {
    match op {
        MirBinaryOp::Add => Ok(left + right),
        MirBinaryOp::Sub => Ok(left - right),
        MirBinaryOp::Mul => Ok(left * right),
        MirBinaryOp::Div => {
            if right == &BigInt::from(0_u8) {
                Err(ScalarDomainError::new("integer division by zero"))
            } else {
                Ok(left / right)
            }
        }
        MirBinaryOp::Mod => {
            if right == &BigInt::from(0_u8) {
                Err(ScalarDomainError::new("integer remainder by zero"))
            } else {
                Ok(left % right)
            }
        }
    }
}

fn from_transfer_parts(
    op: MirBinaryOp,
    left: &ScalarValue,
    right: &ScalarValue,
    interval: ScalarInterval,
    force_unknown: bool,
) -> Result<ScalarValue, ScalarDomainError> {
    let mut result = ScalarValue::from_interval(left.type_node, interval)?;
    result.congruence = match op {
        MirBinaryOp::Add => left.congruence.add(&right.congruence),
        MirBinaryOp::Sub => left.congruence.sub(&right.congruence),
        MirBinaryOp::Mul => left.congruence.multiply(&right.congruence),
        MirBinaryOp::Div | MirBinaryOp::Mod => ScalarCongruence::top(),
    };
    result.affine = transfer_affine(op, left, right);
    result.unknown |= force_unknown;
    Ok(result)
}

fn transfer_affine(op: MirBinaryOp, left: &ScalarValue, right: &ScalarValue) -> Option<AffineForm> {
    match op {
        MirBinaryOp::Add => Some(left.affine.as_ref()?.add(right.affine.as_ref()?)),
        MirBinaryOp::Sub => Some(left.affine.as_ref()?.sub(right.affine.as_ref()?)),
        MirBinaryOp::Mul => {
            if let Some(constant) = left.exact_value() {
                return Some(right.affine.as_ref()?.scale(constant));
            }
            right
                .exact_value()
                .and_then(|constant| left.affine.as_ref().map(|form| form.scale(constant)))
        }
        MirBinaryOp::Div | MirBinaryOp::Mod => None,
    }
}

fn division_failure(
    op: MirBinaryOp,
    type_node: IntegerType,
    left: &ScalarValue,
    right: &ScalarValue,
) -> ScalarFailure {
    if !matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod) {
        return ScalarFailure::None;
    }
    let zero = BigInt::from(0_u8);
    let can_zero = right.interval.lower <= zero && right.interval.upper >= zero;
    let always_zero = right.exact_value() == Some(&zero);
    let signed_overflow = type_node.is_signed()
        && left.interval.lower <= type_node.minimum()
        && left.interval.upper >= type_node.minimum()
        && right.interval.lower <= BigInt::from(-1_i8)
        && right.interval.upper >= BigInt::from(-1_i8);
    if always_zero
        || (left.exact_value() == Some(&type_node.minimum())
            && right.exact_value() == Some(&BigInt::from(-1_i8)))
    {
        ScalarFailure::Always
    } else if can_zero || signed_overflow {
        ScalarFailure::May
    } else {
        ScalarFailure::None
    }
}

fn range_failure(type_node: IntegerType, interval: &ScalarInterval) -> ScalarFailure {
    let minimum = type_node.minimum();
    let maximum = type_node.maximum();
    if interval.upper < minimum || interval.lower > maximum {
        ScalarFailure::Always
    } else if interval.lower < minimum || interval.upper > maximum {
        ScalarFailure::May
    } else {
        ScalarFailure::None
    }
}

fn wrap_integer(type_node: IntegerType, value: BigInt) -> BigInt {
    let unsigned = mathematical_mod(value, &type_node.modulus());
    if type_node.is_signed() && unsigned > type_node.maximum() {
        unsigned - type_node.modulus()
    } else {
        unsigned
    }
}

fn compare_ordered(
    greatest_left: &BigInt,
    least_right: &BigInt,
    least_left: &BigInt,
    greatest_right: &BigInt,
    allow_equal: bool,
) -> BoolLattice {
    let always = if allow_equal {
        greatest_left <= least_right
    } else {
        greatest_left < least_right
    };
    let impossible = if allow_equal {
        least_left > greatest_right
    } else {
        least_left >= greatest_right
    };
    if always {
        BoolLattice::AlwaysTrue
    } else if impossible {
        BoolLattice::AlwaysFalse
    } else {
        BoolLattice::Unknown
    }
}

fn refine_lower(value: &mut ScalarValue, lower: BigInt) -> Result<(), ScalarDomainError> {
    let lower = lower.max(value.interval.lower.clone());
    if lower <= value.interval.upper {
        *value = ScalarValue::from_interval(
            value.type_node,
            ScalarInterval {
                lower,
                upper: value.interval.upper.clone(),
            },
        )?;
    }
    Ok(())
}

fn refine_upper(value: &mut ScalarValue, upper: BigInt) -> Result<(), ScalarDomainError> {
    let upper = upper.min(value.interval.upper.clone());
    if value.interval.lower <= upper {
        *value = ScalarValue::from_interval(
            value.type_node,
            ScalarInterval {
                lower: value.interval.lower.clone(),
                upper,
            },
        )?;
    }
    Ok(())
}

fn ensure_same_type(left: &ScalarValue, right: &ScalarValue) -> Result<(), ScalarDomainError> {
    if left.type_node == right.type_node {
        Ok(())
    } else {
        Err(ScalarDomainError::new(
            "scalar values have different integer types",
        ))
    }
}

fn ensure_in_type(type_node: IntegerType, value: &BigInt) -> Result<(), ScalarDomainError> {
    if value < &type_node.minimum() || value > &type_node.maximum() {
        return Err(ScalarDomainError::new(format!(
            "integer {value} does not fit {type_node:?}"
        )));
    }
    Ok(())
}

fn collect_integer_types(function: &KirFunction) -> BTreeMap<ValueId, IntegerType> {
    function
        .params
        .iter()
        .map(|param| (param.value, &param.type_node))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, &param.type_node))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, &result.type_node))
                }))
        }))
        .filter_map(|(value, type_node)| {
            IntegerType::from_mir(type_node).map(|type_node| (value, type_node))
        })
        .collect()
}

type IncomingEdges = BTreeMap<BlockId, Vec<(BlockId, Vec<ValueId>)>>;

fn collect_incoming_edges(function: &KirFunction) -> IncomingEdges {
    let mut incoming = IncomingEdges::new();
    for block in &function.blocks {
        let edges = match &block.terminator {
            KirTerminator::Return { .. } => Vec::new(),
            KirTerminator::Jump { edge } => vec![edge],
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
        };
        for edge in edges {
            incoming
                .entry(edge.target)
                .or_default()
                .push((block.id, edge.args.clone()));
        }
    }
    incoming
}

fn incoming_values(
    incoming_edges: &IncomingEdges,
    target: BlockId,
    index: usize,
) -> Vec<(BlockId, ValueId)> {
    incoming_edges
        .get(&target)
        .into_iter()
        .flatten()
        .filter_map(|(predecessor, args)| {
            args.get(index).copied().map(|value| (*predecessor, value))
        })
        .collect()
}

fn transfer_instruction(
    instruction: &KirInstructionKind,
    type_node: IntegerType,
    values: &ScalarState,
) -> Result<ScalarValue, ScalarDomainError> {
    match instruction {
        KirInstructionKind::ConstInt { value } => value
            .parse::<BigInt>()
            .map_err(|_| ScalarDomainError::new(format!("invalid KIR integer '{value}'")))
            .and_then(|value| ScalarValue::constant(type_node, value)),
        KirInstructionKind::Copy { value } => Ok(values
            .get(value.index() as usize)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_else(|| ScalarValue::unknown(type_node))),
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } => match (scalar_value(values, *left), scalar_value(values, *right)) {
            (Some(left), Some(right)) => scalar_binary(*op, *semantics, left, right),
            _ => Ok(ScalarValue::unknown(type_node)),
        },
        _ => Ok(ScalarValue::unknown(type_node)),
    }
}

fn has_scalar_transfer(instruction: &KirInstructionKind) -> bool {
    matches!(
        instruction,
        KirInstructionKind::ConstInt { .. }
            | KirInstructionKind::Copy { .. }
            | KirInstructionKind::Binary { .. }
    )
}

fn update_value(values: &mut ScalarState, value: ValueId, next: ScalarValue) -> bool {
    let index = value.index() as usize;
    if values.get(index).and_then(Option::as_ref) == Some(&next) {
        false
    } else {
        if index >= values.len() {
            values.resize(index + 1, None);
        }
        values[index] = Some(next);
        true
    }
}

fn scalar_value(values: &ScalarState, value: ValueId) -> Option<&ScalarValue> {
    values.get(value.index() as usize).and_then(Option::as_ref)
}

fn join_scalar(left: &ScalarValue, right: &ScalarValue) -> ScalarValue {
    if left.type_node != right.type_node {
        return ScalarValue::unknown(left.type_node);
    }
    let mut joined =
        ScalarValue::from_interval(left.type_node, left.interval.hull(&right.interval))
            .unwrap_or_else(|_| ScalarValue::unknown(left.type_node));
    joined.failure = left.failure.max(right.failure);
    joined
}

type ComparisonMap = BTreeMap<ValueId, (MirCompareOp, ValueId, ValueId)>;

fn compute_edge_refinements(
    function: &KirFunction,
    values: &ScalarState,
) -> Result<ScalarEdgeValues, ScalarDomainError> {
    let comparisons = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| {
            let KirInstructionKind::Compare { op, left, right } = instruction.kind else {
                return None;
            };
            instruction
                .results
                .first()
                .map(|result| (result.value, (op, left, right)))
        })
        .collect::<ComparisonMap>();
    let mut result = BTreeMap::new();
    for block in &function.blocks {
        let KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } = &block.terminator
        else {
            continue;
        };
        let Some((op, left, right)) = comparisons.get(condition) else {
            continue;
        };
        let (Some(left_value), Some(right_value)) =
            (scalar_value(values, *left), scalar_value(values, *right))
        else {
            continue;
        };
        let (taken, other) = refine_scalar_comparison(*op, left_value, right_value)?;
        result.insert(
            (block.id, then_edge.target),
            BTreeMap::from([(*left, taken.0), (*right, taken.1)]),
        );
        result.insert(
            (block.id, else_edge.target),
            BTreeMap::from([(*left, other.0), (*right, other.1)]),
        );
    }
    Ok(result)
}

/// Error produced by malformed inputs to the closed scalar domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarDomainError {
    message: String,
}

impl ScalarDomainError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ScalarDomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for ScalarDomainError {}
