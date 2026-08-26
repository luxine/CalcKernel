use std::process::Command;

#[test]
fn rust_rewrite_should_have_a_versioned_git_commit() {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run git rev-parse");

    assert!(
        output.status.success(),
        "Rust rewrite must be committed before release workflows can be run\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
