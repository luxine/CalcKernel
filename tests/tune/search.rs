use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, SourceFile, TuneBudget, apply_tuning_plan, build_kir_module, check,
    enumerate_tuning_space, lower_to_mir, prepare_kir_pre_tune_state, run_deterministic_search,
};

#[test]
fn search_expansion_ordinals_and_bounds_are_deterministic() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let left = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");
    let right = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");

    assert_eq!(left, right);
    assert_eq!(
        left.expansions
            .iter()
            .map(|expansion| expansion.ordinal)
            .collect::<Vec<_>>(),
        (0..u32::try_from(left.expansions.len()).expect("bounded expansions")).collect::<Vec<_>>()
    );
    assert!(left.frontier.len() <= 4);
    assert!(left.compile_selection.len() <= 8);
}

#[test]
fn search_diversity_retains_distinct_alternative_classes() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let search = run_deterministic_search(&state, &space, TuneBudget::Standard).expect("search");
    assert!(!search.compile_selection.is_empty());
    let baseline = apply_tuning_plan(&state, &space, &calckernel::TuningPlan::baseline())
        .expect("ordinary baseline replay");
    assert!(
        search.compile_selection.iter().any(|plan| {
            apply_tuning_plan(&state, &space, plan)
                .is_ok_and(|candidate| candidate.kir_digest() != baseline.kir_digest())
        }),
        "at least one real alternative must change final KIR"
    );
}

fn state() -> KirVerifiedProgramState {
    let source = "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; }";
    let checked = check(&SourceFile::new("search.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::C,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    prepare_kir_pre_tune_state(module, None).expect("verified pre-tune state")
}
