use crate::{FunctionId, KirInstructionKind, KirModule};

use super::super::{ScalarAnalysisConfig, ScalarAnalysisResult, analyze_scalar_function};

/// Complete scalar results owned by one exact KIR state. The pass manager may
/// preserve this cache only across transformations whose contract keeps the
/// CFG, SSA identities, transfer operations, and edge predicates unchanged.
pub(crate) struct ScalarAnalysisCache {
    module_functions: Vec<FunctionId>,
    analyses: Vec<ScalarAnalysisResult>,
}

impl ScalarAnalysisCache {
    pub(crate) fn covers(&self, module: &KirModule) -> bool {
        self.module_functions.len() == module.functions.len()
            && self
                .module_functions
                .iter()
                .zip(&module.functions)
                .all(|(function_id, function)| *function_id == function.id)
    }

    pub(crate) fn analyzed_functions(&self) -> usize {
        self.analyses.len()
    }
}

/// Runs O1 SCCP/range analysis without rewriting KIR. A domain error keeps the
/// pass conservative and prevents cache reuse; malformed KIR is rejected by
/// the mandatory verifier surrounding this named pass.
pub(crate) fn run_sccp_range(module: &KirModule) -> Option<ScalarAnalysisCache> {
    let module_functions = module
        .functions
        .iter()
        .map(|function| function.id)
        .collect();
    let analyses = module
        .functions
        .iter()
        .filter(|function| {
            function.blocks.iter().any(|block| {
                block
                    .instructions
                    .iter()
                    .any(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
            })
        })
        .map(|function| analyze_scalar_function(function, ScalarAnalysisConfig::default()))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(ScalarAnalysisCache {
        module_functions,
        analyses,
    })
}
