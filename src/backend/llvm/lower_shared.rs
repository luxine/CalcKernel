use std::collections::HashMap;

use crate::{
    KirLaneType, KirValueType, MirBinaryOp, MirCompareOp, MirModule, MirPrimitiveTypeName,
    MirRuntimeIntrinsic, MirType, MirUnaryOp,
};

use super::{
    builder::NativeType,
    context::NativeContext,
    error::{NativeError, NativeStage},
    ffi::{BridgeBinaryOp, BridgeCompareOp, BridgeUnaryOp},
};

const LOWERING_ERROR: i32 = 3;

pub(super) struct TypeRegistry<'context> {
    pub(super) void: NativeType<'context>,
    pub(super) i1: NativeType<'context>,
    pub(super) i32: NativeType<'context>,
    pub(super) i64: NativeType<'context>,
    pub(super) f64: NativeType<'context>,
    pub(super) pointer: NativeType<'context>,
    pub(super) slice: NativeType<'context>,
    structs: HashMap<String, NativeType<'context>>,
}

impl<'context> TypeRegistry<'context> {
    pub(super) fn new(
        context: &'context NativeContext,
        module: &MirModule,
    ) -> Result<Self, NativeError> {
        let mut registry = Self {
            void: NativeType::void(context)?,
            i1: NativeType::int(context, 1)?,
            i32: NativeType::int(context, 32)?,
            i64: NativeType::int(context, 64)?,
            f64: NativeType::f64(context)?,
            pointer: NativeType::pointer(context)?,
            slice: NativeType::slice(context)?,
            structs: HashMap::new(),
        };
        for structure in &module.structs {
            registry.structs.insert(
                structure.name.clone(),
                NativeType::named_struct(context, &format!("struct.{}", structure.name))?,
            );
        }
        for structure in &module.structs {
            let fields = structure
                .fields
                .iter()
                .map(|field| registry.get(&field.type_node))
                .collect::<Result<Vec<_>, _>>()?;
            registry
                .structs
                .get(&structure.name)
                .copied()
                .ok_or_else(|| lowering_error("missing native struct declaration"))?
                .set_struct_body(&fields)?;
        }
        Ok(registry)
    }

    pub(super) fn get(&self, type_node: &MirType) -> Result<NativeType<'context>, NativeError> {
        match type_node {
            MirType::Void => Ok(self.void),
            MirType::Primitive(MirPrimitiveTypeName::Bool) => Ok(self.i1),
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
                Ok(self.i32)
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
                Ok(self.i64)
            }
            MirType::Primitive(MirPrimitiveTypeName::F64) => Ok(self.f64),
            MirType::Pointer(_) => Ok(self.pointer),
            MirType::Slice(_) => Ok(self.slice),
            MirType::Struct(name) => self
                .structs
                .get(name)
                .copied()
                .ok_or_else(|| lowering_error(format!("unknown MIR struct type '{name}'"))),
        }
    }

    pub(super) fn get_kir(
        &self,
        type_node: &KirValueType,
    ) -> Result<NativeType<'context>, NativeError> {
        match type_node {
            KirValueType::Scalar(type_node) => self.get(type_node),
            KirValueType::FixedVector { lane, lanes } => {
                NativeType::fixed_vector(self.lane(*lane), u32::from(*lanes))
            }
            KirValueType::Mask { lanes } => NativeType::fixed_vector(self.i1, u32::from(*lanes)),
        }
    }

    pub(super) const fn lane(&self, lane: KirLaneType) -> NativeType<'context> {
        match lane {
            KirLaneType::I32 | KirLaneType::U32 => self.i32,
            KirLaneType::I64 | KirLaneType::U64 => self.i64,
            KirLaneType::F64 => self.f64,
        }
    }
}

pub(super) fn runtime_signature(intrinsic: MirRuntimeIntrinsic) -> (&'static str, Option<MirType>) {
    let primitive = |name| Some(MirType::Primitive(name));
    match intrinsic {
        MirRuntimeIntrinsic::PrintI32 => ("__ck_print_i32", primitive(MirPrimitiveTypeName::I32)),
        MirRuntimeIntrinsic::PrintI64 => ("__ck_print_i64", primitive(MirPrimitiveTypeName::I64)),
        MirRuntimeIntrinsic::PrintU32 => ("__ck_print_u32", primitive(MirPrimitiveTypeName::U32)),
        MirRuntimeIntrinsic::PrintU64 => ("__ck_print_u64", primitive(MirPrimitiveTypeName::U64)),
        MirRuntimeIntrinsic::PrintF64 => ("__ck_print_f64", primitive(MirPrimitiveTypeName::F64)),
        MirRuntimeIntrinsic::PrintBool => {
            ("__ck_print_bool", primitive(MirPrimitiveTypeName::Bool))
        }
        MirRuntimeIntrinsic::PrintNewline => ("__ck_print_newline", None),
    }
}

pub(super) fn binary_op(
    op: MirBinaryOp,
    type_node: &MirType,
) -> Result<BridgeBinaryOp, NativeError> {
    let float = matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64));
    let unsigned = matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    );
    match (op, float, unsigned) {
        (MirBinaryOp::Add, true, _) => Ok(BridgeBinaryOp::FAdd),
        (MirBinaryOp::Sub, true, _) => Ok(BridgeBinaryOp::FSub),
        (MirBinaryOp::Mul, true, _) => Ok(BridgeBinaryOp::FMul),
        (MirBinaryOp::Div, true, _) => Ok(BridgeBinaryOp::FDiv),
        (MirBinaryOp::Mod, true, _) => Err(lowering_error("f64 modulo is unsupported")),
        (MirBinaryOp::Add, false, _) => Ok(BridgeBinaryOp::Add),
        (MirBinaryOp::Sub, false, _) => Ok(BridgeBinaryOp::Sub),
        (MirBinaryOp::Mul, false, _) => Ok(BridgeBinaryOp::Mul),
        (MirBinaryOp::Div, false, true) => Ok(BridgeBinaryOp::UDiv),
        (MirBinaryOp::Div, false, false) => Ok(BridgeBinaryOp::SDiv),
        (MirBinaryOp::Mod, false, true) => Ok(BridgeBinaryOp::URem),
        (MirBinaryOp::Mod, false, false) => Ok(BridgeBinaryOp::SRem),
    }
}

pub(super) fn unary_op(op: MirUnaryOp, type_node: &MirType) -> BridgeUnaryOp {
    match (op, type_node) {
        (MirUnaryOp::Not, _) => BridgeUnaryOp::Not,
        (MirUnaryOp::Neg, MirType::Primitive(MirPrimitiveTypeName::F64)) => BridgeUnaryOp::FNeg,
        (MirUnaryOp::Neg, _) => BridgeUnaryOp::Neg,
    }
}

pub(super) fn compare_op(op: MirCompareOp, type_node: &MirType) -> BridgeCompareOp {
    let float = matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64));
    let unsigned = matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    );
    match (op, float, unsigned) {
        (MirCompareOp::Eq, true, _) => BridgeCompareOp::FcmpOeq,
        (MirCompareOp::Ne, true, _) => BridgeCompareOp::FcmpUne,
        (MirCompareOp::Lt, true, _) => BridgeCompareOp::FcmpOlt,
        (MirCompareOp::Le, true, _) => BridgeCompareOp::FcmpOle,
        (MirCompareOp::Gt, true, _) => BridgeCompareOp::FcmpOgt,
        (MirCompareOp::Ge, true, _) => BridgeCompareOp::FcmpOge,
        (MirCompareOp::Eq, false, _) => BridgeCompareOp::IcmpEq,
        (MirCompareOp::Ne, false, _) => BridgeCompareOp::IcmpNe,
        (MirCompareOp::Lt, false, true) => BridgeCompareOp::IcmpUlt,
        (MirCompareOp::Le, false, true) => BridgeCompareOp::IcmpUle,
        (MirCompareOp::Gt, false, true) => BridgeCompareOp::IcmpUgt,
        (MirCompareOp::Ge, false, true) => BridgeCompareOp::IcmpUge,
        (MirCompareOp::Lt, false, false) => BridgeCompareOp::IcmpSlt,
        (MirCompareOp::Le, false, false) => BridgeCompareOp::IcmpSle,
        (MirCompareOp::Gt, false, false) => BridgeCompareOp::IcmpSgt,
        (MirCompareOp::Ge, false, false) => BridgeCompareOp::IcmpSge,
    }
}

pub(super) fn lowering_error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Module, LOWERING_ERROR, message.into())
}
