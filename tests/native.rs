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
#[path = "native/pgo_layout.rs"]
mod pgo_layout;
#[path = "native/pgo_o3.rs"]
mod pgo_o3;
#[path = "native/profile.rs"]
mod profile;
#[path = "native/profile_generation.rs"]
mod profile_generation;
#[path = "native/run.rs"]
mod run;
#[path = "native/runtime.rs"]
mod runtime;
#[path = "native/runtime_support.rs"]
mod runtime_support;
#[path = "native/static_prefix.rs"]
mod static_prefix;
#[path = "support/mod.rs"]
mod support;
#[path = "native/vector_llvm.rs"]
mod vector_llvm;

#[path = "support/generated.rs"]
mod generated;
