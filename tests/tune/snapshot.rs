use std::fs;
use std::path::PathBuf;

use calckernel::{TuneManifest, capture_workload, stage_invocation_inputs};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn snapshot_capture_is_immutable_after_source_replacement() {
    let root = fresh_root("immutable");
    let runner = root.join("runner");
    let input = root.join("input.bin");
    let manifest_path = root.join("workload.cktune.toml");
    fs::write(&runner, native_fixture_bytes()).expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).expect("executable");
    fs::write(&input, b"before").expect("input");
    let manifest_bytes = manifest_bytes();
    fs::write(&manifest_path, &manifest_bytes).expect("manifest");
    let manifest = TuneManifest::parse(&manifest_bytes, &manifest_path).expect("manifest");
    let captured = capture_workload(&manifest).expect("capture");
    assert_eq!(captured.runner_bytes(), native_fixture_bytes());
    assert_ne!(captured.manifest_digest(), [0; 32]);
    assert!(captured.environment_identities().is_empty());

    fs::write(&input, b"after").expect("replace input");
    let staged = stage_invocation_inputs(&captured, &root.join("run")).expect("stage");
    assert_eq!(
        fs::read(&staged.files()[0]).expect("staged bytes"),
        b"before"
    );
    assert!(staged.map_path().is_file());

    fs::remove_dir_all(root).expect("cleanup");
}

fn fresh_root(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/tune-tests")
        .join(format!("{name}-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).expect("old root");
    }
    fs::create_dir_all(&root).expect("root");
    root
}

fn manifest_bytes() -> Vec<u8> {
    br#"schema = 1
[runner]
path = "runner"
inputs = ["input.bin"]

[[case]]
id = "search"
role = "search"
seed = 1
weight = 1
expected_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[[case]]
id = "validation"
role = "validation"
seed = 2
weight = 1
expected_digest = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
"#
    .to_vec()
}

#[cfg(target_os = "macos")]
fn native_fixture_bytes() -> &'static [u8] {
    b"\xcf\xfa\xed\xfeck-runner"
}

#[cfg(target_os = "linux")]
fn native_fixture_bytes() -> &'static [u8] {
    b"\x7fELFck-runner"
}

#[cfg(windows)]
fn native_fixture_bytes() -> &'static [u8] {
    b"MZck-runner"
}
