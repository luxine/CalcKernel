use sha2::{Digest, Sha256};

use crate::TuneAlternativeClass;

/// One canonical nonbaseline unit choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunePlanChoice {
    pub unit_id: [u8; 32],
    pub variant_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub pre_state_digest: [u8; 32],
    pub post_state_digest: [u8; 32],
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
        let mut record = Vec::with_capacity(2 * 6 + 4 * 5 + 32 * 4 + 1);
        push_field(&mut record, 1, &choice.unit_id);
        push_field(&mut record, 2, &choice.variant_id);
        push_field(&mut record, 3, &[choice.class as u8]);
        push_field(&mut record, 4, &choice.pre_state_digest);
        push_field(&mut record, 5, &choice.post_state_digest);
        hasher.update(
            u32::try_from(record.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        hasher.update(record);
    }
    hasher.finalize().into()
}

fn push_field(output: &mut Vec<u8>, tag: u16, value: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    output.extend_from_slice(value);
}
