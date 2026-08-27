#![cfg(feature = "native-toolchain")]

#[path = "native/abi.rs"]
mod abi;
#[path = "native/artifacts.rs"]
mod artifacts;
#[path = "native/bridge.rs"]
mod bridge;
#[path = "native/differential.rs"]
mod differential;
#[path = "native/libraries.rs"]
mod libraries;
#[path = "native/llvm_ir.rs"]
mod llvm_ir;
#[path = "native/object.rs"]
mod object;
#[path = "native/ownership.rs"]
mod ownership;
#[path = "support/mod.rs"]
mod support;
