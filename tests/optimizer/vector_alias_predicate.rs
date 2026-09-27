use calckernel::{
    KirAlignmentClass, KirBoundsMode, KirBuildConfig, KirConsumer, KirLegalCost,
    KirNativeCpuPolicy, KirOptimizationLevel, KirOverflowMode, KirProfileOperation,
    KirSanitizerMode, KirTargetProfile, KirTargetProfileBuilder, KirVerifiedProgramState,
    SourceFile, VectorizationCandidate, check, check_vectorization_trial_independently,
    discover_vectorization_candidates, import_contract_facts, lower_to_mir,
    prepare_vectorization_trial, run_kir_pass_pipeline,
};

const UNKNOWN_ALIAS_MAP: &str = r#"
export fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 1; i = i + 1; }
}
"#;

const THREE_SLICE_ALIAS_MAP: &str = r#"
export fn map(a: slice<u32>, b: slice<u32>, c: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n { c[i] = a[i] + b[i]; i = i + 1; }
}
"#;

fn profile() -> KirTargetProfile {
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
                && matches!(key.lanes, 2 | 4)
                && matches!(
                    key.operation,
                    KirProfileOperation::Splat
                        | KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
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
        let legalized_type = if key.lanes == 4 {
            "v4i32".to_string()
        } else {
            "v2i32".to_string()
        };
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type,
                },
            )
            .expect("legal vector operation");
    }
    builder.build().expect("native vector profile")
}

fn unknown_alias_candidate() -> (KirVerifiedProgramState, VectorizationCandidate) {
    unknown_alias_candidate_from(UNKNOWN_ALIAS_MAP)
}

fn unknown_alias_candidate_from(source: &str) -> (KirVerifiedProgramState, VectorizationCandidate) {
    let checked = check(&SourceFile::new("vector-alias-predicate.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let profile = profile();
    let module = calckernel::build_kir_module_with_profile(
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
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let state = KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("O2 artifact"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified O2 state");
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.vf == 4 && candidate.version_predicate.is_some())
        .expect("VF4 candidate with runtime alias check");
    (state, candidate)
}

fn prepared_alias_trial() -> (
    KirVerifiedProgramState,
    VectorizationCandidate,
    calckernel::PreparedVectorization,
) {
    let (pre, candidate) = unknown_alias_candidate();
    let prepared = prepare_vectorization_trial(&pre, &candidate).expect("vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "unmodified versioned trial must be independently accepted"
    );
    (pre, candidate, prepared)
}

fn prepared_three_slice_alias_trial() -> (
    KirVerifiedProgramState,
    VectorizationCandidate,
    calckernel::PreparedVectorization,
) {
    let (pre, candidate) = unknown_alias_candidate_from(THREE_SLICE_ALIAS_MAP);
    let prepared = prepare_vectorization_trial(&pre, &candidate).expect("three-slice vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "unmodified three-slice versioned trial must be independently accepted"
    );
    (pre, candidate, prepared)
}

fn version_predicate_mut(
    trial: &mut KirVerifiedProgramState,
) -> &mut calckernel::KirVersionPredicate {
    trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VersionPredicate { predicate } => Some(predicate),
            _ => None,
        })
        .expect("runtime version predicate")
}

#[test]
fn alias_checker_should_reject_preheader_branch_that_bypasses_alias_result() {
    let (pre, candidate, prepared) = prepared_alias_trial();
    let mut forged = prepared.trial.clone();
    let function = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("versioned function");
    let preheader = function
        .blocks
        .iter()
        .find(|block| block.id == candidate.preheader)
        .expect("loop preheader");
    let threshold_result = preheader
        .instructions
        .iter()
        .find_map(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Compare {
                    op: calckernel::MirCompareOp::Ge,
                    ..
                }
            )
            .then(|| instruction.results[0].value)
        })
        .expect("standalone trip-threshold comparison result");
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("mutable loop preheader");
    let calckernel::KirTerminator::Branch { condition, .. } = &mut preheader.terminator else {
        panic!("versioned preheader must branch");
    };
    *condition = threshold_result;

    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "checker accepted a preheader branch that skips the short-trip threshold"
    );
}

#[test]
fn alias_checker_should_reject_forged_trip_threshold_minimum() {
    let (pre, candidate, prepared) = prepared_alias_trial();
    let mut forged = prepared.trial.clone();
    let function = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("versioned function");
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("loop preheader");
    let threshold =
        preheader
            .instructions
            .iter_mut()
            .find_map(|instruction| match &mut instruction.kind {
                calckernel::KirInstructionKind::VersionPredicate { predicate } => predicate
                    .conjuncts
                    .iter_mut()
                    .find_map(|conjunct| match conjunct {
                        calckernel::KirVersionPredicateConjunct::TripThreshold {
                            minimum, ..
                        } => Some(minimum),
                        _ => None,
                    }),
                _ => None,
            });
    // The alias predicate currently contains no trip conjunct. Forge its boundary
    // with the scalar comparison in the preheader, which is the branch's threshold.
    if let Some(minimum) = threshold {
        *minimum = candidate.minimum_trip.saturating_sub(1);
    } else {
        let constant = preheader
            .instructions
            .iter_mut()
            .find_map(|instruction| match &mut instruction.kind {
                calckernel::KirInstructionKind::ConstInt { value }
                    if value.parse::<u32>().ok() == Some(candidate.minimum_trip) =>
                {
                    Some(value)
                }
                _ => None,
            })
            .expect("trip threshold constant");
        *constant = candidate.minimum_trip.saturating_sub(1).to_string();
    }

    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "checker accepted threshold minimum {} instead of {}",
        candidate.minimum_trip.saturating_sub(1),
        candidate.minimum_trip
    );
}

#[test]
fn alias_checker_should_reject_duplicate_alias_conjunct() {
    let (pre, _, prepared) = prepared_alias_trial();
    let mut forged = prepared.trial.clone();
    let predicate = version_predicate_mut(&mut forged);
    let alias = predicate
        .conjuncts
        .iter()
        .find(|conjunct| {
            matches!(
                conjunct,
                calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint { .. }
            )
        })
        .expect("alias conjunct")
        .clone();
    predicate.conjuncts.push(alias);

    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "checker accepted a duplicate alias conjunct"
    );
}

#[test]
fn alias_checker_should_reject_duplicate_pair_that_omits_another_required_pair() {
    let (pre, _, prepared) = prepared_three_slice_alias_trial();
    let mut forged = prepared.trial.clone();
    let predicate = version_predicate_mut(&mut forged);
    let alias_indices = predicate
        .conjuncts
        .iter()
        .enumerate()
        .filter_map(|(index, conjunct)| {
            matches!(
                conjunct,
                calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint { .. }
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    assert!(
        alias_indices.len() >= 2,
        "three-slice map must require at least two write/read alias pairs: {predicate:#?}"
    );
    let first_alias = predicate.conjuncts[alias_indices[0]].clone();
    predicate.conjuncts[alias_indices[1]] = first_alias;

    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "checker accepted duplicate pair coverage that omits another required alias pair"
    );
}

#[test]
fn alias_checker_should_reject_omitted_alias_conjunct() {
    let (pre, _, prepared) = prepared_alias_trial();
    let mut forged = prepared.trial.clone();
    version_predicate_mut(&mut forged).conjuncts.clear();

    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "checker accepted a predicate with no alias conjunct"
    );
}

#[test]
fn alias_checker_should_reject_wrong_interval_count() {
    let (pre, candidate, prepared) = prepared_alias_trial();
    let mut forged = prepared.trial.clone();
    let function = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("versioned function");
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("loop preheader");
    let wrong_count = preheader
        .instructions
        .iter()
        .find_map(|instruction| match instruction.kind {
            calckernel::KirInstructionKind::ConstInt { .. } => {
                instruction.results.first().map(|result| result.value)
            }
            _ => None,
        })
        .expect("dominating u32 constant");
    let predicate = version_predicate_mut(&mut forged);
    let Some(calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
        left_count, ..
    }) = predicate.conjuncts.iter_mut().find(|conjunct| {
        matches!(
            conjunct,
            calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint { .. }
        )
    })
    else {
        panic!("alias interval conjunct");
    };
    *left_count = wrong_count;

    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "checker accepted a runtime alias range count different from trip count {:#?}",
        candidate.version_predicate
    );
}
