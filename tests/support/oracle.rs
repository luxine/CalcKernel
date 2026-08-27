#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn configured_typescript_root() -> Option<PathBuf> {
    std::env::var_os("CALCKERNEL_TS_ROOT").map(PathBuf::from)
}

pub(crate) fn typescript_root() -> PathBuf {
    configured_typescript_root()
        .expect("CALCKERNEL_TS_ROOT must be configured before using the TypeScript oracle")
}

pub(crate) fn typescript_cli() -> Option<PathBuf> {
    let cli = configured_typescript_root()?.join("dist/src/cli.js");
    cli.exists().then_some(cli)
}

pub(crate) fn configured_clang_oracle() -> Option<PathBuf> {
    std::env::var_os("CKC_CLANG_ORACLE").map(PathBuf::from)
}

pub(crate) fn clang_oracle_22() -> Option<PathBuf> {
    let Some(clang) = configured_clang_oracle() else {
        eprintln!("SKIP: CKC_CLANG_ORACLE is not configured for the development oracle");
        return None;
    };
    assert!(
        clang.is_file(),
        "Clang oracle is missing: {}",
        clang.display()
    );
    let output = Command::new(&clang)
        .arg("--version")
        .output()
        .expect("query configured Clang oracle");
    assert!(output.status.success(), "Clang oracle --version failed");
    let version = String::from_utf8_lossy(&output.stdout);
    assert!(
        version.contains("clang version 22.1.8"),
        "Clang oracle must be pinned to 22.1.8, got: {version}"
    );
    Some(clang)
}
