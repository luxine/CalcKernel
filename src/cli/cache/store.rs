use std::{
    fs::{self, File, FileTimes, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

#[cfg(not(unix))]
use super::evict::enforce_soft_limit;
use super::{
    entry::{CacheManifest, decode_entry, encode_entry},
    evict::{DEFAULT_SOFT_LIMIT, enforce_soft_limit_with},
};

#[derive(Debug, Clone)]
pub(super) struct CacheStore {
    root: PathBuf,
    #[cfg(unix)]
    directory: Arc<File>,
}

impl CacheStore {
    pub(super) fn open_at(root: PathBuf) -> Result<Self, String> {
        match fs::symlink_metadata(&root) {
            Ok(metadata) => validate_root(&root, &metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&root)
                    .map_err(|error| format!("create cache root {}: {error}", root.display()))?;
                set_private_directory(&root)?;
                let metadata = fs::symlink_metadata(&root)
                    .map_err(|error| format!("inspect cache root {}: {error}", root.display()))?;
                validate_root(&root, &metadata)?;
            }
            Err(error) => {
                return Err(format!("inspect cache root {}: {error}", root.display()));
            }
        }
        #[cfg(unix)]
        {
            let directory = open_directory_nofollow(&root)?;
            validate_root(
                &root,
                &directory.metadata().map_err(|error| {
                    format!("inspect opened cache root {}: {error}", root.display())
                })?,
            )?;
            Ok(Self {
                root,
                directory: Arc::new(directory),
            })
        }
        #[cfg(not(unix))]
        Ok(Self { root })
    }

    pub(super) fn read(&self, key: &str) -> Option<Vec<u8>> {
        if !valid_key(key) {
            return None;
        }
        let path = self.root.join(key);
        let mut file = self.open_entry(key).ok()?;
        let metadata = file.metadata().ok()?;
        if validate_entry(&path, &metadata).is_err()
            || metadata.len() > 256 * 1024 * 1024 + 32 * 1024
        {
            return None;
        }
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).ok()?);
        file.read_to_end(&mut bytes).ok()?;
        let object = decode_entry(key, &bytes).ok()?.object.to_vec();
        let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
        Some(object)
    }

    pub(super) fn write(&self, manifest: &CacheManifest, object: &[u8]) -> Result<(), String> {
        let bytes = encode_entry(manifest, object).map_err(str::to_string)?;
        let destination = self.root.join(&manifest.key);
        self.reject_unsafe_destination(&manifest.key, &destination)?;
        let (temporary_name, temporary, mut file) = self.open_temporary(&manifest.key)?;
        let result: Result<(), String> = (|| {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("write cache entry {}: {error}", temporary.display()))?;
            self.reject_unsafe_destination(&manifest.key, &destination)?;
            self.atomic_replace(&temporary_name, &temporary, &manifest.key, &destination)?;
            self.sync_directory();
            Ok(())
        })();
        if result.is_err() {
            self.remove_temporary(&temporary_name, &temporary);
        }
        result?;
        self.enforce_soft_limit();
        Ok(())
    }

    pub(super) fn clean(self) -> Result<(), String> {
        for candidate in fs::read_dir(&self.root)
            .map_err(|error| format!("read cache root {}: {error}", self.root.display()))?
        {
            let candidate = candidate.map_err(|error| format!("read cache entry: {error}"))?;
            let name = candidate
                .file_name()
                .into_string()
                .map_err(|_| "cache root contains a non-UTF-8 entry".to_string())?;
            #[cfg(unix)]
            unlink_at(&self.directory, &name)
                .map_err(|error| format!("remove cache entry {name}: {error}"))?;
            #[cfg(not(unix))]
            {
                let metadata = fs::symlink_metadata(candidate.path())
                    .map_err(|error| format!("inspect cache entry {name}: {error}"))?;
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    return Err(format!("refusing nested cache directory {name}"));
                }
                fs::remove_file(candidate.path())
                    .map_err(|error| format!("remove cache entry {name}: {error}"))?;
            }
        }
        fs::remove_dir(&self.root)
            .map_err(|error| format!("remove cache root {}: {error}", self.root.display()))
    }

    #[cfg(unix)]
    fn open_entry(&self, key: &str) -> std::io::Result<File> {
        open_read_nofollow_at(&self.directory, key)
    }

    #[cfg(not(unix))]
    fn open_entry(&self, key: &str) -> std::io::Result<File> {
        open_read_nofollow(&self.root.join(key))
    }

    #[cfg(unix)]
    fn reject_unsafe_destination(&self, key: &str, _path: &Path) -> Result<(), String> {
        reject_unsafe_destination_at(&self.directory, key)
    }

    #[cfg(not(unix))]
    fn reject_unsafe_destination(&self, _key: &str, path: &Path) -> Result<(), String> {
        reject_unsafe_destination(path)
    }

    #[cfg(unix)]
    fn open_temporary(&self, key: &str) -> Result<(String, PathBuf, File), String> {
        let (name, file) = open_new_private_at(&self.directory, key)?;
        Ok((name.clone(), self.root.join(name), file))
    }

    #[cfg(not(unix))]
    fn open_temporary(&self, key: &str) -> Result<(String, PathBuf, File), String> {
        let path = temporary_path(&self.root, key)?;
        let name = path
            .file_name()
            .expect("temporary path has a name")
            .to_string_lossy()
            .into_owned();
        let file = open_new_private(&path)?;
        Ok((name, path, file))
    }

    #[cfg(unix)]
    fn atomic_replace(
        &self,
        temporary_name: &str,
        _temporary: &Path,
        destination_name: &str,
        _destination: &Path,
    ) -> Result<(), String> {
        atomic_replace_at(&self.directory, temporary_name, destination_name)
    }

    #[cfg(not(unix))]
    fn atomic_replace(
        &self,
        _temporary_name: &str,
        temporary: &Path,
        _destination_name: &str,
        destination: &Path,
    ) -> Result<(), String> {
        atomic_replace(temporary, destination)
    }

    #[cfg(unix)]
    fn remove_temporary(&self, name: &str, _path: &Path) {
        let _ = unlink_at(&self.directory, name);
    }

    #[cfg(not(unix))]
    fn remove_temporary(&self, _name: &str, path: &Path) {
        let _ = fs::remove_file(path);
    }

    #[cfg(unix)]
    fn sync_directory(&self) {
        let _ = self.directory.sync_all();
    }

    #[cfg(not(unix))]
    fn sync_directory(&self) {
        sync_directory(&self.root);
    }

    #[cfg(unix)]
    fn enforce_soft_limit(&self) {
        let directory = Arc::clone(&self.directory);
        let _ = enforce_soft_limit_with(&self.root, DEFAULT_SOFT_LIMIT, move |path| {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return Err(std::io::Error::other("cache entry name is not UTF-8"));
            };
            unlink_at(&directory, name)
        });
    }

    #[cfg(not(unix))]
    fn enforce_soft_limit(&self) {
        let _ = enforce_soft_limit(&self.root, DEFAULT_SOFT_LIMIT);
    }
}

fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(unix_o_directory() | unix_o_nofollow() | unix_o_cloexec())
        .open(path)
        .map_err(|error| {
            format!(
                "open cache root {} without following: {error}",
                path.display()
            )
        })
}

#[cfg(unix)]
fn open_read_nofollow_at(directory: &File, name: &str) -> std::io::Result<File> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };

    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in cache key"))?;
    let descriptor = unsafe {
        // SAFETY: `name` is NUL-terminated, `directory` remains open for the
        // call, and the flags neither create nor mutate the entry.
        openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            unix_o_nofollow() | unix_o_cloexec(),
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe {
            // SAFETY: `openat` returned a new owned descriptor exactly once.
            File::from_raw_fd(descriptor)
        })
    }
}

#[cfg(unix)]
fn reject_unsafe_destination_at(directory: &File, name: &str) -> Result<(), String> {
    match open_read_nofollow_at(directory, name) {
        Ok(file) => validate_entry_metadata(
            &file
                .metadata()
                .map_err(|error| format!("inspect cache entry {name}: {error}"))?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("refusing unsafe cache entry {name}: {error}")),
    }
}

#[cfg(unix)]
fn open_new_private_at(directory: &File, key: &str) -> Result<(String, File), String> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..128 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!(".{key}.{}-{serial}.tmp", process::id());
        let c_name = CString::new(name.as_str()).expect("generated cache name has no NUL");
        let descriptor = unsafe {
            // SAFETY: `c_name` is NUL-terminated, the directory is a live
            // owner-checked descriptor, and O_EXCL gives this call ownership
            // of any successfully returned descriptor.
            openat(
                directory.as_raw_fd(),
                c_name.as_ptr(),
                unix_o_write_only()
                    | unix_o_create()
                    | unix_o_exclusive()
                    | unix_o_nofollow()
                    | unix_o_cloexec(),
                0o600_i32,
            )
        };
        if descriptor >= 0 {
            let file = unsafe {
                // SAFETY: `openat` returned a new owned descriptor exactly once.
                File::from_raw_fd(descriptor)
            };
            return Ok((name, file));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(format!("create cache temporary {name}: {error}"));
        }
    }
    Err("could not allocate cache temporary path".to_string())
}

#[cfg(unix)]
fn atomic_replace_at(directory: &File, source: &str, destination: &str) -> Result<(), String> {
    use std::{ffi::CString, os::fd::AsRawFd};

    let source = CString::new(source).expect("generated cache source name has no NUL");
    let destination = CString::new(destination).expect("cache key has no NUL");
    let result = unsafe {
        // SAFETY: Both names are relative, NUL-terminated, and resolved
        // against the same live directory descriptor, so replacement cannot
        // escape to a substituted pathname root.
        renameat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "commit cache entry failed: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn unlink_at(directory: &File, name: &str) -> std::io::Result<()> {
    use std::{ffi::CString, os::fd::AsRawFd};

    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in cache name"))?;
    let result = unsafe {
        // SAFETY: `name` is relative and NUL-terminated and the directory
        // descriptor remains live for the call.
        unlinkat(directory.as_raw_fd(), name.as_ptr(), 0)
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
const fn unix_o_directory() -> i32 {
    0x0010_0000
}
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const fn unix_o_directory() -> i32 {
    0x0000_4000
}
#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(all(target_os = "linux", target_arch = "aarch64"))
))]
const fn unix_o_directory() -> i32 {
    0x0001_0000
}
#[cfg(target_os = "macos")]
const fn unix_o_nofollow() -> i32 {
    0x0000_0100
}
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const fn unix_o_nofollow() -> i32 {
    0x0000_8000
}
#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(all(target_os = "linux", target_arch = "aarch64"))
))]
const fn unix_o_nofollow() -> i32 {
    0x0002_0000
}
#[cfg(target_os = "macos")]
const fn unix_o_cloexec() -> i32 {
    0x0100_0000
}
#[cfg(all(unix, not(target_os = "macos")))]
const fn unix_o_cloexec() -> i32 {
    0x0008_0000
}
#[cfg(target_os = "macos")]
const fn unix_o_write_only() -> i32 {
    0x0000_0001
}
#[cfg(all(unix, not(target_os = "macos")))]
const fn unix_o_write_only() -> i32 {
    0x0000_0001
}
#[cfg(target_os = "macos")]
const fn unix_o_create() -> i32 {
    0x0000_0200
}
#[cfg(all(unix, not(target_os = "macos")))]
const fn unix_o_create() -> i32 {
    0x0000_0040
}
#[cfg(target_os = "macos")]
const fn unix_o_exclusive() -> i32 {
    0x0000_0800
}
#[cfg(all(unix, not(target_os = "macos")))]
const fn unix_o_exclusive() -> i32 {
    0x0000_0080
}

#[cfg(unix)]
unsafe extern "C" {
    fn openat(directory: i32, path: *const core::ffi::c_char, flags: i32, ...) -> i32;
    fn renameat(
        old_directory: i32,
        old_path: *const core::ffi::c_char,
        new_directory: i32,
        new_path: *const core::ffi::c_char,
    ) -> i32;
    fn unlinkat(directory: i32, path: *const core::ffi::c_char, flags: i32) -> i32;
}

#[cfg(not(unix))]
fn reject_unsafe_destination(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(format!("refusing unsafe cache entry {}", path.display()))
        }
        Ok(metadata) => validate_entry(path, &metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect cache entry {}: {error}", path.display())),
    }
}

#[cfg(not(unix))]
fn temporary_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..128 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".{key}.{}-{serial}.tmp", process::id()));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => continue,
            Err(error) => return Err(format!("inspect cache temporary: {error}")),
        }
    }
    Err("could not allocate cache temporary path".to_string())
}

#[cfg(target_os = "windows")]
fn open_read_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .access_mode(GENERIC_READ | FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn open_read_nofollow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(not(unix))]
fn open_new_private(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create cache temporary {}: {error}", path.display()))
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("protect cache root {}: {error}", path.display()))
}

#[cfg(target_os = "windows")]
fn set_private_directory(path: &Path) -> Result<(), String> {
    windows_security::set_owner_only_directory(path)
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn set_private_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn validate_root(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(format!("cache root is not owner-only: {}", path.display()));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn validate_root(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    if metadata.file_type().is_symlink()
        || metadata.file_attributes() & 0x0000_0400 != 0
        || !metadata.is_dir()
    {
        return Err(format!("cache root is unsafe: {}", path.display()));
    }
    windows_security::validate_owner_only(path, true)
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn validate_root(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        Err(format!("cache root is unsafe: {}", path.display()))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn validate_entry_metadata(metadata: &fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !metadata.is_file()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("cache entry is not an owner-only regular file".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_entry(_path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    validate_entry_metadata(metadata)
}

#[cfg(target_os = "windows")]
fn validate_entry(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.file_attributes() & 0x0000_0400 != 0
    {
        return Err("cache entry is not a non-reparse regular file".to_string());
    }
    windows_security::validate_owner_only(path, false)
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn validate_entry(_path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    if !metadata.is_file() {
        return Err("cache entry is not a regular file".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe {
        // SAFETY: `getuid` takes no arguments and has no failure mode.
        getuid()
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn getuid() -> u32;
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| {
        format!(
            "commit cache entry {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(target_os = "windows")]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        // SAFETY: Both vectors are live, NUL-terminated UTF-16 paths and the
        // flags request same-volume atomic replacement with write-through.
        MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8)
    };
    if result == 0 {
        Err(format!(
            "commit cache entry failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}

#[cfg(target_os = "windows")]
mod windows_security {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};

    type Handle = *mut c_void;
    type Sid = *mut c_void;

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER_CLASS: u32 = 1;
    const SE_FILE_OBJECT: u32 = 1;
    const OWNER_SECURITY_INFORMATION: u32 = 0x0000_0001;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
    const SE_DACL_PROTECTED: u16 = 0x1000;
    const ACL_REVISION: u32 = 2;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const OBJECT_INHERIT_ACE: u32 = 0x01;
    const CONTAINER_INHERIT_ACE: u32 = 0x02;
    const FILE_ALL_ACCESS: u32 = 0x001f_01ff;

    #[repr(C)]
    struct SidAndAttributes {
        sid: Sid,
        attributes: u32,
    }

    #[repr(C)]
    struct TokenUser {
        user: SidAndAttributes,
    }

    #[repr(C)]
    struct Acl {
        revision: u8,
        sbz1: u8,
        size: u16,
        ace_count: u16,
        sbz2: u16,
    }

    #[repr(C)]
    struct AceHeader {
        ace_type: u8,
        ace_flags: u8,
        ace_size: u16,
    }

    pub(super) fn set_owner_only_directory(path: &Path) -> Result<(), String> {
        with_current_user_sid(|sid| {
            let sid_length = unsafe {
                // SAFETY: The SID points into the live token buffer owned by
                // `with_current_user_sid` and was returned by Windows.
                GetLengthSid(sid)
            };
            if sid_length == 0 {
                return Err(last_error("measure current Windows user SID"));
            }
            let acl_bytes = std::mem::size_of::<Acl>()
                + 8
                + usize::try_from(sid_length).map_err(|_| "SID length overflow".to_string())?;
            let mut acl_storage = vec![0usize; acl_bytes.div_ceil(std::mem::size_of::<usize>())];
            let acl = acl_storage.as_mut_ptr().cast::<Acl>();
            if unsafe {
                // SAFETY: `acl_storage` is aligned, writable, and at least
                // `acl_bytes` long.
                InitializeAcl(acl, acl_bytes as u32, ACL_REVISION)
            } == 0
            {
                return Err(last_error("initialize private Windows cache ACL"));
            }
            if unsafe {
                // SAFETY: The ACL and SID remain live and Windows validates
                // the requested access mask and inheritance flags.
                AddAccessAllowedAceEx(
                    acl,
                    ACL_REVISION,
                    OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                    FILE_ALL_ACCESS,
                    sid,
                )
            } == 0
            {
                return Err(last_error("populate private Windows cache ACL"));
            }
            let mut path = wide_path(path);
            let status = unsafe {
                // SAFETY: `path` is mutable NUL-terminated UTF-16 and the ACL
                // remains live through the synchronous call.
                SetNamedSecurityInfoW(
                    path.as_mut_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    acl,
                    ptr::null_mut(),
                )
            };
            status_result(status, "protect Windows cache root")
        })
    }

    pub(super) fn validate_owner_only(path: &Path, require_protected: bool) -> Result<(), String> {
        let mut path = wide_path(path);
        let mut owner: Sid = ptr::null_mut();
        let mut dacl: *mut Acl = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = unsafe {
            // SAFETY: All output pointers refer to live storage and Windows
            // allocates `descriptor`, which is released below with LocalFree.
            GetNamedSecurityInfoW(
                path.as_mut_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        status_result(status, "read Windows cache security descriptor")?;
        let result = with_current_user_sid(|current_sid| unsafe {
            // SAFETY: `owner`, `dacl`, and `descriptor` were populated by the
            // successful GetNamedSecurityInfoW call and remain live here.
            if owner.is_null() || EqualSid(owner, current_sid) == 0 {
                return Err("Windows cache object is not owned by the current user".to_string());
            }
            if dacl.is_null() || (*dacl).ace_count != 1 {
                return Err("Windows cache ACL is not owner-only".to_string());
            }
            if require_protected {
                let mut control = 0u16;
                let mut revision = 0u32;
                if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0 {
                    return Err(last_error("inspect Windows cache ACL control"));
                }
                if control & SE_DACL_PROTECTED == 0 {
                    return Err("Windows cache root ACL inherits unsafe permissions".to_string());
                }
            }
            let mut ace: *mut c_void = ptr::null_mut();
            if GetAce(dacl, 0, &mut ace) == 0 || ace.is_null() {
                return Err(last_error("read Windows cache ACL entry"));
            }
            let header = &*ace.cast::<AceHeader>();
            if header.ace_type != ACCESS_ALLOWED_ACE_TYPE || header.ace_size < 12 {
                return Err("Windows cache ACL has an unexpected entry".to_string());
            }
            let mask = ptr::read_unaligned(ace.cast::<u8>().add(4).cast::<u32>());
            let ace_sid = ace.cast::<u8>().add(8).cast::<c_void>();
            if mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS || EqualSid(ace_sid, current_sid) == 0 {
                return Err("Windows cache ACL grants insufficient or non-owner access".to_string());
            }
            Ok(())
        });
        unsafe {
            // SAFETY: `descriptor` is the allocation returned by
            // GetNamedSecurityInfoW and is released exactly once.
            LocalFree(descriptor);
        }
        result
    }

    fn with_current_user_sid<T>(
        operation: impl FnOnce(Sid) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut token: Handle = ptr::null_mut();
        if unsafe {
            // SAFETY: GetCurrentProcess returns a valid pseudo-handle and
            // `token` is writable output storage.
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
        } == 0
        {
            return Err(last_error("open current Windows process token"));
        }
        let result = (|| {
            let mut required = 0u32;
            unsafe {
                // SAFETY: A null first buffer is the documented sizing query.
                GetTokenInformation(token, TOKEN_USER_CLASS, ptr::null_mut(), 0, &mut required);
            }
            if required == 0 {
                return Err(last_error("measure Windows token user"));
            }
            let words = usize::try_from(required)
                .map_err(|_| "Windows token size overflow".to_string())?
                .div_ceil(std::mem::size_of::<usize>());
            let mut storage = vec![0usize; words];
            if unsafe {
                // SAFETY: `storage` is aligned and writable for `required`
                // bytes and the token handle remains live.
                GetTokenInformation(
                    token,
                    TOKEN_USER_CLASS,
                    storage.as_mut_ptr().cast(),
                    required,
                    &mut required,
                )
            } == 0
            {
                return Err(last_error("read Windows token user"));
            }
            let sid = unsafe {
                // SAFETY: Successful TokenUser output starts with TokenUser
                // and its SID pointer stays live while `storage` is live.
                (*storage.as_ptr().cast::<TokenUser>()).user.sid
            };
            if sid.is_null() {
                return Err("Windows token returned a null user SID".to_string());
            }
            operation(sid)
        })();
        unsafe {
            // SAFETY: `token` is the handle returned by OpenProcessToken and
            // is closed exactly once after the SID operation.
            CloseHandle(token);
        }
        result
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn status_result(status: u32, operation: &str) -> Result<(), String> {
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "{operation}: {}",
                std::io::Error::from_raw_os_error(status as i32)
            ))
        }
    }

    fn last_error(operation: &str) -> String {
        format!("{operation}: {}", std::io::Error::last_os_error())
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            information: *mut c_void,
            information_length: u32,
            return_length: *mut u32,
        ) -> i32;
        fn GetLengthSid(sid: Sid) -> u32;
        fn EqualSid(left: Sid, right: Sid) -> i32;
        fn InitializeAcl(acl: *mut Acl, length: u32, revision: u32) -> i32;
        fn AddAccessAllowedAceEx(
            acl: *mut Acl,
            revision: u32,
            flags: u32,
            access: u32,
            sid: Sid,
        ) -> i32;
        fn SetNamedSecurityInfoW(
            name: *mut u16,
            object_type: u32,
            security_information: u32,
            owner: Sid,
            group: Sid,
            dacl: *mut Acl,
            sacl: *mut Acl,
        ) -> u32;
        fn GetNamedSecurityInfoW(
            name: *mut u16,
            object_type: u32,
            security_information: u32,
            owner: *mut Sid,
            group: *mut Sid,
            dacl: *mut *mut Acl,
            sacl: *mut *mut Acl,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn GetSecurityDescriptorControl(
            descriptor: *mut c_void,
            control: *mut u16,
            revision: *mut u32,
        ) -> i32;
        fn GetAce(acl: *mut Acl, index: u32, ace: *mut *mut c_void) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc, thread};

    use super::CacheStore;
    use crate::cli::cache::entry::CacheManifest;

    fn root(label: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "ckc-cache-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    fn manifest(key: &str) -> CacheManifest {
        CacheManifest {
            key: key.to_string(),
            compiler_version: "0.10.0".to_string(),
            llvm_version: "22.1.8".to_string(),
            target_triple: "test-target".to_string(),
            cpu: "test-cpu".to_string(),
            features: String::new(),
            codegen_contract: "strict".to_string(),
            native_abi: 1,
            runtime_abi: 1,
            bridge_abi: 1,
            optimization_level: 3,
            overflow_mode: 0,
            bounds_mode: 0,
            kir_contract_version: 1,
            sanitizer_mode: 0,
            target_profile_digest: "31".repeat(32),
            vector_cost_model_schema: 1,
            vector_proof_schema: 1,
            vector_budget_identity: "vector-budget-schema=1;growth=20".to_string(),
        }
    }

    #[test]
    fn store_should_create_owner_only_root_and_atomically_round_trip_entry() {
        let root = root("roundtrip");
        let store = CacheStore::open_at(root.clone()).expect("open cache store");
        let key = "ab".repeat(32);
        store
            .write(&manifest(&key), b"object bytes")
            .expect("write cache entry");
        assert_eq!(store.read(&key), Some(b"object bytes".to_vec()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root)
                    .expect("root metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(root.join(&key))
                    .expect("entry metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn store_should_reject_unsafe_permissions_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let unsafe_root = root("unsafe");
        fs::create_dir_all(&unsafe_root).expect("create unsafe root");
        fs::set_permissions(&unsafe_root, fs::Permissions::from_mode(0o777))
            .expect("set unsafe mode");
        assert!(CacheStore::open_at(unsafe_root).is_err());

        let real = root("real");
        fs::create_dir_all(&real).expect("create real root");
        let linked = root("linked");
        symlink(&real, &linked).expect("link cache root");
        assert!(CacheStore::open_at(linked).is_err());

        let root = root("entry-link");
        let store = CacheStore::open_at(root.clone()).expect("open safe store");
        let key = "cd".repeat(32);
        let target = root.join("outside");
        fs::write(&target, b"outside").expect("write link target");
        symlink(&target, root.join(&key)).expect("link entry");
        assert_eq!(store.read(&key), None);
        assert!(store.write(&manifest(&key), b"object").is_err());
        assert_eq!(fs::read(target).expect("read untouched target"), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn open_store_should_not_follow_a_replaced_cache_root() {
        use std::os::unix::fs::symlink;

        let root = root("root-replacement");
        let store = CacheStore::open_at(root.clone()).expect("open anchored cache store");
        let original = root.with_extension("original");
        fs::rename(&root, &original).expect("move opened cache root");
        let outside = root.with_extension("outside");
        fs::create_dir_all(&outside).expect("create outside replacement");
        symlink(&outside, &root).expect("replace cache root with symlink");

        let key = "dc".repeat(32);
        let result = store.write(&manifest(&key), b"anchored object");
        assert!(
            result.is_err() || original.join(&key).is_file(),
            "a successful write must remain anchored to the opened root"
        );
        assert!(
            !outside.join(&key).exists(),
            "cache write escaped through replacement root"
        );
    }

    #[test]
    fn corrupt_entry_should_be_a_miss_and_concurrent_writers_should_converge() {
        let root = root("concurrent");
        let store = Arc::new(CacheStore::open_at(root.clone()).expect("open cache store"));
        let key = "ef".repeat(32);
        fs::write(root.join(&key), b"corrupt").expect("write corruption");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join(&key), fs::Permissions::from_mode(0o600))
                .expect("protect corrupt entry");
        }
        assert_eq!(store.read(&key), None);
        let workers = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let key = key.clone();
                thread::spawn(move || store.write(&manifest(&key), b"replacement"))
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker
                .join()
                .expect("join cache writer")
                .expect("cache writer");
        }
        assert_eq!(store.read(&key), Some(b"replacement".to_vec()));
    }
}
