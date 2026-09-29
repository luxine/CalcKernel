#[path = "support/mod.rs"]
mod support;

#[path = "support/generated.rs"]
mod generated;

#[path = "backend/c.rs"]
mod c;
#[path = "backend/header_contracts.rs"]
mod header_contracts;
#[path = "backend/kir_c.rs"]
mod kir_c;
#[path = "backend/kir_wasm.rs"]
mod kir_wasm;
#[path = "backend/llvm.rs"]
mod llvm;
#[path = "backend/wasm.rs"]
mod wasm;
#[path = "backend/wasm_direct.rs"]
mod wasm_direct;
#[path = "backend/wasm_memory.rs"]
mod wasm_memory;
#[path = "backend/wasm_rotation.rs"]
mod wasm_rotation;

#[path = "backend/wasm_affine.rs"]
mod wasm_affine;

#[path = "backend/wasm_piecewise_select.rs"]
mod wasm_piecewise_select;
