use sha2::{Digest, Sha256};

use crate::TuneAlternativeClass;

/// One canonical nonbaseline unit choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunePlanChoice {
    pub unit_id: [u8; 32],
    pub variant_id: [u8; 32],
    pub class: TuneAlternativeClass,
}

/// One canonical exact tuning plan and whole-plan rank material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuningPlan {
    pub choices: Vec<TunePlanChoice>,
    pub predicted_dynamic: u64,
    pub predicted_static: u64,
    pub kir_bytes: u64,
    pub digest: [u8; 32],
}

impl TuningPlan {
    /// Returns the free baseline plan.
    #[must_use]
    pub fn baseline() -> Self {
        Self {
            choices: Vec::new(),
            predicted_dynamic: 0,
            predicted_static: 0,
            kir_bytes: 0,
            digest: plan_digest(&[]),
        }
    }
}

pub(crate) fn plan_digest(choices: &[TunePlanChoice]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-PLAN\0");
    hasher.update(
        u32::try_from(choices.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for choice in choices {
        hasher.update(choice.unit_id);
        hasher.update(choice.variant_id);
        hasher.update([choice.class as u8]);
    }
    hasher.finalize().into()
}
