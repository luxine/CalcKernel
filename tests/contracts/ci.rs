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
        "workflow_dispatch:",
        "workflow_call:",
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
    assert_eq!(
        workflow.matches("CALCKERNEL_TS_ROOT:").count(),
        1,
        "the optional TypeScript oracle root must be scoped only to its owning job"
    );
    let workflow_header = workflow
        .split("jobs:")
        .next()
        .expect("workflow header before jobs");
    assert!(
        !workflow_header.contains("CALCKERNEL_TS_ROOT:"),
        "optional TypeScript oracle configuration must not leak into native jobs"
    );
    assert!(
        quality.contains("CALCKERNEL_TS_ROOT: ${{ github.workspace }}/typescript-oracle"),
        "quality must configure the exact TypeScript oracle checkout it owns"
    );
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
        "scripts/test-sanitized-ownership.sh",
        "cargo test --all-features --locked --test native artifacts",
        "scripts/audit-native-artifact",
        "scripts/audit-jit-memory",
        "native-hosts:",
        "name: native host (${{ matrix.target }})",
        "cargo test --all-features --locked --test native",
        "cargo test --all-features --locked --test native fact_audit_ -- --nocapture",
        "fact-audit-${{ matrix.name }}",
        "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
        "cargo test --all-features --locked --test cli",
        "RUSTFLAGS: -C target-feature=+crt-static",
        "scripts/audit-ckc-release.sh '${{ matrix.executable }}'",
        "scripts/audit-ckc-release.ps1 -Path '${{ matrix.executable }}'",
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
        "--case proof",
        "--cpu baseline",
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
fn native_host_ci_should_preserve_parallel_failures_before_darwin_diagnostics() {
    let workflow = read(".github/workflows/ci.yml");
    let suite = workflow
        .split("      - name: Run required native suite\n")
        .nth(1)
        .expect("native hosts must name the required suite")
        .split("      - name:")
        .next()
        .expect("required native suite step");
    assert!(suite.contains("id: native-suite"));
    assert!(suite.contains("RUST_BACKTRACE: 1"));
    assert!(suite.contains("cargo test --all-features --locked --test native -- --nocapture"));
    assert!(!suite.contains("continue-on-error"));
    assert!(!suite.contains("--test-threads"));

    let diagnostic = workflow
        .split("      - name: Diagnose failed Darwin native suite\n")
        .nth(1)
        .expect("failed Darwin suites need crash diagnostics")
        .split("      - name:")
        .next()
        .expect("Darwin diagnostic step");
    assert!(diagnostic.contains(
        "if: failure() && runner.os == 'macOS' && steps.native-suite.outcome == 'failure'"
    ));
    assert!(diagnostic.contains("timeout-minutes: 15"));
    assert!(
        diagnostic.contains("bash scripts/diagnose-native-darwin.sh target/native-diagnostics")
    );
    assert!(workflow.contains("name: native-diagnostics-${{ matrix.name }}"));
    assert!(workflow.contains("path: target/native-diagnostics"));
}

#[test]
fn darwin_crash_diagnostics_should_capture_serial_and_parallel_backtraces() {
    let script = read("scripts/diagnose-native-darwin.sh");
    for required in [
        "--message-format=json",
        ".target.name == \"native\"",
        "--one-line-on-crash 'thread backtrace all'",
        "settings set target.disable-aslr false",
        "--test-threads=1 --nocapture",
        "replay parallel --nocapture",
        "DiagnosticReports",
        "native-*.ips",
        "program-*.ips",
    ] {
        assert!(
            script.contains(required),
            "crash diagnostics must include {required:?}"
        );
    }
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

    let preserve = workflow
        .find("cp target/ckc-perf/results.json target/ckc-perf/results-baseline.json")
        .expect("performance CI must preserve its raw report");
    let gate = workflow
        .find("python3 scripts/check-native-performance.py target/ckc-perf/results.json")
        .expect("performance CI must run the strict checker");
    assert!(
        preserve < gate,
        "performance CI must preserve raw evidence before a failing gate"
    );
}

#[test]
fn native_bootstrap_action_should_pin_and_cache_the_manifest_source() {
    let action = read(".github/actions/bootstrap-ckc-llvm/action.yml");

    for required in [
        "native/llvm/manifest.toml",
        "native/runtime/**/*.c",
        "native/runtime/**/*.h",
        "native/runtime/**/*.S",
        "native/runtime/**/*.def",
        "native/runtime/**/*.tbd",
        "recipe-digest",
        "ckc-llvm-v3-",
        "llvm-project-22.1.8.src.tar.xz",
        "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888",
        "actions/cache/restore@0057852bfaa89a56745cba8c7296529d2fc39830",
        "actions/cache/save@0057852bfaa89a56745cba8c7296529d2fc39830",
        "Save manifest-addressed LLVM prefix",
        "Save manifest-addressed Clang oracle prefix",
        "scripts/bootstrap-llvm.sh",
        "scripts/bootstrap-llvm.ps1",
        "cache-hit",
        "CKC_LLVM_PREFIX",
        "CKC_CLANG_ORACLE",
        "llvm-build.toml",
        "LLVMDTLTO",
        "llvm-config",
    ] {
        assert!(
            action.contains(required),
            "native bootstrap action must contain {required:?}"
        );
    }
    assert_eq!(
        action.matches("actions/cache/restore@").count(),
        2,
        "release and oracle prefixes must use restore-only cache actions"
    );
    assert_eq!(
        action.matches("actions/cache/save@").count(),
        2,
        "release and oracle prefixes must be saved explicitly"
    );
    assert!(
        !action.contains("uses: actions/cache@"),
        "explicit cache saves must not be paired with a duplicate post-job cache action"
    );
    let validation = action
        .find("- name: Validate cached or built prefix")
        .expect("native bootstrap action must validate prefixes");
    let release_save = action
        .find("- name: Save manifest-addressed LLVM prefix")
        .expect("native bootstrap action must save the release prefix");
    let oracle_save = action
        .find("- name: Save manifest-addressed Clang oracle prefix")
        .expect("native bootstrap action must save the oracle prefix");
    assert!(
        validation < release_save && validation < oracle_save,
        "prefix caches must be saved only after manifest and object validation"
    );
    assert_actions_are_commit_pinned(&action, "native bootstrap");
}

#[test]
fn registered_release_workflow_should_dispatch_feature_candidate_ci_without_publishing() {
    let workflow = read(".github/workflows/native-release.yml");

    for required in [
        "candidate_ci:",
        "Run the feature-branch candidate CI without publishing",
        "if: inputs.candidate_ci == true",
        "uses: ./.github/workflows/ci.yml",
        "inputs.candidate_ci != true",
    ] {
        assert!(
            workflow.contains(required),
            "registered CI dispatcher must contain {required:?}"
        );
    }

    for forbidden in ["capture_v010_baselines", "capture-v010-baselines"] {
        assert!(
            !workflow.contains(forbidden),
            "registered CI dispatcher must not retain temporary baseline capture input {forbidden:?}"
        );
    }
}

#[test]
fn performance_failures_should_keep_same_worker_v010_diagnostics_without_bypassing_the_gate() {
    let workflow = read(".github/workflows/ci.yml");
    let performance = workflow
        .split("  performance:")
        .nth(1)
        .expect("performance job");
    assert!(performance.contains("id: performance-gate"));
    assert!(workflow.contains("performance_diagnostics:"));
    assert!(performance.contains("inputs.performance_diagnostics == true"));
    assert!(performance.contains("if: always() && (steps.performance-gate.outcome == 'failure' || inputs.performance_diagnostics == true)"));
    assert!(performance.contains("path: target/performance-v010-diagnostic"));
    assert!(performance.contains("ref: df816502876fba41676f9ebc190e4fadd18cd5a5"));
    assert!(performance.contains("bash scripts/diagnose-native-performance.sh"));
    assert!(!performance.contains("continue-on-error: true"));
    let script = read("scripts/diagnose-native-performance.sh");
    for required in [
        "df816502876fba41676f9ebc190e4fadd18cd5a5",
        "lscpu --json",
        "sha256sum",
        "cargo bench --features native-toolchain --bench ckc_perf -- --task check --cpu baseline",
        "v0_10_proof_loop_harness.patch",
        "v0_10_mir_optimizer_harness.patch",
        "v0_10_linux_cpp_runtime_harness.patch",
        "v0_10_clang_cpu_harness.patch",
    ] {
        assert!(
            script.contains(required),
            "performance diagnostics must contain {required}"
        );
    }
    assert!(
        !script.contains("check-native-performance.py"),
        "diagnostics must not replace the required gate"
    );
}
