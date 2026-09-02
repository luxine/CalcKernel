use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, SourceFile, TuneArtifactKind, TuneBudget, TuneTrialBuildRequest,
    build_kir_module, check, compile_tune_trial, enumerate_tuning_space, lower_to_mir,
    run_deterministic_search,
};

#[test]
fn trial_artifact_identity_is_stable_and_destination_free() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let search = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");
    let plan = &search.compile_selection[0];
    let request = TuneTrialBuildRequest::new(
        TuneArtifactKind::Dynamic,
        b"primary".to_vec(),
        Some(b"header".to_vec()),
        None,
        vec![("module.o".to_string(), b"object".to_vec())],
        vec!["lld".to_string(), "shared".to_string()],
    );

    let left = compile_tune_trial(&state, &space, plan, request.clone()).expect("trial");
    let right = compile_tune_trial(&state, &space, plan, request).expect("trial");
    assert_eq!(left.identity(), right.identity());
    assert_eq!(left.primary_size(), 7);
    assert_eq!(left.plan_digest(), plan.digest);
}

#[test]
fn trial_identity_detects_primary_object_and_recipe_mutations() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let plan = run_deterministic_search(&state, &space, TuneBudget::Quick)
        .expect("search")
        .compile_selection
        .remove(0);
    let build = |primary: &[u8], object: &[u8], recipe: &str| {
        compile_tune_trial(
            &state,
            &space,
            &plan,
            TuneTrialBuildRequest::new(
                TuneArtifactKind::Executable,
                primary.to_vec(),
                None,
                None,
                vec![("program.o".to_string(), object.to_vec())],
                vec![recipe.to_string()],
            ),
        )
        .expect("trial")
    };

    let original = build(b"primary", b"object", "recipe");
    assert_ne!(
        original.identity(),
        build(b"changed", b"object", "recipe").identity()
    );
    assert_ne!(
        original.identity(),
        build(b"primary", b"changed", "recipe").identity()
    );
    assert_ne!(
        original.identity(),
        build(b"primary", b"object", "changed").identity()
    );
}

pub(crate) fn state() -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new(
        "trial.ck",
        "export fn kernel(n: u32) -> u32 { return (n + 1) * 2; }",
    ));
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
