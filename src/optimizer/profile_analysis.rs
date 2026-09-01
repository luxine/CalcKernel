use std::sync::Arc;

use crate::{CkProfileAnalysis, CkProfileKirPlan, ProofArena};

/// Shared immutable non-proof profile input for optimizer stages.
#[derive(Debug, Clone)]
pub struct CkImmutableProfileAnalysis(Arc<CkProfileAnalysis>);

impl CkImmutableProfileAnalysis {
    #[must_use]
    pub fn new(analysis: CkProfileAnalysis) -> Self {
        Self(Arc::new(analysis))
    }

    #[must_use]
    pub fn get(&self) -> &CkProfileAnalysis {
        &self.0
    }
}

/// Checks that one immutable profile sidecar names exactly the verified plan
/// without importing any observation into the proof arena.
///
/// # Errors
///
/// Rejects stale KIR mappings, missing/reordered site analyses, and any proof
/// arena mutation across the boundary.
pub fn validate_profile_analysis_for_optimizer(
    plan: &CkProfileKirPlan,
    analysis: &CkImmutableProfileAnalysis,
    proofs: &ProofArena,
) -> Result<(), String> {
    let before = proofs.clone();
    super::validate_profile_mapping_for_optimizer(plan, proofs)?;
    if analysis.0.sites.len() != plan.sites.len()
        || analysis
            .0
            .sites
            .iter()
            .zip(&plan.sites)
            .any(|(analyzed, expected)| analyzed.descriptor != *expected)
    {
        return Err("profile analysis does not match the canonical KIR site table".to_string());
    }
    if proofs != &before {
        return Err("profile analysis modified the proof arena".to_string());
    }
    Ok(())
}
