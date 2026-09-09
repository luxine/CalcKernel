use std::collections::BTreeMap;

use super::{
    CkProfile, CkProfileCounter, CkProfileError, CkProfileSiteDescriptor, CkProfileSiteId,
    CkProfileSiteKind,
};

/// Why a syntactically valid profile site cannot guide an optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileUnknownReason {
    Saturated,
    IncompleteTripObservation,
    ArithmeticOverflow,
    MappingUnavailable,
}

/// Immutable non-proof observation associated with one canonical site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CkProfileObservation {
    Scalar(u64),
    Histogram([u64; 16]),
    CandidateConstant { candidates: Vec<u64>, other: u64 },
    Unknown(CkProfileUnknownReason),
}

impl CkProfileObservation {
    /// Returns the checked number of observations when this site is known.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        match self {
            Self::Scalar(value) => Some(*value),
            Self::Histogram(values) => checked_sum(values),
            Self::CandidateConstant { candidates, other } => candidates
                .iter()
                .copied()
                .try_fold(*other, u64::checked_add),
            Self::Unknown(_) => None,
        }
    }
}

/// One validated site and its conservative application status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileAnalyzedSite {
    pub descriptor: CkProfileSiteDescriptor,
    pub observation: CkProfileObservation,
}

/// Closed input for target-profile dynamic-work estimation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileWorkTerm {
    pub site_id: CkProfileSiteId,
    pub function_digest: [u8; 32],
    pub static_cost_units: u64,
}

/// Stable work rank and hot-root decision for one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileFunctionWork {
    pub function_digest: [u8; 32],
    pub dynamic_work: Option<u128>,
    pub rank: Option<u32>,
    pub hot_root: bool,
}

/// Complete immutable profile sidecar. It is deliberately separate from facts and proofs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileAnalysis {
    pub identity_digest: [u8; 32],
    pub sites: Vec<CkProfileAnalyzedSite>,
    pub functions: Vec<CkProfileFunctionWork>,
}

pub(super) fn analyze_profile(
    profile: &CkProfile,
    work_terms: &[CkProfileWorkTerm],
) -> Result<CkProfileAnalysis, CkProfileError> {
    let mut sites = Vec::with_capacity(profile.sites.len());
    let mut observations = BTreeMap::new();
    for (descriptor, record) in profile.sites.iter().zip(&profile.counters) {
        let observation = observation(profile, descriptor, &record.counter);
        observations.insert(descriptor.id, observation.clone());
        sites.push(CkProfileAnalyzedSite {
            descriptor: descriptor.clone(),
            observation,
        });
    }

    let mut work = BTreeMap::<[u8; 32], Option<u128>>::new();
    for term in work_terms {
        let amount = observations
            .get(&term.site_id)
            .and_then(CkProfileObservation::total)
            .map(u128::from)
            .and_then(|count| count.checked_mul(u128::from(term.static_cost_units)));
        let slot = work.entry(term.function_digest).or_insert(Some(0));
        *slot = match (*slot, amount) {
            (Some(total), Some(amount)) => total.checked_add(amount),
            _ => None,
        };
    }
    let functions = rank_function_work(work, &profile.identity.contract)?;
    Ok(CkProfileAnalysis {
        identity_digest: profile.identity.digest()?,
        sites,
        functions,
    })
}

fn observation(
    profile: &CkProfile,
    descriptor: &CkProfileSiteDescriptor,
    counter: &CkProfileCounter,
) -> CkProfileObservation {
    if counter.is_saturated() {
        return CkProfileObservation::Unknown(CkProfileUnknownReason::Saturated);
    }
    if profile.incomplete_observations
        && matches!(descriptor.kind, CkProfileSiteKind::LoopTripHistogram { .. })
    {
        return CkProfileObservation::Unknown(CkProfileUnknownReason::IncompleteTripObservation);
    }
    match counter {
        CkProfileCounter::Scalar(value) => CkProfileObservation::Scalar(*value),
        CkProfileCounter::Histogram { buckets, .. } => CkProfileObservation::Histogram(*buckets),
        CkProfileCounter::CandidateConstant {
            candidates, other, ..
        } => CkProfileObservation::CandidateConstant {
            candidates: candidates.clone(),
            other: *other,
        },
    }
}

fn rank_function_work(
    work: BTreeMap<[u8; 32], Option<u128>>,
    contract: &super::CkProfileContract,
) -> Result<Vec<CkProfileFunctionWork>, CkProfileError> {
    let mut known = work
        .iter()
        .filter_map(|(digest, value)| value.map(|value| (*digest, value)))
        .collect::<Vec<_>>();
    known.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let module_work = known.iter().try_fold(0u128, |total, (_, value)| {
        total
            .checked_add(*value)
            .ok_or(CkProfileError::ArithmeticOverflow("module dynamic work"))
    })?;
    let eligible = known
        .iter()
        .copied()
        .filter(|(_, value)| {
            known.len() == 1
                || ratio_u128_at_least(*value, module_work, contract.minimum_root_work_basis_points)
        })
        .collect::<Vec<_>>();
    let mut selected = BTreeMap::new();
    let mut covered = 0u128;
    for (digest, value) in eligible {
        if ratio_u128_at_least(
            covered,
            module_work,
            contract.hot_work_coverage_basis_points,
        ) {
            break;
        }
        selected.insert(digest, ());
        covered = covered
            .checked_add(value)
            .ok_or(CkProfileError::ArithmeticOverflow("hot work coverage"))?;
    }
    let ranks = known
        .iter()
        .enumerate()
        .map(|(index, (digest, _))| {
            u32::try_from(index + 1)
                .map(|rank| (*digest, rank))
                .map_err(|_| CkProfileError::LengthOverflow)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(work
        .into_iter()
        .map(|(function_digest, dynamic_work)| CkProfileFunctionWork {
            function_digest,
            dynamic_work,
            rank: ranks.get(&function_digest).copied(),
            hot_root: selected.contains_key(&function_digest),
        })
        .collect())
}

/// Checked integer ratio using schema basis points. Zero denominators never pass.
#[must_use]
pub fn profile_ratio_at_least(numerator: u64, denominator: u64, basis_points: u16) -> bool {
    denominator != 0
        && u128::from(numerator) * 10_000 >= u128::from(denominator) * u128::from(basis_points)
}

/// Returns a unique dominant outcome at the requested confidence threshold.
#[must_use]
pub fn profile_site_dominant_outcome(
    counts: &[u64],
    minimum_observations: u64,
    basis_points: u16,
) -> Option<usize> {
    let total = checked_sum(counts)?;
    if total < minimum_observations {
        return None;
    }
    let maximum = counts.iter().copied().max()?;
    let winner = counts.iter().position(|value| *value == maximum)?;
    (counts.iter().filter(|value| **value == maximum).count() == 1
        && profile_ratio_at_least(maximum, total, basis_points))
    .then_some(winner)
}

/// Applies the exact schema-1 cold rule without floating point.
#[must_use]
pub fn profile_is_cold(block_count: u64, function_entries: u64) -> bool {
    function_entries >= 128
        && profile_ratio_at_least(block_count, function_entries, 0)
        && u128::from(block_count) * 10_000 <= u128::from(function_entries) * 100
}

fn checked_sum(values: &[u64]) -> Option<u64> {
    values.iter().copied().try_fold(0u64, u64::checked_add)
}

fn ratio_u128_at_least(numerator: u128, denominator: u128, basis_points: u16) -> bool {
    if denominator == 0 {
        return false;
    }
    numerator
        .checked_mul(10_000)
        .zip(denominator.checked_mul(u128::from(basis_points)))
        .is_some_and(|(left, right)| left >= right)
}
