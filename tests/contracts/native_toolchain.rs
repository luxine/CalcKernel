use std::{collections::BTreeSet, fs, path::Path, process::Command};

#[cfg(unix)]
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

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
fn sanitizer_bridge_build_should_link_the_platform_runtime_explicitly() {
    let build = read("build.rs");
    let script = read("scripts/test-sanitized-ownership.sh");
    for required in [
        "configure_sanitizer_linkage(target)",
        "cargo::rustc-link-lib=asan",
        "cargo::rustc-link-lib=ubsan",
    ] {
        assert!(
            build.contains(required),
            "sanitized bridge linkage must contain {required:?}"
        );
    }
    for required in [
        "detect_leaks=1:halt_on_error=1",
        "sanitized ownership is a Linux-only gate",
        "$(uname -s)\" == Linux",
    ] {
        assert!(
            script.contains(required),
            "sanitized ownership runner must contain {required:?}"
        );
    }
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
    assert!(
        unix.contains("if [[ -n \"$ckc_jobs\" ]]; then")
            && unix.contains("cmake --build \"$ckc_build_dir/build\" --parallel \"$ckc_jobs\"")
            && !unix.contains("ckc_parallel_args"),
        "Unix bootstrap must not expand an empty array under macOS Bash 3.2 set -u"
    );
    assert!(
        unix.contains("ckc_static_libs=(\"${ckc_lld_libs[@]}\" LLVMDTLTO \"${ckc_llvm_libs[@]}\")"),
        "Unix bootstrap must add LLVM 22 DTLTO after LLD and before its LLVM dependencies"
    );
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
    for required in [
        "vswhere.exe",
        "VsDevCmd.bat",
        "Import-MsvcEnvironment",
        "$msvcHostArch = \"amd64\"",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "Microsoft.VisualStudio.Component.VC.Tools.ARM64",
        "VSCMD_ARG_TGT_ARCH",
        "CKC_MSVC_TARGET",
        "Get-Command cl.exe",
        "Get-Command link.exe",
        "CMAKE_C_COMPILER_ID",
        "CMAKE_CXX_COMPILER_ID",
    ] {
        assert!(
            windows.contains(required),
            "Windows bootstrap must import and validate the target MSVC environment with {required}"
        );
    }
    for required in ["-DCMAKE_C_COMPILER=cl.exe", "-DCMAKE_CXX_COMPILER=cl.exe"] {
        assert!(
            windows.contains(required),
            "Windows bootstrap must bind CMake to the MSVC compiler with {required}"
        );
    }
    assert!(
        windows.contains("New-Item -ItemType Directory -Path $manifestDir -Force"),
        "Windows bootstrap must tolerate the runtime step having already created share/ckc"
    );
    assert!(
        windows.contains(
            "$staticLibraries = @(\"lldCOFF\", \"lldCommon\", \"LLVMDTLTO\") + $llvmLibraries"
        ),
        "Windows bootstrap must add LLVM 22 DTLTO after LLD and before its LLVM dependencies"
    );
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
        "fn link_static_linux_cpp_runtime",
        "-print-file-name=libstdc++.a",
        "cargo::rustc-link-search=native=",
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

    let unix_audit = read("scripts/audit-native-artifact.sh");
    for line in unix_audit.lines().filter(|line| line.contains('|')) {
        assert!(
            !line.contains("grep -q") && !line.contains("grep -Eiq"),
            "pipefail audit must not use early-exit grep in a pipeline: {line}"
        );
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

    let darwin_runtime = read("native/runtime/darwin/process.c");
    for required in [
        "___ck_start:",
        "andq $-16, %rsp",
        "callq _main",
        "callq ___ck_platform_exit",
        "bl _main",
        "bl ___ck_platform_exit",
    ] {
        assert!(
            darwin_runtime.contains(required),
            "Darwin freestanding runtime must provide an ABI-safe process entry containing {required:?}"
        );
    }
    let bridge = read("native/bridge/ckc_llvm.cpp");
    assert!(
        bridge.contains("arguments.emplace_back(\"___ck_start\")"),
        "Darwin LLD must enter through the runtime stack-normalizing stub"
    );
    assert!(
        !bridge.contains("arguments.emplace_back(\"_main\")"),
        "Darwin LLD must not expose the C ABI main body as a raw process entry"
    );
}

#[cfg(unix)]
fn write_executable(path: &Path, source: &str) {
    fs::write(path, source).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
}

#[cfg(unix)]
fn mocked_elf_audit_root() -> (PathBuf, PathBuf) {
    let root = super::support::temp::temp_dir("ckc-elf-audit-contract");
    let artifacts = root.join("artifacts");
    let runtime = artifacts.join("runtime");
    let tools = root.join("tools");
    fs::create_dir_all(&runtime).expect("create mock runtime artifacts");
    fs::create_dir_all(&tools).expect("create mock audit tools");
    for relative in [
        "module.o",
        "libmodule.a",
        "libmodule.so",
        "program",
        "runtime/runtime.o",
        "runtime/SHA256SUMS",
    ] {
        fs::write(artifacts.join(relative), b"fixture").expect("write mock artifact");
    }

    let dispatcher = r#"#!/usr/bin/env bash
set -euo pipefail
case "$(basename "$0")" in
  uname) printf 'Linux\n' ;;
  sha256sum) exit 0 ;;
  file) printf '%s: current ar archive\n' "$1" ;;
  nm)
    if [[ " $* " == *' -D --defined-only '* ]]; then
      printf '00000000 T answer\n'
    fi
    ;;
  readelf)
    case "$1:$2" in
      -h:*module.o) printf '  Type:                              REL (Relocatable file)\n' ;;
      -h:*libmodule.so) printf '  Type:                              DYN (Shared object file)\n' ;;
      -h:*program) printf '  Type:                              EXEC (Executable file)\n' ;;
      -d:*) printf 'There is no dynamic section in this file.\n' ;;
      -p:.comment)
        printf "String dump of section '.comment':\n  [     0]  %s\n" "${CKC_TEST_ELF_COMMENT:-Linker: LLD 22.1.8}"
        ;;
      -SW:*) printf '  [ 1] .comment PROGBITS 00000000 000040 000013 01  %s  0   0  1\n' "${CKC_TEST_ELF_COMMENT_FLAGS:-MS}" ;;
      *) printf 'unexpected readelf arguments: %s\n' "$*" >&2; exit 64 ;;
    esac
    ;;
  *) printf 'unexpected mock tool: %s\n' "$0" >&2; exit 64 ;;
esac
"#;
    for tool in ["uname", "sha256sum", "file", "nm", "readelf"] {
        write_executable(&tools.join(tool), dispatcher);
    }
    (artifacts, tools)
}

#[cfg(unix)]
fn run_mocked_elf_audit(comment: &str, flags: &str) -> std::process::Output {
    let (artifacts, tools) = mocked_elf_audit_root();
    let inherited_path = std::env::var_os("PATH").expect("PATH must be set");
    let path = std::env::join_paths(
        std::iter::once(tools.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("construct mock PATH");
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/audit-native-artifact.sh"))
        .arg(&artifacts)
        .env("PATH", path)
        .env("CKC_TEST_ELF_COMMENT", comment)
        .env("CKC_TEST_ELF_COMMENT_FLAGS", flags)
        .output()
        .expect("run native ELF audit with pinned LLD provenance");
    let root = artifacts.parent().expect("mock audit parent");
    let _ = fs::remove_dir_all(root);
    output
}

#[cfg(unix)]
#[test]
fn native_elf_audit_should_accept_pinned_non_alloc_lld_provenance() {
    let output = run_mocked_elf_audit("Linker: LLD 22.1.8", "MS");
    assert!(
        output.status.success(),
        "pinned non-ALLOC LLD provenance is metadata, not a runtime dependency:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn native_elf_audit_should_reject_allocated_or_unpinned_provenance() {
    for (comment, flags, expected) in [
        (
            "Linker: LLD 22.1.8",
            "AMS",
            "ELF producer metadata must be non-ALLOC",
        ),
        (
            "Linker: LLD 22.1.7",
            "MS",
            "missing pinned ELF linker provenance",
        ),
        (
            "Linker: LLD 22.1.80",
            "MS",
            "missing pinned ELF linker provenance",
        ),
    ] {
        let output = run_mocked_elf_audit(comment, flags);
        assert!(
            !output.status.success(),
            "audit unexpectedly accepted {comment}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "audit did not report {expected:?}:\n{}",
            String::from_utf8_lossy(&output.stderr)
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
