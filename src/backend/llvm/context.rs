use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    error::NativeError,
    ffi::{self, CkcLlvmContext},
    jit::{NativeJit, OrcObjectLayer},
    target::NativeTarget,
};

/// Unique owner of one LLVM context.
#[derive(Debug)]
pub struct NativeContext {
    handle: NonNull<CkcLlvmContext>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeContext {
    /// Creates an isolated LLVM context.
    ///
    /// # Errors
    /// Returns a bridge-stage error if LLVM cannot allocate the context.
    pub fn new() -> Result<Self, NativeError> {
        Ok(Self {
            handle: ffi::context_create()?,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) fn handle(&self) -> NonNull<CkcLlvmContext> {
        self.handle
    }
}

impl Drop for NativeContext {
    fn drop(&mut self) {
        // SAFETY: `NativeContext` is the unique owner and calls dispose once.
        unsafe { ffi::context_dispose(self.handle) };
    }
}

/// Root owner for the native context, host target, and empty ORC instance.
#[derive(Debug)]
pub struct NativeToolchain {
    jit: NativeJit,
    target: NativeTarget,
    context: NativeContext,
}

impl NativeToolchain {
    /// Initializes the native owners in dependency order.
    ///
    /// # Errors
    /// Returns the exact context, target, or ORC stage that failed.
    pub fn new() -> Result<Self, NativeError> {
        let context = NativeContext::new()?;
        let target = NativeTarget::host()?;
        let jit = NativeJit::new()?;
        Ok(Self {
            context,
            target,
            jit,
        })
    }

    /// Returns the active ORC object layer.
    #[must_use]
    pub fn object_layer(&self) -> OrcObjectLayer {
        self.jit.object_layer()
    }

    /// Returns the owned context.
    #[must_use]
    pub fn context(&self) -> &NativeContext {
        &self.context
    }

    /// Returns the owned host target.
    #[must_use]
    pub fn target(&self) -> &NativeTarget {
        &self.target
    }
}
