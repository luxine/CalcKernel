use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use crate::backend::{
    llvm::{NativeError, NativeMultiversionObjectBundle, NativeObject, NativeTarget, ffi},
    native_runtime::embedded_profile_runtime_object,
};

/// Unique owner of a deterministic, LLVM-validated static archive.
#[derive(Debug)]
pub struct NativeArchive {
    handle: NonNull<ffi::CkcLlvmArchive>,
    len: usize,
    member_count: usize,
    has_symbol_index: bool,
    member_names: Vec<String>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeArchive {
    fn from_handle(handle: NonNull<ffi::CkcLlvmArchive>, member_names: Vec<String>) -> Self {
        Self {
            len: ffi::archive_size(handle),
            member_count: ffi::archive_member_count(handle),
            has_symbol_index: ffi::archive_has_symbol_index(handle),
            member_names,
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

    /// Returns the exact validated archive member order.
    #[must_use]
    pub fn member_names(&self) -> Vec<&str> {
        self.member_names.iter().map(String::as_str).collect()
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
    let names = vec!["ck_module.o".to_string()];
    ffi::archive_create(&[object.shared_handle()], &names, kind)
        .map(|handle| NativeArchive::from_handle(handle, names))
}

/// Creates a deterministic archive whose physical members preserve the
/// compiler-verified multiversion object names and order.
pub fn create_native_multiversion_static_archive(
    bundle: &NativeMultiversionObjectBundle,
) -> Result<NativeArchive, NativeError> {
    bundle.validate()?;
    let kind = match super::NativePlatform::host() {
        super::NativePlatform::Linux => ffi::BridgeArchiveKind::Gnu,
        super::NativePlatform::Darwin => ffi::BridgeArchiveKind::Darwin,
        super::NativePlatform::Windows => ffi::BridgeArchiveKind::Coff,
    };
    let handles = bundle
        .objects()
        .iter()
        .map(|object| object.object().shared_handle())
        .collect::<Vec<_>>();
    let names = bundle
        .objects()
        .iter()
        .map(|object| object.name().to_string())
        .collect::<Vec<_>>();
    ffi::archive_create(&handles, &names, kind)
        .map(|handle| NativeArchive::from_handle(handle, names))
}

/// Creates a generation-only archive with the module and private collector as
/// separate, indexed object members.
pub fn create_native_profile_generation_static_archive(
    target: &NativeTarget,
    object: &NativeObject,
) -> Result<NativeArchive, NativeError> {
    let runtime = target.parse_cached_object(embedded_profile_runtime_object())?;
    let kind = match super::NativePlatform::host() {
        super::NativePlatform::Linux => ffi::BridgeArchiveKind::Gnu,
        super::NativePlatform::Darwin => ffi::BridgeArchiveKind::Darwin,
        super::NativePlatform::Windows => ffi::BridgeArchiveKind::Coff,
    };
    let names = vec![
        "ck_module.o".to_string(),
        "ck_profile_runtime.o".to_string(),
    ];
    ffi::archive_create(
        &[object.shared_handle(), runtime.shared_handle()],
        &names,
        kind,
    )
    .map(|handle| NativeArchive::from_handle(handle, names))
}
