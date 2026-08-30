use crate::{BlockId, FunctionId, InstructionId, KirInstructionKind, KirModule, ValueId};

use super::super::{
    ContractFactSet, FactUseSite, KirGuardElimination, KirOptimizationExplanation, ProofArena,
    ProofStep, ProofStepId, verify_proof_arena,
};

#[derive(Debug, Clone, Copy)]
struct GuardCandidate {
    function: FunctionId,
    block: BlockId,
    condition_instruction: InstructionId,
    guard_instruction: InstructionId,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_check_elimination(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &mut ProofArena,
    eliminations: &mut Vec<KirGuardElimination>,
    explanations: &mut Vec<KirOptimizationExplanation>,
    generation: u32,
    allow_loop_reasoning: bool,
) -> Result<bool, String> {
    let candidates = collect_candidates(module);
    if candidates.is_empty() {
        return Ok(false);
    }
    let scalars = super::constant_fold::propose_scalar_ranges(module, contracts)?;
    let empty = super::super::FactArena::new(generation);
    let facts = contracts.map_or(&empty, ContractFactSet::facts);
    let validation = verify_proof_arena(module, facts, contracts, &scalars.proofs, generation);
    if let Some(error) = validation.errors.first() {
        return Err(format!(
            "invalid guard range certificate: {}",
            error.message
        ));
    }
    let mut changed = false;
    for candidate in candidates {
        let (mut steps, premises) =
            if let Some((proof, values)) = scalars.values.get(&candidate.function) {
                let certificate = scalars.proofs.get(*proof).ok_or("missing scalar proof")?;
                let mut roots = certificate
                    .steps
                    .iter()
                    .enumerate()
                    .filter_map(|(index, step)| {
                        matches!(step, ProofStep::FactLeaf { .. })
                            .then_some(ProofStepId::from_index(index as u32))
                    })
                    .collect::<Vec<_>>();
                roots.extend(
                    condition_operands(module, candidate)
                        .iter()
                        .filter_map(|value| values.get(value))
                        .copied(),
                );
                certificate
                    .project_steps(&roots)
                    .map_err(|error| error.to_string())?
            } else {
                (Vec::new(), Vec::new())
            };
        let used_trusted_contract = steps.iter().any(|step| matches!(step, ProofStep::FactLeaf { fact }
            if facts.get(*fact).is_some_and(|fact| matches!(fact.origin, super::super::FactOrigin::TrustedContract { .. }))));
        let root = ProofStepId::from_index(steps.len() as u32);
        steps.push(ProofStep::GuardSafety {
            condition_instruction: candidate.condition_instruction,
            premises,
            allow_loop_reasoning,
        });

        let mut proposed = proofs.clone();
        let proof = proposed
            .try_insert(
                FactUseSite {
                    function: candidate.function,
                    block: candidate.block,
                    instruction: Some(candidate.condition_instruction),
                    contract_instance: None,
                },
                steps,
                root,
            )
            .map_err(|error| error.to_string())?;
        let validation = verify_proof_arena(module, facts, contracts, &proposed, generation);
        let accepted = validation.errors.is_empty();
        if !accepted
            && !validation.errors.iter().all(|error| {
                error.proof == Some(proof)
                    && error.step == Some(root.index())
                    && error
                        .message
                        .ends_with("guard-safety claim does not follow from local KIR and premises")
            })
        {
            return Err(format!(
                "invalid guard rewrite certificate: {}",
                validation.errors[0].message
            ));
        }
        if accepted {
            *proofs = proposed;
            remove_guard(module, candidate);
            eliminations.push(KirGuardElimination {
                function: candidate.function,
                block: candidate.block,
                condition_instruction: candidate.condition_instruction,
                guard_instruction: candidate.guard_instruction,
                proof: Some(proof),
                used_trusted_contract,
            });
            explanations.push(KirOptimizationExplanation {
                function: candidate.function,
                block: candidate.block,
                guard_instruction: candidate.guard_instruction,
                removed: true,
                reason: "removed: locally verified guard safety".to_string(),
                proof: Some(proof),
            });
            changed = true;
        } else {
            explanations.push(KirOptimizationExplanation {
                function: candidate.function,
                block: candidate.block,
                guard_instruction: candidate.guard_instruction,
                removed: false,
                reason: "retained: scalar safety is unknown".to_string(),
                proof: None,
            });
        }
    }
    Ok(changed)
}

fn condition_operands(module: &KirModule, candidate: GuardCandidate) -> Vec<ValueId> {
    let Some(instruction) = module
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .and_then(|function| {
            function
                .blocks
                .iter()
                .find(|block| block.id == candidate.block)
        })
        .and_then(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| instruction.id == candidate.condition_instruction)
        })
    else {
        return Vec::new();
    };
    match &instruction.kind {
        KirInstructionKind::Binary { left, right, .. } => vec![*left, *right],
        KirInstructionKind::Unary { operand, .. } => vec![*operand],
        KirInstructionKind::CheckCondition { args, .. } => args.clone(),
        _ => Vec::new(),
    }
}

fn collect_candidates(module: &KirModule) -> Vec<GuardCandidate> {
    let mut candidates = Vec::new();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                let KirInstructionKind::Guard { condition, .. } = instruction.kind else {
                    continue;
                };
                let Some(condition_instruction) = defining_instruction(block, condition) else {
                    continue;
                };
                candidates.push(GuardCandidate {
                    function: function.id,
                    block: block.id,
                    condition_instruction,
                    guard_instruction: instruction.id,
                });
            }
        }
    }
    candidates
}

fn defining_instruction(block: &crate::KirBlock, value: ValueId) -> Option<InstructionId> {
    block.instructions.iter().find_map(|instruction| {
        instruction
            .results
            .iter()
            .any(|result| result.value == value)
            .then_some(instruction.id)
    })
}

fn remove_guard(module: &mut KirModule, candidate: GuardCandidate) {
    if let Some(block) = module
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .and_then(|function| {
            function
                .blocks
                .iter_mut()
                .find(|block| block.id == candidate.block)
        })
    {
        block
            .instructions
            .retain(|instruction| instruction.id != candidate.guard_instruction);
    }
}
