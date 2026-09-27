use calckernel::{
    CkProfileEffectDomain, CkProfileEvent, CkProfileKirMode, CkProfileSiteKind, KirBoundsMode,
    KirBuildConfig, KirConsumer, KirNativeCpuPolicy, KirOverflowMode, KirSanitizerMode,
    KirTargetProfileBuilder, SourceFile, build_kir_module, build_kir_module_with_profile, check,
    lower_to_mir, prepare_ck_profile_kir, print_ck_profile_kir_plan, print_kir_module,
    validate_ck_profile_kir_plan,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const PROFILE_SOURCE: &str = r#"
export fn kernel(items: slice<i32>, n: u32) -> i32 {
  let i: u32 = 0;
  let total: i32 = 0;
  let length: u32 = items.len;
  while i < n {
    if i == 4 { break; }
    if i == 2 { i = i + 1; continue; }
    total = total + items[i];
    i = i + 1;
  }
  if length == 0 { return total; }
  return total + 1;
}
"#;

fn build(source: &str, consumer: KirConsumer) -> calckernel::KirModule {
    let checked = check(&SourceFile::new("profile-kir.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let config = KirBuildConfig {
        consumer,
        overflow_mode: KirOverflowMode::Unchecked,
        bounds_mode: KirBoundsMode::Unchecked,
        sanitizer_mode: KirSanitizerMode::Disabled,
    };
    if matches!(
        consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        let profile = KirTargetProfileBuilder::native(
            consumer,
            "x86_64-unknown-linux-gnu",
            64,
            true,
            KirNativeCpuPolicy::Baseline,
            "baseline-unqueried",
            Vec::new(),
        )
        .expect("deterministic native target profile builder")
        .build()
        .expect("deterministic native target profile");
        build_kir_module_with_profile(&mir, config, profile).expect("native KIR construction")
    } else {
        build_kir_module(&mir, config).expect("KIR construction")
    }
}

#[test]
fn profile_kir_schema3_should_print_closed_sites_effects_and_mapping() {
    let module = build(PROFILE_SOURCE, KirConsumer::NativeLibrary);
    let plan = prepare_ck_profile_kir(&module, CkProfileKirMode::Generate)
        .expect("profile generation plan");
    let text = print_ck_profile_kir_plan(&plan);

    assert!(print_kir_module(&module).starts_with("kir-v3 "));
    assert!(text.starts_with("ck-profile-kir-v1 mode=generate "));
    assert!(text.contains("effect=workload-profile"));
    assert!(text.contains("mapping one-to-one"));
    assert_eq!(validate_ck_profile_kir_plan(&plan), Ok(()));
    assert_no_critical_edges(&plan.module);
    assert!(!plan.sites.is_empty());
    assert_eq!(plan.annotations.len(), plan.sites.len());
    assert_eq!(plan.operations.len(), plan.sites.len());
    assert!(
        plan.operations
            .iter()
            .all(|operation| operation.effect.domain == CkProfileEffectDomain::WorkloadProfile)
    );
    assert!(plan.annotations.iter().any(|annotation| {
        matches!(
            &annotation.event,
            CkProfileEvent::LoopTrip { latches, exits, .. }
                if !latches.is_empty() && !exits.is_empty()
        )
    }));

    let kinds = plan.sites.iter().map(|site| &site.kind).collect::<Vec<_>>();
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, CkProfileSiteKind::FunctionEntry))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, CkProfileSiteKind::Edge { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, CkProfileSiteKind::LoopTripHistogram { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, CkProfileSiteKind::SliceLengthHistogram { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, CkProfileSiteKind::CandidateConstant { .. }))
    );
}

#[test]
fn profile_kir_generate_and_use_should_share_topology_without_use_writes() {
    let module = build(PROFILE_SOURCE, KirConsumer::NativeLibrary);
    let generate =
        prepare_ck_profile_kir(&module, CkProfileKirMode::Generate).expect("generate profile plan");
    let use_plan =
        prepare_ck_profile_kir(&module, CkProfileKirMode::Use).expect("use profile plan");
    let off = prepare_ck_profile_kir(&module, CkProfileKirMode::Off).expect("off profile plan");

    assert_eq!(
        generate.pre_profile_kir_digest,
        use_plan.pre_profile_kir_digest
    );
    assert_eq!(generate.site_table_digest, use_plan.site_table_digest);
    assert_eq!(generate.sites, use_plan.sites);
    assert_eq!(generate.module, use_plan.module);
    assert!(!generate.operations.is_empty());
    assert!(use_plan.operations.is_empty());
    assert!(off.sites.is_empty());
    assert!(off.annotations.is_empty());
    assert!(off.operations.is_empty());
    assert_eq!(off.module, module);
}

#[test]
fn profile_kir_topology_should_be_format_independent_and_semantic_sensitive() {
    let formatted = PROFILE_SOURCE.replace("  ", "    ").replace(
        "let length: u32 = items.len;",
        "// stable comment\n    let length: u32 = items.len;",
    );
    let semantic = PROFILE_SOURCE.replace("i == 4", "i == 5");
    let first = prepare_ck_profile_kir(
        &build(PROFILE_SOURCE, KirConsumer::NativeLibrary),
        CkProfileKirMode::Use,
    )
    .expect("first plan");
    let second = prepare_ck_profile_kir(
        &build(&formatted, KirConsumer::NativeLibrary),
        CkProfileKirMode::Use,
    )
    .expect("formatted plan");
    let changed = prepare_ck_profile_kir(
        &build(&semantic, KirConsumer::NativeLibrary),
        CkProfileKirMode::Use,
    )
    .expect("semantic plan");

    assert_eq!(first.pre_profile_kir_digest, second.pre_profile_kir_digest);
    assert_eq!(first.site_table_digest, second.site_table_digest);
    assert_ne!(first.pre_profile_kir_digest, changed.pre_profile_kir_digest);
    assert_ne!(first.site_table_digest, changed.site_table_digest);
}

#[test]
fn profile_kir_mapping_should_fail_closed_for_forged_or_stale_records() {
    let module = build(PROFILE_SOURCE, KirConsumer::NativeLibrary);
    let plan = prepare_ck_profile_kir(&module, CkProfileKirMode::Generate)
        .expect("profile generation plan");

    let mut forged_id = plan.clone();
    forged_id.operations[0].site_id.0[0] ^= 1;
    assert!(validate_ck_profile_kir_plan(&forged_id).is_err());

    let mut missing = plan.clone();
    missing.operations.pop();
    assert!(validate_ck_profile_kir_plan(&missing).is_err());

    let mut duplicate = plan.clone();
    duplicate.operations.push(duplicate.operations[0].clone());
    assert!(validate_ck_profile_kir_plan(&duplicate).is_err());

    let mut reordered = plan.clone();
    reordered.operations.swap(0, 1);
    assert!(validate_ck_profile_kir_plan(&reordered).is_err());

    let mut stale = plan.clone();
    stale.module.functions[0].name.push_str("_stale");
    assert!(validate_ck_profile_kir_plan(&stale).is_err());

    let mut annotation = plan.clone();
    annotation.annotations[0].event = CkProfileEvent::FunctionEntry {
        function: annotation.module.functions[0].id,
        block: annotation.module.functions[0].blocks[0].id,
    };
    if annotation.annotations[0] == plan.annotations[0] {
        annotation.annotations[0].site_id.0[1] ^= 1;
    }
    assert!(validate_ck_profile_kir_plan(&annotation).is_err());
}

#[test]
fn profile_kir_schema3_golden_digest_should_be_stable() {
    let module = build(PROFILE_SOURCE, KirConsumer::NativeLibrary);
    let plan = prepare_ck_profile_kir(&module, CkProfileKirMode::Generate)
        .expect("profile generation plan");
    let digest: [u8; 32] = Sha256::digest(print_ck_profile_kir_plan(&plan).as_bytes()).into();
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    assert_eq!(
        actual,
        "66fcf336677f53bf492245367fba26652535ac9d69d24a99f434461406467be9"
    );
}

#[test]
fn profile_kir_should_reject_portable_consumers() {
    let module = build(PROFILE_SOURCE, KirConsumer::C);
    let error = prepare_ck_profile_kir(&module, CkProfileKirMode::Generate)
        .expect_err("portable consumers have no hidden instrumentation");

    assert!(error.to_string().contains("Native"));
}

fn assert_no_critical_edges(module: &calckernel::KirModule) {
    for function in &module.functions {
        let mut incoming = BTreeMap::new();
        for block in &function.blocks {
            for target in calckernel::terminator_successors(&block.terminator) {
                *incoming.entry(target).or_insert(0_u32) += 1;
            }
        }
        for block in &function.blocks {
            if let calckernel::KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &block.terminator
            {
                assert!(incoming[&then_edge.target] <= 1);
                assert!(incoming[&else_edge.target] <= 1);
            }
        }
    }
}
