#![cfg(feature = "native-toolchain")]

#[path = "native/bridge.rs"]
mod bridge;
#[path = "native/llvm_ir.rs"]
mod llvm_ir;
#[path = "native/object.rs"]
mod object;
#[path = "native/ownership.rs"]
mod ownership;
