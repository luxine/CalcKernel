//! Reuses complete Wasm32 byte addresses inside one straight-line SIMD block.
use crate::{
    BlockId, InstructionId, KirArithmeticSemantics, KirFunction, KirInstruction,
    KirInstructionKind, KirLaneType, KirVectorMemoryAccess, MirBinaryOp, MirPrimitiveTypeName,
    MirType, ValueId,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundleAddressBase {
    pub id: u32,
    pub block: BlockId,
    /// The first access computes and saves the original complete byte address.
    pub first: InstructionId,
    pub slice: ValueId,
    pub start: ValueId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BundleAddressAccess {
    pub base: u32,
    /// Added using i32.add, never a nonwrapping Wasm memarg offset.
    pub delta_bytes: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BundleAddressPlan {
    pub bases: Vec<BundleAddressBase>,
    pub accesses: BTreeMap<InstructionId, BundleAddressAccess>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Index {
    terms: BTreeMap<ValueId, u32>,
    constant: u32,
}
impl Index {
    fn add(mut self, other: Self) -> Self {
        for (value, coefficient) in other.terms {
            let slot = self.terms.entry(value).or_default();
            *slot = slot.wrapping_add(coefficient);
        }
        self.terms.retain(|_, coefficient| *coefficient != 0);
        self.constant = self.constant.wrapping_add(other.constant);
        self
    }
}

pub(super) fn propose(function: &KirFunction) -> BundleAddressPlan {
    let mut plan = BundleAddressPlan::default();
    if function.blocks.len() > 256 {
        return plan;
    }
    let types: BTreeMap<_, _> = function
        .params
        .iter()
        .map(|p| (p.value, Some(&p.type_node)))
        .chain(function.blocks.iter().flat_map(|b| {
            b.params
                .iter()
                .map(|p| (p.value, p.type_node.as_scalar()))
                .chain(
                    b.instructions
                        .iter()
                        .flat_map(|i| i.results.iter().map(|r| (r.value, r.type_node.as_scalar()))),
                )
        }))
        .collect();
    let mut remaining = 16_384_usize;
    for block in &function.blocks {
        if block.instructions.len() > 512 {
            continue;
        }
        let definitions: BTreeMap<_, _> = block
            .instructions
            .iter()
            .flat_map(|i| i.results.iter().map(move |r| (r.value, i)))
            .collect();
        type Group = Vec<(InstructionId, ValueId, u32)>;
        let mut groups = BTreeMap::<(ValueId, ValueId, BTreeMap<ValueId, u32>), Group>::new();
        for instruction in &block.instructions {
            let Some(access) = vector_access(instruction) else {
                continue;
            };
            if access.lane != KirLaneType::F64
                || access.lanes != 2
                || types.get(&access.slice).copied().flatten()
                    != Some(&MirType::Slice(Box::new(MirType::Primitive(
                        MirPrimitiveTypeName::F64,
                    ))))
            {
                continue;
            }
            let Some(index) = expand(access.start, &definitions, &types, &mut remaining, 0) else {
                continue;
            };
            if index.terms.len() != 2 {
                continue;
            }
            groups
                .entry((access.slice, access.end, index.terms))
                .or_default()
                .push((instruction.id, access.start, index.constant));
        }
        if remaining == 0 {
            return BundleAddressPlan::default();
        }
        for ((slice, _, _), accesses) in groups {
            let Some(&(first, start, origin)) = accesses.first() else {
                continue;
            };
            // The measured, bounded envelope is four consecutive f64x2 chunks.
            // No wider UF or unrelated map is admitted by this backend plan.
            let mut deltas: Vec<_> = accesses.iter().map(|a| a.2.wrapping_sub(origin)).collect();
            deltas.sort_unstable();
            deltas.dedup();
            if deltas != [0, 2, 4, 6] {
                continue;
            }
            if plan.bases.len() == 16 || plan.accesses.len() + accesses.len() > 128 {
                return BundleAddressPlan::default();
            }
            let id = plan.bases.len() as u32;
            plan.bases.push(BundleAddressBase {
                id,
                block: block.id,
                first,
                slice,
                start,
            });
            for (instruction, _, constant) in accesses {
                plan.accesses.insert(
                    instruction,
                    BundleAddressAccess {
                        base: id,
                        delta_bytes: constant.wrapping_sub(origin).wrapping_mul(8),
                    },
                );
            }
        }
    }
    plan
}

fn expand<'a>(
    value: ValueId,
    definitions: &BTreeMap<ValueId, &'a KirInstruction>,
    types: &BTreeMap<ValueId, Option<&'a MirType>>,
    remaining: &mut usize,
    depth: usize,
) -> Option<Index> {
    *remaining = remaining.checked_sub(1)?;
    if depth > 32
        || types.get(&value).copied().flatten()
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
    {
        return None;
    }
    if let Some(instruction) = definitions.get(&value)
        && instruction.effect.is_none()
        && instruction.memory.is_none()
    {
        match &instruction.kind {
            KirInstructionKind::ConstInt { value } => {
                return Some(Index {
                    constant: value.parse::<u32>().ok()?,
                    ..Index::default()
                });
            }
            KirInstructionKind::Copy { value } => {
                return expand(*value, definitions, types, remaining, depth + 1);
            }
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } => {
                return Some(
                    expand(*left, definitions, types, remaining, depth + 1)?.add(expand(
                        *right,
                        definitions,
                        types,
                        remaining,
                        depth + 1,
                    )?),
                );
            }
            _ => {}
        }
    }
    Some(Index {
        terms: BTreeMap::from([(value, 1)]),
        constant: 0,
    })
}
fn vector_access(instruction: &KirInstruction) -> Option<&KirVectorMemoryAccess> {
    match &instruction.kind {
        KirInstructionKind::VectorLoad { access, .. }
        | KirInstructionKind::VectorStore { access, .. } => Some(access),
        _ => None,
    }
}
