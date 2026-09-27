use std::collections::{BTreeMap, BTreeSet};

use crate::{
    EffectAccess, EffectCall, EffectFunction, EffectGraph, EffectSolveConfig, EffectSolveResult,
    EffectSummary, EffectTarget, KirEffectKind, KirFunction, KirInstructionKind,
    KirMemoryRegionOrigin, KirModule, KirPlace, MemoryEffect, MemoryRegionId, MirType, ValueId,
    solve_effect_graph,
};

use super::analyze_regions;

/// Builds the shared effect graph from current KIR and solves it with the canonical SCC engine.
#[must_use]
pub fn solve_kir_effects(
    module: &KirModule,
    unsafe_functions: &BTreeSet<String>,
    config: EffectSolveConfig,
) -> EffectSolveResult {
    let graph = EffectGraph {
        functions: module
            .functions
            .iter()
            .map(|function| kir_effect_function(function, unsafe_functions))
            .collect(),
    };
    solve_effect_graph(&graph, config)
}

fn kir_effect_function(
    function: &KirFunction,
    unsafe_functions: &BTreeSet<String>,
) -> EffectFunction {
    let regions = analyze_regions(function, None).ok();
    let parameter_targets = function
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| {
            let target = match param.type_node {
                MirType::Slice(_) => u32::try_from(index)
                    .map(EffectTarget::Parameter)
                    .unwrap_or(EffectTarget::All),
                MirType::Pointer(_) => EffectTarget::All,
                _ => return None,
            };
            Some((param.value, target))
        })
        .collect::<BTreeMap<_, _>>();
    let region_targets = function
        .regions
        .iter()
        .filter_map(|region| {
            let KirMemoryRegionOrigin::Parameter(value) = region.origin else {
                return None;
            };
            parameter_targets
                .get(&value)
                .copied()
                .map(|target| (region.id, target))
        })
        .collect::<BTreeMap<_, _>>();
    let target_for_region = |region: MemoryRegionId| {
        regions
            .as_ref()
            .and_then(|analysis| analysis.root(region))
            .and_then(|root| region_targets.get(&root).copied())
            .unwrap_or(EffectTarget::All)
    };
    let target_for_value = |value: ValueId| {
        parameter_targets.get(&value).copied().or_else(|| {
            regions
                .as_ref()
                .and_then(|analysis| analysis.region_for_value(value))
                .map(target_for_region)
        })
    };
    let mut direct = EffectSummary::empty();
    let mut calls = Vec::new();
    for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
        match &instruction.kind {
            KirInstructionKind::Load { place } => direct.add_access(EffectAccess {
                target: target_for_region(place_region(place)),
                effect: MemoryEffect::Read,
            }),
            KirInstructionKind::Store { place, .. } => direct.add_access(EffectAccess {
                target: target_for_region(place_region(place)),
                effect: MemoryEffect::Write,
            }),
            KirInstructionKind::Call {
                function_name,
                args,
            } => {
                let is_unsafe = unsafe_functions.contains(function_name);
                direct.unsafe_calls |= is_unsafe;
                calls.push(EffectCall {
                    callee: function_name.clone(),
                    arguments: args.iter().map(|value| target_for_value(*value)).collect(),
                    is_unsafe,
                });
            }
            KirInstructionKind::RuntimeCall { .. } => direct.runtime_effect = true,
            KirInstructionKind::Guard { .. } => direct.may_fail = true,
            _ => {}
        }
        if instruction
            .effect
            .as_ref()
            .is_some_and(|effect| effect.kind == KirEffectKind::MayFail)
        {
            direct.may_fail = true;
        }
    }
    EffectFunction {
        name: function.name.clone(),
        parameter_count: u32::try_from(function.params.len()).unwrap_or(u32::MAX),
        direct,
        calls,
    }
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
