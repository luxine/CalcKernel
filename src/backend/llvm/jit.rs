use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    error::NativeError,
    ffi::{self, CkcLlvmJit},
};

/// ORC object-linking layer selected for the current release target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrcObjectLayer {
    /// LLVM JITLink with its in-process memory manager.
    JitLink,
    /// Reserve-enabled RuntimeDyld used only for COFF AArch64.
    RuntimeDyldCoffAarch64,
}

/// Unique owner of an eager LLJIT instance.
#[derive(Debug)]
pub struct NativeJit {
    handle: NonNull<CkcLlvmJit>,
    object_layer: OrcObjectLayer,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeJit {
    /// Creates an empty host LLJIT with the release object-layer policy.
    ///
    /// # Errors
    /// Returns an ORC-stage error when host detection or LLJIT setup fails.
    pub fn new() -> Result<Self, NativeError> {
        let handle = ffi::jit_create()?;
        let object_layer = match ffi::jit_object_layer(handle) {
            1 => OrcObjectLayer::JitLink,
            2 => OrcObjectLayer::RuntimeDyldCoffAarch64,
            value => {
                // SAFETY: The handle has not escaped and must be returned when
                // the bridge reports an unsupported layer identifier.
                unsafe { ffi::jit_dispose(handle) };
                return Err(NativeError::new(
                    super::NativeStage::Orc,
                    3,
                    format!("bridge returned unknown ORC object layer {value}"),
                ));
            }
        };
        Ok(Self {
            handle,
            object_layer,
            not_send_or_sync: PhantomData,
        })
    }

    /// Returns the object layer fixed for this JIT instance.
    #[must_use]
    pub fn object_layer(&self) -> OrcObjectLayer {
        self.object_layer
    }
}

impl Drop for NativeJit {
    fn drop(&mut self) {
        // SAFETY: `NativeJit` is the unique owner and calls dispose once.
        unsafe { ffi::jit_dispose(self.handle) };
    }
}
