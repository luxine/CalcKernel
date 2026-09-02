use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirVerifiedProgramState, SourceFile, apply_tuning_plan, build_kir_module,
    check, check_tuning_plan, enumerate_tuning_space, lower_to_mir, prepare_kir_pre_tune_state,
    print_kir_module, run_kir_pass_pipeline,
};

const TUNABLE_SOURCE: &str = "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; }";

#[test]
fn tuning_site_unit_and_variant_ids_are_stable() {
    let state = state(TUNABLE_SOURCE);
    let left = enumerate_tuning_space(&state).expect("space");
    let right = enumerate_tuning_space(&state).expect("space");

    assert_eq!(left, right);
    assert_ne!(left.digest, [0; 32]);
    assert!(!left.units.is_empty());
    assert!(left.units.len() <= 64);
    assert!(left.units.iter().all(|unit| unit.variants.len() <= 4));
}

#[test]
fn tuning_plan_checker_rejects_forged_variant_and_preserves_prestate() {
    let state = state(TUNABLE_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let plan = space
        .plan_for_variant(&state, 0, 0)
        .expect("derive plan")
        .expect("plan");
    check_tuning_plan(&state, &space, &plan).expect("checked plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("replay");
    assert_ne!(replayed.kir_digest(), state.kir_digest());

    let mut forged = plan;
    forged.choices[0].variant_id[0] ^= 1;
    assert!(check_tuning_plan(&state, &space, &forged).is_err());
}

#[test]
fn tuning_plan_checker_recomputes_the_candidate_space_from_the_prestate() {
    let state = state(TUNABLE_SOURCE);
    let mut space = enumerate_tuning_space(&state).expect("space");
    let plan = space
        .plan_for_variant(&state, 0, 0)
        .expect("derive plan")
        .expect("plan");
    space.digest[0] ^= 1;

    assert!(check_tuning_plan(&state, &space, &plan).is_err());
}

#[test]
fn tuning_empty_plan_preserves_canonical_kir_bytes() {
    let raw = raw_module(TUNABLE_SOURCE);
    let state = prepare_kir_pre_tune_state(raw.clone(), None).expect("pre-tune");
    let space = enumerate_tuning_space(&state).expect("space");
    let replayed = apply_tuning_plan(&state, &space, &calckernel::TuningPlan::baseline())
        .expect("empty replay");
    let ordinary = run_kir_pass_pipeline(raw, KirOptimizationLevel::O3, None);
    assert!(ordinary.errors.is_empty(), "{:?}", ordinary.errors);

    assert_eq!(
        print_kir_module(replayed.module()),
        print_kir_module(ordinary.artifact.as_ref().expect("ordinary O3 artifact"))
    );
}

fn state(source: &str) -> KirVerifiedProgramState {
    prepare_kir_pre_tune_state(raw_module(source), None).expect("verified pre-tune")
}

fn raw_module(source: &str) -> calckernel::KirModule {
    let checked = check(&SourceFile::new("tuning.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR")
}
