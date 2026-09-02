use calckernel::{
    EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirMultiversionPlanningRequest,
    KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, NativeContext,
    NativeMultiversionObjectBundle, NativeMultiversionTargetSet, SourceFile, build_kir_module,
    check, check_kir_multiversion_bundle, create_native_multiversion_static_archive,
    emit_native_multiversion_objects, import_contract_facts,
    link_native_multiversion_dynamic_library, link_native_multiversion_executable, lower_to_mir,
    propose_kir_multiversion_bundle, run_kir_pass_pipeline,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

fn isolated_cache_root(root: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return root.join("home/Library/Caches/ckc");
    #[cfg(target_os = "linux")]
    return root.join("xdg/ckc");
    #[cfg(target_os = "windows")]
    return root.join("local-app-data/CalcKernel/cache");
}

fn isolated_command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ckc"));
    command.env("PATH", "");
    #[cfg(target_os = "macos")]
    command.env("HOME", root.join("home"));
    #[cfg(target_os = "linux")]
    command.env("XDG_CACHE_HOME", root.join("xdg"));
    #[cfg(target_os = "windows")]
    command.env("LOCALAPPDATA", root.join("local-app-data"));
    command
}

fn cache_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fs::read_dir(root)
        .expect("cache root")
        .map(|entry| entry.expect("cache entry").path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        })
        .map(|path| {
            let bytes = fs::read(&path).expect("cache bytes");
            (path, bytes)
        })
        .collect()
}

fn object_bundle(source: &str, consumer: KirConsumer) -> NativeMultiversionObjectBundle {
    let targets = NativeMultiversionTargetSet::host(consumer).expect("target set");
    let checked = check(&SourceFile::new("multiversion-artifact.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let mut kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    kir.profile = targets.target_set().tiers[0].profile.clone();
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let optimized = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let request = KirMultiversionPlanningRequest {
        logical_pre_state: optimized.artifact.expect("baseline"),
        target_set: targets.target_set().clone(),
        pgo_hot_roots: None,
        shared_growth_consumed: 0,
    };
    let bundle = propose_kir_multiversion_bundle(&request).expect("bundle");
    check_kir_multiversion_bundle(&request, &bundle).expect("checked bundle");
    emit_native_multiversion_objects(
        &NativeContext::new().expect("context"),
        &targets,
        &request,
        &bundle,
        None,
        &EmitLlvmOptions::default(),
    )
    .expect("object bundle")
}

#[test]
fn multiversion_artifact_real_linker_and_archive_matrix_should_accept_named_objects() {
    let library_bundle = object_bundle(
        "export fn answer() -> i32 { return 42; }",
        KirConsumer::NativeLibrary,
    );
    let dynamic =
        link_native_multiversion_dynamic_library(&library_bundle, &["answer".to_string()])
            .expect("dynamic library");
    assert!(!dynamic.as_bytes().is_empty());
    let archive =
        create_native_multiversion_static_archive(&library_bundle).expect("static archive");
    assert_eq!(archive.member_count(), library_bundle.objects().len());
    assert_eq!(
        archive.member_names(),
        library_bundle
            .objects()
            .iter()
            .map(|object| object.name())
            .collect::<Vec<_>>()
    );
    for name in archive.member_names() {
        assert!(
            archive
                .as_bytes()
                .windows(name.len())
                .any(|window| window == name.as_bytes()),
            "archive bytes omit member {name}"
        );
    }

    let executable_bundle = object_bundle(
        "fn main() -> i32 { print_i32(42); print_newline(); return 0; }",
        KirConsumer::NativeExecutable,
    );
    let executable = link_native_multiversion_executable(&executable_bundle).expect("executable");
    assert!(!executable.as_bytes().is_empty());
}

#[test]
fn multiversion_artifact_named_object_manifest_should_be_closed_and_deterministic() {
    let first = object_bundle(
        "export fn answer() -> i32 { return 42; }",
        KirConsumer::NativeLibrary,
    );
    let second = object_bundle(
        "export fn answer() -> i32 { return 42; }",
        KirConsumer::NativeLibrary,
    );
    assert_eq!(
        first.manifest_bytes().expect("manifest"),
        second.manifest_bytes().expect("manifest")
    );
    assert_eq!(
        first.bundle_digest().expect("digest"),
        second.bundle_digest().expect("digest")
    );
}

#[test]
fn multiversion_artifact_cli_should_commit_dynamic_output_and_reject_object_output() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "ckc-multiversion-artifact-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).expect("fixture root");
    let source = root.join("answer.ck");
    fs::write(&source, "export fn answer() -> i32 { return 42; }").expect("source");
    let output = root.join("answer");
    let built = isolated_command(&root)
        .args([
            "build",
            source.to_str().expect("source path"),
            "--kind",
            "dynamic",
            "--out",
            output.to_str().expect("output path"),
            "--cpu",
            "multiversion",
            "-O3",
        ])
        .output()
        .expect("build dynamic");
    assert!(built.status.success(), "{built:?}");
    let dynamic = if cfg!(target_os = "macos") {
        output.with_extension("dylib")
    } else if cfg!(target_os = "windows") {
        output.with_extension("dll")
    } else {
        output.with_extension("so")
    };
    assert!(dynamic.is_file());
    assert!(output.with_extension("h").is_file());

    let object = isolated_command(&root)
        .args([
            "build",
            source.to_str().expect("source path"),
            "--kind",
            "object",
            "--out",
            root.join("bad").to_str().expect("object path"),
            "--cpu",
            "multiversion",
            "-O3",
        ])
        .output()
        .expect("reject object");
    assert!(!object.status.success());
    assert!(String::from_utf8_lossy(&object.stderr).contains("does not support --kind object"));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn cache_multiversion_complete_bundle_should_hit_and_reject_each_corrupt_reference() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "ckc-multiversion-cache-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).expect("fixture root");
    let source = root.join("answer.ck");
    fs::write(&source, "export fn answer() -> i32 { return 42; }").expect("source");
    let output = root.join("answer");
    let run = || {
        isolated_command(&root)
            .args([
                "build",
                source.to_str().expect("source path"),
                "--kind",
                "dynamic",
                "--out",
                output.to_str().expect("output path"),
                "--cpu",
                "multiversion",
                "-O3",
            ])
            .output()
            .expect("build multiversion cache fixture")
    };
    let cold = run();
    assert!(cold.status.success(), "{cold:?}");
    let artifact_path = if cfg!(target_os = "macos") {
        output.with_extension("dylib")
    } else if cfg!(target_os = "windows") {
        output.with_extension("dll")
    } else {
        output.with_extension("so")
    };
    let cold_artifact = fs::read(&artifact_path).expect("cold artifact");
    let cache_root = isolated_cache_root(&root);
    let cold_entries = cache_files(&cache_root);
    assert!(cold_entries.len() >= 3, "{cold_entries:?}");
    let warm = run();
    assert!(warm.status.success(), "{warm:?}");
    assert_eq!(cache_files(&cache_root), cold_entries);
    assert_eq!(
        fs::read(&artifact_path).expect("warm artifact"),
        cold_artifact
    );

    for path in cold_entries.keys() {
        fs::write(path, b"corrupt reference").expect("corrupt cache member");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("restore private mode");
        }
        let repaired = run();
        assert!(repaired.status.success(), "{repaired:?}");
        assert_eq!(
            fs::read(&artifact_path).expect("repaired artifact"),
            cold_artifact
        );
        assert!(
            fs::read(path)
                .expect("repaired entry")
                .starts_with(b"CKCOBJ04")
        );
    }
    let static_output = root.join("answer-static");
    let static_build = isolated_command(&root)
        .args([
            "build",
            source.to_str().expect("source path"),
            "--kind",
            "static",
            "--out",
            static_output.to_str().expect("static output path"),
            "--cpu",
            "multiversion",
            "-O3",
        ])
        .output()
        .expect("build static cache fixture");
    assert!(static_build.status.success(), "{static_build:?}");
    assert!(
        cache_files(&cache_root).len() > cold_entries.len(),
        "physical artifact kind must split cache identity"
    );
    assert!(fs::read_dir(&cache_root).expect("cache root").all(|entry| {
        !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .contains(".tmp")
    }));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn cache_profile_generation_should_bypass_native_object_cache() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "ckc-generation-cache-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).expect("fixture root");
    let root = fs::canonicalize(root).expect("canonical fixture root");
    let profiles = root.join("profiles");
    fs::create_dir(&profiles).expect("profile directory");
    let source = root.join("program.ck");
    fs::write(&source, "fn main() -> i32 { return 0; }").expect("source");
    let built = isolated_command(&root)
        .args([
            "build",
            source.to_str().expect("source path"),
            "--kind",
            "executable",
            "--out",
            root.join("program").to_str().expect("output path"),
            "--pgo-generate",
            profiles.to_str().expect("profiles path"),
            "-O3",
        ])
        .output()
        .expect("generation build");
    assert!(built.status.success(), "{built:?}");
    assert!(!isolated_cache_root(&root).exists());
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn multiversion_artifact_stage09_acceptance_set_should_use_real_bundle_outputs() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/native-acceptance/v0.13-stage-09");
    super::artifacts::write_native_acceptance_artifact_set(&root);
    let library_bundle = object_bundle(
        "export fn answer() -> i32 { return 42; }",
        KirConsumer::NativeLibrary,
    );
    fs::write(
        root.join(if cfg!(target_os = "windows") {
            "module.obj"
        } else {
            "module.o"
        }),
        library_bundle.objects()[0].object().as_bytes(),
    )
    .expect("stage baseline object");
    let archive = create_native_multiversion_static_archive(&library_bundle).expect("archive");
    fs::write(
        root.join(if cfg!(target_os = "windows") {
            "module-static.lib"
        } else {
            "libmodule.a"
        }),
        archive.as_bytes(),
    )
    .expect("stage archive");
    let dynamic =
        link_native_multiversion_dynamic_library(&library_bundle, &["answer".to_string()])
            .expect("dynamic");
    fs::write(
        root.join(if cfg!(target_os = "windows") {
            "module.dll"
        } else if cfg!(target_os = "macos") {
            "libmodule.dylib"
        } else {
            "libmodule.so"
        }),
        dynamic.as_bytes(),
    )
    .expect("stage dynamic");
    if let Some(import) = dynamic.import_library() {
        fs::write(root.join("module-import.lib"), import).expect("stage import library");
    }
    let executable_bundle = object_bundle(
        "fn main() -> i32 { print_i32(42); print_newline(); return 0; }",
        KirConsumer::NativeExecutable,
    );
    let executable = link_native_multiversion_executable(&executable_bundle).expect("executable");
    let executable_path = root.join(if cfg!(target_os = "windows") {
        "program.exe"
    } else {
        "program"
    });
    fs::write(&executable_path, executable.as_bytes()).expect("stage executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable_path, fs::Permissions::from_mode(0o755))
            .expect("make executable");
    }
    fs::write(
        root.join("multiversion-object-manifest.bin"),
        library_bundle.manifest_bytes().expect("manifest"),
    )
    .expect("stage manifest evidence");
}
