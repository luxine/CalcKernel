//! Private, installation-local cache for offline tuning.

mod entry;
mod evict;
mod path;
mod store;

pub use store::{CachedTuneEntry, TuneCache, TuneCacheReceipt};

/// Fixed tuning-cache namespace below CK's platform cache root.
pub const TUNE_CACHE_NAMESPACE: &str = "tune-v1";
/// Hard upper bound for all compile, measurement, and decision entries.
pub const TUNE_CACHE_HARD_LIMIT: u64 = 4 * 1024 * 1024 * 1024;

/// Mutually isolated cache domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TuneCacheDomain {
    Compile = 1,
    Measurement = 2,
    Decision = 3,
}

impl TuneCacheDomain {
    pub(super) const fn directory(self) -> &'static str {
        match self {
            Self::Compile => "compile",
            Self::Measurement => "measurement",
            Self::Decision => "decision",
        }
    }

    pub(super) const fn key_domain(self) -> &'static [u8] {
        match self {
            Self::Compile => b"CK-TUNE-COMPILE-KEY\0",
            Self::Measurement => b"CK-TUNE-MEASUREMENT-KEY\0",
            Self::Decision => b"CK-TUNE-COMPLETED-DECISION-KEY\0",
        }
    }
}

/// Full SHA-256 tuning-cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TuneCacheKey([u8; 32]);

impl TuneCacheKey {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(super) fn hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl From<[u8; 32]> for TuneCacheKey {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}
