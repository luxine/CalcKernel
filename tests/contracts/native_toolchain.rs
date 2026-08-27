use std::{fs, process::Command};

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn native_toolchain_cargo_profile_should_be_explicit_and_optional() {
    let cargo = read("Cargo.toml");

    assert!(
        cargo.contains("native-toolchain = []"),
        "Cargo.toml must declare an opt-in native-toolchain feature"
    );
    assert!(
        repo_root().join("build.rs").is_file(),
        "native bootstrap validation belongs in build.rs"
    );
}

#[test]
fn native_toolchain_manifest_should_pin_the_official_llvm_source() {
    let manifest = read("native/llvm/manifest.toml");

    for required in [
        "version = \"22.1.8\"",
        "tag = \"llvmorg-22.1.8\"",
        "commit = \"ca7933e47d3a3451d81e72ac174dcb5aa28b59d1\"",
        "archive = \"llvm-project-22.1.8.src.tar.xz\"",
        "sha256 = \"922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888\"",
        "host_only = true",
        "static_only = true",
        "include_clang = false",
        "link_components = [\"core\", \"native\", \"orcjit\", \"nativecodegen\", \"lto\"]",
        "minimum_os = \"11.0\"",
        "[profiles.oracle]",
        "include_clang = true",
    ] {
        assert!(
            manifest.contains(required),
            "native LLVM manifest must contain {required:?}"
        );
    }
}

#[test]
fn native_toolchain_bootstrap_should_cover_unix_and_windows() {
    for path in ["scripts/bootstrap-llvm.sh", "scripts/bootstrap-llvm.ps1"] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
    let unix = read("scripts/bootstrap-llvm.sh");
    assert!(unix.contains("CMAKE_OSX_DEPLOYMENT_TARGET=11.0"));
    assert!(unix.contains("ckc_components=(core native orcjit nativecodegen lto)"));
    assert!(!unix.contains("--libnames all"));

    let windows = read("scripts/bootstrap-llvm.ps1");
    assert!(windows.contains("core\", \"native\", \"orcjit\", \"nativecodegen\", \"lto"));
    assert!(!windows.contains("--libnames all"));
}

#[test]
fn native_toolchain_notices_should_be_repository_owned() {
    for path in [
        "native/llvm/LICENSE.TXT",
        "native/llvm/LLD-LICENSE.TXT",
        "native/llvm/third-party/BLAKE3-LICENSE",
        "native/llvm/third-party/COPYRIGHT.regex",
        "src/backend/llvm/notices.rs",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
}

#[test]
fn native_toolchain_bootstrap_outputs_should_remain_untracked() {
    let ignore = read(".gitignore");

    assert!(
        ignore.lines().any(|line| line == "/build/llvm/"),
        "ignore deterministic LLVM bootstrap outputs"
    );
    let tracked = Command::new("git")
        .arg("ls-files")
        .arg("build/llvm")
        .current_dir(repo_root())
        .output()
        .expect("inspect tracked LLVM bootstrap outputs");
    assert!(tracked.status.success(), "git ls-files failed");
    assert!(
        tracked.stdout.is_empty(),
        "LLVM bootstrap output must not be tracked in the source tree"
    );
}

#[test]
fn native_toolchain_bridge_should_define_owned_c_abi_results() {
    let header = read("native/bridge/ckc_llvm.h");

    for required in [
        "CKC_LLVM_BRIDGE_ABI_VERSION",
        "CkcLlvmOwnedBytes",
        "CkcLlvmError",
        "ckc_llvm_bridge_info",
        "ckc_llvm_test_error",
        "ckc_llvm_owned_bytes_dispose",
        "static_assert",
        "_Static_assert",
    ] {
        assert!(
            header.contains(required),
            "native bridge header must contain {required:?}"
        );
    }

    for path in [
        "native/bridge/ckc_llvm.cpp",
        "native/bridge/ownership_smoke.cpp",
        "src/backend/llvm/ffi.rs",
        "src/backend/llvm/error.rs",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
}
