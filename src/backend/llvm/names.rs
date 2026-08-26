use std::{collections::HashSet, path::Path};

use crate::*;

use super::layout::LlvmStructLayout;

#[derive(Debug)]
pub(super) struct LlvmFunctionContext<'layout> {
    pub(in crate::backend) register_counter: usize,
    pub(in crate::backend) used_value_names: HashSet<String>,
    pub(in crate::backend) layout: &'layout LlvmStructLayout,
}

pub(super) fn llvm_next_register(context: &mut LlvmFunctionContext<'_>) -> String {
    loop {
        let name = format!("v{}", context.register_counter);
        context.register_counter += 1;
        if context.used_value_names.insert(name.clone()) {
            return format!("%{name}");
        }
    }
}

pub(super) fn llvm_address_for_value(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => llvm_address_name(name),
        MirValue::Temp { name, .. } => llvm_address_name(&llvm_storage_name_for_temp(name)),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("LLVM constants do not have storage")
        }
    }
}

pub(super) fn llvm_address_name(name: &str) -> String {
    format!("%{name}.addr")
}

pub(super) fn llvm_source_file_name(source_file_name: Option<&str>) -> String {
    source_file_name
        .and_then(|source_file_name| Path::new(source_file_name).file_name())
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("input.ck")
        .to_string()
}

pub(super) fn llvm_escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn llvm_block_label(function: &MirFunction, label: &str) -> String {
    if function
        .blocks
        .first()
        .is_some_and(|block| block.label == label)
    {
        "entry".to_string()
    } else {
        label.to_string()
    }
}

pub(super) fn llvm_storage_name_for_temp(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix('t')
        && !suffix.is_empty()
        && suffix.chars().all(|character| character.is_ascii_digit())
    {
        return format!("ik_tmp{suffix}");
    }

    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("ik_tmp_{sanitized}")
}
