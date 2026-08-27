use std::fs;

use calckernel::{SourceFile, check};

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn repository_should_declare_v0_10_everywhere() {
    let cargo = read("Cargo.toml");
    let lock = read("Cargo.lock");
    assert!(cargo.contains("version = \"0.10.0\""));
    assert!(lock.contains("name = \"calckernel\"\nversion = \"0.10.0\""));
    for path in [
        "README.md",
        "README.zh-CN.md",
        "CHANGELOG.md",
        "CHANGELOG.zh-CN.md",
    ] {
        assert!(read(path).contains("0.10.0"), "{path} must name 0.10.0");
    }
}

#[test]
fn release_tree_should_not_keep_temporary_native_toolchain_plans() {
    for forbidden in [
        "docs/compiler/native-toolchain-design.md",
        "docs/zh-CN/compiler/native-toolchain-design.md",
        "docs/compiler/native-toolchain-implementation",
        "docs/zh-CN/compiler/native-toolchain-implementation",
    ] {
        assert!(
            !repo_root().join(forbidden).exists(),
            "temporary design or execution plan must not ship: {forbidden}"
        );
    }
}

#[test]
fn v0_10_compatibility_manifest_should_cover_every_intentional_change() {
    let manifest = read("tests/fixtures/compatibility/v0_10/manifest.toml");
    for id in [
        "native-build-no-clang",
        "native-artifact-kinds",
        "run-main-print",
        "reserved-native-names",
        "build-llvm-deprecation",
        "native-checked-status",
        "single-native-c-abi",
        "host-only-emit-llvm",
        "no-native-intermediates",
        "emit-c-source-only",
        "unaffected-v0-9-source",
    ] {
        assert!(
            manifest.contains(&format!("id = \"{id}\"")),
            "compatibility manifest is missing {id}"
        );
    }
    for line in manifest.lines() {
        let Some(path) = line.trim().strip_prefix("fixture = \"") else {
            continue;
        };
        let path = path.strip_suffix('"').expect("quoted fixture path");
        assert!(
            repo_root().join(path).is_file(),
            "missing compatibility fixture {path}"
        );
    }
    for line in manifest.lines() {
        let Some(evidence) = line.trim().strip_prefix("evidence = \"") else {
            continue;
        };
        let evidence = evidence.strip_suffix('"').expect("quoted evidence");
        let (path, test_name) = evidence
            .split_once(':')
            .expect("evidence uses path:test_name");
        let source = read(path);
        assert!(
            source.contains(&format!("fn {test_name}")),
            "compatibility evidence does not resolve: {evidence}"
        );
    }
    for fixture in [
        "tests/fixtures/compatibility/v0_10/legacy_export.ck",
        "tests/fixtures/compatibility/v0_10/main_print.ck",
        "tests/fixtures/compatibility/v0_10/native_checked.ck",
        "tests/fixtures/compatibility/v0_10/reserved_print.ck",
    ] {
        assert!(repo_root().join(fixture).is_file(), "missing {fixture}");
    }
}

#[test]
fn v0_10_compatibility_sources_should_parse_at_the_frozen_boundary() {
    for fixture in ["legacy_export.ck", "main_print.ck", "native_checked.ck"] {
        let path = format!("tests/fixtures/compatibility/v0_10/{fixture}");
        let source = SourceFile::new(path.clone(), read(&path));
        let result = check(&source);
        assert_eq!(result.diagnostics, [], "{path} must remain accepted");
    }

    let path = "tests/fixtures/compatibility/v0_10/main_print.ck";
    let source = SourceFile::new(path, read(path));
    assert!(
        check(&source).checked_program.entry.is_some(),
        "0.10 entry fixture must classify main"
    );

    let path = "tests/fixtures/compatibility/v0_10/reserved_print.ck";
    let source = SourceFile::new(path, read(path));
    let messages = check(&source)
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        ["Cannot define reserved compiler builtin 'print_i32'."],
        "reserved-name migration diagnostic is frozen"
    );
}

#[test]
fn repository_should_not_ship_historical_or_generated_trees() {
    for forbidden in [
        "Ai_repository",
        "docs/superpowers",
        "docs/releases",
        "docs/bench",
        "bench",
        ".understand-anything",
    ] {
        assert!(!repo_root().join(forbidden).exists(), "remove {forbidden}");
    }
}

#[test]
fn repository_policy_should_keep_agent_planning_local_and_docs_current() {
    let ignore = read(".gitignore");
    for required in [
        "/target/",
        "/build/",
        "/.worktrees/",
        "/Ai_repository/",
        "/.DS_Store",
        "*.tmp",
        ".understand-anything",
    ] {
        assert!(
            ignore.lines().any(|line| line == required),
            "ignore {required}"
        );
    }

    let policy = read("AGENTS.md");
    for required in [
        "CK / CalcKernel",
        "`ckc`",
        "`.ck`",
        "docs/zh-CN/",
        "tests/README.md",
        "must never be committed or shipped",
    ] {
        assert!(
            policy.contains(required),
            "AGENTS.md must contain {required:?}"
        );
    }
}
