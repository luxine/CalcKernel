use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    context::NativeContext,
    error::NativeError,
    fact_audit::NativeFactProperty,
    ffi::{self, CkcLlvmModule},
    target::NativeTarget,
};

/// Unique owner of one structural LLVM module tied to its context.
#[derive(Debug)]
pub struct NativeModule<'context> {
    handle: NonNull<CkcLlvmModule>,
    context: PhantomData<&'context NativeContext>,
    not_send_or_sync: PhantomData<Rc<()>>,
    pub(super) fact_properties: Vec<NativeFactProperty>,
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
            fact_properties: Vec::new(),
        })
    }

    pub(super) fn handle(&mut self) -> NonNull<CkcLlvmModule> {
        self.handle
    }

    pub(super) const fn shared_handle(&self) -> NonNull<CkcLlvmModule> {
        self.handle
    }

    pub(super) fn configure(
        &mut self,
        target: &NativeTarget,
        source_file_name: &str,
    ) -> Result<(), NativeError> {
        ffi::module_configure(self.handle, target.handle(), source_file_name)
    }

    pub(super) fn register_fact_property(&mut self, property: NativeFactProperty) {
        self.fact_properties.push(property);
    }
}

impl Drop for NativeModule<'_> {
    fn drop(&mut self) {
        // SAFETY: `NativeModule` is the unique owner and its context lifetime
        // is still active while the bridge destroys the module.
        unsafe { ffi::module_dispose(self.handle) };
    }
}
