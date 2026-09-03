use std::collections::BTreeSet;

use num_bigint::BigInt;

use crate::{
    ContractFactPointer, ContractFactPredicate, FactArena, FactPredicate, InstructionId,
    KirFunction, KirInstructionKind, KirPlace, MemoryRegionId, MirBinaryOp, MirPrimitiveTypeName,
    MirType, ValueId,
};

use super::{CanonicalLoopDescriptor, SymbolicByteInterval, analyze_regions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMemoryAccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffineMemoryAccess {
    pub instruction: InstructionId,
    pub kind: LoopMemoryAccessKind,
    pub source_region: MemoryRegionId,
    pub region: MemoryRegionId,
    pub base: ValueId,
    pub slice_base: bool,
    pub induction: ValueId,
    pub trip_start: BigInt,
    pub trip_bound: ValueId,
    pub coefficient: BigInt,
    pub invariant_offset: Option<ValueId>,
    pub bias: BigInt,
    pub element_type: MirType,
    pub element_bytes: u32,
    pub base_alignment: u32,
    pub known_alignment: u32,
    pub slice_interval: Option<SymbolicByteInterval>,
    pub unit_stride: bool,
    pub vector_group_eligible: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopAccessAnalysis {
    pub accesses: Vec<AffineMemoryAccess>,
    pub rejected_instructions: Vec<InstructionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AffineForm {
    coefficient: BigInt,
    invariant: Option<ValueId>,
    bias: BigInt,
}

pub fn analyze_affine_loop_accesses(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    facts: Option<&FactArena>,
) -> Result<LoopAccessAnalysis, String> {
    if descriptor.function != function.id
        || !descriptor.lcssa
        || descriptor.preheader.is_none()
        || descriptor.latch.is_none()
    {
        return Err("affine access analysis requires a fresh canonical loop".to_string());
    }
    let induction = descriptor
        .induction
        .as_ref()
        .ok_or_else(|| "affine access analysis requires a canonical induction".to_string())?;
    let regions = analyze_regions(function, facts).map_err(|error| error.message)?;
    let loop_blocks = descriptor.blocks.iter().copied().collect::<BTreeSet<_>>();
    let mut accesses = Vec::new();
    let mut rejected = Vec::new();
    for instruction in function
        .blocks
        .iter()
        .filter(|block| loop_blocks.contains(&block.id))
        .flat_map(|block| &block.instructions)
    {
        let (kind, place) = match &instruction.kind {
            KirInstructionKind::Load { place } => (LoopMemoryAccessKind::Read, place.as_ref()),
            KirInstructionKind::Store { place, .. } => {
                (LoopMemoryAccessKind::Write, place.as_ref())
            }
            _ => continue,
        };
        let Some((base, index, element_type, source_region, slice_base)) = indexed_place(place)
        else {
            rejected.push(instruction.id);
            continue;
        };
        let mut visiting = BTreeSet::new();
        let Some(form) = affine_form(function, descriptor, induction.value, index, &mut visiting)
        else {
            rejected.push(instruction.id);
            continue;
        };
        let Some(element_bytes) = primitive_bytes(&element_type) else {
            rejected.push(instruction.id);
            continue;
        };
        let region_descriptor = regions.descriptor(source_region);
        let region = region_descriptor.map_or(source_region, |descriptor| descriptor.root);
        let base_alignment = fact_alignment(function, facts, base)
            .unwrap_or(element_bytes)
            .max(element_bytes);
        let known_alignment = access_alignment(
            base_alignment,
            element_bytes,
            &induction.start,
            &form.coefficient,
            form.invariant,
            &form.bias,
        );
        let unit_stride = induction.step == BigInt::from(1) && form.coefficient == BigInt::from(1);
        accesses.push(AffineMemoryAccess {
            instruction: instruction.id,
            kind,
            source_region,
            region,
            base,
            slice_base,
            induction: induction.value,
            trip_start: induction.start.clone(),
            trip_bound: induction.bound,
            coefficient: form.coefficient,
            invariant_offset: form.invariant,
            bias: form.bias,
            element_type,
            element_bytes,
            base_alignment,
            known_alignment,
            slice_interval: region_descriptor
                .and_then(|descriptor| descriptor.byte_interval.clone()),
            unit_stride,
            vector_group_eligible: unit_stride,
        });
    }
    accesses.sort_by_key(|access| access.instruction);
    rejected.sort_unstable();
    Ok(LoopAccessAnalysis {
        accesses,
        rejected_instructions: rejected,
    })
}

fn indexed_place(place: &KirPlace) -> Option<(ValueId, ValueId, MirType, MemoryRegionId, bool)> {
    match place {
        KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            region,
        } => Some((*slice, *index, type_node.clone(), *region, true)),
        KirPlace::Index {
            base,
            index,
            type_node,
            region,
        } => root_value(base).map(|base| (base, *index, type_node.clone(), *region, false)),
        _ => None,
    }
}

fn root_value(place: &KirPlace) -> Option<ValueId> {
    match place {
        KirPlace::Value { value, .. } => Some(*value),
        KirPlace::Deref { pointer, .. } => Some(*pointer),
        KirPlace::Index { base, .. } | KirPlace::Field { base, .. } => root_value(base),
        KirPlace::SliceIndex { slice, .. } => Some(*slice),
    }
}

fn affine_form(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    induction: ValueId,
    value: ValueId,
    visiting: &mut BTreeSet<ValueId>,
) -> Option<AffineForm> {
    if value == induction {
        return Some(AffineForm {
            coefficient: BigInt::from(1),
            invariant: None,
            bias: BigInt::from(0),
        });
    }
    if !visiting.insert(value) {
        return None;
    }
    let result = (|| {
        if let Some(constant) = integer_constant(function, value) {
            Some(AffineForm {
                coefficient: BigInt::from(0),
                invariant: None,
                bias: constant,
            })
        } else if defining_block(function, value)
            .is_some_and(|block| descriptor.blocks.binary_search(&block).is_err())
        {
            invariant_value(function, descriptor, value)
        } else if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            if block == descriptor.header {
                let incoming = incoming_values_with_sources(function, block, index);
                let entry = incoming
                    .iter()
                    .find(|(source, _)| descriptor.blocks.binary_search(source).is_err())
                    .map(|(_, value)| *value)?;
                incoming
                    .iter()
                    .all(|(_, value)| forwarded_from(function, *value, entry, &mut BTreeSet::new()))
                    .then(|| affine_form(function, descriptor, induction, entry, visiting))
                    .flatten()
            } else {
                let incoming = incoming_values(function, block, index);
                let mut forms = incoming
                    .into_iter()
                    .map(|value| affine_form(function, descriptor, induction, value, visiting))
                    .collect::<Option<Vec<_>>>()?;
                let first = forms.pop()?;
                forms.into_iter().all(|form| form == first).then_some(first)
            }
        } else if let Some(instruction) = defining_instruction(function, value) {
            match &instruction.kind {
                KirInstructionKind::Copy { value } => {
                    affine_form(function, descriptor, induction, *value, visiting)
                }
                KirInstructionKind::Binary {
                    op, left, right, ..
                } => {
                    let left = affine_form(function, descriptor, induction, *left, visiting)?;
                    let right = affine_form(function, descriptor, induction, *right, visiting)?;
                    combine_affine(*op, left, right)
                }
                _ => invariant_value(function, descriptor, value),
            }
        } else {
            invariant_value(function, descriptor, value)
        }
    })();
    visiting.remove(&value);
    result
}

fn defining_block(function: &KirFunction, value: ValueId) -> Option<crate::BlockId> {
    function.blocks.iter().find_map(|block| {
        (block.params.iter().any(|param| param.value == value)
            || block.instructions.iter().any(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            }))
        .then_some(block.id)
    })
}

fn invariant_value(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
    value: ValueId,
) -> Option<AffineForm> {
    let owner = function.blocks.iter().find_map(|block| {
        (block.params.iter().any(|param| param.value == value)
            || block.instructions.iter().any(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            }))
        .then_some(block.id)
    });
    if owner.is_some_and(|block| descriptor.blocks.binary_search(&block).is_ok()) {
        None
    } else {
        Some(AffineForm {
            coefficient: BigInt::from(0),
            invariant: Some(value),
            bias: BigInt::from(0),
        })
    }
}

fn combine_affine(op: MirBinaryOp, left: AffineForm, right: AffineForm) -> Option<AffineForm> {
    match op {
        MirBinaryOp::Add | MirBinaryOp::Sub => {
            let sign = if op == MirBinaryOp::Add {
                BigInt::from(1)
            } else {
                BigInt::from(-1)
            };
            let invariant = match (left.invariant, right.invariant, op) {
                (None, invariant, MirBinaryOp::Add) => invariant,
                (invariant, None, _) => invariant,
                _ => return None,
            };
            Some(AffineForm {
                coefficient: left.coefficient + sign.clone() * right.coefficient,
                invariant,
                bias: left.bias + sign * right.bias,
            })
        }
        MirBinaryOp::Mul => {
            if left.coefficient == BigInt::from(0) && left.invariant.is_none() {
                Some(AffineForm {
                    coefficient: left.bias.clone() * right.coefficient,
                    invariant: right.invariant,
                    bias: left.bias * right.bias,
                })
            } else if right.coefficient == BigInt::from(0) && right.invariant.is_none() {
                Some(AffineForm {
                    coefficient: right.bias.clone() * left.coefficient,
                    invariant: left.invariant,
                    bias: right.bias * left.bias,
                })
            } else {
                None
            }
        }
        MirBinaryOp::Div | MirBinaryOp::Mod => None,
    }
}

fn incoming_values(function: &KirFunction, target: crate::BlockId, index: usize) -> Vec<ValueId> {
    incoming_values_with_sources(function, target, index)
        .into_iter()
        .map(|(_, value)| value)
        .collect()
}

fn incoming_values_with_sources(
    function: &KirFunction,
    target: crate::BlockId,
    index: usize,
) -> Vec<(crate::BlockId, ValueId)> {
    function
        .blocks
        .iter()
        .flat_map(|block| {
            let edges = match &block.terminator {
                crate::KirTerminator::Return { .. } => Vec::new(),
                crate::KirTerminator::Jump { edge } => vec![edge],
                crate::KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
            };
            edges.into_iter().map(move |edge| (block.id, edge))
        })
        .filter(|(_, edge)| edge.target == target)
        .filter_map(|(source, edge)| edge.args.get(index).copied().map(|value| (source, value)))
        .collect()
}

fn forwarded_from(
    function: &KirFunction,
    value: ValueId,
    origin: ValueId,
    visiting: &mut BTreeSet<ValueId>,
) -> bool {
    if value == origin {
        return true;
    }
    if !visiting.insert(value) {
        return true;
    }
    let result = if let Some((block, index)) = function.blocks.iter().find_map(|block| {
        block
            .params
            .iter()
            .position(|param| param.value == value)
            .map(|index| (block.id, index))
    }) {
        let incoming = incoming_values(function, block, index);
        !incoming.is_empty()
            && incoming
                .into_iter()
                .all(|value| forwarded_from(function, value, origin, visiting))
    } else {
        matches!(
            defining_instruction(function, value),
            Some(crate::KirInstruction {
                kind: KirInstructionKind::Copy { value },
                ..
            }) if forwarded_from(function, *value, origin, visiting)
        )
    };
    visiting.remove(&value);
    result
}

fn defining_instruction(function: &KirFunction, value: ValueId) -> Option<&crate::KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
}

fn integer_constant(function: &KirFunction, value: ValueId) -> Option<BigInt> {
    let instruction = defining_instruction(function, value)?;
    let KirInstructionKind::ConstInt { value } = &instruction.kind else {
        return None;
    };
    value.parse().ok()
}

fn primitive_bytes(type_node: &MirType) -> Option<u32> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => Some(8),
        MirType::Primitive(MirPrimitiveTypeName::Bool) => Some(1),
        MirType::Pointer(_) | MirType::Slice(_) | MirType::Struct(_) | MirType::Void => None,
    }
}

fn access_alignment(
    base_alignment: u32,
    element_bytes: u32,
    start: &BigInt,
    coefficient: &BigInt,
    invariant: Option<ValueId>,
    bias: &BigInt,
) -> u32 {
    if invariant.is_some() {
        return element_bytes;
    }
    let first = (start * coefficient + bias) * BigInt::from(element_bytes);
    let stride = coefficient * BigInt::from(element_bytes);
    gcd_u32(
        gcd_u32(base_alignment, bigint_alignment_component(&first)),
        bigint_alignment_component(&stride),
    )
    .max(1)
}

fn bigint_alignment_component(value: &BigInt) -> u32 {
    if value == &BigInt::from(0) {
        return 0;
    }
    value
        .to_string()
        .trim_start_matches('-')
        .parse::<u64>()
        .ok()
        .map_or(1, |value| u32::try_from(value).unwrap_or(1))
}

const fn gcd_u32(mut left: u32, mut right: u32) -> u32 {
    if left == 0 {
        return right;
    }
    if right == 0 {
        return left;
    }
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn fact_alignment(function: &KirFunction, facts: Option<&FactArena>, base: ValueId) -> Option<u32> {
    facts?
        .facts()
        .iter()
        .filter_map(|fact| {
            let FactPredicate::Contract(ContractFactPredicate::Aligned { pointer, alignment }) =
                &fact.predicate
            else {
                return None;
            };
            matches!(
                pointer,
                ContractFactPointer::Value(value) | ContractFactPointer::SliceData(value)
                    if forwarded_from(function, base, *value, &mut BTreeSet::new())
            )
            .then_some(*alignment)
        })
        .max()
}
