use std::path::Path;

pub(super) fn llvm_source_file_name(source_file_name: Option<&str>) -> String {
    source_file_name
        .and_then(|source_file_name| Path::new(source_file_name).file_name())
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("input.ck")
        .to_string()
}
