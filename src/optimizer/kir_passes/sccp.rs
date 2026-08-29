use crate::KirModule;

use super::super::{ScalarAnalysisConfig, analyze_scalar_function};

/// O1 SCCP/range is analysis-only: rewrites consume its checked conclusions later.
pub(crate) fn run_sccp_range(module: &mut KirModule) -> bool {
    for function in &module.functions {
        if analyze_scalar_function(function, ScalarAnalysisConfig::default()).is_err() {
            // The bounded analysis has a conservative fallback; a malformed KIR is rejected by
            // the mandatory verifier surrounding this pass.
            return false;
        }
    }
    false
}
