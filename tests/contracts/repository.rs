use std::fs;

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn repository_should_declare_v0_9_everywhere() {
    let cargo = read("Cargo.toml");
    let lock = read("Cargo.lock");
    assert!(cargo.contains("version = \"0.9.0\""));
    assert!(lock.contains("name = \"calckernel\"\nversion = \"0.9.0\""));
    for path in [
        "README.md",
        "README.zh-CN.md",
        "CHANGELOG.md",
        "CHANGELOG.zh-CN.md",
    ] {
        assert!(read(path).contains("0.9.0"), "{path} must name 0.9.0");
    }
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
