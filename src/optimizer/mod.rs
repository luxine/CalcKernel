mod analysis;
mod audit;
mod decision_tree_plan;
mod decision_tree_vector_check;
mod facts;
mod interior_normalize;
mod invariant_load_check;
mod kir_passes;
mod kir_pipeline;
mod multiversion;
mod normalization_unswitch;
mod pgo;
mod profile_analysis;
mod profile_mapping;
mod proof;
mod runtime_unroll_check;
mod slp;
mod slp_check;
mod specialization;
mod specialization_check;
mod stencil_peel;
mod stencil_vector;
mod transaction;
mod unroll;
mod unroll_check;
mod vector_check;
mod vector_plan;
mod vectorize;
mod vectorize_check;
mod verify;

pub use analysis::*;
pub use audit::*;
pub use decision_tree_plan::*;
pub use decision_tree_vector_check::*;
pub use facts::*;
pub use interior_normalize::*;
pub use invariant_load_check::*;
pub use kir_passes::{
    LoopSimplifyResult, MaterializedDecisionTreeVector, canonicalize_kir_loops,
    prepare_decision_tree_vector_trial, prepare_wasm_invariant_load_trial,
    prepare_wasm_runtime_scalar_unroll_trial,
};
pub use kir_pipeline::*;
pub use multiversion::*;
pub use normalization_unswitch::*;
pub use pgo::*;
pub use profile_analysis::*;
pub use profile_mapping::*;
pub use proof::*;
pub use runtime_unroll_check::*;
pub use slp::*;
pub use slp_check::*;
pub use specialization::*;
pub use specialization_check::*;
pub use stencil_peel::*;
pub use stencil_vector::*;
pub use transaction::*;
pub use unroll::*;
pub use unroll_check::*;
pub use vector_check::*;
pub use vector_plan::*;
pub use vectorize::*;
pub use vectorize_check::*;
pub use verify::*;

/// Schema of the deterministic vector cost model stored in Native cache keys.
pub const KIR_VECTOR_COST_MODEL_SCHEMA: u32 = 1;
/// Schema of vector transformation proof records stored in Native cache keys.
pub const KIR_VECTOR_PROOF_SCHEMA: u32 = 1;

pub(crate) const KIR_INLINE_CALLEE_BUDGET: usize = 32;
pub(crate) const KIR_MULTIVERSION_INLINE_CALLEE_BUDGET: usize = 8;
pub(crate) const KIR_PGO_HOT_INLINE_CALLEE_BUDGET: usize = 48;

/// Canonical identity of every fixed 0.12 optimizer budget currently capable
/// of changing Native object bytes. New budgets must extend this string.
#[must_use]
pub const fn kir_vector_budget_identity() -> &'static str {
    "vector-budget-schema=1;predicates=4;minimum-cost-reduction-percent=20"
}
