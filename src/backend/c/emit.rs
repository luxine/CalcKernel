use std::collections::HashSet;

use crate::*;

use super::super::collect_temps;
use super::{checked::*, layout::*, names::*, options::*};

#[must_use]
pub fn emit_c_module(module: &MirModule, options: EmitCOptions) -> String {
    if use_planned_c_emitter(module, options) {
        return emit_planned_c_module(module, options, None);
    }
    let mut out = String::new();
    out.push_str("#include <stdbool.h>\n#include <stdint.h>\n");
    if options.overflow_mode == OverflowMode::Checked {
        out.push_str(
            "#include <stddef.h>\n\n\
             typedef int32_t CK_Status;\n\n\
             #define CK_OK ((CK_Status)0)\n\
             #define CK_ERR_OVERFLOW ((CK_Status)1)\n\
             #define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n\
             #define CK_ERR_NULL_POINTER ((CK_Status)3)\n\
             #define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n\n",
        );
    } else {
        out.push('\n');
    }

    for struct_info in &module.structs {
        out.push_str(&format!("typedef struct {} {{\n", struct_info.name));
        for field in &struct_info.fields {
            out.push_str(&format!("  {} {};\n", c_type(&field.type_node), field.name));
        }
        out.push_str(&format!("}} {};\n\n", struct_info.name));
    }

    for function in &module.functions {
        let signature = if options.overflow_mode == OverflowMode::Checked {
            checked_c_signature(function)
        } else {
            c_signature(function)
        };
        out.push_str(&format!("{signature};\n"));
    }
    if !module.functions.is_empty() {
        out.push('\n');
    }

    for (index, function) in module.functions.iter().enumerate() {
        if options.overflow_mode == OverflowMode::Checked {
            emit_checked_c_function(&mut out, function, options.opt_level);
        } else {
            emit_c_function(&mut out, function);
        }
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

#[must_use]
pub fn emit_c_module_with_header(
    module: &MirModule,
    options: EmitCOptions,
    header_file_name: &str,
) -> String {
    if use_planned_c_emitter(module, options) {
        return emit_planned_c_module(module, options, Some(header_file_name));
    }
    let mut out = String::new();
    out.push_str(&format!(
        "#include \"{}\"\n\n",
        escape_c_include_path(header_file_name)
    ));

    for (index, function) in module.functions.iter().enumerate() {
        if options.overflow_mode == OverflowMode::Checked {
            emit_checked_c_function(&mut out, function, options.opt_level);
        } else {
            emit_c_function(&mut out, function);
        }
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

#[must_use]
pub fn emit_c_header(module: &MirModule, options: EmitCOptions) -> String {
    if use_planned_c_emitter(module, options) {
        return emit_planned_c_header(module, options);
    }
    let mut out = String::new();
    out.push_str("#pragma once\n\n");
    out.push_str("#include <stdint.h>\n#include <stdbool.h>\n");
    if options.overflow_mode == OverflowMode::Checked {
        out.push_str("#include <stddef.h>\n");
    }
    out.push_str(
        "\n#if defined(_WIN32) || defined(__CYGWIN__)\n  #ifdef CK_BUILD_DLL\n    #define CK_API __declspec(dllexport)\n  #else\n    #define CK_API __declspec(dllimport)\n  #endif\n#else\n  #define CK_API __attribute__((visibility(\"default\")))\n#endif\n",
    );
    if options.overflow_mode == OverflowMode::Checked {
        out.push_str(
            "\ntypedef int32_t CK_Status;\n\n#define CK_OK ((CK_Status)0)\n#define CK_ERR_OVERFLOW ((CK_Status)1)\n#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n#define CK_ERR_NULL_POINTER ((CK_Status)3)\n#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n",
        );
    }
    out.push_str("\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n");

    for struct_info in &module.structs {
        out.push_str(&format!("\ntypedef struct {} {{\n", struct_info.name));
        for field in &struct_info.fields {
            out.push_str(&format!("  {} {};\n", c_type(&field.type_node), field.name));
        }
        out.push_str(&format!("}} {};\n", struct_info.name));
    }

    for function in module.functions.iter().filter(|function| function.exported) {
        let signature = if options.overflow_mode == OverflowMode::Checked {
            c_export_signature_checked(function)
        } else {
            c_export_signature(function)
        };
        out.push_str(&format!("\nCK_API {signature};\n"));
    }

    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n");
    out
}

pub(super) fn emit_planned_c_module(
    module: &MirModule,
    options: EmitCOptions,
    header_file_name: Option<&str>,
) -> String {
    let plan = CModulePlan::new(module, options);
    let mut out = String::new();
    if let Some(header_file_name) = header_file_name {
        out.push_str(&format!(
            "#include \"{}\"\n\n",
            escape_c_include_path(header_file_name)
        ));
        let internal = module
            .functions
            .iter()
            .filter(|function| !function.exported)
            .collect::<Vec<_>>();
        for function in &internal {
            out.push_str(&format!(
                "{};\n",
                planned_c_signature(function, &plan, false)
            ));
        }
        if !internal.is_empty() {
            out.push('\n');
        }
    } else {
        out.push_str("#include <stdbool.h>\n#include <stdint.h>\n");
        if plan.status_abi {
            out.push_str("#include <stddef.h>\n");
        }
        out.push('\n');
        emit_planned_status_declarations(&mut out, plan.status_abi);
        emit_planned_type_declarations(&mut out, module, &plan);
        for function in &module.functions {
            out.push_str(&format!(
                "{};\n",
                planned_c_signature(function, &plan, false)
            ));
        }
        if !module.functions.is_empty() {
            out.push('\n');
        }
    }

    for (index, function) in module.functions.iter().enumerate() {
        emit_planned_c_function(&mut out, function, &plan, options);
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

pub(super) fn emit_planned_c_header(module: &MirModule, options: EmitCOptions) -> String {
    let plan = CModulePlan::new(module, options);
    let mut out = String::new();
    out.push_str("#pragma once\n\n#include <stdint.h>\n#include <stdbool.h>\n");
    if plan.status_abi {
        out.push_str("#include <stddef.h>\n");
    }
    out.push_str(
        "\n#if defined(_WIN32) || defined(__CYGWIN__)\n  #ifdef CK_BUILD_DLL\n    #define CK_API __declspec(dllexport)\n  #else\n    #define CK_API __declspec(dllimport)\n  #endif\n#else\n  #define CK_API __attribute__((visibility(\"default\")))\n#endif\n\n",
    );
    emit_planned_status_declarations(&mut out, plan.status_abi);
    emit_planned_type_declarations(&mut out, module, &plan);
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    for function in module.functions.iter().filter(|function| function.exported) {
        out.push_str(&format!(
            "\nCK_API {};\n",
            planned_c_signature(function, &plan, true)
        ));
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n");
    out
}

pub(super) fn planned_c_signature(
    function: &MirFunction,
    plan: &CModulePlan,
    exported_header: bool,
) -> String {
    let function_plan = plan.function(&function.name);
    let prefix = if !exported_header && !function.exported {
        "static "
    } else {
        ""
    };
    let mut params = Vec::new();
    for param in &function.params {
        if let MirType::Slice(element_type) = &param.type_node {
            let (data_name, len_name) = function_plan
                .slice_params
                .get(&param.name)
                .expect("slice parameter must have physical names");
            params.push(format!("{}* {data_name}", plan.type_name(element_type)));
            params.push(format!("uint32_t {len_name}"));
        } else {
            params.push(format!(
                "{} {}",
                plan.type_name(&param.type_node),
                param.name
            ));
        }
    }
    if plan.status_abi && !matches!(function.return_type, MirType::Void) {
        params.push(format!(
            "{}* {}",
            plan.type_name(&function.return_type),
            function_plan.return_pointer
        ));
    }
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    if plan.status_abi {
        format!("{prefix}CK_Status {}({params})", function.name)
    } else {
        format!(
            "{prefix}{} {}({params})",
            plan.type_name(&function.return_type),
            function.name
        )
    }
}

pub(super) fn emit_planned_c_function(
    out: &mut String,
    function: &MirFunction,
    plan: &CModulePlan,
    options: EmitCOptions,
) {
    let function_plan = plan.function(&function.name);
    let context = PlannedCFunctionContext {
        plan,
        function_plan,
        options,
    };
    out.push_str(&format!(
        "{} {{\n",
        planned_c_signature(function, plan, false)
    ));
    let referenced_labels = collect_c_referenced_labels(function);
    let safe_unchecked_binary_targets =
        if options.overflow_mode == OverflowMode::Checked && options.opt_level >= 3 {
            collect_safe_checked_induction_binary_targets(function)
        } else {
            HashSet::new()
        };

    for param in &function.params {
        if matches!(param.type_node, MirType::Slice(_)) {
            out.push_str(&format!(
                "  {} {};\n",
                plan.type_name(&param.type_node),
                param.name
            ));
        }
    }
    for local in &function.locals {
        out.push_str(&format!(
            "  {} {};\n",
            plan.type_name(&local.type_node),
            local.name
        ));
    }
    for (name, type_node) in collect_temps(function) {
        out.push_str(&format!(
            "  {} {};\n",
            plan.type_name(&type_node),
            function_plan
                .temp_names
                .get(&name)
                .expect("temp must have planned name")
        ));
    }
    if plan.status_abi && function_has_call(function) {
        out.push_str(&format!("  CK_Status {};\n", function_plan.status_local));
    }
    if !function.params.is_empty()
        || !function.locals.is_empty()
        || !collect_temps(function).is_empty()
        || (plan.status_abi && function_has_call(function))
    {
        out.push('\n');
    }

    if plan.status_abi && !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "  if ({} == NULL) {{\n    return CK_ERR_NULL_POINTER;\n  }}\n",
            function_plan.return_pointer
        ));
        if !function.blocks.is_empty()
            || function
                .params
                .iter()
                .any(|param| matches!(param.type_node, MirType::Slice(_)))
        {
            out.push('\n');
        }
    }

    let slice_params = function
        .params
        .iter()
        .filter(|param| matches!(param.type_node, MirType::Slice(_)))
        .collect::<Vec<_>>();
    for param in &slice_params {
        let (data_name, len_name) = function_plan
            .slice_params
            .get(&param.name)
            .expect("slice parameter must have physical names");
        out.push_str(&format!("  {}.data = {data_name};\n", param.name));
        out.push_str(&format!("  {}.len = {len_name};\n", param.name));
    }
    if !slice_params.is_empty() && !function.blocks.is_empty() {
        out.push('\n');
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if referenced_labels.contains(&block.label) {
            out.push_str(&format!("{}:\n", block.label));
        }
        for instruction in &block.instructions {
            for line in
                planned_c_instruction_lines(instruction, &context, &safe_unchecked_binary_targets)
            {
                out.push_str("  ");
                out.push_str(&line);
                out.push('\n');
            }
        }
        for line in planned_c_terminator_lines(&block.terminator, &context) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
}

pub(super) fn planned_c_instruction_lines(
    instruction: &MirInstruction,
    context: &PlannedCFunctionContext<'_>,
    safe_unchecked_binary_targets: &HashSet<String>,
) -> Vec<String> {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            vec![format!("{} = {value};", context.lvalue(target))]
        }
        MirInstruction::ConstFloat { target, value } => {
            vec![format!("{} = {value};", context.lvalue(target))]
        }
        MirInstruction::ConstBool { target, value } => vec![format!(
            "{} = {};",
            context.lvalue(target),
            if *value { "true" } else { "false" }
        )],
        MirInstruction::Move { target, value } => vec![format!(
            "{} = {};",
            context.lvalue(target),
            context.value(value)
        )],
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            if context.options.overflow_mode == OverflowMode::Checked
                && !safe_unchecked_binary_targets.contains(&c_value_identity(target))
            {
                planned_checked_c_binary_lines(target, *op, left, right, context)
            } else {
                vec![format!(
                    "{} = {} {} {};",
                    context.lvalue(target),
                    context.value(left),
                    c_binary_op(*op),
                    context.value(right)
                )]
            }
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            if context.options.overflow_mode == OverflowMode::Checked {
                planned_checked_c_unary_lines(target, *op, operand, context)
            } else {
                vec![format!(
                    "{} = {}{};",
                    context.lvalue(target),
                    c_unary_op(*op),
                    context.value(operand)
                )]
            }
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => vec![format!(
            "{} = {} {} {};",
            context.lvalue(target),
            context.value(left),
            c_compare_op(*op),
            context.value(right)
        )],
        MirInstruction::Cast { target, op, value } => {
            let cast = match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => "double",
            };
            vec![format!(
                "{} = ({cast}){};",
                context.lvalue(target),
                context.value(value)
            )]
        }
        MirInstruction::Address { target, place } => {
            let emitted = context.place(place);
            let mut lines = emitted.preludes;
            lines.push(format!(
                "{} = &{};",
                context.lvalue(target),
                emitted.expression
            ));
            lines
        }
        MirInstruction::Load { target, place } => {
            let emitted = context.place(place);
            let mut lines = emitted.preludes;
            lines.push(format!(
                "{} = {};",
                context.lvalue(target),
                emitted.expression
            ));
            lines
        }
        MirInstruction::Store { place, value } => {
            let emitted = context.place(place);
            let mut lines = emitted.preludes;
            lines.push(format!(
                "{} = {};",
                emitted.expression,
                context.value(value)
            ));
            lines
        }
        MirInstruction::MakeSlice { target, data, len } => vec![
            format!("{}.data = {};", context.lvalue(target), context.value(data)),
            format!("{}.len = {};", context.lvalue(target), context.value(len)),
        ],
        MirInstruction::SliceData { target, slice } => vec![format!(
            "{} = {}.data;",
            context.lvalue(target),
            context.value(slice)
        )],
        MirInstruction::SliceLen { target, slice } => vec![format!(
            "{} = {}.len;",
            context.lvalue(target),
            context.value(slice)
        )],
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let target = context.lvalue(target);
            let slice = context.value(slice);
            let start = context.value(start);
            let end = context.value(end);
            let mut lines = Vec::new();
            if context.options.bounds_mode == BoundsMode::Checked {
                lines.extend([
                    format!("if ({start} > {end} || {end} > {slice}.len) {{"),
                    "  return CK_ERR_OUT_OF_BOUNDS;".to_string(),
                    "}".to_string(),
                ]);
            }
            lines.push(format!(
                "{target}.data = ({start} == 0 ? {slice}.data : {slice}.data + {start});"
            ));
            lines.push(format!("{target}.len = {end} - {start};"));
            lines
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let mut args = context.call_args(args);
            if context.plan.status_abi {
                if let Some(target) = target {
                    args.push(format!("&{}", context.lvalue(target)));
                }
                vec![
                    format!(
                        "{} = {function_name}({});",
                        context.function_plan.status_local,
                        args.join(", ")
                    ),
                    format!("if ({} != CK_OK) {{", context.function_plan.status_local),
                    format!("  return {};", context.function_plan.status_local),
                    "}".to_string(),
                ]
            } else {
                let call = format!("{function_name}({})", args.join(", "));
                target.as_ref().map_or_else(
                    || vec![format!("{call};")],
                    |target| vec![format!("{} = {call};", context.lvalue(target))],
                )
            }
        }
    }
}

pub(super) fn planned_c_terminator_lines(
    terminator: &MirTerminator,
    context: &PlannedCFunctionContext<'_>,
) -> Vec<String> {
    match terminator {
        MirTerminator::Return { value } if context.plan.status_abi => value.as_ref().map_or_else(
            || vec!["return CK_OK;".to_string()],
            |value| {
                vec![
                    format!(
                        "*{} = {};",
                        context.function_plan.return_pointer,
                        context.value(value)
                    ),
                    "return CK_OK;".to_string(),
                ]
            },
        ),
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || vec!["return;".to_string()],
            |value| vec![format!("return {};", context.value(value))],
        ),
        MirTerminator::Jump { label } => vec![format!("goto {label};")],
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => vec![
            format!("if ({}) {{", context.value(condition)),
            format!("  goto {then_label};"),
            "} else {".to_string(),
            format!("  goto {else_label};"),
            "}".to_string(),
        ],
    }
}

pub(super) fn emit_c_function(out: &mut String, function: &MirFunction) {
    out.push_str(&format!("{} {{\n", c_signature(function)));
    let referenced_labels = collect_c_referenced_labels(function);
    for local in &function.locals {
        out.push_str(&format!("  {} {};\n", c_type(&local.type_node), local.name));
    }
    let mut seen_temps = HashSet::new();
    for temp in collect_temps(function) {
        if seen_temps.insert(temp.0.clone()) {
            out.push_str(&format!(
                "  {} {};\n",
                c_type(&temp.1),
                c_temp_name(&temp.0)
            ));
        }
    }
    if (!function.locals.is_empty() || !seen_temps.is_empty()) && !function.blocks.is_empty() {
        out.push('\n');
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if referenced_labels.contains(&block.label) {
            out.push_str(&format!("{}:\n", block.label));
        }
        for instruction in &block.instructions {
            out.push_str("  ");
            out.push_str(&emit_c_instruction(instruction));
            out.push('\n');
        }
        for line in emit_c_terminator(&block.terminator) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
}

pub(super) fn c_signature(function: &MirFunction) -> String {
    let prefix = if function.exported { "" } else { "static " };
    let params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{prefix}{} {}({})",
        c_type(&function.return_type),
        function.name,
        params
    )
}

pub(super) fn c_export_signature(function: &MirFunction) -> String {
    let params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} {}({})",
        c_type(&function.return_type),
        function.name,
        params
    )
}

pub(super) fn emit_c_instruction(instruction: &MirInstruction) -> String {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            format!("{} = {};", c_value_lvalue(target), value)
        }
        MirInstruction::ConstFloat { target, value } => {
            format!("{} = {};", c_value_lvalue(target), value)
        }
        MirInstruction::ConstBool { target, value } => {
            format!(
                "{} = {};",
                c_value_lvalue(target),
                if *value { "true" } else { "false" }
            )
        }
        MirInstruction::Move { target, value } => {
            format!("{} = {};", c_value_lvalue(target), c_value(value))
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => format!(
            "{} = {} {} {};",
            c_value_lvalue(target),
            c_value(left),
            c_binary_op(*op),
            c_value(right)
        ),
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => format!(
            "{} = {}{};",
            c_value_lvalue(target),
            c_unary_op(*op),
            c_value(operand)
        ),
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => format!(
            "{} = {} {} {};",
            c_value_lvalue(target),
            c_value(left),
            c_compare_op(*op),
            c_value(right)
        ),
        MirInstruction::Cast { target, op, value } => {
            let cast = match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => "double",
            };
            format!("{} = ({cast}){};", c_value_lvalue(target), c_value(value))
        }
        MirInstruction::Address { target, place } => {
            format!("{} = &{};", c_value_lvalue(target), c_place(place))
        }
        MirInstruction::Load { target, place } => {
            format!("{} = {};", c_value_lvalue(target), c_place(place))
        }
        MirInstruction::Store { place, value } => {
            format!("{} = {};", c_place(place), c_value(value))
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let call = format!(
                "{}({})",
                function_name,
                args.iter().map(c_value).collect::<Vec<_>>().join(", ")
            );
            target.as_ref().map_or_else(
                || format!("{call};"),
                |target| format!("{} = {call};", c_value_lvalue(target)),
            )
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. } => {
            unreachable!("slice modules must use the planned C emitter")
        }
    }
}

pub(super) fn emit_c_terminator(terminator: &MirTerminator) -> Vec<String> {
    match terminator {
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || vec!["return;".to_string()],
            |value| vec![format!("return {};", c_value(value))],
        ),
        MirTerminator::Jump { label } => vec![format!("goto {label};")],
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => vec![
            format!("if ({}) {{", c_value(condition)),
            format!("  goto {then_label};"),
            "} else {".to_string(),
            format!("  goto {else_label};"),
            "}".to_string(),
        ],
    }
}

pub(super) fn collect_c_referenced_labels(function: &MirFunction) -> HashSet<String> {
    let mut labels = HashSet::new();
    for block in &function.blocks {
        match &block.terminator {
            MirTerminator::Jump { label } => {
                labels.insert(label.clone());
            }
            MirTerminator::Branch {
                then_label,
                else_label,
                ..
            } => {
                labels.insert(then_label.clone());
                labels.insert(else_label.clone());
            }
            MirTerminator::Return { .. } => {}
        }
    }
    labels
}

pub(super) fn function_has_call(function: &MirFunction) -> bool {
    function.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MirInstruction::Call { .. }))
    })
}

pub(super) fn c_value(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
        MirValue::Temp { name, .. } => c_temp_name(name),
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

pub(super) fn c_value_lvalue(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
        MirValue::Temp { name, .. } => c_temp_name(name),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("cannot assign to MIR constant")
        }
    }
}

pub(super) fn c_temp_name(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix('t')
        && !suffix.is_empty()
        && suffix.chars().all(|character| character.is_ascii_digit())
    {
        return format!("ik_tmp{suffix}");
    }

    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("ik_tmp_{sanitized}")
}

pub(super) fn c_place(place: &MirPlace) -> String {
    match place {
        MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => name.clone(),
        MirPlace::Deref { pointer, .. } => format!("(*{})", c_value(pointer)),
        MirPlace::Index { base, index, .. } => format!("{}[{}]", c_place(base), c_value(index)),
        MirPlace::SliceIndex { .. } => {
            unreachable!("slice modules must use the planned C emitter")
        }
        MirPlace::Field {
            base, field_name, ..
        } => match &**base {
            MirPlace::Deref { pointer, .. } => format!("{}->{field_name}", c_value(pointer)),
            _ => format!("{}.{}", c_place(base), field_name),
        },
    }
}

pub(super) fn c_type(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => "int32_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::I64) => "int64_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::U32) => "uint32_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::U64) => "uint64_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::Bool) => "bool".to_string(),
        MirType::Pointer(element_type) => format!("{}*", c_type(element_type)),
        MirType::Slice(_) => unreachable!("slice modules must use the planned C emitter"),
        MirType::Struct(name) => name.clone(),
        MirType::Void => "void".to_string(),
    }
}

pub(super) fn c_binary_op(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "+",
        MirBinaryOp::Sub => "-",
        MirBinaryOp::Mul => "*",
        MirBinaryOp::Div => "/",
        MirBinaryOp::Mod => "%",
    }
}

pub(super) fn c_compare_op(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "==",
        MirCompareOp::Ne => "!=",
        MirCompareOp::Lt => "<",
        MirCompareOp::Le => "<=",
        MirCompareOp::Gt => ">",
        MirCompareOp::Ge => ">=",
    }
}

pub(super) fn c_unary_op(op: MirUnaryOp) -> &'static str {
    match op {
        MirUnaryOp::Neg => "-",
        MirUnaryOp::Not => "!",
    }
}
