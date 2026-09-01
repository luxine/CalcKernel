use std::fs;

use calckernel::{
    CkProfileCounter, CkProfileCounterRecord, CkProfileError, CkProfileSiteDescriptor,
    CkProfileSiteId, CkProfileSiteKind, merge_profile_inputs, profile_site_table_digest,
    serialize_profile, serialize_profile_shard, validate_profile_output_path,
};

use super::{fixture_identity, fixture_shard, test_root};

#[test]
fn profile_merge_should_reject_duplicate_run_identity() {
    let shard = fixture_shard(7, 19);
    let error = calckernel::merge_profile_shards(&[shard.clone(), shard])
        .expect_err("reject duplicate run");

    assert_eq!(error, CkProfileError::DuplicateRunIdentity);
}

#[test]
fn profile_merge_should_be_independent_of_input_path_order() {
    let root = test_root("order");
    fs::create_dir_all(&root).expect("create profile test root");
    let first = root.join("z.ckprof-part");
    let second = root.join("a.ckprof-part");
    fs::write(
        &first,
        serialize_profile_shard(&fixture_shard(1, 3)).expect("serialize first"),
    )
    .expect("write first");
    fs::write(
        &second,
        serialize_profile_shard(&fixture_shard(2, 5)).expect("serialize second"),
    )
    .expect("write second");

    let forward = merge_profile_inputs(&[first.clone(), second.clone()])
        .expect("merge forward")
        .profile_bytes;
    let reverse = merge_profile_inputs(&[second, first])
        .expect("merge reverse")
        .profile_bytes;
    fs::remove_dir_all(root).expect("remove profile test root");

    assert_eq!(forward, reverse);
}

#[cfg(unix)]
#[test]
fn profile_merge_should_reject_symlink_input() {
    use std::os::unix::fs::symlink;

    let root = test_root("symlink");
    fs::create_dir_all(&root).expect("create profile test root");
    let shard = root.join("real.ckprof-part");
    let link = root.join("link.ckprof-part");
    fs::write(
        &shard,
        serialize_profile_shard(&fixture_shard(1, 3)).expect("serialize shard"),
    )
    .expect("write shard");
    symlink(&shard, &link).expect("create shard symlink");

    let error = merge_profile_inputs(&[link]).expect_err("reject symlink input");
    fs::remove_dir_all(root).expect("remove profile test root");

    assert!(matches!(error, CkProfileError::SymlinkInput(_)));
}

#[cfg(unix)]
#[test]
fn profile_output_should_reject_symlink_parent() {
    use std::os::unix::fs::symlink;

    let root = test_root("output-symlink");
    let real = root.join("real");
    let linked = root.join("linked");
    fs::create_dir_all(&real).expect("create profile output root");
    symlink(&real, &linked).expect("create output parent symlink");

    let error = validate_profile_output_path(&linked.join("result.ckprof"))
        .expect_err("reject symlink output parent");
    fs::remove_dir_all(root).expect("remove profile output root");

    assert!(matches!(error, CkProfileError::SymlinkInput(_)));
}

#[test]
fn profile_merge_should_saturate_counter_overflow() {
    let first = fixture_shard(1, u64::MAX - 1);
    let second = fixture_shard(2, 3);
    let profile = calckernel::merge_profile_shards(&[first, second]).expect("merge saturation");

    assert_eq!(
        profile.counters[0].counter,
        CkProfileCounter::Scalar(u64::MAX)
    );
    assert!(profile.overflowed);
}

#[test]
fn profile_merge_should_not_mark_unrelated_histogram_saturated() {
    let sites = vec![
        CkProfileSiteDescriptor {
            id: CkProfileSiteId([1; 16]),
            function_digest: [2; 32],
            location: 1,
            kind: CkProfileSiteKind::FunctionEntry,
        },
        CkProfileSiteDescriptor {
            id: CkProfileSiteId([2; 16]),
            function_digest: [3; 32],
            location: 2,
            kind: CkProfileSiteKind::LoopTripHistogram { loop_identity: 9 },
        },
    ];
    let identity = fixture_identity(profile_site_table_digest(&sites).expect("site digest"));
    let make_shard = |run, scalar, bucket| calckernel::CkProfileShard {
        identity: identity.clone(),
        sites: sites.clone(),
        counters: vec![
            CkProfileCounterRecord {
                site_id: sites[0].id,
                counter: CkProfileCounter::Scalar(scalar),
            },
            CkProfileCounterRecord {
                site_id: sites[1].id,
                counter: CkProfileCounter::Histogram {
                    buckets: {
                        let mut buckets = [0; 16];
                        buckets[0] = bucket;
                        buckets
                    },
                    saturated: false,
                },
            },
        ],
        run_id: [run; 16],
        overflowed: false,
        incomplete_observations: false,
    };
    let profile =
        calckernel::merge_profile_shards(&[make_shard(1, u64::MAX, 1), make_shard(2, 1, 1)])
            .expect("merge mixed counters");

    assert!(profile.overflowed);
    assert!(matches!(
        profile.counters[1].counter,
        CkProfileCounter::Histogram {
            buckets,
            saturated: false,
        } if buckets[0] == 2
    ));
}

#[test]
fn profile_merge_should_report_first_identity_mismatch() {
    let first = fixture_shard(1, 3);
    let mut second = fixture_shard(2, 5);
    second.identity.compiler.package_version = "different".to_string();
    let error =
        calckernel::merge_profile_shards(&[first, second]).expect_err("reject identity mismatch");

    assert!(matches!(
        error,
        CkProfileError::IdentityMismatch {
            field: "compiler.packageVersion",
            ..
        }
    ));
}

#[test]
fn profile_merge_should_count_recognized_temporary_files() {
    let root = test_root("temporary");
    fs::create_dir_all(&root).expect("create profile test root");
    fs::write(
        root.join("run.ckprof-part"),
        serialize_profile_shard(&fixture_shard(1, 3)).expect("serialize shard"),
    )
    .expect("write shard");
    fs::write(root.join("orphan.ckprof-part.tmp"), b"partial").expect("write temporary");
    let result = merge_profile_inputs(std::slice::from_ref(&root)).expect("merge directory");
    fs::remove_dir_all(root).expect("remove profile test root");

    assert_eq!(result.ignored_temporary_files, 1);
}

#[test]
fn profile_merge_should_reject_terminal_profile_in_directory() {
    let root = test_root("terminal");
    fs::create_dir_all(&root).expect("create profile test root");
    let profile =
        calckernel::merge_profile_shards(&[fixture_shard(1, 3)]).expect("build terminal profile");
    fs::write(
        root.join("terminal.ckprof"),
        serialize_profile(&profile).expect("serialize terminal profile"),
    )
    .expect("write terminal profile");
    let error = merge_profile_inputs(std::slice::from_ref(&root))
        .expect_err("reject terminal directory input");
    fs::remove_dir_all(root).expect("remove profile test root");

    assert!(matches!(error, CkProfileError::UnsupportedMergeInput(_)));
}

#[test]
fn profile_merge_should_reject_nested_directory() {
    let root = test_root("nested");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("create nested profile test root");
    fs::write(
        nested.join("run.ckprof-part"),
        serialize_profile_shard(&fixture_shard(1, 3)).expect("serialize shard"),
    )
    .expect("write nested shard");

    let error = merge_profile_inputs(std::slice::from_ref(&root))
        .expect_err("reject recursive directory input");
    fs::remove_dir_all(root).expect("remove profile test root");

    assert!(matches!(error, CkProfileError::UnsupportedMergeInput(_)));
}
