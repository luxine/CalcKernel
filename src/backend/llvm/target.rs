use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    error::NativeError,
    ffi::{self, BridgeCpuPolicy, CkcLlvmTarget},
    object::{NativeObject, OptimizedNativeModule},
};

/// Host CPU feature policy for native object generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCpu {
    /// Documented architecture baseline, independent of the build host model.
    Baseline,
    /// Complete CPU and feature set detected on the current host.
    Native,
}

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
        Self::host_with_cpu(NativeCpu::Native)
    }

    /// Creates the host TargetMachine using an explicit CPU policy.
    pub fn host_with_cpu(cpu: NativeCpu) -> Result<Self, NativeError> {
        Ok(Self {
            handle: ffi::target_create_host(match cpu {
                NativeCpu::Baseline => BridgeCpuPolicy::Baseline,
                NativeCpu::Native => BridgeCpuPolicy::Native,
            })?,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) const fn handle(&self) -> NonNull<CkcLlvmTarget> {
        self.handle
    }

    /// Returns the normalized target triple owned by the host TargetMachine.
    pub fn triple(&self) -> Result<String, NativeError> {
        ffi::target_triple(self.handle)
    }

    /// Returns the exact host TargetMachine data-layout string.
    pub fn data_layout(&self) -> Result<String, NativeError> {
        ffi::target_data_layout(self.handle)
    }

    /// Returns LLVM's CPU name for this TargetMachine.
    pub fn cpu(&self) -> Result<String, NativeError> {
        ffi::target_cpu(self.handle)
    }

    /// Returns LLVM's complete feature string for this TargetMachine.
    pub fn features(&self) -> Result<String, NativeError> {
        ffi::target_features(self.handle)
    }

    /// Verifies and emits one module as a host object.
    ///
    /// # Errors
    /// Returns a typed module or object-emission error.
    pub fn emit_object(
        &self,
        mut module: OptimizedNativeModule<'_>,
    ) -> Result<NativeObject, NativeError> {
        ffi::target_emit_object(self.handle, module.module.handle()).map(NativeObject::from_handle)
    }

    /// Revalidates cached bytes as a host relocatable object before reuse.
    #[doc(hidden)]
    pub fn parse_cached_object(&self, bytes: &[u8]) -> Result<NativeObject, NativeError> {
        ffi::target_parse_object(self.handle, bytes).map(NativeObject::from_handle)
    }
}

impl Drop for NativeTarget {
    fn drop(&mut self) {
        // SAFETY: `NativeTarget` is the unique owner and calls dispose once.
        unsafe { ffi::target_dispose(self.handle) };
    }
}
