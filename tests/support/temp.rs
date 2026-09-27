#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn unique_id() -> u128 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    (nanos << 64)
        ^ (u128::from(std::process::id()) << 32)
        ^ u128::from(NEXT.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn temp_dir(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", unique_id()))
}
