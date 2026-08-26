use std::collections::{HashMap, HashSet};

use crate::{MirBlock, MirFunction, MirInstruction, MirModule, MirTerminator, MirValue};

use super::super::{analysis::*, pipeline::*};

pub(in crate::optimizer) fn run_cfg_simplify(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    let mut changed = false;
    for function in &mut module.functions {
        if context.opt_level >= 2 {
            changed |= simplify_constant_branches(function);
            changed |= simplify_jump_targets(function);
        }
        changed |= remove_unreachable_blocks(function);
        if context.opt_level >= 2 {
            changed |= simplify_jump_targets(function);
            changed |= remove_unreachable_blocks(function);
        }
    }
    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn simplify_constant_branches(function: &mut MirFunction) -> bool {
    let constants = collect_const_bool_temps(function);
    let mut changed = false;
    for block in &mut function.blocks {
        let MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } = &block.terminator
        else {
            continue;
        };
        let Some(condition) = get_known_bool(condition, &constants) else {
            continue;
        };
        block.terminator = MirTerminator::Jump {
            label: if condition {
                then_label.clone()
            } else {
                else_label.clone()
            },
        };
        changed = true;
    }
    changed
}

fn simplify_jump_targets(function: &mut MirFunction) -> bool {
    let mut changed = false;
    let blocks_by_label = function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), block.clone()))
        .collect::<HashMap<_, _>>();

    for block in &mut function.blocks {
        match &block.terminator {
            MirTerminator::Jump { label } => {
                let resolved = resolve_empty_jump_target(label, &blocks_by_label);
                if resolved != *label {
                    block.terminator = MirTerminator::Jump { label: resolved };
                    changed = true;
                }
            }
            MirTerminator::Branch {
                condition,
                then_label,
                else_label,
            } => {
                let resolved_then = resolve_empty_jump_target(then_label, &blocks_by_label);
                let resolved_else = resolve_empty_jump_target(else_label, &blocks_by_label);
                if resolved_then == resolved_else {
                    block.terminator = MirTerminator::Jump {
                        label: resolved_then,
                    };
                    changed = true;
                } else if resolved_then != *then_label || resolved_else != *else_label {
                    block.terminator = MirTerminator::Branch {
                        condition: condition.clone(),
                        then_label: resolved_then,
                        else_label: resolved_else,
                    };
                    changed = true;
                }
            }
            MirTerminator::Return { .. } => {}
        }
    }

    changed
}

fn remove_unreachable_blocks(function: &mut MirFunction) -> bool {
    if function.blocks.is_empty() {
        return false;
    }
    let reachable = collect_reachable_labels(function);
    let before = function.blocks.len();
    function
        .blocks
        .retain(|block| reachable.contains(&block.label));
    function.blocks.len() != before
}

fn collect_reachable_labels(function: &MirFunction) -> HashSet<String> {
    let blocks_by_label = function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), block))
        .collect::<HashMap<_, _>>();
    let mut reachable = HashSet::new();
    let mut worklist = vec![function.blocks[0].label.clone()];

    while let Some(label) = worklist.pop() {
        if !reachable.insert(label.clone()) {
            continue;
        }
        let Some(block) = blocks_by_label.get(&label) else {
            continue;
        };
        for target in terminator_targets(&block.terminator) {
            if !reachable.contains(&target) {
                worklist.push(target);
            }
        }
    }
    reachable
}

pub(super) fn terminator_targets(terminator: &MirTerminator) -> Vec<String> {
    match terminator {
        MirTerminator::Jump { label } => vec![label.clone()],
        MirTerminator::Branch {
            then_label,
            else_label,
            ..
        } => vec![then_label.clone(), else_label.clone()],
        MirTerminator::Return { .. } => Vec::new(),
    }
}

fn resolve_empty_jump_target(label: &str, blocks_by_label: &HashMap<String, MirBlock>) -> String {
    let mut current = label.to_string();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        let Some(block) = blocks_by_label.get(&current) else {
            return current;
        };
        let MirTerminator::Jump { label } = &block.terminator else {
            return current;
        };
        if !block.instructions.is_empty() {
            return current;
        }
        current.clone_from(label);
    }
    label.to_string()
}

fn collect_const_bool_temps(function: &MirFunction) -> HashMap<String, bool> {
    let mut constants = HashMap::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let MirInstruction::ConstBool { target, value } = instruction
                && let Some(name) = temp_name(target)
            {
                constants.insert(name.to_string(), *value);
            }
        }
    }
    constants
}

fn get_known_bool(value: &MirValue, constants: &HashMap<String, bool>) -> Option<bool> {
    match value {
        MirValue::ConstBool { value, .. } => Some(*value),
        MirValue::Temp { name, .. } => constants.get(name).copied(),
        MirValue::Param { .. }
        | MirValue::Local { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. } => None,
    }
}
