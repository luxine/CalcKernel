use super::{
    builder::{NativeBuilder, NativeType},
    context::NativeContext,
    error::NativeError,
    ffi,
    module::NativeModule,
};

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

/// Builds and verifies a module that deliberately requests a name for a void call.
#[doc(hidden)]
pub fn test_named_void_call_module() -> Result<String, NativeError> {
    let context = NativeContext::new()?;
    let module = NativeModule::empty(&context)?;
    let void = NativeType::void(&context)?;
    let i32_type = NativeType::int(&context, 32)?;
    let sink = module.add_function("ck_void_sink", void, &[], true)?;
    let source = module.add_function("ck_value_source", i32_type, &[], true)?;
    let caller = module.add_function("ck_void_caller", void, &[], true)?;
    let entry = caller.append_block("entry")?;
    let mut builder = NativeBuilder::new(&context, &module)?;
    builder.position(entry)?;
    let _ = builder.call(source, &[], "ck.named.result")?;
    let _ = builder.call(sink, &[], "ck.requested.name")?;
    builder.return_void()?;
    drop(builder);
    module.verify()?.to_ir_string()
}
