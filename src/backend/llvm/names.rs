use std::path::Path;

use crate::MirFunction;

pub(super) fn llvm_source_file_name(source_file_name: Option<&str>) -> String {
    source_file_name
        .and_then(|source_file_name| Path::new(source_file_name).file_name())
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("input.ck")
        .to_string()
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
