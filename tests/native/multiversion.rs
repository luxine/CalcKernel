use calckernel::{
    EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirCpuIdentity,
    KirMultiversionPlanningRequest, KirMultiversionTierId, KirNativeCpuPolicy,
    KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, NativeContext,
    NativeMultiversionTargetSet, NativeOptimizationLevel, SourceFile, build_kir_module, check,
    check_kir_multiversion_bundle, import_contract_facts, lower_native_kir_module, lower_to_mir,
    propose_kir_multiversion_bundle, run_kir_pass_pipeline,
};

#[test]
fn target_set_host_should_materialize_exact_separate_llvm_target_machines() {
    let materialized = NativeMultiversionTargetSet::host(KirConsumer::NativeLibrary)
        .expect("materialized host target set");
    let target_set = materialized.target_set();
    target_set.validate().expect("checked target set");
    assert_eq!(target_set.schema_version, 1);
    assert_eq!(target_set.tiers[0].id, KirMultiversionTierId::Baseline);
    for tier in &target_set.tiers {
        let target = materialized.target(tier.id).expect("tier target machine");
        assert_eq!(target.triple().expect("triple"), tier.triple);
        assert_eq!(target.data_layout().expect("layout"), tier.data_layout);
        assert_eq!(target.cpu().expect("CPU"), tier.cpu);
        assert_eq!(
            target
                .kir_profile(KirConsumer::NativeLibrary)
                .expect("profile")
                .digest_hex(),
            tier.profile.digest_hex()
        );
    }
}

#[test]
fn variant_feature_modules_should_lower_and_emit_independently_for_each_host_tier() {
    let targets = NativeMultiversionTargetSet::host(KirConsumer::NativeLibrary)
        .expect("materialized host target set");
    let checked = check(&SourceFile::new(
        "native-multiversion.ck",
        "export fn sum(items: slice<i32>, n: u32) -> i32 { let i: u32 = 0; let total: i32 = 0; while i < n { total = total + items[i]; i = i + 1; } return total; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let mut kir = kir;
    kir.profile = targets.target_set().tiers[0].profile.clone();
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let optimized = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let request = KirMultiversionPlanningRequest {
        logical_pre_state: optimized.artifact.expect("baseline"),
        target_set: targets.target_set().clone(),
        pgo_hot_roots: None,
        shared_growth_consumed: 0,
    };
    let bundle = propose_kir_multiversion_bundle(&request).expect("bundle");
    check_kir_multiversion_bundle(&request, &bundle).expect("checked bundle");
    if targets.target_set().tiers.len() > 1 {
        assert!(bundle.roots.iter().any(|root| !root.variants.is_empty()));
    }
    let context = NativeContext::new().expect("context");
    for variant in bundle.roots.iter().flat_map(|root| &root.variants) {
        let target = targets.target(variant.tier).expect("variant target");
        let verified =
            run_kir_pass_pipeline(variant.module.clone(), KirOptimizationLevel::O0, None);
        assert!(verified.errors.is_empty(), "{:?}", verified.errors);
        let object = target
            .emit_object(
                lower_native_kir_module(&context, target, &verified, &EmitLlvmOptions::default())
                    .expect("separate lowering")
                    .verify()
                    .expect("LLVM verify")
                    .audit()
                    .expect("fact audit")
                    .optimize(target, NativeOptimizationLevel::O3)
                    .expect("separate optimization"),
            )
            .expect("separate object");
        assert!(!object.is_empty());
    }
}

#[test]
fn variant_feature_target_profiles_should_be_explicit_and_contained() {
    let materialized = NativeMultiversionTargetSet::host(KirConsumer::NativeExecutable)
        .expect("materialized host target set");
    for tier in &materialized.target_set().tiers {
        let KirCpuIdentity::Native {
            policy,
            name,
            features,
        } = tier.profile.cpu_identity()
        else {
            panic!("Native tier profile")
        };
        assert_eq!(*policy, KirNativeCpuPolicy::Multiversion);
        assert_eq!(name, &tier.cpu);
        assert!(
            tier.llvm_features
                .iter()
                .all(|feature| features.contains(feature))
        );
        assert!(
            tier.required_features
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert_eq!(tier.predicate.hardware_features, tier.required_features);
    }
}
