//! Preheader byte bases for modular slice addresses in closed scalar loops.
use crate::{
    BlockId, InstructionId, KirArithmeticSemantics, KirFunction, KirInstruction,
    KirInstructionKind, KirPlace, KirTerminator, MirBinaryOp, MirPrimitiveTypeName, MirType,
    ValueId,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactoredAddressBase {
    pub id: u32,
    pub header: BlockId,
    pub preheader: BlockId,
    pub slice: ValueId,
    pub offset: ValueId,
    pub element_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactoredAddressAccess {
    pub base: u32,
    /// The original per-access IV operand, before byte scaling.
    pub index: ValueId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FactoredAddressPlan {
    pub bundles: super::factored_bundle::BundleAddressPlan,
    pub bases: Vec<FactoredAddressBase>,
    pub accesses: BTreeMap<InstructionId, FactoredAddressAccess>,
}

pub(crate) fn checked_factored_addresses(function: &KirFunction) -> FactoredAddressPlan {
    let mut proposed = propose(function);
    proposed.bundles = super::factored_bundle::propose(function);
    if super::factored_check::validate(function, &proposed) {
        proposed
    } else {
        FactoredAddressPlan::default()
    }
}

fn propose(function: &KirFunction) -> FactoredAddressPlan {
    if function.blocks.len() > 256 {
        return FactoredAddressPlan::default();
    }
    let mut plan = FactoredAddressPlan::default();
    let dominators = crate::compute_kir_dominators(function);
    for descriptor in crate::analyze_canonical_loops(function).loops {
        if !descriptor.innermost
            || !descriptor.lcssa
            || !descriptor.dedicated_exits
            || !(3..=9).contains(&descriptor.blocks.len())
            || descriptor.exits.len() != 1
        {
            continue;
        }
        let (Some(preheader), Some(latch), Some(induction)) = (
            descriptor.preheader,
            descriptor.latch,
            descriptor.induction.as_ref(),
        ) else {
            continue;
        };
        if induction.start != 0.into()
            || induction.step != 1.into()
            || induction.comparison != crate::MirCompareOp::Lt
            || value_type(function, induction.value)
                != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        {
            continue;
        }
        let members = descriptor.blocks.iter().copied().collect::<BTreeSet<_>>();
        let Some(entry) = function.blocks.iter().find(|block| block.id == preheader) else {
            continue;
        };
        if !matches!(&entry.terminator, KirTerminator::Jump { edge } if edge.target == descriptor.header)
        {
            continue;
        }
        if descriptor.blocks.iter().any(|id| {
            let Some(block) = function.blocks.iter().find(|block| block.id == *id) else { return true; };
            outgoing(&block.terminator).iter().any(|edge| !members.contains(&edge.target) && (*id != descriptor.header || edge.target != descriptor.exits[0]))
                || (*id == latch && !matches!(&block.terminator, KirTerminator::Jump { edge } if edge.target == descriptor.header))
        }) { continue; }
        for block in function
            .blocks
            .iter()
            .filter(|block| members.contains(&block.id))
        {
            for instruction in &block.instructions {
                let Some((slice, index, width)) = access(instruction) else {
                    continue;
                };
                if value_type(function, index)
                    != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
                {
                    continue;
                }
                let Some(index) = strip_copies(function, index) else {
                    continue;
                };
                let Some((_, definition)) = definition(function, index) else {
                    continue;
                };
                let KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left,
                    right,
                    semantics: KirArithmeticSemantics::Modular,
                } = definition.kind
                else {
                    continue;
                };
                let (iv, offset) = if alias_root(function, left, |value, _| {
                    (value == induction.value).then_some(value)
                }) == Some(induction.value)
                {
                    (left, right)
                } else if alias_root(function, right, |value, _| {
                    (value == induction.value).then_some(value)
                }) == Some(induction.value)
                {
                    (right, left)
                } else {
                    continue;
                };
                let stable = |value: ValueId, owner: Option<BlockId>| {
                    (owner.is_none()
                        || owner.is_some_and(|id| {
                            !members.contains(&id) && dominators.dominates(id, preheader)
                        }))
                    .then_some(value)
                };
                let Some(offset) = alias_root(function, offset, stable) else {
                    continue;
                };
                if value_type(function, offset)
                    != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
                {
                    continue;
                }
                let Some(slice) = alias_root(function, slice, stable) else {
                    continue;
                };
                let id = if let Some(base) = plan.bases.iter().find(|base| {
                    base.header == descriptor.header
                        && base.slice == slice
                        && base.offset == offset
                        && base.element_bytes == width
                }) {
                    base.id
                } else {
                    if plan.bases.len() >= 16 {
                        return FactoredAddressPlan::default();
                    }
                    let id = plan.bases.len() as u32;
                    plan.bases.push(FactoredAddressBase {
                        id,
                        header: descriptor.header,
                        preheader,
                        slice,
                        offset,
                        element_bytes: width,
                    });
                    id
                };
                if plan
                    .accesses
                    .insert(
                        instruction.id,
                        FactoredAddressAccess {
                            base: id,
                            index: iv,
                        },
                    )
                    .is_some()
                {
                    return FactoredAddressPlan::default();
                }
                if plan.accesses.len() > 64 {
                    return FactoredAddressPlan::default();
                }
            }
        }
    }
    plan
}

fn alias_root(
    function: &KirFunction,
    start: ValueId,
    terminal: impl Fn(ValueId, Option<BlockId>) -> Option<ValueId>,
) -> Option<ValueId> {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    let mut roots = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if !visited.insert(value) {
            continue;
        }
        if visited.len() > 2048 {
            return None;
        }
        let parameter = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|p| p.value == value)
                .map(|i| (block.id, i))
        });
        let owner = parameter
            .map(|(id, _)| id)
            .or_else(|| definition(function, value).map(|(id, _)| id));
        if function.params.iter().any(|p| p.value == value) {
            roots.insert(terminal(value, None)?);
            continue;
        }
        if let Some(root) = terminal(value, owner) {
            roots.insert(root);
            continue;
        }
        if let Some((target, index)) = parameter {
            let mut count = 0;
            for block in &function.blocks {
                for edge in outgoing(&block.terminator) {
                    if edge.target == target {
                        pending.push(*edge.args.get(index)?);
                        count += 1;
                    }
                }
            }
            if count == 0 {
                return None;
            }
        } else if let Some((_, instruction)) = definition(function, value) {
            if let KirInstructionKind::Copy { value } = instruction.kind {
                pending.push(value);
            } else {
                return None;
            }
        } else {
            return None;
        }
    }
    (roots.len() == 1).then(|| *roots.first().expect("one root"))
}

fn strip_copies(function: &KirFunction, mut value: ValueId) -> Option<ValueId> {
    for _ in 0..16 {
        match definition(function, value)?.1.kind {
            KirInstructionKind::Copy { value: next } => value = next,
            _ => return Some(value),
        }
    }
    None
}

fn access(instruction: &KirInstruction) -> Option<(ValueId, ValueId, u32)> {
    let place = match &instruction.kind {
        KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
            place.as_ref()
        }
        _ => return None,
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node,
        ..
    } = place
    else {
        return None;
    };
    let width = match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => 4,
        MirType::Primitive(
            MirPrimitiveTypeName::F64 | MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64,
        ) => 8,
        _ => return None,
    };
    Some((*slice, *index, width))
}

fn value_type(function: &KirFunction, value: ValueId) -> Option<&MirType> {
    function
        .params
        .iter()
        .find(|p| p.value == value)
        .map(|p| &p.type_node)
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|b| &b.params)
                .find(|p| p.value == value)
                .and_then(|p| p.type_node.as_scalar())
        })
        .or_else(|| {
            definition(function, value).and_then(|(_, i)| {
                i.results
                    .iter()
                    .find(|r| r.value == value)
                    .and_then(|r| r.type_node.as_scalar())
            })
        })
}

fn definition(function: &KirFunction, value: ValueId) -> Option<(BlockId, &KirInstruction)> {
    function.blocks.iter().find_map(|b| {
        b.instructions
            .iter()
            .find(|i| i.results.iter().any(|r| r.value == value))
            .map(|i| (b.id, i))
    })
}

fn outgoing(terminator: &KirTerminator) -> Vec<&crate::KirEdge> {
    match terminator {
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        KirTerminator::Return { .. } => Vec::new(),
    }
}
