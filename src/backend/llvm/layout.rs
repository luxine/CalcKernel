use crate::*;

#[derive(Debug, Clone)]
pub(super) struct LlvmStructLayout {
    fields: std::collections::HashMap<String, std::collections::HashMap<String, usize>>,
}

impl LlvmStructLayout {
    pub(super) fn new(module: &MirModule) -> Self {
        let fields = module
            .structs
            .iter()
            .map(|struct_info| {
                (
                    struct_info.name.clone(),
                    struct_info
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| (field.name.clone(), index))
                        .collect(),
                )
            })
            .collect();
        Self { fields }
    }

    pub(super) fn field_index(&self, struct_name: &str, field_name: &str) -> usize {
        self.fields
            .get(struct_name)
            .and_then(|fields| fields.get(field_name))
            .copied()
            .unwrap_or_else(|| panic!("unknown LLVM struct field {struct_name}.{field_name}"))
    }
}
