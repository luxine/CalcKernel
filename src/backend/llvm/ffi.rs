use std::{ptr, ptr::NonNull, slice};

use super::error::{NativeError, NativeStage};

pub const LLVM_BRIDGE_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeBinaryOp {
    Add = 1,
    Sub = 2,
    Mul = 3,
    SDiv = 4,
    UDiv = 5,
    SRem = 6,
    URem = 7,
    FAdd = 8,
    FSub = 9,
    FMul = 10,
    FDiv = 11,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeUnaryOp {
    Neg = 1,
    FNeg = 2,
    Not = 3,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeCompareOp {
    IcmpEq = 1,
    IcmpNe = 2,
    IcmpSlt = 3,
    IcmpSle = 4,
    IcmpSgt = 5,
    IcmpSge = 6,
    IcmpUlt = 7,
    IcmpUle = 8,
    IcmpUgt = 9,
    IcmpUge = 10,
    FcmpOeq = 11,
    FcmpUne = 12,
    FcmpOlt = 13,
    FcmpOle = 14,
    FcmpOgt = 15,
    FcmpOge = 16,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeCastOp {
    Sext = 1,
    Zext = 2,
    Sitofp = 3,
    Uitofp = 4,
    IntToPtr = 5,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeCpuPolicy {
    Baseline = 1,
    Native = 2,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeOverflowOp {
    SignedAdd = 1,
    UnsignedAdd = 2,
    SignedSub = 3,
    UnsignedSub = 4,
    SignedMul = 5,
    UnsignedMul = 6,
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub(super) enum BridgeAttributeKind {
    ZeroExt = 1,
    SignExt = 2,
    Sret = 3,
    ByVal = 4,
}

#[repr(C)]
#[derive(Debug)]
struct CkcLlvmOwnedBytes {
    data: *mut u8,
    len: usize,
}

impl CkcLlvmOwnedBytes {
    const fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
struct CkcLlvmError {
    code: i32,
    message: CkcLlvmOwnedBytes,
}

impl CkcLlvmError {
    const fn empty() -> Self {
        Self {
            code: 0,
            message: CkcLlvmOwnedBytes::empty(),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
struct CkcLlvmBridgeInfo {
    abi_version: u32,
    llvm_version: CkcLlvmOwnedBytes,
    host_triple: CkcLlvmOwnedBytes,
}

#[repr(C)]
#[derive(Debug, Default)]
struct CkcLlvmJitMemoryAudit {
    allocations: u64,
    instruction_cache_finalizations: u64,
    relocation_write_non_execute: u32,
    final_code_read_execute: u32,
    final_data_non_execute: u32,
    darwin_map_jit: u32,
    darwin_thread_write_protection_supported: u32,
    darwin_thread_write_protection: u32,
}

#[repr(C)]
pub(super) struct CkcLlvmContext {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmModule {
    _private: [u8; 0],
}

#[repr(C)]
pub(in crate::backend) struct CkcLlvmObject {
    _private: [u8; 0],
}

#[repr(C)]
pub(in crate::backend) struct CkcLlvmArchive {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmTarget {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmJit {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmBuilder {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmType {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmValue {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmFunction {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmBlock {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CkcLlvmBytes {
    data: *const u8,
    len: usize,
}

impl CkcLlvmBytes {
    fn new(value: &str) -> Self {
        Self::from_bytes(value.as_bytes())
    }

    fn from_bytes(value: &[u8]) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len(),
        }
    }
}

impl CkcLlvmBridgeInfo {
    const fn empty() -> Self {
        Self {
            abi_version: 0,
            llvm_version: CkcLlvmOwnedBytes::empty(),
            host_triple: CkcLlvmOwnedBytes::empty(),
        }
    }
}

unsafe extern "C" {
    fn ckc_llvm_bridge_info(out: *mut CkcLlvmBridgeInfo, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_test_error(error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_owned_bytes_dispose(bytes: *mut CkcLlvmOwnedBytes);
    fn ckc_llvm_context_create(out: *mut *mut CkcLlvmContext, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_context_dispose(context: *mut CkcLlvmContext);
    fn ckc_llvm_module_create_empty(
        context: *mut CkcLlvmContext,
        out: *mut *mut CkcLlvmModule,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_module_dispose(module: *mut CkcLlvmModule);
    fn ckc_llvm_module_configure(
        module: *mut CkcLlvmModule,
        target: *mut CkcLlvmTarget,
        source_file_name: CkcLlvmBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_module_verify(module: *mut CkcLlvmModule, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_module_print(
        module: *mut CkcLlvmModule,
        out: *mut CkcLlvmOwnedBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_create_host(
        cpu_policy: u32,
        out: *mut *mut CkcLlvmTarget,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_dispose(target: *mut CkcLlvmTarget);
    fn ckc_llvm_target_triple(
        target: *mut CkcLlvmTarget,
        out: *mut CkcLlvmOwnedBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_data_layout(
        target: *mut CkcLlvmTarget,
        out: *mut CkcLlvmOwnedBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_cpu(
        target: *mut CkcLlvmTarget,
        out: *mut CkcLlvmOwnedBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_features(
        target: *mut CkcLlvmTarget,
        out: *mut CkcLlvmOwnedBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_module_optimize(
        module: *mut CkcLlvmModule,
        target: *mut CkcLlvmTarget,
        opt_level: u32,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_module_make_invalid_for_test(
        module: *mut CkcLlvmModule,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_void(
        context: *mut CkcLlvmContext,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_int(
        context: *mut CkcLlvmContext,
        bits: u32,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_f64(
        context: *mut CkcLlvmContext,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_ptr(
        context: *mut CkcLlvmContext,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_slice(
        context: *mut CkcLlvmContext,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_array(
        element: *mut CkcLlvmType,
        count: u32,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_struct(
        context: *mut CkcLlvmContext,
        fields: *const *mut CkcLlvmType,
        field_count: usize,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_named_struct(
        context: *mut CkcLlvmContext,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmType,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_type_set_struct_body(
        type_node: *mut CkcLlvmType,
        fields: *const *mut CkcLlvmType,
        field_count: usize,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_module_add_function(
        module: *mut CkcLlvmModule,
        name: CkcLlvmBytes,
        return_type: *mut CkcLlvmType,
        params: *const *mut CkcLlvmType,
        param_count: usize,
        exported: u32,
        out: *mut *mut CkcLlvmFunction,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_module_preserve_function(
        module: *mut CkcLlvmModule,
        function: *mut CkcLlvmFunction,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_function_param(
        function: *mut CkcLlvmFunction,
        index: usize,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_function_append_block(
        function: *mut CkcLlvmFunction,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmBlock,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_function_add_attribute(
        function: *mut CkcLlvmFunction,
        kind: u32,
        return_attribute: u32,
        param_index: usize,
        pointee_type: *mut CkcLlvmType,
        alignment: u32,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_function_set_dll_export(
        function: *mut CkcLlvmFunction,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_create(
        context: *mut CkcLlvmContext,
        out: *mut *mut CkcLlvmBuilder,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_dispose(builder: *mut CkcLlvmBuilder);
    fn ckc_llvm_builder_position(
        builder: *mut CkcLlvmBuilder,
        block: *mut CkcLlvmBlock,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_alloca(
        builder: *mut CkcLlvmBuilder,
        type_node: *mut CkcLlvmType,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_load(
        builder: *mut CkcLlvmBuilder,
        type_node: *mut CkcLlvmType,
        pointer: *mut CkcLlvmValue,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_store(
        builder: *mut CkcLlvmBuilder,
        value: *mut CkcLlvmValue,
        pointer: *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_const_int(
        type_node: *mut CkcLlvmType,
        text: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_const_float(
        type_node: *mut CkcLlvmType,
        text: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_const_bool(
        context: *mut CkcLlvmContext,
        value: u32,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_const_undef(
        type_node: *mut CkcLlvmType,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_binary(
        builder: *mut CkcLlvmBuilder,
        op: u32,
        left: *mut CkcLlvmValue,
        right: *mut CkcLlvmValue,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_overflow(
        builder: *mut CkcLlvmBuilder,
        op: u32,
        left: *mut CkcLlvmValue,
        right: *mut CkcLlvmValue,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_unary(
        builder: *mut CkcLlvmBuilder,
        op: u32,
        value: *mut CkcLlvmValue,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_compare(
        builder: *mut CkcLlvmBuilder,
        op: u32,
        left: *mut CkcLlvmValue,
        right: *mut CkcLlvmValue,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_cast(
        builder: *mut CkcLlvmBuilder,
        op: u32,
        value: *mut CkcLlvmValue,
        target_type: *mut CkcLlvmType,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_gep(
        builder: *mut CkcLlvmBuilder,
        element_type: *mut CkcLlvmType,
        pointer: *mut CkcLlvmValue,
        indices: *const *mut CkcLlvmValue,
        index_count: usize,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_extract_value(
        builder: *mut CkcLlvmBuilder,
        aggregate: *mut CkcLlvmValue,
        index: u32,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_insert_value(
        builder: *mut CkcLlvmBuilder,
        aggregate: *mut CkcLlvmValue,
        value: *mut CkcLlvmValue,
        index: u32,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_select(
        builder: *mut CkcLlvmBuilder,
        condition: *mut CkcLlvmValue,
        then_value: *mut CkcLlvmValue,
        else_value: *mut CkcLlvmValue,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_call(
        builder: *mut CkcLlvmBuilder,
        function: *mut CkcLlvmFunction,
        args: *const *mut CkcLlvmValue,
        arg_count: usize,
        name: CkcLlvmBytes,
        out: *mut *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_return_void(builder: *mut CkcLlvmBuilder, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_builder_return(
        builder: *mut CkcLlvmBuilder,
        value: *mut CkcLlvmValue,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_branch(
        builder: *mut CkcLlvmBuilder,
        target: *mut CkcLlvmBlock,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_builder_cond_branch(
        builder: *mut CkcLlvmBuilder,
        condition: *mut CkcLlvmValue,
        then_block: *mut CkcLlvmBlock,
        else_block: *mut CkcLlvmBlock,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_emit_object(
        target: *mut CkcLlvmTarget,
        module: *mut CkcLlvmModule,
        out: *mut *mut CkcLlvmObject,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_target_parse_object(
        target: *mut CkcLlvmTarget,
        object_bytes: CkcLlvmBytes,
        out: *mut *mut CkcLlvmObject,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_object_size(object: *const CkcLlvmObject) -> usize;
    fn ckc_llvm_object_data(object: *const CkcLlvmObject) -> *const u8;
    fn ckc_llvm_object_dispose(object: *mut CkcLlvmObject);
    fn ckc_llvm_archive_create(
        object: *const CkcLlvmObject,
        kind: u32,
        out: *mut *mut CkcLlvmArchive,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_archive_size(archive: *const CkcLlvmArchive) -> usize;
    fn ckc_llvm_archive_data(archive: *const CkcLlvmArchive) -> *const u8;
    fn ckc_llvm_archive_member_count(archive: *const CkcLlvmArchive) -> usize;
    fn ckc_llvm_archive_has_symbol_index(archive: *const CkcLlvmArchive) -> u32;
    fn ckc_llvm_archive_dispose(archive: *mut CkcLlvmArchive);
    fn ckc_lld_link_shared(
        object_path: CkcLlvmBytes,
        output_path: CkcLlvmBytes,
        import_library_path: CkcLlvmBytes,
        exports: *const CkcLlvmBytes,
        export_count: usize,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_lld_link_executable(
        object_paths: *const CkcLlvmBytes,
        object_count: usize,
        output_path: CkcLlvmBytes,
        platform_input_path: CkcLlvmBytes,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_jit_create(out: *mut *mut CkcLlvmJit, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_jit_object_layer(jit: *const CkcLlvmJit) -> u32;
    fn ckc_llvm_jit_execute(
        jit: *mut CkcLlvmJit,
        program_object: CkcLlvmBytes,
        runtime_objects: *const CkcLlvmBytes,
        runtime_object_count: usize,
        exit_status: *mut i32,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_jit_memory_audit(
        jit: *const CkcLlvmJit,
        out: *mut CkcLlvmJitMemoryAudit,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_jit_dispose(jit: *mut CkcLlvmJit);
}

/// Static metadata reported by the linked LLVM bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBridgeInfo {
    /// Version of CalcKernel's private bridge ABI.
    pub abi_version: u32,
    /// Exact LLVM release linked into this compiler.
    pub llvm_version: String,
    /// Default host triple reported by the linked LLVM target support.
    pub host_triple: String,
}

pub fn bridge_info() -> Result<NativeBridgeInfo, NativeError> {
    let mut info = CkcLlvmBridgeInfo::empty();
    let mut error = CkcLlvmError::empty();
    // SAFETY: Both pointers refer to initialized, writable C-layout values for
    // the duration of the call. The bridge initializes only its owned fields.
    let status = unsafe { ckc_llvm_bridge_info(&mut info, &mut error) };
    if status != 0 {
        return Err(take_error(NativeStage::Bridge, status, &mut error));
    }

    // Take both buffers before attempting UTF-8 conversion so a malformed
    // first field cannot strand ownership of the second field.
    let llvm_version_bytes = take_vec(&mut info.llvm_version);
    let host_triple_bytes = take_vec(&mut info.host_triple);
    let llvm_version = parse_utf8(llvm_version_bytes)?;
    let host_triple = parse_utf8(host_triple_bytes)?;
    Ok(NativeBridgeInfo {
        abi_version: info.abi_version,
        llvm_version,
        host_triple,
    })
}

pub fn test_error() -> NativeError {
    let mut error = CkcLlvmError::empty();
    // SAFETY: `error` is initialized writable storage and the test hook does
    // not retain the pointer after returning.
    let status = unsafe { ckc_llvm_test_error(&mut error) };
    take_error(NativeStage::Bridge, status, &mut error)
}

pub fn test_invalid_input() -> NativeError {
    let mut error = CkcLlvmError::empty();
    // SAFETY: Passing a null output pointer deliberately exercises the
    // bridge's documented argument validation; `error` remains valid writable
    // storage and no handle can be returned.
    let status = unsafe { ckc_llvm_context_create(ptr::null_mut(), &mut error) };
    take_error(NativeStage::Context, status, &mut error)
}

pub(super) fn context_create() -> Result<NonNull<CkcLlvmContext>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: Both out-pointers reference initialized writable storage and the
    // bridge either leaves the handle null or transfers one owned handle.
    let status = unsafe { ckc_llvm_context_create(&mut handle, &mut error) };
    handle_result(NativeStage::Context, status, handle, &mut error)
}

pub(super) unsafe fn context_dispose(handle: NonNull<CkcLlvmContext>) {
    // SAFETY: The caller transfers the unique live context handle back to the
    // bridge exactly once from `Drop`.
    unsafe { ckc_llvm_context_dispose(handle.as_ptr()) };
}

pub(super) fn module_create_empty(
    context: NonNull<CkcLlvmContext>,
) -> Result<NonNull<CkcLlvmModule>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: The context is kept live by `NativeModule`'s Rust lifetime and
    // both out-pointers reference initialized writable storage.
    let status = unsafe { ckc_llvm_module_create_empty(context.as_ptr(), &mut handle, &mut error) };
    handle_result(NativeStage::Module, status, handle, &mut error)
}

pub(super) unsafe fn module_dispose(handle: NonNull<CkcLlvmModule>) {
    // SAFETY: The caller returns the unique live module handle exactly once.
    unsafe { ckc_llvm_module_dispose(handle.as_ptr()) };
}

pub(super) fn module_configure(
    module: NonNull<CkcLlvmModule>,
    target: NonNull<CkcLlvmTarget>,
    source_file_name: &str,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_module_configure(
            module.as_ptr(),
            target.as_ptr(),
            CkcLlvmBytes::new(source_file_name),
            error,
        )
    })
}

pub(super) fn module_verify(module: NonNull<CkcLlvmModule>) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_module_verify(module.as_ptr(), error)
    })
}

pub(super) fn module_print(module: NonNull<CkcLlvmModule>) -> Result<String, NativeError> {
    owned_string_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_module_print(module.as_ptr(), out, error)
    })
}

pub(super) fn target_create_host(
    cpu_policy: BridgeCpuPolicy,
) -> Result<NonNull<CkcLlvmTarget>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: Both out-pointers reference initialized writable storage and the
    // bridge either leaves the handle null or transfers one owned handle.
    let status = unsafe { ckc_llvm_target_create_host(cpu_policy as u32, &mut handle, &mut error) };
    handle_result(NativeStage::Target, status, handle, &mut error)
}

pub(super) unsafe fn target_dispose(handle: NonNull<CkcLlvmTarget>) {
    // SAFETY: The caller transfers the unique live target handle back to the
    // bridge exactly once from `Drop`.
    unsafe { ckc_llvm_target_dispose(handle.as_ptr()) };
}

pub(super) fn target_triple(target: NonNull<CkcLlvmTarget>) -> Result<String, NativeError> {
    owned_string_call(NativeStage::Target, |out, error| unsafe {
        ckc_llvm_target_triple(target.as_ptr(), out, error)
    })
}

pub(super) fn target_data_layout(target: NonNull<CkcLlvmTarget>) -> Result<String, NativeError> {
    owned_string_call(NativeStage::Target, |out, error| unsafe {
        ckc_llvm_target_data_layout(target.as_ptr(), out, error)
    })
}

pub(super) fn target_cpu(target: NonNull<CkcLlvmTarget>) -> Result<String, NativeError> {
    owned_string_call(NativeStage::Target, |out, error| unsafe {
        ckc_llvm_target_cpu(target.as_ptr(), out, error)
    })
}

pub(super) fn target_features(target: NonNull<CkcLlvmTarget>) -> Result<String, NativeError> {
    owned_string_call(NativeStage::Target, |out, error| unsafe {
        ckc_llvm_target_features(target.as_ptr(), out, error)
    })
}

pub(super) fn module_optimize(
    module: NonNull<CkcLlvmModule>,
    target: NonNull<CkcLlvmTarget>,
    opt_level: u8,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_module_optimize(
            module.as_ptr(),
            target.as_ptr(),
            u32::from(opt_level),
            error,
        )
    })
}

pub(super) fn module_make_invalid_for_test(
    module: NonNull<CkcLlvmModule>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_module_make_invalid_for_test(module.as_ptr(), error)
    })
}

pub(super) fn type_void(
    context: NonNull<CkcLlvmContext>,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_void(context.as_ptr(), out, error)
    })
}

pub(super) fn type_int(
    context: NonNull<CkcLlvmContext>,
    bits: u32,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_int(context.as_ptr(), bits, out, error)
    })
}

pub(super) fn type_f64(
    context: NonNull<CkcLlvmContext>,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_f64(context.as_ptr(), out, error)
    })
}

pub(super) fn type_ptr(
    context: NonNull<CkcLlvmContext>,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_ptr(context.as_ptr(), out, error)
    })
}

pub(super) fn type_slice(
    context: NonNull<CkcLlvmContext>,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_slice(context.as_ptr(), out, error)
    })
}

pub(super) fn type_array(
    element: NonNull<CkcLlvmType>,
    count: u32,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_array(element.as_ptr(), count, out, error)
    })
}

pub(super) fn type_struct(
    context: NonNull<CkcLlvmContext>,
    fields: &[NonNull<CkcLlvmType>],
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    let fields = fields
        .iter()
        .map(|field| field.as_ptr())
        .collect::<Vec<_>>();
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_struct(context.as_ptr(), fields.as_ptr(), fields.len(), out, error)
    })
}

pub(super) fn type_named_struct(
    context: NonNull<CkcLlvmContext>,
    name: &str,
) -> Result<NonNull<CkcLlvmType>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_type_named_struct(context.as_ptr(), CkcLlvmBytes::new(name), out, error)
    })
}

pub(super) fn type_set_struct_body(
    type_node: NonNull<CkcLlvmType>,
    fields: &[NonNull<CkcLlvmType>],
) -> Result<(), NativeError> {
    let fields = fields
        .iter()
        .map(|field| field.as_ptr())
        .collect::<Vec<_>>();
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_type_set_struct_body(type_node.as_ptr(), fields.as_ptr(), fields.len(), error)
    })
}

pub(super) fn module_add_function(
    module: NonNull<CkcLlvmModule>,
    name: &str,
    return_type: NonNull<CkcLlvmType>,
    params: &[NonNull<CkcLlvmType>],
    exported: bool,
) -> Result<NonNull<CkcLlvmFunction>, NativeError> {
    let params = params
        .iter()
        .map(|param| param.as_ptr())
        .collect::<Vec<_>>();
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_module_add_function(
            module.as_ptr(),
            CkcLlvmBytes::new(name),
            return_type.as_ptr(),
            params.as_ptr(),
            params.len(),
            u32::from(exported),
            out,
            error,
        )
    })
}

pub(super) fn module_preserve_function(
    module: NonNull<CkcLlvmModule>,
    function: NonNull<CkcLlvmFunction>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_module_preserve_function(module.as_ptr(), function.as_ptr(), error)
    })
}

pub(super) fn function_param(
    function: NonNull<CkcLlvmFunction>,
    index: usize,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_function_param(
            function.as_ptr(),
            index,
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn function_append_block(
    function: NonNull<CkcLlvmFunction>,
    name: &str,
) -> Result<NonNull<CkcLlvmBlock>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_function_append_block(function.as_ptr(), CkcLlvmBytes::new(name), out, error)
    })
}

pub(super) fn function_add_attribute(
    function: NonNull<CkcLlvmFunction>,
    kind: BridgeAttributeKind,
    return_attribute: bool,
    param_index: usize,
    pointee_type: Option<NonNull<CkcLlvmType>>,
    alignment: u32,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_function_add_attribute(
            function.as_ptr(),
            kind as u32,
            u32::from(return_attribute),
            param_index,
            pointee_type.map_or(ptr::null_mut(), NonNull::as_ptr),
            alignment,
            error,
        )
    })
}

pub(super) fn function_set_dll_export(
    function: NonNull<CkcLlvmFunction>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_function_set_dll_export(function.as_ptr(), error)
    })
}

pub(super) fn builder_create(
    context: NonNull<CkcLlvmContext>,
) -> Result<NonNull<CkcLlvmBuilder>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_create(context.as_ptr(), out, error)
    })
}

pub(super) unsafe fn builder_dispose(builder: NonNull<CkcLlvmBuilder>) {
    unsafe { ckc_llvm_builder_dispose(builder.as_ptr()) };
}

pub(super) fn builder_position(
    builder: NonNull<CkcLlvmBuilder>,
    block: NonNull<CkcLlvmBlock>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_builder_position(builder.as_ptr(), block.as_ptr(), error)
    })
}

pub(super) fn builder_alloca(
    builder: NonNull<CkcLlvmBuilder>,
    type_node: NonNull<CkcLlvmType>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_alloca(
            builder.as_ptr(),
            type_node.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_load(
    builder: NonNull<CkcLlvmBuilder>,
    type_node: NonNull<CkcLlvmType>,
    pointer: NonNull<CkcLlvmValue>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_load(
            builder.as_ptr(),
            type_node.as_ptr(),
            pointer.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_store(
    builder: NonNull<CkcLlvmBuilder>,
    value: NonNull<CkcLlvmValue>,
    pointer: NonNull<CkcLlvmValue>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_builder_store(builder.as_ptr(), value.as_ptr(), pointer.as_ptr(), error)
    })
}

pub(super) fn const_int(
    type_node: NonNull<CkcLlvmType>,
    text: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_const_int(type_node.as_ptr(), CkcLlvmBytes::new(text), out, error)
    })
}

pub(super) fn const_float(
    type_node: NonNull<CkcLlvmType>,
    text: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_const_float(type_node.as_ptr(), CkcLlvmBytes::new(text), out, error)
    })
}

pub(super) fn const_bool(
    context: NonNull<CkcLlvmContext>,
    value: bool,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_const_bool(context.as_ptr(), u32::from(value), out, error)
    })
}

pub(super) fn const_undef(
    type_node: NonNull<CkcLlvmType>,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_const_undef(type_node.as_ptr(), out, error)
    })
}

pub(super) fn builder_binary(
    builder: NonNull<CkcLlvmBuilder>,
    op: BridgeBinaryOp,
    left: NonNull<CkcLlvmValue>,
    right: NonNull<CkcLlvmValue>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_binary(
            builder.as_ptr(),
            op as u32,
            left.as_ptr(),
            right.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_overflow(
    builder: NonNull<CkcLlvmBuilder>,
    op: BridgeOverflowOp,
    left: NonNull<CkcLlvmValue>,
    right: NonNull<CkcLlvmValue>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_overflow(
            builder.as_ptr(),
            op as u32,
            left.as_ptr(),
            right.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_unary(
    builder: NonNull<CkcLlvmBuilder>,
    op: BridgeUnaryOp,
    value: NonNull<CkcLlvmValue>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_unary(
            builder.as_ptr(),
            op as u32,
            value.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_compare(
    builder: NonNull<CkcLlvmBuilder>,
    op: BridgeCompareOp,
    left: NonNull<CkcLlvmValue>,
    right: NonNull<CkcLlvmValue>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_compare(
            builder.as_ptr(),
            op as u32,
            left.as_ptr(),
            right.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_cast(
    builder: NonNull<CkcLlvmBuilder>,
    op: BridgeCastOp,
    value: NonNull<CkcLlvmValue>,
    target_type: NonNull<CkcLlvmType>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_cast(
            builder.as_ptr(),
            op as u32,
            value.as_ptr(),
            target_type.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_gep(
    builder: NonNull<CkcLlvmBuilder>,
    element_type: NonNull<CkcLlvmType>,
    pointer: NonNull<CkcLlvmValue>,
    indices: &[NonNull<CkcLlvmValue>],
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    let indices = indices
        .iter()
        .map(|index| index.as_ptr())
        .collect::<Vec<_>>();
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_gep(
            builder.as_ptr(),
            element_type.as_ptr(),
            pointer.as_ptr(),
            indices.as_ptr(),
            indices.len(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_extract_value(
    builder: NonNull<CkcLlvmBuilder>,
    aggregate: NonNull<CkcLlvmValue>,
    index: u32,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_extract_value(
            builder.as_ptr(),
            aggregate.as_ptr(),
            index,
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_insert_value(
    builder: NonNull<CkcLlvmBuilder>,
    aggregate: NonNull<CkcLlvmValue>,
    value: NonNull<CkcLlvmValue>,
    index: u32,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_insert_value(
            builder.as_ptr(),
            aggregate.as_ptr(),
            value.as_ptr(),
            index,
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_select(
    builder: NonNull<CkcLlvmBuilder>,
    condition: NonNull<CkcLlvmValue>,
    then_value: NonNull<CkcLlvmValue>,
    else_value: NonNull<CkcLlvmValue>,
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_select(
            builder.as_ptr(),
            condition.as_ptr(),
            then_value.as_ptr(),
            else_value.as_ptr(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_call(
    builder: NonNull<CkcLlvmBuilder>,
    function: NonNull<CkcLlvmFunction>,
    args: &[NonNull<CkcLlvmValue>],
    name: &str,
) -> Result<NonNull<CkcLlvmValue>, NativeError> {
    let args = args.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();
    handle_call(NativeStage::Module, |out, error| unsafe {
        ckc_llvm_builder_call(
            builder.as_ptr(),
            function.as_ptr(),
            args.as_ptr(),
            args.len(),
            CkcLlvmBytes::new(name),
            out,
            error,
        )
    })
}

pub(super) fn builder_return_void(builder: NonNull<CkcLlvmBuilder>) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_builder_return_void(builder.as_ptr(), error)
    })
}

pub(super) fn builder_return(
    builder: NonNull<CkcLlvmBuilder>,
    value: NonNull<CkcLlvmValue>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_builder_return(builder.as_ptr(), value.as_ptr(), error)
    })
}

pub(super) fn builder_branch(
    builder: NonNull<CkcLlvmBuilder>,
    target: NonNull<CkcLlvmBlock>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_builder_branch(builder.as_ptr(), target.as_ptr(), error)
    })
}

pub(super) fn builder_cond_branch(
    builder: NonNull<CkcLlvmBuilder>,
    condition: NonNull<CkcLlvmValue>,
    then_block: NonNull<CkcLlvmBlock>,
    else_block: NonNull<CkcLlvmBlock>,
) -> Result<(), NativeError> {
    status_call(NativeStage::Module, |error| unsafe {
        ckc_llvm_builder_cond_branch(
            builder.as_ptr(),
            condition.as_ptr(),
            then_block.as_ptr(),
            else_block.as_ptr(),
            error,
        )
    })
}

pub(super) fn target_emit_object(
    target: NonNull<CkcLlvmTarget>,
    module: NonNull<CkcLlvmModule>,
) -> Result<NonNull<CkcLlvmObject>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: Both owners remain live and uniquely prevent concurrent bridge
    // mutation while the bridge writes one optional object handle.
    let status = unsafe {
        ckc_llvm_target_emit_object(target.as_ptr(), module.as_ptr(), &mut handle, &mut error)
    };
    handle_result(NativeStage::Object, status, handle, &mut error)
}

pub(super) fn target_parse_object(
    target: NonNull<CkcLlvmTarget>,
    bytes: &[u8],
) -> Result<NonNull<CkcLlvmObject>, NativeError> {
    handle_call(NativeStage::Object, |out, error| unsafe {
        ckc_llvm_target_parse_object(target.as_ptr(), CkcLlvmBytes::from_bytes(bytes), out, error)
    })
}

pub(super) fn object_size(handle: NonNull<CkcLlvmObject>) -> usize {
    // SAFETY: The object owner keeps the immutable handle live for this query.
    unsafe { ckc_llvm_object_size(handle.as_ptr()) }
}

pub(super) fn object_data(handle: NonNull<CkcLlvmObject>) -> *const u8 {
    // SAFETY: The object owner keeps the immutable byte storage live.
    unsafe { ckc_llvm_object_data(handle.as_ptr()) }
}

pub(super) unsafe fn object_dispose(handle: NonNull<CkcLlvmObject>) {
    // SAFETY: The caller returns the unique live object handle exactly once.
    unsafe { ckc_llvm_object_dispose(handle.as_ptr()) };
}

#[derive(Debug, Clone, Copy)]
pub(in crate::backend) enum BridgeArchiveKind {
    Gnu = 1,
    Darwin = 2,
    Coff = 3,
}

pub(in crate::backend) fn archive_create(
    object: NonNull<CkcLlvmObject>,
    kind: BridgeArchiveKind,
) -> Result<NonNull<CkcLlvmArchive>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: The object remains live, the kind is allowlisted, and both
    // out-pointers refer to initialized writable storage.
    let status =
        unsafe { ckc_llvm_archive_create(object.as_ptr(), kind as u32, &mut handle, &mut error) };
    handle_result(NativeStage::Archive, status, handle, &mut error)
}

pub(in crate::backend) fn archive_size(handle: NonNull<CkcLlvmArchive>) -> usize {
    // SAFETY: The archive owner keeps the immutable handle live.
    unsafe { ckc_llvm_archive_size(handle.as_ptr()) }
}

pub(in crate::backend) fn archive_data(handle: NonNull<CkcLlvmArchive>) -> *const u8 {
    // SAFETY: The archive owner keeps its immutable byte storage live.
    unsafe { ckc_llvm_archive_data(handle.as_ptr()) }
}

pub(in crate::backend) fn archive_member_count(handle: NonNull<CkcLlvmArchive>) -> usize {
    // SAFETY: The archive owner keeps the immutable handle live.
    unsafe { ckc_llvm_archive_member_count(handle.as_ptr()) }
}

pub(in crate::backend) fn archive_has_symbol_index(handle: NonNull<CkcLlvmArchive>) -> bool {
    // SAFETY: The archive owner keeps the immutable handle live.
    unsafe { ckc_llvm_archive_has_symbol_index(handle.as_ptr()) != 0 }
}

pub(in crate::backend) unsafe fn archive_dispose(handle: NonNull<CkcLlvmArchive>) {
    // SAFETY: The caller returns the unique live archive handle exactly once.
    unsafe { ckc_llvm_archive_dispose(handle.as_ptr()) };
}

pub(in crate::backend) fn lld_link_shared(
    object_path: &str,
    output_path: &str,
    import_library_path: &str,
    exports: &[String],
) -> Result<(), NativeError> {
    let export_bytes = exports
        .iter()
        .map(|name| CkcLlvmBytes::new(name))
        .collect::<Vec<_>>();
    status_call(NativeStage::Link, |error| unsafe {
        ckc_lld_link_shared(
            CkcLlvmBytes::new(object_path),
            CkcLlvmBytes::new(output_path),
            CkcLlvmBytes::new(import_library_path),
            export_bytes.as_ptr(),
            export_bytes.len(),
            error,
        )
    })
}

pub(in crate::backend) fn lld_link_executable(
    object_paths: &[String],
    output_path: &str,
    platform_input_path: &str,
) -> Result<(), NativeError> {
    let object_path_bytes = object_paths
        .iter()
        .map(|path| CkcLlvmBytes::new(path))
        .collect::<Vec<_>>();
    status_call(NativeStage::Link, |error| unsafe {
        ckc_lld_link_executable(
            object_path_bytes.as_ptr(),
            object_path_bytes.len(),
            CkcLlvmBytes::new(output_path),
            CkcLlvmBytes::new(platform_input_path),
            error,
        )
    })
}

pub(super) fn jit_create() -> Result<NonNull<CkcLlvmJit>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: Both out-pointers reference initialized writable storage and the
    // bridge either leaves the handle null or transfers one owned handle.
    let status = unsafe { ckc_llvm_jit_create(&mut handle, &mut error) };
    handle_result(NativeStage::Orc, status, handle, &mut error)
}

pub(super) fn jit_object_layer(handle: NonNull<CkcLlvmJit>) -> u32 {
    // SAFETY: The unique wrapper keeps the JIT handle live for this shared
    // query and the bridge does not retain the pointer.
    unsafe { ckc_llvm_jit_object_layer(handle.as_ptr()) }
}

pub(super) fn jit_execute(
    handle: NonNull<CkcLlvmJit>,
    program_object: &[u8],
    runtime_objects: &[&[u8]],
) -> Result<i32, NativeError> {
    let runtime_bytes = runtime_objects
        .iter()
        .map(|bytes| CkcLlvmBytes::from_bytes(bytes))
        .collect::<Vec<_>>();
    let mut exit_status = 0;
    status_call(NativeStage::Orc, |error| unsafe {
        ckc_llvm_jit_execute(
            handle.as_ptr(),
            CkcLlvmBytes::from_bytes(program_object),
            runtime_bytes.as_ptr(),
            runtime_bytes.len(),
            &mut exit_status,
            error,
        )
    })?;
    Ok(exit_status)
}

pub(super) fn jit_memory_audit(
    handle: NonNull<CkcLlvmJit>,
) -> Result<super::jit::NativeJitMemoryAudit, NativeError> {
    let mut audit = CkcLlvmJitMemoryAudit::default();
    status_call(NativeStage::Orc, |error| unsafe {
        ckc_llvm_jit_memory_audit(handle.as_ptr(), &mut audit, error)
    })?;
    Ok(super::jit::NativeJitMemoryAudit {
        allocations: audit.allocations,
        instruction_cache_finalizations: audit.instruction_cache_finalizations,
        relocation_write_non_execute: audit.relocation_write_non_execute != 0,
        final_code_read_execute: audit.final_code_read_execute != 0,
        final_data_non_execute: audit.final_data_non_execute != 0,
        darwin_map_jit: audit.darwin_map_jit != 0,
        darwin_thread_write_protection_supported: audit.darwin_thread_write_protection_supported
            != 0,
        darwin_thread_write_protection: audit.darwin_thread_write_protection != 0,
    })
}

pub(super) unsafe fn jit_dispose(handle: NonNull<CkcLlvmJit>) {
    // SAFETY: The caller transfers the unique live JIT handle back to the
    // bridge exactly once from `Drop`.
    unsafe { ckc_llvm_jit_dispose(handle.as_ptr()) };
}

fn handle_result<T>(
    stage: NativeStage,
    status: i32,
    handle: *mut T,
    error: &mut CkcLlvmError,
) -> Result<NonNull<T>, NativeError> {
    if status != 0 {
        return Err(take_error(stage, status, error));
    }
    NonNull::new(handle)
        .ok_or_else(|| NativeError::new(stage, 3, "bridge returned a null handle".to_string()))
}

fn handle_call<T>(
    stage: NativeStage,
    call: impl FnOnce(*mut *mut T, *mut CkcLlvmError) -> i32,
) -> Result<NonNull<T>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    let status = call(&mut handle, &mut error);
    handle_result(stage, status, handle, &mut error)
}

fn status_call(
    stage: NativeStage,
    call: impl FnOnce(*mut CkcLlvmError) -> i32,
) -> Result<(), NativeError> {
    let mut error = CkcLlvmError::empty();
    let status = call(&mut error);
    if status == 0 {
        Ok(())
    } else {
        Err(take_error(stage, status, &mut error))
    }
}

fn owned_string_call(
    stage: NativeStage,
    call: impl FnOnce(*mut CkcLlvmOwnedBytes, *mut CkcLlvmError) -> i32,
) -> Result<String, NativeError> {
    let mut bytes = CkcLlvmOwnedBytes::empty();
    let mut error = CkcLlvmError::empty();
    let status = call(&mut bytes, &mut error);
    if status != 0 {
        let _ = take_vec(&mut bytes);
        return Err(take_error(stage, status, &mut error));
    }
    parse_utf8(take_vec(&mut bytes))
}

fn take_error(stage: NativeStage, status: i32, error: &mut CkcLlvmError) -> NativeError {
    let message = take_utf8_lossy(&mut error.message);
    let code = if error.code == 0 { status } else { error.code };
    NativeError::new(stage, code, message)
}

fn parse_utf8(raw: Vec<u8>) -> Result<String, NativeError> {
    String::from_utf8(raw)
        .map_err(|error| NativeError::new(NativeStage::Bridge, 3, error.to_string()))
}

fn take_utf8_lossy(bytes: &mut CkcLlvmOwnedBytes) -> String {
    String::from_utf8_lossy(&take_vec(bytes)).into_owned()
}

fn take_vec(bytes: &mut CkcLlvmOwnedBytes) -> Vec<u8> {
    let value = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        // SAFETY: A successful bridge call owns exactly `len` initialized bytes
        // at `data` until the paired dispose call below.
        unsafe { slice::from_raw_parts(bytes.data, bytes.len) }.to_vec()
    };
    // SAFETY: The bridge allocated this buffer and accepts its own C-layout
    // descriptor exactly once. It clears the descriptor after freeing it.
    unsafe { ckc_llvm_owned_bytes_dispose(bytes) };
    value
}
