//! Handle-relative, bounded, read-only source access for failure evidence.

use std::{
    ffi::{CStr, CString, OsStr, OsString},
    fs::{File, Metadata, OpenOptions, Permissions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::{
            ffi::OsStrExt, ffi::OsStringExt, fs::MetadataExt, fs::OpenOptionsExt,
            fs::PermissionsExt,
        },
    },
    path::{Component, Path},
};

use sha2::{Digest, Sha256};

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn name(value: &OsStr) -> io::Result<CString> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::other("not a single safe path component"));
    }
    CString::new(bytes).map_err(|_| io::Error::other("NUL in path component"))
}

fn owner(metadata: &Metadata, allow_root: bool) -> io::Result<()> {
    // SAFETY: geteuid has no pointer arguments or side effects.
    let uid = unsafe { libc::geteuid() };
    if (metadata.uid() != uid && !(allow_root && metadata.uid() == 0))
        || metadata.mode() & 0o022 != 0
    {
        return Err(io::Error::other(
            "unsafe owner or group/world-writable source",
        ));
    }
    Ok(())
}

pub(super) struct Directory(File);

impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("capture path must be absolute"));
        }
        let mut directory = Self(
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open("/")?,
        );
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(value) => {
                    let entry = name(value)?;
                    let file = directory.open_at(&entry, libc::O_RDONLY | libc::O_DIRECTORY)?;
                    owner(&file.metadata()?, true)?;
                    directory = Self(file);
                }
                _ => return Err(io::Error::other("capture path contains traversal")),
            }
        }
        directory.check_owned()?;
        Ok(directory)
    }

    fn open_at(&self, entry: &CStr, flags: libc::c_int) -> io::Result<File> {
        // SAFETY: the directory owns a live FD; entry is NUL-terminated. The
        // returned FD is uniquely transferred to File, including on errors.
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                entry.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat just returned this new, owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn check_owned(&self) -> io::Result<()> {
        let metadata = self.0.metadata()?;
        if !metadata.is_dir() {
            return Err(io::Error::other("not a directory"));
        }
        owner(&metadata, false)
    }

    pub(super) fn create_directory(&self, value: &str, allow_existing: bool) -> io::Result<Self> {
        self.check_owned()?;
        let entry = name(OsStr::new(value))?;
        // SAFETY: the owned parent descriptor and C string remain live.
        if unsafe { libc::mkdirat(self.0.as_raw_fd(), entry.as_ptr(), 0o700) } != 0 {
            let error = io::Error::last_os_error();
            if !allow_existing || error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let result = Self(self.open_at(&entry, libc::O_RDONLY | libc::O_DIRECTORY)?);
        result.check_owned()?;
        Ok(result)
    }

    pub(super) fn create_file(&self, value: &OsStr) -> io::Result<File> {
        self.check_owned()?;
        let file = self.open_at(&name(value)?, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        Ok(file)
    }

    pub(super) fn identity(&self) -> io::Result<Identity> {
        Ok(Identity::from(&self.0.metadata()?))
    }

    pub(super) fn names(&self, maximum: usize) -> io::Result<(Vec<OsString>, bool)> {
        self.check_owned()?;
        let descriptor = self.0.try_clone()?.into_raw_fd();
        // SAFETY: fdopendir takes ownership only on success. DirStream closes
        // it exactly once, and no other operation uses this duplicate FD.
        let pointer = unsafe { libc::fdopendir(descriptor) };
        if pointer.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: fdopendir failed, so ownership was not transferred.
            drop(unsafe { File::from_raw_fd(descriptor) });
            return Err(error);
        }
        let stream = DirStream(pointer);
        let mut names = Vec::new();
        loop {
            // SAFETY: this Darwin-only module uses this thread's errno slot;
            // stream is a live, exclusively owned directory stream.
            let entry = unsafe {
                *libc::__error() = 0;
                libc::readdir(stream.0)
            };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error);
                }
                names.sort();
                return Ok((names, false));
            }
            // SAFETY: readdir returned a live entry with a NUL-terminated
            // d_name, used only before the next readdir call.
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if names.len() == maximum {
                names.sort();
                return Ok((names, true));
            }
            names.push(OsString::from_vec(bytes.to_vec()));
        }
    }
}

struct DirStream(*mut libc::DIR);

impl Drop for DirStream {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the stream from fdopendir.
        unsafe { libc::closedir(self.0) };
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    pub(super) size: u64,
    uid: u32,
    mode: u32,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl From<&Metadata> for Identity {
    fn from(value: &Metadata) -> Self {
        Self {
            device: value.dev(),
            inode: value.ino(),
            size: value.len(),
            uid: value.uid(),
            mode: value.mode(),
            mtime: (value.mtime(), value.mtime_nsec()),
            ctime: (value.ctime(), value.ctime_nsec()),
        }
    }
}

pub(super) struct Source<'a> {
    parent: &'a Directory,
    entry: CString,
    file: File,
    identity: Identity,
}

pub(super) struct Copied {
    pub(super) identity: Identity,
    pub(super) sha256: String,
    pub(super) bytes: Vec<u8>,
}

impl<'a> Source<'a> {
    pub(super) fn open(parent: &'a Directory, value: &OsStr) -> io::Result<Self> {
        let entry = name(value)?;
        let file = parent.open_at(&entry, libc::O_RDONLY)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("capture source is not a regular file"));
        }
        owner(&metadata, false)?;
        Ok(Self {
            parent,
            entry,
            file,
            identity: Identity::from(&metadata),
        })
    }

    fn unchanged(&self) -> io::Result<()> {
        let current = self.parent.open_at(&self.entry, libc::O_RDONLY)?;
        if Identity::from(&self.file.metadata()?) != self.identity
            || Identity::from(&current.metadata()?) != self.identity
        {
            return Err(io::Error::other(
                "capture source changed during observation",
            ));
        }
        Ok(())
    }

    pub(super) fn copy_to(
        mut self,
        destination: &Directory,
        value: &OsStr,
        maximum: u64,
        remaining: &mut u64,
        retain_bytes: bool,
    ) -> io::Result<Copied> {
        self.unchanged()?;
        if self.identity.size > maximum || self.identity.size > *remaining {
            return Err(io::Error::other(
                "capture file or total byte bound exceeded",
            ));
        }
        let mut output = destination.create_file(value)?;
        *remaining -= self.identity.size;
        let mut left = self.identity.size;
        let mut hash = Sha256::new();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 65536];
        while left != 0 {
            let count = usize::try_from(left.min(buffer.len() as u64))
                .map_err(|_| io::Error::other("capture chunk size overflow"))?;
            self.file.read_exact(&mut buffer[..count])?;
            output.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            if retain_bytes {
                bytes.extend_from_slice(&buffer[..count]);
            }
            left -= count as u64;
        }
        if self.file.read(&mut buffer[..1])? != 0 {
            return Err(io::Error::other("capture source grew during read"));
        }
        self.unchanged()?;
        output.sync_all()?;
        Ok(Copied {
            identity: self.identity,
            sha256: hex(&hash.finalize()),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> std::path::PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/tune-capture-tests")
            .join(format!("handles-{}", super::super::super::unique_id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn changed_source(change: impl FnOnce(&Path)) {
        let root = root();
        let path = root.join("source");
        fs::write(&path, b"original source bytes").unwrap();
        let parent = Directory::open(&root).unwrap();
        let source = Source::open(&parent, OsStr::new("source")).unwrap();
        let output = parent.create_directory("destination", false).unwrap();
        change(&path);
        let mut budget = 1024;
        assert!(
            source
                .copy_to(&output, OsStr::new("copy"), 1024, &mut budget, false)
                .is_err()
        );
        assert!(!root.join("destination/copy").exists());
    }

    #[test]
    fn capture_rejects_source_truncated_after_open() {
        changed_source(|path| fs::write(path, b"short").unwrap());
    }

    #[test]
    fn capture_rejects_source_grown_after_open() {
        changed_source(|path| fs::write(path, b"longer than the original source bytes").unwrap());
    }

    #[test]
    fn capture_rejects_same_size_source_rewrite_after_open() {
        changed_source(|path| fs::write(path, b"changed! source bytes").unwrap());
    }

    #[test]
    fn capture_rejects_source_entry_replaced_after_open() {
        changed_source(|path| {
            fs::rename(path, path.with_extension("retained")).unwrap();
            fs::write(path, b"original source bytes").unwrap();
        });
    }

    #[test]
    fn capture_handle_reads_cannot_be_redirected_by_ancestor_replacement() {
        let root = root();
        let directory = root.join("original");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("source"), b"original data").unwrap();
        let parent = Directory::open(&directory).unwrap();
        let source = Source::open(&parent, OsStr::new("source")).unwrap();
        let destination = Directory::open(&root).unwrap();
        fs::rename(&directory, root.join("retained-directory")).unwrap();
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("source"), b"replacement data").unwrap();
        source
            .copy_to(&destination, OsStr::new("copy"), 1024, &mut 1024, false)
            .unwrap();
        assert_eq!(fs::read(root.join("copy")).unwrap(), b"original data");
    }

    #[test]
    fn capture_copy_never_overwrites_an_existing_file() {
        let root = root();
        fs::write(root.join("source"), b"source").unwrap();
        fs::write(root.join("copy"), b"previous evidence").unwrap();
        let parent = Directory::open(&root).unwrap();
        let source = Source::open(&parent, OsStr::new("source")).unwrap();
        assert!(
            source
                .copy_to(&parent, OsStr::new("copy"), 1024, &mut 1024, false)
                .is_err()
        );
        assert_eq!(fs::read(root.join("copy")).unwrap(), b"previous evidence");
    }

    #[test]
    fn capture_streaming_copy_retains_all_chunks_and_full_hash() {
        let root = root();
        let bytes = (0..150_003)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(root.join("source"), &bytes).unwrap();
        let parent = Directory::open(&root).unwrap();
        let source = Source::open(&parent, OsStr::new("source")).unwrap();
        let mut budget = bytes.len() as u64;
        let copied = source
            .copy_to(&parent, OsStr::new("copy"), budget, &mut budget, true)
            .unwrap();
        assert_eq!(copied.bytes, bytes);
        assert_eq!(fs::read(root.join("copy")).unwrap(), bytes);
        assert_eq!(copied.sha256, hex(&Sha256::digest(&bytes)));
        assert_eq!(budget, 0);
    }

    #[test]
    fn capture_components_reject_traversal_and_nul() {
        for value in [b"".as_slice(), b".", b"..", b"a/b", b"a\0b"] {
            assert!(name(OsStr::from_bytes(value)).is_err());
        }
    }
}
