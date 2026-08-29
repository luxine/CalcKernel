use std::{collections::BTreeSet, env, fs, path::PathBuf};

use sha2::{Digest, Sha256};

const LLVM_VERSION: &str = "22.1.8";
const LLVM_SOURCE_SHA256: &str = "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888";
const LLVM_COMPONENTS: [&str; 5] = ["core", "native", "orcjit", "nativecodegen", "lto"];
const CARGO_PROVENANCE_COMPONENTS: [&str; 26] = [
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
];

fn main() {
    println!("cargo::rerun-if-env-changed=CKC_LLVM_PREFIX");
    println!("cargo::rerun-if-changed=native/llvm/manifest.toml");
    println!("cargo::rerun-if-changed=native/bridge/ckc_llvm.cpp");
    println!("cargo::rerun-if-changed=native/bridge/ckc_llvm.h");
    println!("cargo::rerun-if-changed=native/runtime");
    println!("cargo::rerun-if-changed=third_party");
    println!("cargo::rerun-if-changed=THIRD_PARTY_NOTICES.md");
    println!("cargo::rerun-if-changed=Cargo.lock");

    validate_third_party_provenance();

    let target = env::var("TARGET").expect("Cargo always defines TARGET");
    println!("cargo::rustc-env=CKC_BUILD_TARGET={target}");

    if env::var_os("CARGO_FEATURE_NATIVE_TOOLCHAIN").is_none() {
        return;
    }

    configure_native_toolchain(&target);
}

fn validate_third_party_provenance() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let notices = read_required(
        &root.join("THIRD_PARTY_NOTICES.md"),
        "third-party notice index",
    );
    let lock = read_required(&root.join("Cargo.lock"), "Cargo lockfile");
    let cargo = read_required(
        &root.join("third_party/cargo/provenance.toml"),
        "Cargo provenance",
    );

    require_manifest_value(&cargo, "version", "1.90.0");
    require_manifest_value(
        &cargo,
        "source_sha256",
        "cde088d57064d151b2236f4619aea4a8207e0709eb3035ddc6617d609ab7d453",
    );
    validate_file_hashes(&root, &cargo, "license_files", "license_sha256");

    let mut actual = BTreeSet::new();
    for block in cargo.split("[[cargo]]").skip(1) {
        let name = required_scalar(block, "name", "Cargo provenance block");
        let version = required_scalar(block, "version", name);
        let checksum = required_scalar(block, "crate_sha256", name);
        assert!(
            actual.insert(name.to_string()),
            "duplicate Cargo provenance for {name}"
        );

        let package_identity = format!("name = \"{name}\"\nversion = \"{version}\"");
        assert!(
            lock.contains(&package_identity),
            "Cargo provenance is not present in Cargo.lock: {name} {version}"
        );
        assert!(
            package_block(&lock, name, version)
                .is_some_and(|package| package.contains(&format!("checksum = \"{checksum}\""))),
            "Cargo provenance checksum drift: {name} {version}"
        );
        validate_file_hashes(&root, block, "license_files", "license_sha256");
        for required in [name, version, checksum] {
            assert!(
                notices.contains(required),
                "third-party notice index is stale for {name}: missing {required}"
            );
        }
    }
    let expected: BTreeSet<String> = CARGO_PROVENANCE_COMPONENTS
        .into_iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        actual, expected,
        "Cargo source/build dependency provenance is missing or unreferenced"
    );

    let llvm = read_required(&root.join("native/llvm/manifest.toml"), "LLVM manifest");
    validate_file_hashes(
        &root.join("native/llvm"),
        &llvm,
        "license_files",
        "license_sha256",
    );
    let runtime = read_required(
        &root.join("native/runtime/provenance.toml"),
        "native runtime provenance",
    );
    validate_file_hashes(
        &root.join("native/runtime"),
        &runtime,
        "runtime_files",
        "runtime_source_sha256",
    );
    validate_file_hashes(
        &root.join("native/runtime"),
        &runtime,
        "files",
        "source_sha256",
    );
    validate_file_hashes(
        &root.join("native/runtime"),
        &runtime,
        "license_files",
        "license_sha256",
    );
    for component in ["LLVM 22.1.8", "LLD 22.1.8", "BLAKE3", "regex", "Ryu"] {
        assert!(
            notices.contains(component),
            "third-party notice index is missing {component}"
        );
    }
}

fn read_required(path: &std::path::Path, description: &str) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {description} {}: {error}", path.display()))
}

fn required_scalar<'text>(text: &'text str, key: &str, description: &str) -> &'text str {
    manifest_scalar(text, key).unwrap_or_else(|| panic!("{description} is missing {key}"))
}

fn validate_file_hashes(root: &std::path::Path, manifest: &str, path_key: &str, hash_key: &str) {
    let paths = manifest_array(manifest, path_key);
    let hashes = manifest_array(manifest, hash_key);
    assert_eq!(
        paths.len(),
        hashes.len(),
        "provenance {path_key}/{hash_key} length mismatch"
    );
    assert!(!paths.is_empty(), "provenance {path_key} must not be empty");
    for (relative, expected) in paths.iter().zip(&hashes) {
        assert!(
            !relative.starts_with('/') && !relative.contains(".."),
            "provenance path must remain repository-relative: {relative}"
        );
        let path = root.join(relative);
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("failed to read provenance file {}: {error}", path.display())
        });
        let actual = format!("{:x}", Sha256::digest(bytes));
        assert!(
            actual == expected.as_str(),
            "provenance hash mismatch for {}: actual={actual:?} ({} bytes), expected={expected:?} ({} bytes)",
            path.display(),
            actual.len(),
            expected.len()
        );
    }
}

fn package_block<'text>(lock: &'text str, name: &str, version: &str) -> Option<&'text str> {
    lock.split("[[package]]").find(|block| {
        block.contains(&format!("name = \"{name}\""))
            && block.contains(&format!("version = \"{version}\""))
    })
}

fn configure_native_toolchain(target: &str) {
    let prefix = env::var_os("CKC_LLVM_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("native-toolchain requires CKC_LLVM_PREFIX"));
    let component_manifest = prefix.join("share/ckc/llvm-build.toml");
    println!("cargo::rerun-if-changed={}", component_manifest.display());
    let text = fs::read_to_string(&component_manifest).unwrap_or_else(|error| {
        panic!(
            "failed to read native LLVM component manifest {}: {error}",
            component_manifest.display()
        )
    });

    require_manifest_value(&text, "version", LLVM_VERSION);
    require_manifest_value(&text, "profile", "release");
    require_manifest_value(&text, "source_sha256", LLVM_SOURCE_SHA256);
    require_manifest_value(&text, "static_only", "true");
    require_manifest_value(&text, "target", target);
    assert_eq!(
        manifest_array(&text, "components"),
        LLVM_COMPONENTS,
        "native LLVM component manifest component allowlist mismatch"
    );

    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    println!("cargo::warning=ckc LLVM manifest sha256={digest}");
    println!("cargo::rustc-env=CKC_LLVM_MANIFEST_SHA256={digest}");

    let include_dir = prefix.join("include");
    let lib_dir = prefix.join("lib");
    assert_directory(&include_dir, "LLVM include");
    assert_directory(&lib_dir, "LLVM library");

    let mut bridge = cc::Build::new();
    bridge
        .cpp(true)
        .std("c++20")
        .cpp_link_stdlib(None)
        .static_crt(true)
        .include(&include_dir)
        .include("native/bridge")
        .file("native/bridge/ckc_llvm.cpp")
        .warnings(false);
    if target.contains("apple-darwin") {
        bridge.define("CKC_LLD_DARWIN", None);
    } else if target.contains("windows-msvc") {
        bridge.define("CKC_LLD_COFF", None);
    } else {
        bridge.define("CKC_LLD_ELF", None);
    }
    if target.ends_with("-msvc") {
        bridge.flag_if_supported("/GR-");
        bridge.flag_if_supported("/EHsc");
    } else {
        bridge.flag_if_supported("-fno-rtti");
    }
    bridge.compile("ckc_llvm_bridge");

    println!("cargo::rustc-link-search=native={}", lib_dir.display());
    for library in manifest_array(&text, "static_libraries") {
        let archive = static_library_path(&lib_dir, &library, target);
        assert!(
            archive.is_file(),
            "native LLVM component manifest names missing static library: {}",
            archive.display()
        );
        println!("cargo::rustc-link-lib=static={library}");
    }
    for library in manifest_array(&text, "system_libraries") {
        println!("cargo::rustc-link-lib={library}");
    }
    if target.contains("apple-darwin") {
        println!("cargo::rustc-link-lib=c++");
    } else if target.contains("unknown-linux-gnu") {
        println!("cargo::rustc-link-lib=static=stdc++");
    }

    let runtime_objects = manifest_array(&text, "runtime_objects");
    let runtime_hashes = manifest_array(&text, "runtime_sha256");
    assert_eq!(
        runtime_objects.len(),
        5,
        "native runtime manifest must contain exactly five objects"
    );
    assert_eq!(
        runtime_hashes.len(),
        runtime_objects.len(),
        "native runtime object/hash list length mismatch"
    );
    let expected_suffix = if target.ends_with("-msvc") {
        ".obj"
    } else {
        ".o"
    };
    let runtime_dir = prefix.join("share/ckc/runtime");
    for (index, (name, expected_hash)) in runtime_objects
        .iter()
        .zip(runtime_hashes.iter())
        .enumerate()
    {
        assert!(
            name.ends_with(expected_suffix) && !name.contains('/') && !name.contains('\\'),
            "invalid native runtime object name: {name}"
        );
        let path = runtime_dir.join(name);
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "failed to read native runtime object {}: {error}",
                path.display()
            )
        });
        let actual_hash = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(
            actual_hash,
            *expected_hash,
            "native runtime object hash mismatch for {}",
            path.display()
        );
        println!(
            "cargo::rustc-env=CKC_RUNTIME_OBJECT_{index}={}",
            path.display()
        );
        println!("cargo::rustc-env=CKC_RUNTIME_SHA256_{index}={actual_hash}");
    }
    if target.ends_with("-msvc") {
        let name = manifest_scalar(&text, "runtime_platform_import")
            .expect("native Windows runtime manifest is missing runtime_platform_import");
        assert!(
            name.ends_with(".lib") && !name.contains('/') && !name.contains('\\'),
            "invalid native runtime platform import name: {name}"
        );
        let expected_hash = manifest_scalar(&text, "runtime_platform_import_sha256")
            .expect("native Windows runtime manifest is missing runtime_platform_import_sha256");
        let path = runtime_dir.join(name);
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "failed to read native runtime platform import {}: {error}",
                path.display()
            )
        });
        let actual_hash = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(
            actual_hash,
            expected_hash,
            "native runtime platform import hash mismatch for {}",
            path.display()
        );
        println!(
            "cargo::rustc-env=CKC_RUNTIME_PLATFORM_IMPORT={}",
            path.display()
        );
        println!("cargo::rustc-env=CKC_RUNTIME_PLATFORM_IMPORT_SHA256={actual_hash}");
    }
}

fn static_library_path(lib_dir: &std::path::Path, library: &str, target: &str) -> PathBuf {
    if target.ends_with("-msvc") {
        lib_dir.join(format!("{library}.lib"))
    } else {
        lib_dir.join(format!("lib{library}.a"))
    }
}

fn require_manifest_value(text: &str, key: &str, expected: &str) {
    let actual = manifest_scalar(text, key)
        .unwrap_or_else(|| panic!("native LLVM component manifest is missing {key}"));
    assert_eq!(
        actual, expected,
        "native LLVM component manifest {key} mismatch"
    );
}

fn manifest_scalar<'text>(text: &'text str, key: &str) -> Option<&'text str> {
    text.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        if candidate.trim() != key {
            return None;
        }
        Some(value.trim().trim_matches('"'))
    })
}

fn manifest_array(text: &str, key: &str) -> Vec<String> {
    let Some(value) = manifest_scalar(text, key) else {
        panic!("native LLVM component manifest is missing {key}");
    };
    let value = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or_else(|| panic!("native LLVM component manifest {key} must be an array"));
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_matches('"').to_string())
        .collect()
}

fn assert_directory(path: &std::path::Path, description: &str) {
    assert!(
        path.is_dir(),
        "{description} directory missing: {}",
        path.display()
    );
}
