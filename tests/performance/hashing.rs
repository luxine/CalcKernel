use std::fs;

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

#[test]
fn arm_unix_hashing_should_enable_runtime_detected_sha2_acceleration() {
    let manifest: toml::Value = fs::read_to_string(repo_root().join("Cargo.toml"))
        .expect("Cargo manifest")
        .parse()
        .expect("valid Cargo manifest");
    let target = manifest
        .get("target")
        .and_then(|targets| targets.get("cfg(all(target_arch = \"aarch64\", unix))"))
        .and_then(|target| target.get("dependencies"))
        .and_then(|dependencies| dependencies.get("sha2"));
    let features = target
        .and_then(|dependency| dependency.get("features"))
        .and_then(toml::Value::as_array);
    assert!(
        features.is_some_and(|features| features
            .iter()
            .any(|feature| feature.as_str() == Some("asm"))),
        "ARM Unix builds still select sha2's software-only default instead of its runtime-detected backend"
    );
    assert!(
        manifest["dependencies"]["sha2"].is_str(),
        "Windows and other targets must retain the portable default dependency"
    );
}

#[test]
fn hashing_backend_should_preserve_standard_sha256_vectors_and_chunk_boundaries() {
    for (input, expected) in [
        (
            Vec::new(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc".to_vec(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            vec![b'a'; 1_000_000],
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
        ),
    ] {
        assert_eq!(format!("{:x}", Sha256::digest(&input)), expected);
        for width in [1, 7, 63, 64, 65, 127, 256] {
            let mut digest = Sha256::new();
            for chunk in input.chunks(width) {
                digest.update(chunk);
            }
            assert_eq!(
                format!("{:x}", digest.finalize()),
                expected,
                "input length {}, chunk width {width}",
                input.len()
            );
        }
    }
}
