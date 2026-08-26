use std::collections::{HashMap, HashSet};

use crate::{
    MirBinaryOp, MirCompareOp, MirFunction, MirInstruction, MirModule, MirPlace,
    MirPrimitiveTypeName, MirType, MirUnaryOp, MirValue,
};

use super::super::{analysis::*, pipeline::*};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CseEntry {
    value: MirValue,
    dependencies: HashSet<String>,
}

pub(in crate::optimizer) fn run_local_cse(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    if context.bounds_mode == MirPassBoundsMode::Checked {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }
    let mut changed = false;

    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut expressions: HashMap<String, CseEntry> = HashMap::new();

            for instruction in &mut block.instructions {
                if matches!(
                    instruction,
                    MirInstruction::Store { .. } | MirInstruction::Call { .. }
                ) {
                    expressions.clear();
                }

                let key = cse_key(instruction);
                let target = instruction_target(instruction).cloned();
                if let (Some(key), Some(target @ MirValue::Temp { .. })) = (key, target.clone()) {
                    if let Some(existing) = expressions.get(&key) {
                        *instruction = MirInstruction::Move {
                            target,
                            value: existing.value.clone(),
                        };
                        changed = true;
                    } else {
                        expressions.insert(
                            key,
                            CseEntry {
                                value: target,
                                dependencies: collect_instruction_dependencies(instruction),
                            },
                        );
                    }
                }

                if let Some(target) = target {
                    match target {
                        MirValue::Local { name, .. } => {
                            invalidate_cse_dependency(&mut expressions, &format!("local:{name}"));
                        }
                        MirValue::Param { name, .. } => {
                            invalidate_cse_dependency(&mut expressions, &format!("param:{name}"));
                        }
                        MirValue::Temp { .. }
                        | MirValue::ConstInt { .. }
                        | MirValue::ConstFloat { .. }
                        | MirValue::ConstBool { .. } => {}
                    }
                }
            }
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn cse_key(instruction: &MirInstruction) -> Option<String> {
    match instruction {
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            if is_f64_type(value_type(target)) {
                return float_binary_cse_key(*op, target, left, right);
            }
            let (left_key, right_key) = ordered_value_keys(*op, left, right);
            Some(format!(
                "binary:{}:{}:{left_key}:{right_key}",
                binary_op_key(*op),
                type_key(value_type(target))
            ))
        }
        MirInstruction::Compare {
            target: _,
            op,
            left,
            right,
        } => {
            if is_f64_type(value_type(left)) || is_f64_type(value_type(right)) {
                return None;
            }
            let (left_key, right_key) = ordered_compare_value_keys(*op, left, right);
            Some(format!(
                "compare:{}:{}:{left_key}:{right_key}",
                compare_op_key(*op),
                type_key(value_type(left))
            ))
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            if is_f64_type(value_type(target)) || is_f64_type(value_type(operand)) {
                return float_unary_cse_key(*op, target, operand);
            }
            Some(format!(
                "unary:{}:{}:{}",
                unary_op_key(*op),
                type_key(value_type(target)),
                value_key(operand)
            ))
        }
        MirInstruction::Cast { target, op, value } => Some(format!(
            "cast:{}:{}:{}:{}",
            cast_op_key(*op),
            type_key(value_type(value)),
            type_key(value_type(target)),
            value_key(value)
        )),
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
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

fn float_binary_cse_key(
    op: MirBinaryOp,
    target: &MirValue,
    left: &MirValue,
    right: &MirValue,
) -> Option<String> {
    if !is_f64_type(value_type(target))
        || !is_f64_type(value_type(left))
        || !is_f64_type(value_type(right))
    {
        return None;
    }
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul) {
        return None;
    }
    Some(format!(
        "float-binary:{}:{}:{}:{}",
        binary_op_key(op),
        type_key(value_type(target)),
        value_key(left),
        value_key(right)
    ))
}

fn float_unary_cse_key(op: MirUnaryOp, target: &MirValue, operand: &MirValue) -> Option<String> {
    if !is_f64_type(value_type(target))
        || !is_f64_type(value_type(operand))
        || op != MirUnaryOp::Neg
    {
        return None;
    }
    Some(format!(
        "float-unary:{}:{}:{}",
        unary_op_key(op),
        type_key(value_type(target)),
        value_key(operand)
    ))
}

fn ordered_value_keys(op: MirBinaryOp, left: &MirValue, right: &MirValue) -> (String, String) {
    let left_key = value_key(left);
    let right_key = value_key(right);
    if matches!(op, MirBinaryOp::Add | MirBinaryOp::Mul) && right_key < left_key {
        (right_key, left_key)
    } else {
        (left_key, right_key)
    }
}

fn ordered_compare_value_keys(
    op: MirCompareOp,
    left: &MirValue,
    right: &MirValue,
) -> (String, String) {
    let left_key = value_key(left);
    let right_key = value_key(right);
    if matches!(op, MirCompareOp::Eq | MirCompareOp::Ne) && right_key < left_key {
        (right_key, left_key)
    } else {
        (left_key, right_key)
    }
}

fn collect_instruction_dependencies(instruction: &MirInstruction) -> HashSet<String> {
    let mut dependencies = HashSet::new();
    match instruction {
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            collect_value_dependency(left, &mut dependencies);
            collect_value_dependency(right, &mut dependencies);
        }
        MirInstruction::Unary { operand, .. } => {
            collect_value_dependency(operand, &mut dependencies)
        }
        MirInstruction::Cast { value, .. } => collect_value_dependency(value, &mut dependencies),
        MirInstruction::MakeSlice { data, len, .. } => {
            collect_value_dependency(data, &mut dependencies);
            collect_value_dependency(len, &mut dependencies);
        }
        MirInstruction::SliceData { slice, .. } | MirInstruction::SliceLen { slice, .. } => {
            collect_value_dependency(slice, &mut dependencies);
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            collect_value_dependency(slice, &mut dependencies);
            collect_value_dependency(start, &mut dependencies);
            collect_value_dependency(end, &mut dependencies);
        }
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::Call { .. } => {}
    }
    dependencies
}

fn collect_value_dependency(value: &MirValue, dependencies: &mut HashSet<String>) {
    match value {
        MirValue::Local { name, .. } => {
            dependencies.insert(format!("local:{name}"));
        }
        MirValue::Param { name, .. } => {
            dependencies.insert(format!("param:{name}"));
        }
        MirValue::Temp { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::ConstBool { .. } => {}
    }
}

fn invalidate_cse_dependency(expressions: &mut HashMap<String, CseEntry>, dependency: &str) {
    expressions.retain(|_, entry| !entry.dependencies.contains(dependency));
}

fn value_key(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, type_node } => format!("param:{name}:{}", type_key(type_node)),
        MirValue::Local { name, type_node } => format!("local:{name}:{}", type_key(type_node)),
        MirValue::Temp { name, type_node } => format!("temp:{name}:{}", type_key(type_node)),
        MirValue::ConstInt { text, type_node } => {
            format!("const_int:{text}:{}", type_key(type_node))
        }
        MirValue::ConstFloat { text, type_node } => {
            format!("const_float:{text}:{}", type_key(type_node))
        }
        MirValue::ConstBool { value, .. } => format!("const_bool:{value}"),
    }
}

fn type_key(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => primitive_type_key(*name).to_string(),
        MirType::Pointer(element_type) => format!("ptr<{}>", type_key(element_type)),
        MirType::Slice(element_type) => format!("slice<{}>", type_key(element_type)),
        MirType::Struct(name) => format!("struct:{name}"),
        MirType::Void => "void".to_string(),
    }
}

fn primitive_type_key(name: MirPrimitiveTypeName) -> &'static str {
    match name {
        MirPrimitiveTypeName::I32 => "i32",
        MirPrimitiveTypeName::I64 => "i64",
        MirPrimitiveTypeName::U32 => "u32",
        MirPrimitiveTypeName::U64 => "u64",
        MirPrimitiveTypeName::F64 => "f64",
        MirPrimitiveTypeName::Bool => "bool",
    }
}

fn binary_op_key(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "+",
        MirBinaryOp::Sub => "-",
        MirBinaryOp::Mul => "*",
        MirBinaryOp::Div => "/",
        MirBinaryOp::Mod => "%",
    }
}

fn compare_op_key(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "==",
        MirCompareOp::Ne => "!=",
        MirCompareOp::Lt => "<",
        MirCompareOp::Le => "<=",
        MirCompareOp::Gt => ">",
        MirCompareOp::Ge => ">=",
    }
}

fn unary_op_key(op: MirUnaryOp) -> &'static str {
    match op {
        MirUnaryOp::Neg => "neg",
        MirUnaryOp::Not => "not",
    }
}

fn cast_op_key(op: crate::MirCastOp) -> &'static str {
    match op {
        crate::MirCastOp::I32ToF64 => "i32_to_f64",
        crate::MirCastOp::U32ToF64 => "u32_to_f64",
    }
}

struct AddressEntry {
    pointer: MirValue,
    dependencies: HashSet<String>,
}

pub(in crate::optimizer) fn run_address_cse(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    if context.bounds_mode == MirPassBoundsMode::Checked
        || !matches!(
            context.target_backend,
            MirPassTargetBackend::C | MirPassTargetBackend::Wasm
        )
    {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let mut changed = false;
    for function in &mut module.functions {
        let mut allocator = AddressTempAllocator::new(function);

        for block in &mut function.blocks {
            let mut addresses: HashMap<String, AddressEntry> = HashMap::new();
            let mut next_instructions = Vec::with_capacity(block.instructions.len());

            for instruction in std::mem::take(&mut block.instructions) {
                if matches!(instruction, MirInstruction::Call { .. }) {
                    addresses.clear();
                    next_instructions.push(instruction);
                    continue;
                }

                let original = instruction.clone();
                let mut inserted = Vec::new();
                let rewritten = rewrite_address_instruction(
                    instruction,
                    &mut addresses,
                    &mut allocator,
                    &mut inserted,
                );
                if !inserted.is_empty() || rewritten != original {
                    changed = true;
                }
                next_instructions.extend(inserted);

                let is_store = matches!(rewritten, MirInstruction::Store { .. });
                let target = instruction_target(&rewritten).cloned();
                next_instructions.push(rewritten);

                if is_store {
                    addresses.clear();
                    continue;
                }

                if let Some(target) = target {
                    match target {
                        MirValue::Local { name, .. } => {
                            invalidate_address_dependency(&mut addresses, &format!("local:{name}"));
                        }
                        MirValue::Param { name, .. } => {
                            invalidate_address_dependency(&mut addresses, &format!("param:{name}"));
                        }
                        MirValue::Temp { .. }
                        | MirValue::ConstInt { .. }
                        | MirValue::ConstFloat { .. }
                        | MirValue::ConstBool { .. } => {}
                    }
                }
            }

            block.instructions = next_instructions;
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn rewrite_address_instruction(
    instruction: MirInstruction,
    addresses: &mut HashMap<String, AddressEntry>,
    allocator: &mut AddressTempAllocator,
    inserted: &mut Vec<MirInstruction>,
) -> MirInstruction {
    match instruction {
        MirInstruction::Load { target, place } => MirInstruction::Load {
            target,
            place: rewrite_address_place(place, addresses, allocator, inserted),
        },
        MirInstruction::Store { place, value } => MirInstruction::Store {
            place: rewrite_address_place(place, addresses, allocator, inserted),
            value,
        },
        MirInstruction::Address { target, place } => MirInstruction::Address {
            target,
            place: rewrite_address_place(place, addresses, allocator, inserted),
        },
        other => other,
    }
}

fn rewrite_address_place(
    place: MirPlace,
    addresses: &mut HashMap<String, AddressEntry>,
    allocator: &mut AddressTempAllocator,
    inserted: &mut Vec<MirInstruction>,
) -> MirPlace {
    match place {
        MirPlace::Field {
            base,
            field_name,
            type_node,
        } => {
            if is_indexed_struct_place(&base) {
                let base_type = place_type(&base).clone();
                let pointer = pointer_for_indexed_place(*base, addresses, allocator, inserted);
                return MirPlace::Field {
                    base: Box::new(MirPlace::Deref {
                        pointer,
                        type_node: base_type,
                    }),
                    field_name,
                    type_node,
                };
            }
            MirPlace::Field {
                base: Box::new(rewrite_address_place(*base, addresses, allocator, inserted)),
                field_name,
                type_node,
            }
        }
        MirPlace::Index {
            base,
            index,
            type_node,
        } => {
            let place = MirPlace::Index {
                base,
                index,
                type_node,
            };
            if should_materialize_indexed_place(&place) {
                let deref_type = place_type(&place).clone();
                let pointer = pointer_for_indexed_place(place, addresses, allocator, inserted);
                MirPlace::Deref {
                    pointer,
                    type_node: deref_type,
                }
            } else if let MirPlace::Index {
                base,
                index,
                type_node,
            } = place
            {
                MirPlace::Index {
                    base: Box::new(rewrite_address_place(*base, addresses, allocator, inserted)),
                    index,
                    type_node,
                }
            } else {
                unreachable!()
            }
        }
        MirPlace::SliceIndex { .. } => place,
        MirPlace::Deref { .. } | MirPlace::Param { .. } | MirPlace::Local { .. } => place,
    }
}

fn pointer_for_indexed_place(
    place: MirPlace,
    addresses: &mut HashMap<String, AddressEntry>,
    allocator: &mut AddressTempAllocator,
    inserted: &mut Vec<MirInstruction>,
) -> MirValue {
    let key = indexed_place_key(&place);
    if let Some(entry) = addresses.get(&key) {
        return entry.pointer.clone();
    }

    let pointer = allocator.next(place_type(&place).clone());
    inserted.push(MirInstruction::Address {
        target: pointer.clone(),
        place: place.clone(),
    });
    addresses.insert(
        key,
        AddressEntry {
            pointer: pointer.clone(),
            dependencies: collect_place_dependencies(&place),
        },
    );
    pointer
}

fn is_indexed_struct_place(place: &MirPlace) -> bool {
    matches!(
        place,
        MirPlace::Index {
            type_node: MirType::Struct(_),
            ..
        }
    )
}

fn should_materialize_indexed_place(place: &MirPlace) -> bool {
    matches!(place, MirPlace::Index { type_node, .. } if !matches!(type_node, MirType::Struct(_)))
}

fn indexed_place_key(place: &MirPlace) -> String {
    format!("indexed:{}", place_key(place))
}

#[derive(Debug, Clone)]
struct AddressTempAllocator {
    used: HashSet<String>,
    index: usize,
}

impl AddressTempAllocator {
    fn new(function: &MirFunction) -> Self {
        let mut used = HashSet::new();
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
                    used.insert(name.clone());
                }
            }
        }
        Self { used, index: 0 }
    }

    fn next(&mut self, element_type: MirType) -> MirValue {
        while self.used.contains(&format!("addr{}", self.index)) {
            self.index += 1;
        }
        let name = format!("addr{}", self.index);
        self.index += 1;
        self.used.insert(name.clone());
        MirValue::Temp {
            name,
            type_node: MirType::Pointer(Box::new(element_type)),
        }
    }
}

fn invalidate_address_dependency(
    expressions: &mut HashMap<String, AddressEntry>,
    dependency: &str,
) {
    expressions.retain(|_, entry| !entry.dependencies.contains(dependency));
}

fn collect_place_dependencies(place: &MirPlace) -> HashSet<String> {
    let mut dependencies = HashSet::new();
    collect_place_dependency(place, &mut dependencies);
    dependencies
}

fn collect_place_dependency(place: &MirPlace, dependencies: &mut HashSet<String>) {
    match place {
        MirPlace::Param { name, .. } => {
            dependencies.insert(format!("param:{name}"));
        }
        MirPlace::Local { name, .. } => {
            dependencies.insert(format!("local:{name}"));
        }
        MirPlace::Deref { pointer, .. } => collect_value_dependency(pointer, dependencies),
        MirPlace::Index { base, index, .. } => {
            collect_place_dependency(base, dependencies);
            collect_value_dependency(index, dependencies);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            collect_value_dependency(slice, dependencies);
            collect_value_dependency(index, dependencies);
        }
        MirPlace::Field { base, .. } => collect_place_dependency(base, dependencies),
    }
}

fn place_key(place: &MirPlace) -> String {
    match place {
        MirPlace::Param { name, type_node } => format!("param:{name}:{}", type_key(type_node)),
        MirPlace::Local { name, type_node } => format!("local:{name}:{}", type_key(type_node)),
        MirPlace::Deref { pointer, type_node } => {
            format!("deref:{}:{}", value_key(pointer), type_key(type_node))
        }
        MirPlace::Index {
            base,
            index,
            type_node,
        } => format!(
            "index:{}:{}:{}",
            place_key(base),
            value_key(index),
            type_key(type_node)
        ),
        MirPlace::SliceIndex {
            slice,
            index,
            type_node,
        } => format!(
            "slice_index:{}:{}:{}",
            value_key(slice),
            value_key(index),
            type_key(type_node)
        ),
        MirPlace::Field {
            base,
            field_name,
            type_node,
        } => format!(
            "field:{}:{field_name}:{}",
            place_key(base),
            type_key(type_node)
        ),
    }
}
