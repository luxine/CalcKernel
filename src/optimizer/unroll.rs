use crate::{CandidateBudgetCharge, KirVerifiedProgramState, UnrollCandidate, UnrollPlan};

#[derive(Debug, Clone)]
pub struct PreparedUnroll {
    pub trial: KirVerifiedProgramState,
    pub plan: UnrollPlan,
    pub charge: CandidateBudgetCharge,
}

pub fn prepare_unroll_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &UnrollCandidate,
) -> Result<PreparedUnroll, String> {
    let prepared = super::kir_passes::materialize_unroll_trial(pre_state, candidate)?;
    Ok(PreparedUnroll {
        trial: prepared.trial,
        plan: prepared.plan,
        charge: prepared.charge,
    })
}
