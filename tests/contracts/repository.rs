use std::fs;

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn repository_should_declare_v0_9_everywhere() {
    let cargo = read("Cargo.toml");
    let lock = read("Cargo.lock");
    assert!(cargo.contains("version = \"0.9.0\""));
    assert!(lock.contains("name = \"calckernel\"\nversion = \"0.9.0\""));
    for path in [
        "README.md",
        "README.zh-CN.md",
        "CHANGELOG.md",
        "CHANGELOG.zh-CN.md",
    ] {
        assert!(read(path).contains("0.9.0"), "{path} must name 0.9.0");
    }
}
