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
