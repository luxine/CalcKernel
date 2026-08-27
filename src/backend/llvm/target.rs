use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    error::NativeError,
    ffi::{self, CkcLlvmTarget},
    module::{NativeModule, NativeObject},
};

/// Unique owner of the host LLVM target machine.
#[derive(Debug)]
pub struct NativeTarget {
    handle: NonNull<CkcLlvmTarget>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeTarget {
    /// Detects and creates the current host target machine.
    ///
    /// # Errors
    /// Returns a target-stage error when the host target is unavailable.
    pub fn host() -> Result<Self, NativeError> {
        Ok(Self {
            handle: ffi::target_create_host()?,
            not_send_or_sync: PhantomData,
        })
    }

    /// Verifies and emits one module as a host object.
    ///
    /// # Errors
    /// Returns a typed module or object-emission error.
    pub fn emit_object(&self, module: &mut NativeModule<'_>) -> Result<NativeObject, NativeError> {
        ffi::target_emit_object(self.handle, module.handle()).map(NativeObject::from_handle)
    }
}

impl Drop for NativeTarget {
    fn drop(&mut self) {
        // SAFETY: `NativeTarget` is the unique owner and calls dispose once.
        unsafe { ffi::target_dispose(self.handle) };
    }
}
