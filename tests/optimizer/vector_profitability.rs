use std::collections::BTreeSet;

use calckernel::{
    CkImmutableProfileAnalysis, CkProfileAnalysis, CkProfileAnalyzedSite, CkProfileContract,
    CkProfileFunctionWork, CkProfileKirMode, CkProfileObservation, CkProfileSiteKind,
    KirAlignmentClass, KirBoundsMode, KirBuildConfig, KirConsumer, KirLegalCost,
    KirNativeCpuPolicy, KirOptimizationLevel, KirOverflowMode, KirProfileOperation,
    KirSanitizerMode, KirTargetProfile, KirTargetProfileBuilder, KirVerifiedProgramState,
    SourceFile, build_kir_module_with_profile, check_vectorization_trial_independently,
    discover_vectorization_candidates, import_contract_facts, lower_to_mir, prepare_ck_profile_kir,
    prepare_vectorization_trial, run_kir_multiversion_pass_pipeline, run_kir_pass_pipeline,
    run_profile_guided_kir_multiversion_pass_pipeline, run_profile_guided_kir_pass_pipeline,
};

const EXACT_SHORT_MAP: &str = r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < 4 { b[i] = a[i] + 7; i = i + 1; }
}
"#;

const EXACT_LARGE_COST_MAP: &str = r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < 110000000 { b[i] = a[i] + 7; i = i + 1; }
}
"#;

fn exact_short_profile() -> KirTargetProfile {
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        "aarch64-apple-darwin",
        64,
        true,
        KirNativeCpuPolicy::Baseline,
        "generic",
        vec!["+neon".to_string()],
    )
    .expect("native profile builder");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lane == calckernel::KirLaneType::U32
                && key.lanes == 1
                && matches!(
                    key.operation,
                    KirProfileOperation::Add
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                )
        })
    {
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 10,
                    legalization_parts: 1,
                    legalized_type: "i32".to_string(),
                },
            )
            .expect("scalar cost");
    }
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lane == calckernel::KirLaneType::U32
                && matches!(key.lanes, 2 | 4)
                && matches!(
                    key.operation,
                    KirProfileOperation::Splat
                        | KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
                        | KirProfileOperation::Divide
                        | KirProfileOperation::Negate
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                        | KirProfileOperation::Compare
                        | KirProfileOperation::Select
                        | KirProfileOperation::Cast
                        | KirProfileOperation::Insert
                        | KirProfileOperation::Extract
                        | KirProfileOperation::RuntimePredicate
                        | KirProfileOperation::ReduceAdd
                        | KirProfileOperation::ReduceMultiply
                )
                && (!matches!(
                    key.operation,
                    KirProfileOperation::Load | KirProfileOperation::Store
                ) || key.alignment == KirAlignmentClass::Bytes(4))
        })
    {
        let legalized_type = if key.lanes == 2 { "v2i32" } else { "v4i32" };
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 13,
                    legalization_parts: 1,
                    legalized_type: legalized_type.to_string(),
                },
            )
            .expect("vector cost");
    }
    builder.set_maximum_interleave_factor(1);
    builder.build().expect("native profile")
}

fn exact_short_state() -> KirVerifiedProgramState {
    exact_state(EXACT_SHORT_MAP, "exact-short.ck")
}

fn exact_state(source: &str, name: &str) -> KirVerifiedProgramState {
    state_with_profile(source, name, exact_short_profile())
}

fn state_with_profile(
    source: &str,
    name: &str,
    profile: KirTargetProfile,
) -> KirVerifiedProgramState {
    verified_state_with_profile_level(source, name, profile, KirOptimizationLevel::O1)
}

fn frontier_state_with_profile(
    source: &str,
    name: &str,
    profile: KirTargetProfile,
) -> KirVerifiedProgramState {
    let initial = state_with_profile(source, name, profile);
    let frontier = run_kir_multiversion_pass_pipeline(
        initial.module().clone(),
        KirOptimizationLevel::O3,
        initial.contract_facts(),
    );
    assert!(frontier.errors.is_empty(), "{:?}", frontier.errors);
    KirVerifiedProgramState::from_parts(
        frontier.artifact.expect("deferred native vector artifact"),
        frontier.contract_facts,
        frontier.proofs,
        frontier.eliminated_guards,
        0,
    )
    .expect("verified O3 pre-frontier state")
}

fn verified_state_with_profile_level(
    source: &str,
    name: &str,
    profile: KirTargetProfile,
    level: KirOptimizationLevel,
) -> KirVerifiedProgramState {
    let checked = calckernel::check(&SourceFile::new(name, source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: profile.consumer(),
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).expect("facts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("verified optimized artifact"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified pre-state")
}

fn custom_native_profile(
    scalar_cost: u32,
    vector2_cost: u32,
    vector4_cost: u32,
) -> KirTargetProfile {
    custom_native_profile_for_triple(
        scalar_cost,
        vector2_cost,
        vector4_cost,
        "aarch64-apple-darwin",
    )
}

fn frontier_native_profile() -> KirTargetProfile {
    custom_native_profile_for_triple(100, 13, 13, "x86_64-unknown-linux-gnu")
}

fn custom_native_profile_for_triple(
    scalar_cost: u32,
    vector2_cost: u32,
    vector4_cost: u32,
    triple: &str,
) -> KirTargetProfile {
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        triple,
        64,
        true,
        KirNativeCpuPolicy::Baseline,
        "generic",
        vec![if triple.starts_with("x86_64-") {
            "+sse2".to_string()
        } else {
            "+neon".to_string()
        }],
    )
    .expect("native profile builder");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lane == calckernel::KirLaneType::U32
                && key.lanes == 1
                && matches!(
                    key.operation,
                    KirProfileOperation::Add
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                )
        })
    {
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: scalar_cost,
                    legalization_parts: 1,
                    legalized_type: "i32".to_string(),
                },
            )
            .expect("large scalar cost");
    }
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lane == calckernel::KirLaneType::U32
                && matches!(key.lanes, 2 | 4)
                && matches!(
                    key.operation,
                    KirProfileOperation::Splat
                        | KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
                        | KirProfileOperation::Divide
                        | KirProfileOperation::Negate
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                        | KirProfileOperation::Compare
                        | KirProfileOperation::Select
                        | KirProfileOperation::Cast
                        | KirProfileOperation::Insert
                        | KirProfileOperation::Extract
                        | KirProfileOperation::RuntimePredicate
                        | KirProfileOperation::ReduceAdd
                        | KirProfileOperation::ReduceMultiply
                )
                && (!matches!(
                    key.operation,
                    KirProfileOperation::Load | KirProfileOperation::Store
                ) || key.alignment == KirAlignmentClass::Bytes(4))
        })
    {
        let cost = if key.lanes == 2 {
            vector2_cost
        } else {
            vector4_cost
        };
        let legalized_type = if key.lanes == 2 { "v2i32" } else { "v4i32" };
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost,
                    legalization_parts: 1,
                    legalized_type: legalized_type.to_string(),
                },
            )
            .expect("legal vector cost");
    }
    builder.set_maximum_interleave_factor(4);
    builder.build().expect("large-cost native profile")
}

fn profile_guided_fixture(
    source: &str,
    name: &str,
    trip_bucket: usize,
) -> (
    KirVerifiedProgramState,
    calckernel::CkProfileKirPlan,
    CkImmutableProfileAnalysis,
    Option<calckernel::ContractFactSet>,
) {
    let checked = calckernel::check(&SourceFile::new(name, source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let profile = frontier_native_profile();
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: profile.consumer(),
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).expect("facts");
    let prepared = run_kir_pass_pipeline(module, KirOptimizationLevel::O1, Some(&contracts));
    assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
    let artifact = prepared.artifact.as_ref().expect("O1 artifact");
    let profile_plan = prepare_ck_profile_kir(artifact, CkProfileKirMode::Use).expect("PGO plan");
    let sites = profile_plan
        .sites
        .iter()
        .cloned()
        .map(|descriptor| {
            let observation = match &descriptor.kind {
                CkProfileSiteKind::LoopTripHistogram { .. } => {
                    let mut buckets = [0; 16];
                    buckets[trip_bucket] = 256;
                    CkProfileObservation::Histogram(buckets)
                }
                CkProfileSiteKind::SliceLengthHistogram { .. } => {
                    let mut buckets = [0; 16];
                    buckets[trip_bucket] = 256;
                    CkProfileObservation::Histogram(buckets)
                }
                CkProfileSiteKind::CandidateConstant { candidates, .. } => {
                    CkProfileObservation::CandidateConstant {
                        candidates: vec![256; candidates.len()],
                        other: 0,
                    }
                }
                CkProfileSiteKind::FunctionEntry | CkProfileSiteKind::Edge { .. } => {
                    CkProfileObservation::Scalar(256)
                }
            };
            CkProfileAnalyzedSite {
                descriptor,
                observation,
            }
        })
        .collect();
    let function_digests = profile_plan
        .sites
        .iter()
        .map(|site| site.function_digest)
        .collect::<BTreeSet<_>>();
    let functions = function_digests
        .into_iter()
        .enumerate()
        .map(|(index, function_digest)| CkProfileFunctionWork {
            function_digest,
            dynamic_work: Some(25_600),
            rank: Some(u32::try_from(index + 1).expect("function rank")),
            hot_root: true,
        })
        .collect();
    let analysis = CkImmutableProfileAnalysis::new(CkProfileAnalysis {
        identity_digest: [0x5a; 32],
        sites,
        functions,
    });
    let frontier = run_profile_guided_kir_multiversion_pass_pipeline(
        &profile_plan,
        &analysis,
        &CkProfileContract::schema1(),
        Some(&contracts),
    );
    assert!(frontier.errors.is_empty(), "{:?}", frontier.errors);
    let state = KirVerifiedProgramState::from_parts(
        frontier.artifact.expect("deferred PGO frontier artifact"),
        frontier.contract_facts,
        frontier.proofs,
        frontier.eliminated_guards,
        0,
    )
    .expect("verified PGO O3 pre-frontier state");
    (state, profile_plan, analysis, prepared.contract_facts)
}

fn loop_source(function: &str, arguments: &str, trip: &str, contract: &str, body: &str) -> String {
    let declaration = if contract.trim().is_empty() {
        "export fn"
    } else {
        "export unsafe fn"
    };
    format!(
        "{declaration} {function}({arguments}) -> void\n{contract}\n{{\n  let i: u32 = 0;\n  while i < {trip} {{ {body}; i = i + 1; }}\n}}\n"
    )
}

fn unknown_frontier_scenarios(candidates: &[calckernel::VectorizationCandidate]) -> Vec<u32> {
    let threshold = candidates
        .iter()
        .map(|candidate| candidate.minimum_trip)
        .min()
        .expect("non-empty frontier");
    let width = candidates
        .iter()
        .map(|candidate| u32::from(candidate.vf) * u32::from(candidate.uf))
        .max()
        .expect("non-empty frontier")
        .clamp(1, 64);
    (threshold.saturating_sub(1)..=threshold.saturating_add(width - 1)).collect()
}

fn bounded_endpoint_scenarios(minimum: u32, maximum: u32, width: u32) -> Vec<u32> {
    if minimum >= maximum || maximum.saturating_sub(minimum) < width {
        return (minimum..=maximum).collect();
    }
    let mut scenarios = BTreeSet::new();
    for trip in minimum..minimum.saturating_add(width) {
        scenarios.insert(trip);
    }
    for trip in maximum.saturating_sub(width - 1)..=maximum {
        scenarios.insert(trip);
    }
    scenarios.into_iter().collect()
}

fn accepted_choice(result: &calckernel::KirPassManagerResult) -> (u16, u8) {
    let accepted = result
        .vector_explanations
        .iter()
        .find(|explanation| explanation.disposition == calckernel::CandidateDisposition::Accepted)
        .expect("accepted vector frontier winner");
    (accepted.vf, accepted.uf)
}

fn assert_frontier_matches_reference(
    name: &str,
    state: &KirVerifiedProgramState,
    candidates: &[calckernel::VectorizationCandidate],
    scenarios: &[u32],
    exact_trip: Option<u32>,
    guarded: bool,
) -> ((u16, u8), String) {
    assert!(!candidates.is_empty(), "{name}: {candidates:#?}");
    let expected = super::vectorize::reference_vector_frontier_winner_for_scenarios(
        state, candidates, exact_trip, scenarios,
    );
    let result = run_kir_pass_pipeline(
        state.module().clone(),
        KirOptimizationLevel::O3,
        state.contract_facts(),
    );
    assert!(result.errors.is_empty(), "{name}: {:?}", result.errors);
    assert_eq!(
        accepted_choice(&result),
        expected,
        "{name}: pipeline winner must follow the independently recomputed finite scenario score; candidates={candidates:#?}; explanations={:#?}",
        result.vector_explanations
    );
    let accepted = result
        .vector_explanations
        .iter()
        .find(|explanation| explanation.disposition == calckernel::CandidateDisposition::Accepted)
        .expect("accepted vector frontier winner");
    let artifact = result
        .artifact
        .as_ref()
        .expect("verified optimized artifact");
    let text = calckernel::print_kir_module(artifact);
    assert_eq!(
        text.contains("version_predicate"),
        guarded,
        "{name}: runtime alias legality must remain independent of trip profitability"
    );
    if guarded {
        assert!(
            !accepted.predicates.is_empty(),
            "{name}: accepted guarded candidate must expose its runtime predicate"
        );
    }
    ((accepted.vf, accepted.uf), accepted.profitability.clone())
}

fn profitability_matrix_cases() -> Vec<(&'static str, String, Option<u32>, bool)> {
    let noalias_map = "contract { requires noalias(a, b); effects read(a), write(b); }";
    vec![
        (
            "exact-divisible",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>",
                "64",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            Some(64),
            false,
        ),
        (
            "exact-small-tail",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>",
                "17",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            Some(17),
            false,
        ),
        (
            "exact-large-tail",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>",
                "31",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            Some(31),
            false,
        ),
        (
            "very-short",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>",
                "3",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            Some(3),
            false,
        ),
        (
            "large-trip",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>",
                "256",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            Some(256),
            false,
        ),
        (
            "unknown-trip",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, n: u32",
                "n",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            None,
            false,
        ),
        (
            "guarded-alias",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, n: u32",
                "n",
                "",
                "b[i] = a[i] + 7",
            ),
            None,
            true,
        ),
        (
            "no-runtime-guard",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, n: u32",
                "n",
                noalias_map,
                "b[i] = a[i] + 7",
            ),
            None,
            false,
        ),
        (
            "small-body",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>",
                "64",
                noalias_map,
                "b[i] = a[i] + 1",
            ),
            Some(64),
            false,
        ),
        (
            "large-body",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, x: u32, y: u32, z: u32",
                "64",
                noalias_map,
                "let value: u32 = a[i] + x; value = value + y; value = value + z; b[i] = value",
            ),
            Some(64),
            false,
        ),
        (
            "broadcast-heavy",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, bias0: u32, bias1: u32, bias2: u32",
                "64",
                "contract { requires noalias(a, b); effects read(a), write(b); }",
                "b[i] = a[i] + bias0 + bias1 + bias2",
            ),
            Some(64),
            false,
        ),
        (
            "memory-heavy",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, c: slice<u32>, d: slice<u32>",
                "64",
                "contract { requires noalias(a, b) && noalias(c, b) && noalias(d, b); effects read(a), read(c), read(d), write(b); }",
                "b[i] = a[i] + c[i] + d[i]",
            ),
            Some(64),
            false,
        ),
        (
            "compute-heavy",
            loop_source(
                "map",
                "a: slice<u32>, b: slice<u32>, x0: u32, x1: u32, x2: u32, x3: u32, x4: u32, x5: u32, x6: u32, x7: u32",
                "64",
                noalias_map,
                "let value: u32 = a[i] + x0; value = value + x1; value = value + x2; value = value + x3; value = value + x4; value = value + x5; value = value + x6; value = value + x7; b[i] = value",
            ),
            Some(64),
            false,
        ),
    ]
}

#[test]
fn generalized_profitability_matrix_should_match_independent_scenario_winners() {
    for (name, source, exact_trip, guarded) in profitability_matrix_cases() {
        let state = frontier_state_with_profile(&source, name, frontier_native_profile());
        let discovery = discover_vectorization_candidates(&state);
        if name == "very-short" {
            assert!(
                discovery.candidates.is_empty(),
                "{name}: a three-trip exact loop cannot meet the target group floor: {discovery:#?}"
            );
            let result = run_kir_pass_pipeline(
                state.module().clone(),
                KirOptimizationLevel::O3,
                state.contract_facts(),
            );
            assert!(result.errors.is_empty(), "{name}: {:?}", result.errors);
            assert!(
                result.artifact.is_some(),
                "{name}: scalar fallback artifact"
            );
            continue;
        }
        let candidates = discovery.candidates;
        let shape_candidate = candidates
            .iter()
            .max_by_key(|candidate| u32::from(candidate.vf) * u32::from(candidate.uf))
            .expect("non-empty candidate frontier");
        match name {
            "small-body" => assert_eq!(shape_candidate.operations.len(), 1),
            "large-body" => assert!(shape_candidate.operations.len() >= 3),
            "broadcast-heavy" => assert!(shape_candidate.operations.len() >= 3),
            "memory-heavy" => assert!(shape_candidate.accesses.len() >= 4),
            "compute-heavy" => assert!(shape_candidate.operations.len() >= 8),
            _ => {}
        }
        if name == "broadcast-heavy" {
            let prepared = candidates
                .iter()
                .find_map(|candidate| {
                    prepare_vectorization_trial(&state, candidate)
                        .ok()
                        .filter(|prepared| {
                            !prepared.plan.operations.is_empty()
                                && !prepared.plan.memory_groups.is_empty()
                        })
                })
                .expect("at least one broadcast-heavy candidate materializes");
            assert_eq!(
                check_vectorization_trial_independently(
                    &state,
                    &prepared.trial,
                    &prepared.plan,
                    &prepared.charge,
                ),
                Ok(()),
                "broadcast-heavy plan must remain independently verified"
            );
            let mapped = calckernel::print_kir_module(prepared.trial.module());
            assert!(
                mapped.contains("vector_splat"),
                "three distinct scalar broadcasts must survive into the vector plan: {mapped}"
            );
        }
        let scenarios = exact_trip
            .map(|iterations| vec![iterations])
            .unwrap_or_else(|| unknown_frontier_scenarios(&candidates));
        let (_, explanation) = assert_frontier_matches_reference(
            name,
            &state,
            &candidates,
            &scenarios,
            exact_trip,
            guarded,
        );
        if let Some(iterations) = exact_trip {
            assert!(
                explanation.contains(&format!("trip=exact:{iterations};scenarios=1")),
                "{name}: exact scenarios must use only the original KIR trip count: {explanation}"
            );
            if matches!(name, "exact-small-tail" | "exact-large-tail") {
                let maximum_width = candidates
                    .iter()
                    .map(|candidate| u32::from(candidate.vf) * u32::from(candidate.uf))
                    .max()
                    .expect("candidate width");
                assert!(
                    candidates.iter().any(|candidate| {
                        u32::from(candidate.vf) * u32::from(candidate.uf) == maximum_width
                    }) && iterations % maximum_width
                        == if name == "exact-small-tail" { 1 } else { 15 },
                    "{name}: the matrix input must expose the intended tail against the widest legal candidate; width={maximum_width}, trip={iterations}"
                );
            }
        } else {
            assert!(
                explanation.contains(&format!("trip=unknown;scenarios={}", scenarios.len())),
                "{name}: unknown trip explanation must identify the common finite scenario set: {explanation}"
            );
        }
    }
}

#[test]
fn profitability_winner_should_ignore_source_renaming_and_unrelated_function_order() {
    let base = loop_source(
        "map",
        "a: slice<u32>, b: slice<u32>, n: u32",
        "n",
        "contract { requires noalias(a, b); effects read(a), write(b); }",
        "b[i] = a[i] + 7",
    );
    let renamed = r#"
export unsafe fn transform(source: slice<u32>, destination: slice<u32>, limit: u32) -> void
contract { requires noalias(source, destination); effects read(source), write(destination); }
{
  let cursor: u32 = 0;
  while cursor < limit { destination[cursor] = source[cursor] + 7; cursor = cursor + 1; }
}
"#;
    let reordered =
        format!("export fn unrelated(value: u32) -> u32 {{ return value * 3; }}\n{renamed}");
    let mut outcomes = Vec::new();
    for (name, source) in [
        ("base-source-shape", base.as_str()),
        ("renamed-reordered-source-shape", reordered.as_str()),
    ] {
        let state = frontier_state_with_profile(source, name, frontier_native_profile());
        let candidates = discover_vectorization_candidates(&state).candidates;
        let scenarios = unknown_frontier_scenarios(&candidates);
        outcomes.push(assert_frontier_matches_reference(
            name,
            &state,
            &candidates,
            &scenarios,
            None,
            false,
        ));
    }
    assert_eq!(
        outcomes[0], outcomes[1],
        "renaming and an unrelated function must preserve the scored decision"
    );
}

fn clamped_bound_source() -> &'static str {
    r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let limit: u32 = 16;
  if n == 7 { limit = 31; }
  let i: u32 = 0;
  while i < limit { b[i] = a[i] + 7; i = i + 1; }
}
"#
}

fn runtime_profile_source(guarded: bool) -> String {
    let contract = if guarded {
        ""
    } else {
        "contract { requires noalias(a, b); effects read(a), write(b); }"
    };
    loop_source(
        "map",
        "a: slice<u32>, b: slice<u32>, n: u32",
        "n",
        contract,
        "b[i] = a[i] + 7",
    )
}

#[test]
fn proven_bounded_trip_should_keep_its_proven_endpoints_ahead_of_profile() {
    for (bucket, expected_profile_range) in [(2, (2, 2)), (8, (65, 128))] {
        let (state, profile_plan, analysis, contracts) =
            profile_guided_fixture(clamped_bound_source(), "proven-profile.ck", bucket);
        let candidates = discover_vectorization_candidates(&state).candidates;
        let width = candidates
            .iter()
            .map(|candidate| u32::from(candidate.vf) * u32::from(candidate.uf))
            .max()
            .expect("bounded vector frontier")
            .clamp(1, 64);
        let scenarios = bounded_endpoint_scenarios(16, 31, width);
        let result = run_profile_guided_kir_pass_pipeline(
            &profile_plan,
            &analysis,
            &CkProfileContract::schema1(),
            contracts.as_ref(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let hint = result
            .pgo
            .as_ref()
            .and_then(|pgo| pgo.loop_hints.first())
            .expect("conflicting checked profile hint");
        assert_eq!(
            (hint.minimum_trip, hint.maximum_trip),
            expected_profile_range
        );
        let accepted = result
            .vector_explanations
            .iter()
            .find(|explanation| {
                explanation.disposition == calckernel::CandidateDisposition::Accepted
            })
            .expect("bounded candidate accepted");
        assert!(
            accepted
                .profitability
                .contains("trip=proven-bounded:16..31;scenarios=16"),
            "the non-top scalar interval must retain its source range ahead of PGO bucket {bucket}: {}",
            accepted.profitability
        );
        let expected = super::vectorize::reference_vector_frontier_winner_for_scenarios(
            &state,
            &candidates,
            None,
            &scenarios,
        );
        assert_eq!(
            (accepted.vf, accepted.uf),
            expected,
            "proven bounded endpoint ranking must match the independent score: {:#?}",
            result.vector_explanations
        );
    }
}

#[test]
fn profile_estimate_should_price_bounded_endpoints_without_authorizing_alias_legality() {
    let source = runtime_profile_source(true);
    let (state, profile_plan, analysis, contracts) =
        profile_guided_fixture(&source, "profile-guard.ck", 8);
    let candidates = discover_vectorization_candidates(&state).candidates;
    let width = candidates
        .iter()
        .map(|candidate| u32::from(candidate.vf) * u32::from(candidate.uf))
        .max()
        .expect("profile vector frontier")
        .clamp(1, 64);
    let scenarios = bounded_endpoint_scenarios(65, 128, width);
    assert_eq!(scenarios.len(), 32, "two complete endpoint residue windows");
    let result = run_profile_guided_kir_pass_pipeline(
        &profile_plan,
        &analysis,
        &CkProfileContract::schema1(),
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let accepted = result
        .vector_explanations
        .iter()
        .find(|explanation| explanation.disposition == calckernel::CandidateDisposition::Accepted)
        .expect("profile-estimated guarded candidate");
    assert!(
        accepted
            .profitability
            .contains("trip=profile-estimated:65..128;scenarios=32"),
        "checked profile guidance must remain typed and use only its bounded endpoint window: {}",
        accepted.profitability
    );
    assert!(
        !accepted.predicates.is_empty(),
        "profile must not remove the runtime alias predicate"
    );
    let artifact = result.artifact.as_ref().expect("verified profile artifact");
    assert!(
        calckernel::print_kir_module(artifact).contains("version_predicate"),
        "profile trip data cannot establish non-aliasing"
    );
    let expected = super::vectorize::reference_vector_frontier_winner_for_scenarios(
        &state,
        &candidates,
        None,
        &scenarios,
    );
    assert_eq!(
        (accepted.vf, accepted.uf),
        expected,
        "profile endpoint ranking must match the independent score: {:#?}",
        result.vector_explanations
    );
}

#[test]
fn exact_trip_should_override_a_conflicting_profile_estimate() {
    let source = loop_source(
        "map",
        "a: slice<u32>, b: slice<u32>",
        "16",
        "contract { requires noalias(a, b); effects read(a), write(b); }",
        "b[i] = a[i] + 7",
    );
    for (bucket, expected_profile_range) in [(2, (2, 2)), (8, (65, 128))] {
        let (state, profile_plan, analysis, contracts) =
            profile_guided_fixture(&source, "exact-profile-conflict.ck", bucket);
        let candidates = discover_vectorization_candidates(&state).candidates;
        let result = run_profile_guided_kir_pass_pipeline(
            &profile_plan,
            &analysis,
            &CkProfileContract::schema1(),
            contracts.as_ref(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let hint = result
            .pgo
            .as_ref()
            .and_then(|pgo| pgo.loop_hints.first())
            .expect("conflicting checked profile hint");
        assert_eq!(
            (hint.minimum_trip, hint.maximum_trip),
            expected_profile_range
        );
        let accepted = result
            .vector_explanations
            .iter()
            .find(|explanation| {
                explanation.disposition == calckernel::CandidateDisposition::Accepted
            })
            .expect("exact candidate accepted");
        assert!(
            accepted.profitability.contains("trip=exact:16;scenarios=1"),
            "exact original-KIR trip count must dominate conflicting PGO bucket {bucket}: {}",
            accepted.profitability
        );
        let expected = super::vectorize::reference_vector_frontier_winner_for_scenarios(
            &state,
            &candidates,
            Some(16),
            &[16],
        );
        assert_eq!((accepted.vf, accepted.uf), expected);
    }
}

#[test]
fn exact_divisible_trip_should_be_priced_without_the_runtime_worst_tail() {
    let state = exact_short_state();
    let discovery = discover_vectorization_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 2 && candidate.uf == 1)
        .unwrap_or_else(|| {
            panic!(
                "exact four-iteration loop should have a profitable VF2 candidate: {discovery:#?}"
            )
        });
    assert_eq!(
        candidate.minimum_trip, 4,
        "exact N must replace the runtime threshold"
    );
    assert_eq!(
        candidate.predicted_cost.epilogue, 0,
        "divisible exact N has no scalar tail"
    );

    let prepared = prepare_vectorization_trial(&state, candidate).expect("exact vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let mut forged_plan = prepared.plan.clone();
    forged_plan.cost.epilogue = forged_plan.cost.epilogue.saturating_add(1);
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &forged_plan,
            &prepared.charge,
        )
        .is_err(),
        "independent pricing must reject a forged exact-trip epilogue cost"
    );
}

#[test]
fn exact_trip_cost_overflow_should_fall_back_without_saturating_a_win() {
    let state = exact_state(EXACT_LARGE_COST_MAP, "exact-large-cost.ck");
    let discovery = discover_vectorization_candidates(&state);
    assert!(
        discovery.candidates.is_empty(),
        "an exact trip whose reported cost cannot fit the closed u32 cost schema must not be admitted: {discovery:#?}"
    );
    assert!(
        discovery
            .cost_failures
            .iter()
            .any(|failure| failure.reason == "target-cost"),
        "overflow must be retained as a candidate-keyed conservative cost failure: {discovery:#?}"
    );
}

#[test]
fn runtime_trip_cost_overflow_should_fail_closed_before_frontier_pricing() {
    let source = EXACT_SHORT_MAP
        .replace("b: slice<u32>", "b: slice<u32>, n: u32")
        .replace("i < 4", "i < n");
    let state = state_with_profile(
        &source,
        "runtime-cost-overflow.ck",
        custom_native_profile(300_000_000, 13, 13),
    );
    let discovery = discover_vectorization_candidates(&state);
    assert!(
        discovery.candidates.is_empty(),
        "runtime prices outside the u32 schema cannot enter the optimizer frontier: {discovery:#?}"
    );
    assert!(
        discovery.cost_failures.iter().any(|failure| {
            matches!(
                failure.key,
                calckernel::CandidateKey::LoopFrontier { vf: 2, uf: 1, .. }
            ) && failure.reason == "target-cost"
        }),
        "overflow must retain the failing candidate identity and target-cost reason: {discovery:#?}"
    );
    let optimized = run_kir_pass_pipeline(
        state.module().clone(),
        KirOptimizationLevel::O3,
        state.contract_facts(),
    );
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    assert!(
        optimized.artifact.is_some(),
        "scalar fallback remains valid"
    );
    assert!(optimized.audit.attempts().iter().any(|attempt| {
        attempt.disposition == calckernel::CandidateDisposition::Rejected
            && attempt.reason == "target-cost"
            && matches!(
                attempt.key,
                calckernel::CandidateKey::LoopFrontier { vf: 2, uf: 1, .. }
            )
    }));
}

#[test]
fn runtime_chunk_failure_is_reported_as_insufficient_benefit_before_tail_search() {
    let source = EXACT_SHORT_MAP
        .replace("b: slice<u32>", "b: slice<u32>, n: u32")
        .replace("i < 4", "i < n");
    let state = state_with_profile(
        &source,
        "runtime-chunk-benefit.ck",
        custom_native_profile(10, 30, 13),
    );
    let discovery = discover_vectorization_candidates(&state);
    assert!(
        discovery.cost_failures.iter().any(|failure| {
            matches!(
                failure.key,
                calckernel::CandidateKey::LoopFrontier { vf: 2, uf: 1, .. }
            ) && failure.reason == "insufficient-benefit"
        }),
        "a chunk that cannot meet the 20% contract is not a tail-search failure: {discovery:#?}"
    );
    assert!(!discovery.cost_failures.iter().any(|failure| {
        matches!(
            failure.key,
            calckernel::CandidateKey::LoopFrontier { vf: 2, uf: 1, .. }
        ) && failure.reason == "tail-cost"
    }));
}
