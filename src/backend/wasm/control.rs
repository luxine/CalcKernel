use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    BlockId, KirBlock, KirDominators, KirEdge, KirFunction, KirTerminator, compute_kir_dominators,
};

#[derive(Debug)]
pub(super) struct StructurePlan {
    pub root: StructureRegion,
    pub reachable: BTreeSet<BlockId>,
    pub forward_labels: BTreeMap<BlockId, String>,
    pub loop_labels: BTreeMap<BlockId, String>,
    pub branch_targets: BTreeMap<(BlockId, u8), BranchTarget>,
}

#[derive(Debug)]
pub(super) struct StructureRegion {
    pub owner: Option<BlockId>,
    pub items: Vec<StructureItem>,
}

#[derive(Debug)]
pub(super) enum StructureItem {
    Block(BlockId),
    Loop {
        header: BlockId,
        body: Box<StructureRegion>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BranchTarget {
    Forward(BlockId),
    Loop(BlockId),
}

const MAX_REACHABLE_BLOCKS: usize = 256;
const MAX_LOOP_DEPTH: usize = 32;

type ReachableGraph<'a> = (
    BTreeSet<BlockId>,
    BTreeMap<BlockId, usize>,
    BTreeMap<BlockId, &'a KirBlock>,
);

pub(super) fn plan_structure(function: &KirFunction) -> Option<StructurePlan> {
    let (reachable, order, blocks) = reachable_blocks(function)?;
    if reachable.is_empty() || reachable.len() > MAX_REACHABLE_BLOCKS {
        return None;
    }
    let predecessors = predecessors(function, &reachable);
    let dominators = compute_kir_dominators(function);
    let loops = discover_loops(function, &reachable, &predecessors, &dominators)?;
    let loop_data = attach_loop_parents(loops, &order)?;
    let root = build_region(
        None,
        0,
        &reachable,
        &loop_data,
        &blocks,
        &order,
        function.blocks.first()?.id,
    )?;

    let mut plan = StructurePlan {
        root,
        reachable,
        forward_labels: BTreeMap::new(),
        loop_labels: BTreeMap::new(),
        branch_targets: BTreeMap::new(),
    };
    collect_labels(&plan.root, &mut plan.forward_labels, &mut plan.loop_labels);
    if !validate_scopes(
        &plan.root,
        &BTreeSet::new(),
        &BTreeSet::new(),
        &plan.forward_labels,
        &plan.loop_labels,
        &blocks,
        &mut plan.branch_targets,
    ) {
        return None;
    }
    let expected_edges = plan
        .reachable
        .iter()
        .filter_map(|id| blocks.get(id))
        .map(|block| outgoing_edges(&block.terminator).len())
        .sum::<usize>();
    (plan.branch_targets.len() == expected_edges).then_some(plan)
}

fn reachable_blocks(function: &KirFunction) -> Option<ReachableGraph<'_>> {
    let mut order = BTreeMap::new();
    let mut blocks = BTreeMap::new();
    for (ordinal, block) in function.blocks.iter().enumerate() {
        if order.insert(block.id, ordinal).is_some() {
            return None;
        }
        blocks.insert(block.id, block);
    }
    let entry = function.blocks.first()?.id;
    let mut reachable = BTreeSet::new();
    let mut work = VecDeque::from([entry]);
    while let Some(block_id) = work.pop_front() {
        if !reachable.insert(block_id) {
            continue;
        }
        if reachable.len() > MAX_REACHABLE_BLOCKS {
            return None;
        }
        let block = blocks.get(&block_id)?;
        for (_, edge) in outgoing_edges(&block.terminator) {
            if !blocks.contains_key(&edge.target) {
                return None;
            }
            if !reachable.contains(&edge.target) {
                work.push_back(edge.target);
            }
        }
    }
    Some((reachable, order, blocks))
}

fn outgoing_edges(terminator: &KirTerminator) -> Vec<(u8, &KirEdge)> {
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

fn predecessors(
    function: &KirFunction,
    reachable: &BTreeSet<BlockId>,
) -> BTreeMap<BlockId, Vec<BlockId>> {
    let mut result = reachable
        .iter()
        .copied()
        .map(|block| (block, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for block in &function.blocks {
        if !reachable.contains(&block.id) {
            continue;
        }
        for (_, edge) in outgoing_edges(&block.terminator) {
            if let Some(incoming) = result.get_mut(&edge.target) {
                incoming.push(block.id);
            }
        }
    }
    result
}

#[derive(Debug)]
struct NaturalLoop {
    blocks: BTreeSet<BlockId>,
    parent: Option<BlockId>,
}

fn discover_loops(
    function: &KirFunction,
    reachable: &BTreeSet<BlockId>,
    predecessors: &BTreeMap<BlockId, Vec<BlockId>>,
    dominators: &KirDominators,
) -> Option<BTreeMap<BlockId, NaturalLoop>> {
    let mut latches = BTreeMap::<BlockId, BTreeSet<BlockId>>::new();
    for source in reachable {
        let block = function.blocks.iter().find(|block| block.id == *source)?;
        for (_, edge) in outgoing_edges(&block.terminator) {
            if dominators.dominates(edge.target, *source) {
                latches.entry(edge.target).or_default().insert(*source);
            }
        }
    }

    let mut loops = BTreeMap::new();
    for (header, latch_blocks) in latches {
        let mut loop_blocks = BTreeSet::from([header]);
        let mut work = Vec::new();
        for latch in latch_blocks {
            if loop_blocks.insert(latch) && latch != header {
                work.push(latch);
            }
        }
        while let Some(block) = work.pop() {
            for predecessor in predecessors.get(&block)? {
                if loop_blocks.insert(*predecessor) && *predecessor != header {
                    work.push(*predecessor);
                }
            }
        }
        loops.insert(
            header,
            NaturalLoop {
                blocks: loop_blocks,
                parent: None,
            },
        );
    }

    let headers = loops.keys().copied().collect::<Vec<_>>();
    for (index, left_header) in headers.iter().enumerate() {
        let left = &loops[left_header].blocks;
        for right_header in headers.iter().skip(index + 1) {
            let right = &loops[right_header].blocks;
            if left.is_disjoint(right) {
                continue;
            }
            if left == right || (!left.is_subset(right) && !right.is_subset(left)) {
                return None;
            }
        }
    }

    for (header, natural_loop) in &loops {
        for block in natural_loop.blocks.iter().filter(|block| *block != header) {
            if predecessors
                .get(block)?
                .iter()
                .any(|predecessor| !natural_loop.blocks.contains(predecessor))
            {
                return None;
            }
        }
    }
    Some(loops)
}

fn attach_loop_parents(
    mut loops: BTreeMap<BlockId, NaturalLoop>,
    order: &BTreeMap<BlockId, usize>,
) -> Option<BTreeMap<BlockId, NaturalLoop>> {
    let headers = loops.keys().copied().collect::<Vec<_>>();
    for child_header in &headers {
        let child_blocks = &loops[child_header].blocks;
        let parent = headers
            .iter()
            .filter(|candidate| {
                **candidate != *child_header && child_blocks.is_subset(&loops[candidate].blocks)
            })
            .min_by_key(|candidate| {
                (
                    loops[candidate].blocks.len(),
                    order.get(candidate).copied().unwrap_or(usize::MAX),
                )
            })
            .copied();
        loops.get_mut(child_header)?.parent = parent;
    }
    Some(loops)
}

fn build_region(
    owner: Option<BlockId>,
    depth: usize,
    reachable: &BTreeSet<BlockId>,
    loops: &BTreeMap<BlockId, NaturalLoop>,
    blocks: &BTreeMap<BlockId, &KirBlock>,
    order: &BTreeMap<BlockId, usize>,
    entry: BlockId,
) -> Option<StructureRegion> {
    let child_headers = loops
        .iter()
        .filter_map(|(header, natural_loop)| (natural_loop.parent == owner).then_some(*header))
        .collect::<Vec<_>>();
    if !child_headers.is_empty() && depth >= MAX_LOOP_DEPTH {
        return None;
    }
    let scope_blocks = if let Some(header) = owner {
        loops.get(&header)?.blocks.clone()
    } else {
        reachable.clone()
    };
    let mut representative = BTreeMap::<BlockId, BlockId>::new();
    for block in &scope_blocks {
        representative.insert(*block, *block);
    }
    for child in &child_headers {
        for block in &loops.get(child)?.blocks {
            representative.insert(*block, *child);
        }
    }

    let mut projected_edges = Vec::new();
    for source in &scope_blocks {
        let source_item = *representative.get(source)?;
        for (_, edge) in outgoing_edges(&blocks.get(source)?.terminator) {
            let Some(target_item) = representative.get(&edge.target).copied() else {
                continue;
            };
            if source_item == target_item || owner == Some(edge.target) {
                continue;
            }
            projected_edges.push((source_item, target_item));
        }
    }
    let items = representative.values().copied().collect::<BTreeSet<_>>();
    let sorted = stable_topological_order(&items, &projected_edges, order)?;
    let expected_entry = *representative.get(&entry)?;
    if sorted.first().copied() != Some(expected_entry) {
        return None;
    }

    let mut region_items = Vec::with_capacity(sorted.len());
    for item in sorted {
        if child_headers.contains(&item) {
            let body = build_region(Some(item), depth + 1, reachable, loops, blocks, order, item)?;
            region_items.push(StructureItem::Loop {
                header: item,
                body: Box::new(body),
            });
        } else {
            region_items.push(StructureItem::Block(item));
        }
    }
    Some(StructureRegion {
        owner,
        items: region_items,
    })
}

fn stable_topological_order(
    items: &BTreeSet<BlockId>,
    edges: &[(BlockId, BlockId)],
    order: &BTreeMap<BlockId, usize>,
) -> Option<Vec<BlockId>> {
    let mut indegrees = items
        .iter()
        .copied()
        .map(|item| (item, 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut successors = items
        .iter()
        .copied()
        .map(|item| (item, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for (source, target) in edges {
        if source == target {
            continue;
        }
        successors.get_mut(source)?.push(*target);
        *indegrees.get_mut(target)? += 1;
    }

    let mut ready = BTreeSet::new();
    for (item, indegree) in &indegrees {
        if *indegree == 0 {
            ready.insert((order.get(item).copied()?, *item));
        }
    }
    let mut sorted = Vec::with_capacity(items.len());
    while let Some(next) = ready.iter().next().copied() {
        ready.remove(&next);
        let source = next.1;
        sorted.push(source);
        for target in successors.get(&source)? {
            let indegree = indegrees.get_mut(target)?;
            *indegree = indegree.checked_sub(1)?;
            if *indegree == 0 {
                ready.insert((order.get(target).copied()?, *target));
            }
        }
    }
    (sorted.len() == items.len()).then_some(sorted)
}

fn collect_labels(
    region: &StructureRegion,
    forward_labels: &mut BTreeMap<BlockId, String>,
    loop_labels: &mut BTreeMap<BlockId, String>,
) {
    for item in &region.items {
        let representative = match item {
            StructureItem::Block(block) => *block,
            StructureItem::Loop { header, .. } => *header,
        };
        if region.owner != Some(representative) {
            forward_labels.insert(representative, format!("ik_to_b{}", representative.index()));
        }
        if let StructureItem::Loop { header, body } = item {
            loop_labels.insert(*header, format!("ik_loop_b{}", header.index()));
            collect_labels(body, forward_labels, loop_labels);
        }
    }
}

fn validate_scopes(
    region: &StructureRegion,
    active_loops: &BTreeSet<BlockId>,
    inherited_forward: &BTreeSet<BlockId>,
    forward_labels: &BTreeMap<BlockId, String>,
    loop_labels: &BTreeMap<BlockId, String>,
    blocks: &BTreeMap<BlockId, &KirBlock>,
    branch_targets: &mut BTreeMap<(BlockId, u8), BranchTarget>,
) -> bool {
    for (index, item) in region.items.iter().enumerate() {
        let mut forward = inherited_forward.clone();
        for future in region.items.iter().skip(index + 1) {
            forward.insert(match future {
                StructureItem::Block(block) => *block,
                StructureItem::Loop { header, .. } => *header,
            });
        }
        match item {
            StructureItem::Block(block_id) => {
                let Some(block) = blocks.get(block_id) else {
                    return false;
                };
                for (arm, edge) in outgoing_edges(&block.terminator) {
                    let target = if active_loops.contains(&edge.target) {
                        if !loop_labels.contains_key(&edge.target) {
                            return false;
                        }
                        BranchTarget::Loop(edge.target)
                    } else if forward.contains(&edge.target) {
                        if !forward_labels.contains_key(&edge.target) {
                            return false;
                        }
                        BranchTarget::Forward(edge.target)
                    } else {
                        return false;
                    };
                    if branch_targets.insert((*block_id, arm), target).is_some() {
                        return false;
                    }
                }
            }
            StructureItem::Loop { header, body } => {
                let mut nested_loops = active_loops.clone();
                nested_loops.insert(*header);
                if !validate_scopes(
                    body,
                    &nested_loops,
                    &forward,
                    forward_labels,
                    loop_labels,
                    blocks,
                    branch_targets,
                ) {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FunctionId, KirBlock, KirBoundsMode, KirBuildConfig, KirConsumer, KirEdge, KirOverflowMode,
        KirSanitizerMode, KirTerminator, MirType, SourceFile, ValueId, build_kir_module, check,
        lower_to_mir,
    };

    fn build_function(source: &str) -> KirFunction {
        let checked = check(&SourceFile::new("wasm-control-plan.ck", source));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid WebAssembly KIR");
        module.functions.into_iter().next().expect("one function")
    }

    #[test]
    fn structure_plan_should_cover_a_diamond_once() {
        let function = build_function(
            "export fn diamond(flag: bool, a: i32, b: i32) -> i32 {\
               let value: i32 = a; if flag { value = b; } return value;\
             }",
        );
        let plan = plan_structure(&function).expect("reducible diamond is structured");
        let mut blocks = Vec::new();
        collect_blocks(&plan.root, &mut blocks);
        assert_eq!(blocks.len(), plan.reachable.len());
        assert_eq!(
            blocks.iter().copied().collect::<BTreeSet<_>>(),
            plan.reachable
        );
    }

    #[test]
    fn structure_plan_should_build_nested_loop_regions() {
        let function = build_function(
            "export fn nested(n: u32) -> u32 {\
               let outer: u32 = 0; let hits: u32 = 0;\
               while outer < n { let inner: u32 = 0; while inner < n {\
                 inner = inner + 1; if inner == 2 { continue; }\
                 hits = hits + 1; if inner == 4 { break; }\
               } outer = outer + 1; } return hits;\
             }",
        );
        let plan = plan_structure(&function).expect("reducible nested loops are structured");
        let mut blocks = Vec::new();
        let loop_count = collect_blocks_and_loops(&plan.root, &mut blocks);
        assert_eq!(loop_count, 2);
        assert_eq!(blocks.len(), plan.reachable.len());
        assert_eq!(
            blocks.iter().copied().collect::<BTreeSet<_>>(),
            plan.reachable
        );
    }

    #[test]
    fn structure_plan_should_preserve_both_arms_when_they_share_a_target() {
        let target = BlockId::from_index(1);
        let function = KirFunction {
            id: FunctionId::from_index(0),
            name: "same_target".to_string(),
            exported: true,
            params: Vec::new(),
            return_type: MirType::Void,
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![
                KirBlock {
                    id: BlockId::from_index(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    memory_params: Vec::new(),
                    instructions: Vec::new(),
                    terminator: KirTerminator::Branch {
                        condition: ValueId::from_index(0),
                        then_edge: KirEdge {
                            target,
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                        else_edge: KirEdge {
                            target,
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                    },
                },
                KirBlock {
                    id: target,
                    label: "join".to_string(),
                    params: Vec::new(),
                    memory_params: Vec::new(),
                    instructions: Vec::new(),
                    terminator: KirTerminator::Return {
                        value: None,
                        memory: Vec::new(),
                        effect_order: 0,
                    },
                },
            ],
        };
        let plan = plan_structure(&function).expect("same-target branch is a DAG");
        assert_eq!(plan.branch_targets.len(), 2);
        assert_eq!(
            plan.branch_targets[&(BlockId::from_index(0), 0)],
            BranchTarget::Forward(target)
        );
        assert_eq!(
            plan.branch_targets[&(BlockId::from_index(0), 1)],
            BranchTarget::Forward(target)
        );
    }

    #[test]
    fn structure_plan_should_fall_back_for_a_two_entry_cycle() {
        let first = BlockId::from_index(1);
        let second = BlockId::from_index(2);
        let function = KirFunction {
            id: FunctionId::from_index(0),
            name: "irreducible".to_string(),
            exported: true,
            params: Vec::new(),
            return_type: MirType::Void,
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![
                KirBlock {
                    id: BlockId::from_index(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    memory_params: Vec::new(),
                    instructions: Vec::new(),
                    terminator: KirTerminator::Branch {
                        condition: ValueId::from_index(0),
                        then_edge: KirEdge {
                            target: first,
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                        else_edge: KirEdge {
                            target: second,
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                    },
                },
                KirBlock {
                    id: first,
                    label: "first".to_string(),
                    params: Vec::new(),
                    memory_params: Vec::new(),
                    instructions: Vec::new(),
                    terminator: KirTerminator::Jump {
                        edge: KirEdge {
                            target: second,
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                    },
                },
                KirBlock {
                    id: second,
                    label: "second".to_string(),
                    params: Vec::new(),
                    memory_params: Vec::new(),
                    instructions: Vec::new(),
                    terminator: KirTerminator::Jump {
                        edge: KirEdge {
                            target: first,
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                    },
                },
            ],
        };
        assert!(plan_structure(&function).is_none());
    }

    #[test]
    fn structure_plan_should_accept_a_single_block_self_loop_without_panicking() {
        let block = BlockId::from_index(0);
        let function = KirFunction {
            id: FunctionId::from_index(0),
            name: "self_loop".to_string(),
            exported: true,
            params: Vec::new(),
            return_type: MirType::Void,
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![KirBlock {
                id: block,
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Jump {
                    edge: KirEdge {
                        target: block,
                        args: Vec::new(),
                        memory_args: Vec::new(),
                    },
                },
            }],
        };
        let plan = plan_structure(&function).expect("self loop is a natural loop");
        assert_eq!(plan.loop_labels.len(), 1);
        assert_eq!(plan.branch_targets[&(block, 0)], BranchTarget::Loop(block));
    }

    #[test]
    fn structure_plan_should_fall_back_above_the_reachable_block_budget() {
        let block_count = MAX_REACHABLE_BLOCKS + 1;
        let blocks = (0..block_count)
            .map(|index| {
                let id = BlockId::from_index(index as u32);
                let terminator = if index + 1 < block_count {
                    KirTerminator::Jump {
                        edge: KirEdge {
                            target: BlockId::from_index((index + 1) as u32),
                            args: Vec::new(),
                            memory_args: Vec::new(),
                        },
                    }
                } else {
                    KirTerminator::Return {
                        value: None,
                        memory: Vec::new(),
                        effect_order: 0,
                    }
                };
                KirBlock {
                    id,
                    label: format!("b{index}"),
                    params: Vec::new(),
                    memory_params: Vec::new(),
                    instructions: Vec::new(),
                    terminator,
                }
            })
            .collect();
        let function = KirFunction {
            id: FunctionId::from_index(0),
            name: "over_budget".to_string(),
            exported: true,
            params: Vec::new(),
            return_type: MirType::Void,
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks,
        };
        assert!(plan_structure(&function).is_none());
    }

    fn collect_blocks(region: &StructureRegion, blocks: &mut Vec<BlockId>) {
        let _ = collect_blocks_and_loops(region, blocks);
    }

    fn collect_blocks_and_loops(region: &StructureRegion, blocks: &mut Vec<BlockId>) -> usize {
        region.items.iter().fold(0, |loops, item| match item {
            StructureItem::Block(block) => {
                blocks.push(*block);
                loops
            }
            StructureItem::Loop { header, body } => {
                assert_eq!(body.owner, Some(*header));
                loops + 1 + collect_blocks_and_loops(body, blocks)
            }
        })
    }
}
