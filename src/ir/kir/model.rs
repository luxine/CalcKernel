use std::{error::Error, fmt};

use crate::{
    MirBinaryOp, MirCastOp, MirCompareOp, MirEntryPoint, MirRuntimeIntrinsic, MirStruct, MirType,
    MirUnaryOp,
};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            #[must_use]
            pub const fn from_index(index: u32) -> Self {
                Self(index)
            }

            #[must_use]
            pub const fn index(self) -> u32 {
                self.0
            }
        }
    };
}

define_id!(FunctionId);
define_id!(BlockId);
define_id!(ValueId);
define_id!(InstructionId);
define_id!(MemoryRegionId);
define_id!(MemoryVersionId);
define_id!(FactId);
define_id!(ProofId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KirConsumer {
    C,
    WebAssembly,
    NativeLibrary,
    NativeExecutable,
    Inspection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KirOverflowMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KirBoundsMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KirSanitizerMode {
    Disabled,
    Contracts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KirBuildConfig {
    pub consumer: KirConsumer,
    pub overflow_mode: KirOverflowMode,
    pub bounds_mode: KirBoundsMode,
    pub sanitizer_mode: KirSanitizerMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirModule {
    pub config: KirBuildConfig,
    pub entry: Option<MirEntryPoint>,
    pub structs: Vec<MirStruct>,
    pub functions: Vec<KirFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirFunction {
    pub id: FunctionId,
    pub name: String,
    pub exported: bool,
    pub params: Vec<KirParam>,
    pub return_type: MirType,
    pub regions: Vec<KirMemoryRegion>,
    pub initial_memory: Vec<KirInitialMemory>,
    pub blocks: Vec<KirBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirParam {
    pub value: ValueId,
    pub name: String,
    pub type_node: MirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMemoryRegion {
    pub id: MemoryRegionId,
    pub origin: KirMemoryRegionOrigin,
    pub parent: Option<MemoryRegionId>,
    pub partition: MemoryRegionId,
    pub byte_interval: Option<KirSymbolicByteInterval>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirMemoryRegionOrigin {
    Conservative,
    Parameter(ValueId),
    RawSlice(ValueId),
    Subslice(ValueId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirSymbolicByteInterval {
    pub start: ValueId,
    pub end: ValueId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirInitialMemory {
    pub region: MemoryRegionId,
    pub version: MemoryVersionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirBlock {
    pub id: BlockId,
    pub label: String,
    pub params: Vec<KirBlockParam>,
    pub memory_params: Vec<KirMemoryBlockParam>,
    pub instructions: Vec<KirInstruction>,
    pub terminator: KirTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirBlockParam {
    pub value: ValueId,
    pub slot: String,
    pub type_node: MirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMemoryBlockParam {
    pub version: MemoryVersionId,
    pub region: MemoryRegionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirInstruction {
    pub id: InstructionId,
    pub results: Vec<KirResult>,
    pub kind: KirInstructionKind,
    pub memory: Option<KirMemoryAccess>,
    pub effect: Option<KirOrderedEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirResult {
    pub value: ValueId,
    pub type_node: MirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirInstructionKind {
    Undef {
        slot: String,
    },
    ConstInt {
        value: String,
    },
    ConstFloat {
        value: String,
    },
    ConstBool {
        value: bool,
    },
    Copy {
        value: ValueId,
    },
    Binary {
        op: MirBinaryOp,
        left: ValueId,
        right: ValueId,
        semantics: KirArithmeticSemantics,
    },
    Unary {
        op: MirUnaryOp,
        operand: ValueId,
        semantics: KirArithmeticSemantics,
    },
    Compare {
        op: MirCompareOp,
        left: ValueId,
        right: ValueId,
    },
    Cast {
        op: MirCastOp,
        value: ValueId,
    },
    CheckCondition {
        kind: KirCheckConditionKind,
        args: Vec<ValueId>,
    },
    Guard {
        condition: ValueId,
        failure: KirFailureKind,
    },
    Address {
        place: Box<KirPlace>,
    },
    Load {
        place: Box<KirPlace>,
    },
    Store {
        place: Box<KirPlace>,
        value: ValueId,
    },
    MakeSlice {
        data: ValueId,
        len: ValueId,
    },
    SliceData {
        slice: ValueId,
    },
    SliceLen {
        slice: ValueId,
    },
    Subslice {
        slice: ValueId,
        start: ValueId,
        end: ValueId,
    },
    Call {
        function_name: String,
        args: Vec<ValueId>,
    },
    RuntimeCall {
        intrinsic: MirRuntimeIntrinsic,
        args: Vec<ValueId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KirArithmeticSemantics {
    Modular,
    Checked,
    StrictFloat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KirCheckConditionKind {
    ArithmeticOverflow,
    DivisionByZero,
    SignedDivisionOverflow,
    SliceOutOfBounds,
    InvalidSubslice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KirFailureKind {
    Overflow,
    DivisionByZero,
    OutOfBounds,
    ContractViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirPlace {
    Value {
        value: ValueId,
        type_node: MirType,
        region: MemoryRegionId,
    },
    Deref {
        pointer: ValueId,
        type_node: MirType,
        region: MemoryRegionId,
    },
    Index {
        base: Box<KirPlace>,
        index: ValueId,
        type_node: MirType,
        region: MemoryRegionId,
    },
    SliceIndex {
        slice: ValueId,
        index: ValueId,
        type_node: MirType,
        region: MemoryRegionId,
    },
    Field {
        base: Box<KirPlace>,
        field_name: String,
        type_node: MirType,
        region: MemoryRegionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMemoryAccess {
    pub region: MemoryRegionId,
    pub input: MemoryVersionId,
    pub output: Option<MemoryVersionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirOrderedEffect {
    pub order: u32,
    pub kind: KirEffectKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KirEffectKind {
    ReadMemory,
    WriteMemory,
    MayFail,
    Runtime,
    Call,
    Return,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirTerminator {
    Return {
        value: Option<ValueId>,
        memory: Vec<(MemoryRegionId, MemoryVersionId)>,
        effect_order: u32,
    },
    Jump {
        edge: KirEdge,
    },
    Branch {
        condition: ValueId,
        then_edge: KirEdge,
        else_edge: KirEdge,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirEdge {
    pub target: BlockId,
    pub args: Vec<ValueId>,
    pub memory_args: Vec<MemoryVersionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirBuildError {
    pub message: String,
}

impl KirBuildError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for KirBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for KirBuildError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirValidationError {
    pub message: String,
    pub function: Option<FunctionId>,
    pub block: Option<BlockId>,
    pub instruction: Option<InstructionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirValidationResult {
    pub errors: Vec<KirValidationError>,
}
