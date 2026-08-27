use std::collections::{HashMap, HashSet};

use crate::{
    MirBinaryOp, MirFunction, MirInstruction, MirInstructionEffect, MirModule, MirPlace,
    MirTerminator, MirValue, instruction_effect,
};

use super::super::{analysis::*, pipeline::*};
use super::cfg::terminator_targets;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopBackEdge {
    from: String,
    to: String,
}

#[allow(dead_code)]
struct NaturalLoop {
    header: String,
    back_edge: LoopBackEdge,
    blocks: HashSet<String>,
    preheader: String,
    exit_blocks: Vec<String>,
}

pub(in crate::optimizer) fn run_loop_invariant_code_motion(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    if context.opt_level < 3
        || context.overflow_mode == MirPassOverflowMode::Checked
        || context.bounds_mode == MirPassBoundsMode::Checked
    {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let mut changed = false;
    for function in &mut module.functions {
        changed |= hoist_loop_invariants(function);
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn hoist_loop_invariants(function: &mut MirFunction) -> bool {
    let mut changed = false;
    for natural_loop in analyze_natural_loops(function) {
        changed |= hoist_in_loop(function, &natural_loop);
    }
    changed
}

fn hoist_in_loop(function: &mut MirFunction, natural_loop: &NaturalLoop) -> bool {
    let loop_defined_temps = collect_loop_defined_temps(function, &natural_loop.blocks);
    let loop_assigned_locals = collect_loop_assigned_locals(function, &natural_loop.blocks);
    let mut hoisted_temps = HashSet::new();
    let mut hoisted_instructions = Vec::new();
    let mut changed = false;

    for block in &mut function.blocks {
        if !natural_loop.blocks.contains(&block.label) {
            continue;
        }
        let mut kept = Vec::with_capacity(block.instructions.len());
        for instruction in std::mem::take(&mut block.instructions) {
            if is_hoistable_instruction(
                &instruction,
                &loop_defined_temps,
                &loop_assigned_locals,
                &hoisted_temps,
            ) {
                remember_hoisted_target(&instruction, &mut hoisted_temps);
                hoisted_instructions.push(instruction);
                changed = true;
            } else {
                kept.push(instruction);
            }
        }
        block.instructions = kept;
    }

    if !hoisted_instructions.is_empty()
        && let Some(preheader) = function
            .blocks
            .iter_mut()
            .find(|block| block.label == natural_loop.preheader)
    {
        preheader.instructions.extend(hoisted_instructions);
    }

    changed
}

fn is_hoistable_instruction(
    instruction: &MirInstruction,
    loop_defined_temps: &HashSet<String>,
    loop_assigned_locals: &HashSet<String>,
    hoisted_temps: &HashSet<String>,
) -> bool {
    if instruction_effect(instruction) != MirInstructionEffect::Pure {
        return false;
    }
    match instruction {
        MirInstruction::ConstInt { target, .. } | MirInstruction::ConstBool { target, .. } => {
            matches!(target, MirValue::Temp { .. })
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            matches!(target, MirValue::Temp { .. })
                && matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul)
                && !is_f64_type(value_type(target))
                && !is_f64_type(value_type(left))
                && !is_f64_type(value_type(right))
                && is_invariant_value(
                    left,
                    loop_defined_temps,
                    loop_assigned_locals,
                    hoisted_temps,
                )
                && is_invariant_value(
                    right,
                    loop_defined_temps,
                    loop_assigned_locals,
                    hoisted_temps,
                )
        }
        MirInstruction::ConstFloat { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Unary { .. }
        | MirInstruction::Compare { .. }
        | MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. }
        | MirInstruction::RuntimeCall { .. } => false,
    }
}

fn is_invariant_value(
    value: &MirValue,
    loop_defined_temps: &HashSet<String>,
    loop_assigned_locals: &HashSet<String>,
    hoisted_temps: &HashSet<String>,
) -> bool {
    match value {
        MirValue::ConstInt { .. }
        | MirValue::ConstBool { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::Param { .. } => true,
        MirValue::Local { name, .. } => !loop_assigned_locals.contains(name),
        MirValue::Temp { name, .. } => {
            !loop_defined_temps.contains(name) || hoisted_temps.contains(name)
        }
    }
}

fn collect_loop_defined_temps(
    function: &MirFunction,
    loop_blocks: &HashSet<String>,
) -> HashSet<String> {
    let mut temps = HashSet::new();
    for block in &function.blocks {
        if !loop_blocks.contains(&block.label) {
            continue;
        }
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
                temps.insert(name.clone());
            }
        }
    }
    temps
}

fn collect_loop_assigned_locals(
    function: &MirFunction,
    loop_blocks: &HashSet<String>,
) -> HashSet<String> {
    let mut locals = HashSet::new();
    for block in &function.blocks {
        if !loop_blocks.contains(&block.label) {
            continue;
        }
        for instruction in &block.instructions {
            if let Some(MirValue::Local { name, .. }) = instruction_target(instruction) {
                locals.insert(name.clone());
            }
            if let MirInstruction::Store { place, .. } = instruction {
                collect_assigned_place_local(place, &mut locals);
            }
        }
    }
    locals
}

fn collect_assigned_place_local(place: &MirPlace, locals: &mut HashSet<String>) {
    match place {
        MirPlace::Local { name, .. } => {
            locals.insert(name.clone());
        }
        MirPlace::Field { base, .. } | MirPlace::Index { base, .. } => {
            collect_assigned_place_local(base, locals);
        }
        MirPlace::Param { .. } | MirPlace::Deref { .. } | MirPlace::SliceIndex { .. } => {}
    }
}

fn remember_hoisted_target(instruction: &MirInstruction, hoisted_temps: &mut HashSet<String>) {
    if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
        hoisted_temps.insert(name.clone());
    }
}

fn analyze_natural_loops(function: &MirFunction) -> Vec<NaturalLoop> {
    if function.blocks.is_empty() {
        return Vec::new();
    }

    let labels = function
        .blocks
        .iter()
        .map(|block| block.label.clone())
        .collect::<Vec<_>>();
    let label_set = labels.iter().cloned().collect::<HashSet<_>>();
    let successors = build_successors(function);
    let predecessors = build_predecessors(&labels, &successors);
    let dominators = compute_dominators(&labels, &successors, &predecessors);
    let mut loops = Vec::new();

    for block in &function.blocks {
        for target in successors.get(&block.label).into_iter().flatten() {
            if !label_set.contains(target) {
                continue;
            }
            if !dominators
                .get(&block.label)
                .is_some_and(|doms| doms.contains(target))
            {
                continue;
            }

            let loop_blocks = collect_natural_loop_blocks(target, &block.label, &predecessors);
            if let Some(natural_loop) = describe_simple_loop(
                function,
                LoopBackEdge {
                    from: block.label.clone(),
                    to: target.clone(),
                },
                loop_blocks,
                &predecessors,
                &successors,
            ) {
                loops.push(natural_loop);
            }
        }
    }

    let order = block_order(function);
    loops.sort_by_key(|natural_loop| {
        order
            .get(&natural_loop.header)
            .copied()
            .unwrap_or(usize::MAX)
    });
    loops
}

fn describe_simple_loop(
    function: &MirFunction,
    back_edge: LoopBackEdge,
    blocks: HashSet<String>,
    predecessors: &HashMap<String, HashSet<String>>,
    successors: &HashMap<String, Vec<String>>,
) -> Option<NaturalLoop> {
    let header_block = function
        .blocks
        .iter()
        .find(|block| block.label == back_edge.to)?;
    if !matches!(header_block.terminator, MirTerminator::Branch { .. }) {
        return None;
    }

    let outside_header_predecessors = predecessors
        .get(&back_edge.to)
        .into_iter()
        .flatten()
        .filter(|label| !blocks.contains(*label))
        .cloned()
        .collect::<Vec<_>>();
    let [preheader] = outside_header_predecessors.as_slice() else {
        return None;
    };

    let preheader_block = function
        .blocks
        .iter()
        .find(|block| block.label == *preheader)?;
    if !matches!(
        &preheader_block.terminator,
        MirTerminator::Jump { label } if label == preheader_block_successor(&back_edge)
    ) {
        return None;
    }

    let mut exit_blocks = HashSet::new();
    for label in &blocks {
        for successor in successors.get(label).into_iter().flatten() {
            if !blocks.contains(successor) {
                exit_blocks.insert(successor.clone());
            }
        }
    }
    let order = block_order(function);
    let mut exit_blocks = exit_blocks.into_iter().collect::<Vec<_>>();
    exit_blocks.sort_by_key(|label| order.get(label).copied().unwrap_or(usize::MAX));

    Some(NaturalLoop {
        header: back_edge.to.clone(),
        back_edge,
        blocks,
        preheader: preheader.clone(),
        exit_blocks,
    })
}

fn preheader_block_successor(back_edge: &LoopBackEdge) -> &str {
    &back_edge.to
}

fn collect_natural_loop_blocks(
    header: &str,
    source: &str,
    predecessors: &HashMap<String, HashSet<String>>,
) -> HashSet<String> {
    let mut blocks = HashSet::from([header.to_string(), source.to_string()]);
    let mut worklist = vec![source.to_string()];
    while let Some(label) = worklist.pop() {
        for predecessor in predecessors.get(&label).into_iter().flatten() {
            if blocks.insert(predecessor.clone()) {
                worklist.push(predecessor.clone());
            }
        }
    }
    blocks
}

fn compute_dominators(
    labels: &[String],
    successors: &HashMap<String, Vec<String>>,
    predecessors: &HashMap<String, HashSet<String>>,
) -> HashMap<String, HashSet<String>> {
    let Some(entry) = labels.first() else {
        return HashMap::new();
    };
    let all_labels = labels.iter().cloned().collect::<HashSet<_>>();
    let mut dominators = HashMap::new();
    for label in labels {
        dominators.insert(
            label.clone(),
            if label == entry {
                HashSet::from([entry.clone()])
            } else {
                all_labels.clone()
            },
        );
    }

    let mut changed = true;
    while changed {
        changed = false;
        for label in labels.iter().skip(1) {
            let preds = predecessors
                .get(label)
                .into_iter()
                .flatten()
                .filter(|pred| dominators.contains_key(*pred))
                .collect::<Vec<_>>();
            let mut next = all_labels.clone();
            for pred in preds {
                if let Some(pred_dominators) = dominators.get(pred) {
                    next.retain(|entry| pred_dominators.contains(entry));
                }
            }
            next.insert(label.clone());
            if dominators.get(label) != Some(&next) {
                dominators.insert(label.clone(), next);
                changed = true;
            }
        }
    }

    for (label, successor_list) in successors {
        dominators
            .entry(label.clone())
            .or_insert_with(|| successor_list.iter().cloned().collect());
    }
    dominators
}

fn build_successors(function: &MirFunction) -> HashMap<String, Vec<String>> {
    function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), terminator_targets(&block.terminator)))
        .collect()
}

fn build_predecessors(
    labels: &[String],
    successors: &HashMap<String, Vec<String>>,
) -> HashMap<String, HashSet<String>> {
    let mut predecessors = labels
        .iter()
        .map(|label| (label.clone(), HashSet::new()))
        .collect::<HashMap<_, _>>();
    for (label, successor_list) in successors {
        for successor in successor_list {
            if let Some(preds) = predecessors.get_mut(successor) {
                preds.insert(label.clone());
            }
        }
    }
    predecessors
}

fn block_order(function: &MirFunction) -> HashMap<String, usize> {
    function
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.label.clone(), index))
        .collect()
}
