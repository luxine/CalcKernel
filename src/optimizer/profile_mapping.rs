use crate::{CkProfileKirPlan, ProofArena, validate_ck_profile_kir_plan};

/// Verifies profile mapping as non-proof analysis input for the optimizer.
///
/// The immutable proof arena is accepted only to make the separation boundary
/// explicit: profile annotations cannot create or mutate proof certificates.
///
/// # Errors
///
/// Returns the independent KIR mapping verifier's stable failure.
pub fn validate_profile_mapping_for_optimizer(
    plan: &CkProfileKirPlan,
    proofs: &ProofArena,
) -> Result<(), String> {
    let proof_count = proofs.proofs().len();
    validate_ck_profile_kir_plan(plan).map_err(|error| error.to_string())?;
    if proofs.proofs().len() != proof_count {
        return Err("profile mapping modified the proof arena".to_string());
    }
    Ok(())
}
