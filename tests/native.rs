#![cfg(feature = "native-toolchain")]

#[path = "native/abi.rs"]
mod abi;
#[path = "native/artifacts.rs"]
mod artifacts;
#[path = "native/bridge.rs"]
mod bridge;
#[path = "native/differential.rs"]
mod differential;
#[path = "native/executable.rs"]
mod executable;
#[path = "native/libraries.rs"]
mod libraries;
#[path = "native/llvm_ir.rs"]
mod llvm_ir;
#[path = "native/object.rs"]
mod object;
#[path = "native/ownership.rs"]
mod ownership;
#[path = "native/runtime.rs"]
mod runtime;
#[path = "native/runtime_support.rs"]
mod runtime_support;
#[path = "support/mod.rs"]
mod support;
