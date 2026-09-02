use calckernel::{decode_tune_decision, inspect_tune_json, inspect_tune_text};
use std::fs;
use std::path::PathBuf;

use super::support;

#[test]
fn inspection_schema_one_is_exact_and_path_free() {
    let decision = decode_tune_decision(&support::baseline_decision()).expect("decision");

    let json = inspect_tune_json(&decision).expect("json inspection");
    let text = inspect_tune_text(&decision).expect("text inspection");

    assert!(
        json.starts_with("{\"fileMagic\":\"CKTUNE01\",\"formatSchema\":1,\"decisionDigest\":\"")
    );
    assert!(json.ends_with("]}\n"));
    assert_eq!(json.matches("\"tag\":").count(), 167);
    assert_eq!(json.matches("\"type\":").count(), 167);
    assert!(json.contains(
        "{\"tag\":1,\"type\":\"record:Identity\",\"value\":[{\"tag\":1,\"type\":\"text\",\"value\":\"0.14.0\"}"
    ));
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("Rust_CalcKernel"));

    assert!(text.starts_with("CKTUNE-INSPECT\t1\t"));
    assert!(text.contains("/1\trecord:Identity\tfields=22\n"));
    assert!(text.contains("/1/21/3\tlist:text:256\titems=1\n"));
    assert!(text.contains("/1/21/3/@0\ttext\t\"+sse2\"\n"));
    assert_eq!(text.lines().count(), 178);
    assert!(!text.contains("/Users/"));
}

#[test]
fn inspection_schema_one_matches_tuned_golden_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tune");
    let decision = decode_tune_decision(
        &fs::read(root.join("decision-schema1-tuned.cktune")).expect("tuned fixture"),
    )
    .expect("decision");

    assert_eq!(
        inspect_tune_json(&decision).expect("json"),
        fs::read_to_string(root.join("decision-schema1-inspection.json")).expect("json fixture")
    );
    assert_eq!(
        inspect_tune_text(&decision).expect("text"),
        fs::read_to_string(root.join("decision-schema1-inspection.txt")).expect("text fixture")
    );
}
