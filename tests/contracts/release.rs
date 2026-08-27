use std::{fs, process::Command};

use super::support::oracle::repo_root;

fn assert_actions_are_commit_pinned(workflow: &str) {
    for line in workflow.lines().map(str::trim) {
        let Some(reference) = line.strip_prefix("- uses: ") else {
            continue;
        };
        if reference.starts_with("./") {
            continue;
        }
        let revision = reference
            .rsplit_once('@')
            .unwrap_or_else(|| panic!("release action is missing a revision: {reference}"))
            .1;
        assert!(
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "release action must be pinned to a full commit: {reference}"
        );
    }
}

#[test]
fn release_surface_should_not_include_npm_or_javascript_compatibility_layer() {
    let root = repo_root();
    let forbidden_paths = [
        "package.json",
        "npm",
        ".github/workflows/npm-release.yml",
        "docs/npm-release.md",
        "docs/typescript-test-surface.json",
        "scripts/audit-npm-release-workflow.mjs",
        "scripts/build-npm-binary-matrix.mjs",
        "scripts/cleanup-npm-package.mjs",
        "scripts/prepare-npm-package.mjs",
        "scripts/verify-declaration-parity.mjs",
        "scripts/verify-host-npm-install.mjs",
        "scripts/verify-npm-cutover-evidence.mjs",
        "scripts/verify-npm-publish-artifact.mjs",
        "scripts/verify-npm-publish-result.mjs",
        "scripts/verify-npm-registry-replacement.mjs",
        "scripts/verify-npm-release-signoff-summary.mjs",
        "scripts/verify-npm-release-signoff.mjs",
        "scripts/verify-npm-release.mjs",
        "scripts/verify-public-api-parity.mjs",
    ];

    let present: Vec<&str> = forbidden_paths
        .into_iter()
        .filter(|path| root.join(path).exists())
        .collect();

    assert!(
        present.is_empty(),
        "native ckc release surface must not include npm/JS compatibility files:\n{}",
        present.join("\n")
    );
}

#[test]
fn v0_10_release_identity_should_be_consistent_everywhere() {
    let cargo = fs::read_to_string(repo_root().join("Cargo.toml")).expect("read Cargo.toml");
    let lock = fs::read_to_string(repo_root().join("Cargo.lock")).expect("read Cargo.lock");
    assert!(cargo.contains("version = \"0.10.0\""));
    assert!(lock.contains("name = \"calckernel\"\nversion = \"0.10.0\""));

    for path in [
        "README.md",
        "README.zh-CN.md",
        "CHANGELOG.md",
        "CHANGELOG.zh-CN.md",
    ] {
        assert!(
            fs::read_to_string(repo_root().join(path))
                .unwrap_or_else(|error| panic!("read {path}: {error}"))
                .contains("0.10.0"),
            "{path} must identify 0.10.0"
        );
    }

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("--version")
        .output()
        .expect("run ckc --version");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ckc 0.10.0");

    let workflow = fs::read_to_string(repo_root().join(".github/workflows/native-release.yml"))
        .expect("read release workflow");
    for required in [
        "expected_tag=\"v${cargo_version}\"",
        "--version --verbose",
        "Native ABI",
        "Runtime ABI",
        "ckc-darwin-arm64.tar.gz",
        "ckc-win32-x64.zip",
    ] {
        assert!(
            workflow.contains(required),
            "release identity missing {required:?}"
        );
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn v0_10_verbose_identity_should_report_frozen_native_abis() {
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(["--version", "--verbose"])
        .output()
        .expect("run ckc --version --verbose");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("verbose version is UTF-8");
    for required in [
        "ckc 0.10.0",
        "Native ABI: 1",
        "Runtime ABI: 1",
        "LLVM: 22.1.8",
        "Target:",
        "Code generator:",
        "ORC object layer:",
    ] {
        assert!(
            stdout.contains(required),
            "verbose identity missing {required:?}"
        );
    }
}

#[test]
fn native_release_docs_should_replace_npm_release_docs() {
    let root = repo_root();
    let docs = [
        (
            root.join("docs/project/release.md"),
            root.join("docs/project/release-checklist.md"),
        ),
        (
            root.join("docs/zh-CN/project/release.md"),
            root.join("docs/zh-CN/project/release-checklist.md"),
        ),
    ];

    for (policy, checklist) in docs {
        let text = [policy.as_path(), checklist.as_path()]
            .into_iter()
            .map(|doc| {
                fs::read_to_string(doc)
                    .unwrap_or_else(|error| panic!("read {}: {error}", doc.display()))
            })
            .collect::<Vec<_>>()
            .join("\n");

        for required in [
            "native ckc",
            "cargo build --release",
            "SHA256",
            "GitHub Release",
            "cargo test --all-features --locked",
        ] {
            assert!(
                text.contains(required),
                "{} and {} must document {required:?}",
                policy.display(),
                checklist.display()
            );
        }

        assert!(
            !text.to_ascii_lowercase().contains("npm"),
            "{} and {} must not describe npm package publishing",
            policy.display(),
            checklist.display()
        );
    }
}

#[test]
fn native_release_workflow_should_build_sign_and_archive_native_ckc_artifacts() {
    let workflow_path = repo_root().join(".github/workflows/native-release.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    for required in [
        "name: native ckc release",
        "tags:\n      - \"v*\"",
        "workflow_dispatch:",
        "default: false",
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --all-features --locked",
        "cargo build --release --features native-toolchain --locked",
        "-C target-feature=+crt-static",
        "uses: ./.github/actions/bootstrap-ckc-llvm",
        "profile: release",
        "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888",
        "ckc --help",
        "--version --verbose",
        "' licenses",
        "' run ",
        "--kind executable",
        "audit-ckc-release",
        "audit-native-artifact",
        "audit-jit-memory",
        "GITHUB_REF_NAME",
        "expected_tag=\"v${cargo_version}\"",
        "needs: build-artifacts",
        "if: startsWith(github.ref, 'refs/tags/')",
        "command -v sha256sum",
        "sha256sum",
        "shasum -a 256",
        "actions/upload-artifact",
        "actions/download-artifact",
        "gh release view",
        "gh release create \"${GITHUB_REF_NAME}\" release-artifacts/*",
        "--verify-tag",
        "--title \"CalcKernel ${GITHUB_REF_NAME}\"",
        "--notes-file CHANGELOG.md",
    ] {
        assert!(
            workflow.contains(required),
            "native release workflow must contain {required:?}"
        );
    }

    for archive in [
        "ckc-darwin-arm64.tar.gz",
        "ckc-darwin-x64.tar.gz",
        "ckc-linux-arm64.tar.gz",
        "ckc-linux-x64.tar.gz",
        "ckc-win32-arm64.zip",
        "ckc-win32-x64.zip",
    ] {
        assert_eq!(
            workflow.matches(archive).count(),
            1,
            "native release workflow must declare {archive} exactly once"
        );
    }

    assert_eq!(
        workflow.matches("contents: write").count(),
        1,
        "write permission must exist only on the publish job"
    );

    for forbidden in [
        "npm publish",
        "NODE_AUTH_TOKEN",
        "npm pack",
        "setup-node",
        "CALCKERNEL_TS_ROOT",
        "repository: luxine/CalcKernel",
        "gh release upload",
        "--clobber",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "native release workflow must not contain {forbidden:?}"
        );
    }

    assert_actions_are_commit_pinned(&workflow);

    for audit in [
        "scripts/audit-ckc-release.sh",
        "scripts/audit-ckc-release.ps1",
    ] {
        assert!(
            repo_root().join(audit).is_file(),
            "release dependency audit is missing {audit}"
        );
    }
    let jit_audit = fs::read_to_string(repo_root().join("scripts/audit-jit-memory.sh"))
        .expect("read Darwin JIT release audit");
    for required in [
        "codesign --verify --strict",
        "com.apple.security.cs.allow-jit",
    ] {
        assert!(
            jit_audit.contains(required),
            "Darwin JIT release audit must contain {required:?}"
        );
    }
}

#[test]
fn repository_should_not_keep_javascript_helper_scripts() {
    let scripts_dir = repo_root().join("scripts");
    if !scripts_dir.exists() {
        return;
    }

    let javascript_scripts: Vec<String> = fs::read_dir(&scripts_dir)
        .expect("read scripts directory")
        .map(|entry| entry.expect("read script entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("mjs"))
        .map(|path| {
            path.strip_prefix(repo_root())
                .expect("script under repo root")
                .display()
                .to_string()
        })
        .collect();

    assert!(
        javascript_scripts.is_empty(),
        "native ckc repository must not keep JavaScript helper scripts:\n{}",
        javascript_scripts.join("\n")
    );
}
