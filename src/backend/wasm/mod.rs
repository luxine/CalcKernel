mod binary;
mod emit;
mod layout;
mod plan;

pub use binary::{emit_wasm_module, emit_wasm_module_with_options};
pub use emit::{emit_wat_module, emit_wat_module_with_options};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmitWasmOptions {
    pub opt_level: u8,
}
