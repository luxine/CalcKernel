use std::collections::{BTreeMap, BTreeSet};

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

pub(super) struct ScalarProposals {
    pub proofs: ProofArena,
    pub values: BTreeMap<FunctionId, (ProofId, BTreeMap<ValueId, ProofStepId>)>,
    rewrites: Vec<ConstantRewrite>,
}

pub(crate) fn run_integer_constant_folding(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
    protected: &BTreeSet<InstructionId>,
) -> Result<bool, String> {
    let mut proposals = propose_with_contracts(module, contracts, ScalarAnalysisConfig::default())?;
    proposals
        .rewrites
        .retain(|rewrite| !protected.contains(&rewrite.instruction));
    verify_and_apply_with_contracts(module, contracts, &proposals.proofs, &proposals.rewrites)
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
    let mut scalar_values = BTreeMap::new();
    'functions: for function in &module.functions {
        let Some(entry) = function.blocks.first().map(|block| block.id) else {
            continue;
        };
        let mut values = BTreeMap::<ValueId, (ScalarValue, ProofStepId)>::new();
        let mut booleans = BTreeSet::new();
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
        loop {
            let before = steps.len();
            for block in &function.blocks {
                for (index, param) in block.params.iter().enumerate() {
                    let Some(next) = remaining.checked_sub(1) else {
                        continue 'functions;
                    };
                    remaining = next;
                    if values.contains_key(&param.value)
                        || IntegerType::from_mir(&param.type_node).is_none()
                    {
                        continue;
                    }
                    let Some(edges) = incoming.get(&block.id) else {
                        continue;
                    };
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
                }
                for instruction in &block.instructions {
                    let Some(next) = remaining.checked_sub(1) else {
                        continue 'functions;
                    };
                    remaining = next;
                    let [result] = instruction.results.as_slice() else {
                        continue;
                    };
                    if values.contains_key(&result.value) || booleans.contains(&result.value) {
                        continue;
                    }
                    if instruction.effect.is_some() || instruction.memory.is_some() {
                        continue;
                    }
                    let step = ProofStepId::from_index(
                        u32::try_from(steps.len())
                            .map_err(|_| "SCCP proof exceeds u32 identity space")?,
                    );
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
                        booleans.insert(result.value);
                        continue;
                    }
                    let Some(ty) = IntegerType::from_mir(&result.type_node) else {
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
                            semantics: KirArithmeticSemantics::Modular,
                        } => {
                            let (Some((left, left_step)), Some((right, right_step))) =
                                (values.get(left), values.get(right))
                            else {
                                continue;
                            };
                            let value =
                                scalar_binary(*op, KirArithmeticSemantics::Modular, left, right)
                                    .map_err(|error| error.to_string())?;
                            if value.failure() != ScalarFailure::None {
                                continue;
                            }
                            if let Some(constant) = value.exact_value() {
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
                    values.insert(result.value, (value, step));
                }
            }
            if steps.len() == before {
                break;
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
    })
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
    verify_and_apply_with_contracts(module, None, proofs, rewrites)
}

fn verify_and_apply_with_contracts(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    rewrites: &[ConstantRewrite],
) -> Result<bool, String> {
    if rewrites.is_empty() {
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
                Some(ProofStep::IntegerComparison {
                    instruction,
                    value,
                    result,
                    ..
                }),
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
            )
        {
            return Err(
                "SCCP replacement would erase a checked or effectful operation".to_string(),
            );
        }
    }
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
    Ok(true)
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
