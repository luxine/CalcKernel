use calckernel::{inspect_profile_json, inspect_profile_text, merge_profile_shards};

use super::fixture_shard;

#[test]
fn profile_inspection_json_should_be_deterministic_and_versioned() {
    let profile = merge_profile_shards(&[fixture_shard(7, 19)]).expect("merge fixture");
    let json = inspect_profile_json(&profile).expect("inspect profile JSON");

    assert_eq!(
        json,
        include_str!("../fixtures/profile/inspection-schema1.json")
    );
}

#[test]
fn profile_inspection_text_should_report_coverage_and_counts() {
    let profile = merge_profile_shards(&[fixture_shard(7, 19)]).expect("merge fixture");
    let text = inspect_profile_text(&profile).expect("inspect profile text");

    assert!(text.contains("coverage: 1/1\ncompleted runs: 1\nmerged shards: 1\n"));
}
