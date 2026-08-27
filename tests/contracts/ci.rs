use std::fs;

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn assert_actions_are_commit_pinned(workflow: &str, subject: &str) {
    for line in workflow.lines().map(str::trim) {
        let Some(reference) = line.strip_prefix("- uses: ") else {
            continue;
        };
        if reference.starts_with("./") {
            continue;
        }
        let revision = reference
            .rsplit_once('@')
            .unwrap_or_else(|| panic!("{subject} action is missing a revision: {reference}"))
            .1;
        assert!(
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{subject} action must be pinned to a full commit: {reference}"
        );
    }
}

#[test]
fn daily_ci_should_keep_fast_quality_independent_of_llvm() {
    let workflow = read(".github/workflows/ci.yml");

    for required in [
        "name: CI",
        "pull_request:\n    branches: [main]",
        "push:\n    branches: [main]",
        "contents: read",
        "cancel-in-progress: true",
        "quality:",
        "name: quality (no native toolchain)",
        "runs-on: ubuntu-24.04",
        "repository: luxine/CalcKernel",
        "ref: 5e989939d89d75056e5f3bea25f3bf7204d5529a",
        "corepack prepare pnpm@9.15.9 --activate",
        "cargo fmt --check",
        "cargo clippy --all-targets --locked -- -D warnings",
        "cargo test --locked",
        "cargo build --release --locked",
        "./target/release/ckc emit-c",
        "./target/release/ckc emit-wasm",
    ] {
        assert!(
            workflow.contains(required),
            "daily CI must contain {required:?}"
        );
    }

    let quality = workflow
        .split("  native-integration:")
        .next()
        .expect("quality job before native integration");
    for forbidden in [
        "--all-features",
        "CKC_LLVM_PREFIX",
        "bootstrap-ckc-llvm",
        "emit-llvm",
    ] {
        assert!(
            !quality.contains(forbidden),
            "fast quality job must remain LLVM-independent: {forbidden}"
        );
    }
}

#[test]
fn daily_ci_should_gate_native_integration_and_all_release_hosts() {
    let workflow = read(".github/workflows/ci.yml");

    for required in [
        "native-integration:",
        "name: native integration",
        "uses: ./.github/actions/bootstrap-ckc-llvm",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --all-features --locked",
        "bridge ownership under ASan and UBSan",
        "-fsanitize=address,undefined",
        "cargo test --all-features --locked --test native artifacts",
        "scripts/audit-native-artifact",
        "scripts/audit-jit-memory",
        "native-hosts:",
        "name: native host (${{ matrix.target }})",
        "cargo test --all-features --locked --test native",
        "cargo test --all-features --locked --test cli",
        "darwin-arm64",
        "darwin-x64",
        "linux-arm64",
        "linux-x64",
        "win32-arm64",
        "win32-x64",
        "performance:",
        "name: performance (${{ matrix.arch }})",
        "arch: x86-64",
        "arch: AArch64",
        "profile: oracle",
        "--cpu baseline",
        "--cpu native",
        "scripts/check-native-performance.py",
    ] {
        assert!(
            workflow.contains(required),
            "native CI contract must contain {required:?}"
        );
    }

    for forbidden in [
        "gh release upload",
        "publish-release:",
        "build-artifacts:",
        "tags:",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "daily CI workflow must not publish releases: {forbidden:?}"
        );
    }
    assert_actions_are_commit_pinned(&workflow, "daily CI");
}

#[test]
fn performance_ci_should_select_a_case_from_the_benchmark_manifest() {
    let workflow = read(".github/workflows/ci.yml");
    let cases = read("benches/cases/native-cases.tsv");
    let known = cases
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('\t').next())
        .collect::<Vec<_>>();

    let mut selected = Vec::new();
    for line in workflow.lines().filter(|line| line.contains("cargo bench")) {
        let words = line.split_whitespace().collect::<Vec<_>>();
        if let Some(index) = words.iter().position(|word| *word == "--case") {
            selected.push(*words.get(index + 1).expect("--case value"));
        }
    }
    assert!(
        !selected.is_empty(),
        "performance CI must select a benchmark case"
    );
    for name in selected {
        assert!(
            known.contains(&name),
            "performance CI selected unknown benchmark case {name:?}"
        );
    }
}

#[test]
fn native_bootstrap_action_should_pin_and_cache_the_manifest_source() {
    let action = read(".github/actions/bootstrap-ckc-llvm/action.yml");

    for required in [
        "native/llvm/manifest.toml",
        "llvm-project-22.1.8.src.tar.xz",
        "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888",
        "actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830",
        "scripts/bootstrap-llvm.sh",
        "scripts/bootstrap-llvm.ps1",
        "cache-hit",
        "CKC_LLVM_PREFIX",
        "CKC_CLANG_ORACLE",
        "llvm-build.toml",
        "llvm-config",
    ] {
        assert!(
            action.contains(required),
            "native bootstrap action must contain {required:?}"
        );
    }
    assert_actions_are_commit_pinned(&action, "native bootstrap");
}
