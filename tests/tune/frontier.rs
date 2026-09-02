use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, SourceFile, TuneBudget, build_kir_module, canonical_frontier_digest,
    check, enumerate_tuning_space, lower_to_mir, run_deterministic_search,
};

#[test]
fn frontier_digest_is_canonical_and_sensitive_to_every_expansion() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let frontier = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");
    let digest = canonical_frontier_digest(&frontier);
    assert_eq!(
        hex_digest(&digest),
        "00973591402a63454e0683c1c1402801dc2fc9c0bf236a6fb0c89556ae2acf04"
    );
    assert_eq!(digest, canonical_frontier_digest(&frontier));

    let mut changed = frontier;
    changed.expansions[0].ordinal += 1;
    assert_ne!(digest, canonical_frontier_digest(&changed));
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn state() -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new(
        "frontier.ck",
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
