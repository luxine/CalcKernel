use crate::{BlockId, FunctionId, InstructionId, KirInstructionKind, KirModule, ValueId};

use super::super::{
    ContractFactSet, FactScope, FactUseSite, KirGuardElimination, KirOptimizationExplanation,
    ProofArena, ProofStep, ProofStepId, verify_proof_arena,
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
) -> bool {
    let candidates = collect_candidates(module);
    let mut changed = false;
    for candidate in candidates {
        let fact_ids = contracts.map_or_else(Vec::new, |contracts| {
            contracts
                .facts()
                .facts()
                .iter()
                .filter(|fact| {
                    matches!(fact.scope, FactScope::FunctionEntry(function) if function == candidate.function)
                })
                .map(|fact| fact.id)
                .collect::<Vec<_>>()
        });
        let mut steps = fact_ids
            .iter()
            .map(|fact| ProofStep::FactLeaf { fact: *fact })
            .collect::<Vec<_>>();
        let premises = (0..steps.len())
            .map(|index| ProofStepId::from_index(index as u32))
            .collect::<Vec<_>>();
        let root = ProofStepId::from_index(steps.len() as u32);
        steps.push(ProofStep::GuardSafety {
            condition_instruction: candidate.condition_instruction,
            premises,
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
            .ok();
        let accepted = proof.is_some_and(|_| {
            let empty = super::super::FactArena::new(generation);
            let facts = contracts.map_or(&empty, ContractFactSet::facts);
            verify_proof_arena(module, facts, contracts, &proposed, generation)
                .errors
                .is_empty()
        });
        if accepted {
            let proof = proof.expect("accepted proposal has a proof identity");
            *proofs = proposed;
            remove_guard(module, candidate);
            let used_trusted_contract = !fact_ids.is_empty();
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
    changed
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
