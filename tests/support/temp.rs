#![allow(dead_code)]

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn unique_id() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos()
}

pub(crate) fn temp_dir(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", unique_id()))
}
