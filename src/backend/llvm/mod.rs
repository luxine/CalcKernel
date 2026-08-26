mod emit;
mod layout;
mod names;

pub use emit::emit_llvm_module;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitLlvmOptions {
    pub source_file_name: Option<String>,
    pub target_triple: Option<String>,
}
