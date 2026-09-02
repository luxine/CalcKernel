mod analysis;
mod audit;
mod facts;
mod kir_passes;
mod kir_pipeline;
mod multiversion;
mod pgo;
mod profile_analysis;
mod profile_mapping;
mod proof;
mod slp;
mod slp_check;
mod specialization;
mod specialization_check;
mod transaction;
mod tune;
mod unroll;
mod unroll_check;
mod vector_check;
mod vector_plan;
mod vectorize;
mod vectorize_check;
mod verify;

pub use analysis::*;
pub use audit::*;
pub use facts::*;
pub use kir_passes::{InlineTuningCandidate, LoopSimplifyResult, canonicalize_kir_loops};
pub use kir_pipeline::*;
pub use multiversion::*;
pub use pgo::*;
pub use profile_analysis::*;
pub use profile_mapping::*;
pub use proof::*;
pub use slp::*;
pub use slp_check::*;
pub use specialization::*;
pub use specialization_check::*;
pub use transaction::*;
pub use tune::*;
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

/// Canonical identity of every fixed 0.12 optimizer budget currently capable
/// of changing Native object bytes. New budgets must extend this string.
#[must_use]
pub const fn kir_vector_budget_identity() -> &'static str {
    "vector-budget-schema=1;predicates=4;minimum-cost-reduction-percent=20"
}
