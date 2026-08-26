use std::{collections::HashMap, error::Error, fmt};

use crate::{
    AssignmentStatement, CalcKernelType, CheckedProgram, Expression, FunctionInfo, LetStatement,
    PrimitiveTypeName, Statement, get_expr_type, get_let_type, materialize_integer_literal_type,
    primitive_type,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirPrimitiveTypeName {
    I32,
    I64,
    U32,
    U64,
    F64,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirType {
    Primitive(MirPrimitiveTypeName),
    Pointer(Box<MirType>),
    Struct(String),
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirModule {
    pub structs: Vec<MirStruct>,
    pub functions: Vec<MirFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStruct {
    pub name: String,
    pub fields: Vec<MirStructField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStructField {
    pub name: String,
    pub type_node: MirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirFunction {
    pub name: String,
    pub exported: bool,
    pub params: Vec<MirParam>,
    pub return_type: MirType,
    pub locals: Vec<MirLocal>,
    pub blocks: Vec<MirBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirParam {
    pub name: String,
    pub type_node: MirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirLocal {
    pub name: String,
    pub type_node: MirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirBlock {
    pub label: String,
    pub instructions: Vec<MirInstruction>,
    pub terminator: MirTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirValue {
    Param { name: String, type_node: MirType },
    Local { name: String, type_node: MirType },
    Temp { name: String, type_node: MirType },
    ConstInt { text: String, type_node: MirType },
    ConstFloat { text: String, type_node: MirType },
    ConstBool { value: bool, type_node: MirType },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirPlace {
    Param {
        name: String,
        type_node: MirType,
    },
    Local {
        name: String,
        type_node: MirType,
    },
    Deref {
        pointer: MirValue,
        type_node: MirType,
    },
    Index {
        base: Box<MirPlace>,
        index: MirValue,
        type_node: MirType,
    },
    Field {
        base: Box<MirPlace>,
        field_name: String,
        type_node: MirType,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirCompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirUnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirCastOp {
    I32ToF64,
    U32ToF64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirInstruction {
    ConstInt {
        target: MirValue,
        value: String,
    },
    ConstFloat {
        target: MirValue,
        value: String,
    },
    ConstBool {
        target: MirValue,
        value: bool,
    },
    Move {
        target: MirValue,
        value: MirValue,
    },
    Binary {
        target: MirValue,
        op: MirBinaryOp,
        left: MirValue,
        right: MirValue,
    },
    Unary {
        target: MirValue,
        op: MirUnaryOp,
        operand: MirValue,
    },
    Compare {
        target: MirValue,
        op: MirCompareOp,
        left: MirValue,
        right: MirValue,
    },
    Cast {
        target: MirValue,
        op: MirCastOp,
        value: MirValue,
    },
    Address {
        target: MirValue,
        place: MirPlace,
    },
    Load {
        target: MirValue,
        place: MirPlace,
    },
    Store {
        place: MirPlace,
        value: MirValue,
    },
    Call {
        target: Option<MirValue>,
        function_name: String,
        args: Vec<MirValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirTerminator {
    Return {
        value: Option<MirValue>,
    },
    Jump {
        label: String,
    },
    Branch {
        condition: MirValue,
        then_label: String,
        else_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirLowerError {
    pub message: String,
}

impl MirLowerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MirLowerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for MirLowerError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirValidationError {
    pub message: String,
    pub function_name: Option<String>,
    pub block_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirValidationResult {
    pub errors: Vec<MirValidationError>,
}

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

#[must_use]
pub fn mir_primitive(name: MirPrimitiveTypeName) -> MirType {
    MirType::Primitive(name)
}

#[must_use]
pub fn mir_pointer(element_type: MirType) -> MirType {
    MirType::Pointer(Box::new(element_type))
}

#[must_use]
pub fn mir_struct(name: impl Into<String>) -> MirType {
    MirType::Struct(name.into())
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
            let base = lower_place(context, object)?;
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
    loop {
        let name = format!("ik_sc{}", context.synthetic_local_counter);
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

pub fn print_mir_module(module: &MirModule) -> String {
    let mut parts = Vec::new();
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

fn print_cast_op(op: MirCastOp) -> &'static str {
    match op {
        MirCastOp::I32ToF64 => "i32_to_f64",
        MirCastOp::U32ToF64 => "u32_to_f64",
    }
}

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
        | MirInstruction::Load { target, .. } => Some(target),
        MirInstruction::Call { target, .. } => target.as_ref(),
        MirInstruction::Store { .. } => None,
    }
}

fn contains_void_type(type_node: &MirType) -> bool {
    match type_node {
        MirType::Void => true,
        MirType::Pointer(element_type) => contains_void_type(element_type),
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
