use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    error::NativeError,
    ffi::{self, CkcLlvmObject},
    module::NativeModule,
    passes::NativeOptimizationLevel,
};

/// A module accepted by LLVM after a selected PassBuilder pipeline.
#[derive(Debug)]
pub struct OptimizedNativeModule<'context> {
    pub(super) module: NativeModule<'context>,
    pub(super) level: NativeOptimizationLevel,
}

impl OptimizedNativeModule<'_> {
    /// Returns the optimization pipeline level that produced this module.
    #[must_use]
    pub fn optimization_level(&self) -> NativeOptimizationLevel {
        self.level
    }

    /// Prints the post-optimization module with LLVM's canonical printer.
    pub fn to_ir_string(&self) -> Result<String, NativeError> {
        ffi::module_print(self.module.shared_handle())
    }
}

/// Unique owner of verified target-machine object bytes.
#[derive(Debug)]
pub struct NativeObject {
    handle: NonNull<CkcLlvmObject>,
    len: usize,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeObject {
    pub(super) fn from_handle(handle: NonNull<CkcLlvmObject>) -> Self {
        Self {
            len: ffi::object_size(handle),
            handle,
            not_send_or_sync: PhantomData,
        }
    }

    /// Returns the emitted object size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether this object contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the validated object-file bytes owned by this value.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        let data = ffi::object_data(self.handle);
        debug_assert!(!data.is_null());
        // SAFETY: The bridge keeps this immutable vector live until `Drop`,
        // and `len` came from the same object handle.
        unsafe { std::slice::from_raw_parts(data, self.len) }
    }
}

impl Drop for NativeObject {
    fn drop(&mut self) {
        // SAFETY: `NativeObject` is the unique owner and calls dispose once.
        unsafe { ffi::object_dispose(self.handle) };
    }
}
