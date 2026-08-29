use crate::{BlockId, FunctionId, InstructionId, KirModule, ProofId, validate_kir_module};

use super::{
    ContractFactSet, EvidenceValidationError, EvidenceValidationResult, FactArena, ProofArena,
    ProofStep, kir_passes, verify_proof_arena,
};

type KirSimplePass = fn(&mut KirModule) -> bool;

/// Stable optimization levels for the KIR pass manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KirOptimizationLevel {
    O0,
    O1,
    O2,
    O3,
}

/// One verified pass transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirPassRecord {
    pub name: String,
    pub changed: bool,
    pub verified: bool,
}

/// Evidence authorizing one removed ordered guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirGuardElimination {
    pub function: FunctionId,
    pub block: BlockId,
    pub condition_instruction: InstructionId,
    pub guard_instruction: InstructionId,
    pub proof: Option<ProofId>,
    pub used_trusted_contract: bool,
}

/// Deterministic explanation for an eliminated or retained guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirOptimizationExplanation {
    pub function: FunctionId,
    pub block: BlockId,
    pub guard_instruction: InstructionId,
    pub removed: bool,
    pub reason: String,
    pub proof: Option<ProofId>,
}

/// Transactional output. `artifact` is absent whenever any verification failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirPassManagerResult {
    pub module: KirModule,
    pub artifact: Option<KirModule>,
    pub records: Vec<KirPassRecord>,
    pub errors: Vec<String>,
    pub proofs: ProofArena,
    pub eliminated_guards: Vec<KirGuardElimination>,
    pub explanations: Vec<KirOptimizationExplanation>,
}

#[must_use]
pub fn run_kir_pass_pipeline(
    mut module: KirModule,
    level: KirOptimizationLevel,
    contracts: Option<&ContractFactSet>,
) -> KirPassManagerResult {
    const GENERATION: u32 = 0;
    let mut result = KirPassManagerResult {
        module: module.clone(),
        artifact: None,
        records: Vec::new(),
        errors: Vec::new(),
        proofs: ProofArena::new(GENERATION),
        eliminated_guards: Vec::new(),
        explanations: Vec::new(),
    };

    let input_errors = validate_kir_module(&module).errors;
    if !input_errors.is_empty() {
        result.errors = input_errors
            .into_iter()
            .map(|error| error.message)
            .collect();
        return result;
    }

    if level == KirOptimizationLevel::O0 {
        result.records.push(KirPassRecord {
            name: "verify-o0".to_string(),
            changed: false,
            verified: true,
        });
        result.module = module.clone();
        result.artifact = Some(module);
        return result;
    }

    let passes: [(&str, KirSimplePass); 2] = [
        ("cfg-canonicalize", kir_passes::run_cfg_canonicalize),
        ("sccp-range", kir_passes::run_sccp_range),
    ];
    for (name, pass) in passes {
        let changed = pass(&mut module);
        if !record_verified_pass(&module, name, changed, contracts, &mut result, GENERATION) {
            result.module = module;
            return result;
        }
    }

    let changed = kir_passes::run_check_elimination(
        &mut module,
        contracts,
        &mut result.proofs,
        &mut result.eliminated_guards,
        &mut result.explanations,
        GENERATION,
    );
    if !record_verified_pass(
        &module,
        "check-elimination",
        changed,
        contracts,
        &mut result,
        GENERATION,
    ) {
        result.module = module;
        return result;
    }

    let protected = result
        .eliminated_guards
        .iter()
        .map(|elimination| elimination.condition_instruction)
        .collect();
    let changed = kir_passes::run_dead_code_elimination(&mut module, &protected);
    if !record_verified_pass(
        &module,
        "dead-code-elimination",
        changed,
        contracts,
        &mut result,
        GENERATION,
    ) {
        result.module = module;
        return result;
    }

    let changed = kir_passes::run_cleanup(&mut module);
    if !record_verified_pass(
        &module,
        "cleanup",
        changed,
        contracts,
        &mut result,
        GENERATION,
    ) {
        result.module = module;
        return result;
    }

    result.module = module.clone();
    result.artifact = Some(module);
    result
}

fn record_verified_pass(
    module: &KirModule,
    name: &str,
    changed: bool,
    contracts: Option<&ContractFactSet>,
    result: &mut KirPassManagerResult,
    generation: u32,
) -> bool {
    let evidence = validate_kir_optimization_evidence(
        module,
        contracts,
        &result.proofs,
        &result.eliminated_guards,
        generation,
    );
    let verified = evidence.errors.is_empty();
    result.records.push(KirPassRecord {
        name: name.to_string(),
        changed,
        verified,
    });
    result
        .errors
        .extend(evidence.errors.into_iter().map(|error| error.message));
    verified
}

/// Rechecks the rewritten KIR, all fact/proof certificates, and each rewrite-to-proof binding.
#[must_use]
pub fn validate_kir_optimization_evidence(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    eliminations: &[KirGuardElimination],
    generation: u32,
) -> EvidenceValidationResult {
    let mut errors = Vec::new();
    for error in validate_kir_module(module).errors {
        let authorized_missing_guard = error.instruction.is_some_and(|instruction| {
            eliminations.iter().any(|elimination| {
                elimination.condition_instruction == instruction && elimination.proof.is_some()
            })
        }) && matches!(
            error.message.as_str(),
            "checked arithmetic result is not followed by its required overflow guard"
                | "check condition is not followed by its required guard"
        );
        if !authorized_missing_guard {
            errors.push(EvidenceValidationError {
                message: error.message,
                fact: None,
                proof: None,
                step: None,
            });
        }
    }

    let empty_facts = FactArena::new(generation);
    let facts = contracts.map_or(&empty_facts, ContractFactSet::facts);
    errors.extend(verify_proof_arena(module, facts, contracts, proofs, generation).errors);
    for elimination in eliminations {
        let Some(proof_id) = elimination.proof else {
            errors.push(rewrite_error(
                "guard elimination has no proof certificate",
                None,
            ));
            continue;
        };
        let Some(certificate) = proofs.get(proof_id) else {
            errors.push(rewrite_error(
                "guard elimination names a missing proof certificate",
                Some(proof_id),
            ));
            continue;
        };
        let matches_rewrite = certificate
            .steps
            .get(certificate.root.index() as usize)
            .is_some_and(|root| {
                matches!(
                    root,
                    ProofStep::GuardSafety {
                        condition_instruction,
                        ..
                    } if *condition_instruction == elimination.condition_instruction
                )
            });
        if !matches_rewrite
            || certificate.use_site.function != elimination.function
            || certificate.use_site.block != elimination.block
        {
            errors.push(rewrite_error(
                "guard elimination does not match its proof certificate",
                Some(proof_id),
            ));
        }
    }
    EvidenceValidationResult { errors }
}

fn rewrite_error(message: &str, proof: Option<ProofId>) -> EvidenceValidationError {
    EvidenceValidationError {
        message: message.to_string(),
        fact: None,
        proof,
        step: None,
    }
}
