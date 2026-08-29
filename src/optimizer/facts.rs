use std::{collections::BTreeMap, error::Error, fmt};

use num_bigint::BigInt;

use crate::{
    BlockId, CheckedAffineExpression, CheckedAffineTerm, CheckedContract, CheckedContractPointer,
    CheckedContractPredicate, CheckedProgram, ContractEffectKind, FactId, FunctionId,
    InstructionId, KirFunction, KirInstructionKind, KirModule, ValueId,
};

use super::ScalarInterval;

/// Stable identity for one dynamic unsafe-contract import boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContractInstanceId(u32);

impl ContractInstanceId {
    /// Constructs an identity from its deterministic arena index.
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        Self(index)
    }

    /// Returns the deterministic arena index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Trust boundary for a compiler fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactOrigin {
    /// Locally derived and independently checkable.
    Proven,
    /// Imported from one specific unsafe contract instance.
    TrustedContract { instance: ContractInstanceId },
}

/// Region of KIR in which a fact is allowed to dominate uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactScope {
    FunctionEntry(FunctionId),
    Block {
        function: FunctionId,
        block: BlockId,
    },
    CalleeInstance {
        instance: ContractInstanceId,
        callee: FunctionId,
    },
    InlineClone {
        function: FunctionId,
        clone: u32,
        blocks: Vec<BlockId>,
    },
}

/// Closed fact predicate language. Later stages extend this enum with memory facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactPredicate {
    ValueInterval {
        value: ValueId,
        interval: ScalarInterval,
    },
    Contract(ContractFactPredicate),
}

/// A checked contract predicate after parameter-to-SSA substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractFactPredicate {
    Comparison {
        operator: String,
        left: ContractFactAffineExpression,
        right: ContractFactAffineExpression,
    },
    MultipleOf {
        value: ContractFactAffineExpression,
        modulus: BigInt,
    },
    NoAlias {
        left: ValueId,
        right: ValueId,
    },
    Aligned {
        pointer: ContractFactPointer,
        alignment: u32,
    },
    EffectCeiling {
        is_none: bool,
        items: Vec<(ValueId, ContractEffectKind)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFactAffineExpression {
    pub terms: Vec<ContractFactAffineTermCoefficient>,
    pub constant: BigInt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFactAffineTermCoefficient {
    pub term: ContractFactAffineTerm,
    pub coefficient: BigInt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContractFactAffineTerm {
    Value(ValueId),
    SliceLength(ValueId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractFactPointer {
    Value(ValueId),
    SliceData(ValueId),
}

/// Locally checkable source of a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactDerivation {
    TrustedContractLeaf,
    Constant {
        instruction: InstructionId,
    },
    BinaryTransfer {
        instruction: InstructionId,
        inputs: Vec<FactId>,
    },
    BranchRefinement {
        predecessor: BlockId,
        comparison: InstructionId,
        input: FactId,
        taken: bool,
    },
    LoopInvariant {
        header: BlockId,
        entry: FactId,
        transfer: FactId,
    },
}

impl FactDerivation {
    fn dependencies(&self) -> Vec<FactId> {
        match self {
            Self::TrustedContractLeaf | Self::Constant { .. } => Vec::new(),
            Self::BinaryTransfer { inputs, .. } => inputs.clone(),
            Self::BranchRefinement { input, .. } => vec![*input],
            Self::LoopInvariant {
                entry, transfer, ..
            } => vec![*entry, *transfer],
        }
    }
}

/// One immutable entry in a deterministic fact arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub id: FactId,
    pub origin: FactOrigin,
    pub scope: FactScope,
    pub predicate: FactPredicate,
    pub derivation: FactDerivation,
    pub generation: u32,
}

/// Append-only fact storage. Its vector order is its serialization order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactArena {
    generation: u32,
    facts: Vec<Fact>,
}

impl FactArena {
    /// Creates an arena tied to a KIR analysis generation.
    #[must_use]
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            facts: Vec::new(),
        }
    }

    /// Returns the KIR generation against which these facts were derived.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Returns all facts in stable ID order.
    #[must_use]
    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }

    /// Gets a fact only when its ID belongs to this arena.
    #[must_use]
    pub fn get(&self, id: FactId) -> Option<&Fact> {
        self.facts.get(id.index() as usize)
    }

    /// Mutably accesses an entry for explicit invalidation and verifier mutation tests.
    pub fn get_mut(&mut self, id: FactId) -> Option<&mut Fact> {
        self.facts.get_mut(id.index() as usize)
    }

    /// Appends a fact after checking that every dependency is already defined.
    pub fn try_insert(
        &mut self,
        origin: FactOrigin,
        scope: FactScope,
        predicate: FactPredicate,
        derivation: FactDerivation,
    ) -> Result<FactId, FactArenaError> {
        for dependency in derivation.dependencies() {
            if self.get(dependency).is_none() {
                return Err(FactArenaError::new(format!(
                    "fact dependency fact{} is not already defined",
                    dependency.index()
                )));
            }
        }
        let index = u32::try_from(self.facts.len())
            .map_err(|_| FactArenaError::new("fact arena exceeds u32 identity space"))?;
        let id = FactId::from_index(index);
        self.facts.push(Fact {
            id,
            origin,
            scope,
            predicate,
            derivation,
            generation: self.generation,
        });
        Ok(id)
    }
}

/// Error produced while constructing a fact arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactArenaError {
    message: String,
}

impl FactArenaError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for FactArenaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for FactArenaError {}

/// Stable parameter binding used to validate a trusted contract instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFactBinding {
    pub parameter: String,
    pub value: ValueId,
}

/// Structural source of a contract instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractInstanceSource {
    FunctionEntry,
    Call {
        caller: FunctionId,
        block: BlockId,
        instruction: InstructionId,
    },
    InlineClone {
        source: ContractInstanceId,
        function: FunctionId,
        clone: u32,
    },
}

/// One entry or call-specific import of an unsafe function contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFactInstance {
    pub id: ContractInstanceId,
    pub callee: FunctionId,
    pub source: ContractInstanceSource,
    pub bindings: Vec<ContractFactBinding>,
    pub facts: Vec<FactId>,
}

/// Contract facts plus their structural import records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFactSet {
    facts: FactArena,
    instances: Vec<ContractFactInstance>,
}

impl ContractFactSet {
    #[must_use]
    pub const fn facts(&self) -> &FactArena {
        &self.facts
    }

    #[must_use]
    pub fn instances(&self) -> &[ContractFactInstance] {
        &self.instances
    }
}

/// Location at which a contract fact is proposed for use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactUseSite {
    pub function: FunctionId,
    pub block: BlockId,
    pub instruction: Option<InstructionId>,
    pub contract_instance: Option<ContractInstanceId>,
}

/// Imports entry and per-call unsafe contracts without changing semantic MIR.
pub fn import_contract_facts(
    module: &KirModule,
    checked_program: &CheckedProgram,
    generation: u32,
) -> Result<ContractFactSet, ContractFactImportError> {
    let mut imported = ContractFactSet {
        facts: FactArena::new(generation),
        instances: Vec::new(),
    };
    let function_ids = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function.id))
        .collect::<BTreeMap<_, _>>();

    for function in &module.functions {
        let Some(info) = checked_program.function_map.get(&function.name) else {
            return Err(ContractFactImportError::new(format!(
                "KIR function '{}' has no checked source metadata",
                function.name
            )));
        };
        if !info.is_unsafe {
            continue;
        }
        let bindings = bind_entry(function, &info.params)?;
        append_contract_instance(
            &mut imported,
            function.id,
            ContractInstanceSource::FunctionEntry,
            bindings,
            info.contract.as_ref(),
            FactScope::FunctionEntry(function.id),
        )?;
    }

    for caller in &module.functions {
        for block in &caller.blocks {
            for instruction in &block.instructions {
                let KirInstructionKind::Call {
                    function_name,
                    args,
                } = &instruction.kind
                else {
                    continue;
                };
                let Some(info) = checked_program.function_map.get(function_name) else {
                    continue;
                };
                if !info.is_unsafe {
                    continue;
                }
                let callee = function_ids
                    .get(function_name.as_str())
                    .copied()
                    .ok_or_else(|| {
                        ContractFactImportError::new(format!(
                            "unsafe call '{}' has no reachable KIR callee",
                            function_name
                        ))
                    })?;
                let bindings = bind_call(args, &info.params)?;
                let next_id = next_contract_instance_id(&imported)?;
                append_contract_instance(
                    &mut imported,
                    callee,
                    ContractInstanceSource::Call {
                        caller: caller.id,
                        block: block.id,
                        instruction: instruction.id,
                    },
                    bindings,
                    info.contract.as_ref(),
                    FactScope::CalleeInstance {
                        instance: next_id,
                        callee,
                    },
                )?;
            }
        }
    }
    Ok(imported)
}

/// Clones a dynamic contract instance into exactly the blocks created by inlining.
pub fn clone_contract_instance_for_inline(
    imported: &mut ContractFactSet,
    source: ContractInstanceId,
    function: FunctionId,
    clone: u32,
    mut blocks: Vec<BlockId>,
    value_map: &BTreeMap<ValueId, ValueId>,
) -> Result<ContractInstanceId, ContractFactImportError> {
    let source_instance = imported
        .instances
        .get(source.index() as usize)
        .filter(|instance| instance.id == source)
        .cloned()
        .ok_or_else(|| {
            ContractFactImportError::new(format!(
                "contract instance ci{} is not defined",
                source.index()
            ))
        })?;
    blocks.sort_unstable();
    blocks.dedup();
    let id = next_contract_instance_id(imported)?;
    let scope = FactScope::InlineClone {
        function,
        clone,
        blocks,
    };
    let bindings = source_instance
        .bindings
        .iter()
        .map(|binding| ContractFactBinding {
            parameter: binding.parameter.clone(),
            value: remap_value(binding.value, value_map),
        })
        .collect::<Vec<_>>();
    let mut fact_ids = Vec::new();
    for source_fact in source_instance
        .facts
        .iter()
        .filter_map(|fact| imported.facts.get(*fact))
        .cloned()
        .collect::<Vec<_>>()
    {
        let predicate = remap_predicate(source_fact.predicate, value_map);
        fact_ids.push(
            imported
                .facts
                .try_insert(
                    FactOrigin::TrustedContract { instance: id },
                    scope.clone(),
                    predicate,
                    FactDerivation::TrustedContractLeaf,
                )
                .map_err(ContractFactImportError::from)?,
        );
    }
    imported.instances.push(ContractFactInstance {
        id,
        callee: source_instance.callee,
        source: ContractInstanceSource::InlineClone {
            source,
            function,
            clone,
        },
        bindings,
        facts: fact_ids,
    });
    Ok(id)
}

/// Checks the lexical/dynamic scope boundary without re-proving a contract predicate.
#[must_use]
pub fn contract_fact_dominates_at(
    imported: &ContractFactSet,
    fact: FactId,
    site: FactUseSite,
) -> bool {
    let Some(fact) = imported.facts.get(fact) else {
        return false;
    };
    let FactOrigin::TrustedContract { instance } = fact.origin else {
        return false;
    };
    if site.contract_instance.is_some() && site.contract_instance != Some(instance) {
        return false;
    }
    match &fact.scope {
        FactScope::FunctionEntry(function) => *function == site.function,
        FactScope::Block { function, block } => *function == site.function && *block == site.block,
        FactScope::CalleeInstance {
            instance: scoped,
            callee,
        } => {
            *callee == site.function
                && *scoped == instance
                && site.contract_instance == Some(instance)
        }
        FactScope::InlineClone {
            function, blocks, ..
        } => {
            *function == site.function
                && site.contract_instance == Some(instance)
                && blocks.binary_search(&site.block).is_ok()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFactImportError {
    message: String,
}

impl ContractFactImportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<FactArenaError> for ContractFactImportError {
    fn from(error: FactArenaError) -> Self {
        Self::new(error.message)
    }
}

impl fmt::Display for ContractFactImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for ContractFactImportError {}

fn bind_entry(
    function: &KirFunction,
    params: &[crate::FunctionParamInfo],
) -> Result<Vec<ContractFactBinding>, ContractFactImportError> {
    if function.params.len() != params.len() {
        return Err(ContractFactImportError::new(format!(
            "unsafe function '{}' parameter metadata does not match KIR",
            function.name
        )));
    }
    Ok(params
        .iter()
        .zip(&function.params)
        .map(|(source, kir)| ContractFactBinding {
            parameter: source.name.clone(),
            value: kir.value,
        })
        .collect())
}

fn bind_call(
    args: &[ValueId],
    params: &[crate::FunctionParamInfo],
) -> Result<Vec<ContractFactBinding>, ContractFactImportError> {
    if args.len() != params.len() {
        return Err(ContractFactImportError::new(
            "unsafe call argument count does not match checked contract",
        ));
    }
    Ok(params
        .iter()
        .zip(args)
        .map(|(param, value)| ContractFactBinding {
            parameter: param.name.clone(),
            value: *value,
        })
        .collect())
}

fn next_contract_instance_id(
    imported: &ContractFactSet,
) -> Result<ContractInstanceId, ContractFactImportError> {
    u32::try_from(imported.instances.len())
        .map(ContractInstanceId::from_index)
        .map_err(|_| ContractFactImportError::new("contract instance arena exceeds u32 space"))
}

fn append_contract_instance(
    imported: &mut ContractFactSet,
    callee: FunctionId,
    source: ContractInstanceSource,
    bindings: Vec<ContractFactBinding>,
    contract: Option<&CheckedContract>,
    scope: FactScope,
) -> Result<ContractInstanceId, ContractFactImportError> {
    let id = next_contract_instance_id(imported)?;
    let values = bindings
        .iter()
        .map(|binding| (binding.parameter.as_str(), binding.value))
        .collect::<BTreeMap<_, _>>();
    let mut predicates = Vec::new();
    if let Some(contract) = contract {
        for predicate in &contract.predicates {
            lower_contract_predicate(predicate, &values, &mut predicates)?;
        }
        if let Some(effects) = &contract.effects {
            let items = effects
                .items
                .iter()
                .map(|(name, effect)| {
                    values
                        .get(name.as_str())
                        .copied()
                        .map(|value| (value, *effect))
                        .ok_or_else(|| missing_binding(name))
                })
                .collect::<Result<Vec<_>, _>>()?;
            predicates.push(ContractFactPredicate::EffectCeiling {
                is_none: effects.is_none,
                items,
            });
        }
    }
    let mut facts = Vec::new();
    for predicate in predicates {
        facts.push(
            imported
                .facts
                .try_insert(
                    FactOrigin::TrustedContract { instance: id },
                    scope.clone(),
                    FactPredicate::Contract(predicate),
                    FactDerivation::TrustedContractLeaf,
                )
                .map_err(ContractFactImportError::from)?,
        );
    }
    imported.instances.push(ContractFactInstance {
        id,
        callee,
        source,
        bindings,
        facts,
    });
    Ok(id)
}

fn lower_contract_predicate(
    predicate: &CheckedContractPredicate,
    bindings: &BTreeMap<&str, ValueId>,
    output: &mut Vec<ContractFactPredicate>,
) -> Result<(), ContractFactImportError> {
    match predicate {
        CheckedContractPredicate::Comparison {
            operator,
            left,
            right,
        } => output.push(ContractFactPredicate::Comparison {
            operator: operator.clone(),
            left: lower_affine(left, bindings)?,
            right: lower_affine(right, bindings)?,
        }),
        CheckedContractPredicate::Conjunction(items) => {
            for item in items {
                lower_contract_predicate(item, bindings, output)?;
            }
        }
        CheckedContractPredicate::MultipleOf { value, modulus } => {
            output.push(ContractFactPredicate::MultipleOf {
                value: lower_affine(value, bindings)?,
                modulus: parse_bigint(modulus)?,
            });
        }
        CheckedContractPredicate::NoAlias { left, right } => {
            output.push(ContractFactPredicate::NoAlias {
                left: lookup_binding(bindings, left)?,
                right: lookup_binding(bindings, right)?,
            });
        }
        CheckedContractPredicate::Aligned { pointer, alignment } => {
            let pointer = match pointer {
                CheckedContractPointer::Parameter(name) => {
                    ContractFactPointer::Value(lookup_binding(bindings, name)?)
                }
                CheckedContractPointer::SliceData(name) => {
                    ContractFactPointer::SliceData(lookup_binding(bindings, name)?)
                }
            };
            output.push(ContractFactPredicate::Aligned {
                pointer,
                alignment: *alignment,
            });
        }
    }
    Ok(())
}

fn lower_affine(
    expression: &CheckedAffineExpression,
    bindings: &BTreeMap<&str, ValueId>,
) -> Result<ContractFactAffineExpression, ContractFactImportError> {
    let terms = expression
        .terms
        .iter()
        .map(|term| {
            let value = match &term.term {
                CheckedAffineTerm::Parameter(name) => {
                    ContractFactAffineTerm::Value(lookup_binding(bindings, name)?)
                }
                CheckedAffineTerm::SliceLength(name) => {
                    ContractFactAffineTerm::SliceLength(lookup_binding(bindings, name)?)
                }
            };
            Ok(ContractFactAffineTermCoefficient {
                term: value,
                coefficient: parse_bigint(&term.coefficient)?,
            })
        })
        .collect::<Result<Vec<_>, ContractFactImportError>>()?;
    Ok(ContractFactAffineExpression {
        terms,
        constant: parse_bigint(&expression.constant)?,
    })
}

fn lookup_binding(
    bindings: &BTreeMap<&str, ValueId>,
    name: &str,
) -> Result<ValueId, ContractFactImportError> {
    bindings
        .get(name)
        .copied()
        .ok_or_else(|| missing_binding(name))
}

fn missing_binding(name: &str) -> ContractFactImportError {
    ContractFactImportError::new(format!("contract parameter '{name}' has no KIR binding"))
}

fn parse_bigint(text: &str) -> Result<BigInt, ContractFactImportError> {
    text.parse::<BigInt>().map_err(|_| {
        ContractFactImportError::new(format!("checked contract integer '{text}' is malformed"))
    })
}

fn remap_value(value: ValueId, value_map: &BTreeMap<ValueId, ValueId>) -> ValueId {
    value_map.get(&value).copied().unwrap_or(value)
}

fn remap_predicate(
    predicate: FactPredicate,
    value_map: &BTreeMap<ValueId, ValueId>,
) -> FactPredicate {
    match predicate {
        FactPredicate::ValueInterval { value, interval } => FactPredicate::ValueInterval {
            value: remap_value(value, value_map),
            interval,
        },
        FactPredicate::Contract(contract) => {
            FactPredicate::Contract(remap_contract_predicate(contract, value_map))
        }
    }
}

fn remap_contract_predicate(
    predicate: ContractFactPredicate,
    value_map: &BTreeMap<ValueId, ValueId>,
) -> ContractFactPredicate {
    match predicate {
        ContractFactPredicate::Comparison {
            operator,
            left,
            right,
        } => ContractFactPredicate::Comparison {
            operator,
            left: remap_affine(left, value_map),
            right: remap_affine(right, value_map),
        },
        ContractFactPredicate::MultipleOf { value, modulus } => ContractFactPredicate::MultipleOf {
            value: remap_affine(value, value_map),
            modulus,
        },
        ContractFactPredicate::NoAlias { left, right } => ContractFactPredicate::NoAlias {
            left: remap_value(left, value_map),
            right: remap_value(right, value_map),
        },
        ContractFactPredicate::Aligned { pointer, alignment } => {
            let pointer = match pointer {
                ContractFactPointer::Value(value) => {
                    ContractFactPointer::Value(remap_value(value, value_map))
                }
                ContractFactPointer::SliceData(value) => {
                    ContractFactPointer::SliceData(remap_value(value, value_map))
                }
            };
            ContractFactPredicate::Aligned { pointer, alignment }
        }
        ContractFactPredicate::EffectCeiling { is_none, items } => {
            ContractFactPredicate::EffectCeiling {
                is_none,
                items: items
                    .into_iter()
                    .map(|(value, effect)| (remap_value(value, value_map), effect))
                    .collect(),
            }
        }
    }
}

fn remap_affine(
    expression: ContractFactAffineExpression,
    value_map: &BTreeMap<ValueId, ValueId>,
) -> ContractFactAffineExpression {
    ContractFactAffineExpression {
        terms: expression
            .terms
            .into_iter()
            .map(|term| ContractFactAffineTermCoefficient {
                term: match term.term {
                    ContractFactAffineTerm::Value(value) => {
                        ContractFactAffineTerm::Value(remap_value(value, value_map))
                    }
                    ContractFactAffineTerm::SliceLength(value) => {
                        ContractFactAffineTerm::SliceLength(remap_value(value, value_map))
                    }
                },
                coefficient: term.coefficient,
            })
            .collect(),
        constant: expression.constant,
    }
}

/// Serializes facts without hash iteration, paths, addresses, or timestamps.
#[must_use]
pub fn print_fact_arena(arena: &FactArena) -> String {
    let mut output = format!("facts generation={}\n", arena.generation);
    for fact in &arena.facts {
        output.push_str(&format!(
            "fact{} {} scope={} {} <- {}\n",
            fact.id.index(),
            print_origin(fact.origin),
            print_scope(&fact.scope),
            print_predicate(&fact.predicate),
            print_derivation(&fact.derivation),
        ));
    }
    output
}

fn print_origin(origin: FactOrigin) -> String {
    match origin {
        FactOrigin::Proven => "proven".to_string(),
        FactOrigin::TrustedContract { instance } => {
            format!("trusted-contract(instance=ci{})", instance.index())
        }
    }
}

fn print_scope(scope: &FactScope) -> String {
    match scope {
        FactScope::FunctionEntry(function) => format!("function-entry(f{})", function.index()),
        FactScope::Block { function, block } => {
            format!("block(f{},b{})", function.index(), block.index())
        }
        FactScope::CalleeInstance { instance, callee } => format!(
            "callee-instance(ci{},f{})",
            instance.index(),
            callee.index()
        ),
        FactScope::InlineClone {
            function,
            clone,
            blocks,
        } => format!(
            "inline-clone(f{},c{}; {})",
            function.index(),
            clone,
            blocks
                .iter()
                .map(|block| format!("b{}", block.index()))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn print_predicate(predicate: &FactPredicate) -> String {
    match predicate {
        FactPredicate::ValueInterval { value, interval } => format!(
            "range(v{}, {}..={})",
            value.index(),
            interval.lower(),
            interval.upper()
        ),
        FactPredicate::Contract(predicate) => print_contract_predicate(predicate),
    }
}

fn print_contract_predicate(predicate: &ContractFactPredicate) -> String {
    match predicate {
        ContractFactPredicate::Comparison {
            operator,
            left,
            right,
        } => format!(
            "contract-compare({} {operator} {})",
            print_contract_affine(left),
            print_contract_affine(right)
        ),
        ContractFactPredicate::MultipleOf { value, modulus } => format!(
            "contract-multiple-of({}, {modulus})",
            print_contract_affine(value)
        ),
        ContractFactPredicate::NoAlias { left, right } => {
            format!("contract-noalias(v{}, v{})", left.index(), right.index())
        }
        ContractFactPredicate::Aligned { pointer, alignment } => {
            format!(
                "contract-aligned({}, {alignment})",
                match pointer {
                    ContractFactPointer::Value(value) => format!("v{}", value.index()),
                    ContractFactPointer::SliceData(value) => format!("v{}.data", value.index()),
                }
            )
        }
        ContractFactPredicate::EffectCeiling { is_none, items } => {
            if *is_none {
                "contract-effects(none)".to_string()
            } else {
                format!(
                    "contract-effects({})",
                    items
                        .iter()
                        .map(|(value, effect)| format!(
                            "{}(v{})",
                            match effect {
                                ContractEffectKind::None => "none",
                                ContractEffectKind::Read => "read",
                                ContractEffectKind::Write => "write",
                                ContractEffectKind::ReadWrite => "readwrite",
                            },
                            value.index()
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
    }
}

fn print_contract_affine(expression: &ContractFactAffineExpression) -> String {
    let mut pieces = expression
        .terms
        .iter()
        .map(|term| {
            format!(
                "{}*{}",
                term.coefficient,
                match term.term {
                    ContractFactAffineTerm::Value(value) => format!("v{}", value.index()),
                    ContractFactAffineTerm::SliceLength(value) => {
                        format!("v{}.len", value.index())
                    }
                }
            )
        })
        .collect::<Vec<_>>();
    if expression.constant != BigInt::from(0_u8) || pieces.is_empty() {
        pieces.push(expression.constant.to_string());
    }
    pieces.join("+")
}

fn print_derivation(derivation: &FactDerivation) -> String {
    match derivation {
        FactDerivation::TrustedContractLeaf => "trusted-contract".to_string(),
        FactDerivation::Constant { instruction } => format!("constant(i{})", instruction.index()),
        FactDerivation::BinaryTransfer {
            instruction,
            inputs,
        } => format!(
            "binary(i{}; {})",
            instruction.index(),
            inputs
                .iter()
                .map(|fact| format!("fact{}", fact.index()))
                .collect::<Vec<_>>()
                .join(",")
        ),
        FactDerivation::BranchRefinement {
            predecessor,
            comparison,
            input,
            taken,
        } => format!(
            "branch(b{},i{},fact{},taken={taken})",
            predecessor.index(),
            comparison.index(),
            input.index()
        ),
        FactDerivation::LoopInvariant {
            header,
            entry,
            transfer,
        } => format!(
            "loop(b{},fact{},fact{})",
            header.index(),
            entry.index(),
            transfer.index()
        ),
    }
}
