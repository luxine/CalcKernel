use std::collections::{HashMap, HashSet};

use crate::{
    BoundsMode, MirBinaryOp, MirCastOp, MirCompareOp, MirFunction, MirInstruction, MirModule,
    MirParam, MirPlace, MirPrimitiveTypeName, MirRuntimeIntrinsic, MirTerminator, MirType,
    MirUnaryOp, MirValue, OverflowMode,
};

use super::{
    EmitLlvmOptions,
    abi::{add_export_thunks, implementation_name},
    builder::{NativeBlock, NativeBuilder, NativeFunction, NativeType, NativeValue},
    context::NativeContext,
    error::{NativeError, NativeStage},
    ffi::{BridgeBinaryOp, BridgeCastOp, BridgeCompareOp, BridgeOverflowOp, BridgeUnaryOp},
    layout::LlvmStructLayout,
    module::NativeModule,
    names::{llvm_block_label, llvm_source_file_name, llvm_storage_name_for_temp},
    target::NativeTarget,
};

#[path = "checked.rs"]
mod checked;

const LOWERING_ERROR: i32 = 3;

/// Semantic modes and inspection metadata for native LLVM lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLoweringOptions {
    pub emit: EmitLlvmOptions,
    pub overflow_mode: OverflowMode,
    pub bounds_mode: BoundsMode,
}

impl Default for NativeLoweringOptions {
    fn default() -> Self {
        Self {
            emit: EmitLlvmOptions::default(),
            overflow_mode: OverflowMode::Unchecked,
            bounds_mode: BoundsMode::Unchecked,
        }
    }
}

/// Lowers validated CK MIR into a structural LLVM module for `target`.
///
/// This path constructs LLVM objects directly and never parses textual IR.
pub fn lower_native_llvm_module<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    mir: &MirModule,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    lower_native_llvm_module_with_options(
        context,
        target,
        mir,
        &NativeLoweringOptions {
            emit: options.clone(),
            overflow_mode: OverflowMode::Unchecked,
            bounds_mode: BoundsMode::Unchecked,
        },
    )
}

/// Lowers MIR using explicit overflow and slice-bounds modes.
pub fn lower_native_llvm_module_with_options<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    mir: &MirModule,
    options: &NativeLoweringOptions,
) -> Result<NativeModule<'context>, NativeError> {
    if let Some(requested) = options.emit.target_triple.as_deref() {
        let actual = target.triple()?;
        if requested != actual {
            return Err(lowering_error(format!(
                "requested target triple '{requested}' does not match native target '{actual}'"
            )));
        }
    }

    let mut module = NativeModule::empty(context)?;
    module.configure(
        target,
        &llvm_source_file_name(options.emit.source_file_name.as_deref()),
    )?;

    {
        let types = TypeRegistry::new(context, mir)?;
        let mut functions = HashMap::new();
        let status_abi = status_abi(options);
        for function in &mir.functions {
            let mut params = physical_param_types(&types, &function.params)?;
            if status_abi && !matches!(function.return_type, MirType::Void) {
                params.push(types.pointer);
            }
            let handle = module.add_function(
                &implementation_name(function),
                if status_abi {
                    types.i32
                } else {
                    types.get(&function.return_type)?
                },
                &params,
                false,
            )?;
            functions.insert(function.name.clone(), handle);
        }
        if let Some(entry) = &mir.entry {
            module.preserve_function(require_function(&functions, &entry.function_name)?)?;
        }
        for intrinsic in used_runtime_intrinsics(mir) {
            let (name, parameter) = runtime_signature(intrinsic);
            let params = parameter
                .as_ref()
                .map(|type_node| types.get(type_node))
                .transpose()?
                .into_iter()
                .collect::<Vec<_>>();
            functions.insert(
                name.to_string(),
                module.add_function(name, types.void, &params, true)?,
            );
        }

        let layout = LlvmStructLayout::new(mir);
        for function in &mir.functions {
            lower_function(
                context, &module, &types, &functions, &layout, function, options,
            )?;
        }
        add_export_thunks(
            context, &module, target, mir, &types, &functions, status_abi,
        )?;
    }
    Ok(module)
}

fn status_abi(options: &NativeLoweringOptions) -> bool {
    options.overflow_mode == OverflowMode::Checked || options.bounds_mode == BoundsMode::Checked
}

pub(super) struct TypeRegistry<'context> {
    pub(super) void: NativeType<'context>,
    i1: NativeType<'context>,
    pub(super) i32: NativeType<'context>,
    i64: NativeType<'context>,
    f64: NativeType<'context>,
    pub(super) pointer: NativeType<'context>,
    slice: NativeType<'context>,
    structs: HashMap<String, NativeType<'context>>,
}

impl<'context> TypeRegistry<'context> {
    fn new(context: &'context NativeContext, module: &MirModule) -> Result<Self, NativeError> {
        let mut registry = Self {
            void: NativeType::void(context)?,
            i1: NativeType::int(context, 1)?,
            i32: NativeType::int(context, 32)?,
            i64: NativeType::int(context, 64)?,
            f64: NativeType::f64(context)?,
            pointer: NativeType::pointer(context)?,
            slice: NativeType::slice(context)?,
            structs: HashMap::new(),
        };
        for structure in &module.structs {
            registry.structs.insert(
                structure.name.clone(),
                NativeType::named_struct(context, &format!("struct.{}", structure.name))?,
            );
        }
        for structure in &module.structs {
            let fields = structure
                .fields
                .iter()
                .map(|field| registry.get(&field.type_node))
                .collect::<Result<Vec<_>, _>>()?;
            registry
                .structs
                .get(&structure.name)
                .copied()
                .ok_or_else(|| lowering_error("missing native struct declaration"))?
                .set_struct_body(&fields)?;
        }
        Ok(registry)
    }

    pub(super) fn get(&self, type_node: &MirType) -> Result<NativeType<'context>, NativeError> {
        match type_node {
            MirType::Void => Ok(self.void),
            MirType::Primitive(MirPrimitiveTypeName::Bool) => Ok(self.i1),
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
                Ok(self.i32)
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
                Ok(self.i64)
            }
            MirType::Primitive(MirPrimitiveTypeName::F64) => Ok(self.f64),
            MirType::Pointer(_) => Ok(self.pointer),
            MirType::Slice(_) => Ok(self.slice),
            MirType::Struct(name) => self
                .structs
                .get(name)
                .copied()
                .ok_or_else(|| lowering_error(format!("unknown MIR struct type '{name}'"))),
        }
    }
}

fn physical_param_types<'context>(
    types: &TypeRegistry<'context>,
    params: &[MirParam],
) -> Result<Vec<NativeType<'context>>, NativeError> {
    let mut result = Vec::new();
    for param in params {
        if matches!(param.type_node, MirType::Slice(_)) {
            result.extend([types.pointer, types.i32]);
        } else {
            result.push(types.get(&param.type_node)?);
        }
    }
    Ok(result)
}

fn used_runtime_intrinsics(module: &MirModule) -> Vec<MirRuntimeIntrinsic> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let MirInstruction::RuntimeCall { intrinsic, .. } = instruction
                    && seen.insert(*intrinsic)
                {
                    result.push(*intrinsic);
                }
            }
        }
    }
    result
}

fn runtime_signature(intrinsic: MirRuntimeIntrinsic) -> (&'static str, Option<MirType>) {
    let primitive = |name| Some(MirType::Primitive(name));
    match intrinsic {
        MirRuntimeIntrinsic::PrintI32 => ("__ck_print_i32", primitive(MirPrimitiveTypeName::I32)),
        MirRuntimeIntrinsic::PrintI64 => ("__ck_print_i64", primitive(MirPrimitiveTypeName::I64)),
        MirRuntimeIntrinsic::PrintU32 => ("__ck_print_u32", primitive(MirPrimitiveTypeName::U32)),
        MirRuntimeIntrinsic::PrintU64 => ("__ck_print_u64", primitive(MirPrimitiveTypeName::U64)),
        MirRuntimeIntrinsic::PrintF64 => ("__ck_print_f64", primitive(MirPrimitiveTypeName::F64)),
        MirRuntimeIntrinsic::PrintBool => {
            ("__ck_print_bool", primitive(MirPrimitiveTypeName::Bool))
        }
        MirRuntimeIntrinsic::PrintNewline => ("__ck_print_newline", None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum StorageKey {
    Param(String),
    Local(String),
    Temp(String),
}

#[derive(Clone, Copy)]
struct Storage<'module> {
    pointer: NativeValue<'module>,
}

struct FunctionLowerer<'module, 'context, 'mir> {
    builder: NativeBuilder<'module, 'context>,
    types: &'mir TypeRegistry<'context>,
    functions: &'mir HashMap<String, NativeFunction<'module>>,
    layout: &'mir LlvmStructLayout,
    handle: NativeFunction<'module>,
    options: &'mir NativeLoweringOptions,
    result_pointer: Option<NativeValue<'module>>,
    blocks: HashMap<String, NativeBlock<'module>>,
    storage: HashMap<StorageKey, Storage<'module>>,
    temporary_name: usize,
}

fn lower_function<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    types: &TypeRegistry<'context>,
    functions: &HashMap<String, NativeFunction<'module>>,
    layout: &LlvmStructLayout,
    function: &MirFunction,
    options: &NativeLoweringOptions,
) -> Result<(), NativeError> {
    let handle = require_function(functions, &function.name)?;
    let mut blocks = HashMap::new();
    if function.blocks.is_empty() {
        blocks.insert("entry".to_string(), handle.append_block("entry")?);
    } else {
        for block in &function.blocks {
            blocks.insert(
                block.label.clone(),
                handle.append_block(&llvm_block_label(function, &block.label))?,
            );
        }
    }

    let mut lowerer = FunctionLowerer {
        builder: NativeBuilder::new(context, module)?,
        types,
        functions,
        layout,
        handle,
        options,
        result_pointer: None,
        blocks,
        storage: HashMap::new(),
        temporary_name: 0,
    };
    let entry = if function.blocks.is_empty() {
        lowerer.block("entry")?
    } else {
        lowerer.block(&function.blocks[0].label)?
    };
    lowerer.builder.position(entry)?;
    lowerer.allocate_storage(function)?;
    lowerer.store_parameters(function, handle)?;

    if function.blocks.is_empty() {
        if matches!(function.return_type, MirType::Void) {
            return if lowerer.status_abi() {
                let ok = lowerer.status(0)?;
                lowerer.builder.return_value(ok)
            } else {
                lowerer.builder.return_void()
            };
        }
        return Err(lowering_error(format!(
            "non-void function '{}' has no MIR blocks",
            function.name
        )));
    }
    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            lowerer.builder.position(lowerer.block(&block.label)?)?;
        }
        for instruction in &block.instructions {
            lowerer.instruction(instruction)?;
        }
        lowerer.terminator(&block.terminator)?;
    }
    Ok(())
}

impl<'module, 'context> FunctionLowerer<'module, 'context, '_> {
    fn next_name(&mut self, prefix: &str) -> String {
        let name = format!("{prefix}{}", self.temporary_name);
        self.temporary_name += 1;
        name
    }

    fn block(&self, label: &str) -> Result<NativeBlock<'module>, NativeError> {
        self.blocks
            .get(label)
            .copied()
            .ok_or_else(|| lowering_error(format!("unknown MIR block '{label}'")))
    }

    fn allocate_storage(&mut self, function: &MirFunction) -> Result<(), NativeError> {
        for param in &function.params {
            self.allocate(
                StorageKey::Param(param.name.clone()),
                &param.name,
                &param.type_node,
            )?;
        }
        let mut local_names = HashSet::new();
        for local in &function.locals {
            if local_names.insert(local.name.clone()) {
                self.allocate(
                    StorageKey::Local(local.name.clone()),
                    &local.name,
                    &local.type_node,
                )?;
            }
        }
        for (name, type_node) in collect_temps(function) {
            self.allocate(
                StorageKey::Temp(name.clone()),
                &llvm_storage_name_for_temp(&name),
                &type_node,
            )?;
        }
        Ok(())
    }

    fn allocate(
        &mut self,
        key: StorageKey,
        name: &str,
        type_node: &MirType,
    ) -> Result<(), NativeError> {
        if matches!(type_node, MirType::Void) {
            return Err(lowering_error("void value cannot have storage"));
        }
        let pointer = self
            .builder
            .alloca(self.types.get(type_node)?, &format!("{name}.addr"))?;
        self.storage.insert(key, Storage { pointer });
        Ok(())
    }

    fn store_parameters(
        &mut self,
        function: &MirFunction,
        handle: NativeFunction<'module>,
    ) -> Result<(), NativeError> {
        let mut physical_index = 0;
        for param in &function.params {
            let storage = self.require_storage(&StorageKey::Param(param.name.clone()))?;
            if matches!(param.type_node, MirType::Slice(_)) {
                let data = handle.param(physical_index, &format!("{}.data", param.name))?;
                let len = handle.param(physical_index + 1, &format!("{}.len", param.name))?;
                physical_index += 2;
                let value = self.make_slice(data, len)?;
                self.builder.store(value, storage.pointer)?;
            } else {
                let value = handle.param(physical_index, &param.name)?;
                physical_index += 1;
                self.builder.store(value, storage.pointer)?;
            }
        }
        if self.status_abi() && !matches!(function.return_type, MirType::Void) {
            let result = handle.param(physical_index, "ck_return")?;
            self.result_pointer = Some(result);
            let null = self.builder.const_int(self.types.i64, "0")?;
            // Opaque pointers cannot be compared with an integer. The bridge
            // supplies a null pointer constant through an integer-to-pointer
            // cast so this check remains target-width independent.
            let name = self.next_name("result.null");
            let null =
                self.builder
                    .cast(BridgeCastOp::IntToPtr, null, self.types.pointer, &name)?;
            let name = self.next_name("result.is_null");
            let is_null = self
                .builder
                .compare(BridgeCompareOp::IcmpEq, result, null, &name)?;
            let status = self.status(3)?;
            self.guard_with_status(is_null, status)?;
        }
        Ok(())
    }

    fn instruction(&mut self, instruction: &MirInstruction) -> Result<(), NativeError> {
        match instruction {
            MirInstruction::ConstInt { target, value } => {
                let constant = self
                    .builder
                    .const_int(self.types.get(value_type(target))?, value)?;
                self.store_value(target, constant)
            }
            MirInstruction::ConstFloat { target, value } => {
                let constant = self
                    .builder
                    .const_float(self.types.get(value_type(target))?, value)?;
                self.store_value(target, constant)
            }
            MirInstruction::ConstBool { target, value } => {
                let constant = self.builder.const_bool(*value)?;
                self.store_value(target, constant)
            }
            MirInstruction::Move { target, value } => {
                let value = self.load_value(value)?;
                self.store_value(target, value)
            }
            MirInstruction::Binary {
                target,
                op,
                left,
                right,
            } => {
                let left = self.load_value(left)?;
                let right = self.load_value(right)?;
                let result = if self.options.overflow_mode == OverflowMode::Checked {
                    self.checked_binary(*op, value_type(target), left, right)?
                } else {
                    let name = self.next_name("binary");
                    self.builder
                        .binary(binary_op(*op, value_type(target))?, left, right, &name)?
                };
                self.store_value(target, result)
            }
            MirInstruction::Unary {
                target,
                op,
                operand,
            } => {
                let operand = self.load_value(operand)?;
                let result = if self.options.overflow_mode == OverflowMode::Checked {
                    self.checked_unary(*op, value_type(target), operand)?
                } else {
                    let name = self.next_name("unary");
                    self.builder
                        .unary(unary_op(*op, value_type(target)), operand, &name)?
                };
                self.store_value(target, result)
            }
            MirInstruction::Compare {
                target,
                op,
                left,
                right,
            } => {
                let left_value = self.load_value(left)?;
                let right_value = self.load_value(right)?;
                let name = self.next_name("compare");
                let result = self.builder.compare(
                    compare_op(*op, value_type(left)),
                    left_value,
                    right_value,
                    &name,
                )?;
                self.store_value(target, result)
            }
            MirInstruction::Cast { target, op, value } => {
                let value = self.load_value(value)?;
                let op = match op {
                    MirCastOp::I32ToF64 => BridgeCastOp::Sitofp,
                    MirCastOp::U32ToF64 => BridgeCastOp::Uitofp,
                };
                let name = self.next_name("cast");
                let result =
                    self.builder
                        .cast(op, value, self.types.get(value_type(target))?, &name)?;
                self.store_value(target, result)
            }
            MirInstruction::Address { target, place } => {
                let pointer = self.place_pointer(place)?;
                self.store_value(target, pointer)
            }
            MirInstruction::Load { target, place } => {
                let pointer = self.place_pointer(place)?;
                let name = self.next_name("place.load");
                let value =
                    self.builder
                        .load(self.types.get(value_type(target))?, pointer, &name)?;
                self.store_value(target, value)
            }
            MirInstruction::Store { place, value } => {
                let pointer = self.place_pointer(place)?;
                let value = self.load_value(value)?;
                self.builder.store(value, pointer)
            }
            MirInstruction::MakeSlice { target, data, len } => {
                let data = self.load_value(data)?;
                let len = self.load_value(len)?;
                let value = self.make_slice(data, len)?;
                self.store_value(target, value)
            }
            MirInstruction::SliceData { target, slice } => {
                let slice = self.load_value(slice)?;
                let name = self.next_name("slice.data");
                let data = self.builder.extract_value(slice, 0, &name)?;
                self.store_value(target, data)
            }
            MirInstruction::SliceLen { target, slice } => {
                let slice = self.load_value(slice)?;
                let name = self.next_name("slice.len");
                let len = self.builder.extract_value(slice, 1, &name)?;
                self.store_value(target, len)
            }
            MirInstruction::Subslice {
                target,
                slice,
                start,
                end,
            } => {
                let MirType::Slice(element_type) = value_type(slice) else {
                    return Err(lowering_error("subslice source is not a slice"));
                };
                let descriptor = self.load_value(slice)?;
                let name = self.next_name("subslice.data");
                let data = self.builder.extract_value(descriptor, 0, &name)?;
                let name = self.next_name("subslice.source_len");
                let source_len = self.builder.extract_value(descriptor, 1, &name)?;
                let start_value = self.load_value(start)?;
                let end_value = self.load_value(end)?;
                if self.options.bounds_mode == BoundsMode::Checked {
                    let name = self.next_name("subslice.start_after_end");
                    let invalid_order = self.builder.compare(
                        BridgeCompareOp::IcmpUgt,
                        start_value,
                        end_value,
                        &name,
                    )?;
                    let status = self.status(4)?;
                    self.guard_with_status(invalid_order, status)?;
                    let name = self.next_name("subslice.end_after_len");
                    let invalid_end = self.builder.compare(
                        BridgeCompareOp::IcmpUgt,
                        end_value,
                        source_len,
                        &name,
                    )?;
                    let status = self.status(4)?;
                    self.guard_with_status(invalid_end, status)?;
                }
                let start64 = self.index_to_i64(start_value, value_type(start))?;
                let name = self.next_name("subslice.gep");
                let advanced =
                    self.builder
                        .gep(self.types.get(element_type)?, data, &[start64], &name)?;
                let zero = self.builder.const_int(self.types.i32, "0")?;
                let name = self.next_name("subslice.zero");
                let is_zero =
                    self.builder
                        .compare(BridgeCompareOp::IcmpEq, start_value, zero, &name)?;
                let name = self.next_name("subslice.selected");
                let selected = self.builder.select(is_zero, data, advanced, &name)?;
                let name = self.next_name("subslice.len");
                let len =
                    self.builder
                        .binary(BridgeBinaryOp::Sub, end_value, start_value, &name)?;
                let value = self.make_slice(selected, len)?;
                self.store_value(target, value)
            }
            MirInstruction::Call {
                target,
                function_name,
                args,
            } => {
                let function = require_function(self.functions, function_name)?;
                let mut args = self.physical_args(args)?;
                if self.status_abi() {
                    if let Some(target) = target {
                        args.push(self.require_storage(&storage_key(target)?)?.pointer);
                    }
                    let name = self.next_name("call");
                    let status = self.builder.call(function, &args, &name)?;
                    let zero = self.status(0)?;
                    let name = self.next_name("call.failed");
                    let failed =
                        self.builder
                            .compare(BridgeCompareOp::IcmpNe, status, zero, &name)?;
                    self.guard_with_status(failed, status)
                } else if let Some(target) = target {
                    let name = self.next_name("call");
                    let result = self.builder.call(function, &args, &name)?;
                    self.store_value(target, result)
                } else {
                    self.builder.call(function, &args, "").map(|_| ())
                }
            }
            MirInstruction::RuntimeCall { intrinsic, args } => {
                let (name, _) = runtime_signature(*intrinsic);
                let function = require_function(self.functions, name)?;
                let args = self.physical_args(args)?;
                self.builder.call(function, &args, "").map(|_| ())
            }
        }
    }

    fn terminator(&mut self, terminator: &MirTerminator) -> Result<(), NativeError> {
        match terminator {
            MirTerminator::Return { value: Some(value) } => {
                let value = self.load_value(value)?;
                if self.status_abi() {
                    let result_pointer = self.result_pointer.ok_or_else(|| {
                        lowering_error("checked non-void function is missing result pointer")
                    })?;
                    self.builder.store(value, result_pointer)?;
                    let ok = self.status(0)?;
                    self.builder.return_value(ok)
                } else {
                    self.builder.return_value(value)
                }
            }
            MirTerminator::Return { value: None } => {
                if self.status_abi() {
                    let ok = self.status(0)?;
                    self.builder.return_value(ok)
                } else {
                    self.builder.return_void()
                }
            }
            MirTerminator::Jump { label } => self.builder.branch(self.block(label)?),
            MirTerminator::Branch {
                condition,
                then_label,
                else_label,
            } => {
                let condition = self.load_value(condition)?;
                self.builder.cond_branch(
                    condition,
                    self.block(then_label)?,
                    self.block(else_label)?,
                )
            }
        }
    }

    fn physical_args(
        &mut self,
        args: &[MirValue],
    ) -> Result<Vec<NativeValue<'module>>, NativeError> {
        let mut physical = Vec::new();
        for arg in args {
            let value = self.load_value(arg)?;
            if matches!(value_type(arg), MirType::Slice(_)) {
                let name = self.next_name("arg.data");
                physical.push(self.builder.extract_value(value, 0, &name)?);
                let name = self.next_name("arg.len");
                physical.push(self.builder.extract_value(value, 1, &name)?);
            } else {
                physical.push(value);
            }
        }
        Ok(physical)
    }

    fn make_slice(
        &mut self,
        data: NativeValue<'module>,
        len: NativeValue<'module>,
    ) -> Result<NativeValue<'module>, NativeError> {
        let undef = self.builder.undef(self.types.slice)?;
        let name = self.next_name("slice.data");
        let with_data = self.builder.insert_value(undef, data, 0, &name)?;
        let name = self.next_name("slice.value");
        self.builder.insert_value(with_data, len, 1, &name)
    }

    fn load_value(&mut self, value: &MirValue) -> Result<NativeValue<'module>, NativeError> {
        match value {
            MirValue::ConstInt { text, type_node } => {
                self.builder.const_int(self.types.get(type_node)?, text)
            }
            MirValue::ConstFloat { text, type_node } => {
                self.builder.const_float(self.types.get(type_node)?, text)
            }
            MirValue::ConstBool { value, .. } => self.builder.const_bool(*value),
            _ => {
                let storage = self.require_storage(&storage_key(value)?)?;
                let name = self.next_name("load");
                self.builder
                    .load(self.types.get(value_type(value))?, storage.pointer, &name)
            }
        }
    }

    fn store_value(
        &mut self,
        target: &MirValue,
        value: NativeValue<'module>,
    ) -> Result<(), NativeError> {
        let storage = self.require_storage(&storage_key(target)?)?;
        self.builder.store(value, storage.pointer)
    }

    fn require_storage(&self, key: &StorageKey) -> Result<Storage<'module>, NativeError> {
        self.storage
            .get(key)
            .copied()
            .ok_or_else(|| lowering_error(format!("missing native storage for {key:?}")))
    }

    fn place_pointer(&mut self, place: &MirPlace) -> Result<NativeValue<'module>, NativeError> {
        match place {
            MirPlace::Param { name, type_node } => {
                let storage = self.require_storage(&StorageKey::Param(name.clone()))?;
                if matches!(type_node, MirType::Pointer(_)) {
                    let name = self.next_name("pointer");
                    self.builder
                        .load(self.types.pointer, storage.pointer, &name)
                } else {
                    Ok(storage.pointer)
                }
            }
            MirPlace::Local { name, type_node } => {
                let storage = self.require_storage(&StorageKey::Local(name.clone()))?;
                if matches!(type_node, MirType::Pointer(_)) {
                    let name = self.next_name("pointer");
                    self.builder
                        .load(self.types.pointer, storage.pointer, &name)
                } else {
                    Ok(storage.pointer)
                }
            }
            MirPlace::Deref { pointer, .. } => self.load_value(pointer),
            MirPlace::Index { base, index, .. } => {
                let MirType::Pointer(element_type) = place_type(base) else {
                    return Err(lowering_error("MIR index base is not a pointer"));
                };
                let base = self.place_pointer(base)?;
                let index_value = self.load_value(index)?;
                let index64 = self.index_to_i64(index_value, value_type(index))?;
                let name = self.next_name("index");
                self.builder
                    .gep(self.types.get(element_type)?, base, &[index64], &name)
            }
            MirPlace::SliceIndex { slice, index, .. } => {
                let MirType::Slice(element_type) = value_type(slice) else {
                    return Err(lowering_error("MIR slice index base is not a slice"));
                };
                let descriptor = self.load_value(slice)?;
                let name = self.next_name("slice.data");
                let data = self.builder.extract_value(descriptor, 0, &name)?;
                let name = self.next_name("slice.len");
                let len = self.builder.extract_value(descriptor, 1, &name)?;
                let index_value = self.load_value(index)?;
                if self.options.bounds_mode == BoundsMode::Checked {
                    let name = self.next_name("slice.out_of_bounds");
                    let invalid =
                        self.builder
                            .compare(BridgeCompareOp::IcmpUge, index_value, len, &name)?;
                    let status = self.status(4)?;
                    self.guard_with_status(invalid, status)?;
                }
                let index64 = self.index_to_i64(index_value, value_type(index))?;
                let name = self.next_name("slice.index");
                self.builder
                    .gep(self.types.get(element_type)?, data, &[index64], &name)
            }
            MirPlace::Field {
                base, field_name, ..
            } => {
                let MirType::Struct(struct_name) = place_type(base) else {
                    return Err(lowering_error("MIR field base is not a struct"));
                };
                let base = self.place_pointer(base)?;
                let zero = self.builder.const_int(self.types.i32, "0")?;
                let field_index = self.layout.field_index(struct_name, field_name).to_string();
                let field = self.builder.const_int(self.types.i32, &field_index)?;
                let name = self.next_name("field");
                self.builder.gep(
                    self.types.get(&MirType::Struct(struct_name.clone()))?,
                    base,
                    &[zero, field],
                    &name,
                )
            }
        }
    }

    fn index_to_i64(
        &mut self,
        value: NativeValue<'module>,
        type_node: &MirType,
    ) -> Result<NativeValue<'module>, NativeError> {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => {
                let name = self.next_name("index64");
                self.builder
                    .cast(BridgeCastOp::Sext, value, self.types.i64, &name)
            }
            MirType::Primitive(MirPrimitiveTypeName::U32) => {
                let name = self.next_name("index64");
                self.builder
                    .cast(BridgeCastOp::Zext, value, self.types.i64, &name)
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => Ok(value),
            _ => Err(lowering_error("MIR index is not i32, u32, i64, or u64")),
        }
    }
}

fn storage_key(value: &MirValue) -> Result<StorageKey, NativeError> {
    match value {
        MirValue::Param { name, .. } => Ok(StorageKey::Param(name.clone())),
        MirValue::Local { name, .. } => Ok(StorageKey::Local(name.clone())),
        MirValue::Temp { name, .. } => Ok(StorageKey::Temp(name.clone())),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            Err(lowering_error(
                "constant MIR value cannot be a storage target",
            ))
        }
    }
}

fn collect_temps(function: &MirFunction) -> Vec<(String, MirType)> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, type_node }) = instruction_target(instruction)
                && seen.insert(name.clone())
            {
                result.push((name.clone(), type_node.clone()));
            }
        }
    }
    result
}

fn instruction_target(instruction: &MirInstruction) -> Option<&MirValue> {
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

fn value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}

fn place_type(place: &MirPlace) -> &MirType {
    match place {
        MirPlace::Param { type_node, .. }
        | MirPlace::Local { type_node, .. }
        | MirPlace::Deref { type_node, .. }
        | MirPlace::Index { type_node, .. }
        | MirPlace::SliceIndex { type_node, .. }
        | MirPlace::Field { type_node, .. } => type_node,
    }
}

fn require_function<'module>(
    functions: &HashMap<String, NativeFunction<'module>>,
    name: &str,
) -> Result<NativeFunction<'module>, NativeError> {
    functions
        .get(name)
        .copied()
        .ok_or_else(|| lowering_error(format!("unknown MIR function '{name}'")))
}

fn binary_op(op: MirBinaryOp, type_node: &MirType) -> Result<BridgeBinaryOp, NativeError> {
    let float = matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64));
    let unsigned = matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    );
    match (op, float, unsigned) {
        (MirBinaryOp::Add, true, _) => Ok(BridgeBinaryOp::FAdd),
        (MirBinaryOp::Sub, true, _) => Ok(BridgeBinaryOp::FSub),
        (MirBinaryOp::Mul, true, _) => Ok(BridgeBinaryOp::FMul),
        (MirBinaryOp::Div, true, _) => Ok(BridgeBinaryOp::FDiv),
        (MirBinaryOp::Mod, true, _) => Err(lowering_error("f64 modulo is unsupported")),
        (MirBinaryOp::Add, false, _) => Ok(BridgeBinaryOp::Add),
        (MirBinaryOp::Sub, false, _) => Ok(BridgeBinaryOp::Sub),
        (MirBinaryOp::Mul, false, _) => Ok(BridgeBinaryOp::Mul),
        (MirBinaryOp::Div, false, true) => Ok(BridgeBinaryOp::UDiv),
        (MirBinaryOp::Div, false, false) => Ok(BridgeBinaryOp::SDiv),
        (MirBinaryOp::Mod, false, true) => Ok(BridgeBinaryOp::URem),
        (MirBinaryOp::Mod, false, false) => Ok(BridgeBinaryOp::SRem),
    }
}

fn unary_op(op: MirUnaryOp, type_node: &MirType) -> BridgeUnaryOp {
    match (op, type_node) {
        (MirUnaryOp::Not, _) => BridgeUnaryOp::Not,
        (MirUnaryOp::Neg, MirType::Primitive(MirPrimitiveTypeName::F64)) => BridgeUnaryOp::FNeg,
        (MirUnaryOp::Neg, _) => BridgeUnaryOp::Neg,
    }
}

fn compare_op(op: MirCompareOp, type_node: &MirType) -> BridgeCompareOp {
    let float = matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64));
    let unsigned = matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    );
    match (op, float, unsigned) {
        (MirCompareOp::Eq, true, _) => BridgeCompareOp::FcmpOeq,
        (MirCompareOp::Ne, true, _) => BridgeCompareOp::FcmpUne,
        (MirCompareOp::Lt, true, _) => BridgeCompareOp::FcmpOlt,
        (MirCompareOp::Le, true, _) => BridgeCompareOp::FcmpOle,
        (MirCompareOp::Gt, true, _) => BridgeCompareOp::FcmpOgt,
        (MirCompareOp::Ge, true, _) => BridgeCompareOp::FcmpOge,
        (MirCompareOp::Eq, false, _) => BridgeCompareOp::IcmpEq,
        (MirCompareOp::Ne, false, _) => BridgeCompareOp::IcmpNe,
        (MirCompareOp::Lt, false, true) => BridgeCompareOp::IcmpUlt,
        (MirCompareOp::Le, false, true) => BridgeCompareOp::IcmpUle,
        (MirCompareOp::Gt, false, true) => BridgeCompareOp::IcmpUgt,
        (MirCompareOp::Ge, false, true) => BridgeCompareOp::IcmpUge,
        (MirCompareOp::Lt, false, false) => BridgeCompareOp::IcmpSlt,
        (MirCompareOp::Le, false, false) => BridgeCompareOp::IcmpSle,
        (MirCompareOp::Gt, false, false) => BridgeCompareOp::IcmpSgt,
        (MirCompareOp::Ge, false, false) => BridgeCompareOp::IcmpSge,
    }
}

fn lowering_error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Module, LOWERING_ERROR, message.into())
}
