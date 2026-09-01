use std::collections::BTreeSet;

use calckernel::{
    KIR_MASK_COST_LANE, KirAlignmentClass, KirConsumer, KirCostKey, KirCostSemantics,
    KirCpuIdentity, KirLaneType, KirNativeCpuPolicy, KirOperationAvailability, KirProfileOperation,
    KirTargetIdentity, KirTargetProfile, NativeCpu, NativeTarget,
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
    assert_eq!(
        first.producer_identity(),
        (
            Some("LLVM 22.1.8 TCK_RecipThroughput"),
            Some("ckc-llvm-bridge-abi-3")
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
