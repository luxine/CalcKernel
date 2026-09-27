use std::collections::{BTreeMap, BTreeSet};

use crate::{
    KirBlock, KirEdge, KirFunction, KirInstruction, KirModule, KirTerminator, KirValueType,
    MirFunction, MirPrimitiveTypeName, MirType, MirValue, ValueId, value_type,
};

use super::memory::WasmMemoryPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WasmPhysicalType {
    I32,
    I64,
    F64,
    I32Pair,
    V128,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum WasmSourceType<'a> {
    Mir(&'a MirType),
    Kir(&'a KirValueType),
}

#[derive(Debug)]
pub(super) struct WasmTypedValue<'a> {
    pub value: ValueId,
    pub source_type: WasmSourceType<'a>,
    /// Scalar ABI/leaf view. Vector KIR values deliberately have no MIR projection.
    pub operand: Option<MirValue>,
    pub physical: WasmPhysicalType,
}

#[derive(Debug)]
pub(super) struct WasmLoweredModule<'a> {
    pub source: &'a KirModule,
    pub functions: Vec<WasmLoweredFunction<'a>>,
}

#[derive(Debug)]
pub(super) struct WasmLoweredFunction<'a> {
    pub source: &'a KirFunction,
    pub values: BTreeMap<ValueId, WasmTypedValue<'a>>,
    pub vector_values: BTreeSet<ValueId>,
    pub memory_plan: WasmMemoryPlan,
    pub blocks: Vec<WasmLoweredBlock<'a>>,
    /// Value/signature view for existing sinks; edge copies here are only for temp discovery.
    /// Structured emission must use the typed source blocks and the selected edge's copies.
    pub local_view: MirFunction,
}

impl WasmLoweredFunction<'_> {
    pub(super) fn source_metadata_is_consistent(&self) -> bool {
        if self.values.iter().any(|(id, value)| {
            *id != value.value
                || !source_operand_matches(value.source_type, value.operand.as_ref())
                || physical_type(value.source_type) != Some(value.physical)
        }) {
            return false;
        }
        let vectors = self
            .values
            .iter()
            .filter_map(|(id, value)| (value.physical == WasmPhysicalType::V128).then_some(*id))
            .collect::<BTreeSet<_>>();
        if vectors != self.vector_values {
            return false;
        }
        if self.blocks.len() != self.source.blocks.len() {
            return false;
        }
        self.blocks
            .iter()
            .zip(&self.source.blocks)
            .all(|(lowered, source)| {
                std::ptr::eq(lowered.source, source)
                    && lowered.instructions.len() == source.instructions.len()
                    && lowered
                        .instructions
                        .iter()
                        .zip(&source.instructions)
                        .all(|(instruction, original)| std::ptr::eq(instruction.source, original))
                    && edge_refs(&source.terminator)
                        .iter()
                        .zip(&lowered.edges)
                        .all(|((arm, original), edge)| {
                            edge.arm == *arm
                                && std::ptr::eq(edge.source, *original)
                                && edge.source.memory_args == original.memory_args
                        })
                    && edge_refs(&source.terminator).len() == lowered.edges.len()
            })
    }
}

fn source_operand_matches(source: WasmSourceType<'_>, operand: Option<&MirValue>) -> bool {
    match (source, operand) {
        (WasmSourceType::Mir(expected), Some(operand))
        | (WasmSourceType::Kir(KirValueType::Scalar(expected)), Some(operand)) => {
            value_type(operand) == expected
        }
        (WasmSourceType::Kir(KirValueType::FixedVector { .. }), None) => true,
        (WasmSourceType::Kir(KirValueType::Mask { .. }), None) => true,
        _ => false,
    }
}

fn physical_type(source: WasmSourceType<'_>) -> Option<WasmPhysicalType> {
    let type_node = match source {
        WasmSourceType::Mir(type_node) => type_node,
        WasmSourceType::Kir(KirValueType::Scalar(type_node)) => type_node,
        WasmSourceType::Kir(KirValueType::FixedVector { .. }) => {
            return Some(WasmPhysicalType::V128);
        }
        WasmSourceType::Kir(KirValueType::Mask { .. }) => {
            return Some(WasmPhysicalType::V128);
        }
    };
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => Some(WasmPhysicalType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            Some(WasmPhysicalType::I64)
        }
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(WasmPhysicalType::F64),
        MirType::Slice(_) => Some(WasmPhysicalType::I32Pair),
        MirType::Struct(_) | MirType::Void => None,
    }
}

fn edge_refs(terminator: &KirTerminator) -> Vec<(u8, &KirEdge)> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![(0, edge)],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![(0, then_edge), (1, else_edge)],
    }
}

#[derive(Debug)]
pub(super) struct WasmLoweredBlock<'a> {
    pub source: &'a KirBlock,
    pub instructions: Vec<WasmLoweredInstruction<'a>>,
    /// Edges preserve source order: jump = arm 0; branch = then arm 0, else arm 1.
    pub edges: Vec<WasmLoweredEdge<'a>>,
}

#[derive(Debug)]
pub(super) struct WasmLoweredInstruction<'a> {
    pub source: &'a KirInstruction,
    pub leaves: Vec<crate::MirInstruction>,
}

#[derive(Debug)]
pub(super) struct WasmLoweredEdge<'a> {
    pub arm: u8,
    pub source: &'a KirEdge,
    pub copies: Vec<crate::MirInstruction>,
}
