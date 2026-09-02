use std::{
    fs::{self, FileTimes},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use sha2::{Digest, Sha256};

use super::{
    TUNE_CACHE_HARD_LIMIT, TuneCacheDomain, TuneCacheKey,
    entry::{decode, encode},
    evict,
    path::{create_private, open_private_read, prepare_root, resolve_default_cache_root},
};

/// Immutable digest receipt for one checked cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuneCacheReceipt {
    key_digest: [u8; 32],
    entry_digest: [u8; 32],
}

impl TuneCacheReceipt {
    #[must_use]
    pub const fn key_digest(&self) -> &[u8; 32] {
        &self.key_digest
    }

    #[must_use]
    pub const fn entry_digest(&self) -> &[u8; 32] {
        &self.entry_digest
    }
}

/// Checked cache payload and the receipt that authenticated it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedTuneEntry {
    payload: Vec<u8>,
    receipt: TuneCacheReceipt,
}

impl CachedTuneEntry {
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub const fn receipt(&self) -> &TuneCacheReceipt {
        &self.receipt
    }
}

/// Owner-private tuning cache rooted below one CK cache base.
#[derive(Debug, Clone)]
pub struct TuneCache {
    root: PathBuf,
    salt_digest: [u8; 32],
    hard_limit: u64,
}

impl TuneCache {
    /// Opens the platform-default CK tuning cache.
    pub fn open_default() -> Result<Option<Self>, String> {
        resolve_default_cache_root().map_or(Ok(None), |root| Self::open_at(root).map(Some))
    }

    /// Opens `tune-v1` below `cache_base`, creating private directories and salt.
    pub fn open_at(cache_base: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_at_with_limit(cache_base, TUNE_CACHE_HARD_LIMIT)
    }

    /// Opens a cache with an explicit limit, used for deterministic limit tests.
    pub fn open_at_with_limit(
        cache_base: impl AsRef<Path>,
        hard_limit: u64,
    ) -> Result<Self, String> {
        let (root, salt_digest) = prepare_root(cache_base.as_ref())?;
        let cache = Self {
            root,
            salt_digest,
            hard_limit,
        };
        evict::enforce(&cache.root, cache.hard_limit)?;
        Ok(cache)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub const fn salt_digest(&self) -> &[u8; 32] {
        &self.salt_digest
    }

    /// Derives a domain-separated full key. Measurement keys additionally bind
    /// the private installation salt digest.
    #[must_use]
    pub fn derive_key(&self, domain: TuneCacheDomain, materials: &[&[u8]]) -> TuneCacheKey {
        let mut digest = Sha256::new();
        digest.update(domain.key_domain());
        digest.update(1u32.to_be_bytes());
        if domain == TuneCacheDomain::Measurement {
            digest.update(32u32.to_be_bytes());
            digest.update(self.salt_digest);
        }
        digest.update(
            u32::try_from(materials.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for material in materials {
            digest.update(
                u64::try_from(material.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            digest.update(material);
        }
        TuneCacheKey(digest.finalize().into())
    }

    #[must_use]
    pub fn entry_path(&self, domain: TuneCacheDomain, key: TuneCacheKey) -> PathBuf {
        self.root.join(domain.directory()).join(key.hex())
    }

    /// Reads and authenticates a complete entry. Corrupt entries are removed and
    /// reported as misses; unsafe filesystem objects are hard errors.
    pub fn read(
        &self,
        domain: TuneCacheDomain,
        key: TuneCacheKey,
    ) -> Result<Option<CachedTuneEntry>, String> {
        let path = self.entry_path(domain, key);
        let mut file = match open_private_read(&path) {
            Ok(file) => file,
            Err(error) if error.contains("not found") => return Ok(None),
            Err(error) => return Err(error),
        };
        let length = file
            .metadata()
            .map_err(|error| format!("inspect tuning cache entry: {error}"))?
            .len();
        if length > 1024 * 1024 * 1024 + 128 {
            drop(file);
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(length).map_err(|_| "tuning cache entry length overflow")?,
        );
        file.read_to_end(&mut bytes)
            .map_err(|error| format!("read tuning cache entry: {error}"))?;
        let decoded = match decode(domain, key, &bytes) {
            Ok(decoded) => decoded,
            Err(_) => {
                drop(file);
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
        };
        let payload = decoded.payload.to_vec();
        let receipt = TuneCacheReceipt {
            key_digest: *key.as_bytes(),
            entry_digest: decoded.digest,
        };
        let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
        Ok(Some(CachedTuneEntry { payload, receipt }))
    }

    /// Atomically publishes one complete authenticated entry and enforces the
    /// global 4 GiB limit across all three domains.
    pub fn write(
        &self,
        domain: TuneCacheDomain,
        key: TuneCacheKey,
        payload: &[u8],
    ) -> Result<TuneCacheReceipt, String> {
        let (bytes, entry_digest) = encode(domain, key, payload)?;
        let directory = self.root.join(domain.directory());
        let destination = self.entry_path(domain, key);
        if let Ok(metadata) = fs::symlink_metadata(&destination)
            && (!metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err("refusing unsafe tuning cache destination".to_string());
        }
        let nonce = temporary_nonce()?;
        let temporary = directory.join(format!(".ckc-tune-{}-{nonce}", key.hex()));
        let mut file = create_private(&temporary)?;
        let result = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write tuning cache entry: {error}"));
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        drop(file);
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("publish tuning cache entry: {error}"))?;
        sync_directory(&directory)?;
        evict::enforce(&self.root, self.hard_limit)?;
        Ok(TuneCacheReceipt {
            key_digest: *key.as_bytes(),
            entry_digest,
        })
    }

    /// Safely removes all tuning entries while retaining the installation salt.
    pub fn clean(&self) -> Result<(), String> {
        for domain in [
            TuneCacheDomain::Compile,
            TuneCacheDomain::Measurement,
            TuneCacheDomain::Decision,
        ] {
            let directory = self.root.join(domain.directory());
            for entry in
                fs::read_dir(&directory).map_err(|error| format!("read tuning cache: {error}"))?
            {
                let entry = entry.map_err(|error| format!("read tuning cache entry: {error}"))?;
                let metadata = fs::symlink_metadata(entry.path())
                    .map_err(|error| format!("inspect tuning cache entry: {error}"))?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("refusing unsafe object in tuning cache".to_string());
                }
                fs::remove_file(entry.path())
                    .map_err(|error| format!("remove tuning cache entry: {error}"))?;
            }
            sync_directory(&directory)?;
        }
        Ok(())
    }

    /// Safely removes the complete `tune-v1` namespace, including its private
    /// installation salt. This is reserved for the explicit `ckc cache clean`
    /// operation; normal tuning-cache cleanup retains the salt.
    pub fn remove_namespace(self) -> Result<(), String> {
        self.clean()?;
        let salt = self.root.join("salt");
        let salt_file = open_private_read(&salt)?;
        drop(salt_file);
        fs::remove_file(&salt).map_err(|error| format!("remove tuning cache salt: {error}"))?;
        for domain in [
            TuneCacheDomain::Compile,
            TuneCacheDomain::Measurement,
            TuneCacheDomain::Decision,
        ] {
            let directory = self.root.join(domain.directory());
            fs::remove_dir(&directory)
                .map_err(|error| format!("remove tuning cache directory: {error}"))?;
        }
        fs::remove_dir(&self.root)
            .map_err(|error| format!("remove tuning cache namespace: {error}"))
    }
}

fn temporary_nonce() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    {
        let mut file = std::fs::File::open("/dev/urandom")
            .map_err(|error| format!("open operating-system CSPRNG: {error}"))?;
        file.read_exact(&mut bytes)
            .map_err(|error| format!("read operating-system CSPRNG: {error}"))?;
    }
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        bytes[..4].copy_from_slice(&std::process::id().to_be_bytes());
        bytes[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| format!("open tuning cache directory: {error}"))?;
        directory
            .sync_all()
            .map_err(|error| format!("sync tuning cache directory: {error}"))?;
    }
    Ok(())
}
