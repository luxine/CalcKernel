use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, SourceFile, TuneBudget, build_kir_module, check,
    enumerate_tuning_space, lower_to_mir, run_deterministic_search,
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
    let classes = search
        .compile_selection
        .iter()
        .filter_map(|plan| plan.choices.last().map(|choice| choice.class))
        .collect::<std::collections::BTreeSet<_>>();

    assert!(classes.len() >= 4);
}

fn state() -> KirVerifiedProgramState {
    let source = "export fn kernel(n: u32) -> u32 { let x: u32 = n + 1; return x * 2; }";
    let checked = check(&SourceFile::new("search.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    KirVerifiedProgramState::new(module, None, 0).expect("verified")
}
