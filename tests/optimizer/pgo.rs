use std::collections::BTreeSet;

use calckernel::{
    CkImmutableProfileAnalysis, CkProfileAnalysis, CkProfileAnalyzedSite, CkProfileContract,
    CkProfileFunctionWork, CkProfileKirMode, CkProfileObservation, CkProfileSiteKind,
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, SourceFile, build_kir_module, check, check_profile_guided_optimization,
    import_contract_facts, lower_to_mir, prepare_ck_profile_kir,
    propose_profile_guided_optimization, run_kir_pass_pipeline,
    run_profile_guided_kir_pass_pipeline,
};

const INLINE_SOURCE: &str = r#"
fn helper(x: i32) -> i32 {
  let a01: i32 = x + 1; let a02: i32 = a01 + 1;
  let a03: i32 = a02 + 1; let a04: i32 = a03 + 1;
  let a05: i32 = a04 + 1; let a06: i32 = a05 + 1;
  let a07: i32 = a06 + 1; let a08: i32 = a07 + 1;
  let a09: i32 = a08 + 1; let a10: i32 = a09 + 1;
  let a11: i32 = a10 + 1; let a12: i32 = a11 + 1;
  let a13: i32 = a12 + 1; let a14: i32 = a13 + 1;
  let a15: i32 = a14 + 1; let a16: i32 = a15 + 1;
  let a17: i32 = a16 + 1;
  return a17;
}
export fn kernel(x: i32) -> i32 { return helper(x); }
"#;

fn fixture(
    source: &str,
    observations: u64,
) -> (
    calckernel::CkProfileKirPlan,
    CkImmutableProfileAnalysis,
    Option<calckernel::ContractFactSet>,
) {
    let checked = check(&SourceFile::new("pgo.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let prepared = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, Some(&contracts));
    assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
    let plan = prepare_ck_profile_kir(
        prepared.artifact.as_ref().expect("O1 artifact"),
        CkProfileKirMode::Use,
    )
    .expect("use plan");
    let mut function_digests = BTreeSet::new();
    let sites = plan
        .sites
        .iter()
        .cloned()
        .map(|descriptor| {
            function_digests.insert(descriptor.function_digest);
            let observation = match &descriptor.kind {
                CkProfileSiteKind::FunctionEntry | CkProfileSiteKind::Edge { .. } => {
                    CkProfileObservation::Scalar(observations)
                }
                CkProfileSiteKind::LoopTripHistogram { .. }
                | CkProfileSiteKind::SliceLengthHistogram { .. } => {
                    let mut buckets = [0; 16];
                    buckets[7] = observations;
                    CkProfileObservation::Histogram(buckets)
                }
                CkProfileSiteKind::CandidateConstant { candidates, .. } => {
                    CkProfileObservation::CandidateConstant {
                        candidates: vec![observations; candidates.len()],
                        other: observations / 20,
                    }
                }
            };
            CkProfileAnalyzedSite {
                descriptor,
                observation,
            }
        })
        .collect();
    let functions = function_digests
        .into_iter()
        .enumerate()
        .map(|(index, function_digest)| CkProfileFunctionWork {
            function_digest,
            dynamic_work: Some(u128::from(observations).saturating_mul(100)),
            rank: Some(u32::try_from(index + 1).expect("rank")),
            hot_root: observations >= 128,
        })
        .collect();
    let analysis = CkImmutableProfileAnalysis::new(CkProfileAnalysis {
        identity_digest: [0x5a; 32],
        sites,
        functions,
    });
    (plan, analysis, prepared.contract_facts)
}

#[test]
fn pgo_pipeline_should_validate_profile_before_profile_weighted_o3() {
    let (plan, analysis, contracts) = fixture(
        "export fn kernel(items: slice<i32>, n: u32) -> i32 { let i: u32 = 0; let sum: i32 = 0; while i < n { if i == 7 { sum = sum + items[i]; } i = i + 1; } return sum; }",
        256,
    );
    let result = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.artifact.is_some());
    assert_eq!(result.records[0].name, "pgo-identity-site-validate");
    assert_eq!(result.records[1].name, "pgo-immutable-analysis");
    let guidance = result.pgo.as_ref().expect("checked PGO plan");
    assert!(
        guidance
            .functions
            .iter()
            .all(|function| function.entries == 256)
    );
    assert!(
        guidance
            .loop_hints
            .iter()
            .any(|hint| hint.minimum_trip == 33)
    );
    assert!(
        guidance
            .decisions
            .iter()
            .any(|decision| decision.reason == "dominant-profile-class")
    );
}

#[test]
fn pgo_inline_should_expand_only_the_hot_bounded_direct_call_budget() {
    let (plan, analysis, contracts) = fixture(INLINE_SOURCE, 256);
    let ordinary = run_kir_pass_pipeline(
        plan.module.clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    let profiled = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );

    assert!(ordinary.errors.is_empty(), "{:?}", ordinary.errors);
    assert!(profiled.errors.is_empty(), "{:?}", profiled.errors);
    assert_eq!(ordinary.stats.inlined_calls, 0);
    assert_eq!(profiled.stats.inlined_calls, 1);
    assert!(
        profiled
            .artifact
            .as_ref()
            .expect("profiled artifact")
            .functions
            .iter()
            .any(|function| function.name == "helper"),
        "the unchanged generic body must remain available"
    );
}

#[test]
fn pgo_checker_should_reject_mutated_counts_without_running_the_proposer() {
    let (plan, analysis, _) = fixture(INLINE_SOURCE, 256);
    let contract = CkProfileContract::schema1();
    let mut proposal =
        propose_profile_guided_optimization(&plan, &analysis, &contract).expect("proposal");
    proposal.functions[0].entries += 1;

    let error = check_profile_guided_optimization(&plan, &analysis, &contract, &proposal)
        .expect_err("mutated proposal must fail closed");
    assert!(error.contains("function profile mismatch"), "{error}");
}

#[test]
fn pgo_low_confidence_should_retain_the_ordinary_static_decision() {
    let (plan, analysis, contracts) = fixture(INLINE_SOURCE, 127);
    let ordinary = run_kir_pass_pipeline(
        plan.module.clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    let profiled = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );

    assert_eq!(ordinary.stats.inlined_calls, profiled.stats.inlined_calls);
    assert!(
        profiled
            .pgo
            .as_ref()
            .expect("PGO report")
            .decisions
            .iter()
            .any(|decision| decision.reason == "insufficient-observations")
    );
}

#[test]
fn pgo_output_should_be_deterministic_for_identical_plan_and_analysis() {
    let (plan, analysis, contracts) = fixture(INLINE_SOURCE, 256);
    let first = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );
    let second = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );
    assert_eq!(first.artifact, second.artifact);
    assert_eq!(first.pgo, second.pgo);
    assert_eq!(first.audit, second.audit);
}
