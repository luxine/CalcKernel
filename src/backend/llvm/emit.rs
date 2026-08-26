use std::collections::HashSet;

use crate::*;

use super::super::{collect_temps, is_f64_type, is_unsigned_integer_type, place_type, value_type};
use super::{EmitLlvmOptions, layout::*, names::*};

#[must_use]
pub fn emit_llvm_module(module: &MirModule, options: &EmitLlvmOptions) -> String {
    let mut out = String::new();
    out.push_str("; ModuleID = 'calckernel'\n");
    out.push_str(&format!(
        "source_filename = \"{}\"\n",
        llvm_escape_string(&llvm_source_file_name(options.source_file_name.as_deref()))
    ));
    if let Some(target) = &options.target_triple {
        out.push_str(&format!(
            "target triple = \"{}\"\n",
            llvm_escape_string(target)
        ));
    }
    if !module.structs.is_empty() || !module.functions.is_empty() {
        out.push('\n');
    }

    for struct_info in &module.structs {
        out.push_str(&format!(
            "%struct.{} = type {{ {} }}\n\n",
            struct_info.name,
            struct_info
                .fields
                .iter()
                .map(|field| llvm_storage_type(&field.type_node))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let layout = LlvmStructLayout::new(module);
    for (index, function) in module.functions.iter().enumerate() {
        emit_llvm_function(&mut out, function, &layout);
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

pub(super) fn emit_llvm_function(
    out: &mut String,
    function: &MirFunction,
    layout: &LlvmStructLayout,
) {
    let linkage = if function.exported { "" } else { "internal " };
    let params = function
        .params
        .iter()
        .flat_map(|param| match &param.type_node {
            MirType::Slice(_) => vec![
                format!("ptr %{}.data", param.name),
                format!("i32 %{}.len", param.name),
            ],
            _ => vec![format!(
                "{} %{}",
                llvm_param_type(&param.type_node),
                param.name
            )],
        })
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!(
        "define {linkage}{} @{}({}) {{\n",
        llvm_return_type(&function.return_type),
        function.name,
        params
    ));

    let used_value_names = function
        .params
        .iter()
        .flat_map(|param| match param.type_node {
            MirType::Slice(_) => vec![
                format!("{}.data", param.name),
                format!("{}.len", param.name),
            ],
            _ => vec![param.name.clone()],
        })
        .collect();
    let mut context = LlvmFunctionContext {
        register_counter: 0,
        used_value_names,
        layout,
    };

    if function.blocks.is_empty() {
        out.push_str("entry:\n");
        if matches!(function.return_type, MirType::Void) {
            out.push_str("  ret void\n");
        } else {
            out.push_str(&format!(
                "  ret {} {}\n",
                llvm_return_type(&function.return_type),
                llvm_zero_value(&function.return_type)
            ));
        }
        out.push_str("}\n");
        return;
    }

    for (index, block) in function.blocks.iter().enumerate() {
        out.push_str(&format!("{}:\n", llvm_block_label(function, &block.label)));
        if index == 0 {
            emit_llvm_allocas(out, function);
            emit_llvm_param_stores(out, &mut context, function);
        }
        for instruction in &block.instructions {
            emit_llvm_instruction(out, &mut context, instruction);
        }
        emit_llvm_terminator(out, &mut context, function, &block.terminator);
    }
    out.push_str("}\n");
}

pub(super) fn emit_llvm_allocas(out: &mut String, function: &MirFunction) {
    let mut emitted = HashSet::new();
    for param in &function.params {
        out.push_str(&format!(
            "  {} = alloca {}\n",
            llvm_address_name(&param.name),
            llvm_storage_type(&param.type_node)
        ));
    }
    for local in &function.locals {
        if emitted.insert(local.name.clone()) {
            out.push_str(&format!(
                "  {} = alloca {}\n",
                llvm_address_name(&local.name),
                llvm_storage_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if emitted.insert(name.clone()) {
            out.push_str(&format!(
                "  {} = alloca {}\n",
                llvm_address_name(&llvm_storage_name_for_temp(&name)),
                llvm_storage_type(&type_node)
            ));
        }
    }
}

pub(super) fn emit_llvm_param_stores(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    function: &MirFunction,
) {
    for param in &function.params {
        if matches!(param.type_node, MirType::Slice(_)) {
            let with_data = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_data} = insertvalue {{ ptr, i32 }} undef, ptr %{}.data, 0\n",
                param.name
            ));
            let with_len = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_len} = insertvalue {{ ptr, i32 }} {with_data}, i32 %{}.len, 1\n",
                param.name
            ));
            out.push_str(&format!(
                "  store {{ ptr, i32 }} {with_len}, ptr {}\n",
                llvm_address_name(&param.name)
            ));
        } else {
            out.push_str(&format!(
                "  store {} %{}, ptr {}\n",
                llvm_param_type(&param.type_node),
                param.name,
                llvm_address_name(&param.name)
            ));
        }
    }
}

pub(super) fn emit_llvm_instruction(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    instruction: &MirInstruction,
) {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            emit_llvm_store(out, target, value);
        }
        MirInstruction::ConstFloat { target, value } => {
            emit_llvm_store(out, target, value);
        }
        MirInstruction::ConstBool { target, value } => {
            emit_llvm_store(out, target, if *value { "1" } else { "0" });
        }
        MirInstruction::Move { target, value } => {
            let loaded = llvm_load_value(out, context, value);
            emit_llvm_store(out, target, &loaded);
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            let left_value = llvm_load_value(out, context, left);
            let right_value = llvm_load_value(out, context, right);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = {} {} {}, {}\n",
                llvm_binary_opcode(*op, value_type(target)),
                llvm_value_type(value_type(target)),
                left_value,
                right_value
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            let operand_value = llvm_load_value(out, context, operand);
            let result = llvm_next_register(context);
            match op {
                MirUnaryOp::Not => {
                    out.push_str(&format!("  {result} = xor i1 {operand_value}, true\n"))
                }
                MirUnaryOp::Neg if is_f64_type(value_type(target)) => out.push_str(&format!(
                    "  {result} = fneg {} {operand_value}\n",
                    llvm_value_type(value_type(target))
                )),
                MirUnaryOp::Neg => out.push_str(&format!(
                    "  {result} = sub {} 0, {operand_value}\n",
                    llvm_value_type(value_type(target))
                )),
            }
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            let left_value = llvm_load_value(out, context, left);
            let right_value = llvm_load_value(out, context, right);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = {} {} {} {}, {}\n",
                if is_f64_type(value_type(left)) {
                    "fcmp"
                } else {
                    "icmp"
                },
                llvm_compare_predicate(*op, value_type(left)),
                llvm_value_type(value_type(left)),
                left_value,
                right_value
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Cast { target, op, value } => {
            let value_text = llvm_load_value(out, context, value);
            let result = llvm_next_register(context);
            let opcode = match op {
                MirCastOp::I32ToF64 => "sitofp",
                MirCastOp::U32ToF64 => "uitofp",
            };
            out.push_str(&format!(
                "  {result} = {opcode} {} {value_text} to {}\n",
                llvm_value_type(value_type(value)),
                llvm_value_type(value_type(target))
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Address { target, place } => {
            let pointer = llvm_place_pointer(out, context, place);
            emit_llvm_store(out, target, &pointer);
        }
        MirInstruction::Load { target, place } => {
            let pointer = llvm_place_pointer(out, context, place);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = load {}, ptr {pointer}\n",
                llvm_value_type(value_type(target))
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Store { place, value } => {
            let pointer = llvm_place_pointer(out, context, place);
            let value_text = llvm_load_value(out, context, value);
            out.push_str(&format!(
                "  store {} {}, ptr {pointer}\n",
                llvm_value_type(value_type(value)),
                value_text
            ));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let mut physical_args = Vec::new();
            for arg in args {
                if matches!(value_type(arg), MirType::Slice(_)) {
                    let aggregate = llvm_load_value(out, context, arg);
                    let data = llvm_next_register(context);
                    out.push_str(&format!(
                        "  {data} = extractvalue {{ ptr, i32 }} {aggregate}, 0\n"
                    ));
                    let len = llvm_next_register(context);
                    out.push_str(&format!(
                        "  {len} = extractvalue {{ ptr, i32 }} {aggregate}, 1\n"
                    ));
                    physical_args.push(format!("ptr {data}"));
                    physical_args.push(format!("i32 {len}"));
                } else {
                    let value = llvm_load_value(out, context, arg);
                    physical_args.push(format!("{} {}", llvm_value_type(value_type(arg)), value));
                }
            }
            let args = physical_args.join(", ");
            if let Some(target) = target {
                let result = llvm_next_register(context);
                out.push_str(&format!(
                    "  {result} = call {} @{}({args})\n",
                    llvm_return_type(value_type(target)),
                    function_name
                ));
                emit_llvm_store(out, target, &result);
            } else {
                out.push_str(&format!("  call void @{function_name}({args})\n"));
            }
        }
        MirInstruction::MakeSlice { target, data, len } => {
            let data = llvm_load_value(out, context, data);
            let len = llvm_load_value(out, context, len);
            let with_data = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_data} = insertvalue {{ ptr, i32 }} undef, ptr {data}, 0\n"
            ));
            let descriptor = llvm_next_register(context);
            out.push_str(&format!(
                "  {descriptor} = insertvalue {{ ptr, i32 }} {with_data}, i32 {len}, 1\n"
            ));
            emit_llvm_store(out, target, &descriptor);
        }
        MirInstruction::SliceData { target, slice } => {
            let descriptor = llvm_load_value(out, context, slice);
            let data = llvm_next_register(context);
            out.push_str(&format!(
                "  {data} = extractvalue {{ ptr, i32 }} {descriptor}, 0\n"
            ));
            emit_llvm_store(out, target, &data);
        }
        MirInstruction::SliceLen { target, slice } => {
            let descriptor = llvm_load_value(out, context, slice);
            let len = llvm_next_register(context);
            out.push_str(&format!(
                "  {len} = extractvalue {{ ptr, i32 }} {descriptor}, 1\n"
            ));
            emit_llvm_store(out, target, &len);
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("subslice source must be a slice")
            };
            let descriptor = llvm_load_value(out, context, slice);
            let data = llvm_next_register(context);
            out.push_str(&format!(
                "  {data} = extractvalue {{ ptr, i32 }} {descriptor}, 0\n"
            ));
            let start = llvm_load_value(out, context, start);
            let end = llvm_load_value(out, context, end);
            let start64 = llvm_index_to_i64(
                out,
                context,
                &MirType::Primitive(MirPrimitiveTypeName::U32),
                &start,
            );
            let advanced = llvm_next_register(context);
            out.push_str(&format!(
                "  {advanced} = getelementptr {}, ptr {data}, i64 {start64}\n",
                llvm_storage_type(element_type)
            ));
            let is_zero = llvm_next_register(context);
            out.push_str(&format!("  {is_zero} = icmp eq i32 {start}, 0\n"));
            let selected = llvm_next_register(context);
            out.push_str(&format!(
                "  {selected} = select i1 {is_zero}, ptr {data}, ptr {advanced}\n"
            ));
            let len = llvm_next_register(context);
            out.push_str(&format!("  {len} = sub i32 {end}, {start}\n"));
            let with_data = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_data} = insertvalue {{ ptr, i32 }} undef, ptr {selected}, 0\n"
            ));
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = insertvalue {{ ptr, i32 }} {with_data}, i32 {len}, 1\n"
            ));
            emit_llvm_store(out, target, &result);
        }
    }
}

pub(super) fn emit_llvm_terminator(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    function: &MirFunction,
    terminator: &MirTerminator,
) {
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                let value_text = llvm_load_value(out, context, value);
                out.push_str(&format!(
                    "  ret {} {}\n",
                    llvm_return_type(value_type(value)),
                    value_text
                ));
            } else {
                out.push_str("  ret void\n");
            }
        }
        MirTerminator::Jump { label } => {
            out.push_str(&format!(
                "  br label %{}\n",
                llvm_block_label(function, label)
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let condition = llvm_load_value(out, context, condition);
            out.push_str(&format!(
                "  br i1 {condition}, label %{}, label %{}\n",
                llvm_block_label(function, then_label),
                llvm_block_label(function, else_label)
            ));
        }
    }
}

pub(super) fn emit_llvm_store(out: &mut String, target: &MirValue, value: &str) {
    out.push_str(&format!(
        "  store {} {}, ptr {}\n",
        llvm_storage_type(value_type(target)),
        value,
        llvm_address_for_value(target)
    ));
}

pub(super) fn llvm_load_value(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    value: &MirValue,
) -> String {
    match value {
        MirValue::ConstInt { text, .. } | MirValue::ConstFloat { text, .. } => text.clone(),
        MirValue::ConstBool { value, .. } => {
            if *value {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        MirValue::Param { .. } | MirValue::Local { .. } | MirValue::Temp { .. } => {
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = load {}, ptr {}\n",
                llvm_value_type(value_type(value)),
                llvm_address_for_value(value)
            ));
            result
        }
    }
}

pub(super) fn llvm_place_pointer(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    place: &MirPlace,
) -> String {
    match place {
        MirPlace::Param { name, type_node } | MirPlace::Local { name, type_node } => {
            if matches!(type_node, MirType::Pointer(_)) {
                let result = llvm_next_register(context);
                out.push_str(&format!(
                    "  {result} = load ptr, ptr {}\n",
                    llvm_address_name(name)
                ));
                result
            } else {
                llvm_address_name(name)
            }
        }
        MirPlace::Deref { pointer, .. } => llvm_load_value(out, context, pointer),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("LLVM index base must be pointer");
            };
            let base_pointer = llvm_place_pointer(out, context, base);
            let index_value = llvm_load_value(out, context, index);
            let index64 = llvm_index_to_i64(out, context, value_type(index), &index_value);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = getelementptr {}, ptr {}, i64 {}\n",
                llvm_storage_type(element_type),
                base_pointer,
                index64
            ));
            result
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("slice index base must be a slice")
            };
            let descriptor = llvm_load_value(out, context, slice);
            let data = llvm_next_register(context);
            out.push_str(&format!(
                "  {data} = extractvalue {{ ptr, i32 }} {descriptor}, 0\n"
            ));
            let index = llvm_load_value(out, context, index);
            let index64 = llvm_index_to_i64(
                out,
                context,
                &MirType::Primitive(MirPrimitiveTypeName::U32),
                &index,
            );
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = getelementptr {}, ptr {data}, i64 {index64}\n",
                llvm_storage_type(element_type)
            ));
            result
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("LLVM field base must be struct");
            };
            let base_pointer = llvm_place_pointer(out, context, base);
            let field_index = context.layout.field_index(struct_name, field_name);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = getelementptr %struct.{}, ptr {}, i32 0, i32 {}\n",
                struct_name, base_pointer, field_index
            ));
            result
        }
    }
}

pub(super) fn llvm_index_to_i64(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    index_type: &MirType,
    index_value: &str,
) -> String {
    match index_type {
        MirType::Primitive(MirPrimitiveTypeName::I32) => {
            let result = llvm_next_register(context);
            out.push_str(&format!("  {result} = sext i32 {index_value} to i64\n"));
            result
        }
        MirType::Primitive(MirPrimitiveTypeName::U32) => {
            let result = llvm_next_register(context);
            out.push_str(&format!("  {result} = zext i32 {index_value} to i64\n"));
            result
        }
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            index_value.to_string()
        }
        _ => panic!("LLVM index type must be i32, u32, i64, or u64"),
    }
}

pub(super) fn llvm_storage_type(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
            "i32".to_string()
        }
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            "i64".to_string()
        }
        MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::Bool) => "i1".to_string(),
        MirType::Pointer(_) => "ptr".to_string(),
        MirType::Slice(_) => "{ ptr, i32 }".to_string(),
        MirType::Struct(name) => format!("%struct.{name}"),
        MirType::Void => panic!("void has no LLVM storage type"),
    }
}

pub(super) fn llvm_value_type(type_node: &MirType) -> String {
    llvm_storage_type(type_node)
}

pub(super) fn llvm_param_type(type_node: &MirType) -> String {
    llvm_storage_type(type_node)
}

pub(super) fn llvm_return_type(type_node: &MirType) -> String {
    if matches!(type_node, MirType::Void) {
        "void".to_string()
    } else {
        llvm_storage_type(type_node)
    }
}

pub(super) fn llvm_zero_value(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::F64) => "0.0",
        MirType::Primitive(MirPrimitiveTypeName::Bool) => "0",
        MirType::Primitive(_) | MirType::Pointer(_) => "0",
        MirType::Struct(_) | MirType::Slice(_) => "zeroinitializer",
        MirType::Void => panic!("void has no LLVM zero value"),
    }
}

pub(super) fn llvm_binary_opcode(op: MirBinaryOp, type_node: &MirType) -> &'static str {
    if is_f64_type(type_node) {
        return match op {
            MirBinaryOp::Add => "fadd",
            MirBinaryOp::Sub => "fsub",
            MirBinaryOp::Mul => "fmul",
            MirBinaryOp::Div => "fdiv",
            MirBinaryOp::Mod => panic!("LLVM backend does not support f64 modulo"),
        };
    }

    match op {
        MirBinaryOp::Add => "add",
        MirBinaryOp::Sub => "sub",
        MirBinaryOp::Mul => "mul",
        MirBinaryOp::Div if is_unsigned_integer_type(type_node) => "udiv",
        MirBinaryOp::Div => "sdiv",
        MirBinaryOp::Mod if is_unsigned_integer_type(type_node) => "urem",
        MirBinaryOp::Mod => "srem",
    }
}

pub(super) fn llvm_compare_predicate(op: MirCompareOp, type_node: &MirType) -> &'static str {
    if is_f64_type(type_node) {
        return match op {
            MirCompareOp::Eq => "oeq",
            MirCompareOp::Ne => "une",
            MirCompareOp::Lt => "olt",
            MirCompareOp::Le => "ole",
            MirCompareOp::Gt => "ogt",
            MirCompareOp::Ge => "oge",
        };
    }

    let prefix = if is_unsigned_integer_type(type_node) {
        "u"
    } else {
        "s"
    };
    match op {
        MirCompareOp::Eq => "eq",
        MirCompareOp::Ne => "ne",
        MirCompareOp::Lt if prefix == "u" => "ult",
        MirCompareOp::Lt => "slt",
        MirCompareOp::Le if prefix == "u" => "ule",
        MirCompareOp::Le => "sle",
        MirCompareOp::Gt if prefix == "u" => "ugt",
        MirCompareOp::Gt => "sgt",
        MirCompareOp::Ge if prefix == "u" => "uge",
        MirCompareOp::Ge => "sge",
    }
}
