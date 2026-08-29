#[cfg(feature = "native-toolchain")]
mod abi;
#[cfg(feature = "native-toolchain")]
mod builder;
#[cfg(feature = "native-toolchain")]
mod context;
#[cfg(feature = "native-toolchain")]
mod entry;
#[cfg(feature = "native-toolchain")]
mod error;
#[cfg(feature = "native-toolchain")]
mod fact_audit;
#[cfg(feature = "native-toolchain")]
pub(super) mod ffi;
#[cfg(feature = "native-toolchain")]
mod jit;
#[cfg(feature = "native-toolchain")]
mod kir_lower;
#[cfg(feature = "native-toolchain")]
mod layout;
#[cfg(feature = "native-toolchain")]
mod lower_shared;
#[cfg(feature = "native-toolchain")]
mod module;
#[cfg(feature = "native-toolchain")]
mod names;
mod notices;
#[cfg(feature = "native-toolchain")]
mod object;
#[cfg(feature = "native-toolchain")]
mod passes;
#[cfg(feature = "native-toolchain")]
mod target;
#[cfg(feature = "native-toolchain")]
mod verify;

#[cfg(feature = "native-toolchain")]
pub use context::{NativeContext, NativeToolchain};
#[cfg(feature = "native-toolchain")]
pub use error::{NativeError, NativeStage};
#[cfg(feature = "native-toolchain")]
pub use fact_audit::{
    AuditedNativeModule, NativeFactAuditReport, NativeFactProperty, NativeFactSource,
    NativeStrengtheningKind, test_inject_untracked_flag, test_inject_untracked_strengthening,
};
#[cfg(feature = "native-toolchain")]
pub use ffi::{
    LLVM_BRIDGE_ABI_VERSION, NativeBridgeInfo, bridge_info, test_error, test_invalid_input,
};
#[cfg(feature = "native-toolchain")]
pub use jit::{NativeJit, NativeJitMemoryAudit, OrcObjectLayer};
#[cfg(feature = "native-toolchain")]
pub use kir_lower::lower_native_kir_module;
#[cfg(feature = "native-toolchain")]
pub use module::NativeModule;
pub use notices::{EmbeddedNotice, NATIVE_ABI_VERSION, RUNTIME_ABI_VERSION, embedded_notices};
#[cfg(feature = "native-toolchain")]
pub use object::{NativeObject, OptimizedNativeModule};
#[cfg(feature = "native-toolchain")]
pub use passes::NativeOptimizationLevel;
#[cfg(feature = "native-toolchain")]
pub use target::{NativeCpu, NativeTarget};
#[cfg(feature = "native-toolchain")]
pub use verify::{VerifiedNativeModule, test_invalid_module_verification};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitLlvmOptions {
    pub source_file_name: Option<String>,
    pub target_triple: Option<String>,
}
