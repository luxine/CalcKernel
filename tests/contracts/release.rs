use std::fs;

use super::support::oracle::repo_root;

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
            "cargo test --locked",
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
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --locked",
        "cargo build --release --locked",
        "ckc --help",
        "shasum -a 256",
        "actions/upload-artifact",
    ] {
        assert!(
            workflow.contains(required),
            "native release workflow must contain {required:?}"
        );
    }

    for forbidden in ["npm publish", "NODE_AUTH_TOKEN", "npm pack", "setup-node"] {
        assert!(
            !workflow.contains(forbidden),
            "native release workflow must not contain {forbidden:?}"
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
