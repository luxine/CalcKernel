mod cfg;
mod constant_folding;
mod copy_propagation;
mod cse;
mod dce;
mod inlining;
mod loops;

pub(super) use cfg::run_cfg_simplify;
pub(super) use constant_folding::run_constant_folding;
pub(super) use copy_propagation::run_copy_propagation;
pub(super) use cse::{run_address_cse, run_local_cse};
pub(super) use dce::run_dead_code_elimination;
pub(super) use inlining::run_inline_small_functions;
pub(super) use loops::run_loop_invariant_code_motion;
