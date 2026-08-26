use crate::*;

#[derive(Debug, Clone)]
pub(super) struct WasmFieldLayout {
    offset: usize,
}

#[derive(Debug, Clone)]
pub(super) struct WasmStructLayout {
    fields: std::collections::HashMap<String, std::collections::HashMap<String, WasmFieldLayout>>,
    sizes: std::collections::HashMap<String, usize>,
}

impl WasmStructLayout {
    pub(super) fn new(module: &MirModule) -> Self {
        let mut fields = std::collections::HashMap::new();
        let mut sizes = std::collections::HashMap::new();
        for struct_info in &module.structs {
            let mut offset = 0;
            let mut align = 1;
            let mut field_map = std::collections::HashMap::new();
            for field in &struct_info.fields {
                let field_align = wasm_align_of(&field.type_node, &sizes);
                let field_size = wasm_size_of(&field.type_node, &sizes);
                offset = align_to(offset, field_align);
                field_map.insert(field.name.clone(), WasmFieldLayout { offset });
                offset += field_size;
                align = align.max(field_align);
            }
            fields.insert(struct_info.name.clone(), field_map);
            sizes.insert(struct_info.name.clone(), align_to(offset, align));
        }
        Self { fields, sizes }
    }

    pub(super) fn field_offset(&self, struct_name: &str, field_name: &str) -> usize {
        self.fields
            .get(struct_name)
            .and_then(|fields| fields.get(field_name))
            .map(|field| field.offset)
            .unwrap_or_else(|| panic!("unknown WASM struct field {struct_name}.{field_name}"))
    }

    pub(super) fn size_of(&self, type_node: &MirType) -> usize {
        wasm_size_of(type_node, &self.sizes)
    }

    pub(super) fn align_of(&self, type_node: &MirType) -> usize {
        wasm_align_of(type_node, &self.sizes)
    }
}

pub(super) fn wasm_size_of(
    type_node: &MirType,
    struct_sizes: &std::collections::HashMap<String, usize>,
) -> usize {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => 4,
        MirType::Slice(_) => 8,
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => 8,
        MirType::Struct(name) => *struct_sizes.get(name).unwrap_or(&0),
        MirType::Void => panic!("void has no WASM storage size"),
    }
}

pub(super) fn wasm_align_of(
    type_node: &MirType,
    struct_sizes: &std::collections::HashMap<String, usize>,
) -> usize {
    match type_node {
        MirType::Slice(_) => 4,
        MirType::Struct(name) => struct_sizes.get(name).copied().unwrap_or(1).clamp(1, 8),
        _ => wasm_size_of(type_node, struct_sizes).clamp(1, 8),
    }
}

pub(super) fn align_to(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }
    value.div_ceil(align) * align
}
