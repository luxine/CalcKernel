use std::collections::{HashMap, HashSet};

use crate::*;

use super::super::{collect_temps, instruction_target, place_type, value_type};

#[derive(Debug, Clone)]
pub(super) enum WasmPhysicalValue {
    Scalar(String),
    Slice { data: String, len: String },
}

#[derive(Debug)]
pub(super) struct WasmFunctionPlan {
    pub(in crate::backend) values: HashMap<String, WasmPhysicalValue>,
    pub(in crate::backend) block_local: String,
    pub(in crate::backend) return_scalar: String,
    pub(in crate::backend) return_data: String,
    pub(in crate::backend) return_len: String,
    pub(in crate::backend) address_local: String,
}

impl WasmFunctionPlan {
    pub(super) fn new(function: &MirFunction) -> Self {
        let mut names = WasmIdentifierAllocator::default();
        for param in &function.params {
            names.reserve(&param.name);
        }
        for local in &function.locals {
            names.reserve(&local.name);
        }
        for block in &function.blocks {
            names.reserve(&block.label);
        }

        let mut values = HashMap::new();
        for param in &function.params {
            values.insert(
                format!("param:{}", param.name),
                wasm_allocate_physical_value(&mut names, &param.name, &param.type_node),
            );
        }
        for local in &function.locals {
            values.insert(
                format!("local:{}", local.name),
                wasm_allocate_physical_value(&mut names, &local.name, &local.type_node),
            );
        }
        for (name, type_node) in collect_temps(function) {
            let physical = if matches!(type_node, MirType::Slice(_)) {
                wasm_allocate_physical_value(&mut names, &name, &type_node)
            } else {
                WasmPhysicalValue::Scalar(names.allocate(&name))
            };
            values.insert(format!("temp:{name}"), physical);
        }

        Self {
            values,
            block_local: names.allocate("ik_bb"),
            return_scalar: names.allocate("ik_ret"),
            return_data: names.allocate("ik_ret_data"),
            return_len: names.allocate("ik_ret_len"),
            address_local: names.allocate("ik_addr"),
        }
    }

    pub(super) fn value(&self, value: &MirValue) -> &WasmPhysicalValue {
        self.values
            .get(&wasm_value_identity(value))
            .expect("every named MIR value must have physical WASM names")
    }

    pub(super) fn place_value(&self, place: &MirPlace) -> &WasmPhysicalValue {
        let key = match place {
            MirPlace::Param { name, .. } => format!("param:{name}"),
            MirPlace::Local { name, .. } => format!("local:{name}"),
            _ => panic!("only param and local places have direct WASM local names"),
        };
        self.values
            .get(&key)
            .expect("every named MIR place must have physical WASM names")
    }

    pub(super) fn scalar(&self, value: &MirValue) -> &str {
        match self.value(value) {
            WasmPhysicalValue::Scalar(name) => name,
            WasmPhysicalValue::Slice { .. } => panic!("slice is not a scalar WASM value"),
        }
    }

    pub(super) fn slice(&self, value: &MirValue) -> (&str, &str) {
        match self.value(value) {
            WasmPhysicalValue::Slice { data, len } => (data, len),
            WasmPhysicalValue::Scalar(_) => panic!("scalar is not a paired WASM slice value"),
        }
    }
}

pub(super) fn wasm_allocate_physical_value(
    names: &mut WasmIdentifierAllocator,
    logical_name: &str,
    type_node: &MirType,
) -> WasmPhysicalValue {
    if matches!(type_node, MirType::Slice(_)) {
        WasmPhysicalValue::Slice {
            data: names.allocate(&format!("{logical_name}_data")),
            len: names.allocate(&format!("{logical_name}_len")),
        }
    } else {
        WasmPhysicalValue::Scalar(logical_name.to_string())
    }
}

pub(super) fn wasm_function_uses_slices(function: &MirFunction) -> bool {
    let mut slices = HashSet::new();
    for param in &function.params {
        collect_slice_types_from_type(&param.type_node, &mut slices);
    }
    collect_slice_types_from_type(&function.return_type, &mut slices);
    for local in &function.locals {
        collect_slice_types_from_type(&local.type_node, &mut slices);
    }
    for block in &function.blocks {
        for instruction in &block.instructions {
            collect_slice_types_from_instruction(instruction, &mut slices);
        }
    }
    !slices.is_empty()
}

pub(super) fn collect_wasm_function_names(function: &MirFunction) -> HashSet<String> {
    let mut names = HashSet::new();
    for param in &function.params {
        names.insert(param.name.clone());
    }
    for local in &function.locals {
        names.insert(local.name.clone());
    }
    for block in &function.blocks {
        names.insert(block.label.clone());
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
                names.insert(name.clone());
            }
        }
    }
    names
}

pub(super) fn unique_wasm_internal_name(
    base_name: &str,
    used_names: &mut HashSet<String>,
) -> String {
    if used_names.insert(base_name.to_string()) {
        return base_name.to_string();
    }

    for index in 0.. {
        let candidate = format!("{base_name}{index}");
        if used_names.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded internal name search should always find a name")
}

#[derive(Debug, Default)]
pub(super) struct WasmIdentifierAllocator {
    used: HashSet<String>,
}

impl WasmIdentifierAllocator {
    pub(super) fn reserve(&mut self, name: &str) {
        self.used.insert(name.to_string());
    }

    pub(super) fn allocate(&mut self, preferred: &str) -> String {
        if self.used.insert(preferred.to_string()) {
            return preferred.to_string();
        }
        let mut suffix = 1_u32;
        loop {
            let candidate = format!("{preferred}_{suffix}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

fn wasm_value_identity(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } => format!("param:{name}"),
        MirValue::Local { name, .. } => format!("local:{name}"),
        MirValue::Temp { name, .. } => format!("temp:{name}"),
        MirValue::ConstInt { text, type_node } => {
            format!("const_int:{text}:{}", wasm_type_identity(type_node))
        }
        MirValue::ConstFloat { text, type_node } => {
            format!("const_float:{text}:{}", wasm_type_identity(type_node))
        }
        MirValue::ConstBool { value, .. } => format!("const_bool:{value}"),
    }
}

fn wasm_type_identity(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => match name {
            MirPrimitiveTypeName::I32 => "i32".to_string(),
            MirPrimitiveTypeName::I64 => "i64".to_string(),
            MirPrimitiveTypeName::U32 => "u32".to_string(),
            MirPrimitiveTypeName::U64 => "u64".to_string(),
            MirPrimitiveTypeName::F64 => "f64".to_string(),
            MirPrimitiveTypeName::Bool => "bool".to_string(),
        },
        MirType::Pointer(element_type) => format!("ptr<{}>", wasm_type_identity(element_type)),
        MirType::Slice(element_type) => format!("slice<{}>", wasm_type_identity(element_type)),
        MirType::Struct(name) => format!("struct:{name}"),
        MirType::Void => "void".to_string(),
    }
}

fn collect_slice_types_from_type(type_node: &MirType, slices: &mut HashSet<MirType>) {
    match type_node {
        MirType::Pointer(element_type) => collect_slice_types_from_type(element_type, slices),
        MirType::Slice(element_type) => {
            slices.insert(type_node.clone());
            collect_slice_types_from_type(element_type, slices);
        }
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => {}
    }
}

fn collect_slice_types_from_value(value: &MirValue, slices: &mut HashSet<MirType>) {
    collect_slice_types_from_type(value_type(value), slices);
}

fn collect_slice_types_from_place(place: &MirPlace, slices: &mut HashSet<MirType>) {
    collect_slice_types_from_type(place_type(place), slices);
    match place {
        MirPlace::Deref { pointer, .. } => collect_slice_types_from_value(pointer, slices),
        MirPlace::Index { base, index, .. } => {
            collect_slice_types_from_place(base, slices);
            collect_slice_types_from_value(index, slices);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            collect_slice_types_from_value(slice, slices);
            collect_slice_types_from_value(index, slices);
        }
        MirPlace::Field { base, .. } => collect_slice_types_from_place(base, slices),
        MirPlace::Param { .. } | MirPlace::Local { .. } => {}
    }
}

fn collect_slice_types_from_instruction(
    instruction: &MirInstruction,
    slices: &mut HashSet<MirType>,
) {
    if let Some(target) = instruction_target(instruction) {
        collect_slice_types_from_value(target, slices);
    }
    match instruction {
        MirInstruction::Move { value, .. }
        | MirInstruction::Unary { operand: value, .. }
        | MirInstruction::Cast { value, .. }
        | MirInstruction::SliceData { slice: value, .. }
        | MirInstruction::SliceLen { slice: value, .. } => {
            collect_slice_types_from_value(value, slices);
        }
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            collect_slice_types_from_value(left, slices);
            collect_slice_types_from_value(right, slices);
        }
        MirInstruction::Address { place, .. } | MirInstruction::Load { place, .. } => {
            collect_slice_types_from_place(place, slices);
        }
        MirInstruction::Store { place, value } => {
            collect_slice_types_from_place(place, slices);
            collect_slice_types_from_value(value, slices);
        }
        MirInstruction::MakeSlice { data, len, .. } => {
            collect_slice_types_from_value(data, slices);
            collect_slice_types_from_value(len, slices);
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            collect_slice_types_from_value(slice, slices);
            collect_slice_types_from_value(start, slices);
            collect_slice_types_from_value(end, slices);
        }
        MirInstruction::Call { args, .. } => {
            for arg in args {
                collect_slice_types_from_value(arg, slices);
            }
        }
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. } => {}
    }
}
