use super::model::value_type;
use super::*;

pub fn print_mir_module(module: &MirModule) -> String {
    let mut parts = Vec::new();
    if let Some(entry) = &module.entry {
        parts.push(format!(
            "entry {} -> {}",
            entry.function_name,
            match entry.result {
                MirEntryResult::Void => "void",
                MirEntryResult::I32 => "i32",
            }
        ));
    }
    for struct_info in &module.structs {
        parts.push(print_mir_struct(struct_info));
    }
    for function in &module.functions {
        parts.push(print_mir_function(function));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}\n", parts.join("\n\n"))
    }
}

fn print_mir_struct(struct_info: &MirStruct) -> String {
    let mut lines = vec![format!("struct {} {{", struct_info.name)];
    for field in &struct_info.fields {
        lines.push(format!(
            "  {}: {}",
            field.name,
            print_mir_type(&field.type_node)
        ));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

fn print_mir_function(function: &MirFunction) -> String {
    let exported = if function.exported { "export " } else { "" };
    let params = function
        .params
        .iter()
        .map(|param| format!("{}: {}", param.name, print_mir_type(&param.type_node)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "{exported}fn {}({params}) -> {} {{",
        function.name,
        print_mir_type(&function.return_type)
    )];

    if !function.locals.is_empty() {
        for local in &function.locals {
            lines.push(format!(
                "  local {}: {}",
                local.name,
                print_mir_type(&local.type_node)
            ));
        }
        if !function.blocks.is_empty() {
            lines.push(String::new());
        }
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.push(format!("{}:", block.label));
        for instruction in &block.instructions {
            lines.push(format!("  {}", print_mir_instruction(instruction)));
        }
        lines.push(format!("  {}", print_mir_terminator(&block.terminator)));
    }

    lines.push("}".to_string());
    lines.join("\n")
}

fn print_mir_instruction(instruction: &MirInstruction) -> String {
    match instruction {
        MirInstruction::ConstInt { target, value } => format!(
            "{}: {} = const_int {value}",
            print_mir_value(target),
            print_mir_type(value_type(target))
        ),
        MirInstruction::ConstFloat { target, value } => format!(
            "{}: {} = const_float {value}",
            print_mir_value(target),
            print_mir_type(value_type(target))
        ),
        MirInstruction::ConstBool { target, value } => format!(
            "{}: {} = const_bool {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            if *value { "true" } else { "false" }
        ),
        MirInstruction::Move { target, value } => format!(
            "{}: {} = move {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_value(value)
        ),
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => format!(
            "{}: {} = {} {}, {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_binary_op(*op),
            print_mir_value(left),
            print_mir_value(right)
        ),
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => format!(
            "{}: {} = {} {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_unary_op(*op),
            print_mir_value(operand)
        ),
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => format!(
            "{}: {} = {} {}, {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_compare_op(*op),
            print_mir_value(left),
            print_mir_value(right)
        ),
        MirInstruction::Cast { target, op, value } => format!(
            "{}: {} = cast {} {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_cast_op(*op),
            print_mir_value(value)
        ),
        MirInstruction::Address { target, place } => format!(
            "{}: {} = address {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_place(place)
        ),
        MirInstruction::Load { target, place } => format!(
            "{}: {} = load {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_place(place)
        ),
        MirInstruction::Store { place, value } => {
            format!(
                "store {}, {}",
                print_mir_place(place),
                print_mir_value(value)
            )
        }
        MirInstruction::MakeSlice { target, data, len } => format!(
            "{}: {} = make_slice {}, {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_value(data),
            print_mir_value(len)
        ),
        MirInstruction::SliceData { target, slice } => format!(
            "{}: {} = slice_data {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_value(slice)
        ),
        MirInstruction::SliceLen { target, slice } => format!(
            "{}: {} = slice_len {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_value(slice)
        ),
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => format!(
            "{}: {} = subslice {}, {}, {}",
            print_mir_value(target),
            print_mir_type(value_type(target)),
            print_mir_value(slice),
            print_mir_value(start),
            print_mir_value(end)
        ),
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let call = format!(
                "call {}({})",
                function_name,
                args.iter()
                    .map(print_mir_value)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            target.as_ref().map_or(call.clone(), |target| {
                format!(
                    "{}: {} = {call}",
                    print_mir_value(target),
                    print_mir_type(value_type(target))
                )
            })
        }
        MirInstruction::RuntimeCall { intrinsic, args } => format!(
            "runtime_call {}({})",
            print_runtime_intrinsic(*intrinsic),
            args.iter()
                .map(print_mir_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[must_use]
pub fn print_runtime_intrinsic(intrinsic: MirRuntimeIntrinsic) -> &'static str {
    match intrinsic {
        MirRuntimeIntrinsic::PrintI32 => "print_i32",
        MirRuntimeIntrinsic::PrintI64 => "print_i64",
        MirRuntimeIntrinsic::PrintU32 => "print_u32",
        MirRuntimeIntrinsic::PrintU64 => "print_u64",
        MirRuntimeIntrinsic::PrintF64 => "print_f64",
        MirRuntimeIntrinsic::PrintBool => "print_bool",
        MirRuntimeIntrinsic::PrintNewline => "print_newline",
    }
}

fn print_mir_terminator(terminator: &MirTerminator) -> String {
    match terminator {
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || "return".to_string(),
            |value| format!("return {}", print_mir_value(value)),
        ),
        MirTerminator::Jump { label } => format!("jump {label}"),
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => format!(
            "branch {}, {}, {}",
            print_mir_value(condition),
            then_label,
            else_label
        ),
    }
}

fn print_mir_value(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
        MirValue::Temp { name, .. } => format!("%{name}"),
        MirValue::ConstInt { text, .. } | MirValue::ConstFloat { text, .. } => text.clone(),
        MirValue::ConstBool { value, .. } => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
    }
}

fn print_mir_place(place: &MirPlace) -> String {
    match place {
        MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => name.clone(),
        MirPlace::Deref { pointer, .. } => format!("deref({})", print_mir_value(pointer)),
        MirPlace::Index { base, index, .. } => {
            format!(
                "index({}, {})",
                print_mir_place(base),
                print_mir_value(index)
            )
        }
        MirPlace::SliceIndex { slice, index, .. } => format!(
            "slice_index({}, {})",
            print_mir_value(slice),
            print_mir_value(index)
        ),
        MirPlace::Field {
            base, field_name, ..
        } => {
            format!("field({}, {field_name})", print_mir_place(base))
        }
    }
}

#[must_use]
pub fn print_mir_type(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => print_primitive_type(*name).to_string(),
        MirType::Pointer(element_type) => format!("ptr<{}>", print_mir_type(element_type)),
        MirType::Slice(element_type) => format!("slice<{}>", print_mir_type(element_type)),
        MirType::Struct(name) => name.clone(),
        MirType::Void => "void".to_string(),
    }
}

fn print_primitive_type(name: MirPrimitiveTypeName) -> &'static str {
    match name {
        MirPrimitiveTypeName::I32 => "i32",
        MirPrimitiveTypeName::I64 => "i64",
        MirPrimitiveTypeName::U32 => "u32",
        MirPrimitiveTypeName::U64 => "u64",
        MirPrimitiveTypeName::F64 => "f64",
        MirPrimitiveTypeName::Bool => "bool",
    }
}

fn print_binary_op(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "add",
        MirBinaryOp::Sub => "sub",
        MirBinaryOp::Mul => "mul",
        MirBinaryOp::Div => "div",
        MirBinaryOp::Mod => "mod",
    }
}

fn print_compare_op(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "eq",
        MirCompareOp::Ne => "ne",
        MirCompareOp::Lt => "lt",
        MirCompareOp::Le => "le",
        MirCompareOp::Gt => "gt",
        MirCompareOp::Ge => "ge",
    }
}

fn print_unary_op(op: MirUnaryOp) -> &'static str {
    match op {
        MirUnaryOp::Neg => "neg",
        MirUnaryOp::Not => "not",
    }
}

pub(super) fn print_cast_op(op: MirCastOp) -> &'static str {
    match op {
        MirCastOp::I32ToF64 => "i32_to_f64",
        MirCastOp::U32ToF64 => "u32_to_f64",
    }
}
