use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, KirBlock, KirBlockParam, KirEdge, KirFunction, KirMemoryBlockParam, KirModule,
    KirTerminator, MemoryVersionId, ValueId,
};

use super::super::{LoopFallback, LoopFallbackReason, analyze_natural_loops};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopSimplifyResult {
    pub changed: bool,
    pub normalized_loops: u32,
    pub fallbacks: Vec<LoopFallback>,
}

pub fn canonicalize_kir_loops(module: &mut KirModule) -> Result<LoopSimplifyResult, String> {
    let original = module.clone();
    match canonicalize_kir_loops_transaction(module) {
        Ok(result) => Ok(result),
        Err(error) => {
            *module = original;
            Err(error)
        }
    }
}

fn canonicalize_kir_loops_transaction(
    module: &mut KirModule,
) -> Result<LoopSimplifyResult, String> {
    let mut result = LoopSimplifyResult::default();
    let mut ids = FreshIds::for_module(module)?;
    for function in &mut module.functions {
        let maximum_rounds = function.blocks.len().saturating_mul(4).saturating_add(16);
        let mut normalized_headers = BTreeSet::new();
        let mut completed = false;
        for _ in 0..maximum_rounds {
            let analysis = analyze_natural_loops(function);
            if analysis.budget_exhausted {
                result.fallbacks.push(LoopFallback {
                    function: function.id,
                    header: None,
                    reason: LoopFallbackReason::BudgetExhausted,
                });
                completed = true;
                break;
            }
            if !analysis.irreducible_blocks.is_empty() {
                result.fallbacks.push(LoopFallback {
                    function: function.id,
                    header: None,
                    reason: LoopFallbackReason::IrreducibleControlFlow,
                });
                completed = true;
                break;
            }
            let predecessors = predecessor_map(function);
            let mut changed_this_round = false;
            for loop_info in analysis
                .loops
                .iter()
                .filter(|candidate| {
                    !analysis.loops.iter().any(|child| {
                        child
                            .parent
                            .is_some_and(|parent| analysis.loops[parent].header == candidate.header)
                    })
                })
                .chain(analysis.loops.iter())
            {
                let loop_blocks = loop_info.blocks.iter().copied().collect::<BTreeSet<_>>();
                let entry_sources = predecessors
                    .get(&loop_info.header)
                    .into_iter()
                    .flatten()
                    .filter(|source| !loop_blocks.contains(source))
                    .copied()
                    .collect::<Vec<_>>();
                let has_preheader = matches!(entry_sources.as_slice(), [source]
                    if successor_ids(&block(function, *source).terminator) == [loop_info.header]);
                if !has_preheader {
                    insert_bridge(
                        function,
                        loop_info.header,
                        |source| !loop_blocks.contains(&source),
                        "loop.preheader",
                        &mut ids,
                    )?;
                    normalized_headers.insert(loop_info.header);
                    result.changed = true;
                    changed_this_round = true;
                    break;
                }

                let has_latch = matches!(loop_info.latches.as_slice(), [source]
                    if matches!(&block(function, *source).terminator,
                        KirTerminator::Jump { edge } if edge.target == loop_info.header));
                if !has_latch {
                    insert_bridge(
                        function,
                        loop_info.header,
                        |source| loop_blocks.contains(&source),
                        "loop.latch",
                        &mut ids,
                    )?;
                    normalized_headers.insert(loop_info.header);
                    result.changed = true;
                    changed_this_round = true;
                    break;
                }

                let exits = loop_info
                    .blocks
                    .iter()
                    .flat_map(|source| successor_ids(&block(function, *source).terminator))
                    .filter(|target| !loop_blocks.contains(target))
                    .collect::<BTreeSet<_>>();
                let shared_exit = exits.iter().copied().find(|exit| {
                    predecessors
                        .get(exit)
                        .into_iter()
                        .flatten()
                        .any(|source| !loop_blocks.contains(source))
                });
                if let Some(exit) = shared_exit {
                    insert_bridge(
                        function,
                        exit,
                        |source| loop_blocks.contains(&source),
                        "loop.exit",
                        &mut ids,
                    )?;
                    normalized_headers.insert(loop_info.header);
                    result.changed = true;
                    changed_this_round = true;
                    break;
                }
            }
            if !changed_this_round {
                completed = true;
                break;
            }
        }
        if !completed {
            result.fallbacks.push(LoopFallback {
                function: function.id,
                header: None,
                reason: LoopFallbackReason::UnsafeNormalization,
            });
        }
        result.normalized_loops = result
            .normalized_loops
            .saturating_add(u32::try_from(normalized_headers.len()).unwrap_or(u32::MAX));
    }
    result
        .fallbacks
        .sort_by_key(|fallback| (fallback.function, fallback.header, fallback.reason));
    Ok(result)
}

struct FreshIds {
    block: u32,
    value: u32,
    memory: u32,
}

impl FreshIds {
    fn for_module(module: &KirModule) -> Result<Self, String> {
        fn next(maximum: Option<u32>, kind: &str) -> Result<u32, String> {
            maximum
                .map_or(Some(0), |value| value.checked_add(1))
                .ok_or_else(|| format!("KIR {kind} identity space is exhausted"))
        }
        Ok(Self {
            block: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| &function.blocks)
                    .map(|block| block.id.index())
                    .max(),
                "block",
            )?,
            value: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| {
                        function.params.iter().map(|param| param.value).chain(
                            function.blocks.iter().flat_map(|block| {
                                block.params.iter().map(|param| param.value).chain(
                                    block.instructions.iter().flat_map(|instruction| {
                                        instruction.results.iter().map(|result| result.value)
                                    }),
                                )
                            }),
                        )
                    })
                    .map(ValueId::index)
                    .max(),
                "value",
            )?,
            memory: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| {
                        function
                            .initial_memory
                            .iter()
                            .map(|memory| memory.version)
                            .chain(function.blocks.iter().flat_map(|block| {
                                block.memory_params.iter().map(|param| param.version).chain(
                                    block.instructions.iter().flat_map(|instruction| {
                                        instruction
                                            .memory
                                            .iter()
                                            .flat_map(|memory| [Some(memory.input), memory.output])
                                            .flatten()
                                    }),
                                )
                            }))
                    })
                    .map(MemoryVersionId::index)
                    .max(),
                "memory",
            )?,
        })
    }

    fn block(&mut self) -> Result<BlockId, String> {
        let id = BlockId::from_index(self.block);
        self.block = self
            .block
            .checked_add(1)
            .ok_or_else(|| "KIR block identity space is exhausted".to_string())?;
        Ok(id)
    }

    fn value(&mut self) -> Result<ValueId, String> {
        let id = ValueId::from_index(self.value);
        self.value = self
            .value
            .checked_add(1)
            .ok_or_else(|| "KIR value identity space is exhausted".to_string())?;
        Ok(id)
    }

    fn memory(&mut self) -> Result<MemoryVersionId, String> {
        let id = MemoryVersionId::from_index(self.memory);
        self.memory = self
            .memory
            .checked_add(1)
            .ok_or_else(|| "KIR memory identity space is exhausted".to_string())?;
        Ok(id)
    }
}

fn insert_bridge(
    function: &mut KirFunction,
    target: BlockId,
    redirect_source: impl Fn(BlockId) -> bool,
    label: &str,
    ids: &mut FreshIds,
) -> Result<(), String> {
    let target_block = block(function, target).clone();
    let bridge_id = ids.block()?;
    let mut params = Vec::with_capacity(target_block.params.len());
    for param in &target_block.params {
        params.push(KirBlockParam {
            value: ids.value()?,
            slot: param.slot.clone(),
            type_node: param.type_node.clone(),
        });
    }
    let mut memory_params = Vec::with_capacity(target_block.memory_params.len());
    for param in &target_block.memory_params {
        memory_params.push(KirMemoryBlockParam {
            version: ids.memory()?,
            region: param.region,
        });
    }
    let args = params.iter().map(|param| param.value).collect();
    let memory_args = memory_params.iter().map(|param| param.version).collect();
    let bridge = KirBlock {
        id: bridge_id,
        label: format!("{label}.b{}", target.index()),
        params,
        memory_params,
        instructions: Vec::new(),
        terminator: KirTerminator::Jump {
            edge: KirEdge {
                target,
                args,
                memory_args,
            },
        },
    };
    for source in &mut function.blocks {
        if !redirect_source(source.id) {
            continue;
        }
        for edge in edges_mut(&mut source.terminator) {
            if edge.target == target {
                edge.target = bridge_id;
            }
        }
    }
    function.blocks.push(bridge);
    Ok(())
}

fn block(function: &KirFunction, id: BlockId) -> &KirBlock {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .expect("natural loop block exists")
}

fn predecessor_map(function: &KirFunction) -> BTreeMap<BlockId, Vec<BlockId>> {
    let mut result = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for block in &function.blocks {
        for target in successor_ids(&block.terminator) {
            result.entry(target).or_default().push(block.id);
        }
    }
    for predecessors in result.values_mut() {
        predecessors.sort_unstable();
        predecessors.dedup();
    }
    result
}

fn successor_ids(terminator: &KirTerminator) -> Vec<BlockId> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge.target],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge.target, else_edge.target],
    }
}

fn edges_mut(terminator: &mut KirTerminator) -> Vec<&mut KirEdge> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
    }
}
