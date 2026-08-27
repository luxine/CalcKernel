use std::collections::HashMap;

use crate::{MirEntryResult, MirModule};

use super::{
    builder::{NativeBuilder, NativeFunction},
    context::NativeContext,
    error::{NativeError, NativeStage},
    ffi::{BridgeBinaryOp, BridgeCompareOp},
    lower::TypeRegistry,
    module::NativeModule,
};

const ENTRY_ERROR: i32 = 3;

pub(super) fn add_entry_wrapper<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    mir: &MirModule,
    types: &TypeRegistry<'context>,
    implementations: &HashMap<String, NativeFunction<'module>>,
    checked: bool,
) -> Result<(), NativeError> {
    let entry = mir.entry.as_ref().ok_or_else(|| {
        entry_error("standalone native executable requires fn main() -> void or i32")
    })?;
    let implementation = implementations
        .get(&entry.function_name)
        .copied()
        .ok_or_else(|| entry_error("MIR entry implementation is missing"))?;
    let main = module.add_function("main", types.i32, &[], true)?;
    let main_block = main.append_block("entry")?;
    let mut builder = NativeBuilder::new(context, module)?;
    builder.position(main_block)?;

    if !checked {
        let result = builder.call(
            implementation,
            &[],
            if entry.result == MirEntryResult::Void {
                ""
            } else {
                "ck.exit"
            },
        )?;
        return if entry.result == MirEntryResult::Void {
            let zero = builder.const_int(types.i32, "0")?;
            builder.return_value(zero)
        } else {
            builder.return_value(result)
        };
    }

    let result_pointer = if entry.result == MirEntryResult::I32 {
        Some(builder.alloca(types.i32, "ck.entry.result")?)
    } else {
        None
    };
    let arguments = result_pointer.iter().copied().collect::<Vec<_>>();
    let status = builder.call(implementation, &arguments, "ck.status")?;
    let zero = builder.const_int(types.i32, "0")?;
    let failed = builder.compare(BridgeCompareOp::IcmpNe, status, zero, "ck.status.failed")?;
    let failure_block = main.append_block("runtime.failure")?;
    let success_block = main.append_block("entry.success")?;
    builder.cond_branch(failed, failure_block, success_block)?;

    builder.position(failure_block)?;
    let runtime_fail = module.add_function("__ck_runtime_fail", types.void, &[types.i32], true)?;
    let _ = builder.call(runtime_fail, &[status], "")?;
    let process_base = builder.const_int(types.i32, "239")?;
    let fallback = builder.binary(BridgeBinaryOp::Add, process_base, status, "ck.failure.exit")?;
    builder.return_value(fallback)?;

    builder.position(success_block)?;
    if let Some(pointer) = result_pointer {
        let result = builder.load(types.i32, pointer, "ck.exit")?;
        builder.return_value(result)
    } else {
        let zero = builder.const_int(types.i32, "0")?;
        builder.return_value(zero)
    }
}

fn entry_error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Module, ENTRY_ERROR, message.into())
}
