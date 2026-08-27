#![cfg(feature = "native-toolchain")]

use std::path::{Path, PathBuf};

pub(super) fn object_output_path(path: &Path) -> PathBuf {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if matches!(extension, Some("o" | "obj")) {
        return path.to_path_buf();
    }
    if cfg!(target_os = "windows") {
        path.with_extension("obj")
    } else {
        path.with_extension("o")
    }
}
