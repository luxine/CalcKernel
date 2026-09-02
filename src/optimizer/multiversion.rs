use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::{
    FunctionId, KIR_MULTIVERSION_BUNDLE_SCHEMA, KirConsumer, KirFunction, KirInstructionKind,
    KirModule, KirMultiversionBundle, KirMultiversionDispatchEntry, KirMultiversionExplanation,
    KirMultiversionHiddenSymbol, KirMultiversionRootBundle, KirMultiversionTargetSet,
    KirMultiversionTierId, KirMultiversionVariant, KirOperationAvailability, KirSanitizerMode,
    KirTargetProfile, KirTerminator, KirValueType, kir_function_units,
    kir_multiversion_module_digest, validate_kir_module,
};

/// Fixed CK 0.13 multiversion profitability threshold.
pub const KIR_MULTIVERSION_MINIMUM_BENEFIT_PERCENT: u64 = 10;
/// Fixed CK 0.13 absolute profitability floor in target-cost units.
pub const KIR_MULTIVERSION_MINIMUM_BENEFIT_UNITS: u64 = 2;
/// Maximum accepted enhanced implementations for one public root.
pub const KIR_MULTIVERSION_MAX_ENHANCED_VARIANTS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionPlanningRequest {
    pub logical_pre_state: KirModule,
    pub target_set: KirMultiversionTargetSet,
    /// `Some` means a valid profile is attached and only listed hot roots are
    /// eligible. `None` selects ordinary static target costs.
    pub pgo_hot_roots: Option<BTreeSet<FunctionId>>,
    /// Growth already consumed by earlier O3 specialization/transaction work.
    pub shared_growth_consumed: u32,
}

/// Produces a deterministic proposal. The returned value is not authoritative
/// until `check_kir_multiversion_bundle` succeeds.
pub fn propose_kir_multiversion_bundle(
    request: &KirMultiversionPlanningRequest,
) -> Result<KirMultiversionBundle, String> {
    build_candidate_bundle(request, PlanningAuthority::Proposer)
}

/// Independently reconstructs the closed plan from the immutable inputs and
/// rejects any mutated feature, proof, mapping, symbol, budget, or order data.
pub fn check_kir_multiversion_bundle(
    request: &KirMultiversionPlanningRequest,
    proposal: &KirMultiversionBundle,
) -> Result<(), String> {
    let expected = build_candidate_bundle(request, PlanningAuthority::Checker)?;
    if proposal.schema_version != expected.schema_version
        || proposal.logical_pre_state_digest != expected.logical_pre_state_digest
        || proposal.baseline != expected.baseline
    {
        return Err("multiversion logical pre-state or baseline mismatch".to_string());
    }
    if proposal.target_set != expected.target_set {
        return Err("multiversion target-set or target-profile mismatch".to_string());
    }
    if proposal.baseline_kir_units != expected.baseline_kir_units
        || proposal.shared_growth_consumed_before != expected.shared_growth_consumed_before
        || proposal.trial_audit_units != expected.trial_audit_units
        || proposal.additional_kir_units != expected.additional_kir_units
        || proposal.total_kir_units != expected.total_kir_units
    {
        return Err("multiversion shared growth budget mismatch".to_string());
    }
    if proposal.roots.len() != expected.roots.len() {
        return Err("multiversion root order mismatch".to_string());
    }
    for (actual_root, expected_root) in proposal.roots.iter().zip(&expected.roots) {
        if actual_root.root != expected_root.root
            || actual_root.public_symbol != expected_root.public_symbol
        {
            return Err("multiversion root or symbol order mismatch".to_string());
        }
        if actual_root.variants.len() != expected_root.variants.len() {
            return Err("multiversion variant count or order mismatch".to_string());
        }
        for (actual, expected) in actual_root.variants.iter().zip(&expected_root.variants) {
            if actual.tier != expected.tier {
                return Err("multiversion variant ranking order mismatch".to_string());
            }
            if actual.required_features != expected.required_features
                || actual.target_profile_digest != expected.target_profile_digest
                || actual.feature_audit_digest != expected.feature_audit_digest
            {
                return Err("multiversion variant feature audit mismatch".to_string());
            }
            if actual.proof_digest != expected.proof_digest
                || actual.logical_pre_state_digest != expected.logical_pre_state_digest
            {
                return Err("multiversion variant proof or pre-state mismatch".to_string());
            }
            if actual.kir_units != expected.kir_units
                || actual.predicted_baseline_cost != expected.predicted_baseline_cost
                || actual.predicted_variant_cost != expected.predicted_variant_cost
            {
                return Err("multiversion variant profitability or size mismatch".to_string());
            }
            if actual.hidden_symbols != expected.hidden_symbols || actual.module != expected.module
            {
                return Err(
                    "multiversion variant module mapping or hidden symbol mismatch".to_string(),
                );
            }
            if actual.codegen_digest != expected.codegen_digest {
                return Err("multiversion variant codegen digest mismatch".to_string());
            }
        }
    }
    if proposal.dispatch_plan != expected.dispatch_plan {
        return Err("multiversion dispatch plan or variant order mismatch".to_string());
    }
    if proposal.explanations != expected.explanations {
        return Err("multiversion explanation mismatch".to_string());
    }
    if proposal.digest != expected.digest {
        return Err("multiversion bundle digest mismatch".to_string());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PlanningAuthority {
    Proposer,
    Checker,
}

fn build_candidate_bundle(
    request: &KirMultiversionPlanningRequest,
    _authority: PlanningAuthority,
) -> Result<KirMultiversionBundle, String> {
    validate_request(request)?;
    let baseline = &request.logical_pre_state;
    let pre_state_digest = kir_multiversion_module_digest(baseline);
    let baseline_kir_units = module_units(baseline);
    let growth_limit = baseline_kir_units;
    let mut accepted_growth = request.shared_growth_consumed;
    let mut trial_audit_units = 0u32;
    let call_graph = CallGraph::for_module(baseline);
    let roots = eligible_root_ids(baseline);
    let baseline_profile = &request.target_set.tiers[0].profile;
    let baseline_score = vector_cost_score(baseline_profile);
    let mut root_bundles = Vec::new();
    let mut explanations = Vec::new();

    for root in roots {
        let function = function_by_id(baseline, root)?;
        let public_symbol = function.name.clone();
        let mut candidates = Vec::new();
        if request
            .pgo_hot_roots
            .as_ref()
            .is_some_and(|hot| !hot.contains(&root))
        {
            explanations.push(explanation(root, None, false, "not-pgo-hot"));
        } else if call_graph.is_recursive_from(root) {
            explanations.push(explanation(root, None, false, "recursive-root"));
        } else {
            let closure = call_graph.closure(root);
            let dependent_work =
                closure.iter().try_fold(0u64, |total, function| {
                    Ok::<_, String>(total.saturating_add(target_dependent_work(function_by_id(
                        baseline, *function,
                    )?)))
                })?;
            let reachable_units = closure.iter().try_fold(0u32, |total, function| {
                Ok::<_, String>(
                    total.saturating_add(kir_function_units(function_by_id(baseline, *function)?)),
                )
            })?;
            if dependent_work == 0 {
                explanations.push(explanation(
                    root,
                    None,
                    false,
                    "no-target-dependent-benefit",
                ));
            } else {
                for tier in request.target_set.tiers.iter().skip(1) {
                    let (module, symbols) = build_hidden_variant_module(
                        baseline,
                        &call_graph,
                        root,
                        tier.id,
                        &tier.profile,
                        &pre_state_digest,
                    )?;
                    let units = module_units(&module);
                    // This audit charge is monotonic even when profitability,
                    // rank, or growth checks later reject the trial.
                    trial_audit_units = trial_audit_units.saturating_add(units);
                    let baseline_cost =
                        estimated_cost(reachable_units, dependent_work, baseline_score);
                    let variant_cost = estimated_cost(
                        reachable_units,
                        dependent_work,
                        vector_cost_score(&tier.profile),
                    );
                    let benefit = baseline_cost.saturating_sub(variant_cost);
                    let percentage = benefit.saturating_mul(100) / baseline_cost.max(1);
                    if benefit < KIR_MULTIVERSION_MINIMUM_BENEFIT_UNITS
                        || percentage < KIR_MULTIVERSION_MINIMUM_BENEFIT_PERCENT
                    {
                        explanations.push(explanation(
                            root,
                            Some(tier.id),
                            false,
                            "insufficient-target-benefit",
                        ));
                        continue;
                    }
                    let module_digest = kir_multiversion_module_digest(&module);
                    let feature_audit_digest =
                        digest_feature_audit(tier.id, &tier.required_features, &module_digest);
                    let codegen_digest =
                        digest_codegen(&module_digest, &tier.digest, &feature_audit_digest);
                    let proof_digest = digest_variant_proof(
                        root,
                        tier.id,
                        &pre_state_digest,
                        &module_digest,
                        (baseline_cost, variant_cost),
                        units,
                        &symbols,
                    );
                    candidates.push(KirMultiversionVariant {
                        root,
                        tier: tier.id,
                        module,
                        logical_pre_state_digest: pre_state_digest,
                        target_profile_digest: tier.profile.digest_hex(),
                        required_features: tier.required_features.clone(),
                        predicted_baseline_cost: baseline_cost,
                        predicted_variant_cost: variant_cost,
                        kir_units: units,
                        proof_digest,
                        feature_audit_digest,
                        codegen_digest,
                        hidden_symbols: symbols,
                    });
                }
            }
        }
        candidates.sort_by_key(|variant| {
            (
                variant.predicted_variant_cost,
                variant.kir_units,
                variant.required_features.len(),
                variant.tier,
                variant.root,
            )
        });
        let mut accepted = Vec::new();
        for candidate in candidates {
            if accepted.len() == KIR_MULTIVERSION_MAX_ENHANCED_VARIANTS {
                explanations.push(explanation(root, Some(candidate.tier), false, "non-winner"));
            } else if accepted_growth.saturating_add(candidate.kir_units) > growth_limit {
                explanations.push(explanation(
                    root,
                    Some(candidate.tier),
                    false,
                    "shared-growth-budget-exhausted",
                ));
            } else {
                accepted_growth = accepted_growth.saturating_add(candidate.kir_units);
                explanations.push(explanation(root, Some(candidate.tier), true, "accepted"));
                accepted.push(candidate);
            }
        }
        if request.target_set.tiers.len() == 1 {
            explanations.push(explanation(
                root,
                None,
                false,
                "no-compatible-enhanced-tier",
            ));
        }
        root_bundles.push(KirMultiversionRootBundle {
            root,
            public_symbol,
            variants: accepted,
        });
    }

    let additional_kir_units = root_bundles
        .iter()
        .flat_map(|root| &root.variants)
        .fold(0u32, |total, variant| {
            total.saturating_add(variant.kir_units)
        });
    if additional_kir_units > baseline_kir_units.saturating_sub(request.shared_growth_consumed) {
        return Err("multiversion accepted variants exceed shared growth budget".to_string());
    }
    let dispatch_plan = root_bundles
        .iter()
        .map(|root| {
            let mut ranked_tiers = root
                .variants
                .iter()
                .map(|variant| variant.tier)
                .collect::<Vec<_>>();
            ranked_tiers.push(KirMultiversionTierId::Baseline);
            let mut implementation_symbols = root
                .variants
                .iter()
                .map(|variant| {
                    variant
                        .hidden_symbols
                        .iter()
                        .find(|symbol| symbol.source_name == root.public_symbol)
                        .expect("root symbol is present in its closure")
                        .hidden_name
                        .clone()
                })
                .collect::<Vec<_>>();
            implementation_symbols.push(root.public_symbol.clone());
            KirMultiversionDispatchEntry {
                root: root.root,
                public_symbol: root.public_symbol.clone(),
                ranked_tiers,
                implementation_symbols,
            }
        })
        .collect();
    let mut bundle = KirMultiversionBundle {
        schema_version: KIR_MULTIVERSION_BUNDLE_SCHEMA,
        target_set: request.target_set.clone(),
        logical_pre_state_digest: pre_state_digest,
        baseline: baseline.clone(),
        baseline_kir_units,
        shared_growth_consumed_before: request.shared_growth_consumed,
        trial_audit_units,
        additional_kir_units,
        total_kir_units: baseline_kir_units.saturating_add(additional_kir_units),
        roots: root_bundles,
        dispatch_plan,
        explanations,
        digest: [0; 32],
    };
    bundle.digest = Sha256::digest(bundle.canonical_bytes_without_digest()).into();
    Ok(bundle)
}

fn validate_request(request: &KirMultiversionPlanningRequest) -> Result<(), String> {
    request.target_set.validate()?;
    if request.logical_pre_state.config.consumer != request.target_set.consumer
        || !matches!(
            request.logical_pre_state.config.consumer,
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
        )
    {
        return Err("multiversion planning requires the matching Native consumer".to_string());
    }
    if request.logical_pre_state.config.sanitizer_mode != KirSanitizerMode::Disabled {
        return Err(
            "multiversion planning is incompatible with contract sanitizer mode".to_string(),
        );
    }
    if request.logical_pre_state.profile != request.target_set.tiers[0].profile {
        return Err(
            "multiversion logical pre-state is not bound to the target-set baseline".to_string(),
        );
    }
    let validation = validate_kir_module(&request.logical_pre_state);
    if !validation.errors.is_empty() {
        return Err(format!(
            "multiversion logical pre-state KIR is invalid: {}",
            validation
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if request.pgo_hot_roots.as_ref().is_some_and(|roots| {
        roots
            .iter()
            .any(|root| function_by_id(&request.logical_pre_state, *root).is_err())
    }) {
        return Err("multiversion PGO hot-root set names a missing function".to_string());
    }
    Ok(())
}

fn eligible_root_ids(module: &KirModule) -> Vec<FunctionId> {
    let entry_name = module
        .entry
        .as_ref()
        .map(|entry| entry.function_name.as_str());
    let mut roots = module
        .functions
        .iter()
        .filter(|function| function.exported || entry_name == Some(function.name.as_str()))
        .map(|function| function.id)
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

struct CallGraph {
    by_id: BTreeMap<FunctionId, BTreeSet<FunctionId>>,
}

impl CallGraph {
    fn for_module(module: &KirModule) -> Self {
        let by_name = module
            .functions
            .iter()
            .map(|function| (function.name.clone(), function.id))
            .collect::<BTreeMap<_, _>>();
        let by_id = module
            .functions
            .iter()
            .map(|function| {
                let callees = function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .filter_map(|instruction| match &instruction.kind {
                        KirInstructionKind::Call { function_name, .. } => {
                            by_name.get(function_name)
                        }
                        _ => None,
                    })
                    .copied()
                    .collect();
                (function.id, callees)
            })
            .collect();
        Self { by_id }
    }

    fn closure(&self, root: FunctionId) -> BTreeSet<FunctionId> {
        let mut result = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(function) = pending.pop() {
            if !result.insert(function) {
                continue;
            }
            if let Some(callees) = self.by_id.get(&function) {
                pending.extend(callees.iter().rev().copied());
            }
        }
        result
    }

    fn is_recursive_from(&self, root: FunctionId) -> bool {
        fn visit(
            graph: &CallGraph,
            node: FunctionId,
            active: &mut BTreeSet<FunctionId>,
            complete: &mut BTreeSet<FunctionId>,
        ) -> bool {
            if active.contains(&node) {
                return true;
            }
            if !complete.insert(node) {
                return false;
            }
            active.insert(node);
            let recursive = graph.by_id.get(&node).is_some_and(|callees| {
                callees
                    .iter()
                    .any(|callee| visit(graph, *callee, active, complete))
            });
            active.remove(&node);
            recursive
        }
        visit(self, root, &mut BTreeSet::new(), &mut BTreeSet::new())
    }
}

fn build_hidden_variant_module(
    baseline: &KirModule,
    graph: &CallGraph,
    root: FunctionId,
    tier: KirMultiversionTierId,
    profile: &KirTargetProfile,
    pre_state_digest: &[u8; 32],
) -> Result<(KirModule, Vec<KirMultiversionHiddenSymbol>), String> {
    let closure = graph.closure(root);
    let suffix = format!(
        "{}_{:02x}{:02x}{:02x}{:02x}",
        tier.stable_name().replace('-', "_"),
        pre_state_digest[0],
        pre_state_digest[1],
        pre_state_digest[2],
        pre_state_digest[3],
    );
    let mut symbols = baseline
        .functions
        .iter()
        .filter(|function| closure.contains(&function.id))
        .map(|function| KirMultiversionHiddenSymbol {
            source_name: function.name.clone(),
            hidden_name: format!("__ck_mv_{}_{}", function.name, suffix),
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left.source_name.cmp(&right.source_name));
    let rename = symbols
        .iter()
        .map(|symbol| (symbol.source_name.clone(), symbol.hidden_name.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut functions = baseline
        .functions
        .iter()
        .filter(|function| closure.contains(&function.id))
        .cloned()
        .collect::<Vec<_>>();
    for function in &mut functions {
        function.exported = false;
        function.name = rename
            .get(&function.name)
            .expect("closure function has hidden name")
            .clone();
        for instruction in function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
        {
            if let KirInstructionKind::Call { function_name, .. } = &mut instruction.kind
                && let Some(hidden) = rename.get(function_name)
            {
                function_name.clone_from(hidden);
            }
        }
    }
    let module = KirModule {
        config: baseline.config,
        profile: profile.clone(),
        entry: None,
        structs: baseline.structs.clone(),
        functions,
        tune_layout: None,
    };
    let validation = validate_kir_module(&module);
    if !validation.errors.is_empty() {
        return Err(format!(
            "multiversion hidden variant KIR failed validation: {}",
            validation
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok((module, symbols))
}

fn target_dependent_work(function: &KirFunction) -> u64 {
    let vector = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            instruction.results.iter().any(|result| {
                matches!(
                    result.type_node,
                    KirValueType::FixedVector { .. } | KirValueType::Mask { .. }
                )
            })
        })
        .count();
    let loops = function
        .blocks
        .iter()
        .filter(|block| match &block.terminator {
            KirTerminator::Jump { edge } => edge.target <= block.id,
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => then_edge.target <= block.id || else_edge.target <= block.id,
            KirTerminator::Return { .. } => false,
        })
        .count();
    u64::try_from(vector.saturating_add(loops.saturating_mul(4))).unwrap_or(u64::MAX)
}

fn vector_cost_score(profile: &KirTargetProfile) -> u64 {
    let mut total = 0u64;
    let mut count = 0u64;
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| key.lanes > 1)
    {
        if let Some(KirOperationAvailability::Legal(cost)) = profile.operation_availability(&key) {
            total = total.saturating_add(u64::from(cost.cost));
            count += 1;
        }
    }
    if count == 0 {
        u64::MAX / 1024
    } else {
        total.div_ceil(count)
    }
}

fn estimated_cost(reachable_units: u32, dependent_work: u64, vector_score: u64) -> u64 {
    u64::from(reachable_units).saturating_add(
        dependent_work
            .saturating_mul(vector_score)
            .saturating_mul(8),
    )
}

fn module_units(module: &KirModule) -> u32 {
    module.functions.iter().fold(0u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}

fn function_by_id(module: &KirModule, id: FunctionId) -> Result<&KirFunction, String> {
    module
        .functions
        .iter()
        .find(|function| function.id == id)
        .ok_or_else(|| format!("multiversion function f{} is missing", id.index()))
}

fn explanation(
    root: FunctionId,
    tier: Option<KirMultiversionTierId>,
    accepted: bool,
    reason: &str,
) -> KirMultiversionExplanation {
    KirMultiversionExplanation {
        root,
        tier,
        accepted,
        reason: reason.to_string(),
    }
}

fn digest_variant_proof(
    root: FunctionId,
    tier: KirMultiversionTierId,
    pre_state: &[u8; 32],
    module: &[u8; 32],
    costs: (u64, u64),
    units: u32,
    symbols: &[KirMultiversionHiddenSymbol],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-MULTIVERSION-PROOF\0");
    hasher.update(root.index().to_be_bytes());
    hasher.update(tier.stable_name().as_bytes());
    hasher.update(pre_state);
    hasher.update(module);
    hasher.update(costs.0.to_be_bytes());
    hasher.update(costs.1.to_be_bytes());
    hasher.update(units.to_be_bytes());
    for symbol in symbols {
        hasher.update(symbol.source_name.as_bytes());
        hasher.update([0]);
        hasher.update(symbol.hidden_name.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

fn digest_feature_audit(
    tier: KirMultiversionTierId,
    features: &[String],
    module: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-MULTIVERSION-FEATURE-AUDIT\0");
    hasher.update(tier.stable_name().as_bytes());
    hasher.update(module);
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

fn digest_codegen(module: &[u8; 32], tier: &[u8; 32], audit: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-MULTIVERSION-CODEGEN\0");
    hasher.update(module);
    hasher.update(tier);
    hasher.update(audit);
    hasher.finalize().into()
}
