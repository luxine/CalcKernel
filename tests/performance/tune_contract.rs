use std::{collections::BTreeSet, fs, path::Path};

use calckernel::TuneManifest;

use super::support::oracle::repo_root;

const CASES: [&str; 7] = [
    "branch-layout",
    "call-constant-length",
    "compute-bound",
    "contract-fixed-length",
    "contract-noalias",
    "memory-bound",
    "trip-unroll-simd",
];

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn tune_schema_nine_assets_are_exact_and_partitioned_before_measurement() {
    let cargo = read("Cargo.toml");
    for text in [
        "name = \"tune_perf\"",
        "path = \"benches/tune_perf.rs\"",
        "harness = false",
    ] {
        assert!(cargo.contains(text), "missing tune bench contract: {text}");
    }
    let rows = read("benches/cases/tune-cases.tsv");
    let mut names = BTreeSet::new();
    let logical = rows
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(logical.first(), Some(&"ckc-tune-cases\t1"));
    assert_eq!(logical.len(), 8);
    for row in &logical[1..] {
        let fields = row.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 13, "malformed tune case row: {row}");
        assert!(names.insert(fields[0]));
        assert!(matches!(fields[12], "eligible" | "domain"));
        for digest in [fields[5], fields[8], fields[11]] {
            assert_eq!(digest.len(), 64);
            assert!(
                digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
        }
    }
    assert_eq!(names, CASES.into_iter().collect());

    let release = read("benches/fixtures/tune/release-held-out.tsv");
    assert_eq!(release.lines().count(), 8);
    assert_eq!(
        release.lines().next(),
        Some("ckc-tune-inputs\t1\trelease-held-out")
    );
    for forbidden in ["search", "validation"] {
        assert!(!release.contains(forbidden));
    }
}

#[test]
fn tune_manifests_bind_only_search_and_validation_inputs() {
    let table = read("benches/cases/tune-cases.tsv");
    for row in table
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("ckc-"))
    {
        let fields = row.split('\t').collect::<Vec<_>>();
        let case = fields[0];
        let path = repo_root().join("benches/tune/workloads").join(fields[2]);
        let bytes = fs::read(&path).expect("tune manifest");
        let manifest = TuneManifest::parse(&bytes, &path).expect("closed tune manifest");
        assert_eq!(manifest.cases().len(), 2);
        assert_eq!(manifest.timeout_ms(), 30_000);
        let source = String::from_utf8(bytes).expect("UTF-8 manifest");
        for required in [
            format!("id = \"{case}.search\""),
            format!("id = \"{case}.validation\""),
            format!("expected_digest = \"{}\"", fields[5]),
            format!("expected_digest = \"{}\"", fields[8]),
        ] {
            assert!(source.contains(&required), "{path:?} omitted {required}");
        }
        assert!(!source.contains("release-held-out"));
        assert!(!source.contains(fields[11]));
        assert!(Path::new(fields[1]).is_relative());
        assert!(repo_root().join(fields[1]).is_file());
    }
}

#[test]
fn tune_schema_nine_scripts_pin_collector_checker_and_archive_roles() {
    let combined = [
        read("specs/0.14/performance-schema-9.md"),
        read("scripts/measure-v014-performance.py"),
        read("scripts/check-native-performance.py"),
        read("scripts/package-v014-performance-archive.py"),
        read("scripts/audit-performance-oracles.py"),
        read("scripts/prepare-performance-replay.py"),
    ]
    .join("\n");
    for required in [
        "schemaVersion",
        "candidateVersion",
        "v013ReplayCommit",
        "rotating-six-channel-v1",
        "minimum-then-upper-median",
        "x86-64-v4",
        "aarch64-sve2",
        "CK-V014-PERF-ORDER\\0",
        "CK-V014-PERF-RECIPE\\0",
        "CK-V014-TUNE-SUPERVISOR\\0",
        "--contract-only",
        "--schema-only",
        "--baseline\", choices=(\"0.13\"",
    ] {
        assert!(
            combined.contains(required),
            "missing schema-9 token {required}"
        );
    }
}
