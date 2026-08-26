#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub(crate) fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn configured_typescript_root() -> Option<PathBuf> {
    std::env::var_os("CALCKERNEL_TS_ROOT").map(PathBuf::from)
}

pub(crate) fn typescript_root() -> PathBuf {
    configured_typescript_root()
        .expect("CALCKERNEL_TS_ROOT must be configured before using the TypeScript oracle")
}

pub(crate) fn typescript_cli() -> Option<PathBuf> {
    let cli = configured_typescript_root()?.join("dist/src/cli.js");
    cli.exists().then_some(cli)
}
