use std::{collections::BTreeSet, fs, path::Path, process::Command};

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn quoted_scalar(block: &str, key: &str) -> String {
    let prefix = format!("{key} = \"");
    block
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or_else(|| panic!("missing {key:?} in provenance block:\n{block}"))
        .to_string()
}

fn quoted_array(block: &str, key: &str) -> Vec<String> {
    let prefix = format!("{key} = [");
    let raw = block
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or_else(|| panic!("missing {key:?} in provenance block:\n{block}"));
    raw.split(',')
        .map(str::trim)
        .map(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or_else(|| panic!("invalid quoted value in {key}: {value}"))
                .to_string()
        })
        .collect()
}

fn sha256(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        )
    )
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
    for required in [
        "native/runtime/common/runtime.c",
        "native/runtime/common/format_int.c",
        "native/runtime/common/format_float.c",
        "native/runtime/vendor/ryu/d2s.c",
        "native/runtime/linux/syscalls.S",
        "runtime_sha256",
    ] {
        assert!(unix.contains(required), "Unix bootstrap missing {required}");
    }

    let windows = read("scripts/bootstrap-llvm.ps1");
    assert!(windows.contains("core\", \"native\", \"orcjit\", \"nativecodegen\", \"lto"));
    assert!(!windows.contains("--libnames all"));
    for required in [
        "native/runtime/windows/process.c",
        "native/runtime/platform/kernel32.def",
        "runtime_platform_import_sha256",
        "llvm-lib.exe",
    ] {
        assert!(
            windows.contains(required),
            "Windows bootstrap missing {required}"
        );
    }
}

#[test]
fn release_toolchain_should_static_link_non_system_cpp_runtimes() {
    let unix = read("scripts/bootstrap-llvm.sh");
    assert!(
        unix.contains("LLVM_STATIC_LINK_CXX_STDLIB=ON"),
        "Linux LLVM bootstrap must request a static C++ standard library"
    );

    let windows = read("scripts/bootstrap-llvm.ps1");
    assert!(
        windows.contains("LLVM_USE_CRT_RELEASE=MT"),
        "Windows LLVM bootstrap must use the static release CRT"
    );

    let build = read("build.rs");
    for required in [
        "cpp_link_stdlib(None)",
        "static_crt(true)",
        "cargo::rustc-link-lib=static=stdc++",
        "cargo::rustc-link-lib=c++",
    ] {
        assert!(
            build.contains(required),
            "native bridge build must contain {required:?}"
        );
    }
}

#[test]
fn native_runtime_should_be_source_owned_hashed_and_auditable() {
    for path in [
        "native/runtime/include/ckc_runtime.h",
        "native/runtime/common/runtime.c",
        "native/runtime/common/format_int.c",
        "native/runtime/common/format_float.c",
        "native/runtime/darwin/process.c",
        "native/runtime/linux/syscalls.S",
        "native/runtime/windows/process.c",
        "native/runtime/platform/libSystem.tbd",
        "native/runtime/platform/kernel32.def",
        "native/runtime/provenance.toml",
        "native/runtime/vendor/ryu/d2s.c",
        "native/runtime/vendor/ryu/LICENSE-Apache2",
        "native/runtime/vendor/ryu/LICENSE-Boost",
        "scripts/audit-native-artifact.sh",
        "scripts/audit-native-artifact.ps1",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }

    let build = read("build.rs");
    for required in [
        "runtime_objects",
        "runtime_sha256",
        "CKC_RUNTIME_OBJECT_",
        "runtime_platform_import_sha256",
        "CKC_RUNTIME_PLATFORM_IMPORT",
    ] {
        assert!(build.contains(required), "build.rs missing {required}");
    }

    let runtime = read("native/runtime/common/runtime.c");
    for code in 1..=6 {
        assert!(runtime.contains(&format!("CKR000{code}:")));
    }
    let combined = [
        runtime,
        read("native/runtime/common/format_int.c"),
        read("native/runtime/common/format_float.c"),
    ]
    .join("\n");
    for forbidden in [
        "malloc(",
        "calloc(",
        "realloc(",
        "free(",
        "printf(",
        "snprintf(",
        "setlocale(",
    ] {
        assert!(
            !combined.contains(forbidden),
            "native runtime must not use {forbidden}"
        );
    }
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
fn cargo_and_rust_provenance_should_be_complete_hashed_and_embedded() {
    let provenance = read("third_party/cargo/provenance.toml");
    let notices = read("THIRD_PARTY_NOTICES.md");
    let lock = read("Cargo.lock");
    let expected: BTreeSet<&str> = [
        "block-buffer",
        "bumpalo",
        "cc",
        "cfg-if",
        "cpufeatures",
        "crypto-common",
        "digest",
        "find-msvc-tools",
        "generic-array",
        "leb128fmt",
        "libc",
        "memchr",
        "proc-macro2",
        "quote",
        "sha2",
        "shlex",
        "syn",
        "thiserror",
        "thiserror-impl",
        "typenum",
        "unicode-ident",
        "unicode-width",
        "version_check",
        "wasm-encoder",
        "wast",
        "wat",
    ]
    .into_iter()
    .collect();
    let blocks: Vec<&str> = provenance.split("[[cargo]]").skip(1).collect();
    let actual: BTreeSet<String> = blocks
        .iter()
        .map(|block| quoted_scalar(block, "name"))
        .collect();
    assert_eq!(
        actual,
        expected.iter().map(ToString::to_string).collect(),
        "Cargo source/build dependency provenance must be exact"
    );

    for block in blocks {
        let name = quoted_scalar(block, "name");
        let version = quoted_scalar(block, "version");
        let checksum = quoted_scalar(block, "crate_sha256");
        let lock_identity = format!("name = \"{name}\"\nversion = \"{version}\"");
        assert!(
            lock.contains(&lock_identity),
            "Cargo.lock is missing {name} {version}"
        );
        assert!(
            lock.contains(&format!("checksum = \"{checksum}\"")),
            "Cargo.lock checksum drift for {name} {version}"
        );

        let license_files = quoted_array(block, "license_files");
        let license_hashes = quoted_array(block, "license_sha256");
        assert_eq!(license_files.len(), license_hashes.len());
        for (path, expected_hash) in license_files.iter().zip(&license_hashes) {
            let path = repo_root().join(path);
            assert!(path.is_file(), "missing license file {}", path.display());
            assert_eq!(
                sha256(&path),
                *expected_hash,
                "stale license file for {name}"
            );
        }
        for required in [&name, &version, &checksum] {
            assert!(
                notices.contains(required),
                "third-party notice index does not reference {name} {required}"
            );
        }
    }

    for required in [
        "version = \"1.90.0\"",
        "rust-src-1.90.0.tar.xz",
        "cde088d57064d151b2236f4619aea4a8207e0709eb3035ddc6617d609ab7d453",
        "third_party/licenses/RUST-COPYRIGHT",
        "third_party/licenses/RUST-LICENSE-MIT",
    ] {
        assert!(
            provenance.contains(required),
            "Rust provenance missing {required:?}"
        );
    }

    let source = read("src/backend/llvm/notices.rs");
    for required in [
        "THIRD_PARTY_NOTICES.md",
        "third_party/cargo/provenance.toml",
        "RUST-COPYRIGHT",
        "RUST-LICENSE-MIT",
        "LICENSE-UNICODE",
    ] {
        assert!(
            source.contains(required),
            "embedded notices missing {required:?}"
        );
    }
    let build = read("build.rs");
    assert!(build.contains("validate_third_party_provenance"));

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("licenses")
        .output()
        .expect("run ckc licenses");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("license output is UTF-8");
    assert!(
        stdout.contains(&notices),
        "ckc licenses must embed the exact third-party notice index"
    );
}

#[test]
fn llvm_and_runtime_provenance_should_pin_sources_and_license_hashes() {
    let llvm = read("native/llvm/manifest.toml");
    let runtime = read("native/runtime/provenance.toml");
    let notices = read("THIRD_PARTY_NOTICES.md");

    for required in [
        "license_files = [\"LICENSE.TXT\", \"LLD-LICENSE.TXT\", \"third-party/BLAKE3-LICENSE\", \"third-party/COPYRIGHT.regex\"]",
        "license_sha256 = [\"3340babe8ac7bc6ae294d93aa01c310a250d43d5b760e5c12954882d4e5c83c7\"",
        "f7891568956e34643eb6a0db1462db30820d40d7266e2a78063f2fe233ece5a0",
        "6a94bedb8b707ed97f6e310d0d015ab14e0683ffa0a612b02958581b9cc9fc0e",
        "0424e57d4303164dc59a8509c20dae0518b853692e5c2b0e98b11816fdbc97c7",
    ] {
        assert!(
            llvm.contains(required),
            "LLVM provenance missing {required:?}"
        );
    }
    for required in [
        "source_sha256 = [\"f50df6ebc19075d2aa7b2ff5114bb6b2d953ee905bf2b2d4d8deb5390a36c631\"",
        "license_sha256 = [\"c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4\"",
        "c9bff75738922193e67fa726fa225535870d2aa1059f91452c411736284ad566",
    ] {
        assert!(
            runtime.contains(required),
            "runtime provenance missing {required:?}"
        );
    }
    for component in ["LLVM 22.1.8", "LLD 22.1.8", "BLAKE3", "regex", "Ryu"] {
        assert!(
            notices.contains(component),
            "third-party notice index must enumerate {component}"
        );
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
