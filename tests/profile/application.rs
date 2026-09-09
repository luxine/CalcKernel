use calckernel::{
    CkProfileCounter, CkProfileUnknownReason, CkProfileWorkTerm, apply_profile,
    merge_profile_shards,
};

use super::{fixture_shard, fixture_shard_with_counter, fixture_site};

#[test]
fn application_exact_identity_and_site_table_should_produce_immutable_work_analysis() {
    let shard = fixture_shard(1, 256);
    let profile = merge_profile_shards(&[shard]).expect("profile");
    let term = CkProfileWorkTerm {
        site_id: profile.sites[0].id,
        function_digest: profile.sites[0].function_digest,
        static_cost_units: 7,
    };
    let analysis = apply_profile(
        &profile,
        &profile.identity,
        &profile.sites,
        std::slice::from_ref(&term),
    )
    .expect("apply compatible profile");

    assert_eq!(analysis.sites.len(), 1);
    assert_eq!(analysis.functions.len(), 1);
    assert_eq!(analysis.functions[0].dynamic_work, Some(1792));
    assert!(analysis.functions[0].hot_root);
}

#[test]
fn application_identity_should_report_first_precise_field_and_digests() {
    let profile = merge_profile_shards(&[fixture_shard(1, 256)]).expect("profile");
    let mut expected = profile.identity.clone();
    expected.module.pre_profile_kir_digest = [0xaa; 32];
    let error = apply_profile(&profile, &expected, &profile.sites, &[])
        .expect_err("reject identity mismatch");
    let text = error.to_string();
    assert!(text.contains("module.preProfileKirDigest"), "{text}");
    assert!(text.contains(&"aa".repeat(32)), "{text}");
    assert!(text.contains(&"22".repeat(32)), "{text}");
}

#[test]
fn application_saturated_and_incomplete_sites_should_be_unknown_not_authority() {
    let scalar = fixture_shard_with_counter(1, fixture_site(1), CkProfileCounter::Scalar(u64::MAX));
    let profile = merge_profile_shards(&[scalar]).expect("saturated profile");
    let analysis = apply_profile(&profile, &profile.identity, &profile.sites, &[])
        .expect("apply saturated profile conservatively");
    assert!(matches!(
        analysis.sites[0].observation,
        calckernel::CkProfileObservation::Unknown(CkProfileUnknownReason::Saturated)
    ));
}
