use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, FunctionId, InstructionId, KirArithmeticSemantics, KirInstructionKind, KirModule,
    KirTerminator, ProofId, ValueId,
};

use super::super::{
    BoolLattice, FactArena, FactUseSite, IntegerType, ProofArena, ProofStep, ProofStepId,
    ScalarAnalysisBudget, ScalarAnalysisConfig, ScalarClaim, ScalarFailure, ScalarValue,
    scalar_binary, scalar_compare, verify_proof_arena,
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

pub(crate) fn run_integer_constant_folding(module: &mut KirModule) -> Result<bool, String> {
    let (proofs, rewrites) = propose(module)?;
    verify_and_apply(module, &proofs, &rewrites)
}

fn propose(module: &KirModule) -> Result<(ProofArena, Vec<ConstantRewrite>), String> {
    propose_with_config(module, ScalarAnalysisConfig::default())
}

fn propose_with_config(
    module: &KirModule,
    config: ScalarAnalysisConfig,
) -> Result<(ProofArena, Vec<ConstantRewrite>), String> {
    let mut proofs = ProofArena::new(0);
    let mut rewrites = Vec::new();
    'functions: for function in &module.functions {
        let Some(entry) = function.blocks.first().map(|block| block.id) else {
            continue;
        };
        let mut values = BTreeMap::<ValueId, (ScalarValue, ProofStepId)>::new();
        let mut booleans = BTreeSet::new();
        let mut steps = Vec::new();
        let mut pending = Vec::new();
        let mut remaining = ScalarAnalysisBudget::for_function(function, config).max_steps();
        let mut incoming = BTreeMap::<BlockId, Vec<&crate::KirEdge>>::new();
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
                incoming.entry(edge.target).or_default().push(edge);
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
                        .map(|edge| edge.args.get(index).and_then(|value| values.get(value)))
                        .collect::<Option<Vec<_>>>();
                    let Some(inputs) = inputs else {
                        continue;
                    };
                    let Some((first, _)) = inputs.first() else {
                        continue;
                    };
                    if first.exact_value().is_none()
                        || inputs
                            .iter()
                            .any(|(value, _)| value.exact_value() != first.exact_value())
                    {
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
                            first.interval().clone(),
                            ScalarFailure::None,
                        ),
                    });
                    let value = first.clone();
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
                            let value = ScalarValue::constant(
                                ty,
                                value.parse().map_err(|_| "invalid KIR integer")?,
                            )
                            .map_err(|error| error.to_string())?;
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
                            let Some(constant) = value
                                .exact_value()
                                .filter(|_| value.failure() == ScalarFailure::None)
                            else {
                                continue;
                            };
                            pending.push((
                                instruction.id,
                                result.value,
                                FoldedConstant::Integer(constant.to_string()),
                                step,
                            ));
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
                            let Some(constant) = value.exact_value() else {
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
                            pending.push((
                                instruction.id,
                                result.value,
                                FoldedConstant::Integer(constant.to_string()),
                                step,
                            ));
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
        if pending.is_empty() {
            continue;
        }
        let root = pending
            .last()
            .map(|item| item.3)
            .ok_or("missing SCCP proof root")?;
        let proof = proofs
            .try_insert(use_site(function.id, entry), steps, root)
            .map_err(|error| error.to_string())?;
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
    Ok((proofs, rewrites))
}

fn use_site(function: FunctionId, block: BlockId) -> FactUseSite {
    FactUseSite {
        function,
        block,
        instruction: None,
        contract_instance: None,
    }
}

fn verify_and_apply(
    module: &mut KirModule,
    proofs: &ProofArena,
    rewrites: &[ConstantRewrite],
) -> Result<bool, String> {
    if rewrites.is_empty() {
        return Ok(false);
    }
    // Check the closed derivations against the immutable pre-rewrite KIR. The
    // optimizer's ScalarValue result is never itself authority for a rewrite.
    let validation = verify_proof_arena(module, &FactArena::new(0), None, proofs, 0);
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
}
