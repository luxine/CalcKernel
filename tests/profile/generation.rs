use std::fs;

use calckernel::{
    CkProfileCounter, CkProfileSiteDescriptor, CkProfileSiteId, CkProfileSiteKind,
    anchor_profile_directory, create_profile_shard_template, parse_profile_shard,
    profile_histogram_bucket, profile_site_table_digest,
};

use super::{fixture_identity, test_root};

#[test]
fn generation_template_should_patch_every_counter_slot_without_changing_topology() {
    let sites = vec![
        CkProfileSiteDescriptor {
            id: CkProfileSiteId([1; 16]),
            function_digest: [2; 32],
            location: 1,
            kind: CkProfileSiteKind::FunctionEntry,
        },
        CkProfileSiteDescriptor {
            id: CkProfileSiteId([3; 16]),
            function_digest: [4; 32],
            location: 2,
            kind: CkProfileSiteKind::LoopTripHistogram { loop_identity: 2 },
        },
        CkProfileSiteDescriptor {
            id: CkProfileSiteId([5; 16]),
            function_digest: [6; 32],
            location: 3,
            kind: CkProfileSiteKind::CandidateConstant {
                decision_identity: 3,
                candidates: vec![7, 11],
            },
        },
    ];
    let digest = profile_site_table_digest(&sites).expect("site digest");
    let template = create_profile_shard_template(fixture_identity(digest), sites)
        .expect("runtime shard template");

    assert_eq!(template.site_first_counters, [0, 1, 17]);
    assert_eq!(template.site_counter_counts, [1, 16, 3]);
    assert_eq!(template.counter_offsets.len(), 20);
    assert_eq!(template.digest_offset as usize + 32, template.bytes.len());
    let shard = parse_profile_shard(&template.bytes).expect("zero template is a valid shard");
    assert!(matches!(
        shard.counters[0].counter,
        CkProfileCounter::Scalar(0)
    ));
}

#[test]
fn generation_histogram_bucket_should_cover_closed_schema_boundaries() {
    let cases = [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 3),
        (4, 3),
        (5, 4),
        (65_536, 14),
        (65_537, 15),
        (u32::MAX, 15),
    ];
    for (value, bucket) in cases {
        assert_eq!(profile_histogram_bucket(value), bucket);
    }
}

#[test]
fn generation_directory_should_capture_stable_absolute_identity() {
    let root = test_root("generation-anchor");
    fs::create_dir_all(&root).expect("create collection directory");
    let first = anchor_profile_directory(&root).expect("anchor directory");
    let second = anchor_profile_directory(&root).expect("anchor directory again");
    fs::remove_dir_all(&root).expect("remove collection directory");

    assert!(first.path.is_absolute());
    assert_eq!(first, second);
}

#[cfg(unix)]
#[test]
fn generation_directory_should_reject_indirection_in_any_component() {
    use std::os::unix::fs::symlink;

    let root = test_root("generation-anchor-symlink");
    let real = root.join("real");
    let linked = root.join("linked");
    fs::create_dir_all(real.join("output")).expect("create real collection directory");
    symlink(&real, &linked).expect("create directory symlink");
    let error =
        anchor_profile_directory(&linked.join("output")).expect_err("reject symlink component");
    fs::remove_dir_all(&root).expect("remove collection directory");

    assert!(matches!(error, calckernel::CkProfileError::SymlinkInput(_)));
}
