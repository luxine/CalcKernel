use super::{context::NativeContext, error::NativeError, ffi, module::NativeModule};

/// A structural LLVM module accepted by LLVM's verifier.
#[derive(Debug)]
pub struct VerifiedNativeModule<'context> {
    pub(super) module: NativeModule<'context>,
}

impl<'context> NativeModule<'context> {
    /// Verifies the module and promotes it to a verified typed state.
    pub fn verify(self) -> Result<VerifiedNativeModule<'context>, NativeError> {
        ffi::module_verify(self.shared_handle())?;
        Ok(VerifiedNativeModule { module: self })
    }
}

impl VerifiedNativeModule<'_> {
    /// Prints the verified module using LLVM's canonical printer.
    pub fn to_ir_string(&self) -> Result<String, NativeError> {
        ffi::module_print(self.module.shared_handle())
    }
}

/// Exercises LLVM verifier rejection without exposing unsafe construction.
#[doc(hidden)]
pub fn test_invalid_module_verification() -> NativeError {
    let context = NativeContext::new().expect("LLVM test context creation must succeed");
    let module = NativeModule::empty(&context).expect("LLVM test module creation must succeed");
    ffi::module_make_invalid_for_test(module.shared_handle())
        .expect("LLVM invalid-module test hook must succeed");
    module
        .verify()
        .expect_err("LLVM verifier must reject an unterminated block")
}
