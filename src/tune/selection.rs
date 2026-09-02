use std::collections::{BTreeMap, BTreeSet};

use super::{MeasurementPhase, MeasurementStream, TuneCase, TuneCaseRole};

const Q32_ONE: u64 = 1u64 << 32;

/// Checked median and stability receipt for a complete stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamStatistics {
    pub upper_median_ns: u64,
    pub in_range_samples: u32,
}

/// Immutable candidate tie-break material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateRank {
    pub plan_digest: [u8; 32],
    pub primary_artifact_bytes: u64,
    pub choice_count: u32,
}

/// A search winner admitted to validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchEntrant {
    pub plan_digest: [u8; 32],
    pub score_q32: u64,
    pub primary_artifact_bytes: u64,
    pub choice_count: u32,
}

/// One validation case's independently derived medians and ratio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseMedian {
    pub case_id: String,
    pub baseline_ns: u64,
    pub candidate_ns: u64,
    pub ratio_q32: u64,
}

/// Complete derived evidence for one plan in one validation round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundPlan {
    pub plan_digest: [u8; 32],
    pub case_medians: Vec<CaseMedian>,
    pub aggregate_ratio_q32: u64,
    pub stable: bool,
    pub threshold_passed: bool,
    pub paired_wins: u32,
}

/// One complete validation-round summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundSummary {
    pub round: u8,
    pub plans: Vec<RoundPlan>,
    pub ranked_plan_digests: Vec<[u8; 32]>,
}

/// Selection table reasons from schema 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SelectionReason {
    Tuned = 1,
    NoCandidate = 2,
    ValidationThreshold = 3,
    ValidationDisagreement = 4,
}

/// Candidate terminal states affected by the selection table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CandidateOutcome {
    Baseline = 1,
    CompiledUnmeasured = 2,
    SizeRejected = 3,
    TimedOut = 4,
    SearchNonwinner = 5,
    ValidationThreshold = 6,
    ValidationNonwinner = 7,
    Selected = 8,
}

/// A candidate that entered validation, including a candidate timed out there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionEntrant {
    pub plan_digest: [u8; 32],
    pub timed_out: bool,
}

impl SelectionEntrant {
    #[must_use]
    pub const fn active(plan_digest: [u8; 32]) -> Self {
        Self {
            plan_digest,
            timed_out: false,
        }
    }

    #[must_use]
    pub const fn timed_out(plan_digest: [u8; 32]) -> Self {
        Self {
            plan_digest,
            timed_out: true,
        }
    }
}

/// Final result of the exhaustive four-row selection table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub selected_plan_digest: [u8; 32],
    pub reason: SelectionReason,
    pub certificate_plan_digest: Option<[u8; 32]>,
    pub outcomes: BTreeMap<[u8; 32], CandidateOutcome>,
}

/// Stable fail-closed selection error categories.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    #[error("invalid or incomplete measurement evidence: {0}")]
    InvalidEvidence(&'static str),
    #[error("unstable measurement stream")]
    Unstable,
    #[error("checked integer arithmetic overflow")]
    Overflow,
}

/// Recomputes the upper median and frozen 16-of-20 inclusive stability rule.
pub fn stream_statistics(stream: &MeasurementStream) -> Result<StreamStatistics, SelectionError> {
    if stream.rows.len() != 20 {
        return Err(SelectionError::InvalidEvidence("stream row count"));
    }
    let mut samples = Vec::with_capacity(20);
    for (expected, row) in stream.rows.iter().enumerate() {
        if row.ordinal != u32::try_from(expected).map_err(|_| SelectionError::Overflow)?
            || row.calls_ns.len() != 3
            || row.calls_ns.contains(&0)
            || row.stored_minimum_ns == 0
            || row.stored_minimum_ns
                != *row
                    .calls_ns
                    .iter()
                    .min()
                    .ok_or(SelectionError::InvalidEvidence("measurement calls"))?
        {
            return Err(SelectionError::InvalidEvidence("measurement row"));
        }
        samples.push(row.stored_minimum_ns);
    }
    samples.sort_unstable();
    let median = samples[10];
    let lower = u128::from(median)
        .checked_mul(80)
        .ok_or(SelectionError::Overflow)?;
    let upper = u128::from(median)
        .checked_mul(120)
        .ok_or(SelectionError::Overflow)?;
    let mut in_range = 0u32;
    for sample in samples {
        let scaled = u128::from(sample)
            .checked_mul(100)
            .ok_or(SelectionError::Overflow)?;
        if scaled >= lower && scaled <= upper {
            in_range = in_range.checked_add(1).ok_or(SelectionError::Overflow)?;
        }
    }
    if in_range < 16 {
        return Err(SelectionError::Unstable);
    }
    Ok(StreamStatistics {
        upper_median_ns: median,
        in_range_samples: in_range,
    })
}

/// Ranks the complete stable phase-3 finalist set and takes the fixed bound.
pub fn derive_search_entrants(
    baseline_plan_digest: [u8; 32],
    candidates: &[CandidateRank],
    cases: &[TuneCase],
    streams: &[MeasurementStream],
    limit: u32,
) -> Result<Vec<SearchEntrant>, SelectionError> {
    let search_cases = partition_cases(cases, TuneCaseRole::Search)?;
    let ranks = rank_map(candidates)?;
    let expected_plans = std::iter::once(baseline_plan_digest)
        .chain(candidates.iter().map(|candidate| candidate.plan_digest))
        .collect::<BTreeSet<_>>();
    let index = index_streams(
        streams,
        MeasurementPhase::SearchMeasured,
        0,
        &search_cases,
        &expected_plans,
    )?;
    let mut entrants = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let score = aggregate_medians(
            baseline_plan_digest,
            candidate.plan_digest,
            &search_cases,
            &index,
        )?;
        entrants.push(SearchEntrant {
            plan_digest: candidate.plan_digest,
            score_q32: score,
            primary_artifact_bytes: candidate.primary_artifact_bytes,
            choice_count: candidate.choice_count,
        });
    }
    entrants.sort_by_key(|entry| {
        (
            entry.score_q32,
            entry.primary_artifact_bytes,
            entry.choice_count,
            entry.plan_digest,
        )
    });
    entrants.truncate(usize::try_from(limit).map_err(|_| SelectionError::Overflow)?);
    let _ = ranks;
    Ok(entrants)
}

/// Recomputes all fields and qualification ranking for one validation round.
pub fn derive_round_summary(
    round: u8,
    baseline_plan_digest: [u8; 32],
    candidates: &[CandidateRank],
    cases: &[TuneCase],
    streams: &[MeasurementStream],
) -> Result<RoundSummary, SelectionError> {
    let phase = match round {
        1 => MeasurementPhase::ValidationOneMeasured,
        2 => MeasurementPhase::ValidationTwoMeasured,
        _ => return Err(SelectionError::InvalidEvidence("validation round")),
    };
    let validation_cases = partition_cases(cases, TuneCaseRole::Validation)?;
    let ranks = rank_map(candidates)?;
    let expected_plans = std::iter::once(baseline_plan_digest)
        .chain(candidates.iter().map(|candidate| candidate.plan_digest))
        .collect::<BTreeSet<_>>();
    let index = index_streams(streams, phase, round, &validation_cases, &expected_plans)?;
    let mut plans = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let mut medians = Vec::with_capacity(validation_cases.len());
        let mut ratios = Vec::with_capacity(validation_cases.len());
        for case in &validation_cases {
            let baseline = statistics_for(&index, &case.id, baseline_plan_digest)?;
            let measured = statistics_for(&index, &case.id, candidate.plan_digest)?;
            let ratio = ratio_q32(measured.upper_median_ns, baseline.upper_median_ns)?;
            medians.push(CaseMedian {
                case_id: case.id.clone(),
                baseline_ns: baseline.upper_median_ns,
                candidate_ns: measured.upper_median_ns,
                ratio_q32: ratio,
            });
            ratios.push((case.weight, ratio));
        }
        let aggregate = weighted_q32(&ratios)?;
        let paired_wins = paired_wins(
            baseline_plan_digest,
            candidate.plan_digest,
            &validation_cases,
            &index,
        )?;
        let aggregate_passed = u128::from(aggregate)
            .checked_mul(100)
            .ok_or(SelectionError::Overflow)?
            <= u128::from(Q32_ONE)
                .checked_mul(97)
                .ok_or(SelectionError::Overflow)?;
        let cases_passed = medians.iter().try_fold(true, |passed, median| {
            let scaled = u128::from(median.ratio_q32)
                .checked_mul(100)
                .ok_or(SelectionError::Overflow)?;
            let threshold = u128::from(Q32_ONE)
                .checked_mul(102)
                .ok_or(SelectionError::Overflow)?;
            Ok::<_, SelectionError>(passed && scaled <= threshold)
        })?;
        plans.push(RoundPlan {
            plan_digest: candidate.plan_digest,
            case_medians: medians,
            aggregate_ratio_q32: aggregate,
            stable: true,
            threshold_passed: aggregate_passed && cases_passed && paired_wins >= 16,
            paired_wins,
        });
    }
    plans.sort_by_key(|plan| plan.plan_digest);
    let mut passing = plans
        .iter()
        .filter(|plan| plan.threshold_passed)
        .map(|plan| {
            let rank = ranks[&plan.plan_digest];
            (
                plan.aggregate_ratio_q32,
                rank.primary_artifact_bytes,
                rank.choice_count,
                plan.plan_digest,
            )
        })
        .collect::<Vec<_>>();
    passing.sort();
    Ok(RoundSummary {
        round,
        plans,
        ranked_plan_digests: passing.into_iter().map(|entry| entry.3).collect(),
    })
}

/// Applies the disjoint and exhaustive schema-1 selection table.
pub fn derive_selection(
    baseline_plan_digest: [u8; 32],
    entrants: &[SelectionEntrant],
    round_one: &RoundSummary,
    round_two: &RoundSummary,
) -> Result<Selection, SelectionError> {
    if round_one.round != 1 || round_two.round != 2 {
        return Err(SelectionError::InvalidEvidence("selection rounds"));
    }
    let mut seen = BTreeSet::new();
    if entrants.iter().any(|entrant| {
        !seen.insert(entrant.plan_digest) || entrant.plan_digest == baseline_plan_digest
    }) {
        return Err(SelectionError::InvalidEvidence("validation entrants"));
    }
    let active = entrants
        .iter()
        .filter(|entrant| !entrant.timed_out)
        .map(|entrant| entrant.plan_digest)
        .collect::<BTreeSet<_>>();
    validate_round_for_selection(round_one, &active)?;
    validate_round_for_selection(round_two, &active)?;

    let (reason, selected, certificate) = if entrants.is_empty() {
        (SelectionReason::NoCandidate, baseline_plan_digest, None)
    } else if round_one.ranked_plan_digests.is_empty() || round_two.ranked_plan_digests.is_empty() {
        (
            SelectionReason::ValidationThreshold,
            baseline_plan_digest,
            None,
        )
    } else if round_one.ranked_plan_digests[0] == round_two.ranked_plan_digests[0] {
        let winner = round_one.ranked_plan_digests[0];
        (SelectionReason::Tuned, winner, Some(winner))
    } else {
        (
            SelectionReason::ValidationDisagreement,
            baseline_plan_digest,
            None,
        )
    };
    let mut outcomes = BTreeMap::new();
    for entrant in entrants {
        let outcome = if entrant.timed_out {
            CandidateOutcome::TimedOut
        } else {
            match reason {
                SelectionReason::Tuned if entrant.plan_digest == selected => {
                    CandidateOutcome::Selected
                }
                SelectionReason::Tuned | SelectionReason::ValidationDisagreement => {
                    CandidateOutcome::ValidationNonwinner
                }
                SelectionReason::ValidationThreshold => CandidateOutcome::ValidationThreshold,
                SelectionReason::NoCandidate => {
                    return Err(SelectionError::InvalidEvidence("no-candidate entrants"));
                }
            }
        };
        outcomes.insert(entrant.plan_digest, outcome);
    }
    Ok(Selection {
        selected_plan_digest: selected,
        reason,
        certificate_plan_digest: certificate,
        outcomes,
    })
}

fn validate_round_for_selection(
    round: &RoundSummary,
    active: &BTreeSet<[u8; 32]>,
) -> Result<(), SelectionError> {
    let plan_digests = round
        .plans
        .iter()
        .map(|plan| plan.plan_digest)
        .collect::<Vec<_>>();
    if plan_digests.windows(2).any(|items| items[0] >= items[1])
        || plan_digests.iter().copied().collect::<BTreeSet<_>>() != *active
        || round.plans.iter().any(|plan| !plan.stable)
    {
        return Err(SelectionError::InvalidEvidence("validation plan set"));
    }
    let passing = round
        .plans
        .iter()
        .filter(|plan| plan.threshold_passed)
        .map(|plan| plan.plan_digest)
        .collect::<BTreeSet<_>>();
    let ranked = round
        .ranked_plan_digests
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if ranked.len() != round.ranked_plan_digests.len() || ranked != passing {
        return Err(SelectionError::InvalidEvidence("qualified plan membership"));
    }
    Ok(())
}

fn partition_cases(
    cases: &[TuneCase],
    role: TuneCaseRole,
) -> Result<Vec<TuneCase>, SelectionError> {
    let mut selected = cases
        .iter()
        .filter(|case| case.role == role)
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.id.cmp(&right.id));
    if selected.is_empty()
        || selected.windows(2).any(|items| items[0].id == items[1].id)
        || selected.iter().any(|case| case.weight == 0)
    {
        return Err(SelectionError::InvalidEvidence("case partition"));
    }
    Ok(selected)
}

fn rank_map(
    candidates: &[CandidateRank],
) -> Result<BTreeMap<[u8; 32], CandidateRank>, SelectionError> {
    let mut ranks = BTreeMap::new();
    for candidate in candidates {
        if ranks.insert(candidate.plan_digest, *candidate).is_some() {
            return Err(SelectionError::InvalidEvidence("candidate rank"));
        }
    }
    Ok(ranks)
}

fn index_streams<'a>(
    streams: &'a [MeasurementStream],
    phase: MeasurementPhase,
    round: u8,
    cases: &[TuneCase],
    plans: &BTreeSet<[u8; 32]>,
) -> Result<BTreeMap<(String, [u8; 32]), &'a MeasurementStream>, SelectionError> {
    let case_ids = cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut index = BTreeMap::new();
    for stream in streams {
        if stream.phase != phase
            || stream.round != round
            || !case_ids.contains(stream.case_id.as_str())
            || !plans.contains(&stream.plan_digest)
            || index
                .insert((stream.case_id.clone(), stream.plan_digest), stream)
                .is_some()
        {
            return Err(SelectionError::InvalidEvidence("stream set"));
        }
    }
    let expected = cases
        .len()
        .checked_mul(plans.len())
        .ok_or(SelectionError::Overflow)?;
    if index.len() != expected {
        return Err(SelectionError::InvalidEvidence("incomplete stream set"));
    }
    Ok(index)
}

fn statistics_for(
    streams: &BTreeMap<(String, [u8; 32]), &MeasurementStream>,
    case_id: &str,
    plan_digest: [u8; 32],
) -> Result<StreamStatistics, SelectionError> {
    let stream = streams
        .get(&(case_id.to_string(), plan_digest))
        .ok_or(SelectionError::InvalidEvidence("missing stream"))?;
    stream_statistics(stream)
}

fn aggregate_medians(
    baseline: [u8; 32],
    candidate: [u8; 32],
    cases: &[TuneCase],
    streams: &BTreeMap<(String, [u8; 32]), &MeasurementStream>,
) -> Result<u64, SelectionError> {
    let mut ratios = Vec::with_capacity(cases.len());
    for case in cases {
        let baseline_ns = statistics_for(streams, &case.id, baseline)?.upper_median_ns;
        let candidate_ns = statistics_for(streams, &case.id, candidate)?.upper_median_ns;
        ratios.push((case.weight, ratio_q32(candidate_ns, baseline_ns)?));
    }
    weighted_q32(&ratios)
}

fn paired_wins(
    baseline: [u8; 32],
    candidate: [u8; 32],
    cases: &[TuneCase],
    streams: &BTreeMap<(String, [u8; 32]), &MeasurementStream>,
) -> Result<u32, SelectionError> {
    let mut wins = 0u32;
    for row in 0..20usize {
        let mut ratios = Vec::with_capacity(cases.len());
        for case in cases {
            let baseline_stream = streams
                .get(&(case.id.clone(), baseline))
                .ok_or(SelectionError::InvalidEvidence("missing baseline stream"))?;
            let candidate_stream = streams
                .get(&(case.id.clone(), candidate))
                .ok_or(SelectionError::InvalidEvidence("missing candidate stream"))?;
            ratios.push((
                case.weight,
                ratio_q32(
                    candidate_stream.rows[row].stored_minimum_ns,
                    baseline_stream.rows[row].stored_minimum_ns,
                )?,
            ));
        }
        if weighted_q32(&ratios)? < Q32_ONE {
            wins = wins.checked_add(1).ok_or(SelectionError::Overflow)?;
        }
    }
    Ok(wins)
}

fn ratio_q32(candidate: u64, baseline: u64) -> Result<u64, SelectionError> {
    if candidate == 0 || baseline == 0 {
        return Err(SelectionError::InvalidEvidence("zero timing"));
    }
    let numerator = u128::from(candidate)
        .checked_mul(1u128 << 32)
        .ok_or(SelectionError::Overflow)?;
    checked_ceil_div_u64(numerator, u128::from(baseline))
}

fn weighted_q32(values: &[(u32, u64)]) -> Result<u64, SelectionError> {
    if values.is_empty() {
        return Err(SelectionError::InvalidEvidence("empty weighted score"));
    }
    let mut numerator = 0u128;
    let mut denominator = 0u128;
    for (weight, value) in values {
        if *weight == 0 {
            return Err(SelectionError::InvalidEvidence("zero weight"));
        }
        numerator = numerator
            .checked_add(
                u128::from(*weight)
                    .checked_mul(u128::from(*value))
                    .ok_or(SelectionError::Overflow)?,
            )
            .ok_or(SelectionError::Overflow)?;
        denominator = denominator
            .checked_add(u128::from(*weight))
            .ok_or(SelectionError::Overflow)?;
    }
    checked_ceil_div_u64(numerator, denominator)
}

fn checked_ceil_div_u64(numerator: u128, denominator: u128) -> Result<u64, SelectionError> {
    if denominator == 0 {
        return Err(SelectionError::InvalidEvidence("zero denominator"));
    }
    let quotient = numerator / denominator;
    let rounded = quotient
        .checked_add(u128::from(!numerator.is_multiple_of(denominator)))
        .ok_or(SelectionError::Overflow)?;
    u64::try_from(rounded).map_err(|_| SelectionError::Overflow)
}
