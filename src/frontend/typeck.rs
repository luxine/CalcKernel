use std::collections::{BTreeMap, HashMap, HashSet};

use num_bigint::BigInt;

use super::{
    AssignmentStatement, BlockStatement, ContractEffectClause, ContractEffectKind, Declaration,
    Diagnostic, DiagnosticCode, Expression, FunctionDeclaration, FunctionParam, IfStatement,
    LetStatement, ParseResult, Program, ReturnStatement, SourceFile, SourceSpan, Statement,
    StructDeclaration, StructField, TypeNode, WhileStatement, parse,
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
    pub is_unsafe: bool,
    pub declaration: FunctionDeclaration,
    pub params: Vec<FunctionParamInfo>,
    pub return_type: CalcKernelType,
    pub contract: Option<CheckedContract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedContract {
    pub predicates: Vec<CheckedContractPredicate>,
    pub effects: Option<CheckedContractEffectCeiling>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedContractEffectCeiling {
    pub is_none: bool,
    pub items: Vec<(String, ContractEffectKind)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedAffineExpression {
    pub terms: Vec<CheckedAffineTermCoefficient>,
    pub constant: String,
    pub type_node: CalcKernelType,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckedAffineTerm {
    Parameter(String),
    SliceLength(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedAffineTermCoefficient {
    pub term: CheckedAffineTerm,
    pub coefficient: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedContractPointer {
    Parameter(String),
    SliceData(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedContractPredicate {
    Comparison {
        operator: String,
        left: CheckedAffineExpression,
        right: CheckedAffineExpression,
    },
    Conjunction(Vec<CheckedContractPredicate>),
    MultipleOf {
        value: CheckedAffineExpression,
        modulus: String,
    },
    NoAlias {
        left: String,
        right: String,
    },
    Aligned {
        pointer: CheckedContractPointer,
        alignment: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryResult {
    Void,
    I32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPoint {
    pub name: String,
    pub declaration: FunctionDeclaration,
    pub result: EntryResult,
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
    pub entry: Option<EntryPoint>,
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

#[must_use]
pub fn require_executable_entry(
    source: &SourceFile,
    checked_program: &CheckedProgram,
) -> Option<Diagnostic> {
    if checked_program.entry.is_some() {
        return None;
    }

    let position = checked_program.ast.span.end;
    Some(Diagnostic::error(
        DiagnosticCode::Ck2013,
        "Executable input requires a valid 'main' entry.",
        source.file_name.clone(),
        SourceSpan {
            start: position,
            end: position,
        },
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerBuiltinKind {
    I32ToF64,
    U32ToF64,
    PrintI32,
    PrintI64,
    PrintU32,
    PrintU64,
    PrintF64,
    PrintBool,
    PrintNewline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerBuiltinAvailability {
    AllBackends,
    NativeExecutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerBuiltinEffect {
    Pure,
    ObservableOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerBuiltin {
    pub name: &'static str,
    pub kind: CompilerBuiltinKind,
    pub params: Vec<CalcKernelType>,
    pub return_type: CalcKernelType,
    pub availability: CompilerBuiltinAvailability,
    pub effect: CompilerBuiltinEffect,
}

struct Checker<'source> {
    source: &'source SourceFile,
    program: Program,
    diagnostics: Vec<Diagnostic>,
    symbols: SymbolTable,
    expression_types: TypeMap,
    local_types: LetTypeMap,
    checked_contracts: HashMap<String, CheckedContract>,
    loop_depth: usize,
    unsafe_depth: usize,
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

struct CheckedAffineCandidate {
    form: NormalizedAffineExpression,
    type_node: CalcKernelType,
    is_constant: bool,
}

#[derive(Debug, Clone, Default)]
struct NormalizedAffineExpression {
    terms: BTreeMap<CheckedAffineTerm, BigInt>,
    constant: BigInt,
}

impl NormalizedAffineExpression {
    fn constant(value: BigInt) -> Self {
        Self {
            terms: BTreeMap::new(),
            constant: value,
        }
    }

    fn term(term: CheckedAffineTerm) -> Self {
        Self {
            terms: BTreeMap::from([(term, BigInt::from(1_u8))]),
            constant: BigInt::from(0_u8),
        }
    }

    fn add_scaled(&mut self, other: Self, scale: &BigInt) {
        self.constant += other.constant * scale;
        for (term, coefficient) in other.terms {
            let updated = self
                .terms
                .get(&term)
                .cloned()
                .unwrap_or_else(|| BigInt::from(0_u8))
                + coefficient * scale;
            if updated == BigInt::from(0_u8) {
                self.terms.remove(&term);
            } else {
                self.terms.insert(term, updated);
            }
        }
    }

    fn scaled(mut self, scale: &BigInt) -> Self {
        self.constant *= scale;
        self.terms = self
            .terms
            .into_iter()
            .filter_map(|(term, coefficient)| {
                let coefficient = coefficient * scale;
                (coefficient != BigInt::from(0_u8)).then_some((term, coefficient))
            })
            .collect();
        self
    }

    fn checked(self, type_node: CalcKernelType) -> CheckedAffineExpression {
        CheckedAffineExpression {
            terms: self
                .terms
                .into_iter()
                .map(|(term, coefficient)| CheckedAffineTermCoefficient {
                    term,
                    coefficient: coefficient.to_string(),
                })
                .collect(),
            constant: self.constant.to_string(),
            type_node,
        }
    }
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
            checked_contracts: HashMap::new(),
            loop_depth: 0,
            unsafe_depth: 0,
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
            self.checked_contracts.clone(),
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
            if get_compiler_builtin(&name).is_some() {
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
            if name == "main" {
                if function_decl.is_unsafe || function_decl.contract.is_some() {
                    self.error_with_code(
                        function_decl.name.span,
                        DiagnosticCode::Ck2014,
                        "Program entry 'main' cannot be unsafe or contracted.",
                    );
                }
                if function_decl.exported {
                    self.error_with_code(
                        function_decl.name.span,
                        DiagnosticCode::Ck2013,
                        "Program entry 'main' cannot be exported.",
                    );
                }
                if let Some(param) = function_decl.params.first() {
                    self.error_with_code(
                        param.span,
                        DiagnosticCode::Ck2013,
                        "Program entry 'main' must not declare parameters.",
                    );
                }
                if !matches!(
                    return_type,
                    CalcKernelType::Void | CalcKernelType::Primitive(PrimitiveTypeName::I32)
                ) {
                    self.error_with_code(
                        function_decl.return_type.span(),
                        DiagnosticCode::Ck2013,
                        "Program entry 'main' must return void or i32.",
                    );
                }
            }
            if name != "main" {
                if !function_decl.is_unsafe && function_decl.contract.is_some() {
                    self.error_with_code(
                        function_decl
                            .contract
                            .as_ref()
                            .map_or(function_decl.name.span, |contract| contract.span),
                        DiagnosticCode::Ck2014,
                        "A safe function cannot declare a contract or effects ceiling.",
                    );
                }
                if function_decl.is_unsafe
                    && function_decl
                        .contract
                        .as_ref()
                        .is_none_or(|contract| contract.requirements.is_empty())
                {
                    self.error_with_code(
                        function_decl
                            .contract
                            .as_ref()
                            .map_or(function_decl.name.span, |contract| contract.span),
                        DiagnosticCode::Ck2014,
                        "An unsafe function contract requires at least one 'requires' clause.",
                    );
                }
            }
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
                    declaration: *function_decl,
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
            if function_symbol.declaration != *function_decl {
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

        if let Some(contract) = &declaration.contract
            && declaration.name.name != "main"
            && declaration.is_unsafe
            && !contract.requirements.is_empty()
            && let Some(checked) = self.check_contract(
                contract,
                &scope,
                &declaration
                    .params
                    .iter()
                    .zip(&function_symbol.params)
                    .filter(|(_, type_node)| matches!(type_node, CalcKernelType::Slice(_)))
                    .map(|(param, _)| param.name.name.clone())
                    .collect::<Vec<_>>(),
            )
        {
            self.checked_contracts
                .insert(declaration.name.name.clone(), checked);
        }

        self.loop_depth = 0;
        self.unsafe_depth = 0;
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

    fn check_contract(
        &mut self,
        contract: &super::ContractDeclaration,
        scope: &Scope,
        slice_params: &[String],
    ) -> Option<CheckedContract> {
        let diagnostics_before = self.diagnostics.len();
        let predicates = contract
            .requirements
            .iter()
            .filter_map(|requirement| self.check_contract_predicate(&requirement.expression, scope))
            .collect();
        let effects = contract
            .effects
            .as_ref()
            .and_then(|clause| self.check_contract_effects(clause, scope, slice_params));
        (self.diagnostics.len() == diagnostics_before).then_some(CheckedContract {
            predicates,
            effects,
        })
    }

    fn check_contract_predicate(
        &mut self,
        expression: &Expression,
        scope: &Scope,
    ) -> Option<CheckedContractPredicate> {
        match expression {
            Expression::Parenthesized { expression, .. } => {
                self.check_contract_predicate(expression, scope)
            }
            Expression::Binary {
                operator,
                left,
                right,
                ..
            } if operator == "&&" => {
                let left = self.check_contract_predicate(left, scope)?;
                let right = self.check_contract_predicate(right, scope)?;
                Some(CheckedContractPredicate::Conjunction(vec![left, right]))
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } if is_comparison_operator(operator) => {
                let mut left = self.check_contract_affine(left, scope)?;
                let mut right = self.check_contract_affine(right, scope)?;
                if !same_type(&left.type_node, &right.type_node) {
                    self.contract_error(
                        *span,
                        "Contract comparison operands must have the same integer type.",
                    );
                    return None;
                }
                if matches!(left.type_node, CalcKernelType::IntegerLiteral) {
                    left.type_node =
                        materialize_integer_literal(left.type_node, right.type_node.clone());
                }
                if matches!(right.type_node, CalcKernelType::IntegerLiteral) {
                    right.type_node =
                        materialize_integer_literal(right.type_node, left.type_node.clone());
                }
                if matches!(left.type_node, CalcKernelType::IntegerLiteral) {
                    left.type_node = primitive_i32();
                    right.type_node = primitive_i32();
                }
                Some(CheckedContractPredicate::Comparison {
                    operator: operator.clone(),
                    left: left.form.checked(left.type_node),
                    right: right.form.checked(right.type_node),
                })
            }
            Expression::Call { callee, args, span } => {
                self.check_contract_builtin(callee, args, *span, scope)
            }
            _ => {
                self.contract_error(
                    expression.span(),
                    "Contract requirement must be a comparison, conjunction, or supported predicate.",
                );
                None
            }
        }
    }

    fn check_contract_builtin(
        &mut self,
        callee: &Expression,
        args: &[Expression],
        span: SourceSpan,
        scope: &Scope,
    ) -> Option<CheckedContractPredicate> {
        let Expression::Identifier { name, .. } = callee else {
            self.contract_error(span, "Unsupported contract predicate.");
            return None;
        };
        match name.as_str() {
            "multiple_of" => self.check_contract_multiple_of(args, span, scope),
            "noalias" => self.check_contract_noalias(args, span, scope),
            "aligned" => self.check_contract_aligned(args, span, scope),
            _ => {
                self.contract_error(span, format!("Unsupported contract predicate '{name}'."));
                None
            }
        }
    }

    fn check_contract_multiple_of(
        &mut self,
        args: &[Expression],
        span: SourceSpan,
        scope: &Scope,
    ) -> Option<CheckedContractPredicate> {
        if args.len() != 2 {
            self.contract_error(span, "multiple_of requires two arguments.");
            return None;
        }
        let value = self.check_contract_affine(&args[0], scope)?;
        let Expression::IntegerLiteral { text, .. } = &args[1] else {
            self.contract_error(
                args[1].span(),
                "multiple_of modulus must be a positive integer constant.",
            );
            return None;
        };
        let Some(modulus) =
            BigInt::parse_bytes(text.as_bytes(), 10).filter(|value| *value > BigInt::from(0_u8))
        else {
            self.contract_error(
                args[1].span(),
                "multiple_of modulus must be a positive integer constant.",
            );
            return None;
        };
        Some(CheckedContractPredicate::MultipleOf {
            value: value.form.checked(materialize_integer_literal(
                value.type_node,
                primitive_i32(),
            )),
            modulus: modulus.to_string(),
        })
    }

    fn check_contract_noalias(
        &mut self,
        args: &[Expression],
        span: SourceSpan,
        scope: &Scope,
    ) -> Option<CheckedContractPredicate> {
        if args.len() != 2 {
            self.contract_error(span, "noalias requires two slice parameters.");
            return None;
        }
        let left = self.contract_slice_parameter(&args[0], scope)?;
        let right = self.contract_slice_parameter(&args[1], scope)?;
        Some(CheckedContractPredicate::NoAlias { left, right })
    }

    fn check_contract_aligned(
        &mut self,
        args: &[Expression],
        span: SourceSpan,
        scope: &Scope,
    ) -> Option<CheckedContractPredicate> {
        if args.len() != 2 {
            self.contract_error(span, "aligned requires a pointer and alignment.");
            return None;
        }
        let pointer = self.contract_pointer(&args[0], scope)?;
        let Expression::IntegerLiteral { text, .. } = &args[1] else {
            self.contract_error(
                args[1].span(),
                "aligned value must be a power-of-two u32 constant no greater than 2^31.",
            );
            return None;
        };
        let Some(alignment) = text
            .parse::<u32>()
            .ok()
            .filter(|value| value.is_power_of_two() && *value <= (1_u32 << 31))
        else {
            self.contract_error(
                args[1].span(),
                "aligned value must be a power-of-two u32 constant no greater than 2^31.",
            );
            return None;
        };
        Some(CheckedContractPredicate::Aligned { pointer, alignment })
    }

    fn check_contract_affine(
        &mut self,
        expression: &Expression,
        scope: &Scope,
    ) -> Option<CheckedAffineCandidate> {
        match expression {
            Expression::IntegerLiteral { text, span } => {
                let Some(value) = BigInt::parse_bytes(text.as_bytes(), 10) else {
                    self.contract_error(*span, "Invalid contract integer constant.");
                    return None;
                };
                Some(CheckedAffineCandidate {
                    form: NormalizedAffineExpression::constant(value),
                    type_node: CalcKernelType::IntegerLiteral,
                    is_constant: true,
                })
            }
            Expression::Identifier { name, span } => {
                let Some(symbol) = scope.lookup(name) else {
                    self.contract_error(*span, format!("Unknown contract parameter '{name}'."));
                    return None;
                };
                if !is_integer_primitive(&symbol.type_node) {
                    self.contract_error(*span, "Contract affine terms must be integer values.");
                    return None;
                }
                Some(CheckedAffineCandidate {
                    form: NormalizedAffineExpression::term(CheckedAffineTerm::Parameter(
                        name.clone(),
                    )),
                    type_node: symbol.type_node.clone(),
                    is_constant: false,
                })
            }
            Expression::Field {
                object,
                field,
                span,
            } if field.name == "len" => {
                let Expression::Identifier { name, .. } = object.as_ref() else {
                    self.contract_error(*span, "Contract slice length must name a parameter.");
                    return None;
                };
                let Some(symbol) = scope.lookup(name) else {
                    self.contract_error(*span, format!("Unknown contract parameter '{name}'."));
                    return None;
                };
                if !matches!(symbol.type_node, CalcKernelType::Slice(_)) {
                    self.contract_error(*span, "Contract '.len' requires a slice parameter.");
                    return None;
                }
                Some(CheckedAffineCandidate {
                    form: NormalizedAffineExpression::term(CheckedAffineTerm::SliceLength(
                        name.clone(),
                    )),
                    type_node: primitive_u32(),
                    is_constant: false,
                })
            }
            Expression::Unary {
                operator,
                operand,
                span: _,
            } if operator == "-" => {
                let operand = self.check_contract_affine(operand, scope)?;
                Some(CheckedAffineCandidate {
                    form: operand.form.scaled(&BigInt::from(-1_i8)),
                    type_node: operand.type_node,
                    is_constant: operand.is_constant,
                })
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } if matches!(operator.as_str(), "+" | "-" | "*") => {
                let left = self.check_contract_affine(left, scope)?;
                let right = self.check_contract_affine(right, scope)?;
                if operator == "*" && !left.is_constant && !right.is_constant {
                    self.contract_error(
                        *span,
                        "Contract multiplication must have an integer constant operand.",
                    );
                    return None;
                }
                if !same_type(&left.type_node, &right.type_node) {
                    self.contract_error(
                        *span,
                        "Contract affine operands must have the same integer type.",
                    );
                    return None;
                }
                let type_node = if matches!(left.type_node, CalcKernelType::IntegerLiteral) {
                    right.type_node.clone()
                } else {
                    left.type_node.clone()
                };
                let is_constant = left.is_constant && right.is_constant;
                let form = match operator.as_str() {
                    "+" => {
                        let mut form = left.form;
                        form.add_scaled(right.form, &BigInt::from(1_u8));
                        form
                    }
                    "-" => {
                        let mut form = left.form;
                        form.add_scaled(right.form, &BigInt::from(-1_i8));
                        form
                    }
                    "*" if left.is_constant => right.form.scaled(&left.form.constant),
                    "*" => left.form.scaled(&right.form.constant),
                    _ => unreachable!("matched affine operator"),
                };
                Some(CheckedAffineCandidate {
                    form,
                    type_node,
                    is_constant,
                })
            }
            Expression::Parenthesized { expression, .. } => {
                self.check_contract_affine(expression, scope)
            }
            _ => {
                self.contract_error(
                    expression.span(),
                    "Unsupported or non-affine contract integer expression.",
                );
                None
            }
        }
    }

    fn contract_slice_parameter(
        &mut self,
        expression: &Expression,
        scope: &Scope,
    ) -> Option<String> {
        let Expression::Identifier { name, span } = expression else {
            self.contract_error(
                expression.span(),
                "noalias requires named slice parameters.",
            );
            return None;
        };
        if !scope
            .lookup(name)
            .is_some_and(|symbol| matches!(symbol.type_node, CalcKernelType::Slice(_)))
        {
            self.contract_error(*span, "noalias requires named slice parameters.");
            return None;
        }
        Some(name.clone())
    }

    fn contract_pointer(
        &mut self,
        expression: &Expression,
        scope: &Scope,
    ) -> Option<CheckedContractPointer> {
        match expression {
            Expression::Identifier { name, span: _ }
                if scope.lookup(name).is_some_and(|symbol| {
                    matches!(symbol.type_node, CalcKernelType::Pointer(_))
                }) =>
            {
                Some(CheckedContractPointer::Parameter(name.clone()))
            }
            Expression::Field {
                object,
                field,
                span,
            } if field.name == "data" => {
                let Expression::Identifier { name, .. } = object.as_ref() else {
                    self.contract_error(
                        *span,
                        "aligned requires a pointer parameter or slice.data.",
                    );
                    return None;
                };
                if !scope
                    .lookup(name)
                    .is_some_and(|symbol| matches!(symbol.type_node, CalcKernelType::Slice(_)))
                {
                    self.contract_error(
                        *span,
                        "aligned requires a pointer parameter or slice.data.",
                    );
                    return None;
                }
                Some(CheckedContractPointer::SliceData(name.clone()))
            }
            _ => {
                self.contract_error(
                    expression.span(),
                    "aligned requires a pointer parameter or slice.data.",
                );
                None
            }
        }
    }

    fn check_contract_effects(
        &mut self,
        clause: &ContractEffectClause,
        scope: &Scope,
        slice_params: &[String],
    ) -> Option<CheckedContractEffectCeiling> {
        let diagnostics_before = self.diagnostics.len();
        let mut seen = HashSet::new();
        let mut declared = HashMap::new();
        for item in &clause.items {
            let valid_slice = scope
                .lookup(&item.target.name)
                .is_some_and(|symbol| matches!(symbol.type_node, CalcKernelType::Slice(_)));
            if !valid_slice {
                self.contract_error(
                    item.target.span,
                    "Effect target must be a named slice parameter.",
                );
                continue;
            }
            if !seen.insert(item.target.name.clone()) {
                self.contract_error(
                    item.target.span,
                    "An effect target can appear only once in a ceiling.",
                );
                continue;
            }
            declared.insert(item.target.name.clone(), item.kind);
        }
        let items = slice_params
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    declared
                        .get(name)
                        .copied()
                        .unwrap_or(ContractEffectKind::None),
                )
            })
            .collect();
        (self.diagnostics.len() == diagnostics_before).then_some(CheckedContractEffectCeiling {
            is_none: clause.is_none,
            items,
        })
    }

    fn contract_error(&mut self, span: SourceSpan, message: impl Into<String>) {
        self.error_with_code(span, DiagnosticCode::Ck2015, message);
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
            Statement::Unsafe(statement) => {
                self.unsafe_depth += 1;
                let flow = self.check_block(&statement.block, scope, return_type, true);
                self.unsafe_depth -= 1;
                flow
            }
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

        if let Some(builtin) = get_compiler_builtin(name) {
            let return_type = self.check_builtin_call(&builtin, args, span, scope);
            if value_required && matches!(return_type, CalcKernelType::Void) {
                self.error_with_code(
                    span,
                    DiagnosticCode::Ck2011,
                    "A void compiler builtin call cannot be used where a value is required.",
                );
                return CalcKernelType::Unknown;
            }
            return return_type;
        }

        let Some(function_symbol) = self.symbols.functions.get(name).cloned() else {
            self.error(*name_span, format!("Unknown function '{name}'."));
            for arg in args {
                self.check_expression(arg, scope, None);
            }
            return CalcKernelType::Unknown;
        };

        if function_symbol.declaration.is_unsafe && self.unsafe_depth == 0 {
            self.error_with_code(
                span,
                DiagnosticCode::Ck2014,
                format!(
                    "Call to unsafe function '{}' requires an explicit unsafe block.",
                    function_symbol.name
                ),
            );
        }

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
    checked_contracts: HashMap<String, CheckedContract>,
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
            (symbol.declaration == **function_decl)
                .then(|| to_function_info(symbol, checked_contracts.get(&symbol.name).cloned()))
        })
        .collect();
    let main_declaration_count = ast
        .declarations
        .iter()
        .filter(|declaration| {
            matches!(
                declaration,
                Declaration::Function(function) if function.name.name == "main"
            )
        })
        .count();
    let entry = (main_declaration_count == 1)
        .then(|| functions.iter().find(|function| function.name == "main"))
        .flatten()
        .and_then(|function| {
            if function.exported
                || function.is_unsafe
                || function.declaration.contract.is_some()
                || !function.params.is_empty()
            {
                return None;
            }
            let result = match function.return_type {
                CalcKernelType::Void => EntryResult::Void,
                CalcKernelType::Primitive(PrimitiveTypeName::I32) => EntryResult::I32,
                _ => return None,
            };
            Some(EntryPoint {
                name: function.name.clone(),
                declaration: function.declaration.clone(),
                result,
            })
        });
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
        entry,
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

fn to_function_info(symbol: &FunctionSymbol, contract: Option<CheckedContract>) -> FunctionInfo {
    FunctionInfo {
        name: symbol.name.clone(),
        exported: symbol.declaration.exported,
        is_unsafe: symbol.declaration.is_unsafe,
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
        contract,
    }
}

#[must_use]
pub fn get_compiler_builtin(name: &str) -> Option<CompilerBuiltin> {
    match name {
        "i32_to_f64" => Some(CompilerBuiltin {
            name: "i32_to_f64",
            kind: CompilerBuiltinKind::I32ToF64,
            params: vec![primitive_i32()],
            return_type: primitive_f64(),
            availability: CompilerBuiltinAvailability::AllBackends,
            effect: CompilerBuiltinEffect::Pure,
        }),
        "u32_to_f64" => Some(CompilerBuiltin {
            name: "u32_to_f64",
            kind: CompilerBuiltinKind::U32ToF64,
            params: vec![primitive_u32()],
            return_type: primitive_f64(),
            availability: CompilerBuiltinAvailability::AllBackends,
            effect: CompilerBuiltinEffect::Pure,
        }),
        "print_i32" => Some(native_print_builtin(
            "print_i32",
            CompilerBuiltinKind::PrintI32,
            Some(primitive_i32()),
        )),
        "print_i64" => Some(native_print_builtin(
            "print_i64",
            CompilerBuiltinKind::PrintI64,
            Some(primitive_i64()),
        )),
        "print_u32" => Some(native_print_builtin(
            "print_u32",
            CompilerBuiltinKind::PrintU32,
            Some(primitive_u32()),
        )),
        "print_u64" => Some(native_print_builtin(
            "print_u64",
            CompilerBuiltinKind::PrintU64,
            Some(primitive_u64()),
        )),
        "print_f64" => Some(native_print_builtin(
            "print_f64",
            CompilerBuiltinKind::PrintF64,
            Some(primitive_f64()),
        )),
        "print_bool" => Some(native_print_builtin(
            "print_bool",
            CompilerBuiltinKind::PrintBool,
            Some(primitive_bool()),
        )),
        "print_newline" => Some(native_print_builtin(
            "print_newline",
            CompilerBuiltinKind::PrintNewline,
            None,
        )),
        _ => None,
    }
}

fn native_print_builtin(
    name: &'static str,
    kind: CompilerBuiltinKind,
    parameter: Option<CalcKernelType>,
) -> CompilerBuiltin {
    CompilerBuiltin {
        name,
        kind,
        params: parameter.into_iter().collect(),
        return_type: CalcKernelType::Void,
        availability: CompilerBuiltinAvailability::NativeExecutable,
        effect: CompilerBuiltinEffect::ObservableOutput,
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
