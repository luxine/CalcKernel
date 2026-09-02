#[path = "support/mod.rs"]
mod support;

use std::fs;

use calckernel::{
    CkProfileError, NATIVE_ABI_VERSION, RUNTIME_ABI_VERSION, SourceFile, TuneDecisionError, check,
    decode_tune_decision, parse_profile,
};

use support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn compatibility_v0_13_sources_and_public_runtime_contracts_should_remain_accepted() {
    for path in [
        "tests/fixtures/compatibility/v0_10/native_checked.ck",
        "tests/fixtures/compatibility/v0_11/contracts.ck",
        "tests/fixtures/compatibility/v0_12/vector_optimizer.ck",
    ] {
        let source = SourceFile::new(path, read(path));
        let result = check(&source);
        assert_eq!(result.diagnostics, [], "{path} must remain accepted");
    }
    assert_eq!(NATIVE_ABI_VERSION, 1);
    assert_eq!(RUNTIME_ABI_VERSION, 2);
}

#[test]
fn compatibility_private_v0_13_profile_and_cache_schemas_should_fail_closed() {
    let mut legacy_profile = Vec::from(b"CKPROF01".as_slice());
    legacy_profile.extend_from_slice(&0_u32.to_be_bytes());
    legacy_profile.extend_from_slice(&[0; 32]);
    assert_eq!(
        parse_profile(&legacy_profile),
        Err(CkProfileError::UnsupportedSchema {
            kind: "profile format",
            expected: 1,
            observed: 0,
        })
    );

    let cache = read("src/cli/cache/entry.rs");
    assert!(cache.contains("CKCOBJ04"));
    assert!(cache.contains("MANIFEST_SCHEMA: u32 = 5"));
    let cache_tests = read("tests/native/cache.rs");
    assert!(cache_tests.contains("reject_ckcobj03_entries"));

    let mut future_decision =
        fs::read(repo_root().join("tests/fixtures/tune/decision-schema1-baseline.cktune"))
            .expect("read schema-1 tuning decision");
    future_decision[8..12].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(
        decode_tune_decision(&future_decision),
        Err(TuneDecisionError::UnsupportedSchema)
    );
}

#[test]
fn compatibility_history_should_preserve_v0_10_through_v0_13_identities() {
    for (directory, release) in [
        ("v0_10", "0.10.0"),
        ("v0_11", "0.11.0"),
        ("v0_12", "0.12.0"),
        ("v0_13", "0.13.0"),
    ] {
        let manifest = read(&format!(
            "tests/fixtures/compatibility/{directory}/manifest.toml"
        ));
        assert!(
            manifest.contains(&format!("release = \"{release}\"")),
            "{directory} must preserve {release}"
        );
    }
}
