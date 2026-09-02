use super::{TuneDecision, TuneDecisionError, decode_tune_decision};

/// Failure at either mandatory decision-assembly validation boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecisionAssemblyError {
    #[error("self-contained decision validation failed")]
    Decision(#[from] TuneDecisionError),
    #[error("source-aware replay validation failed: {0}")]
    SourceAware(String),
}

/// Admits decision bytes to publication only after both mandatory checkers pass.
pub fn assemble_decision<F>(
    encoded: Vec<u8>,
    source_aware_check: F,
) -> Result<TuneDecision, DecisionAssemblyError>
where
    F: FnOnce(&TuneDecision) -> Result<(), String>,
{
    let decision = decode_tune_decision(&encoded)?;
    decision.validate_self_contained()?;
    source_aware_check(&decision).map_err(DecisionAssemblyError::SourceAware)?;
    Ok(decision)
}
