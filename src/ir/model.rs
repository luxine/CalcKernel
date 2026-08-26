use std::{error::Error, fmt};

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
    Slice(Box<MirType>),
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
    SliceIndex {
        slice: MirValue,
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
    MakeSlice {
        target: MirValue,
        data: MirValue,
        len: MirValue,
    },
    SliceData {
        target: MirValue,
        slice: MirValue,
    },
    SliceLen {
        target: MirValue,
        slice: MirValue,
    },
    Subslice {
        target: MirValue,
        slice: MirValue,
        start: MirValue,
        end: MirValue,
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
    pub(super) fn new(message: impl Into<String>) -> Self {
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

#[must_use]
pub fn mir_primitive(name: MirPrimitiveTypeName) -> MirType {
    MirType::Primitive(name)
}

#[must_use]
pub fn mir_pointer(element_type: MirType) -> MirType {
    MirType::Pointer(Box::new(element_type))
}

#[must_use]
pub fn mir_slice(element_type: MirType) -> MirType {
    MirType::Slice(Box::new(element_type))
}

#[must_use]
pub fn mir_struct(name: impl Into<String>) -> MirType {
    MirType::Struct(name.into())
}

pub(super) fn value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}

pub(super) fn place_type(place: &MirPlace) -> &MirType {
    match place {
        MirPlace::Param { type_node, .. }
        | MirPlace::Local { type_node, .. }
        | MirPlace::Deref { type_node, .. }
        | MirPlace::Index { type_node, .. }
        | MirPlace::SliceIndex { type_node, .. }
        | MirPlace::Field { type_node, .. } => type_node,
    }
}
