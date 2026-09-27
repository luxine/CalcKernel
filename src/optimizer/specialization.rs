use crate::{
    CandidateBudgetCharge, KirVerifiedProgramState, SpecializationCandidate, SpecializationPlan,
};

/// A proposer-owned specialization trial. The independent checker deliberately
/// lives in a separate module and only consumes these frozen values.
#[derive(Debug, Clone)]
pub struct PreparedSpecialization {
    pub trial: KirVerifiedProgramState,
    pub plan: SpecializationPlan,
    pub charge: CandidateBudgetCharge,
}

pub fn prepare_specialization_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &SpecializationCandidate,
    clone_ordinal: u8,
) -> Result<PreparedSpecialization, String> {
    let prepared =
        super::kir_passes::materialize_specialization_trial(pre_state, candidate, clone_ordinal)?;
    Ok(PreparedSpecialization {
        trial: prepared.trial,
        plan: prepared.plan,
        charge: prepared.charge,
    })
}
