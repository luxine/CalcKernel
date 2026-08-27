use std::fmt;

/// Native compilation stage that produced an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStage {
    /// Versioned C/C++ bridge setup or validation.
    Bridge,
    /// LLVM context ownership.
    Context,
    /// Structural LLVM module ownership or verification.
    Module,
    /// Host target-machine ownership.
    Target,
    /// Target-machine object emission or ownership.
    Object,
    /// ORC JIT ownership or linking.
    Orc,
}

impl fmt::Display for NativeStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bridge => formatter.write_str("LLVM bridge"),
            Self::Context => formatter.write_str("LLVM context"),
            Self::Module => formatter.write_str("LLVM module"),
            Self::Target => formatter.write_str("LLVM target"),
            Self::Object => formatter.write_str("LLVM object"),
            Self::Orc => formatter.write_str("LLVM ORC"),
        }
    }
}

/// Error returned by the self-contained native toolchain.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{stage} failed with code {code}: {message}")]
pub struct NativeError {
    /// Compiler stage that failed.
    pub stage: NativeStage,
    /// Stable bridge or compiler-specific numeric error code.
    pub code: i32,
    /// Owned diagnostic text copied from the native boundary.
    pub message: String,
}

impl NativeError {
    pub(super) fn new(stage: NativeStage, code: i32, message: String) -> Self {
        Self {
            stage,
            code,
            message,
        }
    }
}
