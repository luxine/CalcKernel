use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, SourceFile, TuneBudget, build_kir_module, canonical_frontier_digest,
    check, enumerate_tuning_space, lower_to_mir, prepare_kir_pre_tune_state,
    run_deterministic_search,
};

#[test]
fn frontier_digest_is_canonical_and_sensitive_to_every_expansion() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let frontier = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");
    let digest = canonical_frontier_digest(&space, &frontier);
    assert!(
        !space.units.is_empty(),
        "fixture must expose real CK alternatives"
    );
    assert_eq!(digest, canonical_frontier_digest(&space, &frontier));

    let mut changed = frontier;
    changed.expansions[0].ordinal += 1;
    assert_ne!(digest, canonical_frontier_digest(&space, &changed));
}

fn state() -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new(
        "frontier.ck",
        "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; }",
    ));
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
