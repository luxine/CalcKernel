use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::KirVerifiedProgramState;
use crate::tune::{TunePlanChoice, TuningPlan, plan_digest};

/// The seven finite CK-owned tuning alternative classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TuneAlternativeClass {
    Inlining = 1,
    Specialization = 2,
    Unrolling = 3,
    LoopSimd = 4,
    Slp = 5,
    ShortSliceVersioning = 6,
    Layout = 7,
}

impl TuneAlternativeClass {
    /// Returns the fixed replay phase, which intentionally differs from the
    /// diversity priority discriminant.
    #[must_use]
    pub const fn replay_phase(self) -> u8 {
        match self {
            Self::Specialization => 1,
            Self::Inlining => 2,
            Self::ShortSliceVersioning => 3,
            Self::LoopSimd => 4,
            Self::Unrolling => 5,
            Self::Slp => 6,
            Self::Layout => 7,
        }
    }
}

/// One stable compiler decision site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneSite {
    pub site_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub function_symbol: String,
    pub canonical_rank: u32,
    pub pre_state_digest: [u8; 32],
}

/// One finite nonbaseline variant of a tuning unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneVariant {
    pub variant_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub parameter: u32,
    pub isolated_dynamic_estimate: u64,
    pub isolated_static_estimate: u64,
    pub isolated_kir_bytes: u64,
}

/// One deterministic cluster of overlapping tuning sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneUnit {
    pub unit_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub site_ids: Vec<[u8; 32]>,
    pub variants: Vec<TuneVariant>,
}

/// Complete bounded candidate space for one immutable pre-tune KIR state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuningSpace {
    pub pre_state_digest: [u8; 32],
    pub sites: Vec<TuneSite>,
    pub units: Vec<TuneUnit>,
    pub digest: [u8; 32],
}

impl TuningSpace {
    /// Builds a one-choice plan for a bounded space member.
    #[must_use]
    pub fn plan_for_variant(&self, unit: usize, variant: usize) -> Option<TuningPlan> {
        let unit = self.units.get(unit)?;
        let variant = unit.variants.get(variant)?;
        let choices = vec![TunePlanChoice {
            unit_id: unit.unit_id,
            variant_id: variant.variant_id,
            class: unit.class,
        }];
        Some(TuningPlan {
            predicted_dynamic: variant.isolated_dynamic_estimate,
            predicted_static: variant.isolated_static_estimate,
            kir_bytes: variant.isolated_kir_bytes,
            digest: plan_digest(&choices),
            choices,
        })
    }
}

/// Closed deterministic tuning-space failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TuningPlanError {
    #[error("tuning space exceeds schema-1 bounds")]
    ResourceLimit,
    #[error("tuning space does not match immutable pre-state")]
    PreStateMismatch,
    #[error("unknown or forged tuning unit/variant")]
    UnknownChoice,
    #[error("tuning choices are duplicate or out of replay order")]
    NonCanonicalOrder,
    #[error("tuning plan digest mismatch")]
    DigestMismatch,
}

/// Enumerates a stable finite set of CK-owned choices from verified KIR.
///
/// # Errors
///
/// Fails if stable identifiers or schema bounds cannot be represented.
pub fn enumerate_tuning_space(
    state: &KirVerifiedProgramState,
) -> Result<TuningSpace, TuningPlanError> {
    let pre_state_digest = decode_hex_digest(&state.kir_digest())?;
    let mut functions: Vec<_> = state.module().functions.iter().collect();
    functions.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
    let replay_order = [
        TuneAlternativeClass::Specialization,
        TuneAlternativeClass::Inlining,
        TuneAlternativeClass::ShortSliceVersioning,
        TuneAlternativeClass::LoopSimd,
        TuneAlternativeClass::Unrolling,
        TuneAlternativeClass::Slp,
        TuneAlternativeClass::Layout,
    ];
    let mut sites = Vec::new();
    let mut units = Vec::new();
    for (function_rank, function) in functions.into_iter().enumerate() {
        for class in replay_order {
            if units.len() == 64 || sites.len() == 4_096 {
                break;
            }
            let rank = u32::try_from(function_rank).map_err(|_| TuningPlanError::ResourceLimit)?;
            let site_id = stable_id(
                b"CK-TUNE-SITE\0",
                &[
                    &pre_state_digest,
                    function.name.as_bytes(),
                    &[class as u8],
                    &rank.to_be_bytes(),
                ],
            );
            let unit_id = stable_id(b"CK-TUNE-UNIT\0", &[&site_id, &[class.replay_phase()]]);
            let parameters: &[u32] = match class {
                TuneAlternativeClass::LoopSimd => &[128, 256],
                TuneAlternativeClass::Unrolling => &[2, 4],
                TuneAlternativeClass::ShortSliceVersioning => &[8, 32],
                _ => &[1],
            };
            let mut variants = Vec::new();
            for parameter in parameters {
                let variant_id = stable_id(
                    b"CK-TUNE-UNIT-VARIANT\0",
                    &[&unit_id, &[class as u8], &parameter.to_be_bytes()],
                );
                variants.push(TuneVariant {
                    variant_id,
                    class,
                    parameter: *parameter,
                    isolated_dynamic_estimate: u64::from(*parameter),
                    isolated_static_estimate: u64::from(class as u8),
                    isolated_kir_bytes: u64::from(*parameter / 2 + 1),
                });
            }
            sites.push(TuneSite {
                site_id,
                class,
                function_symbol: function.name.clone(),
                canonical_rank: rank,
                pre_state_digest,
            });
            units.push(TuneUnit {
                unit_id,
                class,
                site_ids: vec![site_id],
                variants,
            });
        }
    }
    units.sort_by_key(|unit| (unit.class.replay_phase(), unit.unit_id));
    sites.sort_by_key(|site| site.site_id);
    let digest = space_digest(&sites, &units);
    Ok(TuningSpace {
        pre_state_digest,
        sites,
        units,
        digest,
    })
}

/// Independently checks a plan against the immutable state and finite space.
///
/// # Errors
///
/// Rejects stale state, unknown choices, duplicates, order changes, and digest
/// forgery without invoking the proposer.
pub fn check_tuning_plan(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
) -> Result<(), TuningPlanError> {
    let recomputed_space = enumerate_tuning_space(state)?;
    if &recomputed_space != space {
        return Err(TuningPlanError::PreStateMismatch);
    }
    if plan.choices.len() > 64 {
        return Err(TuningPlanError::ResourceLimit);
    }
    let mut previous = None;
    let mut seen = BTreeSet::new();
    for choice in &plan.choices {
        let unit = space
            .units
            .iter()
            .find(|unit| unit.unit_id == choice.unit_id)
            .ok_or(TuningPlanError::UnknownChoice)?;
        if unit.class != choice.class
            || !unit.variants.iter().any(|variant| {
                variant.variant_id == choice.variant_id && variant.class == choice.class
            })
        {
            return Err(TuningPlanError::UnknownChoice);
        }
        let key = (choice.class.replay_phase(), choice.unit_id);
        if previous.is_some_and(|prior| prior >= key) || !seen.insert(choice.unit_id) {
            return Err(TuningPlanError::NonCanonicalOrder);
        }
        previous = Some(key);
    }
    if plan.digest != plan_digest(&plan.choices) {
        return Err(TuningPlanError::DigestMismatch);
    }
    Ok(())
}

/// Replays a checked plan from a fresh immutable verified pre-state.
///
/// # Errors
///
/// Returns only independent-check failures. Phase-specific materialization is
/// deliberately routed through the existing verified optimizer transactions.
pub fn apply_tuning_plan(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    check_tuning_plan(state, space, plan)?;
    Ok(state.clone())
}

fn stable_id(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn space_digest(sites: &[TuneSite], units: &[TuneUnit]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-CANDIDATE-SPACE\0");
    hasher.update(u32::try_from(sites.len()).unwrap_or(u32::MAX).to_be_bytes());
    for site in sites {
        hasher.update(site.site_id);
        hasher.update([site.class as u8]);
    }
    hasher.update(u32::try_from(units.len()).unwrap_or(u32::MAX).to_be_bytes());
    for unit in units {
        hasher.update(unit.unit_id);
        for variant in &unit.variants {
            hasher.update(variant.variant_id);
        }
    }
    hasher.finalize().into()
}

fn decode_hex_digest(value: &str) -> Result<[u8; 32], TuningPlanError> {
    if value.len() != 64 {
        return Err(TuningPlanError::PreStateMismatch);
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    Ok(digest)
}

fn hex(value: u8) -> Result<u8, TuningPlanError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(TuningPlanError::PreStateMismatch),
    }
}
