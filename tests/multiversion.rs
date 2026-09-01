use calckernel::{
    KirConsumer, KirMultiversionPlatform, KirMultiversionTargetSet, KirMultiversionTierId,
    KirTargetArchitecture, KirTargetOperatingSystem,
};

fn platform(
    architecture: KirTargetArchitecture,
    operating_system: KirTargetOperatingSystem,
) -> KirMultiversionPlatform {
    KirMultiversionPlatform {
        architecture,
        operating_system,
    }
}

#[test]
fn planning_target_set_schema1_should_cover_the_closed_six_platform_table() {
    let fixtures = [
        (
            platform(
                KirTargetArchitecture::X86_64,
                KirTargetOperatingSystem::Linux,
            ),
            vec![
                KirMultiversionTierId::Baseline,
                KirMultiversionTierId::X86_64V3,
                KirMultiversionTierId::X86_64V4,
            ],
            "360973a6c93d34a782daf8f7d527fa4fec2f71bf6aa477fa4c255e9021463a17",
        ),
        (
            platform(
                KirTargetArchitecture::X86_64,
                KirTargetOperatingSystem::Darwin,
            ),
            vec![
                KirMultiversionTierId::Baseline,
                KirMultiversionTierId::X86_64V3,
                KirMultiversionTierId::X86_64V4,
            ],
            "304d93495f46ec801a513f94994b1873e96e8a6c888c4f18542f534235455501",
        ),
        (
            platform(
                KirTargetArchitecture::X86_64,
                KirTargetOperatingSystem::Windows,
            ),
            vec![
                KirMultiversionTierId::Baseline,
                KirMultiversionTierId::X86_64V3,
                KirMultiversionTierId::X86_64V4,
            ],
            "d818c9555e8195dca156fa3759fe77a329d84c07fb0f84ac3bef6dd99b14805f",
        ),
        (
            platform(
                KirTargetArchitecture::AArch64,
                KirTargetOperatingSystem::Linux,
            ),
            vec![
                KirMultiversionTierId::Baseline,
                KirMultiversionTierId::AArch64Sve,
                KirMultiversionTierId::AArch64Sve2,
            ],
            "093e91850c11a5c219d2989d12930554024c8bfddc173e627de00071e8b78928",
        ),
        (
            platform(
                KirTargetArchitecture::AArch64,
                KirTargetOperatingSystem::Darwin,
            ),
            vec![KirMultiversionTierId::Baseline],
            "35e4d7e02384e972b30928223b671af5c40dce23528f8dae0d75734d41e74303",
        ),
        (
            platform(
                KirTargetArchitecture::AArch64,
                KirTargetOperatingSystem::Windows,
            ),
            vec![KirMultiversionTierId::Baseline],
            "4d41c1e469f8fc59784a633e9db9ee4f04e886b5bbfdfbdaae074b58f781b46f",
        ),
    ];

    for (platform, expected, expected_digest) in fixtures {
        let first = KirMultiversionTargetSet::schema1_fixture(platform, KirConsumer::NativeLibrary)
            .expect("closed target set");
        let second =
            KirMultiversionTargetSet::schema1_fixture(platform, KirConsumer::NativeLibrary)
                .expect("repeat target set");
        assert_eq!(first.schema_version, 1);
        assert_eq!(
            first.tiers.iter().map(|tier| tier.id).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.digest_hex(), expected_digest);
        first.validate().expect("valid target set");
    }
}

#[test]
fn planning_target_set_predicates_should_include_hardware_and_os_state() {
    let x86 = KirMultiversionTargetSet::schema1_fixture(
        platform(
            KirTargetArchitecture::X86_64,
            KirTargetOperatingSystem::Linux,
        ),
        KirConsumer::NativeExecutable,
    )
    .expect("x86 target set");
    let v3 = x86.tier(KirMultiversionTierId::X86_64V3).expect("v3");
    let v4 = x86.tier(KirMultiversionTierId::X86_64V4).expect("v4");
    assert!(v3.required_features.contains(&"avx2".to_string()));
    assert!(v3.predicate.os_state.contains(&"xcr0.xmm-ymm".to_string()));
    assert!(v4.required_features.contains(&"avx512vl".to_string()));
    assert!(
        v4.predicate
            .os_state
            .contains(&"xcr0.opmask-zmm".to_string())
    );

    let arm = KirMultiversionTargetSet::schema1_fixture(
        platform(
            KirTargetArchitecture::AArch64,
            KirTargetOperatingSystem::Linux,
        ),
        KirConsumer::NativeExecutable,
    )
    .expect("AArch64 target set");
    let sve2 = arm.tier(KirMultiversionTierId::AArch64Sve2).expect("SVE2");
    assert_eq!(sve2.required_features, ["sve", "sve2"]);
    assert_eq!(sve2.predicate.os_state, ["linux.sve-state"]);
}

#[test]
fn planning_target_set_should_fail_closed_for_unsupported_platforms() {
    let error = KirMultiversionTargetSet::schema1_for_triple(
        "riscv64-unknown-linux-gnu",
        KirConsumer::NativeLibrary,
    )
    .expect_err("unsupported architecture must fail");
    assert!(error.contains("unsupported multiversion target"), "{error}");
}
