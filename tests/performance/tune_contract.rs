use std::{collections::BTreeSet, fs, path::Path};

use calckernel::TuneManifest;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
fn predicated_update_assets_should_match_frozen_bytes_and_digests() {
    let source = read("benches/fixtures/tune/predicated_update.ck");
    assert_eq!(
        source,
        "export unsafe fn floyd(distance: slice<f64>, n: u32) -> void\ncontract {\n  requires n <= 65535;\n  effects readwrite(distance);\n}\n{\n  let k: u32 = 0;\n  while k < n {\n    let k_row: u32 = k * n;\n    let i: u32 = 0;\n    while i < n {\n      let i_row: u32 = i * n;\n      let dik: f64 = distance[i_row + k];\n      let j: u32 = 0;\n      while j < n {\n        let index: u32 = i_row + j;\n        let candidate: f64 = dik + distance[k_row + j];\n        let old: f64 = distance[index];\n        if candidate < old {\n          distance[index] = candidate;\n        }\n        j = j + 1;\n      }\n      i = i + 1;\n    }\n    k = k + 1;\n  }\n}\n"
    );
    let inputs = [
        (
            "benches/fixtures/tune/predicated-update-training.tsv",
            "ckc-predicated-inputs\t1\ttraining\npredicated-update\ttrain-floyd-128\t128\t113\n",
        ),
        (
            "benches/fixtures/tune/predicated-update-validation.tsv",
            "ckc-predicated-inputs\t1\tvalidation\npredicated-update\tvalidate-floyd-256\t256\t127\n",
        ),
        (
            "benches/fixtures/tune/predicated-update-release.tsv",
            "ckc-predicated-inputs\t1\trelease-held-out\npredicated-update\trelease-floyd-1024\t1024\t131\n",
        ),
    ];
    for (path, exact) in inputs {
        assert_eq!(read(path), exact);
    }
    let manifest = read("benches/tune/workloads/predicated-update.cktune.toml");
    assert_eq!(
        manifest,
        "schema = 1\n\n[runner]\npath = \"../../../target/release/ckc-tune-runner\"\ninput_root = \"../..\"\nargs = [\"--ck-predicated-tune\"]\ninputs = [\"fixtures/tune/predicated-update-training.tsv\", \"fixtures/tune/predicated-update-validation.tsv\"]\ninherit_env = []\ntimeout_ms = 30000\n\n[[case]]\nid = \"predicated-update.search\"\nrole = \"search\"\nseed = 113\nweight = 1\nexpected_digest = \"42c6b833bf2207f5d0716d249099daf28dcf0250e63dbd2a9a4f438a10a215af\"\n\n[[case]]\nid = \"predicated-update.validation\"\nrole = \"validation\"\nseed = 127\nweight = 1\nexpected_digest = \"8b9f2194f5fe7afdfd1d856689ac288d04b70bf984f2310e7011d2ced391aa10\"\n"
    );
    let parsed = TuneManifest::parse(
        manifest.as_bytes(),
        &repo_root().join("benches/tune/workloads/predicated-update.cktune.toml"),
    )
    .expect("predicated manifest");
    assert_eq!(parsed.cases().len(), 2);
    assert!(!manifest.contains("release-held-out"));

    let contract = read("specs/0.14/predicated-update-performance-1.md");
    for digest in [
        "d6105453012eedb8a8db812555f116dd69ca6a8e0242faf81f10038947581608",
        "e21128a8623d0c072111b02c8a3f1ce3309d12f8bf0b3beddc1e0f6342dfc6c8",
        "4d9a1612967ec78ffb3d0ecc035929bb8217cefc13dfeb3fb2e21989186b055d",
    ] {
        assert!(contract.contains(digest));
    }
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
        read("benches/baselines/v0_13_replay.toml"),
        read("specs/0.14/implementation/00-master-control.md"),
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
        "0f9af4ae032c0c3248caff60993795e669d3f8b4",
        "8f454cac97608432a462d6de89949264d5ab5cc33ee9b94c7cb933829bdb72a0",
    ] {
        assert!(
            combined.contains(required),
            "missing schema-9 token {required}"
        );
    }
}

#[test]
fn predicated_update_contract_should_freeze_the_complete_recipe_and_exact_bench_task() {
    let collector = read("scripts/measure-v014-predicated-update.py");
    let checker = read("scripts/check-v014-predicated-update.py");
    let bench = read("benches/tune_perf.rs");
    for path in [
        "benches/fixtures/tune/predicated_update.ck",
        "benches/fixtures/tune/predicated-update-training.tsv",
        "benches/fixtures/tune/predicated-update-validation.tsv",
        "benches/fixtures/tune/predicated-update-release.tsv",
        "benches/tune/workloads/predicated-update.cktune.toml",
        "benches/tune/runner.rs",
        "benches/tune_perf.rs",
        "scripts/measure-v014-predicated-update.py",
        "scripts/check-v014-predicated-update.py",
        "specs/0.14/offline-autotuning.md",
        "specs/0.14/predicated-update-performance-1.md",
        "LICENSE",
        "THIRD_PARTY_NOTICES.md",
    ] {
        assert!(collector.contains(path), "collector recipe omitted {path}");
        assert!(checker.contains(path), "checker recipe omitted {path}");
    }
    for required in [
        "collect-predicated-update",
        "scripts/measure-v014-predicated-update.py",
        "--task <collect|collect-predicated-update> --out <report.json>",
    ] {
        assert!(
            bench.contains(required),
            "bench dispatch omitted {required}"
        );
    }
}

#[test]
fn predicated_update_contract_should_keep_collection_and_judgment_separate() {
    let collector = read("scripts/measure-v014-predicated-update.py");
    let checker = read("scripts/check-v014-predicated-update.py");
    for forbidden in ["95/100", "102/100", "acceptance"] {
        assert!(
            !collector.to_ascii_lowercase().contains(forbidden),
            "collector contains acceptance token {forbidden:?}"
        );
    }
    assert!(!checker.contains("import measure_v014_predicated_update"));
    for required in [
        "check_recipe",
        "check_command",
        "check_cache_scratch",
        "check_publication_locks",
        "check_decision_and_attestation",
        "check_timing_split",
        "check_evidence_inventory",
    ] {
        assert!(checker.contains(required), "checker omitted {required}");
    }
    #[cfg(unix)]
    for path in [
        "scripts/measure-v014-predicated-update.py",
        "scripts/check-v014-predicated-update.py",
    ] {
        let mode = fs::metadata(repo_root().join(path))
            .expect("script metadata")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "{path} must be executable");
    }
}
