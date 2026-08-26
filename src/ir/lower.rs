use std::collections::HashMap;

use crate::{
    AssignmentStatement, CalcKernelType, CheckedProgram, Expression, FunctionInfo, LetStatement,
    PrimitiveTypeName, Statement, get_expr_type, get_let_type, materialize_integer_literal_type,
    primitive_type,
};

use super::model::{place_type, value_type};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MutableMirBlock {
    label: String,
    instructions: Vec<MirInstruction>,
    terminator: Option<MirTerminator>,
}

#[derive(Debug, Default)]
struct MirBuilder {
    temp_counter: usize,
    block_counter: usize,
}

impl MirBuilder {
    fn temp(&mut self, type_node: MirType) -> MirValue {
        let name = format!("t{}", self.temp_counter);
        self.temp_counter += 1;
        MirValue::Temp { name, type_node }
    }

    fn const_bool(value: bool) -> MirValue {
        MirValue::ConstBool {
            value,
            type_node: mir_primitive(MirPrimitiveTypeName::Bool),
        }
    }

    fn next_block_label(&mut self) -> String {
        let label = format!("bb{}", self.block_counter);
        self.block_counter += 1;
        label
    }
}

struct FunctionLowerContext<'program> {
    checked_program: &'program CheckedProgram,
    builder: MirBuilder,
    values: HashMap<String, MirValue>,
    locals: Vec<MirLocal>,
    blocks: Vec<MutableMirBlock>,
    current_block: Option<usize>,
    synthetic_local_counter: usize,
    loop_targets: Vec<LoopTargets>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopTargets {
    continue_label: String,
    break_label: String,
}

pub fn lower_to_mir(checked_program: &CheckedProgram) -> Result<MirModule, MirLowerError> {
    Ok(MirModule {
        structs: checked_program
            .structs
            .iter()
            .map(|struct_info| {
                Ok(MirStruct {
                    name: struct_info.name.clone(),
                    fields: struct_info
                        .fields
                        .iter()
                        .map(|field| {
                            Ok(MirStructField {
                                name: field.name.clone(),
                                type_node: to_mir_type(&field.type_node)?,
                            })
                        })
                        .collect::<Result<Vec<_>, MirLowerError>>()?,
                })
            })
            .collect::<Result<Vec<_>, MirLowerError>>()?,
        functions: checked_program
            .functions
            .iter()
            .map(|function| lower_function(checked_program, function))
            .collect::<Result<Vec<_>, MirLowerError>>()?,
    })
}

fn lower_function(
    checked_program: &CheckedProgram,
    function_info: &FunctionInfo,
) -> Result<MirFunction, MirLowerError> {
    let params = function_info
        .params
        .iter()
        .map(|param| {
            Ok(MirParam {
                name: param.name.clone(),
                type_node: to_mir_type(&param.type_node)?,
            })
        })
        .collect::<Result<Vec<_>, MirLowerError>>()?;
    let return_type = to_mir_type(&function_info.return_type)?;
    let mut values = HashMap::new();
    for param in &params {
        values.insert(
            param.name.clone(),
            MirValue::Param {
                name: param.name.clone(),
                type_node: param.type_node.clone(),
            },
        );
    }

    let mut context = FunctionLowerContext {
        checked_program,
        builder: MirBuilder::default(),
        values,
        locals: Vec::new(),
        blocks: Vec::new(),
        current_block: None,
        synthetic_local_counter: 0,
        loop_targets: Vec::new(),
    };

    start_block(&mut context, None);
    lower_statements(&mut context, &function_info.declaration.body.statements)?;
    if context.current_block.is_some() {
        if matches!(return_type, MirType::Void) {
            set_terminator(&mut context, MirTerminator::Return { value: None })?;
        } else {
            return Err(MirLowerError::new(format!(
                "MIR lowering invariant violation: function '{}' has no return terminator.",
                function_info.name
            )));
        }
    }

    let locals = context.locals.clone();
    let blocks = finalize_blocks(context, &function_info.name)?;

    Ok(MirFunction {
        name: function_info.name.clone(),
        exported: function_info.exported,
        params,
        return_type,
        locals,
        blocks,
    })
}

fn lower_statements(
    context: &mut FunctionLowerContext<'_>,
    statements: &[Statement],
) -> Result<(), MirLowerError> {
    for statement in statements {
        if context.current_block.is_none() {
            return Err(unsupported("statements after return"));
        }
        lower_statement(context, statement)?;
    }
    Ok(())
}

fn lower_statement(
    context: &mut FunctionLowerContext<'_>,
    statement: &Statement,
) -> Result<(), MirLowerError> {
    match statement {
        Statement::Block(block) => lower_statements(context, &block.statements),
        Statement::Let(statement) => lower_let_statement(context, statement),
        Statement::Assignment(statement) => lower_assignment_statement(context, statement),
        Statement::Call(statement) => lower_call_statement(context, &statement.call),
        Statement::Return(statement) => {
            let value = statement
                .value
                .as_ref()
                .map(|value| lower_expression(context, value))
                .transpose()?;
            set_terminator(context, MirTerminator::Return { value })
        }
        Statement::Break(_) => {
            let label = context
                .loop_targets
                .last()
                .map(|targets| targets.break_label.clone())
                .ok_or_else(|| {
                    MirLowerError::new(
                        "MIR lowering invariant violation: 'break' has no enclosing loop.",
                    )
                })?;
            set_terminator(context, MirTerminator::Jump { label })
        }
        Statement::Continue(_) => {
            let label = context
                .loop_targets
                .last()
                .map(|targets| targets.continue_label.clone())
                .ok_or_else(|| {
                    MirLowerError::new(
                        "MIR lowering invariant violation: 'continue' has no enclosing loop.",
                    )
                })?;
            set_terminator(context, MirTerminator::Jump { label })
        }
        Statement::If(statement) => {
            let condition = lower_expression(context, &statement.condition)?;
            let then_label = context.builder.next_block_label();
            let else_or_join_label = context.builder.next_block_label();
            set_terminator(
                context,
                MirTerminator::Branch {
                    condition,
                    then_label: then_label.clone(),
                    else_label: else_or_join_label.clone(),
                },
            )?;

            let then_block = start_block(context, Some(then_label));
            lower_statements(context, &statement.then_block.statements)?;

            let Some(else_block_statement) = &statement.else_block else {
                if !block_has_terminator(context, then_block) {
                    set_block_terminator(
                        context,
                        then_block,
                        MirTerminator::Jump {
                            label: else_or_join_label.clone(),
                        },
                    );
                }
                start_block(context, Some(else_or_join_label));
                return Ok(());
            };

            let else_block = start_block(context, Some(else_or_join_label));
            lower_statements(context, &else_block_statement.statements)?;

            if block_has_terminator(context, then_block)
                && block_has_terminator(context, else_block)
            {
                context.current_block = None;
                return Ok(());
            }

            let join_label = context.builder.next_block_label();
            if !block_has_terminator(context, then_block) {
                set_block_terminator(
                    context,
                    then_block,
                    MirTerminator::Jump {
                        label: join_label.clone(),
                    },
                );
            }
            if !block_has_terminator(context, else_block) {
                set_block_terminator(
                    context,
                    else_block,
                    MirTerminator::Jump {
                        label: join_label.clone(),
                    },
                );
            }
            start_block(context, Some(join_label));
            Ok(())
        }
        Statement::While(statement) => {
            let cond_label = context.builder.next_block_label();
            let body_label = context.builder.next_block_label();
            let exit_label = context.builder.next_block_label();

            set_terminator(
                context,
                MirTerminator::Jump {
                    label: cond_label.clone(),
                },
            )?;

            start_block(context, Some(cond_label.clone()));
            let condition = lower_expression(context, &statement.condition)?;
            set_terminator(
                context,
                MirTerminator::Branch {
                    condition,
                    then_label: body_label.clone(),
                    else_label: exit_label.clone(),
                },
            )?;

            start_block(context, Some(body_label));
            context.loop_targets.push(LoopTargets {
                continue_label: cond_label.clone(),
                break_label: exit_label.clone(),
            });
            let body_result = lower_statements(context, &statement.body.statements);
            context.loop_targets.pop();
            body_result?;
            if context.current_block.is_some() {
                set_terminator(context, MirTerminator::Jump { label: cond_label })?;
            }

            start_block(context, Some(exit_label));
            Ok(())
        }
        Statement::Error { .. } => Err(unsupported("ErrorStatement")),
    }
}

fn lower_let_statement(
    context: &mut FunctionLowerContext<'_>,
    statement: &LetStatement,
) -> Result<(), MirLowerError> {
    let type_node = to_mir_type(&require_let_type(context.checked_program, statement)?)?;
    let local = MirLocal {
        name: statement.name.name.clone(),
        type_node,
    };
    let local_value = MirValue::Local {
        name: local.name.clone(),
        type_node: local.type_node.clone(),
    };
    context.locals.push(local);
    context
        .values
        .insert(statement.name.name.clone(), local_value.clone());

    let initializer = lower_expression(context, &statement.initializer)?;
    emit_instruction(
        context,
        MirInstruction::Move {
            target: local_value,
            value: initializer,
        },
    )
}

fn lower_assignment_statement(
    context: &mut FunctionLowerContext<'_>,
    statement: &AssignmentStatement,
) -> Result<(), MirLowerError> {
    if let Expression::Identifier { .. } = &statement.target {
        let target = require_identifier_value(context, &statement.target)?;
        if !matches!(target, MirValue::Local { .. }) {
            return Err(unsupported("assignment to non-local variable"));
        }
        let value = lower_expression(context, &statement.value)?;
        return emit_instruction(context, MirInstruction::Move { target, value });
    }

    let place = lower_place(context, &statement.target)?;
    let value = lower_expression(context, &statement.value)?;
    emit_instruction(context, MirInstruction::Store { place, value })
}

fn lower_expression(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
) -> Result<MirValue, MirLowerError> {
    match expression {
        Expression::Identifier { .. } => require_identifier_value(context, expression),
        Expression::IntegerLiteral { text, .. } => {
            let type_node = to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?;
            let target = context.builder.temp(type_node);
            emit_instruction(
                context,
                MirInstruction::ConstInt {
                    target: target.clone(),
                    value: text.clone(),
                },
            )?;
            Ok(target)
        }
        Expression::FloatLiteral { text, .. } => {
            let type_node = to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?;
            let target = context.builder.temp(type_node);
            emit_instruction(
                context,
                MirInstruction::ConstFloat {
                    target: target.clone(),
                    value: text.clone(),
                },
            )?;
            Ok(target)
        }
        Expression::BoolLiteral { value, .. } => {
            let type_node = to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?;
            let target = context.builder.temp(type_node);
            emit_instruction(
                context,
                MirInstruction::ConstBool {
                    target: target.clone(),
                    value: *value,
                },
            )?;
            Ok(target)
        }
        Expression::Unary {
            operator, operand, ..
        } => {
            let operand = lower_expression(context, operand)?;
            let type_node = to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?;
            let target = context.builder.temp(type_node);
            emit_instruction(
                context,
                MirInstruction::Unary {
                    target: target.clone(),
                    op: if operator == "-" {
                        MirUnaryOp::Neg
                    } else {
                        MirUnaryOp::Not
                    },
                    operand,
                },
            )?;
            Ok(target)
        }
        Expression::Binary {
            operator,
            left,
            right,
            ..
        } => lower_binary_expression(context, expression, operator, left, right),
        Expression::Call { callee, args, .. } => {
            lower_call_expression(context, expression, callee, args)
        }
        Expression::SliceConstructor { data, len, .. } => {
            let data = lower_expression(context, data)?;
            let len = lower_expression(context, len)?;
            let target = context.builder.temp(to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?);
            emit_instruction(
                context,
                MirInstruction::MakeSlice {
                    target: target.clone(),
                    data,
                    len,
                },
            )?;
            Ok(target)
        }
        Expression::Subslice {
            slice, start, end, ..
        } => {
            let slice = lower_expression(context, slice)?;
            let start = lower_expression(context, start)?;
            let end = lower_expression(context, end)?;
            let target = context.builder.temp(to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?);
            emit_instruction(
                context,
                MirInstruction::Subslice {
                    target: target.clone(),
                    slice,
                    start,
                    end,
                },
            )?;
            Ok(target)
        }
        Expression::Field { object, field, .. }
            if matches!(
                require_expression_type(context.checked_program, object)?,
                CalcKernelType::Slice(_)
            ) =>
        {
            let slice = lower_expression(context, object)?;
            let target = context.builder.temp(to_mir_type(&require_expression_type(
                context.checked_program,
                expression,
            )?)?);
            let instruction = if field.name == "data" {
                MirInstruction::SliceData {
                    target: target.clone(),
                    slice,
                }
            } else {
                MirInstruction::SliceLen {
                    target: target.clone(),
                    slice,
                }
            };
            emit_instruction(context, instruction)?;
            Ok(target)
        }
        Expression::Field { .. } | Expression::Index { .. } => {
            lower_load_expression(context, expression)
        }
        Expression::Parenthesized { expression, .. } => lower_expression(context, expression),
        Expression::Error { .. } => Err(unsupported("ErrorExpression")),
    }
}

fn lower_load_expression(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
) -> Result<MirValue, MirLowerError> {
    let place = lower_place(context, expression)?;
    let target = context.builder.temp(place_type(&place).clone());
    emit_instruction(
        context,
        MirInstruction::Load {
            target: target.clone(),
            place,
        },
    )?;
    Ok(target)
}

fn lower_binary_expression(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
    operator: &str,
    left: &Expression,
    right: &Expression,
) -> Result<MirValue, MirLowerError> {
    if operator == "&&" || operator == "||" {
        return lower_short_circuit_expression(context, expression, operator, left, right);
    }

    let left = lower_expression(context, left)?;
    let right = lower_expression(context, right)?;
    let target = context.builder.temp(to_mir_type(&require_expression_type(
        context.checked_program,
        expression,
    )?)?);

    if let Some(op) = binary_op(operator) {
        emit_instruction(
            context,
            MirInstruction::Binary {
                target: target.clone(),
                op,
                left,
                right,
            },
        )?;
        return Ok(target);
    }

    if let Some(op) = compare_op(operator) {
        emit_instruction(
            context,
            MirInstruction::Compare {
                target: target.clone(),
                op,
                left,
                right,
            },
        )?;
        return Ok(target);
    }

    Err(unsupported(format!("binary operator '{operator}'")))
}

fn lower_short_circuit_expression(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
    operator: &str,
    left: &Expression,
    right: &Expression,
) -> Result<MirValue, MirLowerError> {
    let result = create_synthetic_local(
        context,
        to_mir_type(&require_expression_type(
            context.checked_program,
            expression,
        )?)?,
    );
    let left = lower_expression(context, left)?;
    let first_label = context.builder.next_block_label();
    let second_label = context.builder.next_block_label();
    let join_label = context.builder.next_block_label();
    let rhs_label = if operator == "&&" {
        first_label.clone()
    } else {
        second_label.clone()
    };
    let short_label = if operator == "&&" {
        second_label.clone()
    } else {
        first_label.clone()
    };

    set_terminator(
        context,
        MirTerminator::Branch {
            condition: left,
            then_label: if operator == "&&" {
                rhs_label.clone()
            } else {
                short_label.clone()
            },
            else_label: if operator == "&&" {
                short_label.clone()
            } else {
                rhs_label.clone()
            },
        },
    )?;

    if operator == "&&" {
        lower_short_circuit_rhs_block(context, rhs_label, right, &result, &join_label)?;
        lower_short_circuit_constant_block(context, short_label, false, &result, &join_label)?;
    } else {
        lower_short_circuit_constant_block(context, short_label, true, &result, &join_label)?;
        lower_short_circuit_rhs_block(context, rhs_label, right, &result, &join_label)?;
    }

    start_block(context, Some(join_label));
    Ok(result)
}

fn lower_short_circuit_rhs_block(
    context: &mut FunctionLowerContext<'_>,
    label: String,
    expression: &Expression,
    result: &MirValue,
    join_label: &str,
) -> Result<(), MirLowerError> {
    start_block(context, Some(label));
    let right = lower_expression(context, expression)?;
    emit_instruction(
        context,
        MirInstruction::Move {
            target: result.clone(),
            value: right,
        },
    )?;
    set_terminator(
        context,
        MirTerminator::Jump {
            label: join_label.to_string(),
        },
    )
}

fn lower_short_circuit_constant_block(
    context: &mut FunctionLowerContext<'_>,
    label: String,
    value: bool,
    result: &MirValue,
    join_label: &str,
) -> Result<(), MirLowerError> {
    start_block(context, Some(label));
    emit_instruction(
        context,
        MirInstruction::Move {
            target: result.clone(),
            value: MirBuilder::const_bool(value),
        },
    )?;
    set_terminator(
        context,
        MirTerminator::Jump {
            label: join_label.to_string(),
        },
    )
}

fn lower_call_expression(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
    callee: &Expression,
    args: &[Expression],
) -> Result<MirValue, MirLowerError> {
    let Expression::Identifier { name, .. } = callee else {
        return Err(unsupported("non-identifier call callee"));
    };

    if let Some(op) = cast_builtin_op(name) {
        if args.len() != 1 {
            return Err(MirLowerError::new(format!(
                "MIR lowering invariant violation: compiler builtin '{name}' expects one argument."
            )));
        }
        let value = lower_expression(context, &args[0])?;
        let target = context.builder.temp(to_mir_type(&require_expression_type(
            context.checked_program,
            expression,
        )?)?);
        emit_instruction(
            context,
            MirInstruction::Cast {
                target: target.clone(),
                op,
                value,
            },
        )?;
        return Ok(target);
    }

    let args = args
        .iter()
        .map(|arg| lower_expression(context, arg))
        .collect::<Result<Vec<_>, MirLowerError>>()?;
    let target = context.builder.temp(to_mir_type(&require_expression_type(
        context.checked_program,
        expression,
    )?)?);
    emit_instruction(
        context,
        MirInstruction::Call {
            target: Some(target.clone()),
            function_name: name.clone(),
            args,
        },
    )?;
    Ok(target)
}

fn lower_call_statement(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
) -> Result<(), MirLowerError> {
    let Expression::Call { callee, args, .. } = expression else {
        return Err(MirLowerError::new(
            "MIR lowering invariant violation: call statement has a non-call expression.",
        ));
    };
    let Expression::Identifier { name, .. } = &**callee else {
        return Err(unsupported("non-identifier call callee"));
    };
    if cast_builtin_op(name).is_some() {
        return Err(MirLowerError::new(format!(
            "MIR lowering invariant violation: value builtin '{name}' used as a statement."
        )));
    }
    let args = args
        .iter()
        .map(|arg| lower_expression(context, arg))
        .collect::<Result<Vec<_>, MirLowerError>>()?;
    emit_instruction(
        context,
        MirInstruction::Call {
            target: None,
            function_name: name.clone(),
            args,
        },
    )
}

fn lower_place(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
) -> Result<MirPlace, MirLowerError> {
    match expression {
        Expression::Identifier { .. } => {
            let value = require_identifier_value(context, expression)?;
            match value {
                MirValue::Param { name, type_node } => Ok(MirPlace::Param { name, type_node }),
                MirValue::Local { name, type_node } => Ok(MirPlace::Local { name, type_node }),
                MirValue::Temp { .. }
                | MirValue::ConstInt { .. }
                | MirValue::ConstFloat { .. }
                | MirValue::ConstBool { .. } => Err(unsupported("non-place value")),
            }
        }
        Expression::Index { object, index, .. } => {
            let object_type = require_expression_type(context.checked_program, object)?;
            if matches!(object_type, CalcKernelType::Slice(_)) {
                let slice = lower_expression(context, object)?;
                let index = lower_expression(context, index)?;
                return Ok(MirPlace::SliceIndex {
                    slice,
                    index,
                    type_node: to_mir_type(&require_expression_type(
                        context.checked_program,
                        expression,
                    )?)?,
                });
            }
            let base = lower_pointer_base_place(context, object)?;
            let index = lower_expression(context, index)?;
            Ok(MirPlace::Index {
                base: Box::new(base),
                index,
                type_node: to_mir_type(&require_expression_type(
                    context.checked_program,
                    expression,
                )?)?,
            })
        }
        Expression::Field { object, field, .. } => {
            let base = lower_place(context, object)?;
            Ok(MirPlace::Field {
                base: Box::new(base),
                field_name: field.name.clone(),
                type_node: to_mir_type(&require_expression_type(
                    context.checked_program,
                    expression,
                )?)?,
            })
        }
        Expression::Parenthesized { expression, .. } => lower_place(context, expression),
        _ => Err(unsupported("expression place")),
    }
}

fn lower_pointer_base_place(
    context: &mut FunctionLowerContext<'_>,
    expression: &Expression,
) -> Result<MirPlace, MirLowerError> {
    let ordinary_place = match expression {
        Expression::Identifier { .. } | Expression::Index { .. } => true,
        Expression::Field { object, .. } => !matches!(
            require_expression_type(context.checked_program, object)?,
            CalcKernelType::Slice(_)
        ),
        Expression::Parenthesized { expression, .. } => {
            return lower_pointer_base_place(context, expression);
        }
        _ => false,
    };
    if ordinary_place {
        return lower_place(context, expression);
    }

    let pointer = lower_expression(context, expression)?;
    let local =
        create_synthetic_local_with_prefix(context, "ik_place", value_type(&pointer).clone());
    emit_instruction(
        context,
        MirInstruction::Move {
            target: local.clone(),
            value: pointer,
        },
    )?;
    let MirValue::Local { name, type_node } = local else {
        unreachable!("synthetic place base is always a local")
    };
    Ok(MirPlace::Local { name, type_node })
}

fn start_block(context: &mut FunctionLowerContext<'_>, label: Option<String>) -> usize {
    let label = label.unwrap_or_else(|| context.builder.next_block_label());
    let block = MutableMirBlock {
        label,
        instructions: Vec::new(),
        terminator: None,
    };
    context.blocks.push(block);
    let index = context.blocks.len() - 1;
    context.current_block = Some(index);
    index
}

fn block_has_terminator(context: &FunctionLowerContext<'_>, block_index: usize) -> bool {
    context
        .blocks
        .get(block_index)
        .is_some_and(|block| block.terminator.is_some())
}

fn set_block_terminator(
    context: &mut FunctionLowerContext<'_>,
    block_index: usize,
    terminator: MirTerminator,
) {
    if let Some(block) = context.blocks.get_mut(block_index) {
        block.terminator = Some(terminator);
    }
    if context.current_block == Some(block_index) {
        context.current_block = None;
    }
}

fn create_synthetic_local(context: &mut FunctionLowerContext<'_>, type_node: MirType) -> MirValue {
    create_synthetic_local_with_prefix(context, "ik_sc", type_node)
}

fn create_synthetic_local_with_prefix(
    context: &mut FunctionLowerContext<'_>,
    prefix: &str,
    type_node: MirType,
) -> MirValue {
    loop {
        let name = format!("{prefix}{}", context.synthetic_local_counter);
        context.synthetic_local_counter += 1;
        if context.values.contains_key(&name) {
            continue;
        }
        let local = MirLocal {
            name: name.clone(),
            type_node: type_node.clone(),
        };
        let value = MirValue::Local {
            name: name.clone(),
            type_node,
        };
        context.locals.push(local);
        context.values.insert(name, value.clone());
        return value;
    }
}

fn emit_instruction(
    context: &mut FunctionLowerContext<'_>,
    instruction: MirInstruction,
) -> Result<(), MirLowerError> {
    let Some(block_index) = context.current_block else {
        return Err(unsupported("instruction after return"));
    };
    context.blocks[block_index].instructions.push(instruction);
    Ok(())
}

fn set_terminator(
    context: &mut FunctionLowerContext<'_>,
    terminator: MirTerminator,
) -> Result<(), MirLowerError> {
    let Some(block_index) = context.current_block else {
        return Err(unsupported("terminator after return"));
    };
    context.blocks[block_index].terminator = Some(terminator);
    context.current_block = None;
    Ok(())
}

fn finalize_blocks(
    context: FunctionLowerContext<'_>,
    function_name: &str,
) -> Result<Vec<MirBlock>, MirLowerError> {
    context
        .blocks
        .into_iter()
        .map(|block| {
            let Some(terminator) = block.terminator else {
                return Err(MirLowerError::new(format!(
                    "MIR lowering invariant violation: block '{}' in function '{function_name}' has no terminator.",
                    block.label
                )));
            };
            Ok(MirBlock {
                label: block.label,
                instructions: block.instructions,
                terminator,
            })
        })
        .collect()
}

fn require_identifier_value(
    context: &FunctionLowerContext<'_>,
    expression: &Expression,
) -> Result<MirValue, MirLowerError> {
    let Expression::Identifier { name, .. } = expression else {
        return Err(unsupported("non-identifier value"));
    };
    context.values.get(name).cloned().ok_or_else(|| {
        MirLowerError::new(format!(
            "MIR lowering invariant violation: unknown value '{name}'."
        ))
    })
}

fn require_expression_type(
    checked_program: &CheckedProgram,
    expression: &Expression,
) -> Result<CalcKernelType, MirLowerError> {
    get_expr_type(checked_program, expression)
        .cloned()
        .map(|type_node| {
            materialize_integer_literal_type(
                type_node,
                primitive_type(PrimitiveTypeName::I32),
            )
        })
        .ok_or_else(|| {
            MirLowerError::new(format!(
                "MIR lowering invariant violation: missing expression type for expression at line {}.",
                expression.span().start.line
            ))
        })
}

fn require_let_type(
    checked_program: &CheckedProgram,
    statement: &LetStatement,
) -> Result<CalcKernelType, MirLowerError> {
    get_let_type(checked_program, statement)
        .cloned()
        .map(|type_node| {
            materialize_integer_literal_type(type_node, primitive_type(PrimitiveTypeName::I32))
        })
        .ok_or_else(|| {
            MirLowerError::new(format!(
                "MIR lowering invariant violation: missing local type for '{}'.",
                statement.name.name
            ))
        })
}

fn to_mir_type(type_node: &CalcKernelType) -> Result<MirType, MirLowerError> {
    match materialize_integer_literal_type(
        type_node.clone(),
        primitive_type(PrimitiveTypeName::I32),
    ) {
        CalcKernelType::Primitive(PrimitiveTypeName::I32) => {
            Ok(mir_primitive(MirPrimitiveTypeName::I32))
        }
        CalcKernelType::Primitive(PrimitiveTypeName::I64) => {
            Ok(mir_primitive(MirPrimitiveTypeName::I64))
        }
        CalcKernelType::Primitive(PrimitiveTypeName::U32) => {
            Ok(mir_primitive(MirPrimitiveTypeName::U32))
        }
        CalcKernelType::Primitive(PrimitiveTypeName::U64) => {
            Ok(mir_primitive(MirPrimitiveTypeName::U64))
        }
        CalcKernelType::Primitive(PrimitiveTypeName::F64) => {
            Ok(mir_primitive(MirPrimitiveTypeName::F64))
        }
        CalcKernelType::Primitive(PrimitiveTypeName::Bool) => {
            Ok(mir_primitive(MirPrimitiveTypeName::Bool))
        }
        CalcKernelType::Pointer(element_type) => Ok(mir_pointer(to_mir_type(&element_type)?)),
        CalcKernelType::Slice(element_type) => Ok(mir_slice(to_mir_type(&element_type)?)),
        CalcKernelType::Struct(name) => Ok(mir_struct(name)),
        CalcKernelType::Void => Ok(MirType::Void),
        CalcKernelType::IntegerLiteral => Ok(mir_primitive(MirPrimitiveTypeName::I32)),
        CalcKernelType::Unknown => Err(MirLowerError::new(
            "MIR lowering cannot lower unknown type.",
        )),
    }
}

fn binary_op(operator: &str) -> Option<MirBinaryOp> {
    match operator {
        "+" => Some(MirBinaryOp::Add),
        "-" => Some(MirBinaryOp::Sub),
        "*" => Some(MirBinaryOp::Mul),
        "/" => Some(MirBinaryOp::Div),
        "%" => Some(MirBinaryOp::Mod),
        _ => None,
    }
}

fn compare_op(operator: &str) -> Option<MirCompareOp> {
    match operator {
        "==" => Some(MirCompareOp::Eq),
        "!=" => Some(MirCompareOp::Ne),
        "<" => Some(MirCompareOp::Lt),
        "<=" => Some(MirCompareOp::Le),
        ">" => Some(MirCompareOp::Gt),
        ">=" => Some(MirCompareOp::Ge),
        _ => None,
    }
}

fn cast_builtin_op(name: &str) -> Option<MirCastOp> {
    match name {
        "i32_to_f64" => Some(MirCastOp::I32ToF64),
        "u32_to_f64" => Some(MirCastOp::U32ToF64),
        _ => None,
    }
}

fn unsupported(what: impl AsRef<str>) -> MirLowerError {
    MirLowerError::new(format!(
        "MIR scalar lowering does not support {} yet.",
        what.as_ref()
    ))
}
