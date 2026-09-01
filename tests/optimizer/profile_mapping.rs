use calckernel::{
    CkProfileKirMode, KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode,
    KirSanitizerMode, ProofArena, SourceFile, build_kir_module, check, lower_to_mir,
    prepare_ck_profile_kir, validate_profile_mapping_for_optimizer,
};

#[test]
fn profile_mapping_optimizer_verifier_should_not_import_annotations_into_proofs() {
    let checked = check(&SourceFile::new(
        "profile-mapping.ck",
        "export fn kernel(items: slice<i32>, n: u32) -> u32 { let i: u32 = 0; while i < n { if i == 4 { break; } i = i + 1; } return items.len; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let plan = prepare_ck_profile_kir(&module, CkProfileKirMode::Use).expect("profile use mapping");
    let proofs = ProofArena::new(0);
    let before = proofs.clone();

    assert_eq!(
        validate_profile_mapping_for_optimizer(&plan, &proofs),
        Ok(())
    );
    assert_eq!(proofs, before);
    assert!(proofs.proofs().is_empty());
}

#[test]
fn profile_mapping_optimizer_verifier_should_withhold_stale_cfg() {
    let checked = check(&SourceFile::new(
        "profile-mapping.ck",
        "export fn kernel(n: u32) -> u32 { if n == 0 { return 1; } return n; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let mut plan =
        prepare_ck_profile_kir(&module, CkProfileKirMode::Use).expect("profile use mapping");
    plan.module.functions[0].blocks.swap(0, 1);

    assert!(validate_profile_mapping_for_optimizer(&plan, &ProofArena::new(0)).is_err());
}
