mod cfg;
mod check_elimination;
mod cleanup;
mod constant_fold;
mod dce;
mod dse;
mod gvn;
mod induction;
mod inline;
mod licm;
mod load_forward;
mod loop_simplify;
mod memory;
mod phi_prune;
mod rewrite;
mod sccp;
mod slp;
mod specialize;
mod unroll;
mod vectorize;

pub(crate) use cfg::run_cfg_canonicalize;
pub(crate) use check_elimination::run_check_elimination;
pub(crate) use cleanup::run_cleanup;
pub(crate) use constant_fold::run_integer_constant_folding;
pub(crate) use dce::run_dead_code_elimination;
pub(crate) use dse::run_dead_store_elimination;
pub(crate) use gvn::run_gvn;
pub(crate) use induction::run_induction_simplification;
pub use inline::InlineTuningCandidate;
pub(crate) use inline::{
    check_tuning_inline_independently, discover_tuning_inline_candidates,
    materialize_tuning_inline, run_effect_aware_inline,
};
pub(crate) use licm::run_licm;
pub(crate) use load_forward::run_load_forwarding;
pub use loop_simplify::{LoopSimplifyResult, canonicalize_kir_loops};
pub(crate) use memory::run_memory_ssa_refine;
pub(crate) use sccp::{ScalarAnalysisCache, run_sccp_range};
pub(crate) use slp::{MaterializedSlp, materialize_slp_trial};
pub(crate) use specialize::{materialize_specialization_trial, specialization_clone_name};
pub(crate) use unroll::{MaterializedUnroll, materialize_unroll_trial};
pub(crate) use vectorize::{
    materialize_tuned_vectorization_trial, materialize_vectorization_trial,
};
