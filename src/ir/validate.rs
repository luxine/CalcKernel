use std::collections::HashMap;

use super::print::print_cast_op;
use super::*;

pub fn validate_mir_module(module: &MirModule) -> MirValidationResult {
    let mut ctx = ModuleValidationContext {
        functions: HashMap::new(),
        structs: HashMap::new(),
        errors: Vec::new(),
    };

    for struct_info in &module.structs {
        if ctx.structs.contains_key(&struct_info.name) {
            ctx.errors.push(MirValidationError {
                message: format!("Duplicate struct '{}'.", struct_info.name),
                function_name: None,
                block_label: None,
            });
        } else {
            ctx.structs.insert(struct_info.name.clone(), struct_info);
        }
        for field in &struct_info.fields {
            if contains_void_type(&field.type_node) {
                ctx.errors.push(MirValidationError {
                    message: format!(
                        "Void field '{}' is not allowed in struct '{}'.",
                        field.name, struct_info.name
                    ),
                    function_name: None,
                    block_label: None,
                });
            }
            if let Some(reason) = invalid_slice_type_reason(&field.type_node) {
                ctx.errors.push(MirValidationError {
                    message: format!(
                        "Invalid slice field '{}' in struct '{}': {reason}.",
                        field.name, struct_info.name
                    ),
                    function_name: None,
                    block_label: None,
                });
            }
        }
    }

    for function in &module.functions {
        if ctx.functions.contains_key(&function.name) {
            ctx.errors.push(MirValidationError {
                message: format!("Duplicate function '{}'.", function.name),
                function_name: Some(function.name.clone()),
                block_label: None,
            });
        } else {
            ctx.functions.insert(function.name.clone(), function);
        }
    }

    for function in &module.functions {
        validate_function(&mut ctx, function);
    }

    MirValidationResult { errors: ctx.errors }
}

struct ModuleValidationContext<'module> {
    functions: HashMap<String, &'module MirFunction>,
    structs: HashMap<String, &'module MirStruct>,
    errors: Vec<MirValidationError>,
}

struct FunctionValidationContext<'module, 'ctx> {
    functions: &'ctx HashMap<String, &'module MirFunction>,
    structs: &'ctx HashMap<String, &'module MirStruct>,
    function: &'module MirFunction,
    labels: HashMap<String, ()>,
    params: HashMap<String, MirType>,
    locals: HashMap<String, MirType>,
    temps: HashMap<String, MirType>,
    errors: &'ctx mut Vec<MirValidationError>,
}

fn validate_function(module_ctx: &mut ModuleValidationContext<'_>, function: &MirFunction) {
    let mut ctx = FunctionValidationContext {
        functions: &module_ctx.functions,
        structs: &module_ctx.structs,
        function,
        labels: HashMap::new(),
        params: HashMap::new(),
        locals: HashMap::new(),
        temps: HashMap::new(),
        errors: &mut module_ctx.errors,
    };

    if !matches!(function.return_type, MirType::Void) && contains_void_type(&function.return_type) {
        add_validation_error(
            &mut ctx,
            format!(
                "Function '{}' has invalid void-containing return type {}.",
                function.name,
                print_mir_type(&function.return_type)
            ),
            None,
        );
    }
    if let Some(reason) = invalid_slice_type_reason(&function.return_type) {
        add_validation_error(
            &mut ctx,
            format!(
                "Function '{}' has invalid slice return type: {reason}.",
                function.name
            ),
            None,
        );
    }
    if function.exported && matches!(function.return_type, MirType::Slice(_)) {
        add_validation_error(
            &mut ctx,
            format!(
                "Function '{}' has an exported slice return, which is not allowed.",
                function.name
            ),
            None,
        );
    }

    collect_params(&mut ctx);
    collect_locals(&mut ctx);
    collect_labels(&mut ctx);
    collect_temps(&mut ctx);

    if function.blocks.is_empty() {
        add_validation_error(
            &mut ctx,
            format!("Function '{}' has no entry block.", function.name),
            None,
        );
        return;
    }

    for block in &function.blocks {
        validate_block(&mut ctx, block);
    }
}

fn collect_params(ctx: &mut FunctionValidationContext<'_, '_>) {
    for param in &ctx.function.params {
        if contains_void_type(&param.type_node) {
            add_validation_error(
                ctx,
                format!(
                    "Void parameter '{}' is not allowed in function '{}'.",
                    param.name, ctx.function.name
                ),
                None,
            );
        }
        if let Some(reason) = invalid_slice_type_reason(&param.type_node) {
            add_validation_error(
                ctx,
                format!(
                    "Parameter '{}' in function '{}' has invalid slice type: {reason}.",
                    param.name, ctx.function.name
                ),
                None,
            );
        }
        if ctx.params.contains_key(&param.name) {
            add_validation_error(
                ctx,
                format!(
                    "Duplicate parameter '{}' in function '{}'.",
                    param.name, ctx.function.name
                ),
                None,
            );
        } else {
            ctx.params
                .insert(param.name.clone(), param.type_node.clone());
        }
    }
}

fn collect_locals(ctx: &mut FunctionValidationContext<'_, '_>) {
    for local in &ctx.function.locals {
        if contains_void_type(&local.type_node) {
            add_validation_error(
                ctx,
                format!(
                    "Void local '{}' is not allowed in function '{}'.",
                    local.name, ctx.function.name
                ),
                None,
            );
        }
        if let Some(reason) = invalid_slice_type_reason(&local.type_node) {
            add_validation_error(
                ctx,
                format!(
                    "Local '{}' in function '{}' has invalid slice type: {reason}.",
                    local.name, ctx.function.name
                ),
                None,
            );
        }
        if ctx.locals.contains_key(&local.name) {
            add_validation_error(
                ctx,
                format!(
                    "Duplicate local '{}' in function '{}'.",
                    local.name, ctx.function.name
                ),
                None,
            );
        } else {
            ctx.locals
                .insert(local.name.clone(), local.type_node.clone());
        }
    }
}

fn collect_labels(ctx: &mut FunctionValidationContext<'_, '_>) {
    for block in &ctx.function.blocks {
        if ctx.labels.contains_key(&block.label) {
            add_validation_error(
                ctx,
                format!(
                    "Duplicate block label '{}' in function '{}'.",
                    block.label, ctx.function.name
                ),
                Some(&block.label),
            );
        } else {
            ctx.labels.insert(block.label.clone(), ());
        }
    }
}

fn collect_temps(ctx: &mut FunctionValidationContext<'_, '_>) {
    for block in &ctx.function.blocks {
        for instruction in &block.instructions {
            let Some(target) = instruction_target(instruction) else {
                continue;
            };
            let MirValue::Temp { name, type_node } = target else {
                continue;
            };
            if contains_void_type(type_node) {
                add_validation_error(
                    ctx,
                    format!(
                        "Void temp '%{}' is not allowed in function '{}'.",
                        name, ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
            if let Some(reason) = invalid_slice_type_reason(type_node) {
                add_validation_error(
                    ctx,
                    format!(
                        "Temp '%{}' in function '{}' has invalid slice type: {reason}.",
                        name, ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
            if ctx.temps.contains_key(name) {
                add_validation_error(
                    ctx,
                    format!(
                        "Duplicate temp '%{}' in function '{}'.",
                        name, ctx.function.name
                    ),
                    Some(&block.label),
                );
            } else {
                ctx.temps.insert(name.clone(), type_node.clone());
            }
        }
    }
}

fn validate_block(ctx: &mut FunctionValidationContext<'_, '_>, block: &MirBlock) {
    for instruction in &block.instructions {
        validate_instruction(ctx, block, instruction);
    }
    validate_terminator(ctx, block, &block.terminator);
}

fn validate_instruction(
    ctx: &mut FunctionValidationContext<'_, '_>,
    block: &MirBlock,
    instruction: &MirInstruction,
) {
    match instruction {
        MirInstruction::ConstInt { target, .. } => {
            validate_target(ctx, block, target);
            if !is_integer_type(value_type(target)) {
                add_validation_error(
                    ctx,
                    format!(
                        "const_int target in function '{}' must be integer, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::ConstFloat { target, .. } => {
            validate_target(ctx, block, target);
            if !is_float_type(value_type(target)) {
                add_validation_error(
                    ctx,
                    format!(
                        "const_float target in function '{}' must be f64, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::ConstBool { target, .. } => {
            validate_target(ctx, block, target);
            if !is_bool_type(value_type(target)) {
                add_validation_error(
                    ctx,
                    format!(
                        "const_bool target in function '{}' must be bool, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::Move { target, value } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, value);
            if !same_mir_type(value_type(target), value_type(value)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Move type mismatch in function '{}': expected {}, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target)),
                        print_mir_type(value_type(value))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, left);
            validate_value(ctx, block, right);
            if !same_mir_type(value_type(left), value_type(right)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Binary operands for '{}' in function '{}' must have the same type, got {} and {}.",
                        binary_symbol(*op),
                        ctx.function.name,
                        print_mir_type(value_type(left)),
                        print_mir_type(value_type(right))
                    ),
                    Some(&block.label),
                );
            }
            if *op == MirBinaryOp::Mod {
                if is_float_type(value_type(left)) || is_float_type(value_type(right)) {
                    add_validation_error(
                        ctx,
                        format!(
                            "Binary operator '%' in function '{}' does not support f64 operands.",
                            ctx.function.name
                        ),
                        Some(&block.label),
                    );
                } else if !is_integer_type(value_type(left)) || !is_integer_type(value_type(right))
                {
                    add_validation_error(
                        ctx,
                        format!(
                            "Binary operands for '%' in function '{}' must be integers.",
                            ctx.function.name
                        ),
                        Some(&block.label),
                    );
                }
            } else if !is_numeric_type(value_type(left)) || !is_numeric_type(value_type(right)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Binary operands for '{}' in function '{}' must be numeric.",
                        binary_symbol(*op),
                        ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
            if !same_mir_type(value_type(target), value_type(left)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Binary result for '{}' in function '{}' must be {}, got {}.",
                        binary_symbol(*op),
                        ctx.function.name,
                        print_mir_type(value_type(left)),
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, operand);
            match op {
                MirUnaryOp::Neg => {
                    if !is_numeric_type(value_type(operand)) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Unary neg in function '{}' requires numeric operand, got {}.",
                                ctx.function.name,
                                print_mir_type(value_type(operand))
                            ),
                            Some(&block.label),
                        );
                    }
                    if !same_mir_type(value_type(target), value_type(operand)) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Unary neg result in function '{}' must be {}, got {}.",
                                ctx.function.name,
                                print_mir_type(value_type(operand)),
                                print_mir_type(value_type(target))
                            ),
                            Some(&block.label),
                        );
                    }
                }
                MirUnaryOp::Not => {
                    if !is_bool_type(value_type(operand)) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Unary not in function '{}' requires bool operand, got {}.",
                                ctx.function.name,
                                print_mir_type(value_type(operand))
                            ),
                            Some(&block.label),
                        );
                    }
                    if !is_bool_type(value_type(target)) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Unary not result in function '{}' must be bool, got {}.",
                                ctx.function.name,
                                print_mir_type(value_type(target))
                            ),
                            Some(&block.label),
                        );
                    }
                }
            }
        }
        MirInstruction::Compare {
            target,
            left,
            right,
            ..
        } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, left);
            validate_value(ctx, block, right);
            if !same_mir_type(value_type(left), value_type(right)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Compare operands in function '{}' must have the same type, got {} and {}.",
                        ctx.function.name,
                        print_mir_type(value_type(left)),
                        print_mir_type(value_type(right))
                    ),
                    Some(&block.label),
                );
            }
            if !is_bool_type(value_type(target)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Compare result in function '{}' must be bool, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::Cast { target, op, value } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, value);
            validate_cast(ctx, block, *op, value_type(value), value_type(target));
        }
        MirInstruction::Address { target, place } => {
            validate_target(ctx, block, target);
            validate_place(ctx, block, place);
            match value_type(target) {
                MirType::Pointer(element_type) => {
                    if !same_mir_type(element_type, place_type(place)) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Address result in function '{}' must point to {}, got {}.",
                                ctx.function.name,
                                print_mir_type(place_type(place)),
                                print_mir_type(value_type(target))
                            ),
                            Some(&block.label),
                        );
                    }
                }
                _ => add_validation_error(
                    ctx,
                    format!(
                        "Address result in function '{}' must be pointer, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                ),
            }
        }
        MirInstruction::Load { target, place } => {
            validate_target(ctx, block, target);
            validate_place(ctx, block, place);
            if !same_mir_type(value_type(target), place_type(place)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Load type mismatch in function '{}': place is {}, target is {}.",
                        ctx.function.name,
                        print_mir_type(place_type(place)),
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::Store { place, value } => {
            validate_place(ctx, block, place);
            validate_value(ctx, block, value);
            if !same_mir_type(place_type(place), value_type(value)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Store type mismatch in function '{}': place is {}, value is {}.",
                        ctx.function.name,
                        print_mir_type(place_type(place)),
                        print_mir_type(value_type(value))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::MakeSlice { target, data, len } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, data);
            validate_value(ctx, block, len);
            let MirType::Slice(element_type) = value_type(target) else {
                add_validation_error(
                    ctx,
                    format!(
                        "MakeSlice target in function '{}' must be a slice, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
                return;
            };
            let expected_pointer = MirType::Pointer(element_type.clone());
            if !same_mir_type(value_type(data), &expected_pointer) {
                add_validation_error(
                    ctx,
                    format!(
                        "MakeSlice data in function '{}' must be {}, got {}.",
                        ctx.function.name,
                        print_mir_type(&expected_pointer),
                        print_mir_type(value_type(data))
                    ),
                    Some(&block.label),
                );
            }
            if !is_u32_type(value_type(len)) {
                add_validation_error(
                    ctx,
                    format!(
                        "MakeSlice length in function '{}' must be u32, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(len))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::SliceData { target, slice } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                add_validation_error(
                    ctx,
                    format!(
                        "SliceData input in function '{}' must be a slice, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(slice))
                    ),
                    Some(&block.label),
                );
                return;
            };
            let expected = MirType::Pointer(element_type.clone());
            if !same_mir_type(value_type(target), &expected) {
                add_validation_error(
                    ctx,
                    format!(
                        "SliceData result in function '{}' must be {}, got {}.",
                        ctx.function.name,
                        print_mir_type(&expected),
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::SliceLen { target, slice } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, slice);
            if !matches!(value_type(slice), MirType::Slice(_)) {
                add_validation_error(
                    ctx,
                    format!(
                        "SliceLen input in function '{}' must be a slice, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(slice))
                    ),
                    Some(&block.label),
                );
            }
            if !is_u32_type(value_type(target)) {
                add_validation_error(
                    ctx,
                    format!(
                        "SliceLen result in function '{}' must be u32, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            validate_target(ctx, block, target);
            validate_value(ctx, block, slice);
            validate_value(ctx, block, start);
            validate_value(ctx, block, end);
            if !matches!(value_type(slice), MirType::Slice(_)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Subslice input in function '{}' must be a slice, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(slice))
                    ),
                    Some(&block.label),
                );
            }
            if !same_mir_type(value_type(target), value_type(slice)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Subslice result in function '{}' must match {}, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(slice)),
                        print_mir_type(value_type(target))
                    ),
                    Some(&block.label),
                );
            }
            for (role, value) in [("start", start), ("end", end)] {
                if !is_u32_type(value_type(value)) {
                    add_validation_error(
                        ctx,
                        format!(
                            "Subslice {role} in function '{}' must be u32, got {}.",
                            ctx.function.name,
                            print_mir_type(value_type(value))
                        ),
                        Some(&block.label),
                    );
                }
            }
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            if let Some(target) = target {
                validate_target(ctx, block, target);
            }
            for arg in args {
                validate_value(ctx, block, arg);
            }
            validate_call(ctx, block, function_name, args, target.as_ref());
        }
    }
}

fn validate_cast(
    ctx: &mut FunctionValidationContext<'_, '_>,
    block: &MirBlock,
    op: MirCastOp,
    input_type: &MirType,
    result_type: &MirType,
) {
    let expected_input = match op {
        MirCastOp::I32ToF64 => mir_primitive(MirPrimitiveTypeName::I32),
        MirCastOp::U32ToF64 => mir_primitive(MirPrimitiveTypeName::U32),
    };
    if !same_mir_type(input_type, &expected_input) {
        add_validation_error(
            ctx,
            format!(
                "Cast '{}' input in function '{}' must be {}, got {}.",
                print_cast_op(op),
                ctx.function.name,
                print_mir_type(&expected_input),
                print_mir_type(input_type)
            ),
            Some(&block.label),
        );
    }
    let expected_result = mir_primitive(MirPrimitiveTypeName::F64);
    if !same_mir_type(result_type, &expected_result) {
        add_validation_error(
            ctx,
            format!(
                "Cast '{}' result in function '{}' must be f64, got {}.",
                print_cast_op(op),
                ctx.function.name,
                print_mir_type(result_type)
            ),
            Some(&block.label),
        );
    }
}

fn validate_terminator(
    ctx: &mut FunctionValidationContext<'_, '_>,
    block: &MirBlock,
    terminator: &MirTerminator,
) {
    match terminator {
        MirTerminator::Return { value } => match (&ctx.function.return_type, value) {
            (MirType::Void, None) => {}
            (MirType::Void, Some(value)) => {
                validate_value(ctx, block, value);
                add_validation_error(
                    ctx,
                    format!(
                        "Function '{}' has a value, but a void return cannot have a value.",
                        ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
            (_, None) => add_validation_error(
                ctx,
                format!(
                    "Function '{}': a non-void return requires a value.",
                    ctx.function.name
                ),
                Some(&block.label),
            ),
            (return_type, Some(value)) => {
                validate_value(ctx, block, value);
                if !same_mir_type(value_type(value), return_type) {
                    add_validation_error(
                        ctx,
                        format!(
                            "Return type mismatch in function '{}': expected {}, got {}.",
                            ctx.function.name,
                            print_mir_type(return_type),
                            print_mir_type(value_type(value))
                        ),
                        Some(&block.label),
                    );
                }
            }
        },
        MirTerminator::Jump { label } => {
            if !ctx.labels.contains_key(label) {
                add_validation_error(
                    ctx,
                    format!(
                        "Jump target '{}' does not exist in function '{}'.",
                        label, ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            validate_value(ctx, block, condition);
            if !is_bool_type(value_type(condition)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Branch condition in function '{}' must be bool, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(condition))
                    ),
                    Some(&block.label),
                );
            }
            if !ctx.labels.contains_key(then_label) {
                add_validation_error(
                    ctx,
                    format!(
                        "Branch target '{}' does not exist in function '{}'.",
                        then_label, ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
            if !ctx.labels.contains_key(else_label) {
                add_validation_error(
                    ctx,
                    format!(
                        "Branch target '{}' does not exist in function '{}'.",
                        else_label, ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
        }
    }
}

fn validate_target(
    ctx: &mut FunctionValidationContext<'_, '_>,
    block: &MirBlock,
    target: &MirValue,
) {
    match target {
        MirValue::Temp { name, .. } => {
            if !ctx.temps.contains_key(name) {
                add_validation_error(
                    ctx,
                    format!(
                        "Unknown temp '%{}' in function '{}'.",
                        name, ctx.function.name
                    ),
                    Some(&block.label),
                );
            }
        }
        MirValue::Local { .. } | MirValue::Param { .. } => validate_value(ctx, block, target),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            add_validation_error(
                ctx,
                format!(
                    "Instruction target in function '{}' must be a temp, local, or param.",
                    ctx.function.name
                ),
                Some(&block.label),
            );
        }
    }
}

fn validate_value(ctx: &mut FunctionValidationContext<'_, '_>, block: &MirBlock, value: &MirValue) {
    if contains_void_type(value_type(value)) {
        add_validation_error(
            ctx,
            format!(
                "Void MIR value is not allowed in function '{}'.",
                ctx.function.name
            ),
            Some(&block.label),
        );
    }
    match value {
        MirValue::Param { name, type_node } => match ctx.params.get(name) {
            Some(declared) if !same_mir_type(declared, type_node) => add_validation_error(
                ctx,
                format!(
                    "Param '{}' in function '{}' has type {}, got {}.",
                    name,
                    ctx.function.name,
                    print_mir_type(declared),
                    print_mir_type(type_node)
                ),
                Some(&block.label),
            ),
            Some(_) => {}
            None => add_validation_error(
                ctx,
                format!(
                    "Unknown param '{}' in function '{}'.",
                    name, ctx.function.name
                ),
                Some(&block.label),
            ),
        },
        MirValue::Local { name, type_node } => match ctx.locals.get(name) {
            Some(declared) if !same_mir_type(declared, type_node) => add_validation_error(
                ctx,
                format!(
                    "Local '{}' in function '{}' has type {}, got {}.",
                    name,
                    ctx.function.name,
                    print_mir_type(declared),
                    print_mir_type(type_node)
                ),
                Some(&block.label),
            ),
            Some(_) => {}
            None => add_validation_error(
                ctx,
                format!(
                    "Unknown local '{}' in function '{}'.",
                    name, ctx.function.name
                ),
                Some(&block.label),
            ),
        },
        MirValue::Temp { name, type_node } => match ctx.temps.get(name) {
            Some(declared) if !same_mir_type(declared, type_node) => add_validation_error(
                ctx,
                format!(
                    "Temp '%{}' in function '{}' has type {}, got {}.",
                    name,
                    ctx.function.name,
                    print_mir_type(declared),
                    print_mir_type(type_node)
                ),
                Some(&block.label),
            ),
            Some(_) => {}
            None => add_validation_error(
                ctx,
                format!(
                    "Unknown temp '%{}' in function '{}'.",
                    name, ctx.function.name
                ),
                Some(&block.label),
            ),
        },
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {}
    }
}

fn validate_place(ctx: &mut FunctionValidationContext<'_, '_>, block: &MirBlock, place: &MirPlace) {
    if contains_void_type(place_type(place)) {
        add_validation_error(
            ctx,
            format!(
                "Void MIR place is not allowed in function '{}'.",
                ctx.function.name
            ),
            Some(&block.label),
        );
    }
    match place {
        MirPlace::Param { name, type_node } => validate_value(
            ctx,
            block,
            &MirValue::Param {
                name: name.clone(),
                type_node: type_node.clone(),
            },
        ),
        MirPlace::Local { name, type_node } => validate_value(
            ctx,
            block,
            &MirValue::Local {
                name: name.clone(),
                type_node: type_node.clone(),
            },
        ),
        MirPlace::Deref { pointer, type_node } => {
            validate_value(ctx, block, pointer);
            match value_type(pointer) {
                MirType::Pointer(element_type) => {
                    if !same_mir_type(element_type, type_node) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Deref place type mismatch in function '{}': pointer element is {}, place is {}.",
                                ctx.function.name,
                                print_mir_type(element_type),
                                print_mir_type(type_node)
                            ),
                            Some(&block.label),
                        );
                    }
                }
                other => add_validation_error(
                    ctx,
                    format!(
                        "Deref place in function '{}' requires pointer value, got {}.",
                        ctx.function.name,
                        print_mir_type(other)
                    ),
                    Some(&block.label),
                ),
            }
        }
        MirPlace::Index {
            base,
            index,
            type_node,
        } => {
            validate_place(ctx, block, base);
            validate_value(ctx, block, index);
            if !is_index_type(value_type(index)) {
                add_validation_error(
                    ctx,
                    format!(
                        "Index place in function '{}' requires i32 or u32 index, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(index))
                    ),
                    Some(&block.label),
                );
            }
            match place_type(base) {
                MirType::Pointer(element_type) => {
                    if !same_mir_type(element_type, type_node) {
                        add_validation_error(
                            ctx,
                            format!(
                                "Index place type mismatch in function '{}': expected {}, got {}.",
                                ctx.function.name,
                                print_mir_type(element_type),
                                print_mir_type(type_node)
                            ),
                            Some(&block.label),
                        );
                    }
                }
                other => add_validation_error(
                    ctx,
                    format!(
                        "Index base in function '{}' must be pointer, got {}.",
                        ctx.function.name,
                        print_mir_type(other)
                    ),
                    Some(&block.label),
                ),
            }
        }
        MirPlace::SliceIndex {
            slice,
            index,
            type_node,
        } => {
            validate_value(ctx, block, slice);
            validate_value(ctx, block, index);
            if !is_u32_type(value_type(index)) {
                add_validation_error(
                    ctx,
                    format!(
                        "SliceIndex in function '{}' requires u32 index, got {}.",
                        ctx.function.name,
                        print_mir_type(value_type(index))
                    ),
                    Some(&block.label),
                );
            }
            match value_type(slice) {
                MirType::Slice(element_type) => {
                    if !same_mir_type(element_type, type_node) {
                        add_validation_error(
                            ctx,
                            format!(
                                "SliceIndex place type mismatch in function '{}': expected {}, got {}.",
                                ctx.function.name,
                                print_mir_type(element_type),
                                print_mir_type(type_node)
                            ),
                            Some(&block.label),
                        );
                    }
                }
                other => add_validation_error(
                    ctx,
                    format!(
                        "SliceIndex base in function '{}' must be a slice, got {}.",
                        ctx.function.name,
                        print_mir_type(other)
                    ),
                    Some(&block.label),
                ),
            }
        }
        MirPlace::Field {
            base,
            field_name,
            type_node,
        } => {
            validate_place(ctx, block, base);
            let MirType::Struct(struct_name) = place_type(base) else {
                add_validation_error(
                    ctx,
                    format!(
                        "Field base in function '{}' must be struct, got {}.",
                        ctx.function.name,
                        print_mir_type(place_type(base))
                    ),
                    Some(&block.label),
                );
                return;
            };
            let Some(struct_info) = ctx.structs.get(struct_name) else {
                add_validation_error(
                    ctx,
                    format!(
                        "Unknown struct '{}' in function '{}'.",
                        struct_name, ctx.function.name
                    ),
                    Some(&block.label),
                );
                return;
            };
            let Some(field) = struct_info
                .fields
                .iter()
                .find(|field| field.name == *field_name)
            else {
                add_validation_error(
                    ctx,
                    format!(
                        "Unknown field '{}' on struct '{}' in function '{}'.",
                        field_name, struct_info.name, ctx.function.name
                    ),
                    Some(&block.label),
                );
                return;
            };
            if !same_mir_type(&field.type_node, type_node) {
                add_validation_error(
                    ctx,
                    format!(
                        "Field place type mismatch in function '{}': field '{}' is {}, place is {}.",
                        ctx.function.name,
                        field_name,
                        print_mir_type(&field.type_node),
                        print_mir_type(type_node)
                    ),
                    Some(&block.label),
                );
            }
        }
    }
}

fn validate_call(
    ctx: &mut FunctionValidationContext<'_, '_>,
    block: &MirBlock,
    function_name: &str,
    args: &[MirValue],
    target: Option<&MirValue>,
) {
    let Some(callee) = ctx.functions.get(function_name) else {
        add_validation_error(
            ctx,
            format!(
                "Unknown function '{}' in function '{}'.",
                function_name, ctx.function.name
            ),
            Some(&block.label),
        );
        return;
    };

    if args.len() != callee.params.len() {
        add_validation_error(
            ctx,
            format!(
                "Call to '{}' in function '{}' expects {} argument(s), got {}.",
                function_name,
                ctx.function.name,
                callee.params.len(),
                args.len()
            ),
            Some(&block.label),
        );
    }

    for (index, (arg, param)) in args.iter().zip(&callee.params).enumerate() {
        if !same_mir_type(value_type(arg), &param.type_node) {
            add_validation_error(
                ctx,
                format!(
                    "Call argument {} to '{}' in function '{}' must be {}, got {}.",
                    index + 1,
                    function_name,
                    ctx.function.name,
                    print_mir_type(&param.type_node),
                    print_mir_type(value_type(arg))
                ),
                Some(&block.label),
            );
        }
    }

    match (&callee.return_type, target) {
        (MirType::Void, None) => {}
        (MirType::Void, Some(_)) => add_validation_error(
            ctx,
            format!(
                "Call to '{}' in function '{}': a void call cannot have a target.",
                function_name, ctx.function.name
            ),
            Some(&block.label),
        ),
        (_, None) => add_validation_error(
            ctx,
            format!(
                "Call to '{}' in function '{}': a non-void call requires a target.",
                function_name, ctx.function.name
            ),
            Some(&block.label),
        ),
        (return_type, Some(target)) if !same_mir_type(value_type(target), return_type) => {
            add_validation_error(
                ctx,
                format!(
                    "Call result for '{}' in function '{}' must be {}, got {}.",
                    function_name,
                    ctx.function.name,
                    print_mir_type(return_type),
                    print_mir_type(value_type(target))
                ),
                Some(&block.label),
            );
        }
        _ => {}
    }
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
        MirInstruction::Store { .. } => None,
    }
}

fn contains_void_type(type_node: &MirType) -> bool {
    match type_node {
        MirType::Void => true,
        MirType::Pointer(element_type) | MirType::Slice(element_type) => {
            contains_void_type(element_type)
        }
        MirType::Primitive(_) | MirType::Struct(_) => false,
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

fn same_mir_type(left: &MirType, right: &MirType) -> bool {
    left == right
}

fn is_bool_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::Bool))
}

fn is_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(
            MirPrimitiveTypeName::I32
                | MirPrimitiveTypeName::I64
                | MirPrimitiveTypeName::U32
                | MirPrimitiveTypeName::U64
        )
    )
}

fn is_float_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
}

fn is_numeric_type(type_node: &MirType) -> bool {
    is_integer_type(type_node) || is_float_type(type_node)
}

fn is_index_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
    )
}

fn is_u32_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::U32))
}

fn invalid_slice_type_reason(type_node: &MirType) -> Option<&'static str> {
    match type_node {
        MirType::Slice(element_type) => match element_type.as_ref() {
            MirType::Void => Some("void slice element"),
            MirType::Slice(_) => Some("direct slice element"),
            MirType::Pointer(nested) => invalid_slice_type_reason(nested),
            MirType::Primitive(_) | MirType::Struct(_) => None,
        },
        MirType::Pointer(element_type) => invalid_slice_type_reason(element_type),
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => None,
    }
}

fn binary_symbol(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "+",
        MirBinaryOp::Sub => "-",
        MirBinaryOp::Mul => "*",
        MirBinaryOp::Div => "/",
        MirBinaryOp::Mod => "%",
    }
}

fn add_validation_error(
    ctx: &mut FunctionValidationContext<'_, '_>,
    message: String,
    block_label: Option<&str>,
) {
    ctx.errors.push(MirValidationError {
        message,
        function_name: Some(ctx.function.name.clone()),
        block_label: block_label.map(str::to_string),
    });
}
