use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ContractFactSet, KirBlock, KirEdge, KirFunction, KirInstructionKind, KirMemoryRegionOrigin,
    KirModule, KirTerminator, ValueId,
};

use super::dce::instruction_uses;

pub(crate) fn run_cfg_canonicalize(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
) -> bool {
    let mut changed = false;
    let protected = super::phi_prune::protected_contract_values(contracts);
    for function in &mut module.functions {
        let constants = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter_map(|instruction| match instruction.kind {
                KirInstructionKind::ConstBool { value } => instruction
                    .results
                    .first()
                    .map(|result| (result.value, value)),
                _ => None,
            })
            .collect::<Vec<(ValueId, bool)>>();
        for block in &mut function.blocks {
            let replacement = match &block.terminator {
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => constants
                    .iter()
                    .find_map(|(value, constant)| (*value == *condition).then_some(*constant))
                    .map(|constant| KirTerminator::Jump {
                        edge: if constant {
                            then_edge.clone()
                        } else {
                            else_edge.clone()
                        },
                    }),
                _ => None,
            };
            if let Some(replacement) = replacement {
                block.terminator = replacement;
                changed = true;
            }
        }

        let Some(entry) = function.blocks.first().map(|block| block.id) else {
            continue;
        };
        let mut reachable = BTreeSet::from([entry]);
        loop {
            let before = reachable.len();
            for block in &function.blocks {
                if !reachable.contains(&block.id) {
                    continue;
                }
                match &block.terminator {
                    KirTerminator::Return { .. } => {}
                    KirTerminator::Jump { edge } => {
                        reachable.insert(edge.target);
                    }
                    KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => {
                        reachable.insert(then_edge.target);
                        reachable.insert(else_edge.target);
                    }
                }
            }
            if reachable.len() == before {
                break;
            }
        }
        let before = function.blocks.len();
        function
            .blocks
            .retain(|block| reachable.contains(&block.id));
        changed |= function.blocks.len() != before;

        while let Some(index) =
            function
                .blocks
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(index, block)| {
                    (is_forwarding_block(block)
                        && !has_nonlocal_parameter_uses(function, block, &protected))
                    .then_some(index)
                })
        {
            let KirTerminator::Jump { edge: outgoing } = function.blocks[index].terminator.clone()
            else {
                break;
            };
            let bridge = function.blocks.remove(index);
            for block in &mut function.blocks {
                match &mut block.terminator {
                    KirTerminator::Return { .. } => {}
                    KirTerminator::Jump { edge } => forward_edge(edge, &bridge, &outgoing),
                    KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => {
                        forward_edge(then_edge, &bridge, &outgoing);
                        forward_edge(else_edge, &bridge, &outgoing);
                    }
                }
            }
            changed = true;
        }
        for block in &mut function.blocks {
            if let KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &block.terminator
                && then_edge == else_edge
            {
                block.terminator = KirTerminator::Jump {
                    edge: then_edge.clone(),
                };
                changed = true;
            }
        }
        changed |= super::phi_prune::remove_dead_block_parameters(function, &protected);
    }
    changed
}

fn is_forwarding_block(block: &KirBlock) -> bool {
    block.instructions.is_empty()
        && matches!(&block.terminator, KirTerminator::Jump { edge } if edge.target != block.id)
}

fn has_nonlocal_parameter_uses(
    function: &KirFunction,
    bridge: &KirBlock,
    protected: &BTreeSet<ValueId>,
) -> bool {
    let values = bridge
        .params
        .iter()
        .map(|param| param.value)
        .collect::<BTreeSet<_>>();
    let memories = bridge
        .memory_params
        .iter()
        .map(|param| param.version)
        .collect::<BTreeSet<_>>();
    if !values.is_disjoint(protected) {
        return true;
    }
    // Region metadata and surviving contract bindings also name SSA definitions.
    // This pass composes edges only; it cannot silently remap those consumers.
    if function.regions.iter().any(|region| {
        let origin_used = match region.origin {
            KirMemoryRegionOrigin::Conservative => false,
            KirMemoryRegionOrigin::Parameter(value)
            | KirMemoryRegionOrigin::RawSlice(value)
            | KirMemoryRegionOrigin::Subslice(value) => values.contains(&value),
        };
        origin_used
            || region.byte_interval.as_ref().is_some_and(|interval| {
                values.contains(&interval.start) || values.contains(&interval.end)
            })
    }) {
        return true;
    }
    for block in function.blocks.iter().filter(|block| block.id != bridge.id) {
        if block.instructions.iter().any(|instruction| {
            instruction_uses(instruction)
                .iter()
                .any(|value| values.contains(value))
                || instruction
                    .memory
                    .as_ref()
                    .is_some_and(|memory| memories.contains(&memory.input))
        }) {
            return true;
        }
        let edge_uses = |edge: &KirEdge| {
            edge.args.iter().any(|value| values.contains(value))
                || edge
                    .memory_args
                    .iter()
                    .any(|memory| memories.contains(memory))
        };
        let used = match &block.terminator {
            KirTerminator::Return { value, memory, .. } => {
                value.is_some_and(|value| values.contains(&value))
                    || memory.iter().any(|(_, version)| memories.contains(version))
            }
            KirTerminator::Jump { edge } => edge_uses(edge),
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => values.contains(condition) || edge_uses(then_edge) || edge_uses(else_edge),
        };
        if used {
            return true;
        }
    }
    false
}

fn forward_edge(incoming: &mut KirEdge, bridge: &KirBlock, outgoing: &KirEdge) {
    if incoming.target != bridge.id {
        return;
    }
    let values = bridge
        .params
        .iter()
        .map(|param| param.value)
        .zip(&incoming.args)
        .collect::<BTreeMap<_, _>>();
    let memories = bridge
        .memory_params
        .iter()
        .map(|param| param.version)
        .zip(&incoming.memory_args)
        .collect::<BTreeMap<_, _>>();
    let args = outgoing
        .args
        .iter()
        .map(|value| values.get(value).map_or(*value, |value| **value))
        .collect();
    let memory_args = outgoing
        .memory_args
        .iter()
        .map(|version| memories.get(version).map_or(*version, |version| **version))
        .collect();
    *incoming = KirEdge {
        target: outgoing.target,
        args,
        memory_args,
    };
}
