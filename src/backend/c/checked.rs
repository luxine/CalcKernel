use std::collections::HashSet;

use crate::*;

use super::super::{
    collect_temps, instruction_target, is_f64_type, is_signed_integer_type,
    is_unsigned_integer_type, signed_min_constant, value_type,
};
use super::{emit::*, layout::*};

pub(super) fn emit_planned_status_declarations(out: &mut String, enabled: bool) {
    if !enabled {
        return;
    }
    out.push_str(
        "typedef int32_t CK_Status;\n\n\
         #define CK_OK ((CK_Status)0)\n\
         #define CK_ERR_OVERFLOW ((CK_Status)1)\n\
         #define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n\
         #define CK_ERR_NULL_POINTER ((CK_Status)3)\n\
         #define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n\n",
    );
}

pub(super) fn planned_checked_c_unary_lines(
    target: &MirValue,
    op: MirUnaryOp,
    operand: &MirValue,
    context: &PlannedCFunctionContext<'_>,
) -> Vec<String> {
    let target_text = context.lvalue(target);
    let operand_text = context.value(operand);
    match op {
        MirUnaryOp::Not => vec![format!("{target_text} = !{operand_text};")],
        MirUnaryOp::Neg if is_f64_type(value_type(target)) => {
            vec![format!("{target_text} = -{operand_text};")]
        }
        MirUnaryOp::Neg if is_unsigned_integer_type(value_type(target)) => vec![
            format!(
                "if (__builtin_sub_overflow(({})0, {operand_text}, &{target_text})) {{",
                context.plan.type_name(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
        ],
        MirUnaryOp::Neg => vec![
            format!(
                "if ({operand_text} == {}) {{",
                signed_min_constant(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
            format!("{target_text} = -{operand_text};"),
        ],
    }
}

pub(super) fn planned_checked_c_binary_lines(
    target: &MirValue,
    op: MirBinaryOp,
    left: &MirValue,
    right: &MirValue,
    context: &PlannedCFunctionContext<'_>,
) -> Vec<String> {
    let target_text = context.lvalue(target);
    let left_text = context.value(left);
    let right_text = context.value(right);
    if is_f64_type(value_type(target)) {
        return vec![format!(
            "{target_text} = {left_text} {} {right_text};",
            c_binary_op(op)
        )];
    }
    match op {
        MirBinaryOp::Add => checked_c_overflow_builtin(
            "__builtin_add_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Sub => checked_c_overflow_builtin(
            "__builtin_sub_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Mul => checked_c_overflow_builtin(
            "__builtin_mul_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Div | MirBinaryOp::Mod => {
            let mut lines = vec![
                format!("if ({right_text} == 0) {{"),
                "  return CK_ERR_DIV_BY_ZERO;".to_string(),
                "}".to_string(),
            ];
            if is_signed_integer_type(value_type(target)) {
                lines.extend([
                    format!(
                        "if ({left_text} == {} && {right_text} == -1) {{",
                        signed_min_constant(value_type(target))
                    ),
                    "  return CK_ERR_OVERFLOW;".to_string(),
                    "}".to_string(),
                ]);
            }
            lines.push(format!(
                "{target_text} = {left_text} {} {right_text};",
                c_binary_op(op)
            ));
            lines
        }
    }
}

pub(super) fn emit_checked_c_function(out: &mut String, function: &MirFunction, opt_level: u8) {
    out.push_str(&format!("{} {{\n", checked_c_signature(function)));
    let referenced_labels = collect_c_referenced_labels(function);
    let safe_unchecked_binary_targets = if opt_level >= 3 {
        collect_safe_checked_induction_binary_targets(function)
    } else {
        HashSet::new()
    };
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
    if function_has_call(function) {
        out.push_str("  CK_Status ik_status;\n");
    }
    if !function.locals.is_empty() || !seen_temps.is_empty() || function_has_call(function) {
        out.push('\n');
    }

    if !matches!(function.return_type, MirType::Void) {
        out.push_str("  if (ck_return == NULL) {\n");
        out.push_str("    return CK_ERR_NULL_POINTER;\n");
        out.push_str("  }\n");
        if !function.blocks.is_empty() {
            out.push('\n');
        }
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if referenced_labels.contains(&block.label) {
            out.push_str(&format!("{}:\n", block.label));
        }
        for instruction in &block.instructions {
            for line in emit_checked_c_instruction(instruction, &safe_unchecked_binary_targets) {
                out.push_str("  ");
                out.push_str(&line);
                out.push('\n');
            }
        }
        for line in emit_checked_c_terminator(&block.terminator) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
}

pub(super) fn checked_c_signature(function: &MirFunction) -> String {
    let prefix = if function.exported { "" } else { "static " };
    let mut params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>();
    if !matches!(function.return_type, MirType::Void) {
        params.push(format!("{}* ck_return", c_type(&function.return_type)));
    }
    format!("{prefix}CK_Status {}({})", function.name, params.join(", "))
}

pub(super) fn c_export_signature_checked(function: &MirFunction) -> String {
    let mut params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>();
    if !matches!(function.return_type, MirType::Void) {
        params.push(format!("{}* ck_return", c_type(&function.return_type)));
    }
    format!("CK_Status {}({})", function.name, params.join(", "))
}

pub(super) fn emit_checked_c_instruction(
    instruction: &MirInstruction,
    safe_unchecked_binary_targets: &HashSet<String>,
) -> Vec<String> {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            vec![format!("{} = {};", c_value_lvalue(target), value)]
        }
        MirInstruction::ConstFloat { target, value } => {
            vec![format!("{} = {};", c_value_lvalue(target), value)]
        }
        MirInstruction::ConstBool { target, value } => {
            vec![format!(
                "{} = {};",
                c_value_lvalue(target),
                if *value { "true" } else { "false" }
            )]
        }
        MirInstruction::Move { target, value } => {
            vec![format!("{} = {};", c_value_lvalue(target), c_value(value))]
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => vec![format!(
            "{} = {} {} {};",
            c_value_lvalue(target),
            c_value(left),
            c_compare_op(*op),
            c_value(right)
        )],
        MirInstruction::Cast { target, op, value } => {
            let cast = match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => "double",
            };
            vec![format!(
                "{} = ({cast}){};",
                c_value_lvalue(target),
                c_value(value)
            )]
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => checked_c_unary_lines(target, *op, operand),
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            if safe_unchecked_binary_targets.contains(&c_value_identity(target)) {
                return vec![format!(
                    "{} = {} {} {};",
                    c_value_lvalue(target),
                    c_value(left),
                    c_binary_op(*op),
                    c_value(right)
                )];
            }
            checked_c_binary_lines(target, *op, left, right)
        }
        MirInstruction::Address { target, place } => {
            vec![format!("{} = &{};", c_value_lvalue(target), c_place(place))]
        }
        MirInstruction::Load { target, place } => {
            vec![format!("{} = {};", c_value_lvalue(target), c_place(place))]
        }
        MirInstruction::Store { place, value } => {
            vec![format!("{} = {};", c_place(place), c_value(value))]
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let mut call_args = args.iter().map(c_value).collect::<Vec<_>>();
            if let Some(target) = target {
                call_args.push(format!("&{}", c_value_lvalue(target)));
            }
            vec![
                format!("ik_status = {function_name}({});", call_args.join(", ")),
                "if (ik_status != CK_OK) {".to_string(),
                "  return ik_status;".to_string(),
                "}".to_string(),
            ]
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. } => {
            unreachable!("slice modules must use the planned C emitter")
        }
    }
}

pub(super) fn checked_c_unary_lines(
    target: &MirValue,
    op: MirUnaryOp,
    operand: &MirValue,
) -> Vec<String> {
    let target_text = c_value_lvalue(target);
    let operand_text = c_value(operand);
    match op {
        MirUnaryOp::Not => vec![format!("{target_text} = !{operand_text};")],
        MirUnaryOp::Neg if is_f64_type(value_type(target)) => {
            vec![format!("{target_text} = -{operand_text};")]
        }
        MirUnaryOp::Neg if is_unsigned_integer_type(value_type(target)) => vec![
            format!(
                "if (__builtin_sub_overflow(({})0, {operand_text}, &{target_text})) {{",
                c_type(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
        ],
        MirUnaryOp::Neg => vec![
            format!(
                "if ({operand_text} == {}) {{",
                signed_min_constant(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
            format!("{target_text} = -{operand_text};"),
        ],
    }
}

pub(super) fn checked_c_binary_lines(
    target: &MirValue,
    op: MirBinaryOp,
    left: &MirValue,
    right: &MirValue,
) -> Vec<String> {
    let target_text = c_value_lvalue(target);
    let left_text = c_value(left);
    let right_text = c_value(right);
    if is_f64_type(value_type(target)) {
        return vec![format!(
            "{target_text} = {left_text} {} {right_text};",
            c_binary_op(op)
        )];
    }

    match op {
        MirBinaryOp::Add => checked_c_overflow_builtin(
            "__builtin_add_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Sub => checked_c_overflow_builtin(
            "__builtin_sub_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Mul => checked_c_overflow_builtin(
            "__builtin_mul_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Div | MirBinaryOp::Mod => {
            let mut lines = vec![
                format!("if ({right_text} == 0) {{"),
                "  return CK_ERR_DIV_BY_ZERO;".to_string(),
                "}".to_string(),
            ];
            if is_signed_integer_type(value_type(target)) {
                lines.push(format!(
                    "if ({left_text} == {} && {right_text} == -1) {{",
                    signed_min_constant(value_type(target))
                ));
                lines.push("  return CK_ERR_OVERFLOW;".to_string());
                lines.push("}".to_string());
            }
            lines.push(format!(
                "{target_text} = {left_text} {} {right_text};",
                c_binary_op(op)
            ));
            lines
        }
    }
}

pub(super) fn checked_c_overflow_builtin(
    builtin: &str,
    left: &str,
    right: &str,
    target: &str,
) -> Vec<String> {
    vec![
        format!("if ({builtin}({left}, {right}, &{target})) {{"),
        "  return CK_ERR_OVERFLOW;".to_string(),
        "}".to_string(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BodyIncrementCandidate {
    binary: MirInstruction,
    move_instruction: MirInstruction,
}

pub(super) fn collect_safe_checked_induction_binary_targets(
    function: &MirFunction,
) -> HashSet<String> {
    let mut safe_targets = HashSet::new();

    for header in &function.blocks {
        let MirTerminator::Branch {
            condition,
            then_label,
            ..
        } = &header.terminator
        else {
            continue;
        };
        let Some(MirInstruction::Compare {
            op: MirCompareOp::Lt,
            left:
                induction @ MirValue::Local {
                    type_node: induction_type,
                    ..
                },
            right: limit,
            ..
        }) = find_c_value_def(header, condition)
        else {
            continue;
        };
        if !is_i32_or_u32_type(induction_type)
            || value_type(limit) != induction_type
            || !is_stable_limit_value(limit)
        {
            continue;
        }

        let Some(body) = function
            .blocks
            .iter()
            .find(|block| block.label == *then_label)
        else {
            continue;
        };
        if !matches!(&body.terminator, MirTerminator::Jump { label } if label == &header.label) {
            continue;
        }

        let Some(candidate) = find_body_increment_candidate(body, induction, limit) else {
            continue;
        };
        let Some(init) = find_zero_initialization_before(function, header, induction) else {
            continue;
        };
        if has_unexpected_assignments(
            function,
            induction,
            &[init, candidate.move_instruction.clone()],
        ) {
            continue;
        }

        if let MirInstruction::Binary { target, .. } = candidate.binary {
            safe_targets.insert(c_value_identity(&target));
        }
    }

    safe_targets
}

pub(super) fn find_c_value_def<'block>(
    block: &'block MirBlock,
    value: &MirValue,
) -> Option<&'block MirInstruction> {
    if !matches!(value, MirValue::Temp { .. }) {
        return None;
    }
    let identity = c_value_identity(value);
    block.instructions.iter().find(|instruction| {
        instruction_target(instruction).is_some_and(|target| c_value_identity(target) == identity)
    })
}

pub(super) fn find_body_increment_candidate(
    body: &MirBlock,
    induction: &MirValue,
    limit: &MirValue,
) -> Option<BodyIncrementCandidate> {
    let mut int_constants = std::collections::HashMap::new();
    let mut candidate_binary: Option<MirInstruction> = None;
    let mut candidate_move: Option<MirInstruction> = None;

    for instruction in &body.instructions {
        if let MirInstruction::ConstInt { target, value } = instruction {
            int_constants.insert(c_value_identity(target), value.clone());
            continue;
        }

        if assigns_c_value(instruction, limit) {
            return None;
        }

        if let MirInstruction::Binary {
            target: _,
            op: MirBinaryOp::Add,
            left,
            right,
        } = instruction
            && same_c_value(left, induction)
            && int_constants
                .get(&c_value_identity(right))
                .is_some_and(|value| value == "1")
        {
            if candidate_binary.is_some() {
                return None;
            }
            candidate_binary = Some((*instruction).clone());
            continue;
        }

        if let MirInstruction::Move { target, value } = instruction
            && same_c_value(target, induction)
        {
            let Some(MirInstruction::Binary {
                target: binary_target,
                ..
            }) = &candidate_binary
            else {
                return None;
            };
            if !same_c_value(value, binary_target) || candidate_move.is_some() {
                return None;
            }
            candidate_move = Some((*instruction).clone());
            continue;
        }

        if assigns_c_value(instruction, induction) {
            return None;
        }
    }

    Some(BodyIncrementCandidate {
        binary: candidate_binary?,
        move_instruction: candidate_move?,
    })
}

pub(super) fn find_zero_initialization_before(
    function: &MirFunction,
    header: &MirBlock,
    induction: &MirValue,
) -> Option<MirInstruction> {
    let mut int_constants = std::collections::HashMap::new();

    for block in &function.blocks {
        if block.label == header.label {
            return None;
        }

        for instruction in &block.instructions {
            if let MirInstruction::ConstInt { target, value } = instruction {
                int_constants.insert(c_value_identity(target), value.clone());
                continue;
            }

            if let MirInstruction::Move { target, value } = instruction
                && same_c_value(target, induction)
            {
                return int_constants
                    .get(&c_value_identity(value))
                    .is_some_and(|value| value == "0")
                    .then(|| instruction.clone());
            }

            if assigns_c_value(instruction, induction) {
                return None;
            }
        }

        if matches!(&block.terminator, MirTerminator::Jump { label } if label == &header.label) {
            return None;
        }
    }

    None
}

pub(super) fn has_unexpected_assignments(
    function: &MirFunction,
    value: &MirValue,
    allowed: &[MirInstruction],
) -> bool {
    function.blocks.iter().any(|block| {
        block.instructions.iter().any(|instruction| {
            assigns_c_value(instruction, value)
                && !allowed.iter().any(|allowed| allowed == instruction)
        })
    })
}

pub(super) fn assigns_c_value(instruction: &MirInstruction, value: &MirValue) -> bool {
    instruction_target(instruction).is_some_and(|target| same_c_value(target, value))
}

pub(super) fn same_c_value(left: &MirValue, right: &MirValue) -> bool {
    c_value_identity(left) == c_value_identity(right)
}

pub(super) fn c_value_identity(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } => format!("param:{name}"),
        MirValue::Local { name, .. } => format!("local:{name}"),
        MirValue::Temp { name, .. } => format!("temp:{name}"),
        MirValue::ConstInt { text, type_node } => {
            format!("const_int:{text}:{}", c_type_identity(type_node))
        }
        MirValue::ConstFloat { text, type_node } => {
            format!("const_float:{text}:{}", c_type_identity(type_node))
        }
        MirValue::ConstBool { value, .. } => format!("const_bool:{value}"),
    }
}

pub(super) fn c_type_identity(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => c_primitive_type_identity(*name).to_string(),
        MirType::Pointer(element_type) => format!("ptr<{}>", c_type_identity(element_type)),
        MirType::Slice(element_type) => format!("slice<{}>", c_type_identity(element_type)),
        MirType::Struct(name) => format!("struct:{name}"),
        MirType::Void => "void".to_string(),
    }
}

pub(super) fn c_primitive_type_identity(name: MirPrimitiveTypeName) -> &'static str {
    match name {
        MirPrimitiveTypeName::I32 => "i32",
        MirPrimitiveTypeName::I64 => "i64",
        MirPrimitiveTypeName::U32 => "u32",
        MirPrimitiveTypeName::U64 => "u64",
        MirPrimitiveTypeName::F64 => "f64",
        MirPrimitiveTypeName::Bool => "bool",
    }
}

pub(super) fn is_i32_or_u32_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
    )
}

pub(super) fn is_stable_limit_value(value: &MirValue) -> bool {
    matches!(value, MirValue::Param { .. } | MirValue::Local { .. })
}

pub(super) fn emit_checked_c_terminator(terminator: &MirTerminator) -> Vec<String> {
    match terminator {
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || vec!["return CK_OK;".to_string()],
            |value| {
                vec![
                    format!("*ck_return = {};", c_value(value)),
                    "return CK_OK;".to_string(),
                ]
            },
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
