use super::{diagnostics::Diagnostic, source::SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseResult {
    pub ast: Program,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierNode {
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub declarations: Vec<Declaration>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    Struct(StructDeclaration),
    Function(Box<FunctionDeclaration>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDeclaration {
    pub name: IdentifierNode,
    pub fields: Vec<StructField>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructField {
    pub name: IdentifierNode,
    pub type_node: TypeNode,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDeclaration {
    pub exported: bool,
    pub is_unsafe: bool,
    pub name: IdentifierNode,
    pub params: Vec<FunctionParam>,
    pub return_type: TypeNode,
    pub contract: Option<Box<ContractDeclaration>>,
    pub body: BlockStatement,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractDeclaration {
    pub requirements: Vec<ContractRequirement>,
    pub effects: Option<ContractEffectClause>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRequirement {
    pub expression: Expression,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractEffectKind {
    None,
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEffectItem {
    pub kind: ContractEffectKind,
    pub target: IdentifierNode,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEffectClause {
    pub is_none: bool,
    pub items: Vec<ContractEffectItem>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionParam {
    pub name: IdentifierNode,
    pub type_node: TypeNode,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeNode {
    Primitive {
        name: String,
        span: SourceSpan,
    },
    Void {
        span: SourceSpan,
    },
    Pointer {
        element_type: Box<TypeNode>,
        span: SourceSpan,
    },
    Slice {
        element_type: Box<TypeNode>,
        span: SourceSpan,
    },
    Named {
        name: IdentifierNode,
        span: SourceSpan,
    },
    Error {
        span: SourceSpan,
    },
}

impl TypeNode {
    #[must_use]
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Primitive { span, .. }
            | Self::Void { span }
            | Self::Pointer { span, .. }
            | Self::Slice { span, .. }
            | Self::Named { span, .. }
            | Self::Error { span } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Block(BlockStatement),
    Unsafe(UnsafeStatement),
    Let(LetStatement),
    Assignment(AssignmentStatement),
    Call(CallStatement),
    Return(ReturnStatement),
    Break(BreakStatement),
    Continue(ContinueStatement),
    If(IfStatement),
    While(WhileStatement),
    Error { span: SourceSpan },
}

impl Statement {
    #[must_use]
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Block(statement) => statement.span,
            Self::Unsafe(statement) => statement.span,
            Self::Let(statement) => statement.span,
            Self::Assignment(statement) => statement.span,
            Self::Call(statement) => statement.span,
            Self::Return(statement) => statement.span,
            Self::Break(statement) => statement.span,
            Self::Continue(statement) => statement.span,
            Self::If(statement) => statement.span,
            Self::While(statement) => statement.span,
            Self::Error { span } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockStatement {
    pub statements: Vec<Statement>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsafeStatement {
    pub block: BlockStatement,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetStatement {
    pub name: IdentifierNode,
    pub type_node: TypeNode,
    pub initializer: Expression,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentStatement {
    pub target: Expression,
    pub value: Expression,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallStatement {
    pub call: Expression,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnStatement {
    pub value: Option<Expression>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakStatement {
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueStatement {
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfStatement {
    pub condition: Expression,
    pub then_block: BlockStatement,
    pub else_block: Option<BlockStatement>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileStatement {
    pub condition: Expression,
    pub body: BlockStatement,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Identifier {
        name: String,
        span: SourceSpan,
    },
    IntegerLiteral {
        text: String,
        span: SourceSpan,
    },
    FloatLiteral {
        text: String,
        span: SourceSpan,
    },
    BoolLiteral {
        value: bool,
        span: SourceSpan,
    },
    Unary {
        operator: String,
        operand: Box<Expression>,
        span: SourceSpan,
    },
    Binary {
        operator: String,
        left: Box<Expression>,
        right: Box<Expression>,
        span: SourceSpan,
    },
    Call {
        callee: Box<Expression>,
        args: Vec<Expression>,
        span: SourceSpan,
    },
    SliceConstructor {
        data: Box<Expression>,
        len: Box<Expression>,
        span: SourceSpan,
    },
    Field {
        object: Box<Expression>,
        field: IdentifierNode,
        span: SourceSpan,
    },
    Index {
        object: Box<Expression>,
        index: Box<Expression>,
        span: SourceSpan,
    },
    Subslice {
        slice: Box<Expression>,
        start: Box<Expression>,
        end: Box<Expression>,
        span: SourceSpan,
    },
    Parenthesized {
        expression: Box<Expression>,
        span: SourceSpan,
    },
    Error {
        span: SourceSpan,
    },
}

impl Expression {
    #[must_use]
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Identifier { span, .. }
            | Self::IntegerLiteral { span, .. }
            | Self::FloatLiteral { span, .. }
            | Self::BoolLiteral { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Call { span, .. }
            | Self::SliceConstructor { span, .. }
            | Self::Field { span, .. }
            | Self::Index { span, .. }
            | Self::Subslice { span, .. }
            | Self::Parenthesized { span, .. }
            | Self::Error { span } => *span,
        }
    }
}
