#[cfg(feature = "native-toolchain")]
mod artifact;
mod c;
mod header;
mod llvm;
#[cfg(feature = "native-toolchain")]
mod native_abi;
#[cfg(feature = "native-toolchain")]
mod native_runtime;
mod wasm;

use std::collections::HashSet;

use crate::{MirFunction, MirInstruction, MirPlace, MirPrimitiveTypeName, MirType, MirValue};

#[cfg(feature = "native-toolchain")]
pub use artifact::{
    NativeArchive, NativeArtifactKind, NativeArtifactPaths, NativeDynamicLibrary, NativeExecutable,
    NativePlatform, VerifiedNativeBuild, build_verified_native_artifact,
    create_native_multiversion_static_archive, create_native_profile_generation_static_archive,
    create_native_static_archive, link_native_dynamic_library, link_native_executable,
    link_native_multiversion_dynamic_library, link_native_multiversion_executable,
    link_native_profile_generation_dynamic_library, link_native_profile_generation_executable,
};
pub use c::{
    BoundsMode, EmitCOptions, OverflowMode, emit_c_kir_header, emit_c_kir_module,
    emit_c_kir_module_with_contracts,
};
pub use header::{
    NativeHeaderMode, annotate_unsafe_contracts, emit_native_header,
    emit_native_profile_generation_header,
};
#[cfg(feature = "native-toolchain")]
pub use llvm::{
    Aarch64AuxvSnapshot, AuditedNativeModule, CkLateProfileFunctionLayout, CkLateProfileLayoutPlan,
    CkLateProfileLayoutReport, CkLateProfileRepair, LLVM_BRIDGE_ABI_VERSION, NativeBridgeInfo,
    NativeCapabilityCache, NativeCapabilitySet, NativeContext, NativeCpu, NativeDispatchCandidate,
    NativeDispatchCell, NativeDispatchTable, NativeDispatchThunkContract, NativeDispatchTier,
    NativeError, NativeFactAuditReport, NativeFactProperty, NativeFactSource, NativeJit,
    NativeJitMemoryAudit, NativeModule, NativeMultiversionObject, NativeMultiversionObjectBundle,
    NativeMultiversionObjectRole, NativeMultiversionTargetSet, NativeObject,
    NativeOptimizationLevel, NativeProfileGeneration, NativeStage, NativeStrengtheningKind,
    NativeTarget, NativeToolchain, OptimizedNativeModule, OrcObjectLayer, VerifiedNativeModule,
    X86CpuidSnapshot, bridge_info, build_late_profile_layout_plan, build_tune_layout_plan,
    detect_aarch64_auxv, detect_host_cpu_capabilities, detect_x86_cpuid,
    emit_native_multiversion_objects, lower_native_kir_module,
    lower_native_multiversion_baseline_module, lower_native_multiversion_variant_module,
    lower_native_profile_generation_module, test_add_multiversion_dispatch,
    test_apply_late_layout_bytes, test_error as native_bridge_test_error,
    test_inject_untracked_flag as native_fact_audit_test_inject_untracked_flag,
    test_inject_untracked_strengthening as native_fact_audit_test_inject_untracked,
    test_invalid_input as native_bridge_test_invalid_input, test_invalid_module_verification,
};
pub use llvm::{
    EmbeddedNotice, EmitLlvmOptions, NATIVE_ABI_VERSION, NATIVE_CACHE_ENTRY_MAGIC,
    NATIVE_CACHE_KEY_SCHEMA, NATIVE_CACHE_MANIFEST_SCHEMA, RUNTIME_ABI_VERSION, embedded_notices,
};
#[cfg(feature = "native-toolchain")]
pub use native_abi::{
    NativeAbiArgument, NativeAbiArgumentRole, NativeAbiClassifier, NativeAbiError,
    NativeAbiExtension, NativeAbiFunction, NativeAbiHiddenResult, NativeAbiLayout,
    NativeAbiPassMode, NativeAbiRegister, NativeAbiRegisterClass, NativeAbiTarget, NativeAbiValue,
};
#[cfg(feature = "native-toolchain")]
pub use native_runtime::{
    NATIVE_DISPATCH_RUNTIME_SCHEMA, NATIVE_DISPATCH_RUNTIME_SHA256, NATIVE_PROFILE_RUNTIME_SCHEMA,
    NATIVE_PROFILE_RUNTIME_SHA256, embedded_dispatch_runtime_object,
};
pub use wasm::{EmitWasmOptions, emit_wasm_kir_module, emit_wat_kir_module};

pub(super) fn is_f64_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
}

pub(super) fn is_unsigned_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    )
}

pub(super) fn value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}

pub(super) fn place_type(place: &MirPlace) -> &MirType {
    match place {
        MirPlace::Param { type_node, .. }
        | MirPlace::Local { type_node, .. }
        | MirPlace::Deref { type_node, .. }
        | MirPlace::Index { type_node, .. }
        | MirPlace::SliceIndex { type_node, .. }
        | MirPlace::Field { type_node, .. } => type_node,
    }
}

pub(super) fn collect_temps(function: &MirFunction) -> Vec<(String, MirType)> {
    let mut temps = Vec::new();
    let mut seen = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, type_node }) = instruction_target(instruction)
                && seen.insert(name.clone())
            {
                temps.push((name.clone(), type_node.clone()));
            }
        }
    }
    temps
}

pub(super) fn instruction_target(instruction: &MirInstruction) -> Option<&MirValue> {
    match instruction {
        MirInstruction::ConstInt { target, .. }
        | MirInstruction::ConstFloat { target, .. }
        | MirInstruction::ConstBool { target, .. }
        | MirInstruction::Move { target, .. }
        | MirInstruction::Binary { target, .. }
        | MirInstruction::Unary { target, .. }
        | MirInstruction::Compare { target, .. }
        | MirInstruction::Cast { target, .. }
        | MirInstruction::Address { target, .. }
        | MirInstruction::Load { target, .. }
        | MirInstruction::MakeSlice { target, .. }
        | MirInstruction::SliceData { target, .. }
        | MirInstruction::SliceLen { target, .. }
        | MirInstruction::Subslice { target, .. } => Some(target),
        MirInstruction::Call { target, .. } => target.as_ref(),
        MirInstruction::Store { .. } | MirInstruction::RuntimeCall { .. } => None,
    }
}
