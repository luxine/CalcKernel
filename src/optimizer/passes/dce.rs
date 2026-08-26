use std::collections::HashSet;

use crate::{MirFunction, MirInstruction, MirModule, MirPlace, MirTerminator, MirValue};

use super::super::{analysis::*, pipeline::*};

pub(in crate::optimizer) fn run_dead_code_elimination(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    let mut changed = false;
    for function in &mut module.functions {
        changed |= eliminate_dead_code_in_function(function, context);
    }
    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn eliminate_dead_code_in_function(function: &mut MirFunction, context: &MirPassContext) -> bool {
    let mut changed = false;
    let mut removed = true;
    while removed {
        removed = false;
        let used_temps = collect_used_temps(function);
        for block in &mut function.blocks {
            let before = block.instructions.len();
            block.instructions.retain(|instruction| {
                !is_removable_unused_instruction(instruction, &used_temps, context)
            });
            if block.instructions.len() != before {
                removed = true;
                changed = true;
            }
        }
    }
    changed
}

fn is_removable_unused_instruction(
    instruction: &MirInstruction,
    used_temps: &HashSet<String>,
    context: &MirPassContext,
) -> bool {
    if !is_pure_removable_instruction(instruction, context) {
        return false;
    }
    instruction_target(instruction)
        .and_then(temp_name)
        .is_some_and(|name| !used_temps.contains(name))
}

fn is_pure_removable_instruction(instruction: &MirInstruction, context: &MirPassContext) -> bool {
    if context.overflow_mode == MirPassOverflowMode::Checked
        && matches!(
            instruction,
            MirInstruction::Binary { .. } | MirInstruction::Unary { .. }
        )
    {
        return false;
    }
    if context.bounds_mode == MirPassBoundsMode::Checked
        && matches!(
            instruction,
            MirInstruction::Address { place, .. } if place_contains_slice_index(place)
        )
    {
        return false;
    }
    matches!(
        instruction,
        MirInstruction::ConstInt { .. }
            | MirInstruction::ConstFloat { .. }
            | MirInstruction::ConstBool { .. }
            | MirInstruction::Move { .. }
            | MirInstruction::Binary { .. }
            | MirInstruction::Unary { .. }
            | MirInstruction::Compare { .. }
            | MirInstruction::Cast { .. }
            | MirInstruction::Address { .. }
    )
}

fn place_contains_slice_index(place: &MirPlace) -> bool {
    match place {
        MirPlace::SliceIndex { .. } => true,
        MirPlace::Field { base, .. } | MirPlace::Index { base, .. } => {
            place_contains_slice_index(base)
        }
        MirPlace::Param { .. } | MirPlace::Local { .. } | MirPlace::Deref { .. } => false,
    }
}

fn collect_used_temps(function: &MirFunction) -> HashSet<String> {
    let mut used = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            collect_instruction_uses(instruction, &mut used);
        }
        collect_terminator_uses(&block.terminator, &mut used);
    }
    used
}

fn collect_instruction_uses(instruction: &MirInstruction, used: &mut HashSet<String>) {
    match instruction {
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. } => {}
        MirInstruction::Move { value, .. } | MirInstruction::Cast { value, .. } => {
            collect_value_use(value, used);
        }
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            collect_value_use(left, used);
            collect_value_use(right, used);
        }
        MirInstruction::Unary { operand, .. } => collect_value_use(operand, used),
        MirInstruction::MakeSlice { data, len, .. } => {
            collect_value_use(data, used);
            collect_value_use(len, used);
        }
        MirInstruction::SliceData { slice, .. } | MirInstruction::SliceLen { slice, .. } => {
            collect_value_use(slice, used);
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            collect_value_use(slice, used);
            collect_value_use(start, used);
            collect_value_use(end, used);
        }
        MirInstruction::Address { place, .. } | MirInstruction::Load { place, .. } => {
            collect_place_uses(place, used);
        }
        MirInstruction::Store { place, value } => {
            collect_place_uses(place, used);
            collect_value_use(value, used);
        }
        MirInstruction::Call { args, .. } => {
            for arg in args {
                collect_value_use(arg, used);
            }
        }
    }
}

fn collect_terminator_uses(terminator: &MirTerminator, used: &mut HashSet<String>) {
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                collect_value_use(value, used);
            }
        }
        MirTerminator::Branch { condition, .. } => collect_value_use(condition, used),
        MirTerminator::Jump { .. } => {}
    }
}

fn collect_place_uses(place: &MirPlace, used: &mut HashSet<String>) {
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => {}
        MirPlace::Deref { pointer, .. } => collect_value_use(pointer, used),
        MirPlace::Field { base, .. } => collect_place_uses(base, used),
        MirPlace::Index { base, index, .. } => {
            collect_place_uses(base, used);
            collect_value_use(index, used);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            collect_value_use(slice, used);
            collect_value_use(index, used);
        }
    }
}

fn collect_value_use(value: &MirValue, used: &mut HashSet<String>) {
    if let Some(name) = temp_name(value) {
        used.insert(name.to_string());
    }
}
