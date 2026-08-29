mod cfg;
mod check_elimination;
mod cleanup;
mod dce;
mod sccp;

pub(crate) use cfg::run_cfg_canonicalize;
pub(crate) use check_elimination::run_check_elimination;
pub(crate) use cleanup::run_cleanup;
pub(crate) use dce::run_dead_code_elimination;
pub(crate) use sccp::run_sccp_range;
