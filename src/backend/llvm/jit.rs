use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    error::NativeError,
    ffi::{self, CkcLlvmJit},
    object::NativeObject,
};
use crate::backend::native_runtime::embedded_runtime_objects;

/// ORC object-linking layer selected for the current release target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrcObjectLayer {
    /// LLVM JITLink with its in-process memory manager.
    JitLink,
    /// Reserve-enabled RuntimeDyld used only for COFF AArch64.
    RuntimeDyldCoffAarch64,
}

/// Internal evidence captured by the memory manager that owns JIT pages.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeJitMemoryAudit {
    pub allocations: u64,
    pub instruction_cache_finalizations: u64,
    pub relocation_write_non_execute: bool,
    pub final_code_read_execute: bool,
    pub final_data_non_execute: bool,
    pub darwin_map_jit: bool,
    pub darwin_thread_write_protection_supported: bool,
    pub darwin_thread_write_protection: bool,
}

/// Unique owner of an eager LLJIT instance.
#[derive(Debug)]
pub struct NativeJit {
    handle: NonNull<CkcLlvmJit>,
    object_layer: OrcObjectLayer,
    executed: bool,
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
            executed: false,
            not_send_or_sync: PhantomData,
        })
    }

    /// Returns the object layer fixed for this JIT instance.
    #[must_use]
    pub fn object_layer(&self) -> OrcObjectLayer {
        self.object_layer
    }

    /// Eagerly links one entry-bearing O3 object with the embedded runtime,
    /// resolves the complete object graph, and invokes its process `main`.
    ///
    /// This is an in-process primitive for the private `ckc run` child. Public
    /// callers must isolate untrusted raw-pointer or unchecked CK code in a
    /// child process before using it.
    #[doc(hidden)]
    pub fn execute_entry(&mut self, object: &NativeObject) -> Result<i32, NativeError> {
        if self.executed {
            return Err(NativeError::new(
                super::NativeStage::Orc,
                1,
                "native JIT instance already executed an object".to_string(),
            ));
        }
        self.executed = true;
        ffi::jit_execute(self.handle, object.as_bytes(), &embedded_runtime_objects())
    }

    /// Returns internal evidence recorded by the JIT memory manager.
    #[doc(hidden)]
    pub fn memory_audit(&self) -> Result<NativeJitMemoryAudit, NativeError> {
        ffi::jit_memory_audit(self.handle)
    }
}

impl Drop for NativeJit {
    fn drop(&mut self) {
        // SAFETY: `NativeJit` is the unique owner and calls dispose once.
        unsafe { ffi::jit_dispose(self.handle) };
    }
}
