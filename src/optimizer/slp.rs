use crate::{CandidateBudgetCharge, KirVerifiedProgramState, SlpCandidate, SlpPlan};

#[derive(Debug, Clone)]
pub struct PreparedSlp {
    pub trial: KirVerifiedProgramState,
    pub plan: SlpPlan,
    pub charge: CandidateBudgetCharge,
}

pub fn prepare_slp_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &SlpCandidate,
) -> Result<PreparedSlp, String> {
    let prepared = super::kir_passes::materialize_slp_trial(pre_state, candidate)?;
    Ok(PreparedSlp {
        trial: prepared.trial,
        plan: prepared.plan,
        charge: prepared.charge,
    })
}
