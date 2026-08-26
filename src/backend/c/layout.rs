use std::collections::{HashMap, HashSet};

use crate::*;

use super::super::{collect_temps, instruction_target, place_type, value_type};
use super::{
    checked::{c_primitive_type_identity, c_type_identity},
    emit::c_temp_name,
    names::*,
    options::*,
};

#[derive(Debug)]
pub(super) struct CModulePlan {
    pub(in crate::backend) slice_types: Vec<MirType>,
    pub(in crate::backend) slice_names: HashMap<MirType, String>,
    pub(in crate::backend) functions: HashMap<String, CFunctionPlan>,
    pub(in crate::backend) status_abi: bool,
}

#[derive(Debug)]
pub(super) struct CFunctionPlan {
    pub(in crate::backend) slice_params: HashMap<String, (String, String)>,
    pub(in crate::backend) temp_names: HashMap<String, String>,
    pub(in crate::backend) return_pointer: String,
    pub(in crate::backend) status_local: String,
}

impl CModulePlan {
    pub(super) fn new(module: &MirModule, options: EmitCOptions) -> Self {
        let slice_types = collect_module_slice_types(module);
        let mut global_names = CIdentifierAllocator::default();
        for name in [
            "CK_Status",
            "CK_OK",
            "CK_ERR_OVERFLOW",
            "CK_ERR_DIV_BY_ZERO",
            "CK_ERR_NULL_POINTER",
            "CK_ERR_OUT_OF_BOUNDS",
        ] {
            global_names.reserve(name);
        }
        for struct_info in &module.structs {
            global_names.reserve(&struct_info.name);
        }
        for function in &module.functions {
            global_names.reserve(&function.name);
        }

        let mut slice_names = HashMap::new();
        for slice_type in &slice_types {
            let MirType::Slice(element_type) = slice_type else {
                unreachable!("collected slice types must be slices");
            };
            let preferred = format!("CK_Slice_{}", c_generated_type_name(element_type));
            slice_names.insert(slice_type.clone(), global_names.allocate(&preferred));
        }

        let functions = module
            .functions
            .iter()
            .map(|function| {
                let mut names = CIdentifierAllocator::default();
                for param in &function.params {
                    names.reserve(&param.name);
                }
                for local in &function.locals {
                    names.reserve(&local.name);
                }

                let mut slice_params = HashMap::new();
                for param in &function.params {
                    if matches!(param.type_node, MirType::Slice(_)) {
                        let data = names.allocate(&format!("{}_data", param.name));
                        let len = names.allocate(&format!("{}_len", param.name));
                        slice_params.insert(param.name.clone(), (data, len));
                    }
                }

                let mut temp_names = HashMap::new();
                for (name, _) in collect_temps(function) {
                    let generated = names.allocate(&c_temp_name(&name));
                    temp_names.insert(name, generated);
                }

                let return_pointer = names.allocate("ck_return");
                let status_local = names.allocate("ik_status");
                (
                    function.name.clone(),
                    CFunctionPlan {
                        slice_params,
                        temp_names,
                        return_pointer,
                        status_local,
                    },
                )
            })
            .collect();

        Self {
            slice_types,
            slice_names,
            functions,
            status_abi: options.overflow_mode == OverflowMode::Checked
                || options.bounds_mode == BoundsMode::Checked,
        }
    }

    pub(super) fn function(&self, name: &str) -> &CFunctionPlan {
        self.functions
            .get(name)
            .expect("every MIR function must have a C function plan")
    }

    pub(super) fn type_name(&self, type_node: &MirType) -> String {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => "int32_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::I64) => "int64_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::U32) => "uint32_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::U64) => "uint64_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::Bool) => "bool".to_string(),
            MirType::Pointer(element_type) => format!("{}*", self.type_name(element_type)),
            MirType::Slice(_) => self
                .slice_names
                .get(type_node)
                .cloned()
                .expect("every reachable slice type must have a descriptor name"),
            MirType::Struct(name) => name.clone(),
            MirType::Void => "void".to_string(),
        }
    }
}

pub(super) fn use_planned_c_emitter(module: &MirModule, options: EmitCOptions) -> bool {
    options.bounds_mode == BoundsMode::Checked || !collect_module_slice_types(module).is_empty()
}

pub(super) fn c_generated_type_name(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => c_primitive_type_identity(*name).to_string(),
        MirType::Pointer(element_type) => {
            format!("ptr_{}", c_generated_type_name(element_type))
        }
        MirType::Slice(element_type) => {
            format!("slice_{}", c_generated_type_name(element_type))
        }
        MirType::Struct(name) => sanitize_c_identifier(name),
        MirType::Void => "void".to_string(),
    }
}

pub(super) fn collect_module_slice_types(module: &MirModule) -> Vec<MirType> {
    let mut slices = HashSet::new();
    for struct_info in &module.structs {
        for field in &struct_info.fields {
            collect_slice_types_from_type(&field.type_node, &mut slices);
        }
    }
    for function in &module.functions {
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
            match &block.terminator {
                MirTerminator::Return { value } => {
                    if let Some(value) = value {
                        collect_slice_types_from_value(value, &mut slices);
                    }
                }
                MirTerminator::Branch { condition, .. } => {
                    collect_slice_types_from_value(condition, &mut slices);
                }
                MirTerminator::Jump { .. } => {}
            }
        }
    }
    let mut slices = slices.into_iter().collect::<Vec<_>>();
    slices.sort_by_key(c_type_identity);
    slices
}

pub(super) fn collect_slice_types_from_type(type_node: &MirType, slices: &mut HashSet<MirType>) {
    match type_node {
        MirType::Pointer(element_type) => collect_slice_types_from_type(element_type, slices),
        MirType::Slice(element_type) => {
            slices.insert(type_node.clone());
            collect_slice_types_from_type(element_type, slices);
        }
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => {}
    }
}

pub(super) fn collect_slice_types_from_value(value: &MirValue, slices: &mut HashSet<MirType>) {
    collect_slice_types_from_type(value_type(value), slices);
}

pub(super) fn collect_slice_types_from_place(place: &MirPlace, slices: &mut HashSet<MirType>) {
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

pub(super) fn collect_slice_types_from_instruction(
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

pub(super) fn emit_planned_type_declarations(
    out: &mut String,
    module: &MirModule,
    plan: &CModulePlan,
) {
    for struct_info in &module.structs {
        out.push_str(&format!("typedef struct {0} {0};\n", struct_info.name));
    }
    if !module.structs.is_empty() {
        out.push('\n');
    }

    for slice_type in &plan.slice_types {
        let MirType::Slice(element_type) = slice_type else {
            unreachable!();
        };
        let name = plan.type_name(slice_type);
        out.push_str(&format!(
            "typedef struct {name} {{\n  {}* data;\n  uint32_t len;\n}} {name};\n\n",
            plan.type_name(element_type)
        ));
    }

    for struct_info in dependency_ordered_c_structs(module) {
        out.push_str(&format!("struct {} {{\n", struct_info.name));
        for field in &struct_info.fields {
            out.push_str(&format!(
                "  {} {};\n",
                plan.type_name(&field.type_node),
                field.name
            ));
        }
        out.push_str("};\n\n");
    }
}

pub(super) fn dependency_ordered_c_structs(module: &MirModule) -> Vec<&crate::MirStruct> {
    let names = module
        .structs
        .iter()
        .map(|struct_info| struct_info.name.as_str())
        .collect::<HashSet<_>>();
    let mut emitted = HashSet::new();
    let mut ordered = Vec::new();
    while ordered.len() < module.structs.len() {
        let before = ordered.len();
        for struct_info in &module.structs {
            if emitted.contains(&struct_info.name) {
                continue;
            }
            let dependencies_ready =
                struct_info
                    .fields
                    .iter()
                    .all(|field| match &field.type_node {
                        MirType::Struct(name) if names.contains(name.as_str()) => {
                            emitted.contains(name)
                        }
                        _ => true,
                    });
            if dependencies_ready {
                emitted.insert(struct_info.name.clone());
                ordered.push(struct_info);
            }
        }
        if ordered.len() == before {
            for struct_info in &module.structs {
                if emitted.insert(struct_info.name.clone()) {
                    ordered.push(struct_info);
                }
            }
        }
    }
    ordered
}

pub(super) struct PlannedCFunctionContext<'a> {
    pub(in crate::backend) plan: &'a CModulePlan,
    pub(in crate::backend) function_plan: &'a CFunctionPlan,
    pub(in crate::backend) options: EmitCOptions,
}

impl PlannedCFunctionContext<'_> {
    pub(super) fn value(&self, value: &MirValue) -> String {
        match value {
            MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
            MirValue::Temp { name, .. } => self
                .function_plan
                .temp_names
                .get(name)
                .cloned()
                .expect("every MIR temp must have a planned C name"),
            MirValue::ConstInt { text, .. } | MirValue::ConstFloat { text, .. } => text.clone(),
            MirValue::ConstBool { value, .. } => if *value { "true" } else { "false" }.to_string(),
        }
    }

    pub(super) fn lvalue(&self, value: &MirValue) -> String {
        match value {
            MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
            MirValue::Temp { name, .. } => self
                .function_plan
                .temp_names
                .get(name)
                .cloned()
                .expect("every MIR temp must have a planned C name"),
            MirValue::ConstInt { .. }
            | MirValue::ConstFloat { .. }
            | MirValue::ConstBool { .. } => panic!("cannot assign to MIR constant"),
        }
    }

    pub(super) fn call_args(&self, args: &[MirValue]) -> Vec<String> {
        let mut physical = Vec::new();
        for arg in args {
            let value = self.value(arg);
            if matches!(value_type(arg), MirType::Slice(_)) {
                physical.push(format!("{value}.data"));
                physical.push(format!("{value}.len"));
            } else {
                physical.push(value);
            }
        }
        physical
    }

    pub(super) fn place(&self, place: &MirPlace) -> CPlaceEmission {
        match place {
            MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => CPlaceEmission {
                preludes: Vec::new(),
                expression: name.clone(),
            },
            MirPlace::Deref { pointer, .. } => CPlaceEmission {
                preludes: Vec::new(),
                expression: format!("(*{})", self.value(pointer)),
            },
            MirPlace::Index { base, index, .. } => {
                let mut emitted = self.place(base);
                emitted.expression = format!("{}[{}]", emitted.expression, self.value(index));
                emitted
            }
            MirPlace::SliceIndex { slice, index, .. } => {
                let slice = self.value(slice);
                let index = self.value(index);
                let preludes = if self.options.bounds_mode == BoundsMode::Checked {
                    vec![
                        format!("if ({index} >= {slice}.len) {{"),
                        "  return CK_ERR_OUT_OF_BOUNDS;".to_string(),
                        "}".to_string(),
                    ]
                } else {
                    Vec::new()
                };
                CPlaceEmission {
                    preludes,
                    expression: format!("{slice}.data[{index}]"),
                }
            }
            MirPlace::Field {
                base, field_name, ..
            } => {
                let mut emitted = self.place(base);
                emitted.expression = if let MirPlace::Deref { pointer, .. } = &**base {
                    format!("{}->{field_name}", self.value(pointer))
                } else {
                    format!("{}.{field_name}", emitted.expression)
                };
                emitted
            }
        }
    }
}

pub(super) struct CPlaceEmission {
    pub(in crate::backend) preludes: Vec<String>,
    pub(in crate::backend) expression: String,
}
