use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirMultiversionPlanningRequest,
    KirMultiversionPlatform, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode,
    KirTargetArchitecture, KirTargetOperatingSystem, SourceFile, build_kir_module, check,
    check_kir_multiversion_bundle, import_contract_facts, lower_to_mir,
    propose_kir_multiversion_bundle, run_kir_pass_pipeline,
};

fn request() -> KirMultiversionPlanningRequest {
    let checked = check(&SourceFile::new(
        "multiversion.ck",
        "export fn sum(items: slice<i32>, n: u32) -> i32 { let i: u32 = 0; let total: i32 = 0; while i < n { total = total + items[i]; i = i + 1; } return total; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let target_set = calckernel::KirMultiversionTargetSet::schema1_fixture(
        KirMultiversionPlatform {
            architecture: KirTargetArchitecture::X86_64,
            operating_system: KirTargetOperatingSystem::Linux,
        },
        KirConsumer::NativeLibrary,
    )
    .expect("target set");
    let mut kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    kir.profile = target_set.tiers[0].profile.clone();
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    KirMultiversionPlanningRequest {
        logical_pre_state: result.artifact.expect("O3 artifact"),
        target_set,
        pgo_hot_roots: None,
        shared_growth_consumed: 0,
    }
}

#[test]
fn multiversion_planner_should_build_a_closed_verified_bundle_from_one_pre_state() {
    let request = request();
    let first = propose_kir_multiversion_bundle(&request).expect("proposal");
    let second = propose_kir_multiversion_bundle(&request).expect("repeat proposal");
    check_kir_multiversion_bundle(&request, &first).expect("independent check");

    assert_eq!(first, second);
    assert_eq!(first.baseline, request.logical_pre_state);
    assert!(!first.roots.is_empty());
    assert!(first.roots[0].variants.len() <= 2);
    assert!(
        first.roots[0]
            .variants
            .iter()
            .all(|variant| variant.logical_pre_state_digest == first.logical_pre_state_digest)
    );
    assert!(first.additional_kir_units <= first.baseline_kir_units);
    assert!(first.total_kir_units <= first.baseline_kir_units.saturating_mul(2));
}

#[test]
fn multiversion_checker_should_reject_mutated_proof_feature_budget_and_order() {
    let request = request();
    let proposal = propose_kir_multiversion_bundle(&request).expect("proposal");
    assert!(
        !proposal.roots[0].variants.is_empty(),
        "fixture must be eligible"
    );

    let mut forged = proposal.clone();
    forged.roots[0].variants[0].proof_digest[0] ^= 1;
    assert!(
        check_kir_multiversion_bundle(&request, &forged)
            .expect_err("proof mutation")
            .contains("proof")
    );

    let mut forged = proposal.clone();
    forged.roots[0].variants[0]
        .required_features
        .push("forged".to_string());
    assert!(
        check_kir_multiversion_bundle(&request, &forged)
            .expect_err("feature mutation")
            .contains("feature")
    );

    let mut forged = proposal.clone();
    forged.additional_kir_units = forged.additional_kir_units.saturating_add(1);
    assert!(
        check_kir_multiversion_bundle(&request, &forged)
            .expect_err("budget mutation")
            .contains("budget")
    );

    let mut forged = proposal.clone();
    forged.roots[0].variants[0].predicted_variant_cost += 1;
    assert!(
        check_kir_multiversion_bundle(&request, &forged)
            .expect_err("profit mutation")
            .contains("profitability")
    );

    let mut forged = proposal.clone();
    forged.roots[0].variants[0].hidden_symbols[0]
        .hidden_name
        .push_str("_forged");
    assert!(
        check_kir_multiversion_bundle(&request, &forged)
            .expect_err("symbol mutation")
            .contains("symbol")
    );

    let mut forged = proposal.clone();
    forged.dispatch_plan[0].ranked_tiers.swap(0, 1);
    assert!(
        check_kir_multiversion_bundle(&request, &forged)
            .expect_err("order mutation")
            .contains("order")
    );
}

#[test]
fn multiversion_profile_hotness_and_shared_growth_should_fail_closed() {
    let mut request = request();
    request.pgo_hot_roots = Some(Default::default());
    let cold = propose_kir_multiversion_bundle(&request).expect("cold fallback");
    assert!(cold.roots.iter().all(|root| root.variants.is_empty()));
    assert!(
        cold.explanations
            .iter()
            .any(|item| item.reason == "not-pgo-hot")
    );

    request.pgo_hot_roots = None;
    request.shared_growth_consumed = u32::MAX;
    let exhausted = propose_kir_multiversion_bundle(&request).expect("budget fallback");
    assert!(exhausted.roots.iter().all(|root| root.variants.is_empty()));
    assert!(
        exhausted
            .explanations
            .iter()
            .any(|item| item.reason == "shared-growth-budget-exhausted")
    );
}
