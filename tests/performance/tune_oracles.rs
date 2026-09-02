use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

#[test]
fn tune_oracle_manifest_pins_exact_source_bytes_and_modes() {
    let root = repo_root();
    let manifest = fs::read_to_string(root.join("benches/oracles/tune/manifest.toml"))
        .expect("tune oracle manifest");
    for relative in [
        "benches/oracles/tune/c/tune_oracle.c",
        "benches/oracles/tune/rust/tune_oracle.rs",
    ] {
        let bytes = fs::read(root.join(relative)).expect("oracle source");
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(manifest.contains(&format!("sha256 = \"{digest}\"")));
    }
    for required in [
        "clang_version = \"22.1.8\"",
        "rust_version = \"1.90.0\"",
        "fast_math = false",
        "fp_contraction = false",
        "overflow = \"unchecked-defined-inputs\"",
        "bounds = \"unchecked-defined-inputs\"",
        "-DCK_TUNE_GENERIC=1",
    ] {
        assert!(
            manifest.contains(required),
            "missing oracle contract {required}"
        );
    }
    assert_eq!(manifest.matches("[[case]]").count(), 7);
}

#[test]
fn tune_archive_producer_is_directly_executable_and_deterministic() {
    let root = repo_root();
    let producer = root.join("scripts/package-v014-performance-archive.py");
    #[cfg(unix)]
    {
        let mode = fs::metadata(&producer)
            .expect("archive producer")
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o111,
            0,
            "archive producer must be directly executable"
        );
    }
    let source = fs::read_to_string(&producer).expect("archive producer source");
    assert!(source.starts_with("#!/usr/bin/python3\n"));
    for required in ["PAX_FORMAT", "compresslevel=9", "mtime=0", "filename=\"\""] {
        assert!(
            source.contains(required),
            "missing deterministic archive rule {required}"
        );
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let temporary = root
        .join("target/ckc-perf/tests")
        .join(format!("archive-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&temporary).expect("temporary archive directory");
    let compiler = std::env::current_exe().expect("current test executable");
    let first = temporary.join("first.tar.gz");
    let second = temporary.join("second.tar.gz");
    for output in [&first, &second] {
        let status = Command::new(&producer)
            .current_dir(&root)
            .args(["--compiler"])
            .arg(&compiler)
            .args([
                "--license",
                "LICENSE",
                "--notices",
                "THIRD_PARTY_NOTICES.md",
                "--out",
            ])
            .arg(output)
            .status()
            .expect("run archive producer");
        assert!(status.success());
    }
    assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
    fs::remove_dir_all(temporary).expect("remove temporary archive directory");
}
