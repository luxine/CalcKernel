use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, SourceFile, apply_tuning_plan, build_kir_module, check,
    check_tuning_plan, enumerate_tuning_space, lower_to_mir, print_kir_module,
};

#[test]
fn tuning_site_unit_and_variant_ids_are_stable() {
    let state = state("export fn kernel(n: u32) -> u32 { return n + 1; }");
    let left = enumerate_tuning_space(&state).expect("space");
    let right = enumerate_tuning_space(&state).expect("space");

    assert_eq!(left, right);
    assert_eq!(
        hex_digest(&left.digest),
        "4481aaf04150c2f2ca15a6cebef5150c61f5eb3a2935679e8fa915e736e95764"
    );
    assert!(!left.units.is_empty());
    assert!(left.units.len() <= 64);
    assert!(left.units.iter().all(|unit| unit.variants.len() <= 4));
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn tuning_plan_checker_rejects_forged_variant_and_preserves_prestate() {
    let state = state("export fn kernel(n: u32) -> u32 { return n + 1; }");
    let space = enumerate_tuning_space(&state).expect("space");
    let plan = space.plan_for_variant(0, 0).expect("plan");
    check_tuning_plan(&state, &space, &plan).expect("checked plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("replay");
    assert_eq!(replayed.module(), state.module());

    let mut forged = plan;
    forged.choices[0].variant_id[0] ^= 1;
    assert!(check_tuning_plan(&state, &space, &forged).is_err());
}

#[test]
fn tuning_plan_checker_recomputes_the_candidate_space_from_the_prestate() {
    let state = state("export fn kernel(n: u32) -> u32 { return n + 1; }");
    let mut space = enumerate_tuning_space(&state).expect("space");
    let plan = space.plan_for_variant(0, 0).expect("plan");
    space.digest[0] ^= 1;

    assert!(check_tuning_plan(&state, &space, &plan).is_err());
}

#[test]
fn tuning_empty_plan_preserves_canonical_kir_bytes() {
    let state = state("export fn kernel(n: u32) -> u32 { return n + 1; }");
    let space = enumerate_tuning_space(&state).expect("space");
    let replayed = apply_tuning_plan(&state, &space, &calckernel::TuningPlan::baseline())
        .expect("empty replay");

    assert_eq!(
        print_kir_module(replayed.module()),
        print_kir_module(state.module())
    );
}

fn state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("tuning.ck", source));
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
