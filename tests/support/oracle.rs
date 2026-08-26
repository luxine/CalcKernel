#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub(crate) fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn typescript_root() -> PathBuf {
    std::env::var_os("CALCKERNEL_TS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/lynn/code/CalcKernel"))
}

pub(crate) fn typescript_cli() -> Option<PathBuf> {
    let cli = typescript_root().join("dist/src/cli.js");
    cli.exists().then_some(cli)
}
