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
    NativePlatform, create_native_static_archive, link_native_dynamic_library,
    link_native_executable,
};
pub use c::{
    BoundsMode, EmitCOptions, OverflowMode, emit_c_header, emit_c_kir_header, emit_c_kir_module,
    emit_c_kir_module_with_contracts, emit_c_module, emit_c_module_with_header, try_emit_c_module,
};
pub use header::{NativeHeaderMode, emit_native_header};
pub use llvm::{
    EmbeddedNotice, EmitLlvmOptions, NATIVE_ABI_VERSION, RUNTIME_ABI_VERSION, embedded_notices,
};
#[cfg(feature = "native-toolchain")]
pub use llvm::{
    LLVM_BRIDGE_ABI_VERSION, NativeBridgeInfo, NativeContext, NativeCpu, NativeError, NativeJit,
    NativeJitMemoryAudit, NativeLoweringOptions, NativeModule, NativeObject,
    NativeOptimizationLevel, NativeStage, NativeTarget, NativeToolchain, OptimizedNativeModule,
    OrcObjectLayer, VerifiedNativeModule, bridge_info, lower_native_executable_module_with_options,
    lower_native_llvm_module, lower_native_llvm_module_with_options,
    test_error as native_bridge_test_error, test_invalid_input as native_bridge_test_invalid_input,
    test_invalid_module_verification,
};
#[cfg(feature = "native-toolchain")]
pub use native_abi::{
    NativeAbiArgument, NativeAbiArgumentRole, NativeAbiClassifier, NativeAbiError,
    NativeAbiExtension, NativeAbiFunction, NativeAbiHiddenResult, NativeAbiLayout,
    NativeAbiPassMode, NativeAbiRegister, NativeAbiRegisterClass, NativeAbiTarget, NativeAbiValue,
};
pub use wasm::{
    EmitWasmOptions, emit_wasm_kir_module, emit_wasm_module, emit_wasm_module_with_options,
    emit_wat_kir_module, emit_wat_module, emit_wat_module_with_options,
};

pub(super) fn is_f64_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
}

pub(super) fn is_unsigned_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    )
}

pub(super) fn is_signed_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)
    )
}

pub(super) fn signed_min_constant(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => "INT32_MIN",
        MirType::Primitive(MirPrimitiveTypeName::I64) => "INT64_MIN",
        _ => unreachable!("signed minimum requested for non-signed type"),
    }
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
