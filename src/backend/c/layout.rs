use std::collections::HashSet;

use crate::{MirModule, MirPrimitiveTypeName, MirStruct, MirType};

use super::names::sanitize_c_identifier;

pub(super) fn c_generated_type_name(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => match name {
            MirPrimitiveTypeName::I32 => "i32".to_string(),
            MirPrimitiveTypeName::I64 => "i64".to_string(),
            MirPrimitiveTypeName::U32 => "u32".to_string(),
            MirPrimitiveTypeName::U64 => "u64".to_string(),
            MirPrimitiveTypeName::F64 => "f64".to_string(),
            MirPrimitiveTypeName::Bool => "bool".to_string(),
        },
        MirType::Pointer(element) => format!("ptr_{}", c_generated_type_name(element)),
        MirType::Slice(element) => format!("slice_{}", c_generated_type_name(element)),
        MirType::Struct(name) => sanitize_c_identifier(name),
        MirType::Void => "void".to_string(),
    }
}

pub(super) fn dependency_ordered_c_structs(module: &MirModule) -> Vec<&MirStruct> {
    let names = module
        .structs
        .iter()
        .map(|structure| structure.name.as_str())
        .collect::<HashSet<_>>();
    let mut emitted = HashSet::new();
    let mut ordered = Vec::new();
    while ordered.len() < module.structs.len() {
        let before = ordered.len();
        for structure in &module.structs {
            if emitted.contains(&structure.name) {
                continue;
            }
            let ready = structure.fields.iter().all(|field| match &field.type_node {
                MirType::Struct(name) if names.contains(name.as_str()) => emitted.contains(name),
                _ => true,
            });
            if ready {
                emitted.insert(structure.name.clone());
                ordered.push(structure);
            }
        }
        if ordered.len() == before {
            for structure in &module.structs {
                if emitted.insert(structure.name.clone()) {
                    ordered.push(structure);
                }
            }
        }
    }
    ordered
}
