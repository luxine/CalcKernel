use std::collections::{BTreeMap, BTreeSet};

use calckernel::{
    CkImmutableProfileAnalysis, CkProfileAnalysis, CkProfileAnalyzedSite, CkProfileContract,
    CkProfileFunctionWork, CkProfileKirMode, CkProfileObservation, CkProfileSiteKind,
    KirBoundsMode, KirBuildConfig, KirConsumer, KirMultiversionTargetSet, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, SourceFile, build_kir_module_with_profile, check,
    check_profile_guided_optimization, import_contract_facts, lower_to_mir, prepare_ck_profile_kir,
    project_pgo_plan_for_kir, propose_profile_guided_optimization, run_kir_pass_pipeline,
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

const HOT_COLD_INLINE_SOURCE: &str = r#"
fn hot(x: i32) -> i32 {
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
fn cold(x: i32) -> i32 { return x - 1; }
export fn kernel(x: i32, flag: i32) -> i32 {
  if flag == 7 { return hot(x); }
  return cold(x);
}
"#;

fn fixture(
    source: &str,
    observations: u64,
) -> (
    calckernel::CkProfileKirPlan,
    CkImmutableProfileAnalysis,
    Option<calckernel::ContractFactSet>,
) {
    fixture_with_profile(source, observations, &BTreeMap::new(), 7)
}

fn fixture_with_entries(
    source: &str,
    observations: u64,
    entry_overrides: &BTreeMap<&str, u64>,
) -> (
    calckernel::CkProfileKirPlan,
    CkImmutableProfileAnalysis,
    Option<calckernel::ContractFactSet>,
) {
    fixture_with_profile(source, observations, entry_overrides, 7)
}

fn fixture_with_profile(
    source: &str,
    observations: u64,
    entry_overrides: &BTreeMap<&str, u64>,
    histogram_bucket: usize,
) -> (
    calckernel::CkProfileKirPlan,
    CkImmutableProfileAnalysis,
    Option<calckernel::ContractFactSet>,
) {
    fixture_with_histograms(
        source,
        observations,
        entry_overrides,
        histogram_bucket,
        histogram_bucket,
    )
}

fn fixture_with_histograms(
    source: &str,
    observations: u64,
    entry_overrides: &BTreeMap<&str, u64>,
    loop_histogram_bucket: usize,
    slice_histogram_bucket: usize,
) -> (
    calckernel::CkProfileKirPlan,
    CkImmutableProfileAnalysis,
    Option<calckernel::ContractFactSet>,
) {
    let checked = check(&SourceFile::new("pgo.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let profile = KirMultiversionTargetSet::schema1_for_triple(
        "aarch64-apple-darwin",
        KirConsumer::NativeLibrary,
    )
    .expect("native target-set fixture")
    .tiers[0]
        .profile
        .clone();
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
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
    let function_names = plan
        .annotations
        .iter()
        .filter_map(|annotation| {
            let calckernel::CkProfileEvent::FunctionEntry { function, .. } = annotation.event
            else {
                return None;
            };
            let name = plan
                .module
                .functions
                .iter()
                .find(|candidate| candidate.id == function)?
                .name
                .clone();
            Some((annotation.descriptor.function_digest, name))
        })
        .collect::<BTreeMap<_, _>>();
    let mut function_digests = BTreeSet::new();
    let sites = plan
        .sites
        .iter()
        .cloned()
        .map(|descriptor| {
            function_digests.insert(descriptor.function_digest);
            let observation = match &descriptor.kind {
                CkProfileSiteKind::FunctionEntry => {
                    let name = function_names
                        .get(&descriptor.function_digest)
                        .expect("profile function");
                    CkProfileObservation::Scalar(
                        entry_overrides
                            .get(name.as_str())
                            .copied()
                            .unwrap_or(observations),
                    )
                }
                CkProfileSiteKind::Edge { .. } => CkProfileObservation::Scalar(observations),
                CkProfileSiteKind::LoopTripHistogram { .. } => {
                    let mut buckets = [0; 16];
                    buckets[loop_histogram_bucket] = observations;
                    CkProfileObservation::Histogram(buckets)
                }
                CkProfileSiteKind::SliceLengthHistogram { .. } => {
                    let mut buckets = [0; 16];
                    buckets[slice_histogram_bucket] = observations;
                    CkProfileObservation::Histogram(buckets)
                }
                CkProfileSiteKind::CandidateConstant { candidates, .. } => {
                    CkProfileObservation::CandidateConstant {
                        candidates: vec![observations; candidates.len()],
                        other: if entry_overrides.is_empty() {
                            observations / 20
                        } else {
                            0
                        },
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
        .map(|(index, function_digest)| {
            let entries = plan
                .sites
                .iter()
                .find(|site| {
                    site.function_digest == function_digest
                        && matches!(site.kind, CkProfileSiteKind::FunctionEntry)
                })
                .and_then(|site| {
                    let name = function_names.get(&site.function_digest)?;
                    Some(
                        entry_overrides
                            .get(name.as_str())
                            .copied()
                            .unwrap_or(observations),
                    )
                })
                .unwrap_or(observations);
            CkProfileFunctionWork {
                function_digest,
                dynamic_work: Some(u128::from(entries).saturating_mul(100)),
                rank: Some(u32::try_from(index + 1).expect("rank")),
                hot_root: entries >= 128,
            }
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
fn pgo_short_slice_length_histogram_should_retain_the_scalar_loop() {
    let source = r#"
export unsafe fn kernel(a: slice<u32>, out: slice<u32>) -> void
contract { requires a.len <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let n: u32 = a.len;
  let i: u32 = 0;
  while i < n { out[i] = a[i] + 7; i = i + 1; }
}
"#;
    let (plan, analysis, contracts) = fixture_with_histograms(source, 256, &BTreeMap::new(), 7, 2);
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

    assert!(
        ordinary.stats.vectorized_loops > 0,
        "{:?}",
        ordinary.analysis_fallbacks
    );
    assert_eq!(profiled.stats.vectorized_loops, 0);
    assert!(profiled.analysis_fallbacks.iter().any(|fallback| {
        fallback.pass == "loop-simd" && fallback.reason == "profile-short-slice-retains-scalar"
    }));
}

#[test]
fn pgo_short_trip_histogram_should_retain_the_scalar_loop() {
    let source = r#"
export unsafe fn kernel(a: slice<u32>, out: slice<u32>, n: u32) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] + 7; i = i + 1; }
}
"#;
    let (plan, analysis, contracts) = fixture_with_profile(source, 256, &BTreeMap::new(), 2);
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

    assert!(
        ordinary.stats.vectorized_loops > 0,
        "{:?}",
        ordinary.analysis_fallbacks
    );
    assert_eq!(profiled.stats.vectorized_loops, 0);
    assert!(profiled.analysis_fallbacks.iter().any(|fallback| {
        fallback.pass == "loop-simd" && fallback.reason == "profile-short-trip-retains-scalar"
    }));
}

#[test]
fn pgo_inline_should_keep_profile_cold_successor_as_the_generic_fallback() {
    let entry_overrides = BTreeMap::from([("hot", 256), ("cold", 0), ("kernel", 256)]);
    let (plan, analysis, contracts) =
        fixture_with_entries(HOT_COLD_INLINE_SOURCE, 256, &entry_overrides);
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

    let calls = |result: &calckernel::KirPassManagerResult| {
        result
            .artifact
            .as_ref()
            .expect("verified artifact")
            .functions
            .iter()
            .find(|function| function.name == "kernel")
            .expect("kernel")
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter_map(|instruction| match &instruction.kind {
                calckernel::KirInstructionKind::Call { function_name, .. } => {
                    Some(function_name.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(calls(&ordinary), ["hot".to_string()]);
    assert_eq!(calls(&profiled), ["cold".to_string()]);
}

#[test]
fn pgo_projection_should_preserve_exact_guidance_for_a_separate_module() {
    let entry_overrides = BTreeMap::from([("hot", 256), ("cold", 0), ("kernel", 256)]);
    let (plan, analysis, contracts) =
        fixture_with_entries(HOT_COLD_INLINE_SOURCE, 256, &entry_overrides);
    let profiled = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );
    let mut separate = profiled.artifact.clone().expect("profiled artifact");
    let removed = separate
        .functions
        .iter()
        .find(|function| function.name == "hot")
        .expect("inlined hot helper")
        .id;
    separate.functions.retain(|function| function.id != removed);
    let projected = project_pgo_plan_for_kir(&separate, profiled.pgo.as_ref().expect("PGO plan"))
        .expect("exact projected PGO plan");

    assert!(
        projected
            .functions
            .iter()
            .all(|profile| profile.function != removed)
    );
    assert_eq!(projected.branches.len(), 1);
    assert!(
        projected
            .decisions
            .iter()
            .any(|decision| decision.reason == "mapping-unavailable")
    );
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
