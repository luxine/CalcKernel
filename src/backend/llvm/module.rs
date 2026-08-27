use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    context::NativeContext,
    error::NativeError,
    ffi::{self, CkcLlvmModule, CkcLlvmObject},
};

/// Unique owner of one structural LLVM module tied to its context.
#[derive(Debug)]
pub struct NativeModule<'context> {
    handle: NonNull<CkcLlvmModule>,
    context: PhantomData<&'context NativeContext>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'context> NativeModule<'context> {
    /// Creates a valid empty module in `context`.
    ///
    /// # Errors
    /// Returns a module-stage error if LLVM cannot allocate the module.
    pub fn empty(context: &'context NativeContext) -> Result<Self, NativeError> {
        Ok(Self {
            handle: ffi::module_create_empty(context.handle())?,
            context: PhantomData,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) fn handle(&mut self) -> NonNull<CkcLlvmModule> {
        self.handle
    }
}

impl Drop for NativeModule<'_> {
    fn drop(&mut self) {
        // SAFETY: `NativeModule` is the unique owner and its context lifetime
        // is still active while the bridge destroys the module.
        unsafe { ffi::module_dispose(self.handle) };
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
}

impl Drop for NativeObject {
    fn drop(&mut self) {
        // SAFETY: `NativeObject` is the unique owner and calls dispose once.
        unsafe { ffi::object_dispose(self.handle) };
    }
}
