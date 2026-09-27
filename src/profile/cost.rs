use super::CkProfileError;

/// Canonical signed-magnitude integer used by profile profitability checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CkSignedMagnitude {
    pub negative: bool,
    pub magnitude: u128,
}

impl CkSignedMagnitude {
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            negative: false,
            magnitude: 0,
        }
    }
}

/// Closed affine target-cost formula `fixed + per_unit * value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CkAffineCostFormula {
    pub fixed: u64,
    pub per_unit: u64,
}

/// Exact observation or one complete schema histogram interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileCostDomain {
    Exact(u32),
    HistogramBucket(u8),
}

/// One observed outcome class and the immutable formulas checked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileCostClass {
    pub count: u64,
    pub domain: CkProfileCostDomain,
    pub baseline: CkAffineCostFormula,
    pub selected: CkAffineCostFormula,
}

/// A proposed guarded decision. `reported_net` is untrusted checker input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileCostProposal {
    pub classes: Vec<CkProfileCostClass>,
    pub guard_cost: u64,
    pub reported_net: CkSignedMagnitude,
}

/// Independent profitability result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CkProfileCostDecision {
    Select { net_benefit: u128 },
    Baseline,
}

/// Returns the inclusive interval represented by a schema-1 histogram bucket.
#[must_use]
pub const fn profile_histogram_bucket_range(bucket: u8) -> Option<(u32, u32)> {
    match bucket {
        0 => Some((0, 0)),
        1 => Some((1, 1)),
        2 => Some((2, 2)),
        3 => Some((3, 4)),
        4 => Some((5, 8)),
        5 => Some((9, 16)),
        6 => Some((17, 32)),
        7 => Some((33, 64)),
        8 => Some((65, 128)),
        9 => Some((129, 256)),
        10 => Some((257, 512)),
        11 => Some((513, 1024)),
        12 => Some((1025, 2048)),
        13 => Some((2049, 4096)),
        14 => Some((4097, 65536)),
        15 => Some((65537, u32::MAX)),
        _ => None,
    }
}

/// Recomputes every class lower bound and the complete guarded net benefit.
///
/// # Errors
///
/// Rejects an invalid bucket, empty class set, checked arithmetic overflow, or
/// a proposal whose reported total does not equal the independent result.
pub fn verify_profile_cost_proposal(
    proposal: &CkProfileCostProposal,
) -> Result<CkProfileCostDecision, CkProfileError> {
    if proposal.classes.is_empty() {
        return Err(CkProfileError::Application(
            "profile cost classes are empty",
        ));
    }
    let mut positive = 0u128;
    let mut negative = 0u128;
    let mut observations = 0u128;
    for class in &proposal.classes {
        observations = observations.checked_add(u128::from(class.count)).ok_or(
            CkProfileError::ArithmeticOverflow("profile class observations"),
        )?;
        let lower = class_lower_bound(class)?;
        let weighted = lower
            .magnitude
            .checked_mul(u128::from(class.count))
            .ok_or(CkProfileError::ArithmeticOverflow("weighted profile class"))?;
        let side = if lower.negative {
            &mut negative
        } else {
            &mut positive
        };
        *side = side
            .checked_add(weighted)
            .ok_or(CkProfileError::ArithmeticOverflow("profile class sum"))?;
    }
    negative = negative
        .checked_add(
            observations
                .checked_mul(u128::from(proposal.guard_cost))
                .ok_or(CkProfileError::ArithmeticOverflow("profile guard cost"))?,
        )
        .ok_or(CkProfileError::ArithmeticOverflow("profile negative cost"))?;
    let recomputed = if positive >= negative {
        CkSignedMagnitude {
            negative: false,
            magnitude: positive - negative,
        }
    } else {
        CkSignedMagnitude {
            negative: true,
            magnitude: negative - positive,
        }
    };
    if proposal.reported_net != recomputed {
        return Err(CkProfileError::Application("profile cost total mismatch"));
    }
    Ok(if !recomputed.negative && recomputed.magnitude != 0 {
        CkProfileCostDecision::Select {
            net_benefit: recomputed.magnitude,
        }
    } else {
        CkProfileCostDecision::Baseline
    })
}

fn class_lower_bound(class: &CkProfileCostClass) -> Result<CkSignedMagnitude, CkProfileError> {
    let (lower, upper) = match class.domain {
        CkProfileCostDomain::Exact(value) => (value, value),
        CkProfileCostDomain::HistogramBucket(bucket) => profile_histogram_bucket_range(bucket)
            .ok_or(CkProfileError::InvalidValue("profile.cost.bucket"))?,
    };
    let first = difference(class.baseline, class.selected, lower)?;
    let last = difference(class.baseline, class.selected, upper)?;
    Ok(minimum_signed(first, last))
}

fn difference(
    baseline: CkAffineCostFormula,
    selected: CkAffineCostFormula,
    value: u32,
) -> Result<CkSignedMagnitude, CkProfileError> {
    let baseline = evaluate(baseline, value)?;
    let selected = evaluate(selected, value)?;
    Ok(if baseline >= selected {
        CkSignedMagnitude {
            negative: false,
            magnitude: baseline - selected,
        }
    } else {
        CkSignedMagnitude {
            negative: true,
            magnitude: selected - baseline,
        }
    })
}

fn evaluate(formula: CkAffineCostFormula, value: u32) -> Result<u128, CkProfileError> {
    u128::from(formula.per_unit)
        .checked_mul(u128::from(value))
        .and_then(|variable| variable.checked_add(u128::from(formula.fixed)))
        .ok_or(CkProfileError::ArithmeticOverflow("affine target cost"))
}

fn minimum_signed(left: CkSignedMagnitude, right: CkSignedMagnitude) -> CkSignedMagnitude {
    match (left.negative, right.negative) {
        (true, true) => {
            if left.magnitude >= right.magnitude {
                left
            } else {
                right
            }
        }
        (true, false) => left,
        (false, true) => right,
        (false, false) => {
            if left.magnitude <= right.magnitude {
                left
            } else {
                right
            }
        }
    }
}
