use crate::{
    BlockId, FunctionId, InstructionId, KirFunction, KirModule, ProofId, validate_kir_module,
};

use super::{
    CandidateBudgetCharge, CandidateDisposition, ContractFactSet, EvidenceValidationError,
    EvidenceValidationResult, FactArena, KirOptimizationAuditState, KirVerifiedProgramState,
    ProofArena, ProofStep, TransactionOutcome, check_slp_plan_independently,
    check_specialization_plan_independently, check_unroll_plan_independently,
    check_vectorization_trial_independently, discover_slp_candidates,
    discover_specialization_candidates, discover_unroll_candidates,
    discover_vectorization_candidates, execute_verified_transaction_with_disposition,
    is_specialization_clone, kir_function_units, kir_passes, verify_proof_arena,
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

/// Stable explanation for one independently checked Loop SIMD plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirVectorOptimizationExplanation {
    pub candidate: super::CandidateKey,
    pub disposition: CandidateDisposition,
    pub vf: u16,
    pub uf: u8,
    pub predicates: Vec<String>,
    pub cost: super::KirCostEstimate,
    pub growth: super::VectorPlanGrowth,
    pub proofs: super::VectorProofRoots,
    pub reason: String,
}

impl KirVectorOptimizationExplanation {
    #[must_use]
    pub fn stable_text(&self) -> String {
        format!(
            "vector-plan candidate={} disposition={} vf={} uf={} predicates={} cost=scalar:{};transformed:{};predicates:{};epilogue:{};total:{} growth=function:{}->{};module:{}->{} proofs=canonical:{};partition:{};lanes:{};operations:{};fallback:{};target:{};budget:{} reason={}",
            self.candidate.stable_text(),
            self.disposition.stable_name(),
            self.vf,
            self.uf,
            self.predicates.join(","),
            self.cost.scalar,
            self.cost.transformed_body,
            self.cost.predicates,
            self.cost.epilogue,
            self.cost.total,
            self.growth.original_units,
            self.growth.transformed_units,
            self.growth.module_before_units,
            self.growth.module_after_units,
            self.proofs.canonical_loop.index(),
            self.proofs.trip_partition.index(),
            self.proofs.lane_mapping.index(),
            self.proofs.operation_equivalence.index(),
            self.proofs.fallback_identity.index(),
            self.proofs.target_legality.index(),
            self.proofs.cost_and_budget.index(),
            self.reason,
        )
    }
}

/// A conservative analysis outcome that does not refer to a particular guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirAnalysisFallback {
    pub function: FunctionId,
    pub pass: String,
    pub reason: String,
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
    pub induction_simplifications: u32,
    pub induction_budget_fallbacks: u32,
    pub canonical_loops: u32,
    pub loop_legality_candidates: u32,
    pub specialized_clones: u32,
    pub rejected_specializations: u32,
    pub reused_specializations: u32,
    pub specialization_limit_fallbacks: u32,
    pub full_unrolled_loops: u32,
    pub partial_unrolled_loops_factor_2: u32,
    pub partial_unrolled_loops_factor_4: u32,
    pub rejected_unroll_candidates: u32,
    pub staged_native_unroll_candidates: u32,
    pub slp_packs: u32,
    pub staged_native_slp_candidates: u32,
    pub rejected_slp_candidates: u32,
    pub vectorized_loops: u32,
    pub rejected_vector_candidates: u32,
    pub vector_scalar_fallbacks: u32,
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
    pub vector_explanations: Vec<KirVectorOptimizationExplanation>,
    pub analysis_fallbacks: Vec<KirAnalysisFallback>,
    pub contract_facts: Option<ContractFactSet>,
    pub stats: KirOptimizationStats,
    pub audit: KirOptimizationAuditState,
    /// Independently checked O3 workload guidance, absent for ordinary/O2 builds.
    pub pgo: Option<super::CkPgoOptimizerPlan>,
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
    module: KirModule,
    level: KirOptimizationLevel,
    contracts: Option<&ContractFactSet>,
) -> KirPassManagerResult {
    run_kir_pass_pipeline_with_profile(module, level, contracts, None)
}

pub(crate) fn run_kir_pass_pipeline_with_profile(
    mut module: KirModule,
    level: KirOptimizationLevel,
    contracts: Option<&ContractFactSet>,
    pgo: Option<&super::CkPgoOptimizerPlan>,
) -> KirPassManagerResult {
    const GENERATION: u32 = 0;
    let input_audit = KirOptimizationAuditState::for_module(&module);
    let pending_module = KirModule {
        config: module.config,
        profile: module.profile.clone(),
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
        vector_explanations: Vec::new(),
        analysis_fallbacks: Vec::new(),
        contract_facts: contracts.cloned(),
        stats: KirOptimizationStats::default(),
        audit: input_audit,
        pgo: None,
        verification_cache: None,
    };
    let mut o3_entry_module_units = None;

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

    let changed = kir_passes::run_cfg_canonicalize(&mut module, result.contract_facts.as_ref());
    if changed && let Err(error) = refresh_pre_guard_cfg(&module, &mut result.contract_facts) {
        result.errors.push(error);
        result.module = module;
        return result;
    }
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
    let changed = match run_pre_guard_sccp(&mut module, &mut result.contract_facts) {
        Ok(changed) => changed,
        Err(error) => {
            result.errors.push(error);
            result.module = module;
            return result;
        }
    };
    let mut scalar_analysis_cache = kir_passes::run_sccp_range(&module);
    add_scalar_analysis_stats(&mut result.stats, scalar_analysis_cache.as_ref());
    if !record_current_pass(&module, "sccp-range", changed, &mut result, GENERATION) {
        result.module = module;
        return result;
    }

    if level == KirOptimizationLevel::O3 {
        let simplified = match kir_passes::canonicalize_kir_loops(&mut module) {
            Ok(simplified) => simplified,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
        if simplified.changed
            && let Err(error) = refresh_pre_guard_cfg(&module, &mut result.contract_facts)
        {
            result.errors.push(error);
            result.module = module;
            return result;
        }
        result.stats.canonical_loops = simplified.normalized_loops;
        result
            .analysis_fallbacks
            .extend(
                simplified
                    .fallbacks
                    .into_iter()
                    .map(|fallback| KirAnalysisFallback {
                        function: fallback.function,
                        pass: "loop-simplify".to_string(),
                        reason: fallback.reason.stable_name().to_string(),
                    }),
            );
        if !record_current_pass(
            &module,
            "loop-simplify",
            simplified.changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        scalar_analysis_cache = kir_passes::run_sccp_range(&module);
        add_scalar_analysis_stats(&mut result.stats, scalar_analysis_cache.as_ref());
    }

    let changed = match kir_passes::run_check_elimination(
        &mut module,
        result.contract_facts.as_ref(),
        &mut result.proofs,
        &mut result.eliminated_guards,
        &mut result.explanations,
        GENERATION,
        false,
    ) {
        Ok(changed) => changed,
        Err(error) => {
            result.errors.push(error);
            result.module = module;
            return result;
        }
    };
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

    if level == KirOptimizationLevel::O3 {
        // This is the original-function O3 entry fixed by the 0.12 budget
        // contract. No candidate work has executed before this point.
        result.audit = KirOptimizationAuditState::for_module(&module);
        let mut specialization_discovery =
            discover_specialization_candidates(&module, result.contract_facts.as_ref());
        if let Some(pgo) = pgo {
            specialization_discovery.candidates.sort_by(|left, right| {
                (
                    !pgo.function_is_hot(left.caller),
                    !pgo.function_is_hot(left.callee),
                    &left.key,
                )
                    .cmp(&(
                        !pgo.function_is_hot(right.caller),
                        !pgo.function_is_hot(right.callee),
                        &right.key,
                    ))
            });
        }
        let specialization = if specialization_discovery.candidates.is_empty() {
            specialization_frontier_result(&specialization_discovery)
        } else {
            let mut state =
                match verified_program_state_from_pass_cache(&module, &result, GENERATION, None) {
                    Ok(state) => state,
                    Err(error) => {
                        result.errors.push(error);
                        result.module = module;
                        return result;
                    }
                };
            o3_entry_module_units = Some(state.optimization_entry_module_units());
            let specialization = match run_specialization_frontier(
                &mut state,
                &mut result.audit,
                specialization_discovery,
            ) {
                Ok(specialization) => specialization,
                Err(error) => {
                    result.errors.push(error);
                    result.module = module;
                    return result;
                }
            };
            module = state.module().clone();
            result.contract_facts = state.contract_facts().cloned();
            result.proofs = state.proofs().clone();
            result.eliminated_guards = state.eliminated_guards().to_vec();
            specialization
        };
        o3_entry_module_units.get_or_insert_with(|| {
            module.functions.iter().fold(0_u32, |total, function| {
                total.saturating_add(kir_function_units(function))
            })
        });
        result.stats.specialized_clones = specialization.accepted;
        result.stats.rejected_specializations = specialization.rejected;
        result.stats.reused_specializations = specialization.reused;
        result.stats.specialization_limit_fallbacks = specialization.limits;
        result.analysis_fallbacks.extend(specialization.fallbacks);
        if !record_current_pass(
            &module,
            "specialization-frontier",
            specialization.changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
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
                    pgo,
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

        result.stats.gvn_rewrites =
            kir_passes::run_gvn(&mut module, &result.proofs.instruction_dependencies());
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

        let changed = match kir_passes::run_integer_constant_folding(
            &mut module,
            result.contract_facts.as_ref(),
            &result.proofs,
        ) {
            Ok(changed) => changed,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
        scalar_analysis_cache = kir_passes::run_sccp_range(&module);
        add_scalar_analysis_stats(&mut result.stats, scalar_analysis_cache.as_ref());
        if !record_current_pass(
            &module,
            "sccp-range-post-inline",
            changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        let changed = match kir_passes::run_check_elimination(
            &mut module,
            result.contract_facts.as_ref(),
            &mut result.proofs,
            &mut result.eliminated_guards,
            &mut result.explanations,
            GENERATION,
            true,
        ) {
            Ok(changed) => changed,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
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
        let canonical_loop_analyses = module
            .functions
            .iter()
            .zip(&loop_analyses)
            .map(|(function, natural)| {
                super::analyze_canonical_loops_from_natural_for_discovery(function, natural)
            })
            .collect::<Vec<_>>();
        for (function, analysis) in module.functions.iter().zip(&loop_analyses) {
            let reason = if analysis.budget_exhausted {
                Some("fixed-kir-budget-exhausted")
            } else if !analysis.irreducible_blocks.is_empty() {
                Some("irreducible-control-flow")
            } else {
                None
            };
            if let Some(reason) = reason {
                result.analysis_fallbacks.push(KirAnalysisFallback {
                    function: function.id,
                    pass: "natural-loop-analysis".to_string(),
                    reason: reason.to_string(),
                });
            }
        }
        result.stats.natural_loops = canonical_loop_analyses
            .iter()
            .map(|analysis| analysis.natural_loop_count)
            .fold(0_u32, u32::saturating_add);
        result.stats.induction_variables = canonical_loop_analyses
            .iter()
            .map(|analysis| analysis.induction_count)
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

        for (function, canonical) in module.functions.iter().zip(&canonical_loop_analyses) {
            for fallback in &canonical.fallbacks {
                result.analysis_fallbacks.push(KirAnalysisFallback {
                    function: fallback.function,
                    pass: "loop-legality".to_string(),
                    reason: fallback.reason.stable_name().to_string(),
                });
            }
            if !module.profile.vector_operations_enabled() {
                continue;
            }
            for descriptor in canonical
                .loops
                .iter()
                .filter(|descriptor| descriptor.innermost)
            {
                result.stats.loop_legality_candidates =
                    result.stats.loop_legality_candidates.saturating_add(1);
                match super::analyze_loop_legality(
                    function,
                    descriptor,
                    result.contract_facts.as_ref().map(ContractFactSet::facts),
                ) {
                    Ok(legality) => {
                        for reason in legality.fallback_reasons {
                            result.analysis_fallbacks.push(KirAnalysisFallback {
                                function: function.id,
                                pass: "loop-legality".to_string(),
                                reason: reason.stable_name().to_string(),
                            });
                        }
                    }
                    Err(error) => result.analysis_fallbacks.push(KirAnalysisFallback {
                        function: function.id,
                        pass: "loop-legality".to_string(),
                        reason: format!("analysis-unavailable:{error}"),
                    }),
                }
            }
        }
        result.analysis_fallbacks.sort_by(|left, right| {
            (left.function, left.pass.as_str(), left.reason.as_str()).cmp(&(
                right.function,
                right.pass.as_str(),
                right.reason.as_str(),
            ))
        });
        result.analysis_fallbacks.dedup();
        if !record_current_pass(
            &module,
            "loop-legality-analysis",
            false,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }

        let protected = result.proofs.instruction_dependencies();
        let licm = match kir_passes::run_licm(&mut module, &protected, &loop_analyses) {
            Ok(result) => result,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
        result.stats.hoisted_instructions = licm.hoisted;
        for function in licm.exhausted_functions {
            result.analysis_fallbacks.push(KirAnalysisFallback {
                function,
                pass: "licm".to_string(),
                reason: "fixed-kir-budget-exhausted".to_string(),
            });
        }
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
        let simplified = match kir_passes::run_induction_simplification(
            &mut module,
            &result.proofs,
            &loop_analyses,
        ) {
            Ok(result) => result,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
        let canonical_discovery_preserved = simplified.simplified == 0;
        result.stats.induction_simplifications = simplified.simplified;
        result.stats.induction_budget_fallbacks = simplified.exhausted_functions.len() as u32;
        for function in simplified.exhausted_functions {
            result.analysis_fallbacks.push(KirAnalysisFallback {
                function,
                pass: "induction-simplify".to_string(),
                reason: "fixed-kir-budget-exhausted".to_string(),
            });
        }
        if !record_current_pass(
            &module,
            "induction-simplify",
            simplified.simplified != 0,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        let changed = match kir_passes::run_integer_constant_folding(
            &mut module,
            result.contract_facts.as_ref(),
            &result.proofs,
        ) {
            Ok(changed) => changed,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
        if changed
            || scalar_analysis_cache
                .as_ref()
                .is_none_or(|cache| !cache.covers(&module))
        {
            let loop_scalar_analysis = kir_passes::run_sccp_range(&module);
            add_scalar_analysis_stats(&mut result.stats, loop_scalar_analysis.as_ref());
        }
        if !record_current_pass(
            &module,
            "sccp-range-post-loop",
            changed,
            &mut result,
            GENERATION,
        ) {
            result.module = module;
            return result;
        }
        let changed = match kir_passes::run_check_elimination(
            &mut module,
            result.contract_facts.as_ref(),
            &mut result.proofs,
            &mut result.eliminated_guards,
            &mut result.explanations,
            GENERATION,
            true,
        ) {
            Ok(changed) => changed,
            Err(error) => {
                result.errors.push(error);
                result.module = module;
                return result;
            }
        };
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
        if let Some(unroll_fallbacks) = no_op_native_frontiers(
            &module,
            canonical_discovery_preserved.then_some(canonical_loop_analyses.as_slice()),
        ) {
            result.analysis_fallbacks.extend(unroll_fallbacks);
            for name in [
                "loop-vector-frontier",
                "loop-optimization-frontier",
                "residual-slp-frontier",
            ] {
                if !record_current_pass(&module, name, false, &mut result, GENERATION) {
                    result.module = module;
                    return result;
                }
            }
        } else {
            let optimization_entry_module_units = o3_entry_module_units.unwrap_or_else(|| {
                module.functions.iter().fold(0_u32, |total, function| {
                    total.saturating_add(kir_function_units(function))
                })
            });
            let mut state = match verified_program_state_from_pass_cache(
                &module,
                &result,
                GENERATION,
                Some(optimization_entry_module_units),
            ) {
                Ok(state) => state,
                Err(error) => {
                    result.errors.push(error);
                    result.module = module;
                    return result;
                }
            };
            let vector = match run_native_vector_frontier(&mut state, &mut result.audit, pgo) {
                Ok(vector) => vector,
                Err(error) => {
                    result.errors.push(error);
                    result.module = module;
                    return result;
                }
            };
            module = state.module().clone();
            result.contract_facts = state.contract_facts().cloned();
            result.proofs = state.proofs().clone();
            result.eliminated_guards = state.eliminated_guards().to_vec();
            result.stats.vectorized_loops = vector.accepted;
            result.stats.rejected_vector_candidates = vector.rejected;
            result.stats.vector_scalar_fallbacks = vector.scalar_fallbacks;
            let loop_slp_nonwinners = vector.slp_nonwinners;
            let loop_slp_winners = vector.slp_winners;
            let loop_slp_rejected = vector.slp_rejected;
            result.analysis_fallbacks.extend(vector.fallbacks);
            result.vector_explanations.extend(vector.explanations);
            if !record_current_pass(
                &module,
                "loop-vector-frontier",
                vector.changed,
                &mut result,
                GENERATION,
            ) {
                result.module = module;
                return result;
            }
            let unroll = match run_unroll_frontier(&mut state, &mut result.audit) {
                Ok(unroll) => unroll,
                Err(error) => {
                    result.errors.push(error);
                    result.module = module;
                    return result;
                }
            };
            module = state.module().clone();
            result.contract_facts = state.contract_facts().cloned();
            result.proofs = state.proofs().clone();
            result.eliminated_guards = state.eliminated_guards().to_vec();
            result.stats.full_unrolled_loops = unroll.full;
            result.stats.partial_unrolled_loops_factor_2 = unroll.factor_2;
            result.stats.partial_unrolled_loops_factor_4 = unroll.factor_4;
            result.stats.rejected_unroll_candidates = unroll.rejected;
            result.stats.staged_native_unroll_candidates = unroll.staged_native;
            let combined_unroll_slp = unroll.slp;
            result.analysis_fallbacks.extend(unroll.fallbacks);
            if !record_current_pass(
                &module,
                "loop-optimization-frontier",
                unroll.changed,
                &mut result,
                GENERATION,
            ) {
                result.module = module;
                return result;
            }
            let slp = match run_residual_slp_staging(&mut state, &mut result.audit) {
                Ok(slp) => slp,
                Err(error) => {
                    result.errors.push(error);
                    result.module = module;
                    return result;
                }
            };
            module = state.module().clone();
            result.contract_facts = state.contract_facts().cloned();
            result.proofs = state.proofs().clone();
            result.eliminated_guards = state.eliminated_guards().to_vec();
            result.stats.slp_packs = loop_slp_winners
                .saturating_add(combined_unroll_slp)
                .saturating_add(slp.accepted);
            result.stats.staged_native_slp_candidates = loop_slp_nonwinners;
            result.stats.rejected_slp_candidates = loop_slp_rejected.saturating_add(slp.rejected);
            result.analysis_fallbacks.extend(slp.fallbacks);
            if !record_current_pass(
                &module,
                "residual-slp-frontier",
                slp.changed,
                &mut result,
                GENERATION,
            ) {
                result.module = module;
                return result;
            }
        }
    }

    let protected = result.proofs.instruction_dependencies();
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

#[derive(Debug, Default)]
struct SpecializationFrontierResult {
    changed: bool,
    accepted: u32,
    rejected: u32,
    reused: u32,
    limits: u32,
    fallbacks: Vec<KirAnalysisFallback>,
}

fn specialization_frontier_result(
    discovery: &super::SpecializationDiscovery,
) -> SpecializationFrontierResult {
    let mut result = SpecializationFrontierResult::default();
    result.fallbacks.extend(
        discovery
            .fallbacks
            .iter()
            .filter(|fallback| fallback.reason == "sanitizer-mode-disabled")
            .map(|fallback| KirAnalysisFallback {
                function: fallback.function,
                pass: "specialization".to_string(),
                reason: fallback.reason.clone(),
            }),
    );
    result
}

fn run_specialization_frontier(
    state: &mut KirVerifiedProgramState,
    audit: &mut KirOptimizationAuditState,
    discovery: super::SpecializationDiscovery,
) -> Result<SpecializationFrontierResult, String> {
    let mut result = specialization_frontier_result(&discovery);
    for candidate in discovery.candidates {
        let callee = state
            .module()
            .functions
            .iter()
            .find(|function| function.id == candidate.callee)
            .ok_or_else(|| "specialization candidate callee disappeared".to_string())?;
        let clone_name = kir_passes::specialization_clone_name(
            &callee.name,
            callee.id,
            &candidate.fact_set_digest,
        );
        let existing = state
            .module()
            .functions
            .iter()
            .any(|function| function.name == clone_name);
        let original_prefix = format!("__ck_spec_{}_f{}_", callee.name, callee.id.index());
        let original_clone_count = state
            .module()
            .functions
            .iter()
            .filter(|function| {
                is_specialization_clone(&function.name)
                    && function.name.starts_with(&original_prefix)
            })
            .count();
        let module_clone_count = state
            .module()
            .functions
            .iter()
            .filter(|function| is_specialization_clone(&function.name))
            .count();
        if !existing && (original_clone_count >= 3 || module_clone_count >= 24) {
            let function_units = kir_function_units(callee);
            let facts = u32::try_from(candidate.facts.len()).unwrap_or(u32::MAX);
            let charge = CandidateBudgetCharge {
                functions: vec![candidate.caller, candidate.callee],
                proposer_units: 8_u32
                    .saturating_add(function_units)
                    .saturating_add(facts.saturating_mul(4)),
                checker_units: 16_u32
                    .saturating_add(function_units)
                    .saturating_add(facts.saturating_mul(6)),
            };
            audit.record_noncommitting_attempt(
                candidate.key,
                charge,
                CandidateDisposition::Rejected,
                "specialization-clone-limit",
            )?;
            result.rejected = result.rejected.saturating_add(1);
            result.limits = result.limits.saturating_add(1);
            result.fallbacks.push(KirAnalysisFallback {
                function: candidate.caller,
                pass: "specialization".to_string(),
                reason: "specialization-clone-limit".to_string(),
            });
            continue;
        }
        let clone_ordinal = u8::try_from(original_clone_count.min(2)).unwrap_or(2);
        let materialized =
            match kir_passes::materialize_specialization_trial(state, &candidate, clone_ordinal) {
                Ok(materialized) => materialized,
                Err(error)
                    if error == "trusted-contract-specialization-requires-cloned-fact-scope" =>
                {
                    let function_units = kir_function_units(callee);
                    let facts = u32::try_from(candidate.facts.len()).unwrap_or(u32::MAX);
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        CandidateBudgetCharge {
                            functions: vec![candidate.caller, candidate.callee],
                            proposer_units: 8_u32
                                .saturating_add(function_units)
                                .saturating_add(facts.saturating_mul(4)),
                            checker_units: 16_u32
                                .saturating_add(function_units)
                                .saturating_add(facts.saturating_mul(6)),
                        },
                        CandidateDisposition::Rejected,
                        error.clone(),
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.caller,
                        pass: "specialization".to_string(),
                        reason: error,
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
        let reused = materialized.plan.reused;
        let plan = materialized.plan.clone();
        let clone = plan.clone;
        let original = plan.callee;
        let charge = materialized.charge.clone();
        let proposed = materialized.trial;
        let outcome = execute_verified_transaction_with_disposition(
            state,
            audit,
            candidate.key,
            charge.clone(),
            if reused {
                CandidateDisposition::Reused
            } else {
                CandidateDisposition::Accepted
            },
            move |trial| {
                *trial = proposed;
                Ok(())
            },
            |pre, trial| check_specialization_plan_independently(pre, trial, &plan, &charge),
        );
        match outcome {
            TransactionOutcome::Committed => {
                result.changed = true;
                if reused {
                    result.reused = result.reused.saturating_add(1);
                } else {
                    audit.register_clone_budget(clone, original)?;
                    result.accepted = result.accepted.saturating_add(1);
                }
            }
            TransactionOutcome::Rejected | TransactionOutcome::BudgetExhausted => {
                result.rejected = result.rejected.saturating_add(1);
            }
            TransactionOutcome::CompilerError(error) => return Err(error),
        }
    }
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.pass.as_str(), left.reason.as_str()).cmp(&(
            right.function,
            right.pass.as_str(),
            right.reason.as_str(),
        ))
    });
    result.fallbacks.dedup();
    Ok(result)
}

#[derive(Debug, Default)]
struct UnrollFrontierResult {
    changed: bool,
    full: u32,
    factor_2: u32,
    factor_4: u32,
    rejected: u32,
    staged_native: u32,
    slp: u32,
    fallbacks: Vec<KirAnalysisFallback>,
}

#[derive(Debug, Default)]
struct VectorFrontierResult {
    changed: bool,
    accepted: u32,
    rejected: u32,
    scalar_fallbacks: u32,
    slp_nonwinners: u32,
    slp_winners: u32,
    slp_rejected: u32,
    fallbacks: Vec<KirAnalysisFallback>,
    explanations: Vec<KirVectorOptimizationExplanation>,
}

#[derive(Debug, Clone)]
struct UnrollFrontierAlternative {
    candidate: super::UnrollCandidate,
    unroll: kir_passes::MaterializedUnroll,
    slp: Option<kir_passes::MaterializedSlp>,
    key: super::CandidateKey,
    cost: super::KirCostEstimate,
    charge: CandidateBudgetCharge,
    final_units: u32,
}

fn vector_plan_explanation(
    candidate: super::CandidateKey,
    plan: &super::VectorizationPlan,
    disposition: CandidateDisposition,
    reason: &str,
) -> KirVectorOptimizationExplanation {
    let predicates = plan
        .predicates
        .iter()
        .map(|predicate| match predicate {
            super::VectorPredicate::TripThreshold {
                trip_count,
                minimum,
                ..
            } => format!("trip-threshold:v{}>={minimum}", trip_count.index()),
            super::VectorPredicate::Divisibility { value, divisor, .. } => {
                format!("divisible:v{}%{divisor}", value.index())
            }
            super::VectorPredicate::AddressNonOverlap {
                left, right, bytes, ..
            } => format!(
                "address-nonoverlap:r{}:r{}:count-v{}",
                left.index(),
                right.index(),
                bytes.index()
            ),
            super::VectorPredicate::PowerOfTwoAlignment {
                value, alignment, ..
            } => format!("alignment:v{}:{alignment}", value.index()),
        })
        .collect();
    KirVectorOptimizationExplanation {
        candidate,
        disposition,
        vf: plan.vf,
        uf: plan.uf,
        predicates,
        cost: plan.cost,
        growth: plan.growth,
        proofs: plan.proofs.clone(),
        reason: reason.to_string(),
    }
}

fn loop_frontier_scalar_body_cost(
    candidate: &super::VectorizationCandidate,
) -> Result<u32, String> {
    let priced_trip = candidate.minimum_trip.saturating_add(
        u32::from(candidate.vf)
            .saturating_mul(u32::from(candidate.uf))
            .saturating_sub(1),
    );
    if priced_trip == 0 || !candidate.predicted_cost.scalar.is_multiple_of(priced_trip) {
        return Err("loop frontier scalar cost is not a closed iteration cost".to_string());
    }
    Ok(candidate.predicted_cost.scalar / priced_trip)
}

fn loop_frontier_iterations(
    state: &KirVerifiedProgramState,
    candidate: &super::VectorizationCandidate,
    pgo: Option<&super::CkPgoOptimizerPlan>,
) -> u32 {
    let minimum = candidate.minimum_trip;
    state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .map(super::analyze_canonical_loops_for_discovery)
        .and_then(|analysis| {
            analysis
                .loops
                .into_iter()
                .find(|descriptor| descriptor.id == candidate.loop_id)
        })
        .and_then(|descriptor| match descriptor.trip_count {
            super::LoopTripCount::Exact { iterations } => u32::try_from(iterations).ok(),
            _ => None,
        })
        .unwrap_or(minimum)
        .max(
            pgo.and_then(|profile| profile.loop_minimum_trip(candidate.function, candidate.header))
                .unwrap_or(minimum),
        )
        .max(minimum)
}

fn vector_loop_scope_cost(
    plan: &super::VectorizationPlan,
    candidate: &super::VectorizationCandidate,
    scalar_body_cost: u32,
    iterations: u32,
) -> u64 {
    let chunk_width = u32::from(plan.vf).saturating_mul(u32::from(plan.uf));
    let chunks = iterations / chunk_width;
    let tail = iterations % chunk_width;
    let priced_chunks = candidate.minimum_trip / chunk_width;
    let vector_chunk_cost = if priced_chunks == 0 {
        u32::MAX
    } else {
        plan.cost.transformed_body / priced_chunks
    };
    let priced_tail = chunk_width.saturating_sub(1);
    let epilogue_entry_cost = plan
        .cost
        .epilogue
        .saturating_sub(scalar_body_cost.saturating_mul(priced_tail));
    u64::from(plan.cost.predicates)
        .saturating_add(u64::from(vector_chunk_cost).saturating_mul(u64::from(chunks)))
        .saturating_add(u64::from(scalar_body_cost).saturating_mul(u64::from(tail)))
        .saturating_add(u64::from(epilogue_entry_cost) * u64::from(u8::from(tail != 0)))
}

fn slp_loop_scope_cost(plan: &super::SlpPlan, scalar_body_cost: u32, iterations: u32) -> u64 {
    let transformed_body = scalar_body_cost
        .saturating_sub(plan.cost.scalar)
        .saturating_add(plan.cost.total);
    u64::from(transformed_body).saturating_mul(u64::from(iterations))
}

fn run_native_vector_frontier(
    state: &mut KirVerifiedProgramState,
    audit: &mut KirOptimizationAuditState,
    pgo: Option<&super::CkPgoOptimizerPlan>,
) -> Result<VectorFrontierResult, String> {
    let mut result = VectorFrontierResult::default();
    if !matches!(
        state.module().config.consumer,
        crate::KirConsumer::NativeLibrary | crate::KirConsumer::NativeExecutable
    ) {
        return Ok(result);
    }
    let mut processed = std::collections::BTreeSet::new();
    loop {
        let discovery = discover_vectorization_candidates(state);
        result
            .fallbacks
            .extend(
                discovery
                    .fallbacks
                    .into_iter()
                    .map(|fallback| KirAnalysisFallback {
                        function: fallback.function,
                        pass: "loop-simd".to_string(),
                        reason: fallback.reason,
                    }),
            );
        let Some(loop_identity) = discovery
            .candidates
            .iter()
            .map(|candidate| (candidate.function, candidate.header))
            .find(|identity| !processed.contains(identity))
        else {
            break;
        };
        processed.insert(loop_identity);
        let loop_candidates = discovery
            .candidates
            .into_iter()
            .filter(|candidate| (candidate.function, candidate.header) == loop_identity)
            .collect::<Vec<_>>();
        let profile_scalar_reason = pgo.and_then(|profile| {
            let below_every_vector_threshold = |maximum: u32| {
                loop_candidates
                    .iter()
                    .all(|candidate| maximum < candidate.minimum_trip)
            };
            if profile
                .loop_maximum_trip(loop_identity.0, loop_identity.1)
                .is_some_and(below_every_vector_threshold)
            {
                return Some("profile-short-trip-retains-scalar");
            }
            loop_candidates.first().and_then(|candidate| {
                profile
                    .slice_length_maximum(state.module(), candidate.function, candidate.bound)
                    .filter(|maximum| below_every_vector_threshold(*maximum))
                    .map(|_| "profile-short-slice-retains-scalar")
            })
        });
        if let Some(profile_scalar_reason) = profile_scalar_reason {
            for candidate in loop_candidates {
                let charge = CandidateBudgetCharge::single(
                    candidate.function,
                    candidate.predicted_cost.scalar.saturating_add(8),
                    candidate
                        .predicted_cost
                        .scalar
                        .saturating_mul(2)
                        .saturating_add(16),
                );
                audit.record_noncommitting_attempt(
                    candidate.key,
                    charge,
                    CandidateDisposition::NonWinner,
                    profile_scalar_reason,
                )?;
            }
            result.scalar_fallbacks = result.scalar_fallbacks.saturating_add(1);
            result.fallbacks.push(KirAnalysisFallback {
                function: loop_identity.0,
                pass: "loop-simd".to_string(),
                reason: profile_scalar_reason.to_string(),
            });
            continue;
        }
        let mut vector_alternatives = Vec::new();
        for candidate in loop_candidates {
            let prepared = match kir_passes::materialize_vectorization_trial(state, &candidate) {
                Ok(prepared) => prepared,
                Err(reason) => {
                    let charge = CandidateBudgetCharge::single(
                        candidate.function,
                        candidate.predicted_cost.scalar.saturating_add(8),
                        candidate
                            .predicted_cost
                            .scalar
                            .saturating_mul(2)
                            .saturating_add(16),
                    );
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        charge,
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.function,
                        pass: "loop-simd".to_string(),
                        reason,
                    });
                    continue;
                }
            };
            match check_vectorization_trial_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ) {
                Ok(()) => {
                    let scalar_body_cost = loop_frontier_scalar_body_cost(&candidate)?;
                    let iterations = loop_frontier_iterations(state, &candidate, pgo);
                    vector_alternatives.push((
                        0,
                        candidate,
                        prepared,
                        scalar_body_cost,
                        iterations,
                    ));
                }
                Err(super::TransactionCheckError::Reject(reason)) => {
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        prepared.charge,
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.function,
                        pass: "loop-simd".to_string(),
                        reason,
                    });
                }
                Err(super::TransactionCheckError::Compiler(error)) => return Err(error),
            }
        }
        let common_iterations = vector_alternatives
            .iter()
            .map(|(_, _, _, _, iterations)| *iterations)
            .max()
            .unwrap_or(0);
        for (scope_cost, candidate, prepared, scalar_body_cost, iterations) in
            &mut vector_alternatives
        {
            *iterations = common_iterations;
            *scope_cost = vector_loop_scope_cost(
                &prepared.plan,
                candidate,
                *scalar_body_cost,
                common_iterations,
            );
        }
        vector_alternatives.sort_by(
            |(left_cost, left_candidate, left, _, _),
             (right_cost, right_candidate, right, _, _)| {
                (
                    *left_cost,
                    left.plan.growth.transformed_units,
                    left.plan.vf,
                    left.plan.uf,
                    &left_candidate.key,
                )
                    .cmp(&(
                        *right_cost,
                        right.plan.growth.transformed_units,
                        right.plan.vf,
                        right.plan.uf,
                        &right_candidate.key,
                    ))
            },
        );
        let Some((_, representative, _, scalar_body_cost, iterations)) =
            vector_alternatives.first()
        else {
            continue;
        };
        let protected = state.proofs().instruction_dependencies();
        let loop_slp = state
            .module()
            .functions
            .iter()
            .find(|function| function.id == representative.function)
            .into_iter()
            .flat_map(|function| discover_slp_candidates(function, &protected).candidates)
            .filter(|slp| slp.block == representative.body)
            .collect::<Vec<_>>();
        let mut slp_alternatives = Vec::new();
        for slp_candidate in loop_slp {
            let slp = match kir_passes::materialize_slp_trial(state, &slp_candidate) {
                Ok(slp) => slp,
                Err(reason) => {
                    audit.record_noncommitting_attempt(
                        slp_candidate.key.clone(),
                        slp_fallback_charge(&slp_candidate),
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.slp_rejected = result.slp_rejected.saturating_add(1);
                    continue;
                }
            };
            match check_slp_plan_independently(state, &slp.trial, &slp.plan, &slp.charge) {
                Ok(()) => slp_alternatives.push((
                    slp_loop_scope_cost(&slp.plan, *scalar_body_cost, *iterations),
                    slp_candidate,
                    slp,
                )),
                Err(super::TransactionCheckError::Reject(reason)) => {
                    audit.record_noncommitting_attempt(
                        slp_candidate.key,
                        slp.charge,
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.slp_rejected = result.slp_rejected.saturating_add(1);
                }
                Err(super::TransactionCheckError::Compiler(error)) => return Err(error),
            }
        }
        slp_alternatives.sort_by(
            |(left_cost, left_candidate, left), (right_cost, right_candidate, right)| {
                (
                    *left_cost,
                    left.plan.growth.transformed_units,
                    1_u8,
                    left_candidate.lanes,
                    1_u8,
                    &left_candidate.key,
                )
                    .cmp(&(
                        *right_cost,
                        right.plan.growth.transformed_units,
                        1_u8,
                        right_candidate.lanes,
                        1_u8,
                        &right_candidate.key,
                    ))
            },
        );
        let (vector_scope_cost, vector_candidate, vector_prepared, _, _) = vector_alternatives
            .first()
            .expect("non-empty vector frontier");
        let slp_wins = slp_alternatives
            .first()
            .is_some_and(|(slp_cost, slp_candidate, slp)| {
                (
                    *slp_cost,
                    slp.plan.growth.transformed_units,
                    1_u8,
                    slp_candidate.lanes,
                    1_u8,
                    &slp_candidate.key,
                ) < (
                    *vector_scope_cost,
                    vector_prepared.plan.growth.transformed_units,
                    0_u8,
                    vector_prepared.plan.vf,
                    vector_prepared.plan.uf,
                    &vector_candidate.key,
                )
            });
        if slp_wins {
            let (_, winner_candidate, winner) = slp_alternatives.remove(0);
            for (_, nonwinner_candidate, nonwinner) in slp_alternatives {
                audit.record_noncommitting_attempt(
                    nonwinner_candidate.key,
                    nonwinner.charge,
                    CandidateDisposition::NonWinner,
                    "higher-cost-loop-alternative",
                )?;
                result.slp_nonwinners = result.slp_nonwinners.saturating_add(1);
            }
            for (_, candidate, prepared, _, _) in vector_alternatives {
                audit.record_noncommitting_attempt(
                    candidate.key.clone(),
                    prepared.charge.clone(),
                    CandidateDisposition::NonWinner,
                    "higher-cost-loop-alternative",
                )?;
                result.explanations.push(vector_plan_explanation(
                    candidate.key,
                    &prepared.plan,
                    CandidateDisposition::NonWinner,
                    "higher-cost-loop-alternative",
                ));
            }
            let plan = winner.plan.clone();
            let charge = winner.charge.clone();
            let proposed = winner.trial;
            let outcome = execute_verified_transaction_with_disposition(
                state,
                audit,
                winner_candidate.key,
                charge.clone(),
                CandidateDisposition::Accepted,
                move |trial| {
                    *trial = proposed;
                    Ok(())
                },
                |pre, trial| check_slp_plan_independently(pre, trial, &plan, &charge),
            );
            match outcome {
                TransactionOutcome::Committed => {
                    result.changed = true;
                    result.slp_winners = result.slp_winners.saturating_add(1);
                }
                TransactionOutcome::Rejected | TransactionOutcome::BudgetExhausted => {
                    result.slp_rejected = result.slp_rejected.saturating_add(1);
                }
                TransactionOutcome::CompilerError(error) => return Err(error),
            }
            continue;
        }
        for (_, slp_candidate, slp) in slp_alternatives {
            audit.record_noncommitting_attempt(
                slp_candidate.key,
                slp.charge,
                CandidateDisposition::NonWinner,
                "higher-cost-loop-alternative",
            )?;
            result.slp_nonwinners = result.slp_nonwinners.saturating_add(1);
        }
        let (_, candidate, prepared, _, _) = vector_alternatives.remove(0);
        for (_, nonwinner_candidate, nonwinner, _, _) in vector_alternatives {
            audit.record_noncommitting_attempt(
                nonwinner_candidate.key.clone(),
                nonwinner.charge,
                CandidateDisposition::NonWinner,
                "higher-cost-loop-alternative",
            )?;
            result.explanations.push(vector_plan_explanation(
                nonwinner_candidate.key,
                &nonwinner.plan,
                CandidateDisposition::NonWinner,
                "higher-cost-loop-alternative",
            ));
        }
        let plan = prepared.plan.clone();
        let charge = prepared.charge.clone();
        let proposed = prepared.trial;
        let candidate_key = candidate.key.clone();
        let outcome = execute_verified_transaction_with_disposition(
            state,
            audit,
            candidate.key,
            charge.clone(),
            CandidateDisposition::Accepted,
            move |trial| {
                *trial = proposed;
                Ok(())
            },
            |pre, trial| check_vectorization_trial_independently(pre, trial, &plan, &charge),
        );
        match outcome {
            TransactionOutcome::Committed => {
                result.changed = true;
                result.accepted = result.accepted.saturating_add(1);
                result.scalar_fallbacks = result.scalar_fallbacks.saturating_add(1);
                result.explanations.push(vector_plan_explanation(
                    candidate_key,
                    &plan,
                    CandidateDisposition::Accepted,
                    "accepted",
                ));
            }
            TransactionOutcome::Rejected => {
                result.rejected = result.rejected.saturating_add(1);
                result.explanations.push(vector_plan_explanation(
                    candidate_key,
                    &plan,
                    CandidateDisposition::Rejected,
                    "independent-check-rejected",
                ));
            }
            TransactionOutcome::BudgetExhausted => {
                result.rejected = result.rejected.saturating_add(1);
                result.explanations.push(vector_plan_explanation(
                    candidate_key,
                    &plan,
                    CandidateDisposition::BudgetExhausted,
                    "budget-exhausted",
                ));
            }
            TransactionOutcome::CompilerError(error) => return Err(error),
        }
    }
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.pass.as_str(), left.reason.as_str()).cmp(&(
            right.function,
            right.pass.as_str(),
            right.reason.as_str(),
        ))
    });
    result.fallbacks.dedup();
    Ok(result)
}

fn run_unroll_frontier(
    state: &mut KirVerifiedProgramState,
    audit: &mut KirOptimizationAuditState,
) -> Result<UnrollFrontierResult, String> {
    let mut result = UnrollFrontierResult::default();
    let mut processed = std::collections::BTreeSet::new();
    loop {
        let mut candidates = Vec::new();
        for function in &state.module().functions {
            let loops = super::analyze_canonical_loops_for_discovery(function);
            let discovery = discover_unroll_candidates(function, &loops.loops);
            result
                .fallbacks
                .extend(
                    discovery
                        .fallbacks
                        .into_iter()
                        .map(|fallback| KirAnalysisFallback {
                            function: fallback.function,
                            pass: "unroll".to_string(),
                            reason: fallback.reason,
                        }),
                );
            candidates.extend(discovery.candidates);
        }
        candidates.sort_by(|left, right| left.key.cmp(&right.key));
        let Some(next) = candidates
            .iter()
            .find(|candidate| !processed.contains(&(candidate.function, candidate.header)))
            .map(|candidate| (candidate.function, candidate.header, candidate.loop_id))
        else {
            break;
        };
        processed.insert((next.0, next.1));
        let group = candidates
            .into_iter()
            .filter(|candidate| {
                candidate.function == next.0
                    && candidate.header == next.1
                    && candidate.loop_id == next.2
            })
            .collect::<Vec<_>>();
        let mut accepted = Vec::<UnrollFrontierAlternative>::new();
        for candidate in group {
            let prepared = match kir_passes::materialize_unroll_trial(state, &candidate) {
                Ok(prepared) => prepared,
                Err(reason)
                    if matches!(
                        reason.as_str(),
                        "unroll-certificate-dependency"
                            | "partial-unroll-requires-strict-unit-induction"
                    ) =>
                {
                    let charge = unroll_fallback_charge(&candidate);
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        charge,
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.function,
                        pass: "unroll".to_string(),
                        reason,
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            match super::check_unroll_structure_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ) {
                Ok(()) => {}
                Err(super::TransactionCheckError::Reject(reason)) => {
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        prepared.charge,
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.function,
                        pass: "unroll".to_string(),
                        reason,
                    });
                    continue;
                }
                Err(super::TransactionCheckError::Compiler(error)) => return Err(error),
            }
            match check_unroll_plan_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ) {
                Ok(()) => {
                    let final_units = prepared
                        .trial
                        .module()
                        .functions
                        .iter()
                        .find(|function| function.id == candidate.function)
                        .map(kir_function_units)
                        .ok_or_else(|| "unroll trial function disappeared".to_string())?;
                    accepted.push(UnrollFrontierAlternative {
                        candidate: candidate.clone(),
                        unroll: prepared.clone(),
                        slp: None,
                        key: candidate.key.clone(),
                        cost: prepared.plan.cost,
                        charge: prepared.charge.clone(),
                        final_units,
                    });
                }
                Err(super::TransactionCheckError::Reject(reason)) => {
                    audit.record_noncommitting_attempt(
                        candidate.key.clone(),
                        prepared.charge.clone(),
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                }
                Err(super::TransactionCheckError::Compiler(error)) => return Err(error),
            }

            let protected = prepared.trial.proofs().instruction_dependencies();
            let function = prepared
                .trial
                .module()
                .functions
                .iter()
                .find(|function| function.id == candidate.function)
                .ok_or_else(|| "unroll trial function disappeared before SLP".to_string())?;
            let mut slp_candidates = discover_slp_candidates(function, &protected).candidates;
            slp_candidates.sort_by(|left, right| left.key.cmp(&right.key));
            let mut seen_lanes = std::collections::BTreeSet::new();
            for slp_candidate in slp_candidates
                .into_iter()
                .filter(|slp| seen_lanes.insert(slp.lanes))
            {
                let kind = if candidate.full {
                    super::LoopCandidateKind::FullUnroll
                } else {
                    super::LoopCandidateKind::PartialUnroll
                };
                let combined_key = super::CandidateKey::LoopFrontier {
                    function: candidate.function,
                    loop_id: candidate.loop_id,
                    kind,
                    variant: super::LoopCandidateVariant::Slp,
                    vf: slp_candidate.lanes,
                    uf: candidate.factor,
                };
                let slp = match kir_passes::materialize_slp_trial(&prepared.trial, &slp_candidate) {
                    Ok(slp) => slp,
                    Err(reason) => {
                        audit.record_noncommitting_attempt(
                            combined_key,
                            slp_fallback_charge(&slp_candidate),
                            CandidateDisposition::Rejected,
                            &reason,
                        )?;
                        result.rejected = result.rejected.saturating_add(1);
                        continue;
                    }
                };
                let combined_cost = super::combined_unroll_slp_cost(&prepared.plan, &slp.plan);
                let combined_charge = CandidateBudgetCharge::single(
                    candidate.function,
                    prepared
                        .charge
                        .proposer_units
                        .saturating_add(slp.charge.proposer_units),
                    prepared
                        .charge
                        .checker_units
                        .saturating_add(slp.charge.checker_units),
                );
                match super::check_unroll_slp_trial_independently(
                    state,
                    &prepared.trial,
                    &slp.trial,
                    &prepared.plan,
                    &prepared.charge,
                    &slp.plan,
                    &slp.charge,
                    combined_cost,
                    &combined_charge,
                ) {
                    Ok(()) => accepted.push(UnrollFrontierAlternative {
                        candidate: candidate.clone(),
                        unroll: prepared.clone(),
                        slp: Some(slp.clone()),
                        key: combined_key,
                        cost: combined_cost,
                        charge: combined_charge,
                        final_units: slp.plan.growth.transformed_units,
                    }),
                    Err(super::TransactionCheckError::Reject(reason)) => {
                        audit.record_noncommitting_attempt(
                            combined_key,
                            combined_charge,
                            CandidateDisposition::Rejected,
                            &reason,
                        )?;
                        result.rejected = result.rejected.saturating_add(1);
                    }
                    Err(super::TransactionCheckError::Compiler(error)) => return Err(error),
                }
            }
        }
        accepted.sort_by(|left, right| {
            (
                left.cost.total,
                left.final_units,
                left.candidate.factor,
                &left.key,
            )
                .cmp(&(
                    right.cost.total,
                    right.final_units,
                    right.candidate.factor,
                    &right.key,
                ))
        });
        let Some(winner) = accepted.first().cloned() else {
            continue;
        };
        for alternative in accepted.into_iter().skip(1) {
            audit.record_noncommitting_attempt(
                alternative.key,
                alternative.charge,
                CandidateDisposition::NonWinner,
                "higher-cost-unroll-alternative",
            )?;
        }
        let combined = winner.slp.is_some();
        let outcome = if let Some(slp) = winner.slp.clone() {
            let intermediate = winner.unroll.trial.clone();
            let final_state = slp.trial;
            let unroll_plan = winner.unroll.plan.clone();
            let unroll_charge = winner.unroll.charge.clone();
            let slp_plan = slp.plan;
            let slp_charge = slp.charge;
            let combined_cost = winner.cost;
            let combined_charge = winner.charge.clone();
            execute_verified_transaction_with_disposition(
                state,
                audit,
                winner.key.clone(),
                combined_charge.clone(),
                CandidateDisposition::Accepted,
                move |trial| {
                    *trial = final_state;
                    Ok(())
                },
                move |pre, trial| {
                    super::check_unroll_slp_trial_independently(
                        pre,
                        &intermediate,
                        trial,
                        &unroll_plan,
                        &unroll_charge,
                        &slp_plan,
                        &slp_charge,
                        combined_cost,
                        &combined_charge,
                    )
                },
            )
        } else {
            let plan = winner.unroll.plan.clone();
            let charge = winner.charge.clone();
            let proposed = winner.unroll.trial.clone();
            execute_verified_transaction_with_disposition(
                state,
                audit,
                winner.key.clone(),
                charge.clone(),
                CandidateDisposition::Accepted,
                move |trial| {
                    *trial = proposed;
                    Ok(())
                },
                |pre, trial| check_unroll_plan_independently(pre, trial, &plan, &charge),
            )
        };
        match outcome {
            TransactionOutcome::Committed => {
                result.changed = true;
                if combined {
                    result.slp = result.slp.saturating_add(1);
                }
                if winner.candidate.full {
                    result.full = result.full.saturating_add(1);
                } else if winner.candidate.factor == 2 {
                    result.factor_2 = result.factor_2.saturating_add(1);
                } else {
                    result.factor_4 = result.factor_4.saturating_add(1);
                }
            }
            TransactionOutcome::Rejected | TransactionOutcome::BudgetExhausted => {
                result.rejected = result.rejected.saturating_add(1);
            }
            TransactionOutcome::CompilerError(error) => return Err(error),
        }
    }
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.pass.as_str(), left.reason.as_str()).cmp(&(
            right.function,
            right.pass.as_str(),
            right.reason.as_str(),
        ))
    });
    result.fallbacks.dedup();
    Ok(result)
}

fn cached_frontier_loop_analysis<'a>(
    function: &KirFunction,
    index: usize,
    cached: Option<&'a [super::CanonicalLoopAnalysis]>,
) -> Option<&'a super::CanonicalLoopAnalysis> {
    cached?
        .get(index)
        .filter(|analysis| analysis.function == Some(function.id))
}

fn no_op_native_frontiers(
    module: &KirModule,
    cached_loop_analyses: Option<&[super::CanonicalLoopAnalysis]>,
) -> Option<Vec<KirAnalysisFallback>> {
    let native = matches!(
        module.config.consumer,
        crate::KirConsumer::NativeLibrary | crate::KirConsumer::NativeExecutable
    );
    let vector_and_slp_are_noops = !native
        || (module.config.sanitizer_mode == crate::KirSanitizerMode::Disabled
            && module.config.overflow_mode == crate::KirOverflowMode::Unchecked
            && module.config.bounds_mode == crate::KirBoundsMode::Unchecked
            && !module.profile.vector_operations_enabled());
    if !vector_and_slp_are_noops {
        return None;
    }

    let mut fallbacks = Vec::new();
    for (index, function) in module.functions.iter().enumerate() {
        let recomputed;
        let loops = if let Some(cached) =
            cached_frontier_loop_analysis(function, index, cached_loop_analyses)
        {
            cached
        } else {
            recomputed = super::analyze_canonical_loops_for_discovery(function);
            &recomputed
        };
        let discovery = discover_unroll_candidates(function, &loops.loops);
        if !discovery.candidates.is_empty() {
            return None;
        }
        fallbacks.extend(
            discovery
                .fallbacks
                .into_iter()
                .map(|fallback| KirAnalysisFallback {
                    function: fallback.function,
                    pass: "unroll".to_string(),
                    reason: fallback.reason,
                }),
        );
    }
    fallbacks.sort_by(|left, right| {
        (left.function, left.pass.as_str(), left.reason.as_str()).cmp(&(
            right.function,
            right.pass.as_str(),
            right.reason.as_str(),
        ))
    });
    fallbacks.dedup();
    Some(fallbacks)
}

fn unroll_fallback_charge(candidate: &super::UnrollCandidate) -> CandidateBudgetCharge {
    let emitted_iterations = if candidate.full {
        candidate.trip_count
    } else {
        u32::from(candidate.factor).saturating_add(u32::from(candidate.remainder))
    };
    let mappings = emitted_iterations.saturating_mul(candidate.body_units);
    CandidateBudgetCharge::single(
        candidate.function,
        8_u32
            .saturating_add(candidate.body_units.saturating_add(2))
            .saturating_add(mappings),
        16_u32
            .saturating_add(candidate.body_units.saturating_add(2))
            .saturating_add(mappings.saturating_mul(2)),
    )
}

#[derive(Debug, Default)]
struct SlpStagingResult {
    changed: bool,
    accepted: u32,
    rejected: u32,
    fallbacks: Vec<KirAnalysisFallback>,
}

fn run_residual_slp_staging(
    state: &mut KirVerifiedProgramState,
    audit: &mut KirOptimizationAuditState,
) -> Result<SlpStagingResult, String> {
    let mut result = SlpStagingResult::default();
    if !matches!(
        state.module().config.consumer,
        crate::KirConsumer::NativeLibrary | crate::KirConsumer::NativeExecutable
    ) {
        return Ok(result);
    }
    let mut processed = std::collections::BTreeSet::new();
    loop {
        let protected = state.proofs().instruction_dependencies();
        let mut candidates = state
            .module()
            .functions
            .iter()
            .flat_map(|function| {
                let mut committed_loop_blocks = function
                    .vector_regions
                    .iter()
                    .flat_map(|region| region.blocks.iter().copied())
                    .collect::<std::collections::BTreeSet<_>>();
                if !function.vector_regions.is_empty() {
                    committed_loop_blocks.extend(
                        super::analyze_canonical_loops_for_discovery(function)
                            .loops
                            .into_iter()
                            .flat_map(|descriptor| descriptor.blocks),
                    );
                }
                discover_slp_candidates(function, &protected)
                    .candidates
                    .into_iter()
                    .filter(move |candidate| !committed_loop_blocks.contains(&candidate.block))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.key.cmp(&right.key));
        let Some(first) = candidates
            .iter()
            .find(|candidate| !processed.contains(&candidate.key))
            .cloned()
        else {
            break;
        };
        let group = candidates
            .into_iter()
            .filter(|candidate| {
                candidate.function == first.function
                    && candidate.block == first.block
                    && candidate.root == first.root
                    && !processed.contains(&candidate.key)
            })
            .collect::<Vec<_>>();
        for candidate in &group {
            processed.insert(candidate.key.clone());
        }
        let mut accepted = Vec::new();
        for candidate in group {
            let prepared = match kir_passes::materialize_slp_trial(state, &candidate) {
                Ok(prepared) => prepared,
                Err(reason) if reason == "SLP target operation is unavailable" => {
                    let charge = slp_fallback_charge(&candidate);
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        charge,
                        CandidateDisposition::Rejected,
                        "slp-target-operation-unavailable",
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.function,
                        pass: "residual-slp".to_string(),
                        reason: "slp-target-operation-unavailable".to_string(),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            match check_slp_plan_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ) {
                Ok(()) => accepted.push((candidate, prepared)),
                Err(super::TransactionCheckError::Reject(reason)) => {
                    audit.record_noncommitting_attempt(
                        candidate.key,
                        prepared.charge,
                        CandidateDisposition::Rejected,
                        &reason,
                    )?;
                    result.rejected = result.rejected.saturating_add(1);
                    result.fallbacks.push(KirAnalysisFallback {
                        function: candidate.function,
                        pass: "residual-slp".to_string(),
                        reason,
                    });
                }
                Err(super::TransactionCheckError::Compiler(error)) => return Err(error),
            }
        }
        accepted.sort_by(|(left_candidate, left), (right_candidate, right)| {
            let left_savings = left.plan.cost.scalar.saturating_sub(left.plan.cost.total);
            let right_savings = right.plan.cost.scalar.saturating_sub(right.plan.cost.total);
            right_savings
                .cmp(&left_savings)
                .then_with(|| left.plan.cost.total.cmp(&right.plan.cost.total))
                .then_with(|| {
                    left.plan
                        .growth
                        .transformed_units
                        .cmp(&right.plan.growth.transformed_units)
                })
                .then_with(|| left_candidate.key.cmp(&right_candidate.key))
        });
        let Some((candidate, prepared)) = accepted.first().cloned() else {
            continue;
        };
        for (alternative, prepared) in accepted.into_iter().skip(1) {
            audit.record_noncommitting_attempt(
                alternative.key,
                prepared.charge,
                CandidateDisposition::NonWinner,
                "higher-cost-slp-alternative",
            )?;
        }
        let plan = prepared.plan.clone();
        let charge = prepared.charge.clone();
        let proposed = prepared.trial;
        let outcome = execute_verified_transaction_with_disposition(
            state,
            audit,
            candidate.key,
            charge.clone(),
            CandidateDisposition::Accepted,
            move |trial| {
                *trial = proposed;
                Ok(())
            },
            |pre, trial| check_slp_plan_independently(pre, trial, &plan, &charge),
        );
        match outcome {
            TransactionOutcome::Committed => {
                result.changed = true;
                result.accepted = result.accepted.saturating_add(1);
            }
            TransactionOutcome::Rejected | TransactionOutcome::BudgetExhausted => {
                result.rejected = result.rejected.saturating_add(1);
            }
            TransactionOutcome::CompilerError(error) => return Err(error),
        }
    }
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.pass.as_str(), left.reason.as_str()).cmp(&(
            right.function,
            right.pass.as_str(),
            right.reason.as_str(),
        ))
    });
    result.fallbacks.dedup();
    Ok(result)
}

fn slp_fallback_charge(candidate: &super::SlpCandidate) -> CandidateBudgetCharge {
    let (scalar, emitted) = if candidate.memory.is_some() {
        (u32::from(candidate.lanes).saturating_mul(4), 5)
    } else {
        (
            u32::try_from(candidate.scalar_instructions.len()).unwrap_or(u32::MAX),
            u32::from(candidate.lanes)
                .saturating_mul(3)
                .saturating_add(1),
        )
    };
    CandidateBudgetCharge::single(
        candidate.function,
        8_u32.saturating_add(scalar).saturating_add(emitted),
        16_u32
            .saturating_add(scalar)
            .saturating_add(emitted)
            .saturating_add(emitted.saturating_mul(2)),
    )
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

fn refresh_pre_guard_cfg(
    module: &KirModule,
    contracts: &mut Option<ContractFactSet>,
) -> Result<(), String> {
    let validation = validate_kir_module(module);
    if let Some(error) = validation.errors.first() {
        return Err(error.message.clone());
    }
    if let Some(contracts) = contracts {
        contracts
            .retain_cfg_imports(module)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn run_pre_guard_sccp(
    module: &mut KirModule,
    contracts: &mut Option<ContractFactSet>,
) -> Result<bool, String> {
    // Transient scalar certificates are consumed against each immutable pre-state.
    // CFG edits happen before guard elimination, so no persistent ProofId or
    // guard-rewrite record can refer to the retired edges, parameters or FactIds.
    let proofs = ProofArena::new(0);
    let mut changed = false;
    loop {
        changed |= kir_passes::run_integer_constant_folding(module, contracts.as_ref(), &proofs)?;
        let cfg_changed = kir_passes::run_cfg_canonicalize(module, contracts.as_ref());
        if !cfg_changed {
            return Ok(changed);
        }
        changed = true;
        refresh_pre_guard_cfg(module, contracts)?;
        // Every further round removes a branch, block or scalar block parameter;
        // no pre-guard stage adds any. Scalar analysis keeps its fixed KIR budget.
    }
}

fn verified_program_state_from_pass_cache(
    module: &KirModule,
    result: &KirPassManagerResult,
    generation: u32,
    optimization_entry_module_units: Option<u32>,
) -> Result<KirVerifiedProgramState, String> {
    let cache_matches = result.verification_cache.as_ref().is_some_and(|cached| {
        cached.module == *module
            && cached.proofs == result.proofs
            && cached.eliminated_guards == result.eliminated_guards
            && cached.contract_facts == result.contract_facts
    });
    if !cache_matches {
        return Err(
            "verified KIR pass cache does not match the requested program state".to_string(),
        );
    }
    let module_units = optimization_entry_module_units.unwrap_or_else(|| {
        module.functions.iter().fold(0_u32, |total, function| {
            total.saturating_add(kir_function_units(function))
        })
    });
    KirVerifiedProgramState::from_verified_parts(
        module.clone(),
        result.contract_facts.clone(),
        result.proofs.clone(),
        result.eliminated_guards.clone(),
        generation,
        module_units,
    )
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
    let evidence = result.verification_cache.as_ref().map_or_else(
        || {
            validate_kir_optimization_evidence(
                module,
                result.contract_facts.as_ref(),
                &result.proofs,
                &result.eliminated_guards,
                generation,
            )
        },
        |cached| {
            validate_kir_optimization_evidence_incremental(
                module,
                &cached.module,
                result.contract_facts.as_ref(),
                &result.proofs,
                &result.eliminated_guards,
                generation,
            )
        },
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
    validate_kir_optimization_evidence_with_kir_errors(
        module,
        contracts,
        proofs,
        eliminations,
        generation,
        validate_kir_module(module).errors,
    )
}

fn validate_kir_optimization_evidence_incremental(
    module: &KirModule,
    previous: &KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    eliminations: &[KirGuardElimination],
    generation: u32,
) -> EvidenceValidationResult {
    validate_kir_optimization_evidence_with_kir_errors(
        module,
        contracts,
        proofs,
        eliminations,
        generation,
        crate::validate_kir_module_incremental(module, previous).errors,
    )
}

fn validate_kir_optimization_evidence_with_kir_errors(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    eliminations: &[KirGuardElimination],
    generation: u32,
    kir_errors: Vec<crate::KirValidationError>,
) -> EvidenceValidationResult {
    let mut errors = Vec::new();
    for error in kir_errors {
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
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                let proof = match &instruction.kind {
                    crate::KirInstructionKind::VectorBinary {
                        no_failure_proof, ..
                    }
                    | crate::KirInstructionKind::VectorUnary {
                        no_failure_proof, ..
                    } => *no_failure_proof,
                    _ => None,
                };
                let Some(proof_id) = proof else {
                    continue;
                };
                let Some(certificate) = proofs.get(proof_id) else {
                    errors.push(rewrite_error(
                        "missing vector no-failure proof certificate",
                        Some(proof_id),
                    ));
                    continue;
                };
                if certificate.use_site.function != function.id
                    || certificate.use_site.block != block.id
                    || certificate.use_site.instruction != Some(instruction.id)
                {
                    errors.push(rewrite_error(
                        "vector no-failure proof certificate belongs to a different instruction",
                        Some(proof_id),
                    ));
                }
            }
        }
    }
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

    #[test]
    fn incremental_module_validation_should_preserve_cross_function_identity_checks() {
        let result = verified_result();
        let previous = result.module.clone();
        let mut current = previous.clone();
        let mut duplicate = current.functions[0].clone();
        duplicate.id = FunctionId::from_index(1);
        duplicate.name = "duplicate-identities".to_string();
        current.functions.push(duplicate);

        let full = crate::validate_kir_module(&current)
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>();
        let incremental = crate::validate_kir_module_incremental(&current, &previous)
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>();
        for required in [
            "duplicate memory region id",
            "duplicate memory version definition",
            "duplicate value definition",
            "duplicate block id",
            "duplicate instruction id",
        ] {
            assert!(full.iter().any(|message| message == required));
            assert!(incremental.iter().any(|message| message == required));
        }
    }

    #[test]
    fn no_op_frontier_analysis_cache_should_require_matching_function_identity() {
        let checked = check(&SourceFile::new(
            "frontier-cache.ck",
            "export fn sum(n: u32) -> u32 { let i: u32 = 0; let total: u32 = 0; while i < n { total = total + i; i = i + 1; } return total; }",
        ));
        assert!(checked.diagnostics.is_empty());
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::NativeLibrary,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid KIR");
        let analyses = module
            .functions
            .iter()
            .map(crate::optimizer::analyze_canonical_loops_for_discovery)
            .collect::<Vec<_>>();

        assert!(cached_frontier_loop_analysis(&module.functions[0], 0, Some(&analyses)).is_some());

        let mut stale = analyses.clone();
        stale[0].function = Some(FunctionId::from_index(u32::MAX));
        assert!(cached_frontier_loop_analysis(&module.functions[0], 0, Some(&stale)).is_none());

        assert_eq!(
            no_op_native_frontiers(&module, None),
            no_op_native_frontiers(&module, Some(&analyses)),
            "reusing a matching pre-frontier loop analysis must preserve discovery output"
        );
    }
}
