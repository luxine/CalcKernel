use super::{capture, diagnostics, unique_id};
use std::{
    ffi::OsString,
    fs,
    os::unix::{fs::MetadataExt, fs::PermissionsExt, fs::symlink, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{ExitStatus, Output},
};

struct Fixture {
    root: PathBuf,
    files: [PathBuf; 4],
    cache: PathBuf,
    key: String,
    arguments: Vec<OsString>,
    result: Output,
}

impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/tune-capture-tests")
            .join(format!("capture-{}", unique_id()));
        fs::create_dir_all(&root).unwrap();
        let files =
            ["compiler", "source.ck", "workload.cktune.toml", "runner"].map(|name| root.join(name));
        for (index, path) in files.iter().enumerate() {
            fs::write(path, format!("exact original file {index}\n")).unwrap();
        }
        fs::set_permissions(&files[0], fs::Permissions::from_mode(0o700)).unwrap();
        let base = root.join("cache-base");
        let cache = calckernel::TuneCache::open_at(&base).unwrap();
        let key = cache.derive_key(
            calckernel::TuneCacheDomain::Compile,
            &[b"identity", b"plan"],
        );
        cache
            .write(
                calckernel::TuneCacheDomain::Compile,
                key,
                b"original package payload",
            )
            .unwrap();
        Self {
            cache: base.join(calckernel::TUNE_CACHE_NAMESPACE).join("compile"),
            key: key
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            arguments: vec![
                OsString::from("tune"),
                OsString::from("two words\nnext line"),
            ],
            result: Output {
                status: ExitStatus::from_raw(23 << 8),
                stdout: b"original stdout\0\xff".to_vec(),
                stderr: b"original unstable measurement\n".to_vec(),
            },
            root,
            files,
        }
    }

    fn request(&self) -> capture::Request<'_> {
        capture::Request {
            root: &self.root,
            compiler: &self.files[0],
            source: &self.files[1],
            manifest: &self.files[2],
            runner: &self.files[3],
            compile_cache: &self.cache,
            arguments: &self.arguments,
            original: &self.result,
            identity_context: Ok(b"test-only fixed identity context\n"),
        }
    }

    fn evidence(&self) -> PathBuf {
        self.root.join("post-failure-startup/failed-original")
    }

    fn manifest(&self) -> String {
        fs::read_to_string(self.evidence().join("manifest.txt")).unwrap()
    }
}

fn metadata(path: &Path) -> (u64, u64, u64, i64, i64, u32) {
    let value = fs::symlink_metadata(path).unwrap();
    (
        value.dev(),
        value.ino(),
        value.len(),
        value.mtime(),
        value.mtime_nsec(),
        value.mode(),
    )
}

#[test]
fn failed_session_capture_retains_exact_original_bytes_and_metadata() {
    let fixture = Fixture::new();
    let package = fixture.cache.join(&fixture.key);
    let before = metadata(&package);
    capture::capture(fixture.request(), capture::Limits::DEFAULT).unwrap();
    assert_eq!(
        fs::read(fixture.evidence().join("compile").join(&fixture.key)).unwrap(),
        fs::read(&package).unwrap()
    );
    assert_eq!(metadata(&package), before);
    assert_eq!(
        fs::read(fixture.evidence().join("compiler.bin")).unwrap(),
        fs::read(&fixture.files[0]).unwrap()
    );
    assert_eq!(
        fs::read(fixture.evidence().join("original.stdout")).unwrap(),
        fixture.result.stdout
    );
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
    assert!(fixture.manifest().contains("captureComplete=true\n"));
    assert!(fixture.manifest().contains("envelope-valid"));
}

#[test]
fn captured_images_are_owner_only_non_executable_and_arguments_are_lossless() {
    let fixture = Fixture::new();
    capture::capture(fixture.request(), capture::Limits::DEFAULT).unwrap();
    let compiler = fs::metadata(fixture.evidence().join("compiler.bin")).unwrap();
    assert_eq!(compiler.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(fixture.evidence())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::read_to_string(fixture.evidence().join("arguments.hex")).unwrap(),
        "0\t74756e65\n1\t74776f20776f7264730a6e657874206c696e65\n"
    );
}

#[test]
fn successful_session_capture_performs_no_filesystem_work() {
    let mut fixture = Fixture::new();
    fixture.result.status = ExitStatus::from_raw(0);
    capture::capture(fixture.request(), capture::Limits::DEFAULT).unwrap();
    assert!(!fixture.root.join("post-failure-startup").exists());
}

#[test]
fn capture_never_overwrites_prior_evidence() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.evidence()).unwrap();
    let marker = fixture.evidence().join("compiler.bin");
    fs::write(&marker, b"keep previous evidence").unwrap();
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert_eq!(fs::read(marker).unwrap(), b"keep previous evidence");
}

#[test]
fn capture_rejects_a_symlinked_output_root() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.root.join("post-failure-startup")).unwrap();
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn capture_rejects_source_symlinks_and_keeps_original_failure() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("source-alias.ck");
    symlink(&fixture.files[1], &alias).unwrap();
    let mut request = fixture.request();
    request.source = &alias;
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().join("source.ck").exists());
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
    assert!(fixture.manifest().contains("captureComplete=false\n"));
}

#[test]
fn capture_rejects_a_symlinked_source_ancestor() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("alias");
    symlink(&fixture.root, &alias).unwrap();
    let source = alias.join("source.ck");
    let mut request = fixture.request();
    request.source = &source;
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().join("source.ck").exists());
}

#[test]
fn capture_rejects_symlinked_cache_without_following_it() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("cache-alias");
    symlink(&fixture.cache, &alias).unwrap();
    let mut request = fixture.request();
    request.compile_cache = &alias;
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert!(
        !fixture
            .evidence()
            .join("compile")
            .join(&fixture.key)
            .exists()
    );
}

#[test]
fn capture_preserves_malformed_regular_packages_without_cache_repair() {
    let fixture = Fixture::new();
    let path = fixture.cache.join(&fixture.key);
    fs::write(&path, b"malformed original cache\0").unwrap();
    let before = metadata(&path);
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert_eq!(
        fs::read(fixture.evidence().join("compile").join(&fixture.key)).unwrap(),
        b"malformed original cache\0"
    );
    assert_eq!(metadata(&path), before);
    assert!(fixture.manifest().contains("envelope-invalid"));
}

#[test]
fn capture_reports_file_bound_without_erasing_original_streams() {
    let fixture = Fixture::new();
    let limits = capture::Limits {
        compiler_bytes: 1,
        ..capture::Limits::DEFAULT
    };
    assert!(capture::capture(fixture.request(), limits).is_err());
    assert!(!fixture.evidence().join("compiler.bin").exists());
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
    assert!(fixture.manifest().contains("captureComplete=false\n"));
}

#[test]
fn capture_reports_total_bound_and_preserves_partial_manifest() {
    let fixture = Fixture::new();
    let limits = capture::Limits {
        total_bytes: 1,
        ..capture::Limits::DEFAULT
    };
    assert!(capture::capture(fixture.request(), limits).is_err());
    assert!(fixture.manifest().contains("captureComplete=false\n"));
}

#[test]
fn capture_reports_excessive_cache_count_without_changing_entries() {
    let fixture = Fixture::new();
    let limits = capture::Limits {
        cache_files: 0,
        ..capture::Limits::DEFAULT
    };
    assert!(capture::capture(fixture.request(), limits).is_err());
    assert!(fixture.cache.join(&fixture.key).is_file());
    assert!(fixture.manifest().contains("captureComplete=false\n"));
}

#[test]
fn capture_errors_cannot_replace_the_original_process_result() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.evidence()).unwrap();
    let original_status = fixture.result.status;
    let original_stderr = fixture.result.stderr.clone();
    diagnostics::after_failure(fixture.result.status.success(), || {
        capture::capture(fixture.request(), capture::Limits::DEFAULT)
            .map_err(|error| error.to_string())
    });
    assert_eq!(fixture.result.status, original_status);
    assert_eq!(fixture.result.stderr, original_stderr);
}

#[test]
fn capture_rejects_fifo_sources_without_waiting_for_a_writer() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let fixture = Fixture::new();
    let pipe = fixture.root.join("source-pipe");
    let name = CString::new(pipe.as_os_str().as_bytes()).unwrap();
    // SAFETY: name is a live NUL-terminated path to a new test-owned FIFO.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let mut request = fixture.request();
    request.source = &pipe;
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().join("source.ck").exists());
}

#[test]
fn capture_rejects_directory_sources() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.source = &fixture.root;
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().join("source.ck").exists());
}

#[test]
fn capture_rejects_package_symlinks_without_reading_the_target() {
    let fixture = Fixture::new();
    let entry = fixture.cache.join(&fixture.key);
    let original = fixture.root.join("original-entry");
    fs::rename(&entry, &original).unwrap();
    symlink(&original, &entry).unwrap();
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert!(
        !fixture
            .evidence()
            .join("compile")
            .join(&fixture.key)
            .exists()
    );
}

#[test]
fn capture_records_invalid_cache_names_without_copying_them() {
    let fixture = Fixture::new();
    fs::write(fixture.cache.join("unrelated\nfile"), b"not a package").unwrap();
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().join("compile/unrelated\nfile").exists());
    assert!(fixture.manifest().contains("invalid-cache-name"));
    assert!(
        fixture
            .evidence()
            .join("compile")
            .join(&fixture.key)
            .is_file()
    );
}

#[test]
fn capture_rejects_group_writable_output_directory() {
    let fixture = Fixture::new();
    let root = fixture.root.join("post-failure-startup");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o770)).unwrap();
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().exists());
}

#[test]
fn capture_rejects_group_writable_sources() {
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.files[1], fs::Permissions::from_mode(0o660)).unwrap();
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert!(!fixture.evidence().join("source.ck").exists());
}

fn invalid_envelope(mutate: impl FnOnce(&mut Vec<u8>), rehash: bool) {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    let entry = fixture.cache.join(&fixture.key);
    let mut bytes = fs::read(&entry).unwrap();
    mutate(&mut bytes);
    if rehash {
        let end = bytes.len() - 32;
        let mut hash = Sha256::new();
        hash.update(b"CK-TUNE-CACHE-ENTRY\0");
        hash.update(&bytes[..end]);
        bytes[end..].copy_from_slice(&hash.finalize());
    }
    fs::write(&entry, &bytes).unwrap();
    let before = metadata(&entry);
    assert!(capture::capture(fixture.request(), capture::Limits::DEFAULT).is_err());
    assert_eq!(
        fs::read(fixture.evidence().join("compile").join(&fixture.key)).unwrap(),
        bytes
    );
    assert_eq!(metadata(&entry), before);
    assert!(fixture.manifest().contains("envelope-invalid"));
}

#[test]
fn capture_retains_checksum_mismatch_as_invalid_evidence() {
    invalid_envelope(|bytes| bytes[53] ^= 1, false);
}

#[test]
fn capture_retains_wrong_physical_key_as_invalid_evidence() {
    invalid_envelope(|bytes| bytes[13] ^= 1, true);
}

#[test]
fn capture_retains_wrong_domain_as_invalid_evidence() {
    invalid_envelope(|bytes| bytes[12] = 2, true);
}

#[test]
fn capture_retains_wrong_schema_as_invalid_evidence() {
    invalid_envelope(|bytes| bytes[11] = 2, true);
}

#[test]
fn capture_retains_wrong_declared_length_as_invalid_evidence() {
    invalid_envelope(|bytes| bytes[52] ^= 1, true);
}

#[test]
fn missing_cache_still_retains_original_failure_and_reports_incomplete() {
    let fixture = Fixture::new();
    let missing = fixture.root.join("missing-cache");
    let mut request = fixture.request();
    request.compile_cache = &missing;
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
    assert!(fixture.manifest().contains("captureComplete=false\n"));
}

#[test]
fn identity_observation_error_does_not_discard_failure_bytes() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.identity_context = Err("native target observation unavailable");
    assert!(capture::capture(request, capture::Limits::DEFAULT).is_err());
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
    assert!(fixture.manifest().contains("captureComplete=false\n"));
    assert!(fixture.manifest().contains("error\tidentity-context.txt\t"));
}

#[test]
fn native_identity_context_retains_full_allowlisted_target_material() {
    use calckernel::{KirConsumer, NativeCpu, NativeTarget};
    let context = String::from_utf8(capture::native_identity_context().unwrap()).unwrap();
    let target = NativeTarget::host_with_cpu(NativeCpu::Native).unwrap();
    let raw_features = target.features().unwrap();
    let mut features = raw_features
        .split(',')
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    features.sort();
    features.dedup();
    let hex = |value: &str| {
        value
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    for (name, value) in [
        ("versionHex", hex(env!("CARGO_PKG_VERSION"))),
        (
            "llvmManifestSha256",
            env!("CKC_LLVM_MANIFEST_SHA256").to_string(),
        ),
        (
            "llvmBridgeAbi",
            calckernel::LLVM_BRIDGE_ABI_VERSION.to_string(),
        ),
        ("tripleHex", hex(&target.triple().unwrap())),
        ("cpuHex", hex(&target.cpu().unwrap())),
        ("featuresHex", hex(&features.join(","))),
        (
            "targetProfileDigest",
            target
                .kir_profile(KirConsumer::NativeExecutable)
                .unwrap()
                .digest_hex(),
        ),
    ] {
        assert!(
            context.contains(&format!("{name}={value}\n")),
            "missing {name}: {context}"
        );
    }
    assert!(context.starts_with("CKTUNE-FAILURE-IDENTITY/1\n"));
    assert!(context.contains("observation=after-original-process-exit\n"));
    assert!(context.contains("failedPlanIdentityVerified=false\n"));
    assert_eq!(context.lines().count(), 10);
}

#[test]
fn capture_rejects_package_over_file_bound_before_creating_it() {
    let fixture = Fixture::new();
    let length = fs::metadata(fixture.cache.join(&fixture.key))
        .unwrap()
        .len();
    let limits = capture::Limits {
        file_bytes: length - 1,
        ..capture::Limits::DEFAULT
    };
    assert!(capture::capture(fixture.request(), limits).is_err());
    assert!(
        !fixture
            .evidence()
            .join("compile")
            .join(&fixture.key)
            .exists()
    );
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
}

#[test]
fn capture_success_short_circuits_missing_paths_and_identity_errors() {
    let mut fixture = Fixture::new();
    fixture.result.status = ExitStatus::from_raw(0);
    let missing = fixture.root.join("nonexistent");
    let mut request = fixture.request();
    request.root = &missing;
    request.identity_context = Err("must not inspect");
    capture::capture(request, capture::Limits::DEFAULT).unwrap();
    assert!(!missing.exists());
}

#[test]
fn independent_capture_runs_even_when_prior_comparator_panics() {
    let fixture = Fixture::new();
    diagnostics::after_failure(fixture.result.status.success(), || {
        panic!("comparator failed")
    });
    diagnostics::after_failure(fixture.result.status.success(), || {
        capture::capture(fixture.request(), capture::Limits::DEFAULT)
            .map_err(|error| error.to_string())
    });
    assert_eq!(
        fs::read(fixture.evidence().join("original.stderr")).unwrap(),
        fixture.result.stderr
    );
    assert_eq!(fixture.result.status.into_raw(), 23 << 8);
}
