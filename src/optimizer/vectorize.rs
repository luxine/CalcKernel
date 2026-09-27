use crate::{
    CandidateBudgetCharge, KirVerifiedProgramState, VectorizationCandidate, VectorizationPlan,
};

#[derive(Debug, Clone)]
pub struct PreparedVectorization {
    pub trial: KirVerifiedProgramState,
    pub plan: VectorizationPlan,
    pub charge: CandidateBudgetCharge,
}

pub fn prepare_vectorization_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &VectorizationCandidate,
) -> Result<PreparedVectorization, String> {
    let prepared = super::kir_passes::materialize_vectorization_trial(pre_state, candidate)?;
    Ok(PreparedVectorization {
        trial: prepared.trial,
        plan: prepared.plan,
        charge: prepared.charge,
    })
}
