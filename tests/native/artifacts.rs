use std::path::{Path, PathBuf};

use calckernel::{
    EmitLlvmOptions, NativeArtifactKind, NativeArtifactPaths, NativeContext, NativeObject,
    NativeOptimizationLevel, NativePlatform, NativeTarget, SourceFile, check,
    create_native_static_archive, lower_native_llvm_module, lower_to_mir,
};

fn native_object(source: &str) -> NativeObject {
    let checked = check(&SourceFile::new("artifact.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("lower artifact MIR");
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let module = lower_native_llvm_module(&context, &target, &mir, &EmitLlvmOptions::default())
        .expect("lower native object")
        .verify()
        .expect("verify native object")
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
