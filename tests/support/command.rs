#![allow(dead_code)]

use std::{
    path::Path,
    process::{Command, Output},
};

pub(crate) fn available(program: &str, version_arg: &str) -> bool {
    Command::new(program)
        .arg(version_arg)
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(crate) fn clang_available() -> bool {
    available("clang", "--version")
}

pub(crate) fn node_available() -> bool {
    available("node", "--version")
}

pub(crate) fn python3_available() -> bool {
    available("python3", "--version")
}

pub(crate) fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(crate) fn run_stdout(binary: &Path) -> String {
    let output = Command::new(binary).output().expect("run native harness");
    assert!(
        output.status.success(),
        "native harness {:?} failed with {:?}:\n{}",
        binary,
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("native output should be UTF-8")
}
