use super::*;
use crate::{KirBlock, KirEdge, MemoryVersionId, MirType};

fn edge(target: u32, marker: u32) -> KirEdge {
    KirEdge {
        target: BlockId::from_index(target),
        args: vec![ValueId::from_index(marker)],
        memory_args: vec![MemoryVersionId::from_index(marker + 1)],
    }
}

fn function(terminators: Vec<KirTerminator>) -> KirFunction {
    KirFunction {
        id: FunctionId::from_index(0),
        name: "edge_query".into(),
        exported: false,
        params: Vec::new(),
        return_type: MirType::Void,
        regions: Vec::new(),
        initial_memory: Vec::new(),
        vector_regions: Vec::new(),
        blocks: terminators
            .into_iter()
            .enumerate()
            .map(|(index, terminator)| KirBlock {
                id: BlockId::from_index([701, 3, 9000, 101][index]),
                label: format!("block{index}"),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator,
            })
            .collect(),
    }
}

fn legacy_incoming_edges(function: &KirFunction, target: BlockId) -> Vec<(BlockId, &KirEdge)> {
    function
        .blocks
        .iter()
        .flat_map(|block| {
            let edges = match &block.terminator {
                KirTerminator::Return { .. } => Vec::new(),
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
            };
            edges
                .into_iter()
                .filter(move |edge| edge.target == target)
                .map(move |edge| (block.id, edge))
        })
        .collect()
}

#[test]
fn incoming_edges_should_preserve_both_parallel_edges_and_their_argument_identity() {
    let function = function(vec![
        KirTerminator::Jump { edge: edge(3, 10) },
        KirTerminator::Branch {
            condition: ValueId::from_index(7),
            then_edge: edge(3, 20),
            else_edge: edge(3, 30),
        },
        KirTerminator::Return {
            value: None,
            memory: Vec::new(),
            effect_order: 0,
        },
    ]);
    let actual = incoming_edges(&function, BlockId::from_index(3));
    assert_eq!(
        actual
            .iter()
            .map(|(id, edge)| (
                id.index(),
                edge.args[0].index(),
                edge.memory_args[0].index()
            ))
            .collect::<Vec<_>>(),
        [(701, 10, 11), (3, 20, 21), (3, 30, 31)]
    );
    let expected = legacy_incoming_edges(&function, BlockId::from_index(3));
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|((id, actual), (expected_id, expected))| *id == expected_id
                && std::ptr::eq(*actual, expected))
    );
}

#[test]
fn incoming_edges_should_match_legacy_queries_for_sparse_permuted_and_missing_targets() {
    let targets = [701, 3, 9000, 101, u32::MAX];
    for seed in 0..64 {
        let mut function = function(
            (0..4)
                .map(|index| {
                    let target = targets[(seed + index) % targets.len()];
                    let marker = (index * 10) as u32;
                    match (seed >> (index * 2)) % 4 {
                        0 => KirTerminator::Return {
                            value: None,
                            memory: Vec::new(),
                            effect_order: 0,
                        },
                        1 => KirTerminator::Jump {
                            edge: edge(target, marker),
                        },
                        variant => KirTerminator::Branch {
                            condition: ValueId::from_index(0),
                            then_edge: edge(target, marker),
                            else_edge: edge(
                                if variant == 2 {
                                    target
                                } else {
                                    targets[(seed + index + 1) % targets.len()]
                                },
                                marker + 2,
                            ),
                        },
                    }
                })
                .collect(),
        );
        for _ in 0..2 {
            for target in targets {
                let actual = incoming_edges(&function, BlockId::from_index(target));
                let expected = legacy_incoming_edges(&function, BlockId::from_index(target));
                assert_eq!(actual, expected, "seed={seed} target={target}");
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|((id, actual), (expected_id, expected))| *id == expected_id
                            && std::ptr::eq(*actual, expected))
                );
            }
            function.blocks.reverse();
        }
    }
    assert!(incoming_edges(&function(Vec::new()), BlockId::from_index(0)).is_empty());
}
