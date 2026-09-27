use std::collections::{BTreeMap, BTreeSet};

use crate::{
    EffectSummary, EffectTarget, KirEdge, KirFunction, KirInitialMemory, KirInstructionKind,
    KirMemoryBlockParam, KirModule, KirPlace, KirTerminator, MemoryRegionId, MemoryVersionId,
};

use super::super::facts::FactArena;
use super::{AliasKind, RegionAnalysisError, analyze_regions, query_alias};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySsaFunctionReport {
    pub function: crate::FunctionId,
    pub partition_count: usize,
    pub collapsed_for_call: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySsaReport {
    pub functions: Vec<MemorySsaFunctionReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySsaError {
    pub message: String,
}

impl From<RegionAnalysisError> for MemorySsaError {
    fn from(error: RegionAnalysisError) -> Self {
        Self {
            message: error.message,
        }
    }
}

/// Rebuilds Memory SSA from the shared pairwise alias service.
pub fn refine_memory_ssa(
    module: &mut KirModule,
    facts: Option<&FactArena>,
) -> Result<MemorySsaReport, MemorySsaError> {
    refine_memory_ssa_impl(module, facts, None)
}

/// Effect-aware variant that maps callee parameter effects before partitioning.
pub fn refine_memory_ssa_with_effects(
    module: &mut KirModule,
    facts: Option<&FactArena>,
    summaries: &BTreeMap<String, EffectSummary>,
) -> Result<MemorySsaReport, MemorySsaError> {
    refine_memory_ssa_impl(module, facts, Some(summaries))
}

fn refine_memory_ssa_impl(
    module: &mut KirModule,
    facts: Option<&FactArena>,
    summaries: Option<&BTreeMap<String, EffectSummary>>,
) -> Result<MemorySsaReport, MemorySsaError> {
    let mut next_memory = module
        .functions
        .iter()
        .flat_map(|function| {
            function
                .initial_memory
                .iter()
                .map(|memory| memory.version)
                .chain(function.blocks.iter().flat_map(|block| {
                    block.memory_params.iter().map(|param| param.version).chain(
                        block.instructions.iter().filter_map(|instruction| {
                            instruction.memory.as_ref().and_then(|memory| memory.output)
                        }),
                    )
                }))
        })
        .map(MemoryVersionId::index)
        .max()
        .map_or(0, |maximum| maximum.saturating_add(1));
    let mut reports = Vec::new();
    for function in &mut module.functions {
        let analysis = analyze_regions(function, facts)?;
        let conservative = function
            .regions
            .iter()
            .find(|region| matches!(region.origin, crate::KirMemoryRegionOrigin::Conservative))
            .map(|region| region.id)
            .ok_or_else(|| MemorySsaError {
                message: format!(
                    "KIR function '{}' has no conservative region",
                    function.name
                ),
            })?;
        let collapsed_for_call = if let Some(summaries) = summaries {
            map_call_memory_regions(function, &analysis, summaries, conservative)
        } else {
            function.blocks.iter().any(|block| {
                block
                    .instructions
                    .iter()
                    .any(|instruction| matches!(instruction.kind, KirInstructionKind::Call { .. }))
            })
        };
        let partition_map = if collapsed_for_call {
            function
                .regions
                .iter()
                .map(|region| (region.id, conservative))
                .collect()
        } else {
            plan_partitions(function, &analysis, conservative)
        };
        let mut partitions = accessed_partitions(function, &partition_map, conservative);
        if partitions.is_empty() {
            partitions.insert(conservative);
        }
        for region in &mut function.regions {
            region.partition = partition_map
                .get(&region.id)
                .copied()
                .or_else(|| {
                    analysis
                        .root(region.id)
                        .and_then(|root| partition_map.get(&root).copied())
                })
                .unwrap_or(conservative);
        }
        rebuild_function_memory_ssa(
            function,
            &partition_map,
            &partitions,
            conservative,
            &mut next_memory,
        )?;
        reports.push(MemorySsaFunctionReport {
            function: function.id,
            partition_count: partitions.len(),
            collapsed_for_call,
        });
    }
    Ok(MemorySsaReport { functions: reports })
}

fn map_call_memory_regions(
    function: &mut KirFunction,
    analysis: &super::RegionAnalysis,
    summaries: &BTreeMap<String, EffectSummary>,
    conservative: MemoryRegionId,
) -> bool {
    let mut collapsed = false;
    for instruction in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
    {
        let KirInstructionKind::Call {
            function_name,
            args,
        } = &instruction.kind
        else {
            continue;
        };
        let Some(summary) = summaries.get(function_name) else {
            if let Some(memory) = &mut instruction.memory {
                memory.region = conservative;
            }
            collapsed = true;
            continue;
        };
        let mut regions = BTreeSet::new();
        let mut all = false;
        for access in summary.accesses() {
            match access.target {
                EffectTarget::All => all = true,
                EffectTarget::Parameter(index) => {
                    if let Some(region) = args
                        .get(index as usize)
                        .and_then(|value| analysis.region_for_value(*value))
                    {
                        regions.insert(region);
                    } else {
                        all = true;
                    }
                }
            }
        }
        if all || regions.len() > 1 {
            if let Some(memory) = &mut instruction.memory {
                memory.region = conservative;
            }
            collapsed = true;
        } else if let Some(region) = regions.first().copied() {
            if let Some(memory) = &mut instruction.memory {
                memory.region = region;
            }
        } else {
            instruction.memory = None;
        }
    }
    collapsed
}

fn plan_partitions(
    function: &KirFunction,
    analysis: &super::RegionAnalysis,
    conservative: MemoryRegionId,
) -> BTreeMap<MemoryRegionId, MemoryRegionId> {
    let accessed = accessed_regions(function);
    let mut parent = accessed
        .iter()
        .copied()
        .map(|region| (region, region))
        .collect::<BTreeMap<_, _>>();
    for (index, left) in accessed.iter().enumerate() {
        for right in accessed.iter().skip(index + 1) {
            if query_alias(analysis, *left, *right).kind != AliasKind::NoAlias {
                union(&mut parent, *left, *right);
            }
        }
    }
    let mut components = BTreeMap::<MemoryRegionId, Vec<MemoryRegionId>>::new();
    for region in &accessed {
        let root = find_component(&parent, *region);
        components.entry(root).or_default().push(*region);
    }
    let mut result = function
        .regions
        .iter()
        .map(|region| (region.id, conservative))
        .collect::<BTreeMap<_, _>>();
    for component in components.values() {
        let partition = if component.len() == 1 && component[0] != conservative {
            component[0]
        } else {
            conservative
        };
        for region in component {
            result.insert(*region, partition);
        }
    }
    result
}

fn accessed_regions(function: &KirFunction) -> Vec<MemoryRegionId> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| match &instruction.kind {
            KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
                Some(place_region(place))
            }
            KirInstructionKind::Call { .. } => {
                instruction.memory.as_ref().map(|memory| memory.region)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn accessed_partitions(
    function: &KirFunction,
    partition_map: &BTreeMap<MemoryRegionId, MemoryRegionId>,
    conservative: MemoryRegionId,
) -> BTreeSet<MemoryRegionId> {
    accessed_regions(function)
        .into_iter()
        .map(|region| partition_map.get(&region).copied().unwrap_or(conservative))
        .collect()
}

fn rebuild_function_memory_ssa(
    function: &mut KirFunction,
    partition_map: &BTreeMap<MemoryRegionId, MemoryRegionId>,
    partitions: &BTreeSet<MemoryRegionId>,
    conservative: MemoryRegionId,
    next_memory: &mut u32,
) -> Result<(), MemorySsaError> {
    let initial = partitions
        .iter()
        .map(|region| {
            Ok(KirInitialMemory {
                region: *region,
                version: allocate_memory(next_memory)?,
            })
        })
        .collect::<Result<Vec<_>, MemorySsaError>>()?;
    let initial_map = initial
        .iter()
        .map(|memory| (memory.region, memory.version))
        .collect::<BTreeMap<_, _>>();
    function.initial_memory = initial;
    let mut entries = BTreeMap::new();
    for (index, block) in function.blocks.iter_mut().enumerate() {
        if index == 0 {
            block.memory_params.clear();
            entries.insert(block.id, initial_map.clone());
        } else {
            block.memory_params = partitions
                .iter()
                .map(|region| {
                    Ok(KirMemoryBlockParam {
                        version: allocate_memory(next_memory)?,
                        region: *region,
                    })
                })
                .collect::<Result<Vec<_>, MemorySsaError>>()?;
            entries.insert(
                block.id,
                block
                    .memory_params
                    .iter()
                    .map(|param| (param.region, param.version))
                    .collect(),
            );
        }
    }
    let mut exits = BTreeMap::new();
    for block in &mut function.blocks {
        let mut current = entries.get(&block.id).cloned().unwrap_or_default();
        for instruction in &mut block.instructions {
            let Some(memory) = &mut instruction.memory else {
                continue;
            };
            let source_region = match &instruction.kind {
                KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
                    place_region(place)
                }
                KirInstructionKind::Call { .. } => memory.region,
                _ => memory.region,
            };
            let partition = partition_map
                .get(&source_region)
                .copied()
                .unwrap_or(conservative);
            memory.region = partition;
            memory.input = current
                .get(&partition)
                .copied()
                .ok_or_else(|| MemorySsaError {
                    message: format!(
                        "block b{} has no current memory for partition r{}",
                        block.id.index(),
                        partition.index()
                    ),
                })?;
            if memory.output.is_some() {
                let output = allocate_memory(next_memory)?;
                memory.output = Some(output);
                current.insert(partition, output);
            }
        }
        if let KirTerminator::Return { memory, .. } = &mut block.terminator {
            *memory = partitions
                .iter()
                .filter_map(|region| {
                    current
                        .get(region)
                        .copied()
                        .map(|version| (*region, version))
                })
                .collect();
        }
        exits.insert(block.id, current);
    }
    for block in &mut function.blocks {
        let exit = exits.get(&block.id).cloned().unwrap_or_default();
        match &mut block.terminator {
            KirTerminator::Return { .. } => {}
            KirTerminator::Jump { edge } => rewrite_edge(edge, &exit, &entries)?,
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                rewrite_edge(then_edge, &exit, &entries)?;
                rewrite_edge(else_edge, &exit, &entries)?;
            }
        }
    }
    Ok(())
}

fn rewrite_edge(
    edge: &mut KirEdge,
    exit: &BTreeMap<MemoryRegionId, MemoryVersionId>,
    entries: &BTreeMap<crate::BlockId, BTreeMap<MemoryRegionId, MemoryVersionId>>,
) -> Result<(), MemorySsaError> {
    let target = entries.get(&edge.target).ok_or_else(|| MemorySsaError {
        message: format!("memory edge names missing target b{}", edge.target.index()),
    })?;
    edge.memory_args = target
        .keys()
        .map(|region| {
            exit.get(region).copied().ok_or_else(|| MemorySsaError {
                message: format!(
                    "memory edge to b{} has no version for partition r{}",
                    edge.target.index(),
                    region.index()
                ),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(())
}

fn allocate_memory(next: &mut u32) -> Result<MemoryVersionId, MemorySsaError> {
    let current = *next;
    *next = next.checked_add(1).ok_or_else(|| MemorySsaError {
        message: "Memory SSA identity space exhausted".to_string(),
    })?;
    Ok(MemoryVersionId::from_index(current))
}

fn place_region(place: &KirPlace) -> MemoryRegionId {
    match place {
        KirPlace::Value { region, .. }
        | KirPlace::Deref { region, .. }
        | KirPlace::Index { region, .. }
        | KirPlace::SliceIndex { region, .. }
        | KirPlace::Field { region, .. } => *region,
    }
}

fn find_component(
    parent: &BTreeMap<MemoryRegionId, MemoryRegionId>,
    mut region: MemoryRegionId,
) -> MemoryRegionId {
    while let Some(next) = parent.get(&region).copied()
        && next != region
    {
        region = next;
    }
    region
}

fn union(
    parent: &mut BTreeMap<MemoryRegionId, MemoryRegionId>,
    left: MemoryRegionId,
    right: MemoryRegionId,
) {
    let left = find_component(parent, left);
    let right = find_component(parent, right);
    if left == right {
        return;
    }
    let (root, child) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    parent.insert(child, root);
}
