use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use crate::backend::llvm::{NativeError, NativeObject, ffi};

/// Unique owner of a deterministic, LLVM-validated static archive.
#[derive(Debug)]
pub struct NativeArchive {
    handle: NonNull<ffi::CkcLlvmArchive>,
    len: usize,
    member_count: usize,
    has_symbol_index: bool,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeArchive {
    fn from_handle(handle: NonNull<ffi::CkcLlvmArchive>) -> Self {
        Self {
            len: ffi::archive_size(handle),
            member_count: ffi::archive_member_count(handle),
            has_symbol_index: ffi::archive_has_symbol_index(handle),
            handle,
            not_send_or_sync: PhantomData,
        }
    }

    /// Returns the validated archive bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        let data = ffi::archive_data(self.handle);
        debug_assert!(!data.is_null());
        // SAFETY: The bridge keeps this immutable vector live until `Drop`.
        unsafe { std::slice::from_raw_parts(data, self.len) }
    }

    /// Returns the number of regular members validated by LLVM.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.member_count
    }

    /// Returns whether LLVM parsed the archive symbol table.
    #[must_use]
    pub fn has_symbol_index(&self) -> bool {
        self.has_symbol_index
    }
}

impl Drop for NativeArchive {
    fn drop(&mut self) {
        // SAFETY: `NativeArchive` uniquely owns and disposes this handle once.
        unsafe { ffi::archive_dispose(self.handle) };
    }
}

/// Creates one deterministic static archive from a compiler-validated object.
pub fn create_native_static_archive(object: &NativeObject) -> Result<NativeArchive, NativeError> {
    let kind = match super::NativePlatform::host() {
        super::NativePlatform::Linux => ffi::BridgeArchiveKind::Gnu,
        super::NativePlatform::Darwin => ffi::BridgeArchiveKind::Darwin,
        super::NativePlatform::Windows => ffi::BridgeArchiveKind::Coff,
    };
    ffi::archive_create(object.shared_handle(), kind).map(NativeArchive::from_handle)
}
