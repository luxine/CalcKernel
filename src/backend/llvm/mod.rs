#[cfg(feature = "native-toolchain")]
mod context;
mod emit;
#[cfg(feature = "native-toolchain")]
mod error;
#[cfg(feature = "native-toolchain")]
mod ffi;
#[cfg(feature = "native-toolchain")]
mod jit;
mod layout;
#[cfg(feature = "native-toolchain")]
mod module;
mod names;
mod notices;
#[cfg(feature = "native-toolchain")]
mod target;

#[cfg(feature = "native-toolchain")]
pub use context::{NativeContext, NativeToolchain};
pub use emit::emit_llvm_module;
#[cfg(feature = "native-toolchain")]
pub use error::{NativeError, NativeStage};
#[cfg(feature = "native-toolchain")]
pub use ffi::{
    LLVM_BRIDGE_ABI_VERSION, NativeBridgeInfo, bridge_info, test_error, test_invalid_input,
};
#[cfg(feature = "native-toolchain")]
pub use jit::{NativeJit, OrcObjectLayer};
#[cfg(feature = "native-toolchain")]
pub use module::{NativeModule, NativeObject};
pub use notices::{EmbeddedNotice, NATIVE_ABI_VERSION, RUNTIME_ABI_VERSION, embedded_notices};
#[cfg(feature = "native-toolchain")]
pub use target::NativeTarget;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitLlvmOptions {
    pub source_file_name: Option<String>,
    pub target_triple: Option<String>,
}
