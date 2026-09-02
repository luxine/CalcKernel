#[cfg(feature = "native-toolchain")]
mod abi;
#[cfg(feature = "native-toolchain")]
mod builder;
#[cfg(feature = "native-toolchain")]
mod context;
#[cfg(feature = "native-toolchain")]
mod dispatch;
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
mod late_layout;
#[cfg(feature = "native-toolchain")]
mod layout;
#[cfg(feature = "native-toolchain")]
mod lower_shared;
#[cfg(feature = "native-toolchain")]
mod module;
#[cfg(feature = "native-toolchain")]
mod multiversion;
#[cfg(feature = "native-toolchain")]
mod names;
mod notices;
#[cfg(feature = "native-toolchain")]
mod object;
#[cfg(feature = "native-toolchain")]
mod passes;
#[cfg(feature = "native-toolchain")]
mod profile_generation;
#[cfg(feature = "native-toolchain")]
mod target;
#[cfg(feature = "native-toolchain")]
mod verify;

#[cfg(feature = "native-toolchain")]
pub use context::{NativeContext, NativeToolchain};
#[cfg(feature = "native-toolchain")]
pub use dispatch::{
    Aarch64AuxvSnapshot, NativeCapabilityCache, NativeCapabilitySet, NativeDispatchCandidate,
    NativeDispatchCell, NativeDispatchTable, NativeDispatchThunkContract, NativeDispatchTier,
    X86CpuidSnapshot, detect_aarch64_auxv, detect_host_cpu_capabilities, detect_x86_cpuid,
};
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
pub use kir_lower::{
    lower_native_kir_module, lower_native_multiversion_baseline_module,
    lower_native_multiversion_variant_module, lower_native_profile_generation_module,
    test_add_multiversion_dispatch,
};
#[cfg(feature = "native-toolchain")]
pub use late_layout::{
    CkLateProfileFunctionLayout, CkLateProfileLayoutPlan, CkLateProfileLayoutReport,
    CkLateProfileRepair, build_late_profile_layout_plan, build_tune_layout_plan,
    test_apply_late_layout_bytes,
};
#[cfg(feature = "native-toolchain")]
pub use module::NativeModule;
#[cfg(feature = "native-toolchain")]
pub use multiversion::{
    NativeMultiversionObject, NativeMultiversionObjectBundle, NativeMultiversionObjectRole,
    NativeMultiversionTargetSet, emit_native_multiversion_objects,
};
pub use notices::{
    EmbeddedNotice, NATIVE_ABI_VERSION, NATIVE_CACHE_ENTRY_MAGIC, NATIVE_CACHE_KEY_SCHEMA,
    NATIVE_CACHE_MANIFEST_SCHEMA, RUNTIME_ABI_VERSION, embedded_notices,
};
#[cfg(feature = "native-toolchain")]
pub use object::{NativeObject, OptimizedNativeModule};
#[cfg(feature = "native-toolchain")]
pub use passes::NativeOptimizationLevel;
#[cfg(feature = "native-toolchain")]
pub use profile_generation::NativeProfileGeneration;
#[cfg(feature = "native-toolchain")]
pub use target::{NativeCpu, NativeTarget};
#[cfg(feature = "native-toolchain")]
pub use verify::{VerifiedNativeModule, test_invalid_module_verification};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitLlvmOptions {
    pub source_file_name: Option<String>,
    pub target_triple: Option<String>,
}
