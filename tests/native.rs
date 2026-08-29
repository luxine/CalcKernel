#![cfg(feature = "native-toolchain")]

#[path = "native/abi.rs"]
mod abi;
#[path = "native/artifacts.rs"]
mod artifacts;
#[path = "native/bridge.rs"]
mod bridge;
#[path = "native/cache.rs"]
mod cache;
#[path = "native/contract_sanitizer.rs"]
mod contract_sanitizer;
#[path = "native/differential.rs"]
mod differential;
#[path = "native/executable.rs"]
mod executable;
#[path = "native/fact_audit.rs"]
mod fact_audit;
#[path = "native/jit.rs"]
mod jit;
#[path = "native/libraries.rs"]
mod libraries;
#[path = "native/llvm_ir.rs"]
mod llvm_ir;
#[path = "native/object.rs"]
mod object;
#[path = "native/ownership.rs"]
mod ownership;
#[path = "native/run.rs"]
mod run;
#[path = "native/runtime.rs"]
mod runtime;
#[path = "native/runtime_support.rs"]
mod runtime_support;
#[path = "support/mod.rs"]
mod support;

#[path = "support/generated.rs"]
mod generated;
