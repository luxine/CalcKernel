mod binary;
mod emit;
mod kir;
mod layout;
mod plan;

pub(crate) use binary::emit_wasm_module_with_options;
pub(crate) use emit::emit_wat_module_with_options;
pub use kir::{emit_wasm_kir_module, emit_wat_kir_module};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmitWasmOptions {
    pub opt_level: u8,
}
