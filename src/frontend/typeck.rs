use std::collections::HashMap;

use super::{
    AssignmentStatement, BlockStatement, Declaration, Diagnostic, DiagnosticCode, Expression,
    FunctionDeclaration, FunctionParam, IfStatement, LetStatement, ParseResult, Program,
    ReturnStatement, SourceFile, SourceSpan, Statement, StructDeclaration, StructField, TypeNode,
    WhileStatement, parse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTypeName {
    I32,
    I64,
    U32,
    U64,
    F64,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcKernelType {
    Primitive(PrimitiveTypeName),
    Pointer(Box<CalcKernelType>),
    Slice(Box<CalcKernelType>),
    Struct(String),
    Void,
    IntegerLiteral,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableSymbol {
    pub name: String,
    pub type_node: CalcKernelType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructSymbol {
    pub name: String,
    pub declaration: StructDeclaration,
    pub fields: HashMap<String, CalcKernelType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSymbol {
    pub name: String,
    pub declaration: FunctionDeclaration,
    pub params: Vec<CalcKernelType>,
    pub return_type: CalcKernelType,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTable {
    pub structs: HashMap<String, StructSymbol>,
    pub functions: HashMap<String, FunctionSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructFieldInfo {
    pub name: String,
    pub type_node: CalcKernelType,
    pub declaration: StructField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructInfo {
    pub name: String,
    pub declaration: StructDeclaration,
    pub fields: Vec<StructFieldInfo>,
    pub field_map: HashMap<String, StructFieldInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionParamInfo {
    pub name: String,
    pub type_node: CalcKernelType,
    pub declaration: FunctionParam,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    pub name: String,
    pub exported: bool,
    pub declaration: FunctionDeclaration,
    pub params: Vec<FunctionParamInfo>,
    pub return_type: CalcKernelType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedProgram {
    pub ast: Program,
    pub symbols: SymbolTable,
    pub types: TypeMap,
    pub local_types: LetTypeMap,
    pub structs: Vec<StructInfo>,
    pub functions: Vec<FunctionInfo>,
    pub struct_map: HashMap<String, StructInfo>,
    pub function_map: HashMap<String, FunctionInfo>,
}

pub type TypeMap = HashMap<SourceSpan, CalcKernelType>;
pub type LetTypeMap = HashMap<SourceSpan, CalcKernelType>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedAst {
    pub program: Program,
    pub expression_types: TypeMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub ast: Program,
    pub typed_ast: TypedAst,
    pub checked_program: CheckedProgram,
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: SymbolTable,
}

#[must_use]
pub fn check(source: &SourceFile) -> CheckResult {
    let parse_result = parse(source);
    Checker::new(source, parse_result).check()
}

#[derive(Debug, Clone)]
struct CompilerBuiltin {
    name: &'static str,
    params: Vec<CalcKernelType>,
    return_type: CalcKernelType,
}

struct Checker<'source> {
    source: &'source SourceFile,
    program: Program,
    diagnostics: Vec<Diagnostic>,
    symbols: SymbolTable,
    expression_types: TypeMap,
    local_types: LetTypeMap,
    loop_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeUseContext {
    FunctionReturn,
    Parameter,
    StructField,
    Local,
    PointerElement,
    SliceElement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FlowSummary {
    falls_through: bool,
    returns: bool,
    breaks: bool,
    continues: bool,
}

impl FlowSummary {
    const fn falls_through() -> Self {
        Self {
            falls_through: true,
            returns: false,
            breaks: false,
            continues: false,
        }
    }

    const fn returns() -> Self {
        Self {
            falls_through: false,
            returns: true,
            breaks: false,
            continues: false,
        }
    }

    const fn breaks() -> Self {
        Self {
            falls_through: false,
            returns: false,
            breaks: true,
            continues: false,
        }
    }

    const fn continues() -> Self {
        Self {
            falls_through: false,
            returns: false,
            breaks: false,
            continues: true,
        }
    }

    const fn then(self, next: Self) -> Self {
        Self {
            falls_through: next.falls_through,
            returns: self.returns || next.returns,
            breaks: self.breaks || next.breaks,
            continues: self.continues || next.continues,
        }
    }

    const fn union(self, other: Self) -> Self {
        Self {
            falls_through: self.falls_through || other.falls_through,
            returns: self.returns || other.returns,
            breaks: self.breaks || other.breaks,
            continues: self.continues || other.continues,
        }
    }
}

impl<'source> Checker<'source> {
    fn new(source: &'source SourceFile, parse_result: ParseResult) -> Self {
        Self {
            source,
            program: parse_result.ast,
            diagnostics: parse_result.diagnostics,
            symbols: SymbolTable::default(),
            expression_types: HashMap::new(),
            local_types: HashMap::new(),
            loop_depth: 0,
        }
    }

    fn check(mut self) -> CheckResult {
        self.collect_struct_names();
        self.collect_struct_fields();
        self.collect_function_signatures();
        self.check_function_bodies();

        let typed_ast = TypedAst {
            program: self.program.clone(),
            expression_types: self.expression_types.clone(),
        };
        let checked_program = create_checked_program(
            self.program.clone(),
            self.symbols.clone(),
            self.expression_types.clone(),
            self.local_types.clone(),
        );
        CheckResult {
            ast: self.program,
            typed_ast,
            checked_program,
            diagnostics: self.diagnostics,
            symbols: self.symbols,
        }
    }

    fn collect_struct_names(&mut self) {
        for declaration in self.program.declarations.clone() {
            let Declaration::Struct(struct_decl) = declaration else {
                continue;
            };
            let name = struct_decl.name.name.clone();
            if self.symbols.structs.contains_key(&name) {
                self.error(struct_decl.name.span, format!("Duplicate struct '{name}'."));
                continue;
            }
            self.symbols.structs.insert(
                name.clone(),
                StructSymbol {
                    name,
                    declaration: struct_decl,
                    fields: HashMap::new(),
                },
            );
        }
    }

    fn collect_struct_fields(&mut self) {
        for declaration in self.program.declarations.clone() {
            let Declaration::Struct(struct_decl) = declaration else {
                continue;
            };
            let Some(existing_symbol) = self.symbols.structs.get(&struct_decl.name.name) else {
                continue;
            };
            if existing_symbol.declaration != struct_decl {
                continue;
            }

            let mut resolved_fields = Vec::new();
            let mut duplicate_errors = Vec::new();
            let mut field_names = HashMap::<String, SourceSpan>::new();
            for field in &struct_decl.fields {
                if field_names
                    .insert(field.name.name.clone(), field.name.span)
                    .is_some()
                {
                    duplicate_errors.push((
                        field.name.span,
                        format!(
                            "Duplicate field '{}' in struct '{}'.",
                            field.name.name, struct_decl.name.name
                        ),
                    ));
                    continue;
                }
                let field_type = self.resolve_type(&field.type_node, TypeUseContext::StructField);
                resolved_fields.push((field.name.name.clone(), field_type));
            }

            if let Some(symbol) = self.symbols.structs.get_mut(&struct_decl.name.name) {
                for (name, field_type) in resolved_fields {
                    symbol.fields.insert(name, field_type);
                }
            }
            for (span, message) in duplicate_errors {
                self.error(span, message);
            }
        }
    }

    fn collect_function_signatures(&mut self) {
        for declaration in self.program.declarations.clone() {
            let Declaration::Function(function_decl) = declaration else {
                continue;
            };
            let name = function_decl.name.name.clone();
            if compiler_builtin(&name).is_some() {
                self.error(
                    function_decl.name.span,
                    format!("Cannot define reserved compiler builtin '{name}'."),
                );
                continue;
            }
            if self.symbols.functions.contains_key(&name) {
                self.error(
                    function_decl.name.span,
                    format!("Duplicate function '{name}'."),
                );
                continue;
            }
            let params = function_decl
                .params
                .iter()
                .map(|param| self.resolve_type(&param.type_node, TypeUseContext::Parameter))
                .collect();
            let return_type =
                self.resolve_type(&function_decl.return_type, TypeUseContext::FunctionReturn);
            if function_decl.exported && matches!(return_type, CalcKernelType::Slice(_)) {
                self.error_with_code(
                    function_decl.return_type.span(),
                    DiagnosticCode::Ck2012,
                    "An exported function cannot return a slice.",
                );
            }
            self.symbols.functions.insert(
                name.clone(),
                FunctionSymbol {
                    name,
                    declaration: function_decl,
                    params,
                    return_type,
                },
            );
        }
    }

    fn check_function_bodies(&mut self) {
        for declaration in self.program.declarations.clone() {
            let Declaration::Function(function_decl) = declaration else {
                continue;
            };
            let Some(function_symbol) = self
                .symbols
                .functions
                .get(&function_decl.name.name)
                .cloned()
            else {
                continue;
            };
            if function_symbol.declaration != function_decl {
                continue;
            }
            self.check_function_body(&function_decl, &function_symbol);
        }
    }

    fn check_function_body(
        &mut self,
        declaration: &FunctionDeclaration,
        function_symbol: &FunctionSymbol,
    ) {
        let mut scope = Scope::default();
        for (index, param) in declaration.params.iter().enumerate() {
            let name = param.name.name.clone();
            let type_node = function_symbol
                .params
                .get(index)
                .cloned()
                .unwrap_or(CalcKernelType::Unknown);
            if !scope.declare(VariableSymbol {
                name: name.clone(),
                type_node,
            }) {
                self.error(param.name.span, format!("Duplicate variable '{name}'."));
            }
        }

        self.loop_depth = 0;
        let flow = self.check_block(
            &declaration.body,
            &mut scope,
            &function_symbol.return_type,
            false,
        );
        if flow.falls_through && !matches!(function_symbol.return_type, CalcKernelType::Void) {
            self.error(
                declaration.body.span,
                format!("Missing return in function '{}'.", declaration.name.name),
            );
        }
    }

    fn check_block(
        &mut self,
        block: &BlockStatement,
        scope: &mut Scope,
        return_type: &CalcKernelType,
        create_scope: bool,
    ) -> FlowSummary {
        if create_scope {
            scope.push();
        }
        let mut flow = FlowSummary::falls_through();
        for statement in &block.statements {
            if !flow.falls_through {
                self.error_with_code(
                    statement.span(),
                    DiagnosticCode::Ck2010,
                    "Unreachable statement.",
                );
                continue;
            }
            flow = flow.then(self.check_statement(statement, scope, return_type));
        }
        if create_scope {
            scope.pop();
        }
        flow
    }

    fn check_statement(
        &mut self,
        statement: &Statement,
        scope: &mut Scope,
        return_type: &CalcKernelType,
    ) -> FlowSummary {
        match statement {
            Statement::Block(block) => self.check_block(block, scope, return_type, true),
            Statement::Let(statement) => {
                self.check_let_statement(statement, scope);
                FlowSummary::falls_through()
            }
            Statement::Assignment(statement) => {
                self.check_assignment_statement(statement, scope);
                FlowSummary::falls_through()
            }
            Statement::Call(statement) => {
                self.check_call_statement(&statement.call, scope);
                FlowSummary::falls_through()
            }
            Statement::Return(statement) => {
                self.check_return_statement(statement, scope, return_type);
                FlowSummary::returns()
            }
            Statement::Break(statement) => {
                if self.loop_depth == 0 {
                    self.error_with_code(
                        statement.span,
                        DiagnosticCode::Ck2009,
                        "'break' can only be used inside a while loop.",
                    );
                    FlowSummary::falls_through()
                } else {
                    FlowSummary::breaks()
                }
            }
            Statement::Continue(statement) => {
                if self.loop_depth == 0 {
                    self.error_with_code(
                        statement.span,
                        DiagnosticCode::Ck2009,
                        "'continue' can only be used inside a while loop.",
                    );
                    FlowSummary::falls_through()
                } else {
                    FlowSummary::continues()
                }
            }
            Statement::If(statement) => self.check_if_statement(statement, scope, return_type),
            Statement::While(statement) => {
                self.check_while_statement(statement, scope, return_type)
            }
            Statement::Error { .. } => FlowSummary::falls_through(),
        }
    }

    fn check_let_statement(&mut self, statement: &LetStatement, scope: &mut Scope) {
        let declared_type = self.resolve_type(&statement.type_node, TypeUseContext::Local);
        self.local_types
            .insert(statement.span, declared_type.clone());
        if !scope.declare(VariableSymbol {
            name: statement.name.name.clone(),
            type_node: declared_type.clone(),
        }) {
            self.error(
                statement.name.span,
                format!("Duplicate variable '{}'.", statement.name.name),
            );
        }

        let initializer_type =
            self.check_expression(&statement.initializer, scope, Some(&declared_type));
        if !is_unknown(&declared_type)
            && !is_unknown(&initializer_type)
            && !can_assign(&declared_type, &initializer_type)
        {
            self.type_mismatch_error(
                statement.initializer.span(),
                &declared_type,
                &initializer_type,
                format!(
                    "Cannot initialize '{}': expected {} but got {}.",
                    statement.name.name,
                    type_to_string(&declared_type),
                    type_to_string(&initializer_type)
                ),
            );
        }
    }

    fn check_assignment_statement(&mut self, statement: &AssignmentStatement, scope: &mut Scope) {
        if !is_assignable_expression(&statement.target) {
            self.error(statement.target.span(), "Invalid assignment target.");
        }

        let target_type = self.check_expression(&statement.target, scope, None);
        if let Expression::Field { object, field, .. } = &statement.target
            && matches!(
                self.expression_types.get(&object.span()),
                Some(CalcKernelType::Slice(_))
            )
            && matches!(field.name.as_str(), "data" | "len")
        {
            self.error_with_code(
                statement.target.span(),
                DiagnosticCode::Ck2012,
                "Slice '.data' and '.len' projections are read-only.",
            );
        }
        let value_type = self.check_expression(&statement.value, scope, Some(&target_type));
        if !is_unknown(&target_type)
            && !is_unknown(&value_type)
            && !can_assign(&target_type, &value_type)
        {
            self.type_mismatch_error(
                statement.value.span(),
                &target_type,
                &value_type,
                format!(
                    "Cannot assign {} to {}.",
                    type_to_string(&value_type),
                    type_to_string(&target_type)
                ),
            );
        }
    }

    fn check_return_statement(
        &mut self,
        statement: &ReturnStatement,
        scope: &mut Scope,
        return_type: &CalcKernelType,
    ) {
        match (return_type, &statement.value) {
            (CalcKernelType::Void, None) => {}
            (CalcKernelType::Void, Some(value)) => {
                self.check_expression(value, scope, None);
                self.error_with_code(
                    value.span(),
                    DiagnosticCode::Ck2011,
                    "A void function cannot return a value.",
                );
            }
            (_, None) => self.error_with_code(
                statement.span,
                DiagnosticCode::Ck2011,
                "A non-void function must return a value.",
            ),
            (_, Some(value)) => {
                let value_type = self.check_expression(value, scope, Some(return_type));
                if !is_unknown(return_type)
                    && !is_unknown(&value_type)
                    && !can_assign(return_type, &value_type)
                {
                    self.type_mismatch_error(
                        value.span(),
                        return_type,
                        &value_type,
                        format!(
                            "Return type mismatch: expected {} but got {}.",
                            type_to_string(return_type),
                            type_to_string(&value_type)
                        ),
                    );
                }
            }
        }
    }

    fn check_call_statement(&mut self, call: &Expression, scope: &mut Scope) {
        let Expression::Call { callee, args, span } = call else {
            self.error_with_code(
                call.span(),
                DiagnosticCode::Ck2011,
                "Only a function call may be used as a standalone statement.",
            );
            return;
        };
        let return_type = self.check_call_expression(callee, args, *span, scope, false);
        self.expression_types.insert(*span, return_type.clone());
        if !is_unknown(&return_type) && !matches!(return_type, CalcKernelType::Void) {
            self.error_with_code(
                *span,
                DiagnosticCode::Ck2011,
                "Only a void call may be used as a standalone statement.",
            );
        }
    }

    fn check_if_statement(
        &mut self,
        statement: &IfStatement,
        scope: &mut Scope,
        return_type: &CalcKernelType,
    ) -> FlowSummary {
        let condition_type = materialize_integer_literal(
            self.check_expression(&statement.condition, scope, None),
            primitive_i32(),
        );
        if !is_unknown(&condition_type) && !is_bool(&condition_type) {
            self.error(
                statement.condition.span(),
                format!(
                    "If condition must be bool, got {}.",
                    type_to_string(&condition_type)
                ),
            );
        }
        let then_flow = self.check_block(&statement.then_block, scope, return_type, true);
        let else_flow = statement
            .else_block
            .as_ref()
            .map_or_else(FlowSummary::falls_through, |else_block| {
                self.check_block(else_block, scope, return_type, true)
            });
        then_flow.union(else_flow)
    }

    fn check_while_statement(
        &mut self,
        statement: &WhileStatement,
        scope: &mut Scope,
        return_type: &CalcKernelType,
    ) -> FlowSummary {
        let condition_type = materialize_integer_literal(
            self.check_expression(&statement.condition, scope, None),
            primitive_i32(),
        );
        if !is_unknown(&condition_type) && !is_bool(&condition_type) {
            self.error(
                statement.condition.span(),
                format!(
                    "While condition must be bool, got {}.",
                    type_to_string(&condition_type)
                ),
            );
        }
        self.loop_depth += 1;
        let body_flow = self.check_block(&statement.body, scope, return_type, true);
        self.loop_depth -= 1;
        FlowSummary {
            falls_through: true,
            returns: body_flow.returns,
            breaks: false,
            continues: false,
        }
    }

    fn check_expression(
        &mut self,
        expression: &Expression,
        scope: &mut Scope,
        expected_type: Option<&CalcKernelType>,
    ) -> CalcKernelType {
        let type_node = match expression {
            Expression::Identifier { name, span } => {
                if let Some(symbol) = scope.lookup(name) {
                    symbol.type_node.clone()
                } else {
                    self.error(*span, format!("Unknown variable '{name}'."));
                    CalcKernelType::Unknown
                }
            }
            Expression::IntegerLiteral { .. } => {
                if let Some(expected) = expected_type.filter(|type_node| is_integer(type_node)) {
                    expected.clone()
                } else {
                    CalcKernelType::IntegerLiteral
                }
            }
            Expression::FloatLiteral { .. } => primitive_f64(),
            Expression::BoolLiteral { .. } => primitive_bool(),
            Expression::Unary {
                operator, operand, ..
            } => self.check_unary_expression(operator, operand, scope, expected_type),
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => self.check_binary_expression(operator, left, right, *span, scope, expected_type),
            Expression::Call { callee, args, span } => {
                self.check_call_expression(callee, args, *span, scope, true)
            }
            Expression::SliceConstructor { data, len, .. } => {
                self.check_slice_constructor(data, len, scope)
            }
            Expression::Field {
                object,
                field,
                span: _,
            } => self.check_field_expression(object, field, scope),
            Expression::Index { object, index, .. } => {
                self.check_index_expression(object, index, scope)
            }
            Expression::Subslice {
                slice, start, end, ..
            } => self.check_subslice_expression(slice, start, end, scope),
            Expression::Parenthesized { expression, .. } => {
                self.check_expression(expression, scope, expected_type)
            }
            Expression::Error { .. } => CalcKernelType::Unknown,
        };
        self.expression_types
            .insert(expression.span(), type_node.clone());
        type_node
    }

    fn check_unary_expression(
        &mut self,
        operator: &str,
        operand: &Expression,
        scope: &mut Scope,
        expected_type: Option<&CalcKernelType>,
    ) -> CalcKernelType {
        if operator == "!" {
            let operand_type = materialize_integer_literal(
                self.check_expression(operand, scope, None),
                primitive_i32(),
            );
            if !is_unknown(&operand_type) && !is_bool(&operand_type) {
                self.error(
                    operand.span(),
                    format!(
                        "Unary operator '!' requires bool operand, got {}.",
                        type_to_string(&operand_type)
                    ),
                );
            }
            return primitive_bool();
        }

        let fallback = integer_literal_fallback(expected_type);
        let operand_type = materialize_integer_literal(
            self.check_expression(operand, scope, Some(&fallback)),
            fallback.clone(),
        );
        if !is_unknown(&operand_type) && !is_numeric_type(&operand_type) {
            self.error(
                operand.span(),
                format!(
                    "Unary operator '-' requires integer operand, got {}.",
                    type_to_string(&operand_type)
                ),
            );
            return CalcKernelType::Unknown;
        }
        materialize_integer_literal(operand_type, fallback)
    }

    fn check_binary_expression(
        &mut self,
        operator: &str,
        left: &Expression,
        right: &Expression,
        span: SourceSpan,
        scope: &mut Scope,
        expected_type: Option<&CalcKernelType>,
    ) -> CalcKernelType {
        if is_arithmetic_operator(operator) {
            return self.check_arithmetic_expression(
                operator,
                left,
                right,
                span,
                scope,
                expected_type,
            );
        }
        if is_comparison_operator(operator) {
            return self.check_comparison_expression(operator, left, right, span, scope);
        }
        if operator == "&&" || operator == "||" {
            let left_type = materialize_integer_literal(
                self.check_expression(left, scope, None),
                primitive_i32(),
            );
            let right_type = materialize_integer_literal(
                self.check_expression(right, scope, None),
                primitive_i32(),
            );
            if !is_unknown(&left_type) && !is_bool(&left_type) {
                self.error(
                    left.span(),
                    format!("Logical operator '{operator}' requires bool operands."),
                );
            }
            if !is_unknown(&right_type) && !is_bool(&right_type) {
                self.error(
                    right.span(),
                    format!("Logical operator '{operator}' requires bool operands."),
                );
            }
            return primitive_bool();
        }
        CalcKernelType::Unknown
    }

    fn check_arithmetic_expression(
        &mut self,
        operator: &str,
        left: &Expression,
        right: &Expression,
        span: SourceSpan,
        scope: &mut Scope,
        expected_type: Option<&CalcKernelType>,
    ) -> CalcKernelType {
        let left_raw = self.check_expression(left, scope, None);
        let right_raw = self.check_expression(right, scope, None);
        if is_slice(&left_raw) || is_slice(&right_raw) {
            self.error_with_code(
                span,
                DiagnosticCode::Ck2012,
                format!("Slice values do not support arithmetic operator '{operator}'."),
            );
            return CalcKernelType::Unknown;
        }
        let fallback = integer_literal_fallback(expected_type);
        let left_type = materialize_integer_literal(
            left_raw,
            if matches!(right_raw, CalcKernelType::IntegerLiteral) {
                fallback.clone()
            } else {
                integer_literal_fallback(Some(&right_raw))
            },
        );
        let right_type =
            materialize_integer_literal(right_raw, integer_literal_fallback(Some(&left_type)));
        self.expression_types.insert(left.span(), left_type.clone());
        self.expression_types
            .insert(right.span(), right_type.clone());

        if operator == "%" && (is_float_type(&left_type) || is_float_type(&right_type)) {
            self.error(
                span,
                "Arithmetic operator '%' does not support f64 operands.",
            );
            return CalcKernelType::Unknown;
        }

        if !is_unknown(&left_type)
            && !is_unknown(&right_type)
            && (!is_numeric_type(&left_type)
                || !is_numeric_type(&right_type)
                || !same_type(&left_type, &right_type))
        {
            self.error(
                span,
                format!(
                    "Arithmetic operator '{operator}' requires integer operands of the same type."
                ),
            );
            return CalcKernelType::Unknown;
        }

        materialize_integer_literal(left_type, fallback)
    }

    fn check_comparison_expression(
        &mut self,
        operator: &str,
        left: &Expression,
        right: &Expression,
        span: SourceSpan,
        scope: &mut Scope,
    ) -> CalcKernelType {
        let left_raw = self.check_expression(left, scope, None);
        let right_raw = self.check_expression(right, scope, None);
        if is_slice(&left_raw) || is_slice(&right_raw) {
            self.error_with_code(
                span,
                DiagnosticCode::Ck2012,
                format!("Slice values do not support comparison operator '{operator}'."),
            );
            return primitive_bool();
        }
        let left_type = materialize_integer_literal(
            left_raw,
            if matches!(right_raw, CalcKernelType::IntegerLiteral) {
                primitive_i32()
            } else {
                integer_literal_fallback(Some(&right_raw))
            },
        );
        let right_type =
            materialize_integer_literal(right_raw, integer_literal_fallback(Some(&left_type)));
        self.expression_types.insert(left.span(), left_type.clone());
        self.expression_types
            .insert(right.span(), right_type.clone());
        let valid = if operator == "==" || operator == "!=" {
            same_type(&left_type, &right_type)
        } else {
            is_numeric_type(&left_type)
                && is_numeric_type(&right_type)
                && same_type(&left_type, &right_type)
        };

        if !is_unknown(&left_type) && !is_unknown(&right_type) && !valid {
            self.error(
                span,
                format!("Comparison operator '{operator}' requires compatible operands."),
            );
        }
        primitive_bool()
    }

    fn check_call_expression(
        &mut self,
        callee: &Expression,
        args: &[Expression],
        span: SourceSpan,
        scope: &mut Scope,
        value_required: bool,
    ) -> CalcKernelType {
        let Expression::Identifier {
            name,
            span: name_span,
        } = callee
        else {
            self.error(callee.span(), "Can only call functions by name.");
            for arg in args {
                self.check_expression(arg, scope, None);
            }
            return CalcKernelType::Unknown;
        };

        if let Some(builtin) = compiler_builtin(name) {
            return self.check_builtin_call(&builtin, args, span, scope);
        }

        let Some(function_symbol) = self.symbols.functions.get(name).cloned() else {
            self.error(*name_span, format!("Unknown function '{name}'."));
            for arg in args {
                self.check_expression(arg, scope, None);
            }
            return CalcKernelType::Unknown;
        };

        if args.len() != function_symbol.params.len() {
            self.error(
                span,
                format!(
                    "Function '{}' expects {} argument{} but got {}.",
                    function_symbol.name,
                    function_symbol.params.len(),
                    if function_symbol.params.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    args.len()
                ),
            );
        }

        for (index, arg) in args.iter().enumerate() {
            let expected = function_symbol.params.get(index);
            let arg_type = self.check_expression(arg, scope, expected);
            if let Some(expected) = expected
                && !is_unknown(expected)
                && !is_unknown(&arg_type)
                && !can_assign(expected, &arg_type)
            {
                self.type_mismatch_error(
                    arg.span(),
                    expected,
                    &arg_type,
                    format!(
                        "Argument {} of function '{}' expects {} but got {}.",
                        index + 1,
                        function_symbol.name,
                        type_to_string(expected),
                        type_to_string(&arg_type)
                    ),
                );
            }
        }

        if value_required && matches!(function_symbol.return_type, CalcKernelType::Void) {
            self.error_with_code(
                span,
                DiagnosticCode::Ck2011,
                "A void call cannot be used where a value is required.",
            );
            CalcKernelType::Unknown
        } else {
            function_symbol.return_type
        }
    }

    fn check_builtin_call(
        &mut self,
        builtin: &CompilerBuiltin,
        args: &[Expression],
        span: SourceSpan,
        scope: &mut Scope,
    ) -> CalcKernelType {
        if args.len() != builtin.params.len() {
            self.error(
                span,
                format!(
                    "Compiler builtin '{}' expects {} argument{} but got {}.",
                    builtin.name,
                    builtin.params.len(),
                    if builtin.params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            );
        }

        for (index, arg) in args.iter().enumerate() {
            let expected = builtin.params.get(index);
            let arg_type = self.check_expression(arg, scope, expected);
            if let Some(expected) = expected
                && !is_unknown(expected)
                && !is_unknown(&arg_type)
                && !can_assign(expected, &arg_type)
            {
                self.type_mismatch_error(
                    arg.span(),
                    expected,
                    &arg_type,
                    format!(
                        "Argument {} of compiler builtin '{}' expects {} but got {}.",
                        index + 1,
                        builtin.name,
                        type_to_string(expected),
                        type_to_string(&arg_type)
                    ),
                );
            }
        }

        builtin.return_type.clone()
    }

    fn check_field_expression(
        &mut self,
        object: &Expression,
        field: &crate::IdentifierNode,
        scope: &mut Scope,
    ) -> CalcKernelType {
        let object_type = self.check_expression(object, scope, None);
        if let CalcKernelType::Slice(element_type) = object_type {
            return match field.name.as_str() {
                "data" => CalcKernelType::Pointer(element_type),
                "len" => primitive_u32(),
                _ => {
                    self.error_with_code(
                        field.span,
                        DiagnosticCode::Ck2012,
                        format!("A slice has no projection named '{}'.", field.name),
                    );
                    CalcKernelType::Unknown
                }
            };
        }
        let CalcKernelType::Struct(struct_name) = object_type else {
            if !is_unknown(&object_type) {
                self.error(
                    object.span(),
                    format!(
                        "Field access requires struct value, got {}.",
                        type_to_string(&object_type)
                    ),
                );
            }
            return CalcKernelType::Unknown;
        };

        let Some(struct_symbol) = self.symbols.structs.get(&struct_name) else {
            self.error(
                field.span,
                format!("Struct '{struct_name}' has no field '{}'.", field.name),
            );
            return CalcKernelType::Unknown;
        };
        let Some(field_type) = struct_symbol.fields.get(&field.name) else {
            self.error(
                field.span,
                format!("Struct '{struct_name}' has no field '{}'.", field.name),
            );
            return CalcKernelType::Unknown;
        };
        field_type.clone()
    }

    fn check_index_expression(
        &mut self,
        object: &Expression,
        index: &Expression,
        scope: &mut Scope,
    ) -> CalcKernelType {
        let object_type = self.check_expression(object, scope, None);
        match object_type {
            CalcKernelType::Slice(element_type) => {
                self.check_slice_u32_operand("index", index, scope);
                *element_type
            }
            CalcKernelType::Pointer(element_type) => {
                let index_type = materialize_integer_literal(
                    self.check_expression(index, scope, None),
                    primitive_i32(),
                );
                if !is_unknown(&index_type) && !is_index_integer(&index_type) {
                    self.error(
                        index.span(),
                        format!(
                            "Index expression requires i32 or u32 index, got {}.",
                            type_to_string(&index_type)
                        ),
                    );
                }
                *element_type
            }
            other => {
                self.check_expression(index, scope, None);
                if !is_unknown(&other) {
                    self.error(
                        object.span(),
                        format!(
                            "Index access requires pointer or slice value, got {}.",
                            type_to_string(&other)
                        ),
                    );
                }
                CalcKernelType::Unknown
            }
        }
    }

    fn check_slice_constructor(
        &mut self,
        data: &Expression,
        len: &Expression,
        scope: &mut Scope,
    ) -> CalcKernelType {
        let data_type = self.check_expression(data, scope, None);
        let element_type = match data_type {
            CalcKernelType::Pointer(element_type) => *element_type,
            CalcKernelType::Unknown => CalcKernelType::Unknown,
            other => {
                self.error_with_code(
                    data.span(),
                    DiagnosticCode::Ck2012,
                    format!(
                        "Slice construction requires a raw pointer, got {}.",
                        type_to_string(&other)
                    ),
                );
                CalcKernelType::Unknown
            }
        };
        self.check_slice_u32_operand("length", len, scope);
        if is_unknown(&element_type) {
            CalcKernelType::Unknown
        } else {
            CalcKernelType::Slice(Box::new(element_type))
        }
    }

    fn check_subslice_expression(
        &mut self,
        slice: &Expression,
        start: &Expression,
        end: &Expression,
        scope: &mut Scope,
    ) -> CalcKernelType {
        let slice_type = self.check_expression(slice, scope, None);
        self.check_slice_u32_operand("start", start, scope);
        self.check_slice_u32_operand("end", end, scope);
        if matches!(slice_type, CalcKernelType::Slice(_)) {
            slice_type
        } else {
            if !is_unknown(&slice_type) {
                self.error_with_code(
                    slice.span(),
                    DiagnosticCode::Ck2012,
                    format!(
                        "Sub-slicing requires a slice value, got {}.",
                        type_to_string(&slice_type)
                    ),
                );
            }
            CalcKernelType::Unknown
        }
    }

    fn check_slice_u32_operand(
        &mut self,
        role: &str,
        expression: &Expression,
        scope: &mut Scope,
    ) -> CalcKernelType {
        let type_node = self.check_expression(expression, scope, Some(&primitive_u32()));
        let invalid_literal = invalid_u32_literal(expression);
        if let Some(text) = invalid_literal {
            self.error_with_code(
                expression.span(),
                DiagnosticCode::Ck2012,
                format!("Slice {role} literal '{text}' is not materializable as u32."),
            );
            return CalcKernelType::Unknown;
        }
        if !is_unknown(&type_node) && type_node != primitive_u32() {
            self.error_with_code(
                expression.span(),
                DiagnosticCode::Ck2012,
                format!(
                    "Slice {role} requires u32, got {}.",
                    type_to_string(&type_node)
                ),
            );
            return CalcKernelType::Unknown;
        }
        type_node
    }

    fn resolve_type(&mut self, type_node: &TypeNode, context: TypeUseContext) -> CalcKernelType {
        match type_node {
            TypeNode::Primitive { name, .. } => primitive_type_from_str(name),
            TypeNode::Void { .. } if context == TypeUseContext::FunctionReturn => {
                CalcKernelType::Void
            }
            TypeNode::Void { span } => {
                let message = if context == TypeUseContext::PointerElement {
                    "Void is not allowed as a pointer element type."
                } else {
                    "Void is only allowed as a function return type."
                };
                self.error_with_code(*span, DiagnosticCode::Ck2011, message);
                CalcKernelType::Unknown
            }
            TypeNode::Pointer { element_type, .. } => CalcKernelType::Pointer(Box::new(
                self.resolve_type(element_type, TypeUseContext::PointerElement),
            )),
            TypeNode::Slice { element_type, .. } => {
                if matches!(element_type.as_ref(), TypeNode::Void { .. }) {
                    self.error_with_code(
                        element_type.span(),
                        DiagnosticCode::Ck2012,
                        "A slice element type cannot be void.",
                    );
                    return CalcKernelType::Unknown;
                }
                if matches!(element_type.as_ref(), TypeNode::Slice { .. }) {
                    self.error_with_code(
                        element_type.span(),
                        DiagnosticCode::Ck2012,
                        "A direct slice element type cannot itself be a slice.",
                    );
                    return CalcKernelType::Unknown;
                }
                let element_type = self.resolve_type(element_type, TypeUseContext::SliceElement);
                if !is_unknown(&element_type) && !is_valid_slice_element(&element_type) {
                    self.error_with_code(
                        type_node.span(),
                        DiagnosticCode::Ck2012,
                        "Invalid slice element type.",
                    );
                    CalcKernelType::Unknown
                } else if is_unknown(&element_type) {
                    CalcKernelType::Unknown
                } else {
                    CalcKernelType::Slice(Box::new(element_type))
                }
            }
            TypeNode::Named { name, .. } => {
                if !self.symbols.structs.contains_key(&name.name) {
                    self.error(name.span, format!("Unknown type '{}'.", name.name));
                    return CalcKernelType::Unknown;
                }
                CalcKernelType::Struct(name.name.clone())
            }
            TypeNode::Error { .. } => CalcKernelType::Unknown,
        }
    }

    fn error(&mut self, span: SourceSpan, message: impl Into<String>) {
        let message = message.into();
        self.error_with_code(span, checker_diagnostic_code(&message), message);
    }

    fn type_mismatch_error(
        &mut self,
        span: SourceSpan,
        expected: &CalcKernelType,
        actual: &CalcKernelType,
        message: impl Into<String>,
    ) {
        if is_slice(expected) || is_slice(actual) {
            self.error_with_code(span, DiagnosticCode::Ck2012, message);
        } else {
            self.error(span, message);
        }
    }

    fn error_with_code(
        &mut self,
        span: SourceSpan,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(Diagnostic::error(
            code,
            message,
            self.source.file_name.clone(),
            span,
        ));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    frames: Vec<HashMap<String, VariableSymbol>>,
}

impl Scope {
    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn declare(&mut self, variable: VariableSymbol) -> bool {
        if self.frames.is_empty() {
            self.push();
        }
        let frame = self
            .frames
            .last_mut()
            .expect("scope has at least one frame");
        if frame.contains_key(&variable.name) {
            return false;
        }
        frame.insert(variable.name.clone(), variable);
        true
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&VariableSymbol> {
        self.frames.iter().rev().find_map(|frame| frame.get(name))
    }
}

fn create_checked_program(
    ast: Program,
    symbols: SymbolTable,
    types: TypeMap,
    local_types: LetTypeMap,
) -> CheckedProgram {
    let structs: Vec<StructInfo> = ast
        .declarations
        .iter()
        .filter_map(|declaration| {
            let Declaration::Struct(struct_decl) = declaration else {
                return None;
            };
            let symbol = symbols.structs.get(&struct_decl.name.name)?;
            (symbol.declaration == *struct_decl).then(|| to_struct_info(symbol))
        })
        .collect();
    let functions: Vec<FunctionInfo> = ast
        .declarations
        .iter()
        .filter_map(|declaration| {
            let Declaration::Function(function_decl) = declaration else {
                return None;
            };
            let symbol = symbols.functions.get(&function_decl.name.name)?;
            (symbol.declaration == *function_decl).then(|| to_function_info(symbol))
        })
        .collect();
    CheckedProgram {
        ast,
        symbols,
        types,
        local_types,
        struct_map: structs
            .iter()
            .cloned()
            .map(|struct_info| (struct_info.name.clone(), struct_info))
            .collect(),
        function_map: functions
            .iter()
            .cloned()
            .map(|function_info| (function_info.name.clone(), function_info))
            .collect(),
        structs,
        functions,
    }
}

fn to_struct_info(symbol: &StructSymbol) -> StructInfo {
    let fields: Vec<StructFieldInfo> = symbol
        .declaration
        .fields
        .iter()
        .filter_map(|field| {
            symbol
                .fields
                .get(&field.name.name)
                .map(|type_node| StructFieldInfo {
                    name: field.name.name.clone(),
                    type_node: type_node.clone(),
                    declaration: field.clone(),
                })
        })
        .collect();
    StructInfo {
        name: symbol.name.clone(),
        declaration: symbol.declaration.clone(),
        field_map: fields
            .iter()
            .cloned()
            .map(|field| (field.name.clone(), field))
            .collect(),
        fields,
    }
}

fn to_function_info(symbol: &FunctionSymbol) -> FunctionInfo {
    FunctionInfo {
        name: symbol.name.clone(),
        exported: symbol.declaration.exported,
        declaration: symbol.declaration.clone(),
        params: symbol
            .declaration
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| FunctionParamInfo {
                name: param.name.name.clone(),
                type_node: symbol
                    .params
                    .get(index)
                    .cloned()
                    .unwrap_or(CalcKernelType::Unknown),
                declaration: param.clone(),
            })
            .collect(),
        return_type: symbol.return_type.clone(),
    }
}

fn compiler_builtin(name: &str) -> Option<CompilerBuiltin> {
    match name {
        "i32_to_f64" => Some(CompilerBuiltin {
            name: "i32_to_f64",
            params: vec![primitive_i32()],
            return_type: primitive_f64(),
        }),
        "u32_to_f64" => Some(CompilerBuiltin {
            name: "u32_to_f64",
            params: vec![primitive_u32()],
            return_type: primitive_f64(),
        }),
        _ => None,
    }
}

#[must_use]
pub fn get_expr_type<'program>(
    checked_program: &'program CheckedProgram,
    expression: &Expression,
) -> Option<&'program CalcKernelType> {
    checked_program.types.get(&expression.span())
}

#[must_use]
pub fn get_let_type<'program>(
    checked_program: &'program CheckedProgram,
    statement: &LetStatement,
) -> Option<&'program CalcKernelType> {
    checked_program.local_types.get(&statement.span)
}

#[must_use]
pub fn get_struct_info<'program>(
    checked_program: &'program CheckedProgram,
    name: &str,
) -> Option<&'program StructInfo> {
    checked_program.struct_map.get(name)
}

#[must_use]
pub fn get_field_info<'program>(
    checked_program: &'program CheckedProgram,
    struct_name: &str,
    field_name: &str,
) -> Option<&'program StructFieldInfo> {
    checked_program
        .struct_map
        .get(struct_name)?
        .field_map
        .get(field_name)
}

#[must_use]
pub fn get_function_info<'program>(
    checked_program: &'program CheckedProgram,
    name: &str,
) -> Option<&'program FunctionInfo> {
    checked_program.function_map.get(name)
}

#[must_use]
pub fn primitive_type(name: PrimitiveTypeName) -> CalcKernelType {
    CalcKernelType::Primitive(name)
}

#[must_use]
pub fn materialize_integer_literal_type(
    type_node: CalcKernelType,
    fallback: CalcKernelType,
) -> CalcKernelType {
    materialize_integer_literal(type_node, fallback)
}

fn primitive_type_from_str(name: &str) -> CalcKernelType {
    match name {
        "i32" => primitive_i32(),
        "i64" => primitive_i64(),
        "u32" => primitive_u32(),
        "u64" => primitive_u64(),
        "f64" => primitive_f64(),
        "bool" => primitive_bool(),
        _ => CalcKernelType::Unknown,
    }
}

fn primitive_i32() -> CalcKernelType {
    CalcKernelType::Primitive(PrimitiveTypeName::I32)
}

fn primitive_i64() -> CalcKernelType {
    CalcKernelType::Primitive(PrimitiveTypeName::I64)
}

fn primitive_u32() -> CalcKernelType {
    CalcKernelType::Primitive(PrimitiveTypeName::U32)
}

fn primitive_u64() -> CalcKernelType {
    CalcKernelType::Primitive(PrimitiveTypeName::U64)
}

fn primitive_f64() -> CalcKernelType {
    CalcKernelType::Primitive(PrimitiveTypeName::F64)
}

fn primitive_bool() -> CalcKernelType {
    CalcKernelType::Primitive(PrimitiveTypeName::Bool)
}

fn is_unknown(type_node: &CalcKernelType) -> bool {
    matches!(type_node, CalcKernelType::Unknown)
}

fn is_bool(type_node: &CalcKernelType) -> bool {
    matches!(
        type_node,
        CalcKernelType::Primitive(PrimitiveTypeName::Bool)
    )
}

fn is_integer_primitive(type_node: &CalcKernelType) -> bool {
    matches!(
        type_node,
        CalcKernelType::Primitive(
            PrimitiveTypeName::I32
                | PrimitiveTypeName::I64
                | PrimitiveTypeName::U32
                | PrimitiveTypeName::U64
        )
    )
}

fn is_float_type(type_node: &CalcKernelType) -> bool {
    matches!(type_node, CalcKernelType::Primitive(PrimitiveTypeName::F64))
}

fn is_integer(type_node: &CalcKernelType) -> bool {
    matches!(type_node, CalcKernelType::IntegerLiteral) || is_integer_primitive(type_node)
}

fn is_numeric_type(type_node: &CalcKernelType) -> bool {
    is_integer(type_node) || is_float_type(type_node)
}

fn is_slice(type_node: &CalcKernelType) -> bool {
    matches!(type_node, CalcKernelType::Slice(_))
}

fn is_valid_slice_element(type_node: &CalcKernelType) -> bool {
    matches!(
        type_node,
        CalcKernelType::Primitive(_) | CalcKernelType::Pointer(_) | CalcKernelType::Struct(_)
    )
}

fn is_index_integer(type_node: &CalcKernelType) -> bool {
    matches!(
        type_node,
        CalcKernelType::IntegerLiteral
            | CalcKernelType::Primitive(PrimitiveTypeName::I32 | PrimitiveTypeName::U32)
    )
}

fn same_type(left: &CalcKernelType, right: &CalcKernelType) -> bool {
    if is_unknown(left) || is_unknown(right) {
        return true;
    }
    if matches!(left, CalcKernelType::IntegerLiteral) && is_integer(right) {
        return true;
    }
    if matches!(right, CalcKernelType::IntegerLiteral) && is_integer(left) {
        return true;
    }
    left == right
}

fn can_assign(target: &CalcKernelType, value: &CalcKernelType) -> bool {
    same_type(target, value)
}

fn materialize_integer_literal(
    type_node: CalcKernelType,
    fallback: CalcKernelType,
) -> CalcKernelType {
    if matches!(type_node, CalcKernelType::IntegerLiteral) {
        fallback
    } else {
        type_node
    }
}

fn integer_literal_fallback(type_node: Option<&CalcKernelType>) -> CalcKernelType {
    if type_node.is_some_and(is_integer_primitive) {
        type_node.cloned().unwrap_or_else(primitive_i32)
    } else {
        primitive_i32()
    }
}

fn type_to_string(type_node: &CalcKernelType) -> String {
    match type_node {
        CalcKernelType::Primitive(PrimitiveTypeName::I32) => "i32".to_string(),
        CalcKernelType::Primitive(PrimitiveTypeName::I64) => "i64".to_string(),
        CalcKernelType::Primitive(PrimitiveTypeName::U32) => "u32".to_string(),
        CalcKernelType::Primitive(PrimitiveTypeName::U64) => "u64".to_string(),
        CalcKernelType::Primitive(PrimitiveTypeName::F64) => "f64".to_string(),
        CalcKernelType::Primitive(PrimitiveTypeName::Bool) => "bool".to_string(),
        CalcKernelType::Pointer(element_type) => format!("ptr<{}>", type_to_string(element_type)),
        CalcKernelType::Slice(element_type) => {
            format!("slice<{}>", type_to_string(element_type))
        }
        CalcKernelType::Struct(name) => name.clone(),
        CalcKernelType::Void => "void".to_string(),
        CalcKernelType::IntegerLiteral => "i32".to_string(),
        CalcKernelType::Unknown => "unknown".to_string(),
    }
}

fn invalid_u32_literal(expression: &Expression) -> Option<String> {
    match expression {
        Expression::IntegerLiteral { text, .. } => {
            text.parse::<u32>().is_err().then(|| text.clone())
        }
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => integer_literal_text(operand)
            .map(|text| format!("-{text}"))
            .or_else(|| invalid_u32_literal(operand)),
        Expression::Unary { operand, .. } => invalid_u32_literal(operand),
        Expression::Binary { left, right, .. } => {
            invalid_u32_literal(left).or_else(|| invalid_u32_literal(right))
        }
        Expression::Parenthesized { expression, .. } => invalid_u32_literal(expression),
        _ => None,
    }
}

fn integer_literal_text(expression: &Expression) -> Option<&str> {
    match expression {
        Expression::IntegerLiteral { text, .. } => Some(text),
        Expression::Parenthesized { expression, .. } => integer_literal_text(expression),
        _ => None,
    }
}

fn is_assignable_expression(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Identifier { .. } | Expression::Field { .. } | Expression::Index { .. }
    )
}

fn checker_diagnostic_code(message: &str) -> DiagnosticCode {
    if message.starts_with("Unknown variable") {
        return DiagnosticCode::Ck2001;
    }
    if message.starts_with("Unknown function") {
        return DiagnosticCode::Ck2002;
    }
    if message.starts_with("Unknown type") {
        return DiagnosticCode::Ck2003;
    }
    if message.starts_with("Duplicate") {
        return DiagnosticCode::Ck2005;
    }
    if message.starts_with("If condition") || message.starts_with("While condition") {
        return DiagnosticCode::Ck2006;
    }
    if message.starts_with("Invalid assignment target") {
        return DiagnosticCode::Ck2007;
    }
    if message.starts_with("Missing return") {
        return DiagnosticCode::Ck2008;
    }
    DiagnosticCode::Ck2004
}

fn is_arithmetic_operator(operator: &str) -> bool {
    matches!(operator, "+" | "-" | "*" | "/" | "%")
}

fn is_comparison_operator(operator: &str) -> bool {
    matches!(operator, "==" | "!=" | "<" | "<=" | ">" | ">=")
}
