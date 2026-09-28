mod binary;
mod bulk;
mod control;
mod emit;
mod features;
mod final_ir;
mod ir;
mod kir;
mod layout;
mod lower;
mod memory;
mod placement;
mod plan;

pub use kir::{
    emit_wasm_kir_module, emit_wasm_kir_result, emit_wat_kir_module, emit_wat_kir_result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmitWasmOptions {
    pub opt_level: u8,
}
