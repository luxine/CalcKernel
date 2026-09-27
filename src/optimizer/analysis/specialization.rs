use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;
use sha2::{Digest, Sha256};

use crate::{
    BlockId, CandidateKey, ContractFactAffineTerm, ContractFactPredicate, ContractFactSet,
    ContractInstanceId, ContractInstanceSource, FactId, FunctionId, InstructionId,
    KirInstructionKind, KirModule, KirSanitizerMode, MirPrimitiveTypeName, MirType, ValueId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecializationFactValue {
    Integer { value: String },
    Boolean { value: bool },
    Float { value: String },
    SliceLength { length: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecializationFactSource {
    Constant {
        instruction: InstructionId,
    },
    TrustedContract {
        instance: ContractInstanceId,
        fact: FactId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecializationFact {
    pub parameter_index: u32,
    pub value: SpecializationFactValue,
    pub source: SpecializationFactSource,
}

impl SpecializationFact {
    #[must_use]
    pub fn stable_text(&self) -> String {
        let source = match self.source {
            SpecializationFactSource::Constant { instruction } => {
                format!("constant:i{}", instruction.index())
            }
            SpecializationFactSource::TrustedContract { instance, fact } => {
                format!("contract:ci{}:fact{}", instance.index(), fact.index())
            }
        };
        format!("{}:{source}", self.semantic_text())
    }

    #[must_use]
    pub fn semantic_text(&self) -> String {
        let value = match &self.value {
            SpecializationFactValue::Integer { value } => format!("integer:{value}"),
            SpecializationFactValue::Boolean { value } => format!("boolean:{value}"),
            SpecializationFactValue::Float { value } => format!("float:{value}"),
            SpecializationFactValue::SliceLength { length } => format!("slice-length:{length}"),
        };
        format!("param{}:{value}", self.parameter_index)
    }

    fn subject_key(&self) -> (u32, bool) {
        (
            self.parameter_index,
            matches!(self.value, SpecializationFactValue::SliceLength { .. }),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecializationCandidate {
    pub caller: FunctionId,
    pub block: BlockId,
    pub call: InstructionId,
    pub callee: FunctionId,
    pub facts: Vec<SpecializationFact>,
    pub fact_set_digest: String,
    pub key: CandidateKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecializationFallback {
    pub function: FunctionId,
    pub call: Option<InstructionId>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecializationDiscovery {
    pub candidates: Vec<SpecializationCandidate>,
    pub fallbacks: Vec<SpecializationFallback>,
}

#[must_use]
pub fn discover_specialization_candidates(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
) -> SpecializationDiscovery {
    if module.config.sanitizer_mode != KirSanitizerMode::Disabled {
        return SpecializationDiscovery {
            candidates: Vec::new(),
            fallbacks: module
                .functions
                .iter()
                .map(|function| SpecializationFallback {
                    function: function.id,
                    call: None,
                    reason: "sanitizer-mode-disabled".to_string(),
                })
                .collect(),
        };
    }

    let recursive = recursive_functions(module);
    let mut discovery = SpecializationDiscovery::default();
    for caller in &module.functions {
        if is_specialization_clone(caller.name.as_str()) {
            continue;
        }
        for block in &caller.blocks {
            for instruction in &block.instructions {
                let KirInstructionKind::Call {
                    function_name,
                    args,
                } = &instruction.kind
                else {
                    continue;
                };
                let Some(callee) = module
                    .functions
                    .iter()
                    .find(|function| function.name == *function_name)
                else {
                    continue;
                };
                let excluded = if callee.exported {
                    Some("exported-callee")
                } else if recursive.contains(&callee.id) {
                    Some("recursive-scc")
                } else if is_specialization_clone(callee.name.as_str()) {
                    Some("clone-is-not-a-root")
                } else if callee.id == caller.id {
                    Some("recursive-scc")
                } else if args.len() != callee.params.len() {
                    Some("argument-count-mismatch")
                } else {
                    None
                };
                if let Some(reason) = excluded {
                    discovery.fallbacks.push(SpecializationFallback {
                        function: caller.id,
                        call: Some(instruction.id),
                        reason: reason.to_string(),
                    });
                    continue;
                }

                let mut facts = exact_local_facts(caller, callee, args);
                add_contract_facts(
                    contracts,
                    caller.id,
                    block.id,
                    instruction.id,
                    callee.id,
                    callee,
                    args,
                    &mut facts,
                );
                facts.sort_by_key(SpecializationFact::stable_text);
                facts.dedup_by(|left, right| left.subject_key() == right.subject_key());
                if facts.is_empty() {
                    discovery.fallbacks.push(SpecializationFallback {
                        function: caller.id,
                        call: Some(instruction.id),
                        reason: "no-dominating-specialization-fact".to_string(),
                    });
                    continue;
                }
                let fact_set_digest = specialization_fact_set_digest(&facts);
                let key = CandidateKey::Specialization {
                    caller: caller.id,
                    call: instruction.id,
                    callee: callee.id,
                    fact_set_digest: fact_set_digest.clone(),
                };
                discovery.candidates.push(SpecializationCandidate {
                    caller: caller.id,
                    block: block.id,
                    call: instruction.id,
                    callee: callee.id,
                    facts,
                    fact_set_digest,
                    key,
                });
            }
        }
    }
    discovery
        .candidates
        .sort_by(|left, right| left.key.cmp(&right.key));
    discovery.fallbacks.sort_by(|left, right| {
        (left.function, left.call, left.reason.as_str()).cmp(&(
            right.function,
            right.call,
            right.reason.as_str(),
        ))
    });
    discovery
}

#[must_use]
pub fn specialization_fact_set_digest(facts: &[SpecializationFact]) -> String {
    let mut canonical = facts
        .iter()
        .map(SpecializationFact::semantic_text)
        .collect::<Vec<_>>();
    canonical.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"ck-specialization-facts-v1\0");
    for fact in canonical {
        hasher.update((fact.len() as u64).to_le_bytes());
        hasher.update(fact.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[must_use]
pub fn is_specialization_clone(name: &str) -> bool {
    name.starts_with("__ck_spec_")
}

fn exact_local_facts(
    caller: &crate::KirFunction,
    callee: &crate::KirFunction,
    args: &[ValueId],
) -> Vec<SpecializationFact> {
    callee
        .params
        .iter()
        .zip(args)
        .enumerate()
        .filter_map(|(index, (parameter, argument))| {
            let (value, instruction) = exact_argument(caller, *argument, &parameter.type_node)?;
            Some(SpecializationFact {
                parameter_index: u32::try_from(index).ok()?,
                value,
                source: SpecializationFactSource::Constant { instruction },
            })
        })
        .collect()
}

fn exact_argument(
    caller: &crate::KirFunction,
    argument: ValueId,
    parameter_type: &MirType,
) -> Option<(SpecializationFactValue, InstructionId)> {
    let (instruction, value) = defining_instruction_following_copies(caller, argument)?;
    match (&instruction.kind, parameter_type) {
        (
            KirInstructionKind::ConstInt { value },
            MirType::Primitive(
                MirPrimitiveTypeName::I32
                | MirPrimitiveTypeName::I64
                | MirPrimitiveTypeName::U32
                | MirPrimitiveTypeName::U64,
            ),
        ) => Some((
            SpecializationFactValue::Integer {
                value: value.clone(),
            },
            instruction.id,
        )),
        (
            KirInstructionKind::ConstBool { value },
            MirType::Primitive(MirPrimitiveTypeName::Bool),
        ) => Some((
            SpecializationFactValue::Boolean { value: *value },
            instruction.id,
        )),
        (
            KirInstructionKind::ConstFloat { value },
            MirType::Primitive(MirPrimitiveTypeName::F64),
        ) => Some((
            SpecializationFactValue::Float {
                value: value.clone(),
            },
            instruction.id,
        )),
        (KirInstructionKind::MakeSlice { len, .. }, MirType::Slice(_)) => {
            let (length_instruction, _) = defining_instruction_following_copies(caller, *len)?;
            let KirInstructionKind::ConstInt { value } = &length_instruction.kind else {
                return None;
            };
            let length = value.parse::<u32>().ok()?;
            Some((
                SpecializationFactValue::SliceLength { length },
                length_instruction.id,
            ))
        }
        _ => {
            let _ = value;
            None
        }
    }
}

fn defining_instruction_following_copies(
    function: &crate::KirFunction,
    mut value: ValueId,
) -> Option<(&crate::KirInstruction, ValueId)> {
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(value) {
            return None;
        }
        let instruction = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            })?;
        if let KirInstructionKind::Copy { value: input } = instruction.kind {
            value = input;
            continue;
        }
        return Some((instruction, value));
    }
}

#[allow(clippy::too_many_arguments)]
fn add_contract_facts(
    contracts: Option<&ContractFactSet>,
    caller: FunctionId,
    block: BlockId,
    call: InstructionId,
    callee_id: FunctionId,
    callee: &crate::KirFunction,
    args: &[ValueId],
    facts: &mut Vec<SpecializationFact>,
) {
    let Some(contracts) = contracts else {
        return;
    };
    let Some(instance) = contracts.instances().iter().find(|instance| {
        instance.callee == callee_id
            && matches!(
                instance.source,
                ContractInstanceSource::Call {
                    caller: source_caller,
                    block: source_block,
                    instruction: source_call,
                } if source_caller == caller && source_block == block && source_call == call
            )
    }) else {
        return;
    };
    let mut occupied = facts
        .iter()
        .map(SpecializationFact::subject_key)
        .collect::<BTreeSet<_>>();
    for fact_id in &instance.facts {
        let Some(fact) = contracts.facts().get(*fact_id) else {
            continue;
        };
        let crate::FactPredicate::Contract(ContractFactPredicate::Comparison {
            operator,
            left,
            right,
        }) = &fact.predicate
        else {
            continue;
        };
        if operator != "==" {
            continue;
        }
        let Some((term, value)) = exact_equality(left, right) else {
            continue;
        };
        let subject_value = match term {
            ContractFactAffineTerm::Value(value) => (value, false),
            ContractFactAffineTerm::SliceLength(value) => (value, true),
        };
        let Some(index) = args
            .iter()
            .position(|argument| *argument == subject_value.0)
        else {
            continue;
        };
        let Ok(parameter_index) = u32::try_from(index) else {
            continue;
        };
        let subject = (parameter_index, subject_value.1);
        if occupied.contains(&subject) {
            continue;
        }
        let specialized_value = if subject_value.1 {
            let Ok(length) = value.to_string().parse::<u32>() else {
                continue;
            };
            SpecializationFactValue::SliceLength { length }
        } else {
            let Some(parameter) = callee.params.get(index) else {
                continue;
            };
            match parameter.type_node {
                MirType::Primitive(MirPrimitiveTypeName::I32)
                | MirType::Primitive(MirPrimitiveTypeName::I64)
                | MirType::Primitive(MirPrimitiveTypeName::U32)
                | MirType::Primitive(MirPrimitiveTypeName::U64) => {
                    SpecializationFactValue::Integer {
                        value: value.to_string(),
                    }
                }
                _ => continue,
            }
        };
        facts.push(SpecializationFact {
            parameter_index,
            value: specialized_value,
            source: SpecializationFactSource::TrustedContract {
                instance: instance.id,
                fact: *fact_id,
            },
        });
        occupied.insert(subject);
    }
}

fn exact_equality(
    left: &crate::ContractFactAffineExpression,
    right: &crate::ContractFactAffineExpression,
) -> Option<(ContractFactAffineTerm, BigInt)> {
    let mut terms = BTreeMap::<ContractFactAffineTerm, BigInt>::new();
    for term in &left.terms {
        *terms.entry(term.term).or_default() += &term.coefficient;
    }
    for term in &right.terms {
        *terms.entry(term.term).or_default() -= &term.coefficient;
    }
    terms.retain(|_, coefficient| *coefficient != BigInt::from(0));
    let entries = terms.into_iter().collect::<Vec<_>>();
    let [(term, coefficient)] = entries.as_slice() else {
        return None;
    };
    if coefficient != &BigInt::from(1) && coefficient != &BigInt::from(-1) {
        return None;
    }
    let constant = &left.constant - &right.constant;
    let value = -constant / coefficient;
    ((coefficient * &value) + (&left.constant - &right.constant) == BigInt::from(0))
        .then_some((*term, value))
}

fn recursive_functions(module: &KirModule) -> BTreeSet<FunctionId> {
    let names = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function.id))
        .collect::<BTreeMap<_, _>>();
    let graph = module
        .functions
        .iter()
        .map(|function| {
            let targets = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter_map(|instruction| match &instruction.kind {
                    KirInstructionKind::Call { function_name, .. } => {
                        names.get(function_name.as_str())
                    }
                    _ => None,
                })
                .copied()
                .collect::<BTreeSet<_>>();
            (function.id, targets)
        })
        .collect::<BTreeMap<_, _>>();
    graph
        .keys()
        .copied()
        .filter(|root| reaches(*root, *root, &graph, &mut BTreeSet::new(), true))
        .collect()
}

fn reaches(
    current: FunctionId,
    target: FunctionId,
    graph: &BTreeMap<FunctionId, BTreeSet<FunctionId>>,
    visited: &mut BTreeSet<FunctionId>,
    first: bool,
) -> bool {
    if !first && current == target {
        return true;
    }
    if !visited.insert(current) {
        return false;
    }
    graph.get(&current).is_some_and(|successors| {
        successors
            .iter()
            .any(|next| reaches(*next, target, graph, visited, false))
    })
}
