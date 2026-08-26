use std::collections::HashMap;

use crate::{MirBinaryOp, MirCompareOp, MirInstruction, MirModule, MirType, MirUnaryOp, MirValue};

use super::super::{analysis::*, pipeline::*};

#[derive(Debug, Clone, PartialEq, Eq)]
enum KnownConstant {
    Int { value: i128, type_node: MirType },
    Bool { value: bool, type_node: MirType },
}

pub(in crate::optimizer) fn run_constant_folding(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    if context.overflow_mode == MirPassOverflowMode::Checked {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let mut changed = false;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut constants = HashMap::new();
            for instruction in &mut block.instructions {
                if let Some(folded) = fold_instruction(instruction, &constants) {
                    *instruction = folded;
                    remember_instruction_constant(instruction, &mut constants);
                    changed = true;
                } else {
                    forget_instruction_target(instruction, &mut constants);
                    remember_instruction_constant(instruction, &mut constants);
                }
            }
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn fold_instruction(
    instruction: &MirInstruction,
    constants: &HashMap<String, KnownConstant>,
) -> Option<MirInstruction> {
    match instruction {
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            let left = get_known_constant(left, constants)?;
            let right = get_known_constant(right, constants)?;
            let (KnownConstant::Int { value: left, .. }, KnownConstant::Int { value: right, .. }) =
                (left, right)
            else {
                return None;
            };
            fold_binary(*op, left, right, value_type(target)).map(|value| {
                MirInstruction::ConstInt {
                    target: target.clone(),
                    value: value.to_string(),
                }
            })
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            let left = get_known_constant(left, constants)?;
            let right = get_known_constant(right, constants)?;
            let value = match (left, right) {
                (
                    KnownConstant::Int { value: left, .. },
                    KnownConstant::Int { value: right, .. },
                ) => fold_int_compare(*op, left, right),
                (
                    KnownConstant::Bool { value: left, .. },
                    KnownConstant::Bool { value: right, .. },
                ) => fold_bool_compare(*op, left, right),
                _ => None,
            }?;
            Some(MirInstruction::ConstBool {
                target: target.clone(),
                value,
            })
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => fold_unary(*op, get_known_constant(operand, constants)?, target),
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => None,
    }
}

fn remember_instruction_constant(
    instruction: &MirInstruction,
    constants: &mut HashMap<String, KnownConstant>,
) {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            if let (Some(name), Ok(value)) = (temp_name(target), value.parse::<i128>()) {
                constants.insert(
                    name.to_string(),
                    KnownConstant::Int {
                        value,
                        type_node: value_type(target).clone(),
                    },
                );
            }
        }
        MirInstruction::ConstBool { target, value } => {
            if let Some(name) = temp_name(target) {
                constants.insert(
                    name.to_string(),
                    KnownConstant::Bool {
                        value: *value,
                        type_node: value_type(target).clone(),
                    },
                );
            }
        }
        MirInstruction::ConstFloat { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Binary { .. }
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
        | MirInstruction::Call { .. } => {}
    }
}

fn forget_instruction_target(
    instruction: &MirInstruction,
    constants: &mut HashMap<String, KnownConstant>,
) {
    if let Some(target) = instruction_target(instruction)
        && let Some(name) = temp_name(target)
    {
        constants.remove(name);
    }
}

fn get_known_constant(
    value: &MirValue,
    constants: &HashMap<String, KnownConstant>,
) -> Option<KnownConstant> {
    match value {
        MirValue::ConstInt { text, type_node } => {
            text.parse::<i128>().ok().map(|value| KnownConstant::Int {
                value,
                type_node: type_node.clone(),
            })
        }
        MirValue::ConstBool { value, type_node } => Some(KnownConstant::Bool {
            value: *value,
            type_node: type_node.clone(),
        }),
        MirValue::Temp { name, .. } => constants.get(name).cloned(),
        MirValue::ConstFloat { .. } | MirValue::Param { .. } | MirValue::Local { .. } => None,
    }
}

fn fold_binary(op: MirBinaryOp, left: i128, right: i128, type_node: &MirType) -> Option<i128> {
    if !is_integer_type(type_node) {
        return None;
    }
    if matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod) && right == 0 {
        return None;
    }
    if matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod)
        && is_signed_integer_type(type_node)
        && left == integer_min(type_node)?
        && right == -1
    {
        return None;
    }

    let result = match op {
        MirBinaryOp::Add => left.checked_add(right)?,
        MirBinaryOp::Sub => left.checked_sub(right)?,
        MirBinaryOp::Mul => left.checked_mul(right)?,
        MirBinaryOp::Div => left.checked_div(right)?,
        MirBinaryOp::Mod => left.checked_rem(right)?,
    };

    fits_integer_type(result, type_node).then_some(result)
}

fn fold_unary(op: MirUnaryOp, operand: KnownConstant, target: &MirValue) -> Option<MirInstruction> {
    match op {
        MirUnaryOp::Not => match operand {
            KnownConstant::Bool { value, .. } => Some(MirInstruction::ConstBool {
                target: target.clone(),
                value: !value,
            }),
            KnownConstant::Int { .. } => None,
        },
        MirUnaryOp::Neg => {
            let KnownConstant::Int { value, .. } = operand else {
                return None;
            };
            if !is_integer_type(value_type(target)) {
                return None;
            }
            let value = value.checked_neg()?;
            fits_integer_type(value, value_type(target)).then(|| MirInstruction::ConstInt {
                target: target.clone(),
                value: value.to_string(),
            })
        }
    }
}

fn fold_int_compare(op: MirCompareOp, left: i128, right: i128) -> Option<bool> {
    Some(match op {
        MirCompareOp::Eq => left == right,
        MirCompareOp::Ne => left != right,
        MirCompareOp::Lt => left < right,
        MirCompareOp::Le => left <= right,
        MirCompareOp::Gt => left > right,
        MirCompareOp::Ge => left >= right,
    })
}

fn fold_bool_compare(op: MirCompareOp, left: bool, right: bool) -> Option<bool> {
    match op {
        MirCompareOp::Eq => Some(left == right),
        MirCompareOp::Ne => Some(left != right),
        MirCompareOp::Lt | MirCompareOp::Le | MirCompareOp::Gt | MirCompareOp::Ge => None,
    }
}
