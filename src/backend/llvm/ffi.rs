use std::{ptr, ptr::NonNull, slice};

use super::error::{NativeError, NativeStage};

pub const LLVM_BRIDGE_ABI_VERSION: u32 = 1;

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
pub(super) struct CkcLlvmContext {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmModule {
    _private: [u8; 0],
}

#[repr(C)]
pub(super) struct CkcLlvmObject {
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
    fn ckc_llvm_target_create_host(out: *mut *mut CkcLlvmTarget, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_target_dispose(target: *mut CkcLlvmTarget);
    fn ckc_llvm_target_emit_object(
        target: *mut CkcLlvmTarget,
        module: *mut CkcLlvmModule,
        out: *mut *mut CkcLlvmObject,
        error: *mut CkcLlvmError,
    ) -> i32;
    fn ckc_llvm_object_size(object: *const CkcLlvmObject) -> usize;
    fn ckc_llvm_object_dispose(object: *mut CkcLlvmObject);
    fn ckc_llvm_jit_create(out: *mut *mut CkcLlvmJit, error: *mut CkcLlvmError) -> i32;
    fn ckc_llvm_jit_object_layer(jit: *const CkcLlvmJit) -> u32;
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

pub(super) fn target_create_host() -> Result<NonNull<CkcLlvmTarget>, NativeError> {
    let mut handle = ptr::null_mut();
    let mut error = CkcLlvmError::empty();
    // SAFETY: Both out-pointers reference initialized writable storage and the
    // bridge either leaves the handle null or transfers one owned handle.
    let status = unsafe { ckc_llvm_target_create_host(&mut handle, &mut error) };
    handle_result(NativeStage::Target, status, handle, &mut error)
}

pub(super) unsafe fn target_dispose(handle: NonNull<CkcLlvmTarget>) {
    // SAFETY: The caller transfers the unique live target handle back to the
    // bridge exactly once from `Drop`.
    unsafe { ckc_llvm_target_dispose(handle.as_ptr()) };
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

pub(super) fn object_size(handle: NonNull<CkcLlvmObject>) -> usize {
    // SAFETY: The object owner keeps the immutable handle live for this query.
    unsafe { ckc_llvm_object_size(handle.as_ptr()) }
}

pub(super) unsafe fn object_dispose(handle: NonNull<CkcLlvmObject>) {
    // SAFETY: The caller returns the unique live object handle exactly once.
    unsafe { ckc_llvm_object_dispose(handle.as_ptr()) };
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
