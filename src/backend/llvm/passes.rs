use super::{
    error::{NativeError, NativeStage},
    ffi,
    object::OptimizedNativeModule,
    target::NativeTarget,
    verify::VerifiedNativeModule,
};

/// LLVM optimization level paired with the same CK MIR optimization level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NativeOptimizationLevel {
    O0 = 0,
    O1 = 1,
    O2 = 2,
    O3 = 3,
}

impl TryFrom<u8> for NativeOptimizationLevel {
    type Error = NativeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::O0),
            1 => Ok(Self::O1),
            2 => Ok(Self::O2),
            3 => Ok(Self::O3),
            _ => Err(NativeError::new(
                NativeStage::Module,
                1,
                format!("invalid native optimization level {value}; expected 0 through 3"),
            )),
        }
    }
}

impl<'context> VerifiedNativeModule<'context> {
    /// Runs LLVM's matching default PassBuilder pipeline and verifies again.
    pub fn optimize(
        self,
        target: &NativeTarget,
        level: NativeOptimizationLevel,
    ) -> Result<OptimizedNativeModule<'context>, NativeError> {
        ffi::module_optimize(self.module.shared_handle(), target.handle(), level as u8)?;
        Ok(OptimizedNativeModule {
            module: self.module,
            level,
        })
    }
}
