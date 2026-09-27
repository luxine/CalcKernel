use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    BlockId, FunctionId, InstructionId, KirArithmeticSemantics, KirInstructionKind, KirModule,
    KirTerminator, ProofId, ValueId,
};

use super::super::{
    BoolLattice, ContractFactSet, FactArena, FactPredicate, FactScope, FactUseSite, IntegerType,
    ProofArena, ProofStep, ProofStepId, ScalarAnalysisBudget, ScalarAnalysisConfig, ScalarClaim,
    ScalarFailure, ScalarValue, contract_scalar_interval, refine_scalar_comparison, scalar_binary,
    scalar_compare, verify_proof_arena,
};

enum FoldedConstant {
    Integer(String),
    Boolean(bool),
}

struct ConstantRewrite {
    function: FunctionId,
    instruction: InstructionId,
    value: ValueId,
    constant: FoldedConstant,
    proof: ProofId,
    step: ProofStepId,
}

struct PhiConstantRewrite {
    function: FunctionId,
    block: BlockId,
    value: ValueId,
    constant: FoldedConstant,
    proof: ProofId,
    step: ProofStepId,
}

pub(super) struct ScalarProposals {
    pub proofs: ProofArena,
    pub values: BTreeMap<FunctionId, (ProofId, BTreeMap<ValueId, ProofStepId>)>,
    rewrites: Vec<ConstantRewrite>,
    phis: Vec<PhiConstantRewrite>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ScalarWorkItem {
    Phi { block: usize, index: usize },
    Instruction { block: usize, index: usize },
}

struct ScalarWorklist {
    queue: VecDeque<ScalarWorkItem>,
    queued: BTreeSet<ScalarWorkItem>,
    users: BTreeMap<ValueId, Vec<ScalarWorkItem>>,
}

impl ScalarWorklist {
    fn new(function: &crate::KirFunction) -> Self {
        let mut worklist = Self {
            queue: VecDeque::new(),
            queued: BTreeSet::new(),
            users: BTreeMap::new(),
        };
        let block_indices = function
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id, index))
            .collect::<BTreeMap<_, _>>();
        for (block_index, block) in function.blocks.iter().enumerate() {
            for index in 0..block.params.len() {
                worklist.enqueue(ScalarWorkItem::Phi {
                    block: block_index,
                    index,
                });
            }
            for (index, instruction) in block.instructions.iter().enumerate() {
                let item = ScalarWorkItem::Instruction {
                    block: block_index,
                    index,
                };
                worklist.enqueue(item);
                for value in super::dce::instruction_uses(instruction) {
                    worklist.users.entry(value).or_default().push(item);
                }
            }
            let edges = match &block.terminator {
                KirTerminator::Return { .. } => Vec::new(),
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
            };
            let comparison = match block.terminator {
                KirTerminator::Branch { condition, .. } => block
                    .instructions
                    .iter()
                    .find(|instruction| {
                        instruction
                            .results
                            .first()
                            .is_some_and(|result| result.value == condition)
                    })
                    .and_then(|instruction| match instruction.kind {
                        KirInstructionKind::Compare { left, right, .. } => Some((left, right)),
                        _ => None,
                    }),
                _ => None,
            };
            for edge in edges {
                let Some(&target) = block_indices.get(&edge.target) else {
                    continue;
                };
                for (index, &value) in edge.args.iter().enumerate() {
                    let item = ScalarWorkItem::Phi {
                        block: target,
                        index,
                    };
                    worklist.users.entry(value).or_default().push(item);
                    if let Some((left, right)) = comparison
                        && (value == left || value == right)
                    {
                        // A phi's range also depends on its incoming path condition,
                        // including comparison operands that are not phi arguments.
                        for bound in [left, right] {
                            worklist.users.entry(bound).or_default().push(item);
                        }
                    }
                }
            }
        }
        worklist
    }

    fn enqueue(&mut self, item: ScalarWorkItem) {
        if self.queued.insert(item) {
            self.queue.push_back(item);
        }
    }

    fn pop(&mut self) -> Option<ScalarWorkItem> {
        let item = self.queue.pop_front()?;
        self.queued.remove(&item);
        Some(item)
    }

    fn value_changed(&mut self, value: ValueId) {
        if let Some(users) = self.users.get(&value) {
            for &item in users {
                if self.queued.insert(item) {
                    self.queue.push_back(item);
                }
            }
        }
    }
}

pub(crate) fn run_integer_constant_folding(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
    live_proofs: &ProofArena,
) -> Result<bool, String> {
    let mut proposals = propose_with_contracts(module, contracts, ScalarAnalysisConfig::default())?;
    let protected = live_proofs.instruction_dependencies();
    let protected_phis = live_proofs.block_parameter_dependencies();
    proposals
        .rewrites
        .retain(|rewrite| !protected.contains(&rewrite.instruction));
    proposals
        .phis
        .retain(|rewrite| !protected_phis.contains(&rewrite.value));
    verify_and_apply_with_contracts(
        module,
        contracts,
        &proposals.proofs,
        &proposals.rewrites,
        &proposals.phis,
    )
}

pub(super) fn propose_scalar_ranges(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
) -> Result<ScalarProposals, String> {
    propose_with_contracts(module, contracts, ScalarAnalysisConfig::default())
}

#[cfg(test)]
fn propose(module: &KirModule) -> Result<(ProofArena, Vec<ConstantRewrite>), String> {
    propose_with_config(module, ScalarAnalysisConfig::default())
}

#[cfg(test)]
fn propose_with_config(
    module: &KirModule,
    config: ScalarAnalysisConfig,
) -> Result<(ProofArena, Vec<ConstantRewrite>), String> {
    let proposals = propose_with_contracts(module, None, config)?;
    Ok((proposals.proofs, proposals.rewrites))
}

fn propose_with_contracts(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    config: ScalarAnalysisConfig,
) -> Result<ScalarProposals, String> {
    let mut proofs = ProofArena::new(0);
    let mut rewrites = Vec::new();
    let mut phis = Vec::new();
    let mut scalar_values = BTreeMap::new();
    'functions: for function in &module.functions {
        let Some(entry) = function.blocks.first().map(|block| block.id) else {
            continue;
        };
        let mut values = BTreeMap::<ValueId, (ScalarValue, ProofStepId)>::new();
        let mut booleans = BTreeMap::<ValueId, (bool, ProofStepId)>::new();
        let mut steps = Vec::new();
        let mut pending = Vec::new();
        let mut remaining = ScalarAnalysisBudget::for_function(function, config).max_steps();
        let entry_facts = contracts.map_or_else(Vec::new, |contracts| contracts.facts().facts().iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(owner) if owner == function.id)).collect::<Vec<_>>());
        for fact in &entry_facts {
            steps.push(ProofStep::FactLeaf { fact: fact.id });
        }
        let fact_steps = (0..steps.len())
            .map(|index| ProofStepId::from_index(index as u32))
            .collect::<Vec<_>>();
        for param in &function.params {
            let Some(next) = remaining.checked_sub(1) else {
                continue 'functions;
            };
            remaining = next;
            let Some(ty) = IntegerType::from_mir(&param.type_node) else {
                continue;
            };
            let interval = contract_scalar_interval(
                entry_facts.iter().filter_map(|fact| match &fact.predicate {
                    FactPredicate::Contract(predicate) => Some(predicate),
                    _ => None,
                }),
                param.value,
                ty,
            );
            let (value, proof_step) = if let Some(interval) = interval {
                let value = ScalarValue::from_interval(ty, interval.clone())
                    .map_err(|error| error.to_string())?;
                (
                    value,
                    ProofStep::ContractRange {
                        block: entry,
                        premises: fact_steps.clone(),
                        claim: ScalarClaim::new(param.value, interval, ScalarFailure::None),
                    },
                )
            } else {
                let value = ScalarValue::unknown(ty);
                let claim =
                    ScalarClaim::new(param.value, value.interval().clone(), ScalarFailure::None);
                (value, ProofStep::TypeBounds { claim })
            };
            let step = ProofStepId::from_index(
                u32::try_from(steps.len()).map_err(|_| "SCCP proof exceeds u32 identity space")?,
            );
            steps.push(proof_step);
            values.insert(param.value, (value, step));
        }
        let mut incoming =
            BTreeMap::<BlockId, Vec<(BlockId, Option<bool>, &crate::KirEdge)>>::new();
        for block in &function.blocks {
            let edges = match &block.terminator {
                KirTerminator::Return { .. } => Vec::new(),
                KirTerminator::Jump { edge } => vec![(None, edge)],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![(Some(true), then_edge), (Some(false), else_edge)],
            };
            for (taken, edge) in edges {
                incoming
                    .entry(edge.target)
                    .or_default()
                    .push((block.id, taken, edge));
            }
        }
        let mut worklist = ScalarWorklist::new(function);
        while let Some(item) = worklist.pop() {
            let Some(next) = remaining.checked_sub(1) else {
                continue 'functions;
            };
            remaining = next;
            match item {
                ScalarWorkItem::Phi { block, index } => {
                    let block = &function.blocks[block];
                    let param = &block.params[index];
                    let Some(edges) = incoming.get(&block.id) else {
                        continue;
                    };
                    if param.type_node
                        == crate::KirValueType::Scalar(crate::MirType::Primitive(
                            crate::MirPrimitiveTypeName::Bool,
                        ))
                    {
                        if booleans.contains_key(&param.value) {
                            continue;
                        }
                        let inputs = edges
                            .iter()
                            .map(|(_, _, edge)| {
                                edge.args
                                    .get(index)
                                    .and_then(|value| booleans.get(value))
                                    .copied()
                            })
                            .collect::<Option<Vec<_>>>();
                        let Some(inputs) = inputs else {
                            continue;
                        };
                        let Some(&(constant, _)) = inputs.first() else {
                            continue;
                        };
                        if inputs.iter().any(|(value, _)| *value != constant) {
                            continue;
                        }
                        let step = ProofStepId::from_index(
                            u32::try_from(steps.len())
                                .map_err(|_| "SCCP proof exceeds u32 identity space")?,
                        );
                        steps.push(ProofStep::BooleanPhiJoin {
                            block: block.id,
                            inputs: inputs.iter().map(|(_, step)| *step).collect(),
                            value: param.value,
                            result: constant,
                        });
                        booleans.insert(param.value, (constant, step));
                        worklist.value_changed(param.value);
                        continue;
                    }
                    if IntegerType::from_kir(&param.type_node).is_none() {
                        continue;
                    }
                    let inputs = edges
                        .iter()
                        .map(|(_, _, edge)| {
                            edge.args
                                .get(index)
                                .and_then(|value| values.get(value))
                                .cloned()
                        })
                        .collect::<Option<Vec<_>>>();
                    let Some(mut inputs) = inputs else {
                        continue;
                    };
                    let start = steps.len();
                    for ((predecessor, taken, edge), input) in edges.iter().zip(&mut inputs) {
                        if let Some(taken) = taken {
                            refine_edge_input(
                                function,
                                (*predecessor, block.id, *taken, edge.args[index]),
                                &values,
                                &mut steps,
                                input,
                            )?;
                        }
                    }
                    let Some((first, _)) = inputs.first() else {
                        continue;
                    };
                    let lower = inputs
                        .iter()
                        .map(|(value, _)| value.interval().lower())
                        .min()
                        .ok_or("missing phi interval")?
                        .clone();
                    let upper = inputs
                        .iter()
                        .map(|(value, _)| value.interval().upper())
                        .max()
                        .ok_or("missing phi interval")?
                        .clone();
                    let value = ScalarValue::from_interval(
                        first.type_node(),
                        crate::ScalarInterval::new(lower, upper)
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?;
                    if values
                        .get(&param.value)
                        .is_some_and(|(old, _)| old == &value)
                    {
                        steps.truncate(start);
                        continue;
                    }
                    let step = ProofStepId::from_index(
                        u32::try_from(steps.len())
                            .map_err(|_| "SCCP proof exceeds u32 identity space")?,
                    );
                    steps.push(ProofStep::PhiJoin {
                        block: block.id,
                        inputs: inputs.iter().map(|(_, step)| *step).collect(),
                        claim: ScalarClaim::new(
                            param.value,
                            value.interval().clone(),
                            ScalarFailure::None,
                        ),
                    });
                    values.insert(param.value, (value, step));
                    worklist.value_changed(param.value);
                }
                ScalarWorkItem::Instruction { block, index } => {
                    let instruction = &function.blocks[block].instructions[index];
                    let Some(result) = instruction.results.first() else {
                        continue;
                    };
                    if instruction.results.len() != 1
                        && !matches!(
                            instruction.kind,
                            KirInstructionKind::Binary {
                                semantics: KirArithmeticSemantics::Checked,
                                ..
                            }
                        )
                    {
                        continue;
                    }
                    if booleans.contains_key(&result.value) {
                        continue;
                    }
                    if instruction.effect.is_some() || instruction.memory.is_some() {
                        continue;
                    }
                    let step = ProofStepId::from_index(
                        u32::try_from(steps.len())
                            .map_err(|_| "SCCP proof exceeds u32 identity space")?,
                    );
                    let pending_start = pending.len();
                    if let Some((constant, inputs)) = boolean_transfer(instruction, &booleans) {
                        steps.push(ProofStep::BooleanTransfer {
                            instruction: instruction.id,
                            inputs,
                            value: result.value,
                            result: constant,
                        });
                        if !matches!(instruction.kind, KirInstructionKind::ConstBool { .. }) {
                            pending.push((
                                instruction.id,
                                result.value,
                                FoldedConstant::Boolean(constant),
                                step,
                            ));
                        }
                        booleans.insert(result.value, (constant, step));
                        worklist.value_changed(result.value);
                        continue;
                    }
                    if let KirInstructionKind::Compare { op, left, right } = instruction.kind {
                        let (Some((left, left_step)), Some((right, right_step))) =
                            (values.get(&left), values.get(&right))
                        else {
                            continue;
                        };
                        let constant = match scalar_compare(op, left, right)
                            .map_err(|error| error.to_string())?
                        {
                            BoolLattice::AlwaysTrue => true,
                            BoolLattice::AlwaysFalse => false,
                            BoolLattice::Unknown => continue,
                        };
                        steps.push(ProofStep::IntegerComparison {
                            instruction: instruction.id,
                            left: *left_step,
                            right: *right_step,
                            value: result.value,
                            result: constant,
                        });
                        pending.push((
                            instruction.id,
                            result.value,
                            FoldedConstant::Boolean(constant),
                            step,
                        ));
                        booleans.insert(result.value, (constant, step));
                        worklist.value_changed(result.value);
                        continue;
                    }
                    let Some(ty) = IntegerType::from_kir(&result.type_node) else {
                        continue;
                    };
                    let value = match &instruction.kind {
                        KirInstructionKind::ConstInt { value } => {
                            let Ok(value) = ScalarValue::constant(
                                ty,
                                value.parse().map_err(|_| "invalid KIR integer")?,
                            ) else {
                                // Preserve the existing literal lowering when its spelling
                                // is outside this abstract domain; do not invent a range.
                                continue;
                            };
                            steps.push(ProofStep::Constant {
                                instruction: instruction.id,
                                claim: ScalarClaim::new(
                                    result.value,
                                    value.interval().clone(),
                                    ScalarFailure::None,
                                ),
                            });
                            value
                        }
                        KirInstructionKind::Binary {
                            op,
                            left,
                            right,
                            semantics,
                        } => {
                            let (Some((left, left_step)), Some((right, right_step))) =
                                (values.get(left), values.get(right))
                            else {
                                continue;
                            };
                            let value = scalar_binary(*op, *semantics, left, right)
                                .map_err(|error| error.to_string())?;
                            if value.failure() != ScalarFailure::None {
                                continue;
                            }
                            if *semantics == KirArithmeticSemantics::Modular
                                && let Some(constant) = value.exact_value()
                            {
                                pending.push((
                                    instruction.id,
                                    result.value,
                                    FoldedConstant::Integer(constant.to_string()),
                                    step,
                                ));
                            }
                            steps.push(ProofStep::BinaryTransfer {
                                instruction: instruction.id,
                                left: *left_step,
                                right: *right_step,
                                claim: ScalarClaim::new(
                                    result.value,
                                    value.interval().clone(),
                                    ScalarFailure::None,
                                ),
                            });
                            value
                        }
                        KirInstructionKind::Copy { value } => {
                            let Some((value, input)) = values.get(value) else {
                                continue;
                            };
                            steps.push(ProofStep::CopyTransfer {
                                instruction: instruction.id,
                                input: *input,
                                claim: ScalarClaim::new(
                                    result.value,
                                    value.interval().clone(),
                                    value.failure(),
                                ),
                            });
                            if let Some(constant) = value.exact_value() {
                                pending.push((
                                    instruction.id,
                                    result.value,
                                    FoldedConstant::Integer(constant.to_string()),
                                    step,
                                ));
                            }
                            value.clone()
                        }
                        _ => continue,
                    };
                    if values
                        .get(&result.value)
                        .is_some_and(|(old, _)| old == &value)
                    {
                        steps.truncate(step.index() as usize);
                        pending.truncate(pending_start);
                        continue;
                    }
                    values.insert(result.value, (value, step));
                    worklist.value_changed(result.value);
                }
            }
        }
        if steps.is_empty() {
            continue;
        }
        let root = ProofStepId::from_index(
            u32::try_from(steps.len() - 1).map_err(|_| "SCCP proof exceeds u32 identity space")?,
        );
        let proof = proofs
            .try_insert(use_site(function.id, entry), steps, root)
            .map_err(|error| error.to_string())?;
        for block in &function.blocks {
            for param in &block.params {
                if let Some((value, step)) = values.get(&param.value)
                    && let Some(constant) = value.exact_value()
                {
                    phis.push(PhiConstantRewrite {
                        function: function.id,
                        block: block.id,
                        value: param.value,
                        constant: FoldedConstant::Integer(constant.to_string()),
                        proof,
                        step: *step,
                    });
                } else if let Some((constant, step)) = booleans.get(&param.value) {
                    phis.push(PhiConstantRewrite {
                        function: function.id,
                        block: block.id,
                        value: param.value,
                        constant: FoldedConstant::Boolean(*constant),
                        proof,
                        step: *step,
                    });
                }
            }
        }
        scalar_values.insert(
            function.id,
            (
                proof,
                values
                    .into_iter()
                    .map(|(value, (_, step))| (value, step))
                    .collect(),
            ),
        );
        rewrites.extend(
            pending
                .into_iter()
                .map(|(instruction, value, constant, step)| ConstantRewrite {
                    function: function.id,
                    instruction,
                    value,
                    constant,
                    proof,
                    step,
                }),
        );
    }
    Ok(ScalarProposals {
        proofs,
        values: scalar_values,
        rewrites,
        phis,
    })
}

fn boolean_transfer(
    instruction: &crate::KirInstruction,
    values: &BTreeMap<ValueId, (bool, ProofStepId)>,
) -> Option<(bool, Vec<ProofStepId>)> {
    match instruction.kind {
        KirInstructionKind::ConstBool { value } => Some((value, Vec::new())),
        KirInstructionKind::Copy { value } => values
            .get(&value)
            .map(|(value, step)| (*value, vec![*step])),
        KirInstructionKind::Unary {
            op: crate::MirUnaryOp::Not,
            operand,
            ..
        } => values
            .get(&operand)
            .map(|(value, step)| (!value, vec![*step])),
        KirInstructionKind::Compare { op, left, right } => {
            let (left, left_step) = values.get(&left)?;
            let (right, right_step) = values.get(&right)?;
            let result = match op {
                crate::MirCompareOp::Eq => left == right,
                crate::MirCompareOp::Ne => left != right,
                _ => return None,
            };
            Some((result, vec![*left_step, *right_step]))
        }
        _ => None,
    }
}

fn use_site(function: FunctionId, block: BlockId) -> FactUseSite {
    FactUseSite {
        function,
        block,
        instruction: None,
        contract_instance: None,
    }
}

fn refine_edge_input(
    function: &crate::KirFunction,
    edge: (BlockId, BlockId, bool, ValueId),
    values: &BTreeMap<ValueId, (ScalarValue, ProofStepId)>,
    steps: &mut Vec<ProofStep>,
    input: &mut (ScalarValue, ProofStepId),
) -> Result<(), String> {
    let (predecessor, target, taken, argument) = edge;
    let Some(block) = function.blocks.iter().find(|block| block.id == predecessor) else {
        return Ok(());
    };
    let KirTerminator::Branch { condition, .. } = block.terminator else {
        return Ok(());
    };
    let Some(comparison) = block.instructions.iter().find(|instruction| {
        instruction
            .results
            .first()
            .is_some_and(|result| result.value == condition)
    }) else {
        return Ok(());
    };
    let KirInstructionKind::Compare { op, left, right } = comparison.kind else {
        return Ok(());
    };
    if argument != left && argument != right {
        return Ok(());
    }
    let (Some((left_value, left_step)), Some((right_value, right_step))) =
        (values.get(&left), values.get(&right))
    else {
        return Ok(());
    };
    let Ok((true_values, false_values)) = refine_scalar_comparison(op, left_value, right_value)
    else {
        return Ok(());
    };
    let refined = if taken { true_values } else { false_values };
    let refined = if argument == left {
        refined.0
    } else {
        refined.1
    };
    if refined.interval() == input.0.interval() {
        return Ok(());
    }
    let step = ProofStepId::from_index(
        u32::try_from(steps.len()).map_err(|_| "SCCP proof exceeds u32 identity space")?,
    );
    steps.push(ProofStep::BranchRefinement {
        predecessor,
        target,
        comparison: comparison.id,
        left: *left_step,
        right: *right_step,
        taken,
        claim: ScalarClaim::new(argument, refined.interval().clone(), ScalarFailure::None),
    });
    *input = (refined, step);
    Ok(())
}

#[cfg(test)]
fn verify_and_apply(
    module: &mut KirModule,
    proofs: &ProofArena,
    rewrites: &[ConstantRewrite],
) -> Result<bool, String> {
    verify_and_apply_with_contracts(module, None, proofs, rewrites, &[])
}

fn verify_and_apply_with_contracts(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    rewrites: &[ConstantRewrite],
    phis: &[PhiConstantRewrite],
) -> Result<bool, String> {
    if rewrites.is_empty() && phis.is_empty() {
        return Ok(false);
    }
    // Check the closed derivations against the immutable pre-rewrite KIR. The
    // optimizer's ScalarValue result is never itself authority for a rewrite.
    let empty_facts = FactArena::new(0);
    let facts = contracts.map_or(&empty_facts, ContractFactSet::facts);
    let validation = verify_proof_arena(module, facts, contracts, proofs, 0);
    if !validation.errors.is_empty() {
        return Err(format!(
            "invalid SCCP rewrite certificate: {}",
            validation.errors[0].message
        ));
    }
    for rewrite in rewrites {
        let proof = proofs
            .get(rewrite.proof)
            .ok_or("missing SCCP rewrite proof")?;
        let bound = match (
            proof.steps.get(rewrite.step.index() as usize),
            &rewrite.constant,
        ) {
            (
                Some(
                    ProofStep::BinaryTransfer {
                        instruction, claim, ..
                    }
                    | ProofStep::CopyTransfer {
                        instruction, claim, ..
                    },
                ),
                FoldedConstant::Integer(constant),
            ) => {
                *instruction == rewrite.instruction
                    && claim.value == rewrite.value
                    && claim.failure == ScalarFailure::None
                    && claim.interval.lower() == claim.interval.upper()
                    && claim.interval.lower().to_string() == *constant
            }
            (
                Some(
                    ProofStep::IntegerComparison {
                        instruction,
                        value,
                        result,
                        ..
                    }
                    | ProofStep::BooleanTransfer {
                        instruction,
                        value,
                        result,
                        ..
                    },
                ),
                FoldedConstant::Boolean(constant),
            ) => {
                *instruction == rewrite.instruction && *value == rewrite.value && result == constant
            }
            _ => false,
        };
        if proof.use_site.function != rewrite.function || !bound {
            return Err("SCCP replacement does not match its certificate".to_string());
        }
        let original = module
            .functions
            .iter()
            .find(|function| function.id == rewrite.function)
            .and_then(|function| {
                function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .find(|instruction| instruction.id == rewrite.instruction)
            })
            .ok_or("SCCP replacement instruction is missing")?;
        if original.effect.is_some()
            || original.memory.is_some()
            || original.results.len() != 1
            || !matches!(
                original.kind,
                KirInstructionKind::Binary {
                    semantics: KirArithmeticSemantics::Modular,
                    ..
                } | KirInstructionKind::Copy { .. }
                    | KirInstructionKind::Compare { .. }
                    | KirInstructionKind::Unary {
                        op: crate::MirUnaryOp::Not,
                        ..
                    }
            )
        {
            return Err(
                "SCCP replacement would erase a checked or effectful operation".to_string(),
            );
        }
    }
    let phi_replacements = prepare_phi_replacements(module, proofs, phis)?;
    let replacements = rewrites
        .iter()
        .map(|rewrite| (rewrite.instruction, &rewrite.constant))
        .collect::<BTreeMap<_, _>>();
    for instruction in module
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
    {
        if let Some(value) = replacements.get(&instruction.id) {
            instruction.kind = match value {
                FoldedConstant::Integer(value) => KirInstructionKind::ConstInt {
                    value: value.clone(),
                },
                FoldedConstant::Boolean(value) => KirInstructionKind::ConstBool { value: *value },
            };
        }
    }
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(replacements) = phi_replacements.get(&(function.id, block.id)) {
                block.params = std::mem::take(&mut block.params)
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, param)| {
                        (!replacements.contains_key(&index)).then_some(param)
                    })
                    .collect();
                let old = std::mem::take(&mut block.instructions);
                block.instructions.extend(replacements.values().cloned());
                block.instructions.extend(old);
            }
            let edges = match &mut block.terminator {
                KirTerminator::Return { .. } => Vec::new(),
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
            };
            for edge in edges {
                if let Some(replacements) = phi_replacements.get(&(function.id, edge.target)) {
                    edge.args = edge
                        .args
                        .iter()
                        .enumerate()
                        .filter_map(|(index, value)| {
                            (!replacements.contains_key(&index)).then_some(*value)
                        })
                        .collect();
                }
            }
        }
    }
    Ok(true)
}

type PhiReplacements = BTreeMap<(FunctionId, BlockId), BTreeMap<usize, crate::KirInstruction>>;

fn prepare_phi_replacements(
    module: &KirModule,
    proofs: &ProofArena,
    phis: &[PhiConstantRewrite],
) -> Result<PhiReplacements, String> {
    let mut replacements = BTreeMap::new();
    if phis.is_empty() {
        return Ok(replacements);
    }
    let first = match module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
    {
        Some(last) => last
            .checked_add(1)
            .ok_or("SCCP instruction identity space exhausted")?,
        None => 0,
    };
    let count =
        u32::try_from(phis.len()).map_err(|_| "SCCP instruction identity space exhausted")?;
    first
        .checked_add(count - 1)
        .ok_or("SCCP instruction identity space exhausted")?;
    for (offset, rewrite) in phis.iter().enumerate() {
        let proof = proofs.get(rewrite.proof).ok_or("missing SCCP phi proof")?;
        let bound = match (
            proof.steps.get(rewrite.step.index() as usize),
            &rewrite.constant,
        ) {
            (Some(ProofStep::PhiJoin { block, claim, .. }), FoldedConstant::Integer(constant)) => {
                *block == rewrite.block
                    && claim.value == rewrite.value
                    && claim.failure == ScalarFailure::None
                    && claim.interval.lower() == claim.interval.upper()
                    && claim.interval.lower().to_string() == *constant
            }
            (
                Some(ProofStep::BooleanPhiJoin {
                    block,
                    value,
                    result,
                    ..
                }),
                FoldedConstant::Boolean(constant),
            ) => *block == rewrite.block && *value == rewrite.value && result == constant,
            _ => false,
        };
        if proof.use_site.function != rewrite.function || !bound {
            return Err("SCCP phi replacement does not match its certificate".to_string());
        }
        let (index, param) = module
            .functions
            .iter()
            .find(|function| function.id == rewrite.function)
            .and_then(|function| {
                function
                    .blocks
                    .iter()
                    .find(|block| block.id == rewrite.block)
            })
            .and_then(|block| {
                block
                    .params
                    .iter()
                    .enumerate()
                    .find(|(_, param)| param.value == rewrite.value)
            })
            .ok_or("SCCP replacement block parameter is missing")?;
        let replacement = crate::KirInstruction {
            id: InstructionId::from_index(first + offset as u32),
            results: vec![crate::KirResult {
                value: param.value,
                type_node: param.type_node.clone(),
            }],
            kind: match &rewrite.constant {
                FoldedConstant::Integer(value) => KirInstructionKind::ConstInt {
                    value: value.clone(),
                },
                FoldedConstant::Boolean(value) => KirInstructionKind::ConstBool { value: *value },
            },
            memory: None,
            effect: None,
        };
        let block_replacements: &mut BTreeMap<usize, crate::KirInstruction> = replacements
            .entry((rewrite.function, rewrite.block))
            .or_default();
        if block_replacements.insert(index, replacement).is_some() {
            return Err("duplicate SCCP phi replacement".to_string());
        }
    }
    Ok(replacements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode, SourceFile,
        build_kir_module, check, lower_to_mir,
    };

    fn module() -> KirModule {
        module_from_source("export fn answer() -> i32 { return (20 + 22) * 2; }")
    }

    fn module_from_source(source: &str) -> KirModule {
        let checked = check(&SourceFile::new("constant-fold.ck", source));
        assert_eq!(checked.diagnostics, []);
        build_kir_module(
            &lower_to_mir(&checked.checked_program).expect("MIR"),
            KirBuildConfig {
                consumer: KirConsumer::Inspection,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Checked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR")
    }

    fn phi_module() -> KirModule {
        module_from_source(
            "export fn phi(flag: bool) -> i32 { let x: i32 = 0; if flag { x = 42; } else { x = 42; } return x + 1; }",
        )
    }

    fn boolean_phi_module() -> KirModule {
        module_from_source(
            "export fn choose(flag: bool) -> bool { let selected: bool = false; if flag { selected = true; } else { selected = true; } return selected; }",
        )
    }

    #[test]
    fn boolean_certificate_should_reject_a_false_transfer_without_partial_mutation() {
        let mut module = module_from_source("export fn negated() -> bool { return !true; }");
        let before = module.clone();
        let mut proposals = propose_scalar_ranges(&module, None).expect("proposal");
        let rewrite = proposals.rewrites.first().expect("not rewrite");
        let proof = proposals.proofs.get_mut(rewrite.proof).expect("proof");
        let ProofStep::BooleanTransfer { result, .. } =
            &mut proof.steps[rewrite.step.index() as usize]
        else {
            panic!("boolean transfer")
        };
        *result = !*result;
        assert!(
            verify_and_apply_with_contracts(
                &mut module,
                None,
                &proposals.proofs,
                &proposals.rewrites,
                &proposals.phis
            )
            .expect_err("forged truth value")
            .contains("boolean claim")
        );
        assert_eq!(module, before);
    }

    #[test]
    fn boolean_certificate_should_reject_missing_or_wrong_arm_phi_premises() {
        for missing in [true, false] {
            let mut module = boolean_phi_module();
            let before = module.clone();
            let mut proposals = propose_scalar_ranges(&module, None).expect("proposal");
            let rewrite = proposals.phis.last().expect("join phi");
            let proof = proposals.proofs.get_mut(rewrite.proof).expect("proof");
            let ProofStep::BooleanPhiJoin { inputs, .. } =
                &mut proof.steps[rewrite.step.index() as usize]
            else {
                panic!("boolean phi")
            };
            assert_eq!(inputs.len(), 2);
            if missing {
                inputs.pop();
            } else {
                inputs[1] = inputs[0];
            }
            assert!(
                verify_and_apply_with_contracts(
                    &mut module,
                    None,
                    &proposals.proofs,
                    &proposals.rewrites,
                    &proposals.phis
                )
                .expect_err("invalid phi")
                .contains("every incoming edge")
            );
            assert_eq!(module, before);
        }
    }

    #[test]
    fn boolean_certificate_should_reject_a_wrong_phi_replacement_atomically() {
        let mut module = boolean_phi_module();
        let before = module.clone();
        let mut proposals = propose_scalar_ranges(&module, None).expect("proposal");
        let rewrite = proposals.phis.last_mut().expect("join phi");
        let FoldedConstant::Boolean(value) = &mut rewrite.constant else {
            panic!("bool")
        };
        assert!(*value);
        *value = false;
        assert!(
            verify_and_apply_with_contracts(
                &mut module,
                None,
                &proposals.proofs,
                &proposals.rewrites,
                &proposals.phis
            )
            .expect_err("wrong replacement")
            .contains("replacement does not match")
        );
        assert_eq!(module, before);
    }

    #[test]
    fn boolean_certificate_should_preserve_live_phi_and_instruction_dependencies() {
        let mut module = boolean_phi_module();
        let before = module.clone();
        let proposals = propose_scalar_ranges(&module, None).expect("proposal");
        assert!(!proposals.phis.is_empty());
        assert!(
            !run_integer_constant_folding(&mut module, None, &proposals.proofs).expect("preserve")
        );
        assert_eq!(module, before);
        assert!(verify_ranges(&module, &proposals.proofs).errors.is_empty());
    }

    #[test]
    fn boolean_certificate_should_discard_budget_exhausted_proposals_deterministically() {
        let module = boolean_phi_module();
        for budget in [0, 1, 4] {
            let proposals =
                propose_with_contracts(&module, None, ScalarAnalysisConfig::with_max_steps(budget))
                    .expect("bounded");
            assert!(proposals.phis.is_empty());
            assert!(proposals.rewrites.is_empty());
            assert!(proposals.proofs.proofs().is_empty());
        }
        let first = propose_scalar_ranges(&module, None).expect("proposal");
        let second = propose_scalar_ranges(&module, None).expect("proposal");
        assert_eq!(first.proofs, second.proofs);
        let printed = crate::print_proof_arena(&first.proofs);
        assert!(printed.contains("boolean-phi") && printed.contains("boolean i"));
    }

    #[test]
    fn constant_phi_rewrite_should_reject_a_wrong_value_before_any_instruction_mutation() {
        let mut module = phi_module();
        let before = module.clone();
        let mut proposals = propose_scalar_ranges(&module, None).expect("proposal");
        assert!(!proposals.rewrites.is_empty());
        proposals.phis.last_mut().expect("constant phi").constant =
            FoldedConstant::Integer("43".to_string());
        let error = verify_and_apply_with_contracts(
            &mut module,
            None,
            &proposals.proofs,
            &proposals.rewrites,
            &proposals.phis,
        )
        .expect_err("wrong phi value");
        assert!(error.contains("phi replacement does not match"), "{error}");
        assert_eq!(module, before);
    }

    #[test]
    fn constant_phi_rewrite_should_reject_incomplete_incoming_edge_evidence() {
        let mut module = phi_module();
        let before = module.clone();
        let mut proposals = propose_scalar_ranges(&module, None).expect("proposal");
        let rewrite = proposals.phis.last().expect("join phi");
        let proof = proposals.proofs.get_mut(rewrite.proof).expect("proof");
        let ProofStep::PhiJoin { inputs, .. } = &mut proof.steps[rewrite.step.index() as usize]
        else {
            panic!("phi proof");
        };
        assert_eq!(inputs.len(), 2);
        inputs.pop();
        let error = verify_and_apply_with_contracts(
            &mut module,
            None,
            &proposals.proofs,
            &proposals.rewrites,
            &proposals.phis,
        )
        .expect_err("missing edge");
        assert!(error.contains("every incoming edge"), "{error}");
        assert_eq!(module, before);
    }

    #[test]
    fn constant_phi_rewrite_should_fail_atomically_when_instruction_ids_are_exhausted() {
        let mut module = phi_module();
        module.functions[0]
            .blocks
            .last_mut()
            .expect("return block")
            .instructions
            .last_mut()
            .expect("arithmetic")
            .id = InstructionId::from_index(u32::MAX);
        let before = module.clone();
        let proposals = propose_scalar_ranges(&module, None).expect("proposal");
        let error = verify_and_apply_with_contracts(
            &mut module,
            None,
            &proposals.proofs,
            &proposals.rewrites,
            &proposals.phis,
        )
        .expect_err("no fresh identity");
        assert!(error.contains("identity space exhausted"), "{error}");
        assert_eq!(module, before);
    }

    #[test]
    fn constant_phi_rewrite_should_preserve_definitions_required_by_live_certificates() {
        let mut module = phi_module();
        let before = module.clone();
        let proposals = propose_scalar_ranges(&module, None).expect("proposal");
        assert!(!proposals.phis.is_empty());
        assert!(
            !run_integer_constant_folding(&mut module, None, &proposals.proofs).expect("preserved")
        );
        assert_eq!(module, before);
        assert!(verify_ranges(&module, &proposals.proofs).errors.is_empty());
    }

    #[test]
    fn constant_phi_rewrite_should_discard_every_phi_proposal_after_budget_exhaustion() {
        let module = phi_module();
        for budget in [0, 1, 4] {
            let proposals =
                propose_with_contracts(&module, None, ScalarAnalysisConfig::with_max_steps(budget))
                    .expect("bounded");
            assert!(proposals.phis.is_empty(), "budget {budget}");
            assert!(proposals.rewrites.is_empty(), "budget {budget}");
            assert!(proposals.proofs.proofs().is_empty(), "budget {budget}");
        }
    }

    #[test]
    fn constant_rewrite_sparse_worklist_should_handle_reverse_block_order_with_linear_budget() {
        let expression = std::iter::repeat_n("1", 40).collect::<Vec<_>>().join(" + ");
        let mut module = module_from_source(&format!(
            "export fn chain() -> i32 {{ return {expression}; }}"
        ));
        let function = &mut module.functions[0];
        let original = function.blocks.remove(0);
        let count = original.instructions.len();
        for (index, instruction) in original.instructions.into_iter().enumerate() {
            function.blocks.push(crate::KirBlock {
                id: BlockId::from_index(index as u32),
                label: format!("chain_{index}"),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: vec![instruction],
                terminator: if index + 1 == count {
                    original.terminator.clone()
                } else {
                    KirTerminator::Jump {
                        edge: crate::KirEdge {
                            target: BlockId::from_index(index as u32 + 1),
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                    }
                },
            });
        }
        function.blocks[1..].reverse();
        let validation = crate::validate_kir_module(&module);
        assert!(validation.errors.is_empty(), "{validation:?}");
        let (proofs, rewrites) = propose_with_config(
            &module,
            ScalarAnalysisConfig::with_max_steps(count as u32 * 3),
        )
        .expect("bounded sparse proposal");

        assert_eq!(
            rewrites.len(),
            39,
            "only dependent users should be revisited"
        );
        assert!(verify_and_apply(&mut module, &proofs, &rewrites).expect("verified rewrite"));
        assert!(module.functions[0].blocks[1].instructions.iter().any(|instruction|
            matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "40")));
    }

    #[test]
    fn constant_rewrite_should_reject_wrong_replacement_without_partial_mutation() {
        let mut module = module();
        let before = module.clone();
        let (proofs, mut rewrites) = propose(&module).expect("proposal");
        rewrites.last_mut().expect("second rewrite").constant =
            FoldedConstant::Integer("85".to_string());

        let error = verify_and_apply(&mut module, &proofs, &rewrites).expect_err("wrong result");
        assert!(error.contains("does not match its certificate"), "{error}");
        assert_eq!(module, before);
    }

    #[test]
    fn constant_rewrite_should_reject_false_claim_without_partial_mutation() {
        let mut module = module();
        let before = module.clone();
        let (mut proofs, rewrites) = propose(&module).expect("proposal");
        let rewrite = rewrites.last().expect("second rewrite");
        let step =
            &mut proofs.get_mut(rewrite.proof).expect("proof").steps[rewrite.step.index() as usize];
        let ProofStep::BinaryTransfer { claim, .. } = step else {
            panic!("binary certificate");
        };
        claim.interval =
            super::super::super::ScalarInterval::new(85.into(), 85.into()).expect("interval");

        let error = verify_and_apply(&mut module, &proofs, &rewrites).expect_err("false claim");
        assert!(
            error.contains("invalid SCCP rewrite certificate"),
            "{error}"
        );
        assert_eq!(module, before);
    }

    #[test]
    fn constant_rewrite_should_reject_stale_operation_before_any_mutation() {
        let mut module = module();
        let (proofs, rewrites) = propose(&module).expect("proposal");
        let target = rewrites.last().expect("second rewrite").instruction;
        let instruction = module.functions[0].blocks[0]
            .instructions
            .iter_mut()
            .find(|instruction| instruction.id == target)
            .expect("binary");
        let KirInstructionKind::Binary { op, .. } = &mut instruction.kind else {
            panic!("binary");
        };
        *op = crate::MirBinaryOp::Sub;
        let before = module.clone();

        assert!(verify_and_apply(&mut module, &proofs, &rewrites).is_err());
        assert_eq!(module, before);
    }

    #[test]
    fn constant_rewrite_should_reject_false_comparison_certificate() {
        let mut module = module_from_source("export fn compare() -> bool { return 20 < 22; }");
        let before = module.clone();
        let (mut proofs, rewrites) = propose(&module).expect("proposal");
        let rewrite = rewrites.last().expect("comparison rewrite");
        let step =
            &mut proofs.get_mut(rewrite.proof).expect("proof").steps[rewrite.step.index() as usize];
        let ProofStep::IntegerComparison { result, .. } = step else {
            panic!("comparison proof");
        };
        *result = false;

        let error =
            verify_and_apply(&mut module, &proofs, &rewrites).expect_err("false comparison");
        assert!(error.contains("integer comparison claim"), "{error}");
        assert_eq!(module, before);
    }

    #[test]
    fn constant_rewrite_should_reject_copy_of_a_different_value() {
        let mut module = module();
        let instructions = &mut module.functions[0].blocks[0].instructions;
        let first = instructions[0].results[0].value;
        let second = instructions[1].results[0].value;
        instructions[3].kind = KirInstructionKind::Copy { value: first };
        let (proofs, rewrites) = propose(&module).expect("proposal");
        module.functions[0].blocks[0].instructions[3].kind =
            KirInstructionKind::Copy { value: second };
        let before = module.clone();

        let error = verify_and_apply(&mut module, &proofs, &rewrites).expect_err("stale copy");
        assert!(error.contains("copy claim"), "{error}");
        assert_eq!(module, before);
    }

    #[test]
    fn constant_rewrite_should_discard_partial_proposals_when_budget_is_exhausted() {
        let module = module();
        for budget in [0, 1, 4] {
            let (proofs, rewrites) =
                propose_with_config(&module, crate::ScalarAnalysisConfig::with_max_steps(budget))
                    .expect("bounded proposal");
            assert!(
                rewrites.is_empty(),
                "budget {budget} must not publish partial analysis"
            );
            assert!(proofs.proofs().is_empty());
        }
    }

    #[test]
    fn constant_rewrite_should_reject_a_missing_phi_edge() {
        let mut module = module_from_source(
            "export fn phi(flag: bool) -> i32 { let x: i32 = 0; if flag { x = 42; } else { x = 42; } return x + 1; }",
        );
        let before = module.clone();
        let (mut proofs, rewrites) = propose(&module).expect("proposal");
        let proof = proofs
            .get_mut(rewrites.last().expect("rewrite").proof)
            .expect("proof");
        let inputs = proof
            .steps
            .iter_mut()
            .find_map(|step| match step {
                ProofStep::PhiJoin { inputs, .. } if inputs.len() == 2 => Some(inputs),
                _ => None,
            })
            .expect("two incoming edges");
        inputs.pop();

        let error =
            verify_and_apply(&mut module, &proofs, &rewrites).expect_err("incomplete phi proof");
        assert!(error.contains("every incoming edge"), "{error}");
        assert_eq!(module, before);
    }

    fn range_module() -> KirModule {
        module_from_source(
            "export fn bounded(n: u32) -> bool { if n < 8 { return n < 16; } return n < 8; }",
        )
    }

    #[test]
    fn constant_rewrite_sparse_worklist_should_revisit_phi_when_branch_bounds_arrive() {
        let mut module = range_module();
        move_entry_branch_after_successors(&mut module);
        let (proofs, rewrites) = propose(&module).expect("proposal");
        assert_eq!(rewrites.len(), 2, "both path-local comparisons must fold");
        assert!(verify_and_apply(&mut module, &proofs, &rewrites).expect("valid scoped proofs"));
    }

    #[test]
    fn constant_rewrite_sparse_worklist_should_propagate_late_refinement_into_arithmetic() {
        for (bound, expected) in [(1, 1), (8, 0)] {
            let mut module = module_from_source(&format!(
                "export fn bounded(n: u32) -> u32 {{ if n < {bound} {{ return n + 7; }} return n; }}"
            ));
            move_entry_branch_after_successors(&mut module);
            let (proofs, rewrites) = propose(&module).expect("proposal");
            assert_eq!(rewrites.len(), expected, "bound {bound}");
            let (repeated, _) = propose(&module).expect("deterministic proposal");
            assert_eq!(proofs, repeated);
            assert_eq!(
                verify_and_apply(&mut module, &proofs, &rewrites).expect("verified"),
                expected != 0
            );
            if bound == 1 {
                assert!(module.functions[0].blocks[1].instructions.iter().any(|instruction|
                    matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "7")));
            }
        }
    }

    fn move_entry_branch_after_successors(module: &mut KirModule) {
        let function = &mut module.functions[0];
        let mut branch = function.blocks[0].clone();
        branch.id = BlockId::from_index(function.blocks.len() as u32);
        branch.label = "late_layout_branch".to_string();
        function.blocks[0].instructions.clear();
        function.blocks[0].terminator = KirTerminator::Jump {
            edge: crate::KirEdge {
                target: branch.id,
                args: Vec::new(),
                memory_args: Vec::new(),
            },
        };
        function.blocks.push(branch);
        let validation = crate::validate_kir_module(module);
        assert!(validation.errors.is_empty(), "{validation:?}");
    }

    fn verify_ranges(module: &KirModule, proofs: &ProofArena) -> crate::EvidenceValidationResult {
        verify_proof_arena(module, &FactArena::new(0), None, proofs, 0)
    }

    #[test]
    fn scalar_certificate_should_reject_a_narrowed_type_bound() {
        let module = range_module();
        let mut proposals = propose_scalar_ranges(&module, None).expect("ranges");
        let proof = proposals
            .proofs
            .get_mut(ProofId::from_index(0))
            .expect("proof");
        let ProofStep::TypeBounds { claim } = &mut proof.steps[0] else {
            panic!("unconstrained parameter");
        };
        claim.interval = crate::ScalarInterval::new(0.into(), 7.into()).expect("interval");
        let validation = verify_ranges(&module, &proposals.proofs);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.message.contains("type-bound")),
            "{validation:?}"
        );
    }

    #[test]
    fn scalar_certificate_should_reject_branch_evidence_at_its_predecessor() {
        let module = range_module();
        let mut proposals = propose_scalar_ranges(&module, None).expect("ranges");
        assert!(verify_ranges(&module, &proposals.proofs).errors.is_empty());
        let proof = proposals
            .proofs
            .get_mut(ProofId::from_index(0))
            .expect("proof");
        let (refinement, comparison, right) = proof
            .steps
            .iter()
            .enumerate()
            .find_map(|(index, step)| match step {
                ProofStep::BranchRefinement {
                    comparison,
                    right,
                    taken: true,
                    ..
                } => Some((ProofStepId::from_index(index as u32), *comparison, *right)),
                _ => None,
            })
            .expect("true edge refinement");
        let value = module.functions[0].blocks[0]
            .instructions
            .iter()
            .find(|instruction| instruction.id == comparison)
            .expect("comparison")
            .results[0]
            .value;
        proof.root = ProofStepId::from_index(proof.steps.len() as u32);
        proof.steps.push(ProofStep::IntegerComparison {
            instruction: comparison,
            left: refinement,
            right,
            value,
            result: true,
        });
        let validation = verify_ranges(&module, &proposals.proofs);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.message.contains("outside its proven scope")),
            "{validation:?}"
        );
    }

    #[test]
    fn scalar_certificate_should_distinguish_two_arms_with_the_same_target() {
        let mut module = range_module();
        let KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &mut module.functions[0].blocks[0].terminator
        else {
            panic!("branch");
        };
        *else_edge = then_edge.clone();
        let mut proposals = propose_scalar_ranges(&module, None).expect("ranges");
        assert!(verify_ranges(&module, &proposals.proofs).errors.is_empty());
        let proof = proposals
            .proofs
            .get_mut(ProofId::from_index(0))
            .expect("proof");
        let inputs = proof
            .steps
            .iter_mut()
            .find_map(|step| match step {
                ProofStep::PhiJoin { inputs, .. } if inputs.len() == 2 => Some(inputs),
                _ => None,
            })
            .expect("two arms of the same branch");
        inputs[1] = inputs[0];
        let validation = verify_ranges(&module, &proposals.proofs);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.message.contains("phi input scope")),
            "{validation:?}"
        );
    }

    #[test]
    fn scalar_certificate_should_reject_a_forged_contract_interval() {
        let source = "export unsafe fn bounded(n: u32) -> bool contract { requires n < 8; } { return n < 8; }";
        let module = module_from_source(source);
        let checked = check(&SourceFile::new("constant-fold.ck", source));
        let contracts =
            crate::import_contract_facts(&module, &checked.checked_program, 0).expect("contracts");
        let mut proposals = propose_scalar_ranges(&module, Some(&contracts)).expect("ranges");
        assert!(
            verify_proof_arena(
                &module,
                contracts.facts(),
                Some(&contracts),
                &proposals.proofs,
                0
            )
            .errors
            .is_empty()
        );
        let proof = proposals
            .proofs
            .get_mut(ProofId::from_index(0))
            .expect("proof");
        let claim = proof
            .steps
            .iter_mut()
            .find_map(|step| match step {
                ProofStep::ContractRange { claim, .. } => Some(claim),
                _ => None,
            })
            .expect("contract range");
        claim.interval = crate::ScalarInterval::new(0.into(), 6.into()).expect("interval");
        let validation = verify_proof_arena(
            &module,
            contracts.facts(),
            Some(&contracts),
            &proposals.proofs,
            0,
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.message.contains("contract range")),
            "{validation:?}"
        );
    }
}
