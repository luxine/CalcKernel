//! Rust implementation of the CK / CalcKernel compiler.

mod backend;
mod frontend;
mod ir;
mod optimizer;
mod profile;
mod tune;

#[cfg(test)]
#[path = "../tests/ir/allocation_counter.rs"]
mod test_allocation_counter;

pub use backend::*;
pub use frontend::*;
pub use ir::*;
pub use optimizer::*;
pub use profile::*;
pub use tune::*;
