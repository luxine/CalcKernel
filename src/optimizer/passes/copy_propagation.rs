use std::collections::{HashMap, HashSet};

use crate::{MirInstruction, MirModule, MirPlace, MirTerminator, MirValue, instruction_effect};

use super::super::{analysis::*, pipeline::*};

pub(in crate::optimizer) fn run_copy_propagation(
    module: &mut MirModule,
    _context: &MirPassContext,
) -> MirPassResult {
    let mut changed = false;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut copies = HashMap::new();
            for instruction in &mut block.instructions {
                changed |= rewrite_instruction_copies(instruction, &copies);

                if instruction_effect(instruction).invalidates_value_facts() {
                    copies.clear();
                    continue;
                }

                if let Some(target) = instruction_target(instruction)
                    && let Some(name) = temp_name(target)
                {
                    copies.remove(name);
                }

                if let MirInstruction::Move { target, value } = instruction
                    && let Some(name) = temp_name(target)
                {
                    copies.insert(name.to_string(), value.clone());
                }
            }
            changed |= rewrite_terminator_copies(&mut block.terminator, &copies);
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn rewrite_instruction_copies(
    instruction: &mut MirInstruction,
    copies: &HashMap<String, MirValue>,
) -> bool {
    let mut changed = false;
    match instruction {
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. } => false,
        MirInstruction::Move { value, .. } | MirInstruction::Cast { value, .. } => {
            changed |= rewrite_value_copy(value, copies);
            changed
        }
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            changed |= rewrite_value_copy(left, copies);
            changed |= rewrite_value_copy(right, copies);
            changed
        }
        MirInstruction::Unary { operand, .. } => {
            changed |= rewrite_value_copy(operand, copies);
            changed
        }
        MirInstruction::MakeSlice { data, len, .. } => {
            changed |= rewrite_value_copy(data, copies);
            changed |= rewrite_value_copy(len, copies);
            changed
        }
        MirInstruction::SliceData { slice, .. } | MirInstruction::SliceLen { slice, .. } => {
            rewrite_value_copy(slice, copies)
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            changed |= rewrite_value_copy(slice, copies);
            changed |= rewrite_value_copy(start, copies);
            changed |= rewrite_value_copy(end, copies);
            changed
        }
        MirInstruction::Address { place, .. } | MirInstruction::Load { place, .. } => {
            rewrite_place_copies(place, copies)
        }
        MirInstruction::Store { place, value } => {
            changed |= rewrite_place_copies(place, copies);
            changed |= rewrite_value_copy(value, copies);
            changed
        }
        MirInstruction::Call { args, .. } | MirInstruction::RuntimeCall { args, .. } => {
            for arg in args {
                changed |= rewrite_value_copy(arg, copies);
            }
            changed
        }
    }
}

fn rewrite_terminator_copies(
    terminator: &mut MirTerminator,
    copies: &HashMap<String, MirValue>,
) -> bool {
    match terminator {
        MirTerminator::Return { value } => value
            .as_mut()
            .is_some_and(|value| rewrite_value_copy(value, copies)),
        MirTerminator::Branch { condition, .. } => rewrite_value_copy(condition, copies),
        MirTerminator::Jump { .. } => false,
    }
}

fn rewrite_place_copies(place: &mut MirPlace, copies: &HashMap<String, MirValue>) -> bool {
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => false,
        MirPlace::Deref { pointer, .. } => rewrite_value_copy(pointer, copies),
        MirPlace::Field { base, .. } => rewrite_place_copies(base, copies),
        MirPlace::Index { base, index, .. } => {
            rewrite_place_copies(base, copies) | rewrite_value_copy(index, copies)
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            rewrite_value_copy(slice, copies) | rewrite_value_copy(index, copies)
        }
    }
}

fn rewrite_value_copy(value: &mut MirValue, copies: &HashMap<String, MirValue>) -> bool {
    let resolved = resolve_copy(value, copies);
    if *value == resolved {
        return false;
    }
    *value = resolved;
    true
}

fn resolve_copy(value: &MirValue, copies: &HashMap<String, MirValue>) -> MirValue {
    let mut current = value.clone();
    let mut seen = HashSet::new();
    while let MirValue::Temp { name, .. } = &current {
        if !seen.insert(name.clone()) {
            return current;
        }
        let Some(next) = copies.get(name) else {
            return current;
        };
        current = next.clone();
    }
    current
}
