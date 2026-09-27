use std::collections::BTreeSet;

use calckernel::{
    EmitLlvmOptions, KIR_MASK_COST_LANE, KirAlignmentClass, KirBoundsMode, KirBuildConfig,
    KirConsumer, KirCostKey, KirCostSemantics, KirCpuIdentity, KirLaneType, KirNativeCpuPolicy,
    KirOperationAvailability, KirOptimizationLevel, KirOverflowMode, KirProfileOperation,
    KirSanitizerMode, KirTargetIdentity, KirTargetProfile, NativeContext, NativeCpu,
    NativeMultiversionTargetSet, NativeOptimizationLevel, NativeTarget, SourceFile,
    build_kir_module_with_profile, check, import_contract_facts, lower_native_kir_module,
    lower_to_mir, run_kir_pass_pipeline,
};

#[test]
fn target_profile_should_be_complete_canonical_and_target_bound() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let first = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("first target profile");
    let second = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("second target profile");

    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.digest_hex(), second.digest_hex());
    assert_eq!(
        first.cost_entry_count(),
        KirTargetProfile::fixed_query_universe().len()
    );
    assert!(first.vector_operations_enabled());
    assert!(first.maximum_interleave_factor() >= 1);
    #[cfg(target_arch = "x86_64")]
    assert!(
        first.maximum_interleave_factor() >= 4,
        "x86 target profile must expose the closed four-chain KIR frontier"
    );
    assert_eq!(
        first.producer_identity(),
        (
            Some("LLVM 22.1.8 TCK_RecipThroughput"),
            Some("ckc-llvm-bridge-abi-4")
        )
    );
    assert_eq!(
        first.target_identity(),
        &KirTargetIdentity::Native {
            triple: target.triple().expect("target triple")
        }
    );
    assert!(matches!(
        first.cpu_identity(),
        KirCpuIdentity::Native {
            policy: KirNativeCpuPolicy::Baseline,
            ..
        }
    ));

    let keys = KirTargetProfile::fixed_query_universe();
    let unique = keys.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), keys.len());
    assert!(
        keys.iter()
            .all(|key| first.operation_availability(key).is_some())
    );
    assert!(keys.iter().any(|key| {
        matches!(
            first.operation_availability(key),
            Some(KirOperationAvailability::Legal(cost)) if key.lanes > 1 && cost.cost > 0
        )
    }));
    assert!(keys.iter().all(|key| {
        key.operation != KirProfileOperation::MaskNot
            || key.lane == KIR_MASK_COST_LANE
            || matches!(
                first.operation_availability(key),
                Some(KirOperationAvailability::Unavailable)
            )
    }));

    let cast = first
        .operation_availability(&KirCostKey {
            operation: KirProfileOperation::Cast,
            lane: KirLaneType::I32,
            lanes: 4,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::NotApplicable,
        })
        .expect("closed cast query");
    if let KirOperationAvailability::Legal(cost) = cast {
        assert!(
            cost.legalized_type.contains("double"),
            "cast legalization must describe its f64 result: {cost:?}"
        );
    }
}

#[test]
fn target_profile_should_contain_native_features_without_baseline_host_leakage() {
    let baseline_target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline");
    let native_target = NativeTarget::host_with_cpu(NativeCpu::Native).expect("native");
    let baseline = baseline_target
        .kir_profile(KirConsumer::NativeExecutable)
        .expect("baseline profile");
    let native = native_target
        .kir_profile(KirConsumer::NativeExecutable)
        .expect("native profile");

    let KirCpuIdentity::Native {
        policy: baseline_policy,
        name: baseline_name,
        features: baseline_features,
    } = baseline.cpu_identity()
    else {
        panic!("baseline CPU identity")
    };
    let KirCpuIdentity::Native {
        policy: native_policy,
        features: native_features,
        ..
    } = native.cpu_identity()
    else {
        panic!("native CPU identity")
    };
    assert_eq!(*baseline_policy, KirNativeCpuPolicy::Baseline);
    assert_eq!(*native_policy, KirNativeCpuPolicy::Native);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(baseline_name, "generic");
    #[cfg(target_arch = "x86_64")]
    assert_eq!(baseline_name, "x86-64");
    assert!(baseline_features.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(native_features.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn baseline_profile_should_price_f64_slp_setup_and_division_as_emitted_work() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("baseline profile");
    for operation in [
        KirProfileOperation::Splat,
        KirProfileOperation::Insert,
        KirProfileOperation::Divide,
        KirProfileOperation::Extract,
    ] {
        let semantics = if operation == KirProfileOperation::Divide {
            KirCostSemantics::StrictFloat
        } else {
            KirCostSemantics::NotApplicable
        };
        let availability = profile.operation_availability(&KirCostKey {
            operation,
            lane: KirLaneType::F64,
            lanes: 2,
            semantics,
            alignment: KirAlignmentClass::NotApplicable,
        });
        assert!(
            matches!(availability, Some(KirOperationAvailability::Legal(cost)) if cost.cost >= 1),
            "f64x2 {operation:?} must have a positive structural cost, got {availability:?}"
        );
    }
}

fn same_snapshot(first: &KirTargetProfile, second: &KirTargetProfile) -> bool {
    // The identity is stored inside the immutable Arc backing, not a temporary.
    std::ptr::eq(first.cpu_identity(), second.cpu_identity())
}

#[test]
fn target_profile_should_reuse_immutable_snapshot_for_each_consumer() {
    for cpu in [
        NativeCpu::Baseline,
        NativeCpu::Native,
        NativeCpu::Multiversion,
    ] {
        let target = NativeTarget::host_with_cpu(cpu).expect("target");
        for consumer in [KirConsumer::NativeLibrary, KirConsumer::NativeExecutable] {
            let first = target.kir_profile(consumer).expect("first profile");
            let second = target.kir_profile(consumer).expect("second profile");
            assert!(
                same_snapshot(&first, &second),
                "{cpu:?}/{consumer:?} rebuilt an immutable target profile"
            );
        }
    }
}

#[test]
fn target_profile_cache_should_keep_consumers_separate() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("target");
    let library = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("library");
    let executable = target
        .kir_profile(KirConsumer::NativeExecutable)
        .expect("executable");
    assert_eq!(library.consumer(), KirConsumer::NativeLibrary);
    assert_eq!(executable.consumer(), KirConsumer::NativeExecutable);
    assert_ne!(library.digest_hex(), executable.digest_hex());
    assert!(!same_snapshot(&library, &executable));
    assert!(same_snapshot(
        &library,
        &target
            .kir_profile(KirConsumer::NativeLibrary)
            .expect("library again")
    ));
    assert!(same_snapshot(
        &executable,
        &target
            .kir_profile(KirConsumer::NativeExecutable)
            .expect("executable again")
    ));
}

#[test]
fn target_profile_cache_should_be_scoped_to_one_target_machine() {
    let first_target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("first target");
    let second_target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("second target");
    let first = first_target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("first");
    let second = second_target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("second");
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert!(
        !same_snapshot(&first, &second),
        "target instances shared a cache"
    );
}

#[test]
fn target_profile_cache_should_keep_cpu_policies_separate() {
    let profiles = [
        NativeCpu::Baseline,
        NativeCpu::Native,
        NativeCpu::Multiversion,
    ]
    .map(|cpu| {
        NativeTarget::host_with_cpu(cpu)
            .expect("target")
            .kir_profile(KirConsumer::NativeLibrary)
            .expect("profile")
    });
    for first in 0..profiles.len() {
        for second in first + 1..profiles.len() {
            assert_ne!(profiles[first].digest_hex(), profiles[second].digest_hex());
            assert!(!same_snapshot(&profiles[first], &profiles[second]));
        }
    }
}

#[test]
fn target_profile_cache_should_reject_unsupported_consumers_after_warmup() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("target");
    let library = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("library");
    for consumer in [
        KirConsumer::C,
        KirConsumer::WebAssembly,
        KirConsumer::Inspection,
    ] {
        let error = target
            .kir_profile(consumer)
            .expect_err("reject non-native consumer");
        assert_eq!(
            error.message,
            "native target profile requires a Native consumer"
        );
    }
    assert!(same_snapshot(
        &library,
        &target
            .kir_profile(KirConsumer::NativeLibrary)
            .expect("library after errors")
    ));
}

#[test]
fn target_profile_snapshot_should_outlive_its_target_machine() {
    let (profile, bytes) = {
        let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("target");
        let profile = target
            .kir_profile(KirConsumer::NativeLibrary)
            .expect("profile");
        let bytes = profile.canonical_bytes();
        (profile, bytes)
    };
    profile.validate().expect("owned profile remains valid");
    assert_eq!(profile.canonical_bytes(), bytes);
}

#[test]
fn target_profile_cache_should_survive_object_emission() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("target");
    let before = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("profile");
    let checked = check(&SourceFile::new(
        "profile-cache.ck",
        "export fn answer() -> i32 { return 42; }",
    ));
    assert!(checked.diagnostics.is_empty());
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        before.clone(),
    )
    .expect("KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let context = NativeContext::new().expect("context");
    let optimized =
        lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
            .expect("lower")
            .verify()
            .expect("verify")
            .audit()
            .expect("audit")
            .optimize(&target, NativeOptimizationLevel::O3)
            .expect("optimize");
    let object = target.emit_object(optimized).expect("object");
    assert!(!object.is_empty());
    let after = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("profile after emission");
    let independent = NativeTarget::host_with_cpu(NativeCpu::Baseline)
        .expect("independent")
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("independent profile");
    assert_eq!(after.canonical_bytes(), independent.canonical_bytes());
    assert!(
        same_snapshot(&before, &after),
        "emission discarded the immutable snapshot"
    );
}

#[test]
fn target_profile_cache_should_cover_explicit_multiversion_targets() {
    for consumer in [KirConsumer::NativeLibrary, KirConsumer::NativeExecutable] {
        let targets = NativeMultiversionTargetSet::host(consumer).expect("target set");
        for tier in &targets.target_set().tiers {
            let target = targets.target(tier.id).expect("materialized target");
            let repeated = target.kir_profile(consumer).expect("materialized profile");
            assert_eq!(repeated.canonical_bytes(), tier.profile.canonical_bytes());
            assert!(
                same_snapshot(&repeated, &tier.profile),
                "{consumer:?}/{:?} rebuilt its explicit target profile",
                tier.id
            );
        }
    }
}
