use sha2::{Digest, Sha256};

use super::{ExpansionDisposition, SearchFrontier};
use crate::TuningPlan;

/// Computes the schema-1 identity of the complete deterministic search result.
#[must_use]
pub fn canonical_frontier_digest(frontier: &SearchFrontier) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-FRONTIER\0");
    hasher.update(bounded_len(frontier.expansions.len()));
    for expansion in &frontier.expansions {
        hasher.update(expansion.ordinal.to_be_bytes());
        hasher.update(expansion.parent_plan_digest);
        hasher.update(expansion.unit_id);
        hasher.update(expansion.variant_id);
        hasher.update([match expansion.disposition {
            ExpansionDisposition::Legal => 1,
            ExpansionDisposition::Duplicate => 2,
        }]);
        hasher.update(expansion.result_plan_digest);
        hasher.update(expansion.whole_plan_dynamic.to_be_bytes());
        hasher.update(expansion.whole_plan_static.to_be_bytes());
        hasher.update(expansion.whole_plan_kir_bytes.to_be_bytes());
    }
    hash_plans(&mut hasher, &frontier.frontier);
    hash_plans(&mut hasher, &frontier.compile_selection);
    hasher.finalize().into()
}

fn hash_plans(hasher: &mut Sha256, plans: &[TuningPlan]) {
    hasher.update(bounded_len(plans.len()));
    for plan in plans {
        hasher.update(plan.digest);
        hasher.update(plan.predicted_dynamic.to_be_bytes());
        hasher.update(plan.predicted_static.to_be_bytes());
        hasher.update(plan.kir_bytes.to_be_bytes());
        hasher.update(bounded_len(plan.choices.len()));
        for choice in &plan.choices {
            hasher.update(choice.unit_id);
            hasher.update(choice.variant_id);
            hasher.update([choice.class as u8]);
        }
    }
}

fn bounded_len(value: usize) -> [u8; 4] {
    u32::try_from(value).unwrap_or(u32::MAX).to_be_bytes()
}
