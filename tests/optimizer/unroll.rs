use calckernel::{
    CandidateDisposition, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationAuditState,
    KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, KirVerifiedProgramState, SourceFile,
    TransactionOutcome, analyze_canonical_loops, build_kir_module, check,
    check_unroll_plan_independently, discover_unroll_candidates, execute_verified_transaction,
    lower_to_mir, prepare_unroll_trial, run_kir_pass_pipeline, unroll_profitability_threshold,
};

fn state(source: &str, consumer: KirConsumer) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("unroll.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    KirVerifiedProgramState::new(module, None, 0).expect("verified state")
}

fn candidates(state: &KirVerifiedProgramState) -> Vec<calckernel::UnrollCandidate> {
    let function = &state.module().functions[0];
    let loops = analyze_canonical_loops(function);
    discover_unroll_candidates(function, &loops.loops).candidates
}

#[test]
fn full_unroll_should_cover_exact_zero_through_eight_and_reject_trip_or_body_neighbors() {
    for trip in 0..=8 {
        let source = format!(
            "export fn sum() -> u32 {{ let i: u32 = 0; let total: u32 = 0; while i < {trip} {{ total = total + i; i = i + 1; }} return total; }}"
        );
        let pre = state(&source, KirConsumer::C);
        let candidates = candidates(&pre);
        assert_eq!(candidates.len(), 1, "trip={trip}: {candidates:?}");
        assert!(candidates[0].full);
        let prepared = prepare_unroll_trial(&pre, &candidates[0]).expect("full unroll proposal");
        assert_eq!(
            check_unroll_plan_independently(
                &pre,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Ok(()),
            "trip={trip}"
        );
    }

    let trip_neighbor = state(
        "export fn sum() -> u32 { let i: u32 = 0; while i < 9 { i = i + 1; } return i; }",
        KirConsumer::C,
    );
    assert!(candidates(&trip_neighbor).iter().all(|item| !item.full));

    let body_neighbor = state(
        "export fn sum() -> u32 { let i: u32 = 0; let a: u32 = 0; while i < 8 { a = a + 1; a = a + 2; a = a + 3; a = a + 4; a = a + 5; a = a + 6; a = a + 7; a = a + 8; a = a + 9; a = a + 10; a = a + 11; a = a + 12; a = a + 13; a = a + 14; a = a + 15; a = a + 16; i = i + 1; } return a; }",
        KirConsumer::C,
    );
    assert!(candidates(&body_neighbor).iter().all(|item| !item.full));
}

#[test]
fn full_unroll_pipeline_should_commit_scalar_winner_for_portable_consumers() {
    for consumer in [KirConsumer::C, KirConsumer::WebAssembly] {
        let pre = state(
            "export fn sum() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 8 { total = total + i; i = i + 1; } return total; }",
            consumer,
        );
        let result = run_kir_pass_pipeline(pre.module().clone(), KirOptimizationLevel::O3, None);
        assert!(
            result.errors.is_empty(),
            "{consumer:?}: {:?}",
            result.errors
        );
        assert_eq!(result.stats.full_unrolled_loops, 1, "{consumer:?}");
        let function = &result.artifact.as_ref().unwrap().functions[0];
        assert!(
            analyze_canonical_loops(function).loops.is_empty(),
            "{consumer:?}"
        );
        assert!(function.vector_regions.is_empty());
    }
}

#[test]
fn partial_unroll_should_materialize_factor_two_four_and_exact_remainder() {
    for (trip, factor, remainder) in [(12_u32, 2_u8, 0_u8), (12, 4, 0), (11, 2, 1), (11, 4, 3)] {
        let source = format!(
            "export fn sum() -> u32 {{ let i: u32 = 0; let total: u32 = 0; while i < {trip} {{ total = total + i; i = i + 1; }} return total; }}"
        );
        let pre = state(&source, KirConsumer::WebAssembly);
        let candidate = candidates(&pre)
            .into_iter()
            .find(|candidate| candidate.factor == factor)
            .expect("partial candidate");
        assert_eq!(candidate.remainder, remainder);
        let prepared = prepare_unroll_trial(&pre, &candidate).expect("partial unroll proposal");
        assert_eq!(
            check_unroll_plan_independently(
                &pre,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Ok(())
        );
        assert_eq!(prepared.plan.remainder, remainder);
        assert_eq!(
            prepared.plan.instruction_mapping.len(),
            usize::from(factor + remainder) * usize::try_from(candidate.body_units).unwrap()
        );
    }
}

#[test]
fn partial_unroll_should_reject_calls_guards_and_possible_failure_stop_points() {
    for source in [
        "fn step(n: u32) -> u32 { return n + 1; } export fn sum() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = step(total); i = i + 1; } return total; }",
        "export fn sum(d: u32) -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total / d; i = i + 1; } return total; }",
    ] {
        let pre = state(source, KirConsumer::C);
        assert!(candidates(&pre).is_empty(), "{source}");
    }
}

#[test]
fn unroll_should_allow_strict_f64_division_but_reject_integer_division() {
    let strict = state(
        "export fn repeated(a: f64, b: f64) -> f64 { let i: u32 = 0; let value: f64 = 0.0; while i < 2 { value = a / b; i = i + 1; } return value; }",
        KirConsumer::C,
    );
    assert!(
        candidates(&strict).iter().any(|candidate| candidate.full),
        "strict f64 division cannot fail and is legal to duplicate"
    );

    let integer = state(
        "export fn repeated(a: u32, b: u32) -> u32 { let i: u32 = 0; let value: u32 = 0; while i < 2 { value = a / b; i = i + 1; } return value; }",
        KirConsumer::C,
    );
    assert!(candidates(&integer).is_empty());
}

#[test]
fn unroll_checker_should_reject_coverage_order_remainder_cost_growth_and_budget_mutations() {
    let pre = state(
        "export fn sum() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 11 { total = total + i; i = i + 1; } return total; }",
        KirConsumer::C,
    );
    let candidate = candidates(&pre)
        .into_iter()
        .find(|candidate| candidate.factor == 4)
        .unwrap();
    let prepared = prepare_unroll_trial(&pre, &candidate).unwrap();

    let mut coverage = prepared.plan.clone();
    coverage.instruction_mapping.pop();
    assert!(
        check_unroll_plan_independently(&pre, &prepared.trial, &coverage, &prepared.charge)
            .is_err()
    );
    let mut order = prepared.plan.clone();
    order.instruction_mapping.swap(0, 1);
    assert!(
        check_unroll_plan_independently(&pre, &prepared.trial, &order, &prepared.charge).is_err()
    );
    let mut remainder = prepared.plan.clone();
    remainder.remainder = 0;
    assert!(
        check_unroll_plan_independently(&pre, &prepared.trial, &remainder, &prepared.charge)
            .is_err()
    );
    let mut cost = prepared.plan.clone();
    cost.cost.total += 1;
    assert!(
        check_unroll_plan_independently(&pre, &prepared.trial, &cost, &prepared.charge).is_err()
    );
    let mut growth = prepared.plan.clone();
    growth.growth.module_after_units += 1;
    assert!(
        check_unroll_plan_independently(&pre, &prepared.trial, &growth, &prepared.charge).is_err()
    );
    let mut charge = prepared.charge.clone();
    charge.checker_units += 1;
    assert!(
        check_unroll_plan_independently(&pre, &prepared.trial, &prepared.plan, &charge).is_err()
    );
}

#[test]
fn unroll_transaction_should_commit_complete_state_and_keep_rejection_audit_debits() {
    let mut pre = state(
        "export fn sum() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 8 { total = total + i; i = i + 1; } return total; }",
        KirConsumer::C,
    );
    let candidate = candidates(&pre).remove(0);
    let prepared = prepare_unroll_trial(&pre, &candidate).unwrap();
    let plan = prepared.plan.clone();
    let charge = prepared.charge.clone();
    let proposed = prepared.trial;
    let mut audit = KirOptimizationAuditState::for_module(pre.module());
    let outcome = execute_verified_transaction(
        &mut pre,
        &mut audit,
        candidate.key,
        charge.clone(),
        move |trial| {
            *trial = proposed;
            Ok(())
        },
        |before, after| check_unroll_plan_independently(before, after, &plan, &charge),
    );
    assert_eq!(outcome, TransactionOutcome::Committed);
    assert_eq!(
        audit.attempts()[0].disposition,
        CandidateDisposition::Accepted
    );
}

#[test]
fn unroll_checker_profitability_should_enforce_exact_ten_percent_and_two_units() {
    assert!(unroll_profitability_threshold(
        calckernel::KirCostEstimate::new(20, 18, 0, 0)
    ));
    assert!(!unroll_profitability_threshold(
        calckernel::KirCostEstimate::new(20, 19, 0, 0)
    ));
    assert!(!unroll_profitability_threshold(
        calckernel::KirCostEstimate::new(21, 19, 1, 0)
    ));
}
