#[path = "support/mod.rs"]
mod support;

#[path = "backend/c.rs"]
mod c;
#[path = "backend/control_void_slice.rs"]
mod control_void_slice;
#[path = "backend/kir_c.rs"]
mod kir_c;
#[path = "backend/kir_wasm.rs"]
mod kir_wasm;
#[path = "backend/llvm.rs"]
mod llvm;
#[path = "backend/surface.rs"]
mod surface;
#[path = "backend/wasm.rs"]
mod wasm;
