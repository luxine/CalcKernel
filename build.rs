use std::{env, fs, path::PathBuf};

use sha2::{Digest, Sha256};

const LLVM_VERSION: &str = "22.1.8";
const LLVM_SOURCE_SHA256: &str = "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888";
const LLVM_COMPONENTS: [&str; 5] = ["core", "native", "orcjit", "nativecodegen", "lto"];

fn main() {
    println!("cargo::rerun-if-env-changed=CKC_LLVM_PREFIX");
    println!("cargo::rerun-if-changed=native/llvm/manifest.toml");
    println!("cargo::rerun-if-changed=native/bridge/ckc_llvm.cpp");
    println!("cargo::rerun-if-changed=native/bridge/ckc_llvm.h");
    println!("cargo::rerun-if-changed=native/runtime");

    let target = env::var("TARGET").expect("Cargo always defines TARGET");
    println!("cargo::rustc-env=CKC_BUILD_TARGET={target}");

    if env::var_os("CARGO_FEATURE_NATIVE_TOOLCHAIN").is_none() {
        return;
    }

    configure_native_toolchain(&target);
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
