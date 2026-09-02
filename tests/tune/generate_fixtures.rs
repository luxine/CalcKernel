use std::fs;
use std::path::PathBuf;

use calckernel::{decode_tune_decision, inspect_tune_json, inspect_tune_text};

use super::support;

#[test]
#[ignore = "maintainer-only normative fixture generator"]
fn generate_decision_schema_one_fixtures() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tune");
    fs::create_dir_all(&root).expect("fixture directory");
    let baseline = support::baseline_decision();
    let tuned = support::tuned_decision();
    let decision = decode_tune_decision(&tuned).expect("tuned fixture");

    fs::write(
        root.join("decision-schema1-framing.hex"),
        b"u8=ff\nu16=1234\nu32=12345678\nu64=0123456789abcdef\nu128=0123456789abcdeffedcba9876543210\nbool-false=00\nbool-true=01\nd32=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\ntext=00000003434b21\nbytes=0000000300ff7f\nrecord=0000000700010000000101\nlist-u8=00000003010203\noptional-none=00\noptional-u32=0100000001\n",
    )
    .expect("framing fixture");
    fs::write(root.join("decision-schema1-baseline.cktune"), baseline).expect("baseline fixture");
    fs::write(root.join("decision-schema1-tuned.cktune"), tuned).expect("tuned fixture");
    fs::write(
        root.join("decision-schema1-inspection.json"),
        inspect_tune_json(&decision).expect("json fixture"),
    )
    .expect("json fixture");
    fs::write(
        root.join("decision-schema1-inspection.txt"),
        inspect_tune_text(&decision).expect("text fixture"),
    )
    .expect("text fixture");
}
