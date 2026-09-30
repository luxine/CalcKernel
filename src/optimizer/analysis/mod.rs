mod affine;
mod alias;
mod budget;
mod congruence;
mod contract_range;
mod decision_tree;
mod dependence;
mod effects;
mod invariant_load;
mod known_bits;
mod loop_access;
mod loops;
mod memory_ssa;
mod regions;
mod runtime_unroll;
mod scalar;
mod slp;
mod specialization;
mod unroll;
mod vectorize;

pub use affine::*;
pub use alias::*;
pub use budget::*;
pub use congruence::*;
pub(crate) use contract_range::contract_scalar_interval;
pub use decision_tree::*;
pub use dependence::*;
pub use effects::*;
pub use invariant_load::*;
pub use known_bits::*;
pub use loop_access::*;
pub use loops::*;
pub use memory_ssa::*;
pub use regions::*;
pub use runtime_unroll::*;
pub use scalar::*;
pub use slp::*;
pub use specialization::*;
pub use unroll::*;
pub use vectorize::*;
pub(crate) use vectorize::{
    wasm_affine_loop_state_is_forwarded, wasm_matmul_interleave_eligible,
    wasm_matmul_interleave_source_is_closed,
};
