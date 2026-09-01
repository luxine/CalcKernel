use std::{
    fs,
    path::{Path, PathBuf},
};

use calckernel::{
    BoundsMode, EmitLlvmOptions, KirConsumer, NativeArtifactKind, NativeArtifactPaths,
    NativeContext, NativeObject, NativeOptimizationLevel, NativePlatform, NativeTarget,
    OverflowMode, create_native_static_archive, link_native_dynamic_library,
    lower_native_kir_module,
};

use super::runtime_support::executable_bytes;
use super::support::compiler::optimized_module;

fn native_object(source: &str) -> NativeObject {
    let kir = optimized_module(
        source,
        3,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let module = lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("lower native object")
        .verify()
        .expect("verify native object")
        .audit()
        .expect("audit native object facts")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("optimize native object");
    target.emit_object(module).expect("emit native object")
}

#[test]
fn artifact_paths_should_use_exact_platform_suffixes_and_header_pairs() {
    let cases = [
        (
            NativePlatform::Linux,
            ("program", "libvalue.so", "libvalue.a", "value.o"),
        ),
        (
            NativePlatform::Darwin,
            ("program", "libvalue.dylib", "libvalue.a", "value.o"),
        ),
        (
            NativePlatform::Windows,
            ("program.exe", "value.dll", "value.lib", "value.obj"),
        ),
    ];
    for (platform, (executable, dynamic, static_library, object)) in cases {
        for (kind, base, expected) in [
            (NativeArtifactKind::Executable, "program", executable),
            (NativeArtifactKind::Dynamic, "libvalue", dynamic),
            (NativeArtifactKind::Static, "libvalue", static_library),
            (NativeArtifactKind::Object, "value", object),
        ] {
            let paths = NativeArtifactPaths::new(platform, kind, Path::new(base));
            assert_eq!(
                paths.primary,
                PathBuf::from(expected),
                "{platform:?} {kind:?}"
            );
            assert_eq!(
                paths.header,
                (!matches!(kind, NativeArtifactKind::Executable))
                    .then(|| PathBuf::from(base).with_extension("h"))
            );
            assert_eq!(
                paths.import_library,
                (platform == NativePlatform::Windows && kind == NativeArtifactKind::Dynamic)
                    .then(|| PathBuf::from(base).with_extension("lib"))
            );
        }
    }
}

#[test]
fn explicit_matching_suffix_should_not_be_duplicated() {
    let paths = NativeArtifactPaths::new(
        NativePlatform::Darwin,
        NativeArtifactKind::Dynamic,
        Path::new("calc.dylib"),
    );
    assert_eq!(paths.primary, PathBuf::from("calc.dylib"));
    assert_eq!(paths.header, Some(PathBuf::from("calc.h")));
}

#[test]
fn static_archive_should_be_deterministic_valid_and_index_export_symbols() {
    let object = native_object("export fn answer() -> i32 { return 42; }");
    let first = create_native_static_archive(&object).expect("first archive");
    let second = create_native_static_archive(&object).expect("second archive");
    assert_eq!(first.as_bytes(), second.as_bytes());
    assert!(first.as_bytes().starts_with(b"!<arch>\n"));
    assert_eq!(first.member_count(), 1);
    assert!(first.has_symbol_index());
    assert!(
        first
            .as_bytes()
            .windows("answer".len())
            .any(|window| window == b"answer")
    );
}

#[test]
fn archive_api_should_accept_only_verified_native_objects() {
    let signature: fn(&NativeObject) -> Result<calckernel::NativeArchive, calckernel::NativeError> =
        create_native_static_archive;
    let _ = signature;
}

#[test]
fn native_acceptance_artifact_set_should_cover_every_audited_kind() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/native-acceptance");
    write_native_acceptance_artifact_set(&root);
}

#[test]
fn native_release_candidate_stage10_set_should_include_private_runtime_closure() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/native-acceptance/v0.13-stage-10");
    write_native_acceptance_artifact_set(&root);
    let suffix = if NativePlatform::host() == NativePlatform::Windows {
        "obj"
    } else {
        "o"
    };
    assert!(
        root.join("runtime")
            .join(format!("profile_runtime.{suffix}"))
            .is_file()
    );
    assert!(
        root.join("runtime")
            .join(format!("dispatch_runtime.{suffix}"))
            .is_file()
    );
}

pub(super) fn write_native_acceptance_artifact_set(root: &Path) {
    let runtime_dir = root.join("runtime");
    fs::create_dir_all(&runtime_dir).expect("create native acceptance directory");

    let object = native_object("export fn answer() -> i32 { return 42; }");
    let platform = NativePlatform::host();
    let object_name = if platform == NativePlatform::Windows {
        "module.obj"
    } else {
        "module.o"
    };
    fs::write(root.join(object_name), object.as_bytes()).expect("write audited object");

    let archive = create_native_static_archive(&object).expect("create audited archive");
    let archive_name = if platform == NativePlatform::Windows {
        "module-static.lib"
    } else {
        "libmodule.a"
    };
    fs::write(root.join(archive_name), archive.as_bytes()).expect("write audited archive");

    let dynamic = link_native_dynamic_library(&object, &["answer".to_string()])
        .expect("create audited dynamic library");
    let dynamic_name = match platform {
        NativePlatform::Linux => "libmodule.so",
        NativePlatform::Darwin => "libmodule.dylib",
        NativePlatform::Windows => "module.dll",
    };
    fs::write(root.join(dynamic_name), dynamic.as_bytes()).expect("write audited dynamic library");
    if let Some(import_library) = dynamic.import_library() {
        fs::write(root.join("module-import.lib"), import_library)
            .expect("write audited import library");
    }

    let executable = executable_bytes(
        "fn main() -> i32 { print_i32(42); print_newline(); return 0; }",
        calckernel::OverflowMode::Checked,
        calckernel::BoundsMode::Checked,
    );
    let executable_name = if platform == NativePlatform::Windows {
        "program.exe"
    } else {
        "program"
    };
    let executable_path = root.join(executable_name);
    fs::write(&executable_path, executable).expect("write audited executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable_path, fs::Permissions::from_mode(0o755))
            .expect("make audited executable runnable");
    }

    let runtime_objects = [
        (
            "runtime",
            env!("CKC_RUNTIME_OBJECT_0"),
            env!("CKC_RUNTIME_SHA256_0"),
        ),
        (
            "format_int",
            env!("CKC_RUNTIME_OBJECT_1"),
            env!("CKC_RUNTIME_SHA256_1"),
        ),
        (
            "format_float",
            env!("CKC_RUNTIME_OBJECT_2"),
            env!("CKC_RUNTIME_SHA256_2"),
        ),
        (
            "ryu",
            env!("CKC_RUNTIME_OBJECT_3"),
            env!("CKC_RUNTIME_SHA256_3"),
        ),
        (
            "platform",
            env!("CKC_RUNTIME_OBJECT_4"),
            env!("CKC_RUNTIME_SHA256_4"),
        ),
        (
            "dispatch_runtime",
            env!("CKC_DISPATCH_RUNTIME_OBJECT"),
            env!("CKC_DISPATCH_RUNTIME_SHA256"),
        ),
        (
            "profile_runtime",
            env!("CKC_PROFILE_RUNTIME_OBJECT"),
            env!("CKC_PROFILE_RUNTIME_SHA256"),
        ),
    ];
    let suffix = if platform == NativePlatform::Windows {
        "obj"
    } else {
        "o"
    };
    let mut hashes = String::new();
    for (stem, source, hash) in runtime_objects {
        let name = format!("{stem}.{suffix}");
        fs::copy(source, runtime_dir.join(&name)).expect("copy audited runtime object");
        hashes.push_str(&format!("{hash}  {name}\n"));
    }
    #[cfg(target_os = "windows")]
    {
        fs::copy(
            env!("CKC_RUNTIME_PLATFORM_IMPORT"),
            runtime_dir.join("kernel32.lib"),
        )
        .expect("copy audited runtime import library");
        hashes.push_str(&format!(
            "{}  kernel32.lib\n",
            env!("CKC_RUNTIME_PLATFORM_IMPORT_SHA256")
        ));
    }
    fs::write(runtime_dir.join("SHA256SUMS"), hashes).expect("write runtime hash evidence");
}
