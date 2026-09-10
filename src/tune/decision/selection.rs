//! Contract-2 selection checks derived independently from the retained raw records.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    TuneDecisionError, append_field, copy_digest, domain_hash, parse_bool, parse_digest_values,
    parse_list_header, parse_record_envelope, parse_record_fields, parse_record_prefix, parse_text,
    parse_u32, parse_u64, parse_u64_list, record_domain_hash, record_envelope, require_exact_end,
};

type Digest = [u8; 32];
type Check<T> = Result<T, TuneDecisionError>;
const ONE: u128 = 1 << 32;

struct Case<'a> {
    id: &'a str,
    role: u8,
    weight: u32,
    digest: Digest,
    iterations: u64,
}

struct Samples {
    values: [u64; 20],
    median: u64,
}

struct Candidate<'a> {
    fields: Vec<&'a [u8]>,
    digest: Digest,
    bytes: u64,
    choices: u32,
    outcome: u8,
    timeout_phase: Option<u8>,
    samples: BTreeMap<(u8, &'a str), Samples>,
}

#[derive(Clone, Copy)]
struct Score {
    value: u64,
    bytes: u64,
    choices: u32,
    digest: Digest,
}

fn invalid(field: &'static str) -> TuneDecisionError {
    TuneDecisionError::InvalidValue(field)
}

pub(super) fn validate(top: &[&[u8]]) -> Check<()> {
    let contract = parse_record_fields(top[1], 1..=32, "Contract")?;
    let cases = cases(top)?;
    let encoded = parse_record_fields(top[5], 1..=2, "Candidates")?;
    let baseline = candidate(parse_record_envelope(encoded[0], "Candidate")?, &cases)?;
    if baseline.outcome != 1 || baseline.timeout_phase.is_some() {
        return Err(invalid("Candidate.baseline outcome"));
    }
    let trials = records(encoded[1], 32, "Candidates.trials")?
        .into_iter()
        .map(|bytes| candidate(bytes, &cases))
        .collect::<Check<Vec<_>>>()?;
    if trials
        .windows(2)
        .any(|pair| pair[0].digest >= pair[1].digest)
        || trials
            .iter()
            .any(|trial| trial.digest == baseline.digest || trial.outcome == 1)
    {
        return Err(invalid("Candidates.trials selection order"));
    }
    check_session(top, &baseline)?;
    let search_cases = cases
        .values()
        .filter(|case| case.role == 1)
        .collect::<Vec<_>>();
    let validation_cases = cases
        .values()
        .filter(|case| case.role == 2)
        .collect::<Vec<_>>();
    for case in &search_cases {
        sample(&baseline, 3, case.id)?;
    }
    for case in &validation_cases {
        sample(&baseline, 5, case.id)?;
        sample(&baseline, 7, case.id)?;
    }
    let mut search_scores = Vec::new();
    for trial in &trials {
        let survives_search =
            trial.outcome >= 5 || trial.timeout_phase.is_some_and(|phase| phase >= 4);
        if survives_search {
            let value = aggregate(&baseline, trial, 3, &search_cases, None)?;
            search_scores.push(Score {
                value,
                bytes: trial.bytes,
                choices: trial.choices,
                digest: trial.digest,
            });
        }
    }
    let mut entrants = rank(search_scores);
    entrants.truncate(
        usize::try_from(parse_u32(contract[10], "Contract.validationEntrantLimit")?)
            .map_err(|_| invalid("Contract.validationEntrantLimit"))?,
    );
    let entrants = entrants.into_iter().collect::<BTreeSet<_>>();
    let mut active = Vec::new();
    for trial in &trials {
        let entered = entrants.contains(&trial.digest);
        let invalid_membership = if entered {
            trial.outcome == 5
        } else {
            trial.outcome >= 6
                || trial.timeout_phase.is_some_and(|phase| phase >= 4)
                || trial.samples.keys().any(|(phase, _)| *phase >= 5)
        };
        if invalid_membership {
            return Err(invalid("Candidate.validation entrant membership"));
        }
        if entered && trial.timeout_phase.is_none() {
            active.push(trial);
        }
    }
    let selection = parse_record_fields(top[6], 1..=5, "Selection")?;
    let first = round(selection[0], 1, &baseline, &active, &validation_cases)?;
    let second = round(selection[1], 2, &baseline, &active, &validation_cases)?;
    let (reason, selected) = if entrants.is_empty() {
        (2, baseline.digest)
    } else if first.is_empty() || second.is_empty() {
        (3, baseline.digest)
    } else if first[0] == second[0] {
        (1, first[0])
    } else {
        (4, baseline.digest)
    };
    if selection[3] != [reason] || selection[2] != selected {
        return Err(invalid("Selection.derived result"));
    }
    for trial in &active {
        let expected = if reason == 3 {
            6
        } else if reason == 1 && trial.digest == selected {
            8
        } else {
            7
        };
        if trial.outcome != expected {
            return Err(invalid("Candidate.derived outcome"));
        }
    }
    if reason == 1 {
        let selected = active
            .iter()
            .find(|trial| trial.digest == selected)
            .ok_or_else(|| invalid("Selection.selected candidate"))?;
        certificate(top, &selection, &contract, selected, &trials)?;
    }
    Ok(())
}

fn cases<'a>(top: &[&'a [u8]]) -> Check<BTreeMap<&'a str, Case<'a>>> {
    let workload = parse_record_fields(top[2], 1..=8, "Workload")?;
    let environment = parse_record_fields(top[3], 1..=19, "Environment")?;
    let mut calibrations = BTreeMap::new();
    for bytes in records(environment[16], 16, "Environment.calibrations")? {
        let fields = parse_record_fields(bytes, 1..=6, "Calibration")?;
        let id = parse_text(fields[0], "Calibration.caseId")?;
        if calibrations
            .insert(id, parse_u64(fields[1], "Calibration.iterations")?)
            .is_some()
        {
            return Err(invalid("Calibration.case set"));
        }
    }
    let mut cases = BTreeMap::new();
    for bytes in records(workload[7], 16, "Workload.cases")? {
        let fields = parse_record_fields(bytes, 1..=5, "CaseIdentity")?;
        let id = parse_text(fields[0], "CaseIdentity.id")?;
        let case = Case {
            id,
            role: fields[1][0],
            weight: parse_u32(fields[3], "CaseIdentity.weight")?,
            digest: copy_digest(fields[4], "CaseIdentity.expectedDigest")?,
            iterations: *calibrations
                .get(id)
                .ok_or_else(|| invalid("Calibration.case set"))?,
        };
        if cases.insert(id, case).is_some() {
            return Err(invalid("CaseIdentity.id set"));
        }
    }
    if cases.len() != calibrations.len() {
        return Err(invalid("Calibration.case set"));
    }
    Ok(cases)
}

fn candidate<'a>(bytes: &'a [u8], cases: &BTreeMap<&str, Case<'_>>) -> Check<Candidate<'a>> {
    let fields = parse_record_fields(bytes, 1..=12, "Candidate")?;
    let digest = copy_digest(fields[0], "Candidate.planDigest")?;
    let timeout_phase = if fields[10][0] == 0 {
        None
    } else {
        let timeout = parse_record_fields(
            parse_record_envelope(&fields[10][1..], "TimeoutRecord")?,
            1..=6,
            "TimeoutRecord",
        )?;
        Some(timeout[0][0])
    };
    let outcome = fields[5][0];
    if (outcome == 4) != timeout_phase.is_some() {
        return Err(invalid("Candidate.timeout outcome"));
    }
    let mut samples = BTreeMap::new();
    for bytes in records(fields[8], 48, "Candidate.streams")? {
        let stream = parse_record_fields(bytes, 1..=7, "MeasurementStream")?;
        let phase = stream[0][0];
        let id = parse_text(stream[2], "MeasurementStream.caseId")?;
        let case = cases
            .get(id)
            .ok_or_else(|| invalid("MeasurementStream.caseId"))?;
        let expected = match phase {
            3 => (1, 0),
            5 => (2, 1),
            7 => (2, 2),
            _ => return Err(invalid("MeasurementStream.phase")),
        };
        if (case.role, stream[1][0]) != expected
            || stream[3] != digest
            || stream[6] != case.digest
            || parse_u64(stream[4], "MeasurementStream.iterations")? != case.iterations
            || timeout_phase.is_some_and(|timeout| phase > timeout)
        {
            return Err(invalid("MeasurementStream.selection identity"));
        }
        let rows = records(stream[5], 20, "MeasurementStream.rows")?;
        if rows.len() != 20 {
            return Err(invalid("MeasurementStream.rows"));
        }
        let mut values = [0; 20];
        for (ordinal, bytes) in rows.into_iter().enumerate() {
            let row = parse_record_fields(bytes, 1..=4, "MeasurementRow")?;
            let calls = parse_u64_list(row[2], 3, "MeasurementRow.callsNs")?;
            let minimum = calls
                .iter()
                .copied()
                .min()
                .ok_or_else(|| invalid("MeasurementRow.callsNs"))?;
            if parse_u32(row[0], "MeasurementRow.ordinal")? as usize != ordinal
                || minimum == 0
                || parse_u64(row[3], "MeasurementRow.storedMinimumNs")? != minimum
            {
                return Err(invalid("MeasurementRow.selection evidence"));
            }
            values[ordinal] = minimum;
        }
        let mut sorted = values;
        sorted.sort_unstable();
        let median = sorted[10];
        let stable = values
            .iter()
            .filter(|value| {
                let scaled = u128::from(**value) * 5;
                u128::from(median) * 4 <= scaled && scaled <= u128::from(median) * 6
            })
            .count()
            >= 16;
        if !stable {
            return Err(invalid("MeasurementStream.selection stability"));
        }
        if samples
            .insert((phase, id), Samples { values, median })
            .is_some()
        {
            return Err(invalid("MeasurementStream.selection duplicate"));
        }
    }
    if matches!(outcome, 2 | 3) && !samples.is_empty() {
        return Err(invalid("Candidate.unmeasured streams"));
    }
    Ok(Candidate {
        digest,
        bytes: parse_u64(fields[4], "Candidate.primaryArtifactBytes")?,
        choices: parse_list_header(fields[1], 64, "Candidate.choices")?.0,
        outcome,
        timeout_phase,
        fields,
        samples,
    })
}

fn sample<'a>(candidate: &'a Candidate<'_>, phase: u8, case: &'a str) -> Check<&'a Samples> {
    candidate
        .samples
        .get(&(phase, case))
        .ok_or_else(|| invalid("Candidate.incomplete selection streams"))
}

fn ratio(candidate: u64, baseline: u64) -> Check<u64> {
    if baseline == 0 || candidate == 0 {
        return Err(invalid("Selection.zero timing"));
    }
    let value = (u128::from(candidate) * ONE).div_ceil(u128::from(baseline));
    u64::try_from(value).map_err(|_| invalid("Selection.ratio overflow"))
}

fn aggregate(
    baseline: &Candidate<'_>,
    candidate: &Candidate<'_>,
    phase: u8,
    cases: &[&Case<'_>],
    ordinal: Option<usize>,
) -> Check<u64> {
    let mut numerator = 0u128;
    let mut denominator = 0u128;
    for case in cases {
        let before = sample(baseline, phase, case.id)?;
        let after = sample(candidate, phase, case.id)?;
        let (before, after) = ordinal.map_or((before.median, after.median), |row| {
            (before.values[row], after.values[row])
        });
        let weighted = u128::from(case.weight) * u128::from(ratio(after, before)?);
        numerator = numerator
            .checked_add(weighted)
            .ok_or_else(|| invalid("Selection.score overflow"))?;
        denominator += u128::from(case.weight);
    }
    if denominator == 0 {
        return Err(invalid("Selection.empty case partition"));
    }
    u64::try_from(numerator.div_ceil(denominator)).map_err(|_| invalid("Selection.score overflow"))
}

// Independent implementation: partition the score-sorted suffix before resolving ties.
fn rank(mut scores: Vec<Score>) -> Vec<Digest> {
    scores.sort_by_key(|score| (score.value, score.digest));
    let mut suffix = scores.as_mut_slice();
    while let Some(first) = suffix.first() {
        let anchor = first.value;
        let length = suffix
            .partition_point(|score| (u128::from(score.value) - u128::from(anchor)) * 100 <= ONE);
        let (group, rest) = suffix.split_at_mut(length);
        group.sort_by_key(|score| (score.bytes, score.choices, score.digest));
        suffix = rest;
    }
    scores.into_iter().map(|score| score.digest).collect()
}

fn round(
    bytes: &[u8],
    number: u8,
    baseline: &Candidate<'_>,
    active: &[&Candidate<'_>],
    cases: &[&Case<'_>],
) -> Check<Vec<Digest>> {
    let fields = parse_record_fields(
        parse_record_envelope(bytes, "RoundSummary")?,
        1..=3,
        "RoundSummary",
    )?;
    let plans = records(fields[1], 4, "RoundSummary.plans")?;
    if fields[0] != [number] || plans.len() != active.len() {
        return Err(invalid("RoundSummary.derived plan set"));
    }
    let phase = if number == 1 { 5 } else { 7 };
    let mut qualifiers = Vec::new();
    for (bytes, candidate) in plans.into_iter().zip(active) {
        let plan = parse_record_fields(bytes, 1..=6, "RoundPlan")?;
        if plan[0] != candidate.digest {
            return Err(invalid("RoundSummary.derived plan set"));
        }
        let medians = records(plan[1], 16, "RoundPlan.caseMedians")?;
        if medians.len() != cases.len() {
            return Err(invalid("RoundPlan.derived case set"));
        }
        let mut cases_passed = true;
        for (bytes, case) in medians.into_iter().zip(cases) {
            let fields = parse_record_fields(bytes, 1..=4, "CaseMedian")?;
            let before = sample(baseline, phase, case.id)?.median;
            let after = sample(candidate, phase, case.id)?.median;
            let ratio = ratio(after, before)?;
            if parse_text(fields[0], "CaseMedian.caseId")? != case.id
                || parse_u64(fields[1], "CaseMedian.baselineNs")? != before
                || parse_u64(fields[2], "CaseMedian.candidateNs")? != after
                || parse_u64(fields[3], "CaseMedian.ratioQ32")? != ratio
            {
                return Err(invalid("CaseMedian.derived values"));
            }
            cases_passed &= u128::from(ratio) * 100 <= ONE * 102;
        }
        let score = aggregate(baseline, candidate, phase, cases, None)?;
        let mut wins = 0;
        for ordinal in 0..20 {
            wins += u32::from(
                u128::from(aggregate(baseline, candidate, phase, cases, Some(ordinal))?) < ONE,
            );
        }
        let qualifies = u128::from(score) * 100 <= ONE * 97 && cases_passed && wins >= 16;
        if parse_u64(plan[2], "RoundPlan.aggregateRatioQ32")? != score
            || !parse_bool(plan[3], "RoundPlan.stable")?
            || parse_bool(plan[4], "RoundPlan.thresholdPassed")? != qualifies
            || parse_u32(plan[5], "RoundPlan.pairedWins")? != wins
        {
            return Err(invalid("RoundPlan.derived values"));
        }
        if qualifies {
            qualifiers.push(Score {
                value: score,
                bytes: candidate.bytes,
                choices: candidate.choices,
                digest: candidate.digest,
            });
        }
    }
    let ranked = rank(qualifiers);
    if parse_digest_values(fields[2], 4, "RoundSummary.rankedPlanDigests")? != ranked {
        return Err(invalid("RoundSummary.derived ranking"));
    }
    Ok(ranked)
}

fn check_session(top: &[&[u8]], baseline: &Candidate<'_>) -> Check<()> {
    let environment = parse_record_fields(top[3], 1..=19, "Environment")?;
    let mut seed = Vec::new();
    for (index, field) in environment[..16].iter().enumerate() {
        append_field(&mut seed, index as u16 + 1, field);
    }
    let mut base = Vec::new();
    for (index, field) in [
        baseline.fields[0],
        baseline.fields[2],
        baseline.fields[3],
        baseline.fields[4],
    ]
    .into_iter()
    .enumerate()
    {
        append_field(&mut base, index as u16 + 1, field);
    }
    let mut material = Vec::new();
    for (index, fields) in [top[0], top[1], top[2], &seed, top[4], &base]
        .into_iter()
        .enumerate()
    {
        append_field(&mut material, index as u16 + 1, &record_envelope(fields));
    }
    if environment[17] != record_domain_hash(b"CK-TUNE-SESSION\0", &material) {
        return Err(invalid("Environment.derived session digest"));
    }
    Ok(())
}

fn certificate(
    top: &[&[u8]],
    selection: &[&[u8]],
    contract: &[&[u8]],
    selected: &Candidate<'_>,
    trials: &[Candidate<'_>],
) -> Check<()> {
    let fields = parse_record_fields(
        parse_record_envelope(&selection[4][1..], "Certificate")?,
        1..=8,
        "Certificate",
    )?;
    let replay = parse_record_fields(top[7], 1..=10, "Replay")?;
    let mut correctness = BTreeMap::new();
    let cases = cases(top)?;
    for trial in trials {
        for (_, id) in trial.samples.keys() {
            correctness.insert(*id, cases[id].digest);
        }
    }
    let mut material = (correctness.len() as u32).to_be_bytes().to_vec();
    for (id, digest) in correctness {
        material.extend_from_slice(&(id.len() as u32).to_be_bytes());
        material.extend_from_slice(id.as_bytes());
        material.extend_from_slice(&digest);
    }
    if fields[0] != selected.digest
        || fields[1] != replay[0]
        || fields[2] != contract[31]
        || fields[3] != domain_hash(b"CK-TUNE-VALIDATION-ROUND\0", selection[0])
        || fields[4] != domain_hash(b"CK-TUNE-VALIDATION-ROUND\0", selection[1])
        || fields[5] != domain_hash(b"CK-TUNE-CORRECTNESS\0", &material)
        || fields[6] != selected.fields[2]
        || fields[7] != selected.fields[3]
    {
        return Err(invalid("Certificate.derived values"));
    }
    Ok(())
}

fn records<'a>(bytes: &'a [u8], maximum: u32, context: &'static str) -> Check<Vec<&'a [u8]>> {
    let (count, mut offset) = parse_list_header(bytes, maximum, context)?;
    let mut values = Vec::new();
    for _ in 0..count {
        let remaining = bytes
            .get(offset..)
            .ok_or(TuneDecisionError::Truncated(context))?;
        let (value, length) = parse_record_prefix(remaining, context)?;
        values.push(value);
        offset = offset
            .checked_add(length)
            .ok_or(TuneDecisionError::ResourceLimit(context))?;
    }
    require_exact_end(bytes, offset, context)?;
    Ok(values)
}
