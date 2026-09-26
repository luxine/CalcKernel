use std::collections::HashMap;

use super::{
    ContractEffectClause, Declaration, Diagnostic, Expression, FunctionDeclaration, IdentifierNode,
    SourceFile, SourcePosition, SourceSpan, Statement, StructDeclaration, TypeNode,
    get_compiler_builtin, lex, parse,
};

/// Stable identity for a declaration in one editor analysis snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// Identity for a lexical scope in one editor analysis snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Struct,
    Field,
    Parameter,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorOccurrenceKind {
    Identifier,
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSymbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub declaration: SourceSpan,
    pub scope_id: ScopeId,
    pub renameable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorOccurrence {
    pub name: String,
    /// Offsets in this span use UTF-16 code units, matching SourceSpan/LSP.
    pub span: SourceSpan,
    pub symbol_id: Option<SymbolId>,
    pub kind: EditorOccurrenceKind,
    pub is_declaration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorScope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorTextEdit {
    /// Offsets in this span use UTF-16 code units, matching SourceSpan/LSP.
    pub span: SourceSpan,
    pub new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorRenameError {
    NoSymbol,
    NotRenameable,
    InvalidName,
    ReservedName,
    Main,
    Conflict,
    BindingChanged,
    IncompleteAnalysis,
}

impl EditorRenameError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoSymbol => "No renameable symbol at this position.",
            Self::NotRenameable => "This identifier cannot be renamed.",
            Self::InvalidName => "The new name is not a valid CK identifier.",
            Self::ReservedName => "The new name is reserved by CK or a compiler builtin.",
            Self::Main => "The program entry function 'main' cannot be renamed.",
            Self::Conflict => "The new name conflicts with a declaration in the same scope.",
            Self::BindingChanged => "The rename would change one or more identifier bindings.",
            Self::IncompleteAnalysis => "Rename is unavailable because the source is incomplete.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAnalysis {
    pub source: SourceFile,
    pub symbols: Vec<EditorSymbol>,
    pub scopes: Vec<EditorScope>,
    pub occurrences: Vec<EditorOccurrence>,
    /// Parser and lexer diagnostics. Type diagnostics remain the responsibility of `check`.
    pub diagnostics: Vec<Diagnostic>,
    /// False when parsing or indexing encountered an error AST or duplicate declaration.
    pub is_complete: bool,
}

impl EditorAnalysis {
    #[must_use]
    pub fn definition_at(&self, utf16_offset: usize) -> Option<&EditorSymbol> {
        self.occurrence_at(utf16_offset)
            .and_then(|occurrence| occurrence.symbol_id)
            .and_then(|symbol_id| self.symbols.iter().find(|symbol| symbol.id == symbol_id))
    }

    #[must_use]
    pub fn references_at(&self, utf16_offset: usize) -> Vec<&EditorOccurrence> {
        let Some(symbol_id) = self
            .occurrence_at(utf16_offset)
            .and_then(|occurrence| occurrence.symbol_id)
        else {
            return Vec::new();
        };
        self.occurrences
            .iter()
            .filter(|occurrence| occurrence.symbol_id == Some(symbol_id))
            .collect()
    }

    /// Returns edits for every occurrence bound to the symbol at `utf16_offset`.
    /// Ranges use UTF-16 offsets; source slicing during safety validation converts each offset
    /// explicitly to a UTF-8 byte boundary.
    pub fn rename(
        &self,
        utf16_offset: usize,
        new_name: &str,
    ) -> Result<Vec<EditorTextEdit>, EditorRenameError> {
        if !self.is_complete {
            return Err(EditorRenameError::IncompleteAnalysis);
        }
        let occurrence = self
            .occurrence_at(utf16_offset)
            .ok_or(EditorRenameError::NoSymbol)?;
        let symbol_id = occurrence
            .symbol_id
            .ok_or(EditorRenameError::NotRenameable)?;
        let symbol = self
            .symbols
            .iter()
            .find(|symbol| symbol.id == symbol_id)
            .ok_or(EditorRenameError::NotRenameable)?;
        if symbol.kind == SymbolKind::Function && symbol.name == "main" {
            return Err(EditorRenameError::Main);
        }
        if !symbol.renameable {
            return Err(EditorRenameError::NotRenameable);
        }
        if !is_identifier(new_name) {
            return Err(EditorRenameError::InvalidName);
        }
        if is_reserved_name(new_name) {
            return Err(EditorRenameError::ReservedName);
        }
        if symbol.name == new_name {
            return Ok(Vec::new());
        }
        if self.symbols.iter().any(|candidate| {
            candidate.id != symbol_id
                && candidate.name == new_name
                && candidate.scope_id == symbol.scope_id
                && same_namespace(candidate.kind, symbol.kind)
        }) {
            return Err(EditorRenameError::Conflict);
        }

        let mut edits: Vec<_> = self
            .occurrences
            .iter()
            .filter(|candidate| candidate.symbol_id == Some(symbol_id))
            .map(|candidate| EditorTextEdit {
                span: candidate.span,
                new_text: new_name.to_owned(),
            })
            .collect();
        if edits.is_empty() {
            return Err(EditorRenameError::NotRenameable);
        }
        edits.sort_by_key(|edit| edit.span.start.offset);

        let updated_source =
            apply_edits(&self.source, &edits).ok_or(EditorRenameError::BindingChanged)?;
        let updated = analyze_editor(&updated_source);
        if !updated.is_complete || !bindings_are_stable(self, &updated, symbol_id, new_name, &edits)
        {
            return Err(EditorRenameError::BindingChanged);
        }
        Ok(edits)
    }

    fn occurrence_at(&self, utf16_offset: usize) -> Option<&EditorOccurrence> {
        self.occurrences.iter().find(|occurrence| {
            occurrence.span.start.offset <= utf16_offset
                && utf16_offset < occurrence.span.end.offset
        })
    }
}

#[must_use]
pub fn analyze_editor(source: &SourceFile) -> EditorAnalysis {
    let parsed = parse(source);
    let incomplete = !parsed.diagnostics.is_empty();
    let mut builder = EditorIndexBuilder::new(source.clone(), parsed.diagnostics, incomplete);
    builder.build(&parsed.ast.declarations);
    builder.analysis.is_complete = !builder.incomplete;
    builder.analysis
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EditorType {
    Struct(String),
    Pointer(Box<EditorType>),
    Slice(Box<EditorType>),
    Other,
    Unknown,
}

#[derive(Debug, Clone)]
struct FunctionMeta {
    id: SymbolId,
    return_type: EditorType,
    declaration_start: usize,
}

#[derive(Debug, Clone)]
struct StructMeta {
    id: SymbolId,
    scope_id: ScopeId,
    declaration_start: usize,
}

#[derive(Debug, Clone)]
struct Binding {
    id: SymbolId,
    type_node: EditorType,
}

struct EditorIndexBuilder {
    analysis: EditorAnalysis,
    incomplete: bool,
    next_symbol: u32,
    next_scope: u32,
    global_scope: ScopeId,
    functions: HashMap<String, FunctionMeta>,
    structs: HashMap<String, StructMeta>,
    fields: HashMap<(String, String), SymbolId>,
    field_types: HashMap<SymbolId, EditorType>,
}

impl EditorIndexBuilder {
    fn new(source: SourceFile, diagnostics: Vec<Diagnostic>, incomplete: bool) -> Self {
        let global_scope = ScopeId(0);
        let analysis = EditorAnalysis {
            source: source.clone(),
            symbols: Vec::new(),
            scopes: vec![EditorScope {
                id: global_scope,
                parent: None,
                span: SourceSpan {
                    start: SourcePosition {
                        offset: 0,
                        line: 1,
                        column: 1,
                    },
                    end: source_end(&source),
                },
            }],
            occurrences: Vec::new(),
            diagnostics,
            is_complete: false,
        };
        Self {
            analysis,
            incomplete,
            next_symbol: 0,
            next_scope: 1,
            global_scope,
            functions: HashMap::new(),
            structs: HashMap::new(),
            fields: HashMap::new(),
            field_types: HashMap::new(),
        }
    }

    fn build(&mut self, declarations: &[Declaration]) {
        // First pass: create every global function and struct symbol.
        for declaration in declarations {
            match declaration {
                Declaration::Struct(structure) => self.index_struct_name(structure),
                Declaration::Function(function) => self.index_function_name(function),
            }
        }
        // Second pass: create fields and resolve all type references with the full struct table.
        for declaration in declarations {
            match declaration {
                Declaration::Struct(structure) => self.index_struct_fields(structure),
                Declaration::Function(function) => self.index_function_types(function),
            }
        }
        // Function bodies are indexed only after globals, fields, and signatures are known.
        for declaration in declarations {
            if let Declaration::Function(function) = declaration {
                self.index_function_body(function);
            }
        }
        self.analysis
            .occurrences
            .sort_by_key(|occurrence| occurrence.span.start.offset);
    }

    fn index_struct_name(&mut self, declaration: &StructDeclaration) {
        let scope_id = self.add_scope(Some(self.global_scope), declaration.span);
        let name = &declaration.name.name;
        let id = if self.structs.contains_key(name) {
            self.incomplete = true;
            None
        } else {
            let id = self.add_symbol(
                name,
                SymbolKind::Struct,
                declaration.name.span,
                self.global_scope,
                true,
            );
            self.structs.insert(
                name.clone(),
                StructMeta {
                    id,
                    scope_id,
                    declaration_start: declaration.name.span.start.offset,
                },
            );
            Some(id)
        };
        self.add_occurrence(
            &declaration.name,
            id,
            EditorOccurrenceKind::Identifier,
            true,
        );
    }

    fn index_function_name(&mut self, declaration: &FunctionDeclaration) {
        let name = &declaration.name.name;
        let id = if self.functions.contains_key(name) {
            self.incomplete = true;
            None
        } else {
            let id = self.add_symbol(
                name,
                SymbolKind::Function,
                declaration.name.span,
                self.global_scope,
                name != "main",
            );
            self.functions.insert(
                name.clone(),
                FunctionMeta {
                    id,
                    return_type: self.type_from_node(&declaration.return_type),
                    declaration_start: declaration.name.span.start.offset,
                },
            );
            Some(id)
        };
        self.add_occurrence(
            &declaration.name,
            id,
            EditorOccurrenceKind::Identifier,
            true,
        );
    }

    fn index_struct_fields(&mut self, declaration: &StructDeclaration) {
        let Some(struct_meta) = self.structs.get(&declaration.name.name).cloned() else {
            for field in &declaration.fields {
                self.add_occurrence(&field.name, None, EditorOccurrenceKind::Identifier, true);
                self.index_type(&field.type_node);
            }
            return;
        };
        if struct_meta.declaration_start != declaration.name.span.start.offset {
            for field in &declaration.fields {
                self.add_occurrence(&field.name, None, EditorOccurrenceKind::Identifier, true);
                self.index_type(&field.type_node);
            }
            return;
        }
        for field in &declaration.fields {
            let key = (declaration.name.name.clone(), field.name.name.clone());
            let id = if self.fields.contains_key(&key) {
                self.incomplete = true;
                None
            } else {
                let id = self.add_symbol(
                    &field.name.name,
                    SymbolKind::Field,
                    field.name.span,
                    struct_meta.scope_id,
                    true,
                );
                self.fields.insert(key, id);
                self.field_types
                    .insert(id, self.type_from_node(&field.type_node));
                Some(id)
            };
            self.add_occurrence(&field.name, id, EditorOccurrenceKind::Identifier, true);
            self.index_type(&field.type_node);
        }
    }

    fn index_function_types(&mut self, declaration: &FunctionDeclaration) {
        for param in &declaration.params {
            self.index_type(&param.type_node);
        }
        self.index_type(&declaration.return_type);
        let return_type = self.type_from_node(&declaration.return_type);
        if let Some(function) = self.functions.get_mut(&declaration.name.name)
            && function.declaration_start == declaration.name.span.start.offset
        {
            function.return_type = return_type;
        }
    }

    fn index_function_body(&mut self, declaration: &FunctionDeclaration) {
        let Some(meta) = self.functions.get(&declaration.name.name).cloned() else {
            return;
        };
        if meta.declaration_start != declaration.name.span.start.offset {
            return;
        }
        let function_scope = self.add_scope(Some(self.global_scope), declaration.body.span);
        let mut scopes = vec![(function_scope, HashMap::<String, Binding>::new())];
        for param in &declaration.params {
            let binding_type = self.type_from_node(&param.type_node);
            let duplicate = scopes[0].1.contains_key(&param.name.name);
            let id = if duplicate {
                self.incomplete = true;
                None
            } else {
                let id = self.add_symbol(
                    &param.name.name,
                    SymbolKind::Parameter,
                    param.name.span,
                    function_scope,
                    true,
                );
                scopes[0].1.insert(
                    param.name.name.clone(),
                    Binding {
                        id,
                        type_node: binding_type,
                    },
                );
                Some(id)
            };
            self.add_occurrence(&param.name, id, EditorOccurrenceKind::Identifier, true);
        }

        if let Some(contract) = &declaration.contract {
            for requirement in &contract.requirements {
                self.index_expression_mode(&requirement.expression, &mut scopes, true);
            }
            if let Some(effects) = &contract.effects {
                self.index_effects(effects, &mut scopes);
            }
        }
        self.index_statements(&declaration.body.statements, &mut scopes);
        let _ = meta;
    }

    fn index_effects(
        &mut self,
        effects: &ContractEffectClause,
        scopes: &mut Vec<(ScopeId, HashMap<String, Binding>)>,
    ) {
        for item in &effects.items {
            let binding = scopes
                .iter()
                .rev()
                .find_map(|(_, frame)| frame.get(&item.target.name));
            self.add_occurrence(
                &item.target,
                binding.map(|binding| binding.id),
                EditorOccurrenceKind::Identifier,
                false,
            );
        }
    }

    fn index_statements(
        &mut self,
        statements: &[Statement],
        scopes: &mut Vec<(ScopeId, HashMap<String, Binding>)>,
    ) {
        for statement in statements {
            match statement {
                Statement::Block(block) => self.index_nested_block(block, scopes),
                Statement::Unsafe(statement) => self.index_nested_block(&statement.block, scopes),
                Statement::Let(statement) => {
                    self.index_type(&statement.type_node);
                    let scope_id = scopes.last().expect("function scope exists").0;
                    let duplicate = scopes
                        .last()
                        .is_some_and(|(_, frame)| frame.contains_key(&statement.name.name));
                    let id = if duplicate {
                        self.incomplete = true;
                        None
                    } else {
                        let id = self.add_symbol(
                            &statement.name.name,
                            SymbolKind::Local,
                            statement.name.span,
                            scope_id,
                            true,
                        );
                        scopes.last_mut().expect("function scope exists").1.insert(
                            statement.name.name.clone(),
                            Binding {
                                id,
                                type_node: self.type_from_node(&statement.type_node),
                            },
                        );
                        Some(id)
                    };
                    self.add_occurrence(
                        &statement.name,
                        id,
                        EditorOccurrenceKind::Identifier,
                        true,
                    );
                    // The checker declares the local before checking its initializer.
                    self.index_expression(&statement.initializer, scopes);
                }
                Statement::Assignment(statement) => {
                    self.index_expression(&statement.target, scopes);
                    self.index_expression(&statement.value, scopes);
                }
                Statement::Call(statement) => {
                    self.index_expression(&statement.call, scopes);
                }
                Statement::Return(statement) => {
                    if let Some(value) = &statement.value {
                        self.index_expression(value, scopes);
                    }
                }
                Statement::Break(_) | Statement::Continue(_) => {}
                Statement::If(statement) => {
                    self.index_expression(&statement.condition, scopes);
                    self.index_nested_block(&statement.then_block, scopes);
                    if let Some(else_block) = &statement.else_block {
                        self.index_nested_block(else_block, scopes);
                    }
                }
                Statement::While(statement) => {
                    self.index_expression(&statement.condition, scopes);
                    self.index_nested_block(&statement.body, scopes);
                }
                Statement::Error { .. } => self.incomplete = true,
            }
        }
    }

    fn index_nested_block(
        &mut self,
        block: &super::BlockStatement,
        scopes: &mut Vec<(ScopeId, HashMap<String, Binding>)>,
    ) {
        let parent = scopes.last().map(|(scope, _)| *scope);
        let scope_id = self.add_scope(parent, block.span);
        scopes.push((scope_id, HashMap::new()));
        self.index_statements(&block.statements, scopes);
        scopes.pop();
    }

    fn index_expression(
        &mut self,
        expression: &Expression,
        scopes: &mut Vec<(ScopeId, HashMap<String, Binding>)>,
    ) -> EditorType {
        self.index_expression_mode(expression, scopes, false)
    }

    fn index_expression_mode(
        &mut self,
        expression: &Expression,
        scopes: &mut Vec<(ScopeId, HashMap<String, Binding>)>,
        in_contract: bool,
    ) -> EditorType {
        match expression {
            Expression::Identifier { name, span } => {
                let binding = scopes
                    .iter()
                    .rev()
                    .find_map(|(_, frame)| frame.get(name))
                    .cloned();
                self.add_occurrence_span(
                    name,
                    *span,
                    binding.as_ref().map(|binding| binding.id),
                    EditorOccurrenceKind::Identifier,
                    false,
                );
                binding.map_or(EditorType::Unknown, |binding| binding.type_node)
            }
            Expression::IntegerLiteral { .. }
            | Expression::FloatLiteral { .. }
            | Expression::BoolLiteral { .. } => EditorType::Other,
            Expression::Unary { operand, .. }
            | Expression::Parenthesized {
                expression: operand,
                ..
            } => self.index_expression_mode(operand, scopes, in_contract),
            Expression::Binary { left, right, .. } => {
                self.index_expression_mode(left, scopes, in_contract);
                self.index_expression_mode(right, scopes, in_contract);
                EditorType::Other
            }
            Expression::Call { callee, args, .. } => {
                let return_type = if let Expression::Identifier { name, span } = callee.as_ref() {
                    if get_compiler_builtin(name).is_some() {
                        self.add_occurrence_span(
                            name,
                            *span,
                            None,
                            EditorOccurrenceKind::Builtin,
                            false,
                        );
                        get_compiler_builtin(name)
                            .map(|builtin| editor_type_from_calc(&builtin.return_type))
                            .unwrap_or(EditorType::Unknown)
                    } else if in_contract && is_contract_builtin(name) {
                        self.add_occurrence_span(
                            name,
                            *span,
                            None,
                            EditorOccurrenceKind::Builtin,
                            false,
                        );
                        EditorType::Other
                    } else if let Some(function) = self.functions.get(name).cloned() {
                        self.add_occurrence_span(
                            name,
                            *span,
                            Some(function.id),
                            EditorOccurrenceKind::Identifier,
                            false,
                        );
                        function.return_type.clone()
                    } else {
                        self.add_occurrence_span(
                            name,
                            *span,
                            None,
                            EditorOccurrenceKind::Identifier,
                            false,
                        );
                        EditorType::Unknown
                    }
                } else {
                    self.index_expression_mode(callee, scopes, in_contract);
                    EditorType::Unknown
                };
                for argument in args {
                    self.index_expression_mode(argument, scopes, in_contract);
                }
                return_type
            }
            Expression::SliceConstructor { data, len, .. } => {
                let data_type = self.index_expression_mode(data, scopes, in_contract);
                self.index_expression_mode(len, scopes, in_contract);
                match data_type {
                    EditorType::Pointer(element) => EditorType::Slice(element),
                    _ => EditorType::Slice(Box::new(EditorType::Unknown)),
                }
            }
            Expression::Field { object, field, .. } => {
                let object_type = self.index_expression_mode(object, scopes, in_contract);
                match object_type {
                    EditorType::Slice(element) if matches!(field.name.as_str(), "data" | "len") => {
                        self.add_occurrence(field, None, EditorOccurrenceKind::Builtin, false);
                        if field.name == "data" {
                            EditorType::Pointer(element)
                        } else {
                            EditorType::Other
                        }
                    }
                    EditorType::Struct(struct_name) => {
                        let id = self.fields.get(&(struct_name, field.name.clone())).copied();
                        self.add_occurrence(field, id, EditorOccurrenceKind::Identifier, false);
                        id.and_then(|id| self.field_types.get(&id).cloned())
                            .unwrap_or(EditorType::Unknown)
                    }
                    _ => {
                        self.add_occurrence(field, None, EditorOccurrenceKind::Identifier, false);
                        EditorType::Unknown
                    }
                }
            }
            Expression::Index { object, index, .. } => {
                let object_type = self.index_expression_mode(object, scopes, in_contract);
                self.index_expression_mode(index, scopes, in_contract);
                match object_type {
                    EditorType::Pointer(element) | EditorType::Slice(element) => *element,
                    _ => EditorType::Unknown,
                }
            }
            Expression::Subslice {
                slice, start, end, ..
            } => {
                let slice_type = self.index_expression_mode(slice, scopes, in_contract);
                self.index_expression_mode(start, scopes, in_contract);
                self.index_expression_mode(end, scopes, in_contract);
                match slice_type {
                    EditorType::Slice(element) => EditorType::Slice(element),
                    _ => EditorType::Unknown,
                }
            }
            Expression::Error { .. } => {
                self.incomplete = true;
                EditorType::Unknown
            }
        }
    }

    fn index_type(&mut self, type_node: &TypeNode) {
        match type_node {
            TypeNode::Pointer { element_type, .. } | TypeNode::Slice { element_type, .. } => {
                self.index_type(element_type);
            }
            TypeNode::Named { name, .. } => {
                let id = self.structs.get(&name.name).map(|structure| structure.id);
                self.add_occurrence(name, id, EditorOccurrenceKind::Identifier, false);
            }
            TypeNode::Primitive { .. } | TypeNode::Void { .. } => {}
            TypeNode::Error { .. } => self.incomplete = true,
        }
    }

    fn type_from_node(&self, type_node: &TypeNode) -> EditorType {
        match type_node {
            TypeNode::Pointer { element_type, .. } => {
                EditorType::Pointer(Box::new(self.type_from_node(element_type)))
            }
            TypeNode::Slice { element_type, .. } => {
                EditorType::Slice(Box::new(self.type_from_node(element_type)))
            }
            TypeNode::Named { name, .. } => {
                if self.structs.contains_key(&name.name) {
                    EditorType::Struct(name.name.clone())
                } else {
                    EditorType::Unknown
                }
            }
            TypeNode::Primitive { .. } | TypeNode::Void { .. } => EditorType::Other,
            TypeNode::Error { .. } => EditorType::Unknown,
        }
    }

    fn add_scope(&mut self, parent: Option<ScopeId>, span: SourceSpan) -> ScopeId {
        let id = ScopeId(self.next_scope);
        self.next_scope += 1;
        self.analysis.scopes.push(EditorScope { id, parent, span });
        id
    }

    fn add_symbol(
        &mut self,
        name: &str,
        kind: SymbolKind,
        declaration: SourceSpan,
        scope_id: ScopeId,
        renameable: bool,
    ) -> SymbolId {
        let id = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        self.analysis.symbols.push(EditorSymbol {
            id,
            name: name.to_owned(),
            kind,
            declaration,
            scope_id,
            renameable,
        });
        id
    }

    fn add_occurrence(
        &mut self,
        identifier: &IdentifierNode,
        symbol_id: Option<SymbolId>,
        kind: EditorOccurrenceKind,
        is_declaration: bool,
    ) {
        self.add_occurrence_span(
            &identifier.name,
            identifier.span,
            symbol_id,
            kind,
            is_declaration,
        );
    }

    fn add_occurrence_span(
        &mut self,
        name: &str,
        span: SourceSpan,
        symbol_id: Option<SymbolId>,
        kind: EditorOccurrenceKind,
        is_declaration: bool,
    ) {
        self.analysis.occurrences.push(EditorOccurrence {
            name: name.to_owned(),
            span,
            symbol_id,
            kind,
            is_declaration,
        });
    }
}

fn source_end(source: &SourceFile) -> SourcePosition {
    let mut line = 1;
    let mut column = 1;
    for character in source.text.chars() {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += character.len_utf16();
        }
    }
    SourcePosition {
        offset: source.text.encode_utf16().count(),
        line,
        column,
    }
}

fn editor_type_from_calc(type_node: &super::CalcKernelType) -> EditorType {
    match type_node {
        super::CalcKernelType::Pointer(_) => EditorType::Pointer(Box::new(EditorType::Unknown)),
        super::CalcKernelType::Slice(_) => EditorType::Slice(Box::new(EditorType::Unknown)),
        super::CalcKernelType::Struct(name) => EditorType::Struct(name.clone()),
        super::CalcKernelType::Unknown => EditorType::Unknown,
        _ => EditorType::Other,
    }
}

fn is_identifier(name: &str) -> bool {
    let lexed = lex(&SourceFile::new("<rename>", name));
    lexed.diagnostics.is_empty()
        && lexed.tokens.len() == 2
        && lexed.tokens[0].kind == super::TokenKind::Identifier
        && lexed.tokens[0].text == name
        && lexed.tokens[1].kind == super::TokenKind::Eof
}

fn is_reserved_name(name: &str) -> bool {
    get_compiler_builtin(name).is_some() || is_contract_builtin(name)
}

fn is_contract_builtin(name: &str) -> bool {
    matches!(name, "aligned" | "multiple_of" | "noalias")
}

fn same_namespace(left: SymbolKind, right: SymbolKind) -> bool {
    matches!(
        (left, right),
        (SymbolKind::Function, SymbolKind::Function)
            | (SymbolKind::Struct, SymbolKind::Struct)
            | (SymbolKind::Field, SymbolKind::Field)
            | (
                SymbolKind::Parameter | SymbolKind::Local,
                SymbolKind::Parameter | SymbolKind::Local
            )
    )
}

fn utf16_offset_to_byte(text: &str, utf16_offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte_offset, character) in text.char_indices() {
        if units == utf16_offset {
            return Some(byte_offset);
        }
        let next = units + character.len_utf16();
        if utf16_offset < next {
            // Reject offsets in the middle of an astral character's surrogate pair.
            return None;
        }
        units = next;
    }
    (units == utf16_offset).then_some(text.len())
}

fn apply_edits(source: &SourceFile, edits: &[EditorTextEdit]) -> Option<SourceFile> {
    let mut converted = edits
        .iter()
        .map(|edit| {
            Some((
                utf16_offset_to_byte(&source.text, edit.span.start.offset)?,
                utf16_offset_to_byte(&source.text, edit.span.end.offset)?,
                edit.new_text.as_str(),
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    converted.sort_by_key(|(start, _, _)| *start);
    if converted.windows(2).any(|window| window[0].1 > window[1].0) {
        return None;
    }
    let mut text = source.text.clone();
    for (start, end, replacement) in converted.into_iter().rev() {
        text.replace_range(start..end, replacement);
    }
    Some(SourceFile::new(source.file_name.clone(), text))
}

fn bindings_are_stable(
    original: &EditorAnalysis,
    updated: &EditorAnalysis,
    renamed: SymbolId,
    new_name: &str,
    edits: &[EditorTextEdit],
) -> bool {
    let Some(updated_symbol) = updated.symbols.iter().find(|symbol| symbol.id == renamed) else {
        return false;
    };
    if updated_symbol.name != new_name {
        return false;
    }

    original.occurrences.iter().all(|occurrence| {
        let mut shift = 0isize;
        let mut expected_span = occurrence.span;
        let mut edited = false;
        for edit in edits {
            let old_start = edit.span.start.offset;
            let old_end = edit.span.end.offset;
            if old_start == occurrence.span.start.offset && old_end == occurrence.span.end.offset {
                let shifted_start = old_start as isize + shift;
                let Some(start) = usize::try_from(shifted_start).ok() else {
                    return false;
                };
                expected_span = SourceSpan {
                    start: position_with_offset(occurrence.span.start, start),
                    end: position_with_offset(
                        occurrence.span.end,
                        start + edit.new_text.encode_utf16().count(),
                    ),
                };
                edited = true;
                break;
            }
            if old_end <= occurrence.span.start.offset {
                shift +=
                    edit.new_text.encode_utf16().count() as isize - (old_end - old_start) as isize;
            }
        }
        if !edited {
            let Some(start) = usize::try_from(occurrence.span.start.offset as isize + shift).ok()
            else {
                return false;
            };
            let Some(end) = usize::try_from(occurrence.span.end.offset as isize + shift).ok()
            else {
                return false;
            };
            expected_span.start.offset = start;
            expected_span.end.offset = end;
        }
        let Some(candidate) = updated.occurrences.iter().find(|candidate| {
            candidate.span.start.offset == expected_span.start.offset
                && candidate.span.end.offset == expected_span.end.offset
        }) else {
            return false;
        };
        candidate.symbol_id == occurrence.symbol_id
            && candidate.kind == occurrence.kind
            && candidate.is_declaration == occurrence.is_declaration
            && candidate.name
                == if occurrence.symbol_id == Some(renamed) {
                    new_name
                } else {
                    occurrence.name.as_str()
                }
    }) && original.occurrences.len() == updated.occurrences.len()
}

fn position_with_offset(mut position: SourcePosition, offset: usize) -> SourcePosition {
    position.offset = offset;
    position
}
