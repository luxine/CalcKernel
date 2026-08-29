#![allow(dead_code)]

use calckernel::{
    BoundsMode, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirPassManagerResult, KirSanitizerMode, OverflowMode, SourceFile, build_kir_module, check,
    import_contract_facts, lower_to_mir, run_kir_pass_pipeline,
};

pub(crate) fn optimized_module(
    source_text: &str,
    opt_level: u8,
    consumer: KirConsumer,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> KirPassManagerResult {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer,
            overflow_mode: match overflow_mode {
                OverflowMode::Unchecked => KirOverflowMode::Unchecked,
                OverflowMode::Checked => KirOverflowMode::Checked,
            },
            bounds_mode: match bounds_mode {
                BoundsMode::Unchecked => KirBoundsMode::Unchecked,
                BoundsMode::Checked => KirBoundsMode::Checked,
            },
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR construction should succeed");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("contract fact import should succeed");
    let optimized = run_kir_pass_pipeline(
        kir,
        match opt_level {
            0 => KirOptimizationLevel::O0,
            1 => KirOptimizationLevel::O1,
            2 => KirOptimizationLevel::O2,
            3 => KirOptimizationLevel::O3,
            _ => panic!("invalid optimization level {opt_level}"),
        },
        Some(&contracts),
    );
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    assert!(optimized.artifact.is_some());
    optimized
}

pub(crate) fn kir_build_error(
    source_text: &str,
    consumer: KirConsumer,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> String {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    build_kir_module(
        &mir,
        KirBuildConfig {
            consumer,
            overflow_mode: match overflow_mode {
                OverflowMode::Unchecked => KirOverflowMode::Unchecked,
                OverflowMode::Checked => KirOverflowMode::Checked,
            },
            bounds_mode: match bounds_mode {
                BoundsMode::Unchecked => KirBoundsMode::Unchecked,
                BoundsMode::Checked => KirBoundsMode::Checked,
            },
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect_err("KIR construction should reject an unsupported artifact")
    .to_string()
}

pub(crate) fn verified_artifact(result: &KirPassManagerResult) -> &calckernel::KirModule {
    result.artifact.as_ref().expect("verified KIR artifact")
}
