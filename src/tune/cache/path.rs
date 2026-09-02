use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::TUNE_CACHE_NAMESPACE;

pub(super) fn resolve_default_cache_root() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        nonempty("LOCALAPPDATA").map(|root| PathBuf::from(root).join("CalcKernel/cache"))
    } else if cfg!(target_os = "macos") {
        nonempty("HOME").map(|root| PathBuf::from(root).join("Library/Caches/ckc"))
    } else {
        nonempty("XDG_CACHE_HOME")
            .map(|root| PathBuf::from(root).join("ckc"))
            .or_else(|| nonempty("HOME").map(|root| PathBuf::from(root).join(".cache/ckc")))
    }
}

fn nonempty(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

pub(super) fn prepare_root(base: &Path) -> Result<(PathBuf, [u8; 32]), String> {
    ensure_private_directory(base)?;
    let root = base.join(TUNE_CACHE_NAMESPACE);
    ensure_private_directory(&root)?;
    for child in ["compile", "measurement", "decision"] {
        ensure_private_directory(&root.join(child))?;
    }
    let salt_path = root.join("salt");
    let salt = match open_private_read(&salt_path) {
        Ok(mut file) => {
            let mut value = [0u8; 32];
            file.read_exact(&mut value)
                .map_err(|error| format!("read tuning cache salt: {error}"))?;
            let mut trailing = [0u8; 1];
            if file
                .read(&mut trailing)
                .map_err(|error| format!("read tuning cache salt: {error}"))?
                != 0
            {
                return Err("tuning cache salt has trailing bytes".to_string());
            }
            value
        }
        Err(error) if error.contains("not found") => {
            let value = random_salt()?;
            let mut file = create_private(&salt_path)?;
            use std::io::Write;
            file.write_all(&value)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("write tuning cache salt: {error}"))?;
            value
        }
        Err(error) => return Err(error),
    };
    let mut digest = Sha256::new();
    digest.update(b"CK-TUNE-MEASUREMENT-SALT\0");
    digest.update(32u32.to_be_bytes());
    digest.update(salt);
    Ok((root, digest.finalize().into()))
}

pub(super) fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_private_directory(path, &metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| {
                format!("create tuning cache directory {}: {error}", path.display())
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
                    format!("protect tuning cache directory {}: {error}", path.display())
                })?;
            }
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!("inspect tuning cache directory {}: {error}", path.display())
            })?;
            validate_private_directory(path, &metadata)
        }
        Err(error) => Err(format!(
            "inspect tuning cache directory {}: {error}",
            path.display()
        )),
    }
}

fn validate_private_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "tuning cache path is not a real directory: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::getuid() } || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(format!(
                "tuning cache directory is not owner-only: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn create_private(path: &Path) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| {
                format!(
                    "create private tuning cache file {}: {error}",
                    path.display()
                )
            })
    }
    #[cfg(not(unix))]
    OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            format!(
                "create private tuning cache file {}: {error}",
                path.display()
            )
        })
}

pub(super) fn open_private_read(path: &Path) -> Result<File, String> {
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
    };
    #[cfg(not(unix))]
    let file = OpenOptions::new().read(true).open(path);
    let file = file.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "tuning cache file not found".to_string()
        } else {
            format!("open private tuning cache file {}: {error}", path.display())
        }
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect tuning cache file {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("tuning cache entry is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::getuid() } || metadata.permissions().mode() & 0o077 != 0
        {
            return Err("tuning cache entry is not owner-only".to_string());
        }
    }
    Ok(file)
}

fn random_salt() -> Result<[u8; 32], String> {
    let mut salt = [0u8; 32];
    #[cfg(unix)]
    {
        let mut source = File::open("/dev/urandom")
            .map_err(|error| format!("open operating-system CSPRNG: {error}"))?;
        source
            .read_exact(&mut salt)
            .map_err(|error| format!("read operating-system CSPRNG: {error}"))?;
    }
    #[cfg(windows)]
    {
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                salt.as_mut_ptr(),
                salt.len() as u32,
                0x0000_0002,
            )
        };
        if status < 0 {
            return Err(format!("BCryptGenRandom failed with status {status}"));
        }
    }
    #[cfg(all(not(unix), not(windows)))]
    return Err("no operating-system CSPRNG adapter".to_string());
    Ok(salt)
}

#[cfg(windows)]
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut core::ffi::c_void,
        buffer: *mut u8,
        length: u32,
        flags: u32,
    ) -> i32;
}
