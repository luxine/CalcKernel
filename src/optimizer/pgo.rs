use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{
    BlockId, CkImmutableProfileAnalysis, CkProfileContract, CkProfileEvent, CkProfileKirMode,
    CkProfileKirPlan, CkProfileObservation, CkProfileSiteId, ContractFactSet, FunctionId,
    InstructionId, KirConsumer, KirInstructionKind, KirModule, KirOptimizationLevel,
    KirPassManagerResult, KirPassRecord, KirSanitizerMode, KirTerminator, MirCompareOp, ProofArena,
    ValueId, profile_histogram_bucket_range, profile_site_dominant_outcome,
    validate_ck_profile_kir_plan, validate_profile_analysis_for_optimizer,
};

/// Closed profile-guided decision families. None of these records are safety facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CkPgoDecisionKind {
    FunctionEntry,
    Edge,
    LoopTrip,
    SliceLength,
    CandidateConstant,
}

/// One deterministic accepted or conservative profile decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkPgoDecision {
    pub site_id: CkProfileSiteId,
    pub kind: CkPgoDecisionKind,
    pub observations: Option<u64>,
    pub selected_class: Option<u8>,
    pub accepted: bool,
    pub reason: String,
}

/// Checked function-entry metadata and optimizer hotness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkPgoFunctionProfile {
    pub function: FunctionId,
    pub function_digest: [u8; 32],
    pub site_id: CkProfileSiteId,
    pub entries: u64,
    pub confident: bool,
    pub hot: bool,
}

/// Exact branch weights derived from an equality/inequality value site whose
/// compare result is the named KIR branch condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkPgoBranchProfile {
    pub function: FunctionId,
    pub block: BlockId,
    pub instruction: InstructionId,
    pub site_id: CkProfileSiteId,
    pub then_count: u64,
    pub else_count: u64,
}

/// Conservative full-bucket trip range used only by dynamic cost ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkPgoLoopHint {
    pub function: FunctionId,
    pub header: BlockId,
    pub site_id: CkProfileSiteId,
    pub bucket: u8,
    pub minimum_trip: u32,
    pub maximum_trip: u32,
    pub observations: u64,
}

/// One dominant value/length class. The ordinary branch remains the generic fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkPgoValueHint {
    pub function: FunctionId,
    pub block: BlockId,
    pub instruction: InstructionId,
    pub site_id: CkProfileSiteId,
    pub selected_class: u8,
    pub observations: u64,
}

/// Immutable checked input to the O3 optimizer and LLVM metadata boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkPgoOptimizerPlan {
    pub identity_digest: [u8; 32],
    pub pre_profile_kir_digest: [u8; 32],
    pub functions: Vec<CkPgoFunctionProfile>,
    pub branches: Vec<CkPgoBranchProfile>,
    pub loop_hints: Vec<CkPgoLoopHint>,
    pub value_hints: Vec<CkPgoValueHint>,
    pub decisions: Vec<CkPgoDecision>,
    pub audit_digest: [u8; 32],
}

impl CkPgoOptimizerPlan {
    #[must_use]
    pub(crate) fn function_is_hot(&self, function: FunctionId) -> bool {
        self.functions
            .iter()
            .any(|profile| profile.function == function && profile.confident && profile.hot)
    }

    #[must_use]
    pub(crate) fn loop_minimum_trip(&self, function: FunctionId, header: BlockId) -> Option<u32> {
        self.loop_hints
            .iter()
            .find(|hint| hint.function == function && hint.header == header)
            .map(|hint| hint.minimum_trip)
    }

    #[must_use]
    pub(crate) fn loop_maximum_trip(&self, function: FunctionId, header: BlockId) -> Option<u32> {
        self.loop_hints
            .iter()
            .find(|hint| hint.function == function && hint.header == header)
            .map(|hint| hint.maximum_trip)
    }

    /// Returns the conservative upper end of a dominant slice-length bucket
    /// only when the checked profile site still names the instruction that
    /// defines this exact SSA loop bound. This can rank profitability, but it
    /// never proves a bound or removes the generic scalar path.
    #[must_use]
    pub(crate) fn slice_length_maximum(
        &self,
        module: &KirModule,
        function: FunctionId,
        bound: ValueId,
    ) -> Option<u32> {
        let body = module
            .functions
            .iter()
            .find(|candidate| candidate.id == function)?;
        self.value_hints.iter().find_map(|hint| {
            if hint.function != function
                || !self.decisions.iter().any(|decision| {
                    decision.site_id == hint.site_id
                        && decision.accepted
                        && decision.kind == CkPgoDecisionKind::SliceLength
                })
            {
                return None;
            }
            let mut instructions = body
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| instruction.id == hint.instruction);
            let instruction = instructions.next()?;
            if instructions.next().is_some()
                || !matches!(instruction.kind, KirInstructionKind::SliceLen { .. })
                || instruction.results.first().map(|result| result.value) != Some(bound)
            {
                return None;
            }
            profile_histogram_bucket_range(hint.selected_class).map(|(_minimum, maximum)| maximum)
        })
    }

    /// Returns whether `block` is the direct successor of an exact profiled
    /// branch whose observed share is within the frozen schema-1 cold limit.
    /// This is profitability guidance only: the block remains the generic
    /// semantic fallback and is never treated as unreachable.
    #[must_use]
    pub(crate) fn block_is_profile_cold(
        &self,
        module: &KirModule,
        function: FunctionId,
        block: BlockId,
    ) -> bool {
        let Some(function_body) = module
            .functions
            .iter()
            .find(|candidate| candidate.id == function)
        else {
            return false;
        };
        let cold_basis_points = u128::from(CkProfileContract::schema1().cold_basis_points);
        self.branches.iter().any(|profile| {
            if profile.function != function {
                return false;
            }
            let Some(branch_block) = function_body
                .blocks
                .iter()
                .find(|candidate| candidate.id == profile.block)
            else {
                return false;
            };
            let KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &branch_block.terminator
            else {
                return false;
            };
            let (target, count) = if then_edge.target == block {
                (then_edge.target, profile.then_count)
            } else if else_edge.target == block {
                (else_edge.target, profile.else_count)
            } else {
                return false;
            };
            let total = u128::from(profile.then_count) + u128::from(profile.else_count);
            target == block && total != 0 && u128::from(count) * 10_000 <= total * cold_basis_points
        })
    }

    /// Returns whether every remaining direct call to `callee` is contained in
    /// a checked profile-cold block. At least one call is required. The result
    /// can guide LLVM placement/inlining, but never removes the callee.
    #[must_use]
    #[cfg_attr(not(feature = "native-toolchain"), allow(dead_code))]
    pub(crate) fn function_is_profile_cold(&self, module: &KirModule, callee: FunctionId) -> bool {
        let Some(callee_name) = module
            .functions
            .iter()
            .find(|function| function.id == callee)
            .map(|function| function.name.as_str())
        else {
            return false;
        };
        let mut found = false;
        for caller in &module.functions {
            for block in &caller.blocks {
                for instruction in &block.instructions {
                    if matches!(
                        &instruction.kind,
                        KirInstructionKind::Call { function_name, .. }
                            if function_name == callee_name
                    ) {
                        found = true;
                        if !self.block_is_profile_cold(module, caller.id, block.id) {
                            return false;
                        }
                    }
                }
            }
        }
        found
    }
}

/// Builds an untrusted proposal from immutable observations. The independent
/// checker must accept the complete closed record before it reaches O3.
pub fn propose_profile_guided_optimization(
    profile_plan: &CkProfileKirPlan,
    analysis: &CkImmutableProfileAnalysis,
    contract: &CkProfileContract,
) -> Result<CkPgoOptimizerPlan, String> {
    validate_profile_boundary(profile_plan, analysis, contract)?;
    let mut proposal = derive_proposal(profile_plan, analysis.get(), contract)?;
    proposal.audit_digest = optimizer_plan_digest(&proposal);
    Ok(proposal)
}

/// Independently reconstructs every profile class and rejects any forged field.
pub fn check_profile_guided_optimization(
    profile_plan: &CkProfileKirPlan,
    analysis: &CkImmutableProfileAnalysis,
    contract: &CkProfileContract,
    proposal: &CkPgoOptimizerPlan,
) -> Result<(), String> {
    validate_profile_boundary(profile_plan, analysis, contract)?;
    if proposal.identity_digest != analysis.get().identity_digest
        || proposal.pre_profile_kir_digest != profile_plan.pre_profile_kir_digest
    {
        return Err("PGO optimizer identity mismatch".to_string());
    }
    let expected = independently_reconstruct_plan(profile_plan, analysis.get(), contract)?;
    if proposal.functions != expected.functions {
        return Err("PGO function profile mismatch".to_string());
    }
    if proposal.branches != expected.branches {
        return Err("PGO branch profile mismatch".to_string());
    }
    if proposal.loop_hints != expected.loop_hints {
        return Err("PGO loop profile mismatch".to_string());
    }
    if proposal.value_hints != expected.value_hints {
        return Err("PGO value profile mismatch".to_string());
    }
    if proposal.decisions != expected.decisions {
        return Err("PGO decision ledger mismatch".to_string());
    }
    if proposal.audit_digest != optimizer_plan_digest(proposal) {
        return Err("PGO optimizer audit digest mismatch".to_string());
    }
    Ok(())
}

/// Runs the existing verified O3 transaction pipeline with checked workload
/// profitability input. Failure withholds the artifact instead of falling back
/// from a malformed profile mapping.
#[must_use]
pub fn run_profile_guided_kir_pass_pipeline(
    profile_plan: &CkProfileKirPlan,
    analysis: &CkImmutableProfileAnalysis,
    contract: &CkProfileContract,
    contracts: Option<&ContractFactSet>,
) -> KirPassManagerResult {
    let mut proposal = match propose_profile_guided_optimization(profile_plan, analysis, contract)
        .and_then(|proposal| {
            check_profile_guided_optimization(profile_plan, analysis, contract, &proposal)?;
            Ok(proposal)
        }) {
        Ok(proposal) => proposal,
        Err(error) => return failed_profile_result(profile_plan, contracts, error),
    };
    let mut result = super::run_kir_pass_pipeline_with_profile(
        profile_plan.module.clone(),
        KirOptimizationLevel::O3,
        contracts,
        Some(&proposal),
    );
    if result.errors.is_empty()
        && let Some(artifact) = result.artifact.as_ref()
    {
        reconcile_profile_mappings(artifact, &mut proposal);
        if let Err(error) = validate_pgo_plan_for_kir(artifact, &proposal) {
            result.errors.push(error);
            result.artifact = None;
        }
    }
    if result.errors.is_empty() {
        result.pgo = Some(proposal);
        result.records.splice(
            0..0,
            [
                KirPassRecord {
                    name: "pgo-identity-site-validate".to_string(),
                    changed: false,
                    verified: true,
                },
                KirPassRecord {
                    name: "pgo-immutable-analysis".to_string(),
                    changed: false,
                    verified: true,
                },
            ],
        );
    }
    result
}

/// Rechecks that every metadata record still has an exact structural KIR target.
pub fn validate_pgo_plan_for_kir(
    module: &KirModule,
    plan: &CkPgoOptimizerPlan,
) -> Result<(), String> {
    if plan.audit_digest != optimizer_plan_digest(plan) {
        return Err("PGO metadata audit digest mismatch".to_string());
    }
    for profile in &plan.functions {
        if module
            .functions
            .iter()
            .all(|function| function.id != profile.function)
        {
            return Err("PGO metadata names a missing function".to_string());
        }
    }
    for branch in &plan.branches {
        let function = module
            .functions
            .iter()
            .find(|function| function.id == branch.function)
            .ok_or_else(|| "PGO metadata names a missing branch function".to_string())?;
        let block = function
            .blocks
            .iter()
            .find(|block| block.id == branch.block)
            .ok_or_else(|| "PGO metadata names a missing branch block".to_string())?;
        let mut instructions = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| instruction.id == branch.instruction);
        let instruction = instructions
            .next()
            .ok_or_else(|| "PGO metadata names a missing compare".to_string())?;
        if instructions.next().is_some() {
            return Err("PGO metadata compare mapping is ambiguous".to_string());
        }
        let condition = instruction
            .results
            .first()
            .ok_or_else(|| "PGO metadata compare has no result".to_string())?
            .value;
        if !matches!(
            block.terminator,
            KirTerminator::Branch {
                condition: branch_condition,
                ..
            } if branch_condition == condition
        ) {
            return Err("PGO metadata branch mapping is stale".to_string());
        }
    }
    for hint in &plan.loop_hints {
        if module
            .functions
            .iter()
            .find(|function| function.id == hint.function)
            .is_none_or(|function| function.blocks.iter().all(|block| block.id != hint.header))
        {
            return Err("PGO loop mapping is stale".to_string());
        }
    }
    Ok(())
}

/// Projects checked workload guidance onto one independently materialized KIR
/// module, conservatively dropping every site whose exact mapping is absent.
/// This lets separately lowered multiversion modules retain valid PGO guidance
/// without guessing mappings for functions outside their closed root.
pub fn project_pgo_plan_for_kir(
    module: &KirModule,
    plan: &CkPgoOptimizerPlan,
) -> Result<CkPgoOptimizerPlan, String> {
    if plan.audit_digest != optimizer_plan_digest(plan) {
        return Err("PGO metadata audit digest mismatch".to_string());
    }
    let mut projected = plan.clone();
    reconcile_profile_mappings(module, &mut projected);
    validate_pgo_plan_for_kir(module, &projected)?;
    Ok(projected)
}

fn validate_profile_boundary(
    profile_plan: &CkProfileKirPlan,
    analysis: &CkImmutableProfileAnalysis,
    contract: &CkProfileContract,
) -> Result<(), String> {
    if profile_plan.mode != CkProfileKirMode::Use {
        return Err("O3 PGO requires use-mode profile KIR".to_string());
    }
    if contract != &CkProfileContract::schema1() {
        return Err("O3 PGO contract is not schema 1".to_string());
    }
    if !matches!(
        profile_plan.module.config.consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        return Err("O3 PGO requires a Native consumer".to_string());
    }
    if profile_plan.module.config.sanitizer_mode != KirSanitizerMode::Disabled {
        return Err("O3 PGO is disabled by contract sanitizer mode".to_string());
    }
    validate_ck_profile_kir_plan(profile_plan).map_err(|error| error.to_string())?;
    validate_profile_analysis_for_optimizer(profile_plan, analysis, &ProofArena::new(0))
}

fn derive_proposal(
    profile_plan: &CkProfileKirPlan,
    analysis: &crate::CkProfileAnalysis,
    contract: &CkProfileContract,
) -> Result<CkPgoOptimizerPlan, String> {
    let observations = observation_map(analysis);
    let hot = analysis
        .functions
        .iter()
        .map(|function| (function.function_digest, function.hot_root))
        .collect::<BTreeMap<_, _>>();
    let mut output = empty_plan(profile_plan, analysis);
    for annotation in &profile_plan.annotations {
        let observation = observations
            .get(&annotation.site_id)
            .ok_or_else(|| "PGO proposal site is missing".to_string())?;
        propose_one(
            &profile_plan.module,
            annotation,
            observation,
            hot.get(&annotation.descriptor.function_digest)
                .copied()
                .unwrap_or(false),
            contract,
            &mut output,
        )?;
    }
    canonicalize_plan(&mut output);
    Ok(output)
}

fn independently_reconstruct_plan(
    profile_plan: &CkProfileKirPlan,
    analysis: &crate::CkProfileAnalysis,
    contract: &CkProfileContract,
) -> Result<CkPgoOptimizerPlan, String> {
    let mut output = empty_plan(profile_plan, analysis);
    for annotation in &profile_plan.annotations {
        let analyzed = analysis
            .sites
            .iter()
            .find(|site| site.descriptor.id == annotation.site_id)
            .ok_or_else(|| "PGO checker site is missing".to_string())?;
        let hot = analysis
            .functions
            .iter()
            .find(|function| function.function_digest == annotation.descriptor.function_digest)
            .is_some_and(|function| function.hot_root);
        check_one(
            &profile_plan.module,
            annotation,
            &analyzed.observation,
            hot,
            contract,
            &mut output,
        )?;
    }
    canonicalize_plan(&mut output);
    Ok(output)
}

fn empty_plan(
    profile_plan: &CkProfileKirPlan,
    analysis: &crate::CkProfileAnalysis,
) -> CkPgoOptimizerPlan {
    CkPgoOptimizerPlan {
        identity_digest: analysis.identity_digest,
        pre_profile_kir_digest: profile_plan.pre_profile_kir_digest,
        functions: Vec::new(),
        branches: Vec::new(),
        loop_hints: Vec::new(),
        value_hints: Vec::new(),
        decisions: Vec::new(),
        audit_digest: [0; 32],
    }
}

fn observation_map(
    analysis: &crate::CkProfileAnalysis,
) -> BTreeMap<CkProfileSiteId, &CkProfileObservation> {
    analysis
        .sites
        .iter()
        .map(|site| (site.descriptor.id, &site.observation))
        .collect()
}

fn propose_one(
    module: &KirModule,
    annotation: &crate::CkProfileSiteAnnotation,
    observation: &CkProfileObservation,
    hot: bool,
    contract: &CkProfileContract,
    output: &mut CkPgoOptimizerPlan,
) -> Result<(), String> {
    derive_one(module, annotation, observation, hot, contract, output)
}

fn check_one(
    module: &KirModule,
    annotation: &crate::CkProfileSiteAnnotation,
    observation: &CkProfileObservation,
    hot: bool,
    contract: &CkProfileContract,
    output: &mut CkPgoOptimizerPlan,
) -> Result<(), String> {
    // Deliberately do not call `propose_one` or `derive_one`. This is a second
    // reconstruction from the frozen KIR event, counter class, and contract.
    let (kind, observations, selected_class, accepted, reason) = match &annotation.event {
        CkProfileEvent::FunctionEntry { function, .. } => {
            let entries = match observation {
                CkProfileObservation::Scalar(entries) => *entries,
                CkProfileObservation::Unknown(_) => 0,
                _ => return Err("PGO checker function counter shape changed".to_string()),
            };
            let confident = entries >= contract.minimum_decision_observations;
            output.functions.push(CkPgoFunctionProfile {
                function: *function,
                function_digest: annotation.descriptor.function_digest,
                site_id: annotation.site_id,
                entries,
                confident,
                hot: confident && hot,
            });
            (
                CkPgoDecisionKind::FunctionEntry,
                observation.total(),
                None,
                confident,
                if confident {
                    "confident-function-entry"
                } else {
                    "insufficient-observations"
                },
            )
        }
        CkProfileEvent::Edge { .. } => {
            let total = observation.total();
            let confident =
                total.is_some_and(|value| value >= contract.minimum_decision_observations);
            (
                CkPgoDecisionKind::Edge,
                total,
                None,
                confident,
                if confident {
                    "confident-edge-count"
                } else {
                    "insufficient-observations"
                },
            )
        }
        CkProfileEvent::LoopTrip {
            function, header, ..
        } => {
            let (counts, total) = histogram(observation)?;
            let winner = profile_site_dominant_outcome(
                &counts,
                contract.minimum_decision_observations,
                contract.histogram_dominance_basis_points,
            );
            if let Some(index) = winner {
                let bucket = u8::try_from(index)
                    .map_err(|_| "PGO checker histogram class overflow".to_string())?;
                let (minimum_trip, maximum_trip) = profile_histogram_bucket_range(bucket)
                    .ok_or_else(|| "PGO checker histogram bucket is invalid".to_string())?;
                output.loop_hints.push(CkPgoLoopHint {
                    function: *function,
                    header: *header,
                    site_id: annotation.site_id,
                    bucket,
                    minimum_trip,
                    maximum_trip,
                    observations: total,
                });
            }
            (
                CkPgoDecisionKind::LoopTrip,
                Some(total),
                winner.and_then(|value| u8::try_from(value).ok()),
                winner.is_some(),
                if winner.is_some() {
                    "dominant-profile-class"
                } else if total < contract.minimum_decision_observations {
                    "insufficient-observations"
                } else {
                    "no-dominant-profile-class"
                },
            )
        }
        CkProfileEvent::SliceLength {
            function,
            block,
            instruction,
            ..
        } => {
            let (counts, total) = histogram(observation)?;
            let winner = profile_site_dominant_outcome(
                &counts,
                contract.minimum_decision_observations,
                contract.histogram_dominance_basis_points,
            );
            if let Some(index) = winner {
                output.value_hints.push(CkPgoValueHint {
                    function: *function,
                    block: *block,
                    instruction: *instruction,
                    site_id: annotation.site_id,
                    selected_class: u8::try_from(index)
                        .map_err(|_| "PGO checker length class overflow".to_string())?,
                    observations: total,
                });
            }
            (
                CkPgoDecisionKind::SliceLength,
                Some(total),
                winner.and_then(|value| u8::try_from(value).ok()),
                winner.is_some(),
                if winner.is_some() {
                    "dominant-profile-class"
                } else if total < contract.minimum_decision_observations {
                    "insufficient-observations"
                } else {
                    "no-dominant-profile-class"
                },
            )
        }
        CkProfileEvent::CandidateConstant {
            function,
            block,
            instruction,
            ..
        } => {
            let (counts, total) = candidate_counts(observation)?;
            let winner = profile_site_dominant_outcome(
                &counts,
                contract.minimum_decision_observations,
                contract.branch_dominance_basis_points,
            );
            let candidate_count = counts.len().saturating_sub(1);
            let candidate_winner = winner.filter(|winner| *winner < candidate_count);
            if let Some(index) = candidate_winner {
                output.value_hints.push(CkPgoValueHint {
                    function: *function,
                    block: *block,
                    instruction: *instruction,
                    site_id: annotation.site_id,
                    selected_class: u8::try_from(index)
                        .map_err(|_| "PGO checker value class overflow".to_string())?,
                    observations: total,
                });
                if candidate_count == 1
                    && let Some(branch) = exact_equality_branch(
                        module,
                        *function,
                        *block,
                        *instruction,
                        annotation.site_id,
                        counts[0],
                        counts[1],
                    )?
                {
                    output.branches.push(branch);
                }
            }
            (
                CkPgoDecisionKind::CandidateConstant,
                Some(total),
                candidate_winner.and_then(|value| u8::try_from(value).ok()),
                candidate_winner.is_some(),
                if candidate_winner.is_some() {
                    "dominant-profile-class"
                } else if total < contract.minimum_decision_observations {
                    "insufficient-observations"
                } else {
                    "no-dominant-profile-class"
                },
            )
        }
    };
    output.decisions.push(CkPgoDecision {
        site_id: annotation.site_id,
        kind,
        observations,
        selected_class,
        accepted,
        reason: reason.to_string(),
    });
    Ok(())
}

fn derive_one(
    module: &KirModule,
    annotation: &crate::CkProfileSiteAnnotation,
    observation: &CkProfileObservation,
    hot: bool,
    contract: &CkProfileContract,
    output: &mut CkPgoOptimizerPlan,
) -> Result<(), String> {
    let (kind, observations, selected, accepted, reason) = match &annotation.event {
        CkProfileEvent::FunctionEntry { function, .. } => {
            let entries = match observation {
                CkProfileObservation::Scalar(entries) => *entries,
                CkProfileObservation::Unknown(_) => 0,
                _ => return Err("PGO function entry counter shape changed".to_string()),
            };
            let confident = entries >= contract.minimum_decision_observations;
            output.functions.push(CkPgoFunctionProfile {
                function: *function,
                function_digest: annotation.descriptor.function_digest,
                site_id: annotation.site_id,
                entries,
                confident,
                hot: confident && hot,
            });
            (
                CkPgoDecisionKind::FunctionEntry,
                observation.total(),
                None,
                confident,
                if confident {
                    "confident-function-entry"
                } else {
                    "insufficient-observations"
                },
            )
        }
        CkProfileEvent::Edge { .. } => {
            let total = observation.total();
            (
                CkPgoDecisionKind::Edge,
                total,
                None,
                total.is_some_and(|value| value >= contract.minimum_decision_observations),
                if total.is_some_and(|value| value >= contract.minimum_decision_observations) {
                    "confident-edge-count"
                } else {
                    "insufficient-observations"
                },
            )
        }
        CkProfileEvent::LoopTrip {
            function, header, ..
        } => {
            let (counts, total) = histogram(observation)?;
            let winner = profile_site_dominant_outcome(
                &counts,
                contract.minimum_decision_observations,
                contract.histogram_dominance_basis_points,
            );
            if let Some(bucket) = winner {
                let bucket =
                    u8::try_from(bucket).map_err(|_| "PGO histogram class overflow".to_string())?;
                let (minimum_trip, maximum_trip) = profile_histogram_bucket_range(bucket)
                    .ok_or_else(|| "PGO histogram bucket is invalid".to_string())?;
                output.loop_hints.push(CkPgoLoopHint {
                    function: *function,
                    header: *header,
                    site_id: annotation.site_id,
                    bucket,
                    minimum_trip,
                    maximum_trip,
                    observations: total,
                });
            }
            (
                CkPgoDecisionKind::LoopTrip,
                Some(total),
                winner.and_then(|value| u8::try_from(value).ok()),
                winner.is_some(),
                if winner.is_some() {
                    "dominant-profile-class"
                } else if total < contract.minimum_decision_observations {
                    "insufficient-observations"
                } else {
                    "no-dominant-profile-class"
                },
            )
        }
        CkProfileEvent::SliceLength {
            function,
            block,
            instruction,
            ..
        } => {
            let (counts, total) = histogram(observation)?;
            let winner = profile_site_dominant_outcome(
                &counts,
                contract.minimum_decision_observations,
                contract.histogram_dominance_basis_points,
            );
            if let Some(selected_class) = winner {
                output.value_hints.push(CkPgoValueHint {
                    function: *function,
                    block: *block,
                    instruction: *instruction,
                    site_id: annotation.site_id,
                    selected_class: u8::try_from(selected_class)
                        .map_err(|_| "PGO length class overflow".to_string())?,
                    observations: total,
                });
            }
            (
                CkPgoDecisionKind::SliceLength,
                Some(total),
                winner.and_then(|value| u8::try_from(value).ok()),
                winner.is_some(),
                if winner.is_some() {
                    "dominant-profile-class"
                } else if total < contract.minimum_decision_observations {
                    "insufficient-observations"
                } else {
                    "no-dominant-profile-class"
                },
            )
        }
        CkProfileEvent::CandidateConstant {
            function,
            block,
            instruction,
            ..
        } => {
            let (counts, total) = candidate_counts(observation)?;
            let winner = profile_site_dominant_outcome(
                &counts,
                contract.minimum_decision_observations,
                contract.branch_dominance_basis_points,
            );
            let candidate_count = counts.len().saturating_sub(1);
            let candidate_winner = winner.filter(|winner| *winner < candidate_count);
            if let Some(selected_class) = candidate_winner {
                output.value_hints.push(CkPgoValueHint {
                    function: *function,
                    block: *block,
                    instruction: *instruction,
                    site_id: annotation.site_id,
                    selected_class: u8::try_from(selected_class)
                        .map_err(|_| "PGO value class overflow".to_string())?,
                    observations: total,
                });
                if candidate_count == 1
                    && let Some(branch) = exact_equality_branch(
                        module,
                        *function,
                        *block,
                        *instruction,
                        annotation.site_id,
                        counts[0],
                        counts[1],
                    )?
                {
                    output.branches.push(branch);
                }
            }
            (
                CkPgoDecisionKind::CandidateConstant,
                Some(total),
                candidate_winner.and_then(|value| u8::try_from(value).ok()),
                candidate_winner.is_some(),
                if candidate_winner.is_some() {
                    "dominant-profile-class"
                } else if total < contract.minimum_decision_observations {
                    "insufficient-observations"
                } else {
                    "no-dominant-profile-class"
                },
            )
        }
    };
    output.decisions.push(CkPgoDecision {
        site_id: annotation.site_id,
        kind,
        observations,
        selected_class: selected,
        accepted,
        reason: reason.to_string(),
    });
    Ok(())
}

fn histogram(observation: &CkProfileObservation) -> Result<(Vec<u64>, u64), String> {
    let CkProfileObservation::Histogram(buckets) = observation else {
        if matches!(observation, CkProfileObservation::Unknown(_)) {
            return Ok((vec![0; 16], 0));
        }
        return Err("PGO histogram counter shape changed".to_string());
    };
    let total = buckets
        .iter()
        .copied()
        .try_fold(0u64, u64::checked_add)
        .ok_or_else(|| "PGO histogram total overflow".to_string())?;
    Ok((buckets.to_vec(), total))
}

fn candidate_counts(observation: &CkProfileObservation) -> Result<(Vec<u64>, u64), String> {
    let CkProfileObservation::CandidateConstant { candidates, other } = observation else {
        if matches!(observation, CkProfileObservation::Unknown(_)) {
            return Ok((vec![0], 0));
        }
        return Err("PGO candidate counter shape changed".to_string());
    };
    let mut counts = candidates.clone();
    counts.push(*other);
    let total = counts
        .iter()
        .copied()
        .try_fold(0u64, u64::checked_add)
        .ok_or_else(|| "PGO candidate total overflow".to_string())?;
    Ok((counts, total))
}

fn exact_equality_branch(
    module: &KirModule,
    function_id: FunctionId,
    block_id: BlockId,
    instruction_id: InstructionId,
    site_id: CkProfileSiteId,
    matches: u64,
    other: u64,
) -> Result<Option<CkPgoBranchProfile>, String> {
    let block = module
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .and_then(|function| function.blocks.iter().find(|block| block.id == block_id))
        .ok_or_else(|| "PGO value site block is missing".to_string())?;
    let instruction = block
        .instructions
        .iter()
        .find(|instruction| instruction.id == instruction_id)
        .ok_or_else(|| "PGO value site instruction is missing".to_string())?;
    let KirInstructionKind::Compare { op, .. } = instruction.kind else {
        return Err("PGO value site is not a compare".to_string());
    };
    let Some(condition) = instruction.results.first().map(|result| result.value) else {
        return Err("PGO value compare has no result".to_string());
    };
    if !matches!(block.terminator, KirTerminator::Branch { condition: value, .. } if value == condition)
    {
        return Ok(None);
    }
    let (then_count, else_count) = match op {
        MirCompareOp::Eq => (matches, other),
        MirCompareOp::Ne => (other, matches),
        MirCompareOp::Lt | MirCompareOp::Le | MirCompareOp::Gt | MirCompareOp::Ge => {
            return Ok(None);
        }
    };
    Ok(Some(CkPgoBranchProfile {
        function: function_id,
        block: block_id,
        instruction: instruction_id,
        site_id,
        then_count,
        else_count,
    }))
}

fn canonicalize_plan(plan: &mut CkPgoOptimizerPlan) {
    plan.functions.sort_by_key(|profile| profile.function);
    plan.branches
        .sort_by_key(|profile| (profile.function, profile.block, profile.instruction));
    plan.loop_hints
        .sort_by_key(|hint| (hint.function, hint.header));
    plan.value_hints
        .sort_by_key(|hint| (hint.function, hint.block, hint.instruction));
    plan.decisions.sort_by_key(|decision| decision.site_id);
}

fn reconcile_profile_mappings(module: &KirModule, plan: &mut CkPgoOptimizerPlan) {
    let mut unavailable = Vec::new();
    plan.functions.retain(|profile| {
        let retained = module
            .functions
            .iter()
            .any(|function| function.id == profile.function);
        if !retained {
            unavailable.push(profile.site_id);
        }
        retained
    });
    plan.branches.retain(|profile| {
        let retained = exact_branch_mapping(module, profile);
        if !retained {
            unavailable.push(profile.site_id);
        }
        retained
    });
    plan.loop_hints.retain(|hint| {
        let retained = module
            .functions
            .iter()
            .find(|function| function.id == hint.function)
            .is_some_and(|function| function.blocks.iter().any(|block| block.id == hint.header));
        if !retained {
            unavailable.push(hint.site_id);
        }
        retained
    });
    plan.value_hints.retain(|hint| {
        let retained = module
            .functions
            .iter()
            .find(|function| function.id == hint.function)
            .and_then(|function| function.blocks.iter().find(|block| block.id == hint.block))
            .is_some_and(|block| {
                block
                    .instructions
                    .iter()
                    .any(|instruction| instruction.id == hint.instruction)
            });
        if !retained {
            unavailable.push(hint.site_id);
        }
        retained
    });
    unavailable.sort_unstable();
    unavailable.dedup();
    for decision in &mut plan.decisions {
        if unavailable.binary_search(&decision.site_id).is_ok() {
            decision.accepted = false;
            decision.selected_class = None;
            decision.reason = "mapping-unavailable".to_string();
        }
    }
    canonicalize_plan(plan);
    plan.audit_digest = optimizer_plan_digest(plan);
}

fn exact_branch_mapping(module: &KirModule, profile: &CkPgoBranchProfile) -> bool {
    let Some(function) = module
        .functions
        .iter()
        .find(|function| function.id == profile.function)
    else {
        return false;
    };
    let Some(block) = function
        .blocks
        .iter()
        .find(|block| block.id == profile.block)
    else {
        return false;
    };
    let mut instructions = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| instruction.id == profile.instruction);
    let Some(condition) = instructions
        .next()
        .and_then(|instruction| instruction.results.first())
        .map(|result| result.value)
    else {
        return false;
    };
    if instructions.next().is_some() {
        return false;
    }
    let Some(branch_condition) = (match &block.terminator {
        KirTerminator::Branch { condition, .. } => Some(*condition),
        KirTerminator::Return { .. } | KirTerminator::Jump { .. } => None,
    }) else {
        return false;
    };
    condition == branch_condition
}

fn optimizer_plan_digest(plan: &CkPgoOptimizerPlan) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-PGO-OPTIMIZER-PLAN-1\0");
    hasher.update(plan.identity_digest);
    hasher.update(plan.pre_profile_kir_digest);
    for function in &plan.functions {
        hasher.update(function.function.index().to_be_bytes());
        hasher.update(function.function_digest);
        hasher.update(function.site_id.0);
        hasher.update(function.entries.to_be_bytes());
        hasher.update([u8::from(function.confident), u8::from(function.hot)]);
    }
    for branch in &plan.branches {
        hasher.update(branch.function.index().to_be_bytes());
        hasher.update(branch.block.index().to_be_bytes());
        hasher.update(branch.instruction.index().to_be_bytes());
        hasher.update(branch.site_id.0);
        hasher.update(branch.then_count.to_be_bytes());
        hasher.update(branch.else_count.to_be_bytes());
    }
    for hint in &plan.loop_hints {
        hasher.update(hint.function.index().to_be_bytes());
        hasher.update(hint.header.index().to_be_bytes());
        hasher.update(hint.site_id.0);
        hasher.update([hint.bucket]);
        hasher.update(hint.minimum_trip.to_be_bytes());
        hasher.update(hint.maximum_trip.to_be_bytes());
        hasher.update(hint.observations.to_be_bytes());
    }
    for hint in &plan.value_hints {
        hasher.update(hint.function.index().to_be_bytes());
        hasher.update(hint.block.index().to_be_bytes());
        hasher.update(hint.instruction.index().to_be_bytes());
        hasher.update(hint.site_id.0);
        hasher.update([hint.selected_class]);
        hasher.update(hint.observations.to_be_bytes());
    }
    for decision in &plan.decisions {
        hasher.update(decision.site_id.0);
        hasher.update([decision.kind as u8]);
        hasher.update(decision.observations.unwrap_or(u64::MAX).to_be_bytes());
        hasher.update([decision.selected_class.unwrap_or(u8::MAX)]);
        hasher.update([u8::from(decision.accepted)]);
        hasher.update((decision.reason.len() as u64).to_be_bytes());
        hasher.update(decision.reason.as_bytes());
    }
    hasher.finalize().into()
}

fn failed_profile_result(
    profile_plan: &CkProfileKirPlan,
    contracts: Option<&ContractFactSet>,
    error: String,
) -> KirPassManagerResult {
    let mut result = super::run_kir_pass_pipeline(
        profile_plan.module.clone(),
        KirOptimizationLevel::O0,
        contracts,
    );
    result.artifact = None;
    result.errors.push(error);
    result
}
