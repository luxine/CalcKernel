use crate::{BlockId, FunctionId, InstructionId, KirModule, ProofId, validate_kir_module};

use super::{
    ContractFactSet, EvidenceValidationError, EvidenceValidationResult, FactArena, ProofArena,
    ProofStep, kir_passes, verify_proof_arena,
};

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

/// Deterministic rewrite counts used by acceptance and optimization explanations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KirOptimizationStats {
    pub scalar_functions_analyzed: u32,
    pub inlined_calls: u32,
    pub gvn_rewrites: u32,
    pub forwarded_loads: u32,
    pub eliminated_stores: u32,
    pub natural_loops: u32,
    pub induction_variables: u32,
    pub hoisted_instructions: u32,
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
    pub contract_facts: Option<ContractFactSet>,
    pub stats: KirOptimizationStats,
    verification_cache: Option<VerifiedKirState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedKirState {
    module: KirModule,
    proofs: ProofArena,
    eliminated_guards: Vec<KirGuardElimination>,
    contract_facts: Option<ContractFactSet>,
}

#[must_use]
pub fn run_kir_pass_pipeline(
    mut module: KirModule,
    level: KirOptimizationLevel,
    contracts: Option<&ContractFactSet>,
) -> KirPassManagerResult {
    const GENERATION: u32 = 0;
    let pending_module = KirModule {
        config: module.config,
        entry: None,
        structs: Vec::new(),
        functions: Vec::new(),
    };
    let mut result = KirPassManagerResult {
        module: pending_module,
        artifact: None,
        records: Vec::new(),
        errors: Vec::new(),
        proofs: ProofArena::new(GENERATION),
        eliminated_guards: Vec::new(),
        explanations: Vec::new(),
        contract_facts: contracts.cloned(),
        stats: KirOptimizationStats::default(),
        verification_cache: None,
    };

    let input_evidence = validate_kir_optimization_evidence(
        &module,
        result.contract_facts.as_ref(),
        &result.proofs,
        &result.eliminated_guards,
        GENERATION,
    );
    if !input_evidence.errors.is_empty() {
        result.errors = input_evidence
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect();
        result.module = module;
        return result;
    }
    result.verification_cache = Some(VerifiedKirState {
        module: module.clone(),
        proofs: result.proofs.clone(),
        eliminated_guards: result.eliminated_guards.clone(),
        contract_facts: result.contract_facts.clone(),
    });

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

    let changed = kir_passes::run_cfg_canonicalize(&mut module);
    if !record_current_pass(
        &module,
        "cfg-canonicalize",
        changed,
        &mut result,
        GENERATION,
    ) {
        result.module = module;
        return result;
    }
    let mut scalar_analysis_cache = kir_passes::run_sccp_range(&module);
    add_scalar_analysis_stats(&mut result.stats, scalar_analysis_cache.as_ref());
    if !record_current_pass(&module, "sccp-range", false, &mut result, GENERATION) {
        result.module = module;
        return result;
    }

    let changed = kir_passes::run_check_elimination(
        &mut module,
        result.contract_facts.as_ref(),
        &mut result.proofs,
        &mut result.eliminated_guards,
        &mut result.explanations,
        GENERATION,
        false,
    );
    if !record_current_pass(
        &module,
        "check-elimination",
        changed,
        &mut result,
        GENERATION,
    ) {
        result.module = module;
        return result;
    }

    if matches!(level, KirOptimizationLevel::O2 | KirOptimizationLevel::O3) {
        result.stats.inlined_calls =
            if module.config.sanitizer_mode == crate::KirSanitizerMode::Contracts {
                0
            } else {
                kir_passes::run_effect_aware_inline(
                    &mut module,
                    &mut result.contract_facts,
                    &result.eliminated_guards,
                )
            };
        if result.stats.inlined_calls != 0 {
            kir_passes::run_cleanup(&mut module);
        }
        if !record_current_pass(
            &module,
            "effect-aware-inline",
            result.stats.inlined_calls != 0,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }

        let memory_changed =
            match kir_passes::run_memory_ssa_refine(&mut module, result.contract_facts.as_ref()) {
                Ok(changed) => changed,
                Err(error) => {
                    result.errors.push(error);
                    result.module = module;
                    return result;
                }
            };
        if !record_current_pass(
            &module,
            "memory-ssa-refine",
            memory_changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }

        result.stats.gvn_rewrites = kir_passes::run_gvn(&mut module);
        if !record_current_pass(
            &module,
            "gvn",
            result.stats.gvn_rewrites != 0,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }

        result.stats.forwarded_loads = kir_passes::run_load_forwarding(&mut module);
        if !record_current_pass(
            &module,
            "load-forwarding",
            result.stats.forwarded_loads != 0,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }

        result.stats.eliminated_stores = kir_passes::run_dead_store_elimination(&mut module);
        if !record_current_pass(
            &module,
            "dead-store-elimination",
            result.stats.eliminated_stores != 0,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }

        scalar_analysis_cache = kir_passes::run_sccp_range(&module);
        add_scalar_analysis_stats(&mut result.stats, scalar_analysis_cache.as_ref());
        if !record_current_pass(
            &module,
            "sccp-range-post-inline",
            false,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        let changed = kir_passes::run_check_elimination(
            &mut module,
            result.contract_facts.as_ref(),
            &mut result.proofs,
            &mut result.eliminated_guards,
            &mut result.explanations,
            GENERATION,
            true,
        );
        if !record_current_pass(
            &module,
            "check-elimination-post-inline",
            changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
    }

    if level == KirOptimizationLevel::O3 {
        let loop_analyses = module
            .functions
            .iter()
            .map(super::analyze_natural_loops)
            .collect::<Vec<_>>();
        result.stats.natural_loops = loop_analyses
            .iter()
            .map(|analysis| u32::try_from(analysis.loops.len()).unwrap_or(u32::MAX))
            .fold(0_u32, u32::saturating_add);
        result.stats.induction_variables = loop_analyses
            .iter()
            .map(|analysis| u32::try_from(analysis.inductions.len()).unwrap_or(u32::MAX))
            .fold(0_u32, u32::saturating_add);
        if !record_current_pass(
            &module,
            "natural-loop-analysis",
            false,
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
        result.stats.hoisted_instructions =
            kir_passes::run_licm(&mut module, &protected, &loop_analyses);
        if !record_current_pass(
            &module,
            "licm",
            result.stats.hoisted_instructions != 0,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        if !record_current_pass(
            &module,
            "induction-simplify",
            false,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        if scalar_analysis_cache
            .as_ref()
            .is_none_or(|cache| !cache.covers(&module))
        {
            let loop_scalar_analysis = kir_passes::run_sccp_range(&module);
            add_scalar_analysis_stats(&mut result.stats, loop_scalar_analysis.as_ref());
        }
        if !record_current_pass(
            &module,
            "sccp-range-post-loop",
            false,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        let changed = kir_passes::run_check_elimination(
            &mut module,
            result.contract_facts.as_ref(),
            &mut result.proofs,
            &mut result.eliminated_guards,
            &mut result.explanations,
            GENERATION,
            true,
        );
        if !record_current_pass(
            &module,
            "check-elimination-post-loop",
            changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
    }

    let protected = result
        .eliminated_guards
        .iter()
        .map(|elimination| elimination.condition_instruction)
        .collect();
    let changed = kir_passes::run_dead_code_elimination(&mut module, &protected);
    if !record_current_pass(
        &module,
        "dead-code-elimination",
        changed,
        &mut result,
        GENERATION,
    ) {
        result.module = module;
        return result;
    }

    let changed = kir_passes::run_cleanup(&mut module);
    if !record_current_pass(&module, "cleanup", changed, &mut result, GENERATION) {
        result.module = module;
        return result;
    }

    result.module = module.clone();
    result.artifact = Some(module);
    result
}

fn add_scalar_analysis_stats(
    stats: &mut KirOptimizationStats,
    cache: Option<&kir_passes::ScalarAnalysisCache>,
) {
    let analyzed = cache.map_or(0, |cache| {
        u32::try_from(cache.analyzed_functions()).unwrap_or(u32::MAX)
    });
    stats.scalar_functions_analyzed = stats.scalar_functions_analyzed.saturating_add(analyzed);
}

fn record_verified_pass(
    module: &KirModule,
    name: &str,
    changed: bool,
    result: &mut KirPassManagerResult,
    generation: u32,
) -> bool {
    // A pass's preservation declaration is untrusted in every build profile.
    // Reuse evidence only after independently checking the entire verified state.
    let cache_hit = !changed
        && result.verification_cache.as_ref().is_some_and(|cached| {
            cached.module == *module
                && cached.proofs == result.proofs
                && cached.eliminated_guards == result.eliminated_guards
                && cached.contract_facts == result.contract_facts
        });
    if cache_hit {
        result.records.push(KirPassRecord {
            name: name.to_string(),
            changed,
            verified: true,
        });
        return true;
    }
    let evidence = validate_kir_optimization_evidence(
        module,
        result.contract_facts.as_ref(),
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
    if verified {
        result.verification_cache = Some(VerifiedKirState {
            module: module.clone(),
            proofs: result.proofs.clone(),
            eliminated_guards: result.eliminated_guards.clone(),
            contract_facts: result.contract_facts.clone(),
        });
    }
    verified
}

fn record_current_pass(
    module: &KirModule,
    name: &str,
    changed: bool,
    result: &mut KirPassManagerResult,
    generation: u32,
) -> bool {
    record_verified_pass(module, name, changed, result, generation)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FactId, KirBoundsMode, KirBuildConfig, KirConsumer, KirInstructionKind, KirOverflowMode,
        KirSanitizerMode, SourceFile, ValueId, build_kir_module, check, import_contract_facts,
        lower_to_mir,
    };

    fn verified_result() -> KirPassManagerResult {
        let checked = check(&SourceFile::new(
            "verification-cache.ck",
            "export unsafe fn answer(n: u32) -> u32 contract { requires n < 8; } { return n + 1; }",
        ));
        assert!(checked.diagnostics.is_empty());
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let kir = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::Inspection,
                overflow_mode: KirOverflowMode::Checked,
                bounds_mode: KirBoundsMode::Checked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid KIR");
        let contracts =
            import_contract_facts(&kir, &checked.checked_program, 0).expect("valid contract facts");
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, Some(&contracts));
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.eliminated_guards.len(), 1);
        result
    }

    fn rejects_false_preservation(mut result: KirPassManagerResult) {
        let module = result.module.clone();
        assert!(
            !record_current_pass(&module, "fault-injected-pass", false, &mut result, 0),
            "a pass's false no-change claim must not bypass evidence validation"
        );
        assert!(!result.errors.is_empty());
        assert!(!result.records.last().expect("failed pass record").verified);
    }

    #[test]
    fn verifier_cache_should_reject_unreported_ir_mutation() {
        let mut result = verified_result();
        let binary = result.module.functions[0].blocks[0]
            .instructions
            .iter_mut()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
            .expect("arithmetic instruction");
        let KirInstructionKind::Binary { left, .. } = &mut binary.kind else {
            unreachable!();
        };
        *left = ValueId::from_index(u32::MAX);
        rejects_false_preservation(result);
    }

    #[test]
    fn verifier_cache_should_reject_unreported_proof_mutation() {
        let mut result = verified_result();
        result
            .proofs
            .get_mut(ProofId::from_index(0))
            .expect("proof")
            .generation = 1;
        rejects_false_preservation(result);
    }

    #[test]
    fn verifier_cache_should_reject_unreported_rewrite_mutation() {
        let mut result = verified_result();
        result.eliminated_guards[0].proof = None;
        rejects_false_preservation(result);
    }

    #[test]
    fn verifier_cache_should_reject_unreported_contract_mutation() {
        let mut result = verified_result();
        result
            .contract_facts
            .as_mut()
            .expect("contracts")
            .facts_mut()
            .get_mut(FactId::from_index(0))
            .expect("fact")
            .generation = 1;
        rejects_false_preservation(result);
    }

    #[test]
    fn verifier_cache_should_accept_identical_verified_state() {
        let mut result = verified_result();
        let module = result.module.clone();
        assert!(record_current_pass(
            &module,
            "unchanged-pass",
            false,
            &mut result,
            0
        ));
        assert!(result.errors.is_empty());
    }
}
