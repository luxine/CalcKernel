use std::collections::HashSet;

use crate::*;

use super::super::{collect_temps, is_f64_type, is_unsigned_integer_type, place_type, value_type};
use super::{EmitWasmOptions, layout::*, plan::*};

#[must_use]
pub fn emit_wat_module_with_options(module: &MirModule, options: EmitWasmOptions) -> String {
    let layout = WasmStructLayout::new(module);
    let mut out = String::new();
    out.push_str("(module\n");
    out.push_str("  (memory (export \"memory\") 1)\n");
    out.push_str("  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n");
    for function in &module.functions {
        out.push('\n');
        emit_wat_function(&mut out, function, &layout, options);
    }
    out.push_str(")\n");
    out
}

pub(super) fn emit_wat_function(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
    options: EmitWasmOptions,
) {
    if wasm_function_uses_slices(function) {
        emit_wat_slice_function(out, function, layout, options);
        return;
    }
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        out.push_str(&format!(
            "    (param ${} {})\n",
            param.name,
            wasm_type(&param.type_node)
        ));
    }
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (result {})\n",
            wasm_type(&function.return_type)
        ));
    }

    let mut locals = HashSet::new();
    for local in &function.locals {
        if locals.insert(local.name.clone()) {
            out.push_str(&format!(
                "    (local ${} {})\n",
                local.name,
                wasm_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if locals.insert(name.clone()) {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(&type_node)));
        }
    }

    if function.blocks.len() == 1 {
        for instruction in &function.blocks[0].instructions {
            emit_wat_instruction(out, instruction, layout, 4);
        }
        emit_wat_terminator(out, &function.blocks[0].terminator, None, 4);
    } else if let Some(loop_context) = (options.opt_level >= 3)
        .then(|| detect_simple_wasm_while(function))
        .flatten()
    {
        emit_structured_wasm_while(out, &loop_context, layout);
    } else {
        emit_dispatched_wasm_function(out, function, layout);
    }
    out.push_str("  )\n");
}

pub(super) fn emit_wat_slice_function(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
    options: EmitWasmOptions,
) {
    let plan = WasmFunctionPlan::new(function);
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        match plan
            .values
            .get(&format!("param:{}", param.name))
            .expect("param must have physical WASM names")
        {
            WasmPhysicalValue::Scalar(name) => out.push_str(&format!(
                "    (param ${name} {})\n",
                wasm_type(&param.type_node)
            )),
            WasmPhysicalValue::Slice { data, len } => {
                out.push_str(&format!("    (param ${data} i32)\n"));
                out.push_str(&format!("    (param ${len} i32)\n"));
            }
        }
    }
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str("    (result i32 i32)\n"),
        type_node => out.push_str(&format!("    (result {})\n", wasm_type(type_node))),
    }

    for local in &function.locals {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("local:{}", local.name))
                .expect("local must have physical WASM names"),
            &local.type_node,
        );
    }
    for (name, type_node) in collect_temps(function) {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("temp:{name}"))
                .expect("temp must have physical WASM names"),
            &type_node,
        );
    }
    out.push_str(&format!("    (local ${} i32)\n", plan.address_local));

    if function.blocks.len() == 1 {
        for instruction in &function.blocks[0].instructions {
            emit_wat_paired_instruction(out, instruction, layout, &plan, 4);
        }
        emit_wat_paired_terminator(out, &function.blocks[0].terminator, None, &plan, 4);
    } else if let Some(loop_context) = (options.opt_level >= 3)
        .then(|| detect_simple_wasm_while(function))
        .flatten()
    {
        emit_structured_wasm_while_paired(out, &loop_context, layout, &plan);
    } else {
        emit_dispatched_wasm_function_paired(out, function, layout, &plan);
    }
    out.push_str("  )\n");
}

pub(super) fn emit_wat_physical_local(
    out: &mut String,
    value: &WasmPhysicalValue,
    type_node: &MirType,
) {
    match value {
        WasmPhysicalValue::Scalar(name) => {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(type_node)));
        }
        WasmPhysicalValue::Slice { data, len } => {
            out.push_str(&format!("    (local ${data} i32)\n"));
            out.push_str(&format!("    (local ${len} i32)\n"));
        }
    }
}

pub(super) fn emit_structured_wasm_while_paired(
    out: &mut String,
    loop_context: &StructuredWasmWhile<'_>,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
) {
    for instruction in &loop_context.entry.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 4);
    }
    out.push_str(&format!("    block ${}\n", loop_context.exit_label));
    out.push_str(&format!("      loop ${}\n", loop_context.loop_label));
    for instruction in &loop_context.header.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 8);
    }
    let MirTerminator::Branch { condition, .. } = &loop_context.header.terminator else {
        unreachable!("structured while header is always a branch")
    };
    emit_wat_paired_scalar_value(out, condition, plan, 8);
    out.push_str("        i32.eqz\n");
    out.push_str(&format!("        br_if ${}\n", loop_context.exit_label));
    for instruction in &loop_context.body.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 8);
    }
    out.push_str(&format!("        br ${}\n", loop_context.loop_label));
    out.push_str("      end\n    end\n");
    for instruction in &loop_context.exit.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 4);
    }
    emit_wat_paired_terminator(out, &loop_context.exit.terminator, None, plan, 4);
}

pub(super) fn emit_dispatched_wasm_function_paired(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
) {
    out.push_str(&format!("    (local ${} i32)\n", plan.block_local));
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => {
            out.push_str(&format!("    (local ${} i32)\n", plan.return_data));
            out.push_str(&format!("    (local ${} i32)\n", plan.return_len));
        }
        type_node => out.push_str(&format!(
            "    (local ${} {})\n",
            plan.return_scalar,
            wasm_type(type_node)
        )),
    }
    out.push_str("    i32.const 0\n");
    out.push_str(&format!("    local.set ${}\n", plan.block_local));
    out.push_str("    block $ik_exit\n      loop $ik_dispatch\n");
    for index in 0..function.blocks.len() {
        out.push_str(&format!(
            "{}block $ik_case{index}\n",
            " ".repeat(8 + index * 2)
        ));
    }
    let case_labels = (0..function.blocks.len())
        .map(|index| format!("$ik_case{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dispatch_indent = " ".repeat(8 + function.blocks.len() * 2);
    out.push_str(&format!(
        "{dispatch_indent}local.get ${}\n{dispatch_indent}br_table {case_labels} $ik_case0\n",
        plan.block_local
    ));
    for index in (0..function.blocks.len()).rev() {
        let block_indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(block_indent)));
        let block = &function.blocks[index];
        for instruction in &block.instructions {
            emit_wat_paired_instruction(out, instruction, layout, plan, block_indent);
        }
        emit_wat_paired_terminator(out, &block.terminator, Some(function), plan, block_indent);
    }
    out.push_str("      end\n    end\n");
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str(&format!(
            "    local.get ${}\n    local.get ${}\n",
            plan.return_data, plan.return_len
        )),
        _ => out.push_str(&format!("    local.get ${}\n", plan.return_scalar)),
    }
}

pub(super) fn emit_wat_paired_instruction(
    out: &mut String,
    instruction: &MirInstruction,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match instruction {
        MirInstruction::ConstInt { target, value } => out.push_str(&format!(
            "{pad}{}.const {value}\n{pad}local.set ${}\n",
            wasm_type(value_type(target)),
            plan.scalar(target)
        )),
        MirInstruction::ConstFloat { target, value } => out.push_str(&format!(
            "{pad}f64.const {value}\n{pad}local.set ${}\n",
            plan.scalar(target)
        )),
        MirInstruction::ConstBool { target, value } => out.push_str(&format!(
            "{pad}i32.const {}\n{pad}local.set ${}\n",
            if *value { 1 } else { 0 },
            plan.scalar(target)
        )),
        MirInstruction::Move { target, value }
            if matches!(value_type(target), MirType::Slice(_)) =>
        {
            let (target_data, target_len) = plan.slice(target);
            let (value_data, value_len) = plan.slice(value);
            out.push_str(&format!(
                "{pad}local.get ${value_data}\n{pad}local.set ${target_data}\n\
                 {pad}local.get ${value_len}\n{pad}local.set ${target_len}\n"
            ));
        }
        MirInstruction::Move { target, value } => {
            emit_wat_paired_scalar_value(out, value, plan, indent);
            out.push_str(&format!("{pad}local.set ${}\n", plan.scalar(target)));
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_paired_scalar_value(out, left, plan, indent);
            emit_wat_paired_scalar_value(out, right, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_binary_instruction(*op, value_type(left)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => emit_wat_paired_unary(out, *op, operand, target, plan, indent),
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_paired_scalar_value(out, left, plan, indent);
            emit_wat_paired_scalar_value(out, right, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_compare_instruction(*op, value_type(left)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Cast { target, op, value } => {
            emit_wat_paired_scalar_value(out, value, plan, indent);
            let opcode = match op {
                MirCastOp::I32ToF64 => "f64.convert_i32_s",
                MirCastOp::U32ToF64 => "f64.convert_i32_u",
            };
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::Address { target, place } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!("{pad}local.set ${}\n", plan.scalar(target)));
        }
        MirInstruction::Load { target, place }
            if matches!(value_type(target), MirType::Slice(_)) =>
        {
            let (data, len) = plan.slice(target);
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}local.set ${}\n\
                 {pad}local.get ${}\n{pad}i32.load offset=0 align=4\n{pad}local.set ${data}\n\
                 {pad}local.get ${}\n{pad}i32.load offset=4 align=4\n{pad}local.set ${len}\n",
                plan.address_local, plan.address_local, plan.address_local
            ));
        }
        MirInstruction::Load { target, place } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                layout.align_of(value_type(target)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Store { place, value }
            if matches!(value_type(value), MirType::Slice(_)) =>
        {
            let (data, len) = plan.slice(value);
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}local.set ${}\n\
                 {pad}local.get ${}\n{pad}local.get ${data}\n{pad}i32.store offset=0 align=4\n\
                 {pad}local.get ${}\n{pad}local.get ${len}\n{pad}i32.store offset=4 align=4\n",
                plan.address_local, plan.address_local, plan.address_local
            ));
        }
        MirInstruction::Store { place, value } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            emit_wat_paired_scalar_value(out, value, plan, indent);
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(value_type(value)),
                layout.align_of(value_type(value))
            ));
        }
        MirInstruction::MakeSlice { target, data, len } => {
            let (target_data, target_len) = plan.slice(target);
            emit_wat_paired_scalar_value(out, data, plan, indent);
            out.push_str(&format!("{pad}local.set ${target_data}\n"));
            emit_wat_paired_scalar_value(out, len, plan, indent);
            out.push_str(&format!("{pad}local.set ${target_len}\n"));
        }
        MirInstruction::SliceData { target, slice } => {
            let (data, _) = plan.slice(slice);
            out.push_str(&format!(
                "{pad}local.get ${data}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::SliceLen { target, slice } => {
            let (_, len) = plan.slice(slice);
            out.push_str(&format!(
                "{pad}local.get ${len}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let (target_data, target_len) = plan.slice(target);
            let (slice_data, _) = plan.slice(slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("subslice source must be a slice")
            };
            out.push_str(&format!("{pad}local.get ${slice_data}\n"));
            emit_wat_paired_scalar_value(out, start, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n{pad}local.set ${target_data}\n",
                layout.size_of(element_type)
            ));
            emit_wat_paired_scalar_value(out, end, plan, indent);
            emit_wat_paired_scalar_value(out, start, plan, indent);
            out.push_str(&format!("{pad}i32.sub\n{pad}local.set ${target_len}\n"));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            for arg in args {
                if matches!(value_type(arg), MirType::Slice(_)) {
                    emit_wat_paired_slice_value(out, arg, plan, indent);
                } else {
                    emit_wat_paired_scalar_value(out, arg, plan, indent);
                }
            }
            out.push_str(&format!("{pad}call ${function_name}\n"));
            if let Some(target) = target {
                match plan.value(target) {
                    WasmPhysicalValue::Scalar(name) => {
                        out.push_str(&format!("{pad}local.set ${name}\n"));
                    }
                    WasmPhysicalValue::Slice { data, len } => {
                        out.push_str(&format!("{pad}local.set ${len}\n{pad}local.set ${data}\n"))
                    }
                }
            }
        }
        MirInstruction::RuntimeCall { .. } => {
            unreachable!("runtime calls must be rejected before WebAssembly emission")
        }
    }
}

pub(super) fn emit_wat_paired_scalar_value(
    out: &mut String,
    value: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match value {
        MirValue::Param { .. } | MirValue::Local { .. } | MirValue::Temp { .. } => {
            out.push_str(&format!("{pad}local.get ${}\n", plan.scalar(value)));
        }
        MirValue::ConstInt { text, type_node } => {
            out.push_str(&format!("{pad}{}.const {text}\n", wasm_type(type_node)));
        }
        MirValue::ConstFloat { text, .. } => out.push_str(&format!("{pad}f64.const {text}\n")),
        MirValue::ConstBool { value, .. } => {
            out.push_str(&format!("{pad}i32.const {}\n", if *value { 1 } else { 0 }))
        }
    }
}

pub(super) fn emit_wat_paired_slice_value(
    out: &mut String,
    value: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    let (data, len) = plan.slice(value);
    out.push_str(&format!("{pad}local.get ${data}\n{pad}local.get ${len}\n"));
}

pub(super) fn emit_wat_paired_unary(
    out: &mut String,
    op: MirUnaryOp,
    operand: &MirValue,
    target: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match op {
        MirUnaryOp::Not => {
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}i32.eqz\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirUnaryOp::Neg if is_f64_type(value_type(operand)) => {
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}f64.neg\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirUnaryOp::Neg => {
            out.push_str(&format!(
                "{pad}{}.const 0\n",
                wasm_type(value_type(operand))
            ));
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}{}.sub\n{pad}local.set ${}\n",
                wasm_type(value_type(operand)),
                plan.scalar(target)
            ));
        }
    }
}

pub(super) fn emit_wat_paired_address(
    out: &mut String,
    place: &MirPlace,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => match plan.place_value(place) {
            WasmPhysicalValue::Scalar(name) => {
                out.push_str(&format!("{pad}local.get ${name}\n"));
            }
            WasmPhysicalValue::Slice { .. } => {
                panic!("a logical slice local is not directly addressable")
            }
        },
        MirPlace::Deref { pointer, .. } => emit_wat_paired_scalar_value(out, pointer, plan, indent),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("WAT index base must be pointer");
            };
            emit_wat_paired_address(out, base, layout, plan, indent);
            emit_wat_paired_scalar_value(out, index, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            let (data, _) = plan.slice(slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("slice index base must be a slice")
            };
            out.push_str(&format!("{pad}local.get ${data}\n"));
            emit_wat_paired_scalar_value(out, index, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("WAT field base must be struct");
            };
            emit_wat_paired_address(out, base, layout, plan, indent);
            let offset = layout.field_offset(struct_name, field_name);
            if offset != 0 {
                out.push_str(&format!("{pad}i32.const {offset}\n{pad}i32.add\n"));
            }
        }
    }
}

pub(super) fn emit_wat_paired_terminator(
    out: &mut String,
    terminator: &MirTerminator,
    function: Option<&MirFunction>,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                if matches!(value_type(value), MirType::Slice(_)) {
                    emit_wat_paired_slice_value(out, value, plan, indent);
                    if function.is_some() {
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}local.set ${}\n{pad}br $ik_exit\n",
                            plan.return_len, plan.return_data
                        ));
                    } else {
                        out.push_str(&format!("{pad}return\n"));
                    }
                } else {
                    emit_wat_paired_scalar_value(out, value, plan, indent);
                    if function.is_some() {
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}br $ik_exit\n",
                            plan.return_scalar
                        ));
                    } else {
                        out.push_str(&format!("{pad}return\n"));
                    }
                }
            } else if function.is_some() {
                out.push_str(&format!("{pad}br $ik_exit\n"));
            } else {
                out.push_str(&format!("{pad}return\n"));
            }
        }
        MirTerminator::Jump { label } => {
            let index = block_index(function.expect("dispatcher function"), label);
            out.push_str(&format!(
                "{pad}i32.const {index}\n{pad}local.set ${}\n{pad}br $ik_dispatch\n",
                plan.block_local
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let function = function.expect("dispatcher function");
            emit_wat_paired_scalar_value(out, condition, plan, indent);
            out.push_str(&format!(
                "{pad}if\n{pad}  i32.const {}\n{pad}  local.set ${}\n{pad}else\n{pad}  i32.const {}\n{pad}  local.set ${}\n{pad}end\n{pad}br $ik_dispatch\n",
                block_index(function, then_label),
                plan.block_local,
                block_index(function, else_label),
                plan.block_local
            ));
        }
    }
}

pub(super) struct StructuredWasmWhile<'a> {
    entry: &'a MirBlock,
    header: &'a MirBlock,
    body: &'a MirBlock,
    exit: &'a MirBlock,
    loop_label: String,
    exit_label: String,
}

pub(super) fn detect_simple_wasm_while(function: &MirFunction) -> Option<StructuredWasmWhile<'_>> {
    if function.blocks.len() != 4 {
        return None;
    }

    let entry = function.blocks.first()?;
    let MirTerminator::Jump {
        label: header_label,
    } = &entry.terminator
    else {
        return None;
    };

    let header = wasm_block_by_label(function, header_label)?;
    let MirTerminator::Branch {
        then_label,
        else_label,
        ..
    } = &header.terminator
    else {
        return None;
    };

    let body = wasm_block_by_label(function, then_label)?;
    let exit = wasm_block_by_label(function, else_label)?;
    if !matches!(&body.terminator, MirTerminator::Jump { label } if label == &header.label)
        || !matches!(exit.terminator, MirTerminator::Return { .. })
    {
        return None;
    }

    let matched_labels = [
        entry.label.as_str(),
        header.label.as_str(),
        body.label.as_str(),
        exit.label.as_str(),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    if matched_labels.len() != function.blocks.len() {
        return None;
    }

    let mut used_names = collect_wasm_function_names(function);
    let loop_label = unique_wasm_internal_name("ik_loop", &mut used_names);
    let exit_label = unique_wasm_internal_name("ik_exit", &mut used_names);
    Some(StructuredWasmWhile {
        entry,
        header,
        body,
        exit,
        loop_label,
        exit_label,
    })
}

pub(super) fn wasm_block_by_label<'a>(
    function: &'a MirFunction,
    label: &str,
) -> Option<&'a MirBlock> {
    function.blocks.iter().find(|block| block.label == label)
}

pub(super) fn emit_structured_wasm_while(
    out: &mut String,
    loop_context: &StructuredWasmWhile<'_>,
    layout: &WasmStructLayout,
) {
    for instruction in &loop_context.entry.instructions {
        emit_wat_instruction(out, instruction, layout, 4);
    }
    out.push_str(&format!("    block ${}\n", loop_context.exit_label));
    out.push_str(&format!("      loop ${}\n", loop_context.loop_label));

    for instruction in &loop_context.header.instructions {
        emit_wat_instruction(out, instruction, layout, 8);
    }
    let MirTerminator::Branch { condition, .. } = &loop_context.header.terminator else {
        unreachable!("structured while header is always a branch")
    };
    emit_wat_value(out, condition, 8);
    out.push_str("        i32.eqz\n");
    out.push_str(&format!("        br_if ${}\n", loop_context.exit_label));

    for instruction in &loop_context.body.instructions {
        emit_wat_instruction(out, instruction, layout, 8);
    }
    out.push_str(&format!("        br ${}\n", loop_context.loop_label));
    out.push_str("      end\n");
    out.push_str("    end\n");

    for instruction in &loop_context.exit.instructions {
        emit_wat_instruction(out, instruction, layout, 4);
    }
    emit_wat_terminator(out, &loop_context.exit.terminator, None, 4);
}

pub(super) fn emit_dispatched_wasm_function(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
) {
    out.push_str("    (local $ik_bb i32)\n");
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (local $ik_ret {})\n",
            wasm_type(&function.return_type)
        ));
    }
    out.push_str("    i32.const 0\n");
    out.push_str("    local.set $ik_bb\n");
    out.push_str("    block $ik_exit\n");
    out.push_str("      loop $ik_dispatch\n");
    for index in 0..function.blocks.len() {
        out.push_str(&format!(
            "{}block $ik_case{index}\n",
            " ".repeat(8 + index * 2)
        ));
    }
    let case_labels = (0..function.blocks.len())
        .map(|index| format!("$ik_case{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dispatch_indent = " ".repeat(8 + function.blocks.len() * 2);
    out.push_str(&format!("{dispatch_indent}local.get $ik_bb\n"));
    out.push_str(&format!(
        "{dispatch_indent}br_table {case_labels} $ik_case0\n"
    ));
    for index in (0..function.blocks.len()).rev() {
        let block_indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(block_indent)));
        let block = &function.blocks[index];
        for instruction in &block.instructions {
            emit_wat_instruction(out, instruction, layout, block_indent);
        }
        emit_wat_terminator(out, &block.terminator, Some(function), block_indent);
    }
    out.push_str("      end\n");
    out.push_str("    end\n");
    if !matches!(function.return_type, MirType::Void) {
        out.push_str("    local.get $ik_ret\n");
    }
}

pub(super) fn emit_wat_instruction(
    out: &mut String,
    instruction: &MirInstruction,
    layout: &WasmStructLayout,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            out.push_str(&format!(
                "{pad}{}.const {value}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                wat_local_name(target)
            ));
        }
        MirInstruction::ConstFloat { target, value } => {
            out.push_str(&format!(
                "{pad}f64.const {value}\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirInstruction::ConstBool { target, value } => {
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}local.set ${}\n",
                if *value { 1 } else { 0 },
                wat_local_name(target)
            ));
        }
        MirInstruction::Move { target, value } => {
            emit_wat_value(out, value, indent);
            out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_value(out, left, indent);
            emit_wat_value(out, right, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_binary_instruction(*op, value_type(left)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            emit_wat_unary(out, *op, operand, target, indent);
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_value(out, left, indent);
            emit_wat_value(out, right, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_compare_instruction(*op, value_type(left)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Cast { target, op, value } => {
            emit_wat_value(out, value, indent);
            let opcode = match op {
                MirCastOp::I32ToF64 => "f64.convert_i32_s",
                MirCastOp::U32ToF64 => "f64.convert_i32_u",
            };
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirInstruction::Address { target, place } => {
            emit_wat_address(out, place, layout, indent);
            out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
        }
        MirInstruction::Load { target, place } => {
            emit_wat_address(out, place, layout, indent);
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                layout.align_of(value_type(target)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Store { place, value } => {
            emit_wat_address(out, place, layout, indent);
            emit_wat_value(out, value, indent);
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(value_type(value)),
                layout.align_of(value_type(value))
            ));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            for arg in args {
                emit_wat_value(out, arg, indent);
            }
            out.push_str(&format!("{pad}call ${function_name}\n"));
            if let Some(target) = target {
                out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
            }
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::RuntimeCall { .. } => {
            unreachable!("slice/runtime functions require a validated artifact plan")
        }
    }
}

pub(super) fn emit_wat_terminator(
    out: &mut String,
    terminator: &MirTerminator,
    function: Option<&MirFunction>,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                emit_wat_value(out, value, indent);
                if function.is_some() {
                    out.push_str(&format!("{pad}local.set $ik_ret\n{pad}br $ik_exit\n"));
                } else {
                    out.push_str(&format!("{pad}return\n"));
                }
            } else if function.is_some() {
                out.push_str(&format!("{pad}br $ik_exit\n"));
            } else {
                out.push_str(&format!("{pad}return\n"));
            }
        }
        MirTerminator::Jump { label } => {
            let index = block_index(function.expect("dispatcher function"), label);
            out.push_str(&format!(
                "{pad}i32.const {index}\n{pad}local.set $ik_bb\n{pad}br $ik_dispatch\n"
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let function = function.expect("dispatcher function");
            emit_wat_value(out, condition, indent);
            out.push_str(&format!(
                "{pad}if\n{pad}  i32.const {}\n{pad}  local.set $ik_bb\n{pad}else\n{pad}  i32.const {}\n{pad}  local.set $ik_bb\n{pad}end\n{pad}br $ik_dispatch\n",
                block_index(function, then_label),
                block_index(function, else_label)
            ));
        }
    }
}

pub(super) fn emit_wat_value(out: &mut String, value: &MirValue, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => {
            out.push_str(&format!("{pad}local.get ${name}\n"));
        }
        MirValue::ConstInt { text, type_node } => {
            out.push_str(&format!("{pad}{}.const {text}\n", wasm_type(type_node)));
        }
        MirValue::ConstFloat { text, .. } => {
            out.push_str(&format!("{pad}f64.const {text}\n"));
        }
        MirValue::ConstBool { value, .. } => {
            out.push_str(&format!("{pad}i32.const {}\n", if *value { 1 } else { 0 }));
        }
    }
}

pub(super) fn emit_wat_unary(
    out: &mut String,
    op: MirUnaryOp,
    operand: &MirValue,
    target: &MirValue,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match op {
        MirUnaryOp::Not => {
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}i32.eqz\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirUnaryOp::Neg if is_f64_type(value_type(operand)) => {
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}f64.neg\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirUnaryOp::Neg => {
            out.push_str(&format!(
                "{pad}{}.const 0\n",
                wasm_type(value_type(operand))
            ));
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}{}.sub\n{pad}local.set ${}\n",
                wasm_type(value_type(operand)),
                wat_local_name(target)
            ));
        }
    }
}

pub(super) fn wat_local_name(value: &MirValue) -> &str {
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => name,
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("WAT locals cannot be MIR constants")
        }
    }
}

pub(super) fn emit_wat_address(
    out: &mut String,
    place: &MirPlace,
    layout: &WasmStructLayout,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match place {
        MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => {
            out.push_str(&format!("{pad}local.get ${name}\n"));
        }
        MirPlace::Deref { pointer, .. } => emit_wat_value(out, pointer, indent),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("WAT index base must be pointer");
            };
            emit_wat_address(out, base, layout, indent);
            emit_wat_value(out, index, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::SliceIndex { .. } => {
            unreachable!("slice functions must use the paired WAT emitter")
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("WAT field base must be struct");
            };
            emit_wat_address(out, base, layout, indent);
            let offset = layout.field_offset(struct_name, field_name);
            if offset != 0 {
                out.push_str(&format!("{pad}i32.const {offset}\n{pad}i32.add\n"));
            }
        }
    }
}

pub(super) fn wat_binary_instruction(op: MirBinaryOp, type_node: &MirType) -> String {
    if is_f64_type(type_node) {
        return match op {
            MirBinaryOp::Add => "f64.add".to_string(),
            MirBinaryOp::Sub => "f64.sub".to_string(),
            MirBinaryOp::Mul => "f64.mul".to_string(),
            MirBinaryOp::Div => "f64.div".to_string(),
            MirBinaryOp::Mod => panic!("WAT backend does not support f64 modulo"),
        };
    }
    let wasm = wasm_type(type_node);
    match op {
        MirBinaryOp::Add => format!("{wasm}.add"),
        MirBinaryOp::Sub => format!("{wasm}.sub"),
        MirBinaryOp::Mul => format!("{wasm}.mul"),
        MirBinaryOp::Div if is_unsigned_integer_type(type_node) => format!("{wasm}.div_u"),
        MirBinaryOp::Div => format!("{wasm}.div_s"),
        MirBinaryOp::Mod if is_unsigned_integer_type(type_node) => format!("{wasm}.rem_u"),
        MirBinaryOp::Mod => format!("{wasm}.rem_s"),
    }
}

pub(super) fn wat_compare_instruction(op: MirCompareOp, type_node: &MirType) -> String {
    if is_f64_type(type_node) {
        return match op {
            MirCompareOp::Eq => "f64.eq",
            MirCompareOp::Ne => "f64.ne",
            MirCompareOp::Lt => "f64.lt",
            MirCompareOp::Le => "f64.le",
            MirCompareOp::Gt => "f64.gt",
            MirCompareOp::Ge => "f64.ge",
        }
        .to_string();
    }
    let wasm = wasm_type(type_node);
    match op {
        MirCompareOp::Eq => format!("{wasm}.eq"),
        MirCompareOp::Ne => format!("{wasm}.ne"),
        MirCompareOp::Lt if is_unsigned_integer_type(type_node) => format!("{wasm}.lt_u"),
        MirCompareOp::Lt => format!("{wasm}.lt_s"),
        MirCompareOp::Le if is_unsigned_integer_type(type_node) => format!("{wasm}.le_u"),
        MirCompareOp::Le => format!("{wasm}.le_s"),
        MirCompareOp::Gt if is_unsigned_integer_type(type_node) => format!("{wasm}.gt_u"),
        MirCompareOp::Gt => format!("{wasm}.gt_s"),
        MirCompareOp::Ge if is_unsigned_integer_type(type_node) => format!("{wasm}.ge_u"),
        MirCompareOp::Ge => format!("{wasm}.ge_s"),
    }
}

pub(super) fn wasm_type(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => "i32",
        MirType::Slice(_) => unreachable!("logical slices must be lowered to paired WASM values"),
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => "i64",
        MirType::Primitive(MirPrimitiveTypeName::F64) => "f64",
        MirType::Struct(_) => panic!("struct values are not WASM scalar values"),
        MirType::Void => panic!("void is not a WASM scalar value"),
    }
}

pub(super) fn block_index(function: &MirFunction, label: &str) -> usize {
    function
        .blocks
        .iter()
        .position(|block| block.label == label)
        .unwrap_or_else(|| panic!("unknown WAT block label {label}"))
}
