//! Typed representation of the closed WAT grammar emitted by this backend.
//!
//! The emitter writes short chunks (usually complete lines) into `FinalWasmBuilder`.
//! The builder owns only a partial line plus typed declarations and instructions;
//! the binary path never materializes a module-sized WAT string.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum WasmValueType {
    I32,
    I64,
    F32,
    F64,
    V128,
}

impl WasmValueType {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "i32" => Some(Self::I32),
            "i64" => Some(Self::I64),
            "f32" => Some(Self::F32),
            "f64" => Some(Self::F64),
            "v128" => Some(Self::V128),
            _ => None,
        }
    }

    pub(super) fn wat(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::V128 => "v128",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FinalLocal {
    pub name: String,
    pub ty: WasmValueType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FinalWasmFunction {
    pub name: String,
    pub export_name: Option<String>,
    pub params: Vec<FinalLocal>,
    pub results: Vec<WasmValueType>,
    pub locals: Vec<FinalLocal>,
    pub body: Vec<FinalInstruction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FinalInstruction {
    /// Retained only for stable, readable WAT output. The encoder ignores it.
    pub indent: usize,
    pub kind: FinalInstructionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FinalInstructionKind {
    Simple(SimpleOpcode),
    I32Const(i32),
    I64Const(i64),
    F64Const(u64),
    V128Const(Vec<u8>),
    LocalGet(String),
    LocalSet(String),
    LocalTee(String),
    Call(String),
    Block {
        label: Option<String>,
        results: Vec<WasmValueType>,
    },
    Loop {
        label: Option<String>,
        results: Vec<WasmValueType>,
    },
    If {
        label: Option<String>,
        results: Vec<WasmValueType>,
    },
    Else,
    End,
    Br(String),
    BrIf(String),
    BrTable {
        targets: Vec<String>,
        default: String,
    },
    Load {
        opcode: MemoryOpcode,
        offset: u64,
        align: u32,
    },
    Store {
        opcode: MemoryOpcode,
        offset: u64,
        align: u32,
    },
    MemorySize,
    MemoryGrow,
    MemoryCopy,
    MemoryFill,
    Lane {
        opcode: LaneOpcode,
        lane: u8,
    },
    Shuffle([u8; 16]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MemoryOpcode {
    I32Load,
    I64Load,
    F32Load,
    F64Load,
    I32Load8S,
    I32Load8U,
    I32Load16S,
    I32Load16U,
    I64Load8S,
    I64Load8U,
    I64Load16S,
    I64Load16U,
    I64Load32S,
    I64Load32U,
    I32Store,
    I64Store,
    F32Store,
    F64Store,
    I32Store8,
    I32Store16,
    I64Store8,
    I64Store16,
    I64Store32,
    V128Load,
    V128Load8x8S,
    V128Load8x8U,
    V128Load16x4S,
    V128Load16x4U,
    V128Load32x2S,
    V128Load32x2U,
    V128Load8Splat,
    V128Load16Splat,
    V128Load32Splat,
    V128Load64Splat,
    V128Load32Zero,
    V128Load64Zero,
    V128Store,
}

impl MemoryOpcode {
    fn parse(text: &str) -> Option<(Self, bool)> {
        let (opcode, store) = match text {
            "i32.load" => (Self::I32Load, false),
            "i64.load" => (Self::I64Load, false),
            "f32.load" => (Self::F32Load, false),
            "f64.load" => (Self::F64Load, false),
            "i32.load8_s" => (Self::I32Load8S, false),
            "i32.load8_u" => (Self::I32Load8U, false),
            "i32.load16_s" => (Self::I32Load16S, false),
            "i32.load16_u" => (Self::I32Load16U, false),
            "i64.load8_s" => (Self::I64Load8S, false),
            "i64.load8_u" => (Self::I64Load8U, false),
            "i64.load16_s" => (Self::I64Load16S, false),
            "i64.load16_u" => (Self::I64Load16U, false),
            "i64.load32_s" => (Self::I64Load32S, false),
            "i64.load32_u" => (Self::I64Load32U, false),
            "i32.store" => (Self::I32Store, true),
            "i64.store" => (Self::I64Store, true),
            "f32.store" => (Self::F32Store, true),
            "f64.store" => (Self::F64Store, true),
            "i32.store8" => (Self::I32Store8, true),
            "i32.store16" => (Self::I32Store16, true),
            "i64.store8" => (Self::I64Store8, true),
            "i64.store16" => (Self::I64Store16, true),
            "i64.store32" => (Self::I64Store32, true),
            "v128.load" => (Self::V128Load, false),
            "v128.load8x8_s" => (Self::V128Load8x8S, false),
            "v128.load8x8_u" => (Self::V128Load8x8U, false),
            "v128.load16x4_s" => (Self::V128Load16x4S, false),
            "v128.load16x4_u" => (Self::V128Load16x4U, false),
            "v128.load32x2_s" => (Self::V128Load32x2S, false),
            "v128.load32x2_u" => (Self::V128Load32x2U, false),
            "v128.load8_splat" => (Self::V128Load8Splat, false),
            "v128.load16_splat" => (Self::V128Load16Splat, false),
            "v128.load32_splat" => (Self::V128Load32Splat, false),
            "v128.load64_splat" => (Self::V128Load64Splat, false),
            "v128.load32_zero" => (Self::V128Load32Zero, false),
            "v128.load64_zero" => (Self::V128Load64Zero, false),
            "v128.store" => (Self::V128Store, true),
            _ => return None,
        };
        Some((opcode, store))
    }

    pub(super) fn wat(self) -> &'static str {
        match self {
            Self::I32Load => "i32.load",
            Self::I64Load => "i64.load",
            Self::F32Load => "f32.load",
            Self::F64Load => "f64.load",
            Self::I32Load8S => "i32.load8_s",
            Self::I32Load8U => "i32.load8_u",
            Self::I32Load16S => "i32.load16_s",
            Self::I32Load16U => "i32.load16_u",
            Self::I64Load8S => "i64.load8_s",
            Self::I64Load8U => "i64.load8_u",
            Self::I64Load16S => "i64.load16_s",
            Self::I64Load16U => "i64.load16_u",
            Self::I64Load32S => "i64.load32_s",
            Self::I64Load32U => "i64.load32_u",
            Self::I32Store => "i32.store",
            Self::I64Store => "i64.store",
            Self::F32Store => "f32.store",
            Self::F64Store => "f64.store",
            Self::I32Store8 => "i32.store8",
            Self::I32Store16 => "i32.store16",
            Self::I64Store8 => "i64.store8",
            Self::I64Store16 => "i64.store16",
            Self::I64Store32 => "i64.store32",
            Self::V128Load => "v128.load",
            Self::V128Load8x8S => "v128.load8x8_s",
            Self::V128Load8x8U => "v128.load8x8_u",
            Self::V128Load16x4S => "v128.load16x4_s",
            Self::V128Load16x4U => "v128.load16x4_u",
            Self::V128Load32x2S => "v128.load32x2_s",
            Self::V128Load32x2U => "v128.load32x2_u",
            Self::V128Load8Splat => "v128.load8_splat",
            Self::V128Load16Splat => "v128.load16_splat",
            Self::V128Load32Splat => "v128.load32_splat",
            Self::V128Load64Splat => "v128.load64_splat",
            Self::V128Load32Zero => "v128.load32_zero",
            Self::V128Load64Zero => "v128.load64_zero",
            Self::V128Store => "v128.store",
        }
    }

    pub(super) fn is_store(self) -> bool {
        matches!(
            self,
            Self::I32Store
                | Self::I64Store
                | Self::F32Store
                | Self::F64Store
                | Self::I32Store8
                | Self::I32Store16
                | Self::I64Store8
                | Self::I64Store16
                | Self::I64Store32
                | Self::V128Store
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LaneOpcode {
    I8x16ExtractLaneS,
    I8x16ExtractLaneU,
    I8x16ReplaceLane,
    I16x8ExtractLaneS,
    I16x8ExtractLaneU,
    I16x8ReplaceLane,
    I32x4ExtractLane,
    I32x4ReplaceLane,
    I64x2ExtractLane,
    I64x2ReplaceLane,
    F32x4ExtractLane,
    F32x4ReplaceLane,
    F64x2ExtractLane,
    F64x2ReplaceLane,
}

impl LaneOpcode {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "i8x16.extract_lane_s" => Self::I8x16ExtractLaneS,
            "i8x16.extract_lane_u" => Self::I8x16ExtractLaneU,
            "i8x16.replace_lane" => Self::I8x16ReplaceLane,
            "i16x8.extract_lane_s" => Self::I16x8ExtractLaneS,
            "i16x8.extract_lane_u" => Self::I16x8ExtractLaneU,
            "i16x8.replace_lane" => Self::I16x8ReplaceLane,
            "i32x4.extract_lane" => Self::I32x4ExtractLane,
            "i32x4.replace_lane" => Self::I32x4ReplaceLane,
            "i64x2.extract_lane" => Self::I64x2ExtractLane,
            "i64x2.replace_lane" => Self::I64x2ReplaceLane,
            "f32x4.extract_lane" => Self::F32x4ExtractLane,
            "f32x4.replace_lane" => Self::F32x4ReplaceLane,
            "f64x2.extract_lane" => Self::F64x2ExtractLane,
            "f64x2.replace_lane" => Self::F64x2ReplaceLane,
            _ => return None,
        })
    }

    pub(super) fn wat(self) -> &'static str {
        match self {
            Self::I8x16ExtractLaneS => "i8x16.extract_lane_s",
            Self::I8x16ExtractLaneU => "i8x16.extract_lane_u",
            Self::I8x16ReplaceLane => "i8x16.replace_lane",
            Self::I16x8ExtractLaneS => "i16x8.extract_lane_s",
            Self::I16x8ExtractLaneU => "i16x8.extract_lane_u",
            Self::I16x8ReplaceLane => "i16x8.replace_lane",
            Self::I32x4ExtractLane => "i32x4.extract_lane",
            Self::I32x4ReplaceLane => "i32x4.replace_lane",
            Self::I64x2ExtractLane => "i64x2.extract_lane",
            Self::I64x2ReplaceLane => "i64x2.replace_lane",
            Self::F32x4ExtractLane => "f32x4.extract_lane",
            Self::F32x4ReplaceLane => "f32x4.replace_lane",
            Self::F64x2ExtractLane => "f64x2.extract_lane",
            Self::F64x2ReplaceLane => "f64x2.replace_lane",
        }
    }
}

macro_rules! simple_opcodes {
    ($($name:ident => $text:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(super) enum SimpleOpcode { $($name),+ }

        impl SimpleOpcode {
            fn parse(text: &str) -> Option<Self> {
                Some(match text { $($text => Self::$name,)+ _ => return None })
            }

            pub(super) fn wat(self) -> &'static str {
                match self { $(Self::$name => $text,)+ }
            }

            pub(super) fn encoder_instruction(self) -> wasm_encoder::Instruction<'static> {
                match self { $(Self::$name => wasm_encoder::Instruction::$name,)+ }
            }
        }
    };
}

simple_opcodes! {
    Unreachable => "unreachable", Nop => "nop", Drop => "drop", Select => "select", Return => "return",
    I32Eqz => "i32.eqz", I32Eq => "i32.eq", I32Ne => "i32.ne", I32LtS => "i32.lt_s", I32LtU => "i32.lt_u", I32GtS => "i32.gt_s", I32GtU => "i32.gt_u", I32LeS => "i32.le_s", I32LeU => "i32.le_u", I32GeS => "i32.ge_s", I32GeU => "i32.ge_u",
    I64Eqz => "i64.eqz", I64Eq => "i64.eq", I64Ne => "i64.ne", I64LtS => "i64.lt_s", I64LtU => "i64.lt_u", I64GtS => "i64.gt_s", I64GtU => "i64.gt_u", I64LeS => "i64.le_s", I64LeU => "i64.le_u", I64GeS => "i64.ge_s", I64GeU => "i64.ge_u",
    F32Eq => "f32.eq", F32Ne => "f32.ne", F32Lt => "f32.lt", F32Gt => "f32.gt", F32Le => "f32.le", F32Ge => "f32.ge",
    F64Eq => "f64.eq", F64Ne => "f64.ne", F64Lt => "f64.lt", F64Gt => "f64.gt", F64Le => "f64.le", F64Ge => "f64.ge",
    I32Clz => "i32.clz", I32Ctz => "i32.ctz", I32Popcnt => "i32.popcnt", I32Add => "i32.add", I32Sub => "i32.sub", I32Mul => "i32.mul", I32DivS => "i32.div_s", I32DivU => "i32.div_u", I32RemS => "i32.rem_s", I32RemU => "i32.rem_u", I32And => "i32.and", I32Or => "i32.or", I32Xor => "i32.xor", I32Shl => "i32.shl", I32ShrS => "i32.shr_s", I32ShrU => "i32.shr_u", I32Rotl => "i32.rotl", I32Rotr => "i32.rotr",
    I64Clz => "i64.clz", I64Ctz => "i64.ctz", I64Popcnt => "i64.popcnt", I64Add => "i64.add", I64Sub => "i64.sub", I64Mul => "i64.mul", I64DivS => "i64.div_s", I64DivU => "i64.div_u", I64RemS => "i64.rem_s", I64RemU => "i64.rem_u", I64And => "i64.and", I64Or => "i64.or", I64Xor => "i64.xor", I64Shl => "i64.shl", I64ShrS => "i64.shr_s", I64ShrU => "i64.shr_u", I64Rotl => "i64.rotl", I64Rotr => "i64.rotr",
    F32Abs => "f32.abs", F32Neg => "f32.neg", F32Ceil => "f32.ceil", F32Floor => "f32.floor", F32Trunc => "f32.trunc", F32Nearest => "f32.nearest", F32Sqrt => "f32.sqrt", F32Add => "f32.add", F32Sub => "f32.sub", F32Mul => "f32.mul", F32Div => "f32.div", F32Min => "f32.min", F32Max => "f32.max", F32Copysign => "f32.copysign",
    F64Abs => "f64.abs", F64Neg => "f64.neg", F64Ceil => "f64.ceil", F64Floor => "f64.floor", F64Trunc => "f64.trunc", F64Nearest => "f64.nearest", F64Sqrt => "f64.sqrt", F64Add => "f64.add", F64Sub => "f64.sub", F64Mul => "f64.mul", F64Div => "f64.div", F64Min => "f64.min", F64Max => "f64.max", F64Copysign => "f64.copysign",
    I32WrapI64 => "i32.wrap_i64", I32TruncF32S => "i32.trunc_f32_s", I32TruncF32U => "i32.trunc_f32_u", I32TruncF64S => "i32.trunc_f64_s", I32TruncF64U => "i32.trunc_f64_u", I64ExtendI32S => "i64.extend_i32_s", I64ExtendI32U => "i64.extend_i32_u", I64TruncF32S => "i64.trunc_f32_s", I64TruncF32U => "i64.trunc_f32_u", I64TruncF64S => "i64.trunc_f64_s", I64TruncF64U => "i64.trunc_f64_u",
    F32ConvertI32S => "f32.convert_i32_s", F32ConvertI32U => "f32.convert_i32_u", F32ConvertI64S => "f32.convert_i64_s", F32ConvertI64U => "f32.convert_i64_u", F32DemoteF64 => "f32.demote_f64", F64ConvertI32S => "f64.convert_i32_s", F64ConvertI32U => "f64.convert_i32_u", F64ConvertI64S => "f64.convert_i64_s", F64ConvertI64U => "f64.convert_i64_u", F64PromoteF32 => "f64.promote_f32", I32ReinterpretF32 => "i32.reinterpret_f32", I64ReinterpretF64 => "i64.reinterpret_f64", F32ReinterpretI32 => "f32.reinterpret_i32", F64ReinterpretI64 => "f64.reinterpret_i64",
    I32Extend8S => "i32.extend8_s", I32Extend16S => "i32.extend16_s", I64Extend8S => "i64.extend8_s", I64Extend16S => "i64.extend16_s", I64Extend32S => "i64.extend32_s",
    I32TruncSatF32S => "i32.trunc_sat_f32_s", I32TruncSatF32U => "i32.trunc_sat_f32_u", I32TruncSatF64S => "i32.trunc_sat_f64_s", I32TruncSatF64U => "i32.trunc_sat_f64_u", I64TruncSatF32S => "i64.trunc_sat_f32_s", I64TruncSatF32U => "i64.trunc_sat_f32_u", I64TruncSatF64S => "i64.trunc_sat_f64_s", I64TruncSatF64U => "i64.trunc_sat_f64_u",
    V128Not => "v128.not", V128And => "v128.and", V128AndNot => "v128.andnot", V128Or => "v128.or", V128Xor => "v128.xor", V128Bitselect => "v128.bitselect", V128AnyTrue => "v128.any_true",
    I8x16Swizzle => "i8x16.swizzle", I8x16Splat => "i8x16.splat", I16x8Splat => "i16x8.splat", I32x4Splat => "i32x4.splat", I64x2Splat => "i64x2.splat", F32x4Splat => "f32x4.splat", F64x2Splat => "f64x2.splat",
    I8x16Abs => "i8x16.abs", I8x16Neg => "i8x16.neg", I8x16Popcnt => "i8x16.popcnt", I8x16AllTrue => "i8x16.all_true", I8x16Bitmask => "i8x16.bitmask",
    I16x8Abs => "i16x8.abs", I16x8Neg => "i16x8.neg", I16x8Q15MulrSatS => "i16x8.q15mulr_sat_s", I16x8AllTrue => "i16x8.all_true", I16x8Bitmask => "i16x8.bitmask",
    I32x4Abs => "i32x4.abs", I32x4Neg => "i32x4.neg", I32x4AllTrue => "i32x4.all_true", I32x4Bitmask => "i32x4.bitmask",
    I64x2Abs => "i64x2.abs", I64x2Neg => "i64x2.neg", I64x2AllTrue => "i64x2.all_true", I64x2Bitmask => "i64x2.bitmask",
    F32x4Ceil => "f32x4.ceil", F32x4Floor => "f32x4.floor", F32x4Trunc => "f32x4.trunc", F32x4Nearest => "f32x4.nearest", F32x4Abs => "f32x4.abs", F32x4Neg => "f32x4.neg", F32x4Sqrt => "f32x4.sqrt", F32x4Add => "f32x4.add", F32x4Sub => "f32x4.sub", F32x4Mul => "f32x4.mul", F32x4Div => "f32x4.div", F32x4Min => "f32x4.min", F32x4Max => "f32x4.max", F32x4PMin => "f32x4.pmin", F32x4PMax => "f32x4.pmax",
    F64x2Abs => "f64x2.abs", F64x2Neg => "f64x2.neg", F64x2Sqrt => "f64x2.sqrt", F64x2Add => "f64x2.add", F64x2Sub => "f64x2.sub", F64x2Mul => "f64x2.mul", F64x2Div => "f64x2.div", F64x2Min => "f64x2.min", F64x2Max => "f64x2.max", F64x2PMin => "f64x2.pmin", F64x2PMax => "f64x2.pmax",
    F64x2ConvertLowI32x4S => "f64x2.convert_low_i32x4_s", F64x2ConvertLowI32x4U => "f64x2.convert_low_i32x4_u",
    I8x16Eq => "i8x16.eq", I8x16Ne => "i8x16.ne", I8x16LtS => "i8x16.lt_s", I8x16LtU => "i8x16.lt_u", I8x16GtS => "i8x16.gt_s", I8x16GtU => "i8x16.gt_u", I8x16LeS => "i8x16.le_s", I8x16LeU => "i8x16.le_u", I8x16GeS => "i8x16.ge_s", I8x16GeU => "i8x16.ge_u",
    I16x8Eq => "i16x8.eq", I16x8Ne => "i16x8.ne", I16x8LtS => "i16x8.lt_s", I16x8LtU => "i16x8.lt_u", I16x8GtS => "i16x8.gt_s", I16x8GtU => "i16x8.gt_u", I16x8LeS => "i16x8.le_s", I16x8LeU => "i16x8.le_u", I16x8GeS => "i16x8.ge_s", I16x8GeU => "i16x8.ge_u",
    I32x4Add => "i32x4.add", I32x4Sub => "i32x4.sub", I32x4Mul => "i32x4.mul",
    I32x4Eq => "i32x4.eq", I32x4Ne => "i32x4.ne", I32x4LtS => "i32x4.lt_s", I32x4LtU => "i32x4.lt_u", I32x4GtS => "i32x4.gt_s", I32x4GtU => "i32x4.gt_u", I32x4LeS => "i32x4.le_s", I32x4LeU => "i32x4.le_u", I32x4GeS => "i32x4.ge_s", I32x4GeU => "i32x4.ge_u",
    I64x2Eq => "i64x2.eq", I64x2Ne => "i64x2.ne", I64x2LtS => "i64x2.lt_s", I64x2GtS => "i64x2.gt_s", I64x2LeS => "i64x2.le_s", I64x2GeS => "i64x2.ge_s",
    F32x4Eq => "f32x4.eq", F32x4Ne => "f32x4.ne", F32x4Lt => "f32x4.lt", F32x4Gt => "f32x4.gt", F32x4Le => "f32x4.le", F32x4Ge => "f32x4.ge",
    F64x2Eq => "f64x2.eq", F64x2Ne => "f64x2.ne", F64x2Lt => "f64x2.lt", F64x2Gt => "f64x2.gt", F64x2Le => "f64x2.le", F64x2Ge => "f64x2.ge",
}

#[derive(Debug, Clone, Default)]
pub(super) struct FinalWasmModule {
    pub functions: Vec<FinalWasmFunction>,
    pub memory_minimum: u32,
    pub target_metadata: Option<Vec<u8>>,
}

impl FinalWasmModule {
    pub(super) fn set_target_metadata(&mut self, metadata: Vec<u8>) -> Result<(), String> {
        if self.target_metadata.replace(metadata).is_some() {
            return Err("WebAssembly module contains more than one ck.wasm.target payload".into());
        }
        Ok(())
    }

    pub(super) fn to_wat(&self) -> String {
        let mut out = String::from("(module\n");
        out.push_str(&format!(
            "  (memory (export \"memory\") {})\n",
            self.memory_minimum
        ));
        out.push_str("  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n");
        for function in &self.functions {
            out.push('\n');
            out.push_str("  (func $");
            out.push_str(&function.name);
            if let Some(export_name) = &function.export_name {
                out.push_str(&format!(" (export \"{export_name}\")"));
            }
            out.push('\n');
            for param in &function.params {
                out.push_str(&format!("    (param ${} {})\n", param.name, param.ty.wat()));
            }
            if !function.results.is_empty() {
                out.push_str("    (result");
                for result in &function.results {
                    out.push(' ');
                    out.push_str(result.wat());
                }
                out.push_str(")\n");
            }
            for local in &function.locals {
                out.push_str(&format!("    (local ${} {})\n", local.name, local.ty.wat()));
            }
            for instruction in &function.body {
                out.push_str(&" ".repeat(instruction.indent));
                write_instruction(&mut out, &instruction.kind);
                out.push('\n');
            }
            out.push_str("  )\n");
        }
        if let Some(metadata) = &self.target_metadata {
            let text = String::from_utf8_lossy(metadata);
            let escaped = text
                .bytes()
                .map(|byte| match byte {
                    b'"' => "\\22".to_string(),
                    b'\\' => "\\5c".to_string(),
                    0x20..=0x7e => char::from(byte).to_string(),
                    _ => format!("\\{byte:02x}"),
                })
                .collect::<String>();
            out.push_str(&format!("  (@custom \"ck.wasm.target\" \"{escaped}\")\n"));
        }
        out.push_str(")\n");
        out
    }
}

/// Shared emitter surface. The `String` implementation keeps the existing WAT
/// debug path working while final emission can target the incremental builder.
pub(super) trait WasmOutput {
    fn push_str(&mut self, text: &str);
    fn push(&mut self, character: char);
}

impl WasmOutput for String {
    fn push_str(&mut self, text: &str) {
        String::push_str(self, text);
    }
    fn push(&mut self, character: char) {
        String::push(self, character);
    }
}

#[derive(Debug, Default)]
pub(super) struct FinalWasmBuilder {
    module: FinalWasmModule,
    current_function: Option<FinalWasmFunction>,
    pending: String,
    failure: Option<String>,
    module_open: bool,
    module_closed: bool,
}

impl FinalWasmBuilder {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn push_str(&mut self, text: &str) {
        if self.failure.is_some() {
            return;
        }
        self.pending.push_str(text);
        let input = std::mem::take(&mut self.pending);
        let mut start = 0;
        for (index, byte) in input.bytes().enumerate() {
            if byte == b'\n' {
                if let Err(error) = self.parse_line(&input[start..index]) {
                    self.failure = Some(error);
                    return;
                }
                start = index + 1;
            }
        }
        self.pending.push_str(&input[start..]);
    }

    pub(super) fn push(&mut self, character: char) {
        let mut buffer = [0; 4];
        self.push_str(character.encode_utf8(&mut buffer));
    }

    pub(super) fn finish(mut self) -> Result<FinalWasmModule, String> {
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.parse_line(&line)?;
        }
        if !self.module_open || !self.module_closed || self.current_function.is_some() {
            return Err("generated WebAssembly text ended before its module was complete".into());
        }
        validate_module(&self.module)?;
        Ok(self.module)
    }

    fn parse_line(&mut self, raw: &str) -> Result<(), String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if self.module_closed {
            return Err(format!(
                "generated text appeared after the module terminator: {trimmed}"
            ));
        }
        if !self.module_open {
            if trimmed != "(module" {
                return Err(format!(
                    "unexpected generated WebAssembly text before module: {trimmed}"
                ));
            }
            self.module_open = true;
            self.module.memory_minimum = 1;
            return Ok(());
        }
        if let Some(function) = self.current_function.as_mut() {
            if trimmed == ")" {
                let complete = self
                    .current_function
                    .take()
                    .expect("current function was present");
                self.module.functions.push(complete);
                return Ok(());
            }
            if let Some(inner) = trimmed
                .strip_prefix("(param ")
                .and_then(|x| x.strip_suffix(')'))
            {
                function.params.push(parse_local(inner, "param")?);
                return Ok(());
            }
            if let Some(inner) = trimmed
                .strip_prefix("(local ")
                .and_then(|x| x.strip_suffix(')'))
            {
                function.locals.push(parse_local(inner, "local")?);
                return Ok(());
            }
            if let Some(inner) = trimmed
                .strip_prefix("(result ")
                .and_then(|x| x.strip_suffix(')'))
            {
                if !function.results.is_empty() {
                    return Err("generated function contains multiple result declarations".into());
                }
                function.results = parse_types(inner)?;
                if function.results.is_empty() {
                    return Err("generated result declaration is empty".into());
                }
                return Ok(());
            }
            function.body.push(parse_instruction(raw)?);
            return Ok(());
        }
        if trimmed == ")" {
            self.module_closed = true;
            return Ok(());
        }
        if trimmed.starts_with("(func $") {
            self.current_function = Some(parse_function_header(trimmed)?);
            return Ok(());
        }
        if trimmed.starts_with("(memory ") {
            self.module.memory_minimum = parse_memory(trimmed)?;
            return Ok(());
        }
        if trimmed.starts_with("(@custom ") {
            self.module
                .set_target_metadata(parse_target_custom(trimmed)?)?;
            return Ok(());
        }
        if trimmed.starts_with("(global ") {
            if trimmed != "(global (export \"__ck_heap_base\") i32 (i32.const 0))" {
                return Err(format!(
                    "unsupported generated global declaration: {trimmed}"
                ));
            }
            return Ok(());
        }
        Err(format!(
            "unsupported generated module declaration: {trimmed}"
        ))
    }
}

impl WasmOutput for FinalWasmBuilder {
    fn push_str(&mut self, text: &str) {
        FinalWasmBuilder::push_str(self, text);
    }
    fn push(&mut self, character: char) {
        FinalWasmBuilder::push(self, character);
    }
}

fn parse_memory(line: &str) -> Result<u32, String> {
    let value = line
        .strip_prefix("(memory (export \"memory\") ")
        .and_then(|text| text.strip_suffix(')'))
        .ok_or_else(|| format!("unsupported generated memory declaration: {line}"))?;
    value
        .parse()
        .map_err(|_| format!("invalid generated memory minimum: {value}"))
}

fn parse_function_header(line: &str) -> Result<FinalWasmFunction, String> {
    let inner = line
        .strip_prefix("(func $")
        .ok_or_else(|| format!("invalid generated function header: {line}"))?;
    let (name, export_name) = if let Some((name, rest)) = inner.split_once(" (export \"") {
        let export_name = rest
            .strip_suffix("\")")
            .ok_or_else(|| format!("invalid export in function header: {line}"))?;
        (name, Some(export_name.to_string()))
    } else {
        (inner, None)
    };
    if !valid_identifier(name)
        || export_name
            .as_deref()
            .is_some_and(|value| value.contains('"'))
    {
        return Err(format!("invalid generated function identifier: {line}"));
    }
    Ok(FinalWasmFunction {
        name: name.to_string(),
        export_name,
        params: Vec::new(),
        results: Vec::new(),
        locals: Vec::new(),
        body: Vec::new(),
    })
}

fn parse_local(inner: &str, kind: &str) -> Result<FinalLocal, String> {
    let mut pieces = inner.split_whitespace();
    let name = pieces
        .next()
        .and_then(|text| text.strip_prefix('$'))
        .filter(|text| valid_identifier(text))
        .ok_or_else(|| format!("invalid generated {kind} declaration: ({kind} {inner})"))?;
    let ty = pieces
        .next()
        .and_then(WasmValueType::parse)
        .ok_or_else(|| format!("invalid generated {kind} type: ({kind} {inner})"))?;
    if pieces.next().is_some() {
        return Err(format!(
            "extra tokens in generated {kind} declaration: ({kind} {inner})"
        ));
    }
    Ok(FinalLocal {
        name: name.to_string(),
        ty,
    })
}

fn parse_types(text: &str) -> Result<Vec<WasmValueType>, String> {
    text.split_whitespace()
        .map(|text| {
            WasmValueType::parse(text)
                .ok_or_else(|| format!("unsupported generated WebAssembly value type: {text}"))
        })
        .collect()
}

fn valid_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
}

fn parse_instruction(raw: &str) -> Result<FinalInstruction, String> {
    let indent = raw.len() - raw.trim_start_matches(' ').len();
    let line = raw.trim();
    let mut words = line.split_whitespace();
    let opcode = words
        .next()
        .ok_or_else(|| "empty generated instruction".to_string())?;
    let rest = words.collect::<Vec<_>>();
    let fail = || format!("unsupported generated WebAssembly instruction: {line}");
    let parse_label = |text: &str| -> Result<String, String> {
        let name = text
            .strip_prefix('$')
            .filter(|name| valid_identifier(name))
            .ok_or_else(fail)?;
        Ok(name.to_string())
    };
    let kind = match opcode {
        "block" | "loop" | "if" => {
            let (label, results) = parse_control_header(line)?;
            match opcode {
                "block" => FinalInstructionKind::Block { label, results },
                "loop" => FinalInstructionKind::Loop { label, results },
                _ => FinalInstructionKind::If { label, results },
            }
        }
        "else" if rest.is_empty() => FinalInstructionKind::Else,
        "end" if rest.is_empty() => FinalInstructionKind::End,
        "br" | "br_if" if rest.len() == 1 => {
            let label = parse_label(rest[0])?;
            if opcode == "br" {
                FinalInstructionKind::Br(label)
            } else {
                FinalInstructionKind::BrIf(label)
            }
        }
        "br_table" if !rest.is_empty() => {
            let labels = rest
                .iter()
                .map(|text| parse_label(text))
                .collect::<Result<Vec<_>, _>>()?;
            let default = labels.last().cloned().ok_or_else(fail)?;
            FinalInstructionKind::BrTable {
                targets: labels[..labels.len() - 1].to_vec(),
                default,
            }
        }
        "local.get" | "local.set" | "local.tee" if rest.len() == 1 => {
            let name = parse_label(rest[0])?;
            match opcode {
                "local.get" => FinalInstructionKind::LocalGet(name),
                "local.set" => FinalInstructionKind::LocalSet(name),
                _ => FinalInstructionKind::LocalTee(name),
            }
        }
        "call" if rest.len() == 1 => FinalInstructionKind::Call(parse_label(rest[0])?),
        "i32.const" if rest.len() == 1 => {
            FinalInstructionKind::I32Const(parse_i32(rest[0]).ok_or_else(fail)?)
        }
        "i64.const" if rest.len() == 1 => {
            FinalInstructionKind::I64Const(parse_i64(rest[0]).ok_or_else(fail)?)
        }
        "f64.const" if rest.len() == 1 => {
            FinalInstructionKind::F64Const(parse_f64_bits(rest[0]).ok_or_else(fail)?)
        }
        "v128.const" => FinalInstructionKind::V128Const(parse_v128_const(&rest).ok_or_else(fail)?),
        "memory.copy" if rest.is_empty() => FinalInstructionKind::MemoryCopy,
        "memory.fill" if rest.is_empty() => FinalInstructionKind::MemoryFill,
        "memory.size" if rest.is_empty() => FinalInstructionKind::MemorySize,
        "memory.grow" if rest.is_empty() => FinalInstructionKind::MemoryGrow,
        "i8x16.shuffle" if rest.len() == 16 => {
            let values = rest
                .iter()
                .map(|value| value.parse::<u8>().ok().filter(|value| *value < 32))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(fail)?;
            let immediate: [u8; 16] = values.try_into().map_err(|_| fail())?;
            FinalInstructionKind::Shuffle(immediate)
        }
        _ if LaneOpcode::parse(opcode).is_some() && rest.len() == 1 => {
            let lane = rest[0].parse::<u8>().map_err(|_| fail())?;
            FinalInstructionKind::Lane {
                opcode: LaneOpcode::parse(opcode).expect("checked above"),
                lane,
            }
        }
        _ if MemoryOpcode::parse(opcode).is_some() => {
            let (memory_opcode, is_store) = MemoryOpcode::parse(opcode).expect("checked above");
            let mut offset = None;
            let mut align = None;
            for immediate in rest {
                if let Some(value) = immediate.strip_prefix("offset=") {
                    offset = Some(parse_u64(value).ok_or_else(fail)?);
                } else if let Some(value) = immediate.strip_prefix("align=") {
                    let bytes = parse_u32(value).ok_or_else(fail)?;
                    if !bytes.is_power_of_two() {
                        return Err(fail());
                    }
                    align = Some(bytes.trailing_zeros());
                } else {
                    return Err(fail());
                }
            }
            let offset = offset.ok_or_else(fail)?;
            let align = align.ok_or_else(fail)?;
            if is_store {
                FinalInstructionKind::Store {
                    opcode: memory_opcode,
                    offset,
                    align,
                }
            } else {
                FinalInstructionKind::Load {
                    opcode: memory_opcode,
                    offset,
                    align,
                }
            }
        }
        _ => FinalInstructionKind::Simple(
            SimpleOpcode::parse(opcode)
                .filter(|_| rest.is_empty())
                .ok_or_else(fail)?,
        ),
    };
    Ok(FinalInstruction { indent, kind })
}

fn parse_control_header(line: &str) -> Result<(Option<String>, Vec<WasmValueType>), String> {
    let mut pieces = line.split_whitespace();
    let _kind = pieces.next();
    let mut label = None;
    let mut results = Vec::new();
    for piece in pieces {
        if let Some(name) = piece.strip_prefix('$') {
            if label.is_some() || !valid_identifier(name) {
                return Err(format!("invalid generated control label: {line}"));
            }
            label = Some(name.to_string());
        } else if piece == "(result" {
            // `result` can be followed by one or more types; the closing `)` is
            // attached to the last type in the emitter's flat-line grammar.
            continue;
        } else {
            let value = piece.trim_end_matches(')');
            results.push(
                WasmValueType::parse(value)
                    .ok_or_else(|| format!("unsupported generated block result type in {line}"))?,
            );
        }
    }
    if line.contains("(result") && !line.ends_with(')') {
        return Err(format!("invalid generated control result: {line}"));
    }
    Ok((label, results))
}

fn parse_v128_const(words: &[&str]) -> Option<Vec<u8>> {
    let (&lane_type, values) = words.split_first()?;
    let width = match lane_type {
        "i8x16" => 1,
        "i16x8" => 2,
        "i32x4" | "f32x4" => 4,
        "i64x2" | "f64x2" => 8,
        _ => return None,
    };
    if values.len() != 16 / width {
        return None;
    }
    let mut bytes = Vec::with_capacity(16);
    for value in values {
        match lane_type {
            "f32x4" => bytes.extend(parse_f32_bits(value)?.to_le_bytes()),
            "f64x2" => bytes.extend(parse_f64_bits(value)?.to_le_bytes()),
            _ => {
                let bits = (width * 8) as u32;
                let number = parse_integer_bits(value, bits)?;
                bytes.extend_from_slice(&number.to_le_bytes()[..width]);
            }
        }
    }
    Some(bytes)
}

fn parse_i32(text: &str) -> Option<i32> {
    Some(parse_integer_bits(text, 32)? as u32 as i32)
}

fn parse_i64(text: &str) -> Option<i64> {
    Some(parse_integer_bits(text, 64)? as i64)
}

fn parse_integer_bits(text: &str, width: u32) -> Option<u64> {
    let (negative, digits) = if let Some(value) = text.strip_prefix('-') {
        (true, value)
    } else {
        (false, text)
    };
    let magnitude = if let Some(hex) = digits.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        digits.parse::<u64>().ok()?
    };
    if width == 64 {
        return if negative {
            if magnitude > (1u64 << 63) {
                None
            } else {
                Some(0u64.wrapping_sub(magnitude))
            }
        } else {
            Some(magnitude)
        };
    }
    let modulus = 1u64.checked_shl(width)?;
    if negative {
        let max_negative = 1u64 << (width - 1);
        if magnitude > max_negative {
            return None;
        }
        Some((modulus - magnitude) & (modulus - 1))
    } else if magnitude < modulus {
        Some(magnitude)
    } else {
        None
    }
}

fn parse_u32(text: &str) -> Option<u32> {
    parse_u64(text)?.try_into().ok()
}
fn parse_u64(text: &str) -> Option<u64> {
    if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

fn parse_f32_bits(text: &str) -> Option<u32> {
    if let Some((negative, payload)) = parse_nan_payload(text) {
        if payload == 0 || payload >= (1 << 23) {
            return None;
        }
        return Some((u32::from(negative) << 31) | (0xff << 23) | payload as u32);
    }
    let value = match text {
        "inf" | "+inf" => f32::INFINITY,
        "-inf" => f32::NEG_INFINITY,
        "nan" | "+nan" | "nan:canonical" | "nan:arithmetic" => f32::from_bits(0x7fc0_0000),
        "-nan" | "-nan:canonical" | "-nan:arithmetic" => f32::from_bits(0xffc0_0000),
        _ => text.parse::<f32>().ok()?,
    };
    Some(value.to_bits())
}

fn parse_f64_bits(text: &str) -> Option<u64> {
    if let Some((negative, payload)) = parse_nan_payload(text) {
        if payload == 0 || payload >= (1u64 << 52) {
            return None;
        }
        return Some((u64::from(negative) << 63) | (0x7ff << 52) | payload);
    }
    Some(parse_float(text)?.to_bits())
}

fn parse_nan_payload(text: &str) -> Option<(bool, u64)> {
    let (negative, spelling) = if let Some(value) = text.strip_prefix('-') {
        (true, value)
    } else {
        (false, text)
    };
    Some((
        negative,
        u64::from_str_radix(spelling.strip_prefix("nan:0x")?, 16).ok()?,
    ))
}
fn parse_float(text: &str) -> Option<f64> {
    match text {
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "nan:canonical" | "nan:arithmetic" => {
            Some(f64::from_bits(0x7ff8_0000_0000_0000))
        }
        "-nan" | "-nan:canonical" | "-nan:arithmetic" => {
            Some(f64::from_bits(0xfff8_0000_0000_0000))
        }
        _ => text.parse().ok(),
    }
}

fn format_f64(bits: u64) -> String {
    let negative = bits >> 63 != 0;
    let exponent = (bits >> 52) & 0x7ff;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0x7ff && fraction != 0 {
        format!("{}nan:0x{fraction:013x}", if negative { "-" } else { "" })
    } else if exponent == 0x7ff {
        format!("{}inf", if negative { "-" } else { "" })
    } else {
        f64::from_bits(bits).to_string()
    }
}

fn parse_target_custom(line: &str) -> Result<Vec<u8>, String> {
    let inner = line
        .strip_prefix("(@custom \"ck.wasm.target\" \"")
        .and_then(|text| text.strip_suffix("\")"))
        .ok_or_else(|| format!("unsupported generated custom section: {line}"))?;
    let bytes = inner.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        let escape = bytes
            .get(index + 1..index + 3)
            .ok_or_else(|| format!("invalid custom section escape: {line}"))?;
        let byte = u8::from_str_radix(
            std::str::from_utf8(escape)
                .map_err(|_| format!("invalid custom section escape: {line}"))?,
            16,
        )
        .map_err(|_| format!("invalid custom section escape: {line}"))?;
        output.push(byte);
        index += 3;
    }
    Ok(output)
}

fn write_instruction(out: &mut String, instruction: &FinalInstructionKind) {
    match instruction {
        FinalInstructionKind::Simple(opcode) => out.push_str(opcode.wat()),
        FinalInstructionKind::I32Const(value) => out.push_str(&format!("i32.const {value}")),
        FinalInstructionKind::I64Const(value) => out.push_str(&format!("i64.const {value}")),
        FinalInstructionKind::F64Const(bits) => {
            out.push_str(&format!("f64.const {}", format_f64(*bits)))
        }
        FinalInstructionKind::V128Const(bytes) => {
            out.push_str("v128.const i8x16");
            for byte in bytes {
                let signed = i8::from_le_bytes([*byte]);
                out.push_str(&format!(" {signed}"));
            }
        }
        FinalInstructionKind::LocalGet(name) => out.push_str(&format!("local.get ${name}")),
        FinalInstructionKind::LocalSet(name) => out.push_str(&format!("local.set ${name}")),
        FinalInstructionKind::LocalTee(name) => out.push_str(&format!("local.tee ${name}")),
        FinalInstructionKind::Call(name) => out.push_str(&format!("call ${name}")),
        FinalInstructionKind::Block { label, results } => {
            write_control(out, "block", label.as_deref(), results)
        }
        FinalInstructionKind::Loop { label, results } => {
            write_control(out, "loop", label.as_deref(), results)
        }
        FinalInstructionKind::If { label, results } => {
            write_control(out, "if", label.as_deref(), results)
        }
        FinalInstructionKind::Else => out.push_str("else"),
        FinalInstructionKind::End => out.push_str("end"),
        FinalInstructionKind::Br(label) => out.push_str(&format!("br ${label}")),
        FinalInstructionKind::BrIf(label) => out.push_str(&format!("br_if ${label}")),
        FinalInstructionKind::BrTable { targets, default } => {
            out.push_str("br_table");
            for target in targets {
                out.push_str(&format!(" ${target}"));
            }
            out.push_str(&format!(" ${default}"));
        }
        FinalInstructionKind::Load {
            opcode,
            offset,
            align,
        }
        | FinalInstructionKind::Store {
            opcode,
            offset,
            align,
        } => {
            out.push_str(opcode.wat());
            out.push_str(&format!(
                " offset={offset} align={}",
                1u32.checked_shl(*align).unwrap_or(0)
            ));
        }
        FinalInstructionKind::MemorySize => out.push_str("memory.size"),
        FinalInstructionKind::MemoryGrow => out.push_str("memory.grow"),
        FinalInstructionKind::MemoryCopy => out.push_str("memory.copy"),
        FinalInstructionKind::MemoryFill => out.push_str("memory.fill"),
        FinalInstructionKind::Lane { opcode, lane } => {
            out.push_str(&format!("{} {lane}", opcode.wat()))
        }
        FinalInstructionKind::Shuffle(lanes) => {
            out.push_str("i8x16.shuffle");
            for lane in lanes {
                out.push_str(&format!(" {lane}"));
            }
        }
    }
}

fn write_control(out: &mut String, kind: &str, label: Option<&str>, results: &[WasmValueType]) {
    out.push_str(kind);
    if let Some(label) = label {
        out.push_str(&format!(" ${label}"));
    }
    if !results.is_empty() {
        out.push_str(" (result");
        for result in results {
            out.push_str(&format!(" {}", result.wat()));
        }
        out.push(')');
    }
}

pub(super) fn validate_module(module: &FinalWasmModule) -> Result<(), String> {
    if module.memory_minimum == 0 {
        return Err("generated WebAssembly memory must have at least one page".into());
    }
    let mut function_names = HashSet::new();
    for function in &module.functions {
        if !function_names.insert(function.name.as_str()) {
            return Err(format!(
                "duplicate generated function name: ${}",
                function.name
            ));
        }
        let mut locals = HashSet::new();
        for local in function.params.iter().chain(&function.locals) {
            if !locals.insert(local.name.as_str()) {
                return Err(format!(
                    "duplicate generated local name in ${}: ${}",
                    function.name, local.name
                ));
            }
        }
    }
    if let Some(metadata) = &module.target_metadata
        && metadata.is_empty()
    {
        return Err("ck.wasm.target metadata payload must not be empty".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATED: &str = r#"(module
  (memory (export "memory") 1)
  (global (export "__ck_heap_base") i32 (i32.const 0))
  (func $all (export "all")
    (param $input i32)
    (result i32)
    (local $vector v128)
    i32.const 4294967295
    i64.const 18446744073709551615
    f64.const -0.0
    f64.const nan:0x123
    v128.const i32x4 1 2 3 4294967295
    v128.load64_zero offset=0 align=8
    local.set $vector
    local.get $vector
    i32x4.extract_lane 2
    f64x2.convert_low_i32x4_s
    i64.div_s
    local.get $input
    memory.size
    memory.copy
    memory.fill
    i32x4.add
    v128.bitselect
    i8x16.shuffle 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15
    block $exit
    loop $again
    if (result i32)
    i32.const 0
    else
    i32.const 1
    end
    br_if $exit
    br $again
    br_table $again $exit
    end
    end
    return
  )
)
"#;

    #[test]
    fn builder_accepts_chunked_flat_emitter_lines_and_round_trips_to_wat() {
        let mut builder = FinalWasmBuilder::new();
        for chunk in GENERATED.as_bytes().chunks(11) {
            builder.push_str(std::str::from_utf8(chunk).expect("ASCII fixture"));
        }
        let mut module = builder.finish().expect("typed generated module");
        module
            .set_target_metadata(br#"{"schema":2,"target":"wasm32"}"#.to_vec())
            .expect("target payload");
        let wat = module.to_wat();
        assert!(wat.contains("memory.copy"));
        assert!(wat.contains("i32x4.extract_lane 2"));
        assert!(wat.contains("nan:0x0000000000123"));
        wat::parse_str(wat).expect("printed final IR is valid WAT");

        let function = &module.functions[0];
        assert!(matches!(
            function.body[0].kind,
            FinalInstructionKind::I32Const(-1)
        ));
        assert!(matches!(
            function.body[1].kind,
            FinalInstructionKind::I64Const(-1)
        ));
        assert!(matches!(
            function.body[2].kind,
            FinalInstructionKind::F64Const(0x8000_0000_0000_0000)
        ));
    }

    #[test]
    fn builder_fails_closed_on_unknown_instructions_and_duplicate_metadata() {
        let mut unsupported = FinalWasmBuilder::new();
        unsupported.push_str("(module\n  (memory (export \"memory\") 1)\n  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n  (func $f\n    i32.opcode_escape\n  )\n)\n");
        assert!(
            unsupported
                .finish()
                .unwrap_err()
                .contains("unsupported generated WebAssembly instruction")
        );

        let mut duplicate = FinalWasmModule::default();
        duplicate.set_target_metadata(vec![1]).unwrap();
        assert!(duplicate.set_target_metadata(vec![2]).is_err());
    }
}
