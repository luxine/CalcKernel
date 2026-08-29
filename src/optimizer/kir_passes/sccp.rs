use crate::KirModule;

use super::super::{ScalarAnalysisConfig, ScalarAnalysisResult, analyze_scalar_function};

/// Complete scalar results owned by one exact KIR state. The pass manager may
/// preserve this cache only across transformations whose contract keeps the
/// CFG, SSA identities, transfer operations, and edge predicates unchanged.
pub(crate) struct ScalarAnalysisCache {
    analyses: Vec<ScalarAnalysisResult>,
}

impl ScalarAnalysisCache {
    pub(crate) fn covers(&self, module: &KirModule) -> bool {
        self.analyses.len() == module.functions.len()
            && self
                .analyses
                .iter()
                .zip(&module.functions)
                .all(|(analysis, function)| analysis.function() == function.id)
    }
}

/// Runs O1 SCCP/range analysis without rewriting KIR. A domain error keeps the
/// pass conservative and prevents cache reuse; malformed KIR is rejected by
/// the mandatory verifier surrounding this named pass.
pub(crate) fn run_sccp_range(module: &KirModule) -> Option<ScalarAnalysisCache> {
    let analyses = module
        .functions
        .iter()
        .map(|function| analyze_scalar_function(function, ScalarAnalysisConfig::default()))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(ScalarAnalysisCache { analyses })
}
