use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::Path,
};

use super::PublicationError;

pub(crate) fn random_transaction_id() -> Result<[u8; 16], PublicationError> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    {
        let mut source = OpenOptions::new().read(true).open("/dev/urandom")?;
        source.read_exact(&mut bytes)?;
    }
    #[cfg(windows)]
    {
        let status = unsafe {
            // SAFETY: the output slice is live and BCrypt fills exactly its length.
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                0x0000_0002,
            )
        };
        if status < 0 {
            return Err(PublicationError::Io(format!(
                "BCryptGenRandom failed with status {status}"
            )));
        }
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        return Err(PublicationError::Identity(
            "no operating-system CSPRNG adapter",
        ));
    }
    Ok(bytes)
}

pub(crate) fn create_private(path: &Path) -> Result<File, PublicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(Into::into)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .custom_flags(0x0020_0000)
            .open(path)?;
        if let Err(error) = windows_security::protect_owner_only(&file) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
        Ok(file)
    }
    #[cfg(all(not(unix), not(windows)))]
    OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(path)
        .map_err(Into::into)
}

pub(crate) fn make_executable(file: &File) -> Result<(), PublicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(any(windows, all(not(unix), not(windows))))]
    let _ = file;
    Ok(())
}

pub(crate) fn open_private_nofollow(path: &Path) -> Result<File, PublicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        validate_private_file(path, &file)?;
        Ok(file)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(0x0020_0000)
            .open(path)?;
        validate_private_file(path, &file)?;
        Ok(file)
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        validate_private_file(path, &file)?;
        Ok(file)
    }
}

pub(crate) fn open_regular_nofollow_read(path: &Path) -> Result<File, PublicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(PublicationError::Identity(
                "publication member is not a regular file",
            ));
        }
        Ok(file)
    }
    #[cfg(windows)]
    {
        use std::os::{windows::fs::MetadataExt, windows::fs::OpenOptionsExt};
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x0020_0000)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.file_attributes() & 0x0000_0400 != 0
        {
            return Err(PublicationError::Identity(
                "publication member is not a non-reparse regular file",
            ));
        }
        Ok(file)
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let file = OpenOptions::new().read(true).open(path)?;
        if !file.metadata()?.is_file() {
            return Err(PublicationError::Identity(
                "publication member is not a regular file",
            ));
        }
        Ok(file)
    }
}

pub(crate) fn read_private_nofollow(path: &Path, limit: u64) -> Result<Vec<u8>, PublicationError> {
    let mut file = open_private_nofollow(path)?;
    let length = file.metadata()?.len();
    if length > limit {
        return Err(PublicationError::Identity("reserved file exceeds bound"));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(length).map_err(|_| PublicationError::Identity("file size overflow"))?,
    );
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn validate_private_file(path: &Path, file: &File) -> Result<(), PublicationError> {
    let metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::getuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(PublicationError::Identity(
                "reserved file is not an owner-only regular file",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.file_attributes() & 0x0000_0400 != 0
        {
            return Err(PublicationError::Identity(
                "reserved file is not a non-reparse regular file",
            ));
        }
        windows_security::validate_owner_only(file)?;
    }
    #[cfg(all(not(unix), not(windows)))]
    if !metadata.is_file() {
        return Err(PublicationError::Identity("reserved file is not regular"));
    }
    let _ = path;
    Ok(())
}

pub(crate) fn atomic_no_replace(
    source: &Path,
    destination: &Path,
) -> Result<bool, PublicationError> {
    #[cfg(unix)]
    {
        match std::fs::hard_link(source, destination) {
            Ok(()) => {
                std::fs::remove_file(source)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(windows)]
    {
        match move_file(source, destination, false) {
            Ok(()) => Ok(true),
            Err(PublicationError::Io(_)) if destination.exists() => Ok(false),
            Err(error) => Err(error),
        }
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = (source, destination);
        Err(PublicationError::Identity(
            "atomic no-replace is unsupported",
        ))
    }
}

pub(crate) fn atomic_replace(source: &Path, destination: &Path) -> Result<(), PublicationError> {
    #[cfg(unix)]
    {
        std::fs::rename(source, destination)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        move_file(source, destination, true)
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = (source, destination);
        Err(PublicationError::Identity("atomic replace is unsupported"))
    }
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), PublicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        directory.sync_all()?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(path)?;
        directory.sync_all()?;
        Ok(())
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = path;
        Err(PublicationError::Identity("directory flush is unsupported"))
    }
}

pub(crate) struct AdvisoryLock {
    file: File,
}

impl AdvisoryLock {
    pub(crate) fn acquire(file: File) -> Result<Self, PublicationError> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let mut overlapped = unsafe { std::mem::zeroed() };
            let result = unsafe {
                // SAFETY: the handle and OVERLAPPED remain live for the call.
                windows_sys::Win32::Storage::FileSystem::LockFileEx(
                    file.as_raw_handle().cast(),
                    0x0000_0002,
                    0,
                    u32::MAX,
                    u32::MAX,
                    &mut overlapped,
                )
            };
            if result == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(Self { file })
    }
}

impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let mut overlapped = unsafe { std::mem::zeroed() };
            let _ = unsafe {
                windows_sys::Win32::Storage::FileSystem::UnlockFileEx(
                    self.file.as_raw_handle().cast(),
                    0,
                    u32::MAX,
                    u32::MAX,
                    &mut overlapped,
                )
            };
        }
    }
}

#[cfg(windows)]
fn move_file(source: &Path, destination: &Path, replace: bool) -> Result<(), PublicationError> {
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
    let flags = 0x0000_0008 | if replace { 0x0000_0001 } else { 0 };
    let result = unsafe {
        // SAFETY: both path buffers are live NUL-terminated UTF-16 strings.
        MoveFileExW(source.as_ptr(), destination.as_ptr(), flags)
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
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

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

#[cfg(windows)]
mod windows_security {
    use std::{ffi::c_void, fs::File, os::windows::io::AsRawHandle, ptr};

    use super::PublicationError;

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

    pub(super) fn protect_owner_only(file: &File) -> Result<(), PublicationError> {
        with_current_user_sid(|sid| {
            let sid_length = unsafe {
                // SAFETY: Windows supplied this SID in the live token buffer.
                GetLengthSid(sid)
            };
            if sid_length == 0 {
                return Err(last_error("measure current Windows user SID"));
            }
            let acl_bytes = std::mem::size_of::<Acl>()
                + 8
                + usize::try_from(sid_length)
                    .map_err(|_| PublicationError::Identity("Windows SID length overflow"))?;
            let mut storage = vec![0usize; acl_bytes.div_ceil(std::mem::size_of::<usize>())];
            let acl = storage.as_mut_ptr().cast::<Acl>();
            if unsafe {
                // SAFETY: `storage` is aligned and writable for `acl_bytes`.
                InitializeAcl(acl, acl_bytes as u32, ACL_REVISION)
            } == 0
            {
                return Err(last_error("initialize owner-only Windows ACL"));
            }
            if unsafe {
                // SAFETY: the ACL and current-user SID remain live for the call.
                AddAccessAllowedAceEx(acl, ACL_REVISION, 0, FILE_ALL_ACCESS, sid)
            } == 0
            {
                return Err(last_error("populate owner-only Windows ACL"));
            }
            let status = unsafe {
                // SAFETY: the file handle is live and the ACL remains live synchronously.
                SetSecurityInfo(
                    file.as_raw_handle().cast(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    acl,
                    ptr::null_mut(),
                )
            };
            status_result(status, "protect Windows publication file")
        })
    }

    pub(super) fn validate_owner_only(file: &File) -> Result<(), PublicationError> {
        let mut owner: Sid = ptr::null_mut();
        let mut dacl: *mut Acl = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = unsafe {
            // SAFETY: all output pointers are writable and the file handle is live.
            GetSecurityInfo(
                file.as_raw_handle().cast(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        status_result(status, "read Windows publication security descriptor")?;
        let result = with_current_user_sid(|current_sid| unsafe {
            // SAFETY: the descriptor and all returned pointers stay live until LocalFree below.
            if owner.is_null() || EqualSid(owner, current_sid) == 0 {
                return Err(PublicationError::Identity(
                    "Windows publication file is not owned by the current user",
                ));
            }
            if dacl.is_null() || (*dacl).ace_count != 1 {
                return Err(PublicationError::Identity(
                    "Windows publication ACL is not owner-only",
                ));
            }
            let mut control = 0u16;
            let mut revision = 0u32;
            if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0 {
                return Err(last_error("inspect Windows publication ACL control"));
            }
            if control & SE_DACL_PROTECTED == 0 {
                return Err(PublicationError::Identity(
                    "Windows publication ACL inherits permissions",
                ));
            }
            let mut ace: *mut c_void = ptr::null_mut();
            if GetAce(dacl, 0, &mut ace) == 0 || ace.is_null() {
                return Err(last_error("read Windows publication ACL entry"));
            }
            let header = &*ace.cast::<AceHeader>();
            if header.ace_type != ACCESS_ALLOWED_ACE_TYPE || header.ace_size < 12 {
                return Err(PublicationError::Identity(
                    "Windows publication ACL entry is invalid",
                ));
            }
            let mask = ptr::read_unaligned(ace.cast::<u8>().add(4).cast::<u32>());
            let ace_sid = ace.cast::<u8>().add(8).cast::<c_void>();
            if mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS || EqualSid(ace_sid, current_sid) == 0 {
                return Err(PublicationError::Identity(
                    "Windows publication ACL grants non-owner access",
                ));
            }
            Ok(())
        });
        unsafe {
            // SAFETY: `descriptor` is the allocation returned by GetSecurityInfo.
            LocalFree(descriptor);
        }
        result
    }

    fn with_current_user_sid<T>(
        operation: impl FnOnce(Sid) -> Result<T, PublicationError>,
    ) -> Result<T, PublicationError> {
        let mut token: Handle = ptr::null_mut();
        if unsafe {
            // SAFETY: the process pseudo-handle is valid and token is writable.
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
        } == 0
        {
            return Err(last_error("open current Windows process token"));
        }
        let result = (|| {
            let mut required = 0u32;
            unsafe {
                // SAFETY: null buffer is the documented sizing query.
                GetTokenInformation(token, TOKEN_USER_CLASS, ptr::null_mut(), 0, &mut required);
            }
            if required == 0 {
                return Err(last_error("measure Windows token user"));
            }
            let words = usize::try_from(required)
                .map_err(|_| PublicationError::Identity("Windows token size overflow"))?
                .div_ceil(std::mem::size_of::<usize>());
            let mut storage = vec![0usize; words];
            if unsafe {
                // SAFETY: storage is aligned and writable for the requested bytes.
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
                // SAFETY: successful TOKEN_USER output begins with TokenUser.
                (*storage.as_ptr().cast::<TokenUser>()).user.sid
            };
            if sid.is_null() {
                return Err(PublicationError::Identity(
                    "Windows token returned null SID",
                ));
            }
            operation(sid)
        })();
        unsafe {
            // SAFETY: token is the handle returned by OpenProcessToken.
            CloseHandle(token);
        }
        result
    }

    fn status_result(status: u32, operation: &str) -> Result<(), PublicationError> {
        if status == 0 {
            Ok(())
        } else {
            Err(PublicationError::Io(format!(
                "{operation}: {}",
                std::io::Error::from_raw_os_error(status as i32)
            )))
        }
    }

    fn last_error(operation: &str) -> PublicationError {
        PublicationError::Io(format!("{operation}: {}", std::io::Error::last_os_error()))
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
        fn SetSecurityInfo(
            handle: Handle,
            object_type: u32,
            security_information: u32,
            owner: Sid,
            group: Sid,
            dacl: *mut Acl,
            sacl: *mut Acl,
        ) -> u32;
        fn GetSecurityInfo(
            handle: Handle,
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
