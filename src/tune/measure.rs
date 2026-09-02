use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{InvocationResult, RunnerFailure, TuneCase, TuneCaseRole};

const ORDER_DOMAIN: &[u8] = b"CK-TUNE-ORDER\0";
const WARMUP_ROWS: u32 = 3;
const MEASURED_ROWS: u32 = 20;
const MEASURED_CALLS: u8 = 3;

type StreamCoordinate = (String, [u8; 32]);
type RowAccumulator = BTreeMap<StreamCoordinate, Vec<MeasurementRow>>;

/// Frozen scheduling phases from decision schema 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum MeasurementPhase {
    CandidateSmoke = 1,
    SearchWarmup = 2,
    SearchMeasured = 3,
    ValidationOneWarmup = 4,
    ValidationOneMeasured = 5,
    ValidationTwoWarmup = 6,
    ValidationTwoMeasured = 7,
}

/// One immutable baseline or candidate channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementChannel {
    pub plan_digest: [u8; 32],
    pub primary_artifact_bytes: u64,
    pub choice_count: u32,
    pub is_baseline: bool,
}

impl MeasurementChannel {
    #[must_use]
    pub const fn baseline(plan_digest: [u8; 32], primary_artifact_bytes: u64) -> Self {
        Self {
            plan_digest,
            primary_artifact_bytes,
            choice_count: 0,
            is_baseline: true,
        }
    }

    #[must_use]
    pub const fn candidate(
        plan_digest: [u8; 32],
        primary_artifact_bytes: u64,
        choice_count: u32,
    ) -> Self {
        Self {
            plan_digest,
            primary_artifact_bytes,
            choice_count,
            is_baseline: false,
        }
    }
}

/// Exact coordinate of one attempted or skipped runner invocation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MeasurementCoordinate {
    pub phase: MeasurementPhase,
    pub round: u8,
    pub row: u32,
    pub case_id: String,
    pub plan_digest: [u8; 32],
    pub call: u8,
}

/// Canonical observable result for an invocation slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementEventOutcome {
    Completed(u64),
    TimedOut(u64),
    Skipped,
}

/// One canonical event-log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementEvent {
    pub coordinate: MeasurementCoordinate,
    pub outcome: MeasurementEventOutcome,
}

/// One scored row: exactly three calls and their minimum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementRow {
    pub ordinal: u32,
    pub permutation_key: [u8; 32],
    pub calls_ns: Vec<u64>,
    pub stored_minimum_ns: u64,
}

/// One complete 20-row stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementStream {
    pub phase: MeasurementPhase,
    pub round: u8,
    pub case_id: String,
    pub plan_digest: [u8; 32],
    pub iterations: u64,
    pub rows: Vec<MeasurementRow>,
    pub correctness_digest: [u8; 32],
}

/// Exact candidate timeout coordinate retained in a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutRecord {
    pub phase: MeasurementPhase,
    pub round: u8,
    pub row: u32,
    pub case_id: String,
    pub plan_digest: [u8; 32],
    pub call: u8,
    pub elapsed_ns: u64,
}

/// Complete log and all complete scored streams from one scheduler operation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MeasurementRun {
    pub events: Vec<MeasurementEvent>,
    pub streams: Vec<MeasurementStream>,
    pub timeouts: Vec<TimeoutRecord>,
}

/// Fail-closed measurement errors. Candidate timeouts are records, not errors.
#[derive(Debug, thiserror::Error)]
pub enum MeasurementFailure {
    #[error("invalid measurement configuration: {0}")]
    InvalidConfiguration(String),
    #[error("runner failure: {0}")]
    Runner(#[from] RunnerFailure),
    #[error("runner result does not match the frozen case")]
    Correctness,
}

/// Deterministic schema-1 state-machine scheduler.
#[derive(Debug, Clone)]
pub struct MeasurementScheduler {
    session_digest: [u8; 32],
    channels: Vec<MeasurementChannel>,
    cases: Vec<TuneCase>,
    iterations: BTreeMap<String, u64>,
    rejected: BTreeSet<[u8; 32]>,
}

impl MeasurementScheduler {
    /// Freezes and validates the baseline-first channel list and case calibration map.
    pub fn new(
        session_digest: [u8; 32],
        channels: Vec<MeasurementChannel>,
        mut cases: Vec<TuneCase>,
        calibrations: &[(&str, u64)],
    ) -> Result<Self, MeasurementFailure> {
        if channels.is_empty()
            || !channels[0].is_baseline
            || channels.iter().skip(1).any(|item| item.is_baseline)
            || channels
                .windows(2)
                .skip(1)
                .any(|items| items[0].plan_digest >= items[1].plan_digest)
        {
            return Err(MeasurementFailure::InvalidConfiguration(
                "channels must be one baseline followed by unique ascending plan digests".into(),
            ));
        }
        cases.sort_by(|left, right| left.id.cmp(&right.id));
        if cases.is_empty() || cases.windows(2).any(|items| items[0].id == items[1].id) {
            return Err(MeasurementFailure::InvalidConfiguration(
                "cases must be nonempty and have unique ids".into(),
            ));
        }
        let mut iterations = BTreeMap::new();
        for (case_id, count) in calibrations {
            if *count == 0 || iterations.insert((*case_id).to_string(), *count).is_some() {
                return Err(MeasurementFailure::InvalidConfiguration(
                    "calibrations must be unique and positive".into(),
                ));
            }
        }
        if cases.iter().any(|case| !iterations.contains_key(&case.id))
            || iterations.len() != cases.len()
        {
            return Err(MeasurementFailure::InvalidConfiguration(
                "calibration set must exactly match cases".into(),
            ));
        }
        Ok(Self {
            session_digest,
            channels,
            cases,
            iterations,
            rejected: BTreeSet::new(),
        })
    }

    /// Runs one correctness-smoke invocation per finalist and search case.
    pub fn run_smoke<F>(&mut self, mut invoke: F) -> Result<MeasurementRun, MeasurementFailure>
    where
        F: FnMut(
            &MeasurementCoordinate,
            &TuneCase,
            &MeasurementChannel,
            u64,
        ) -> Result<InvocationResult, RunnerFailure>,
    {
        let cases = self.partition_cases(TuneCaseRole::Search);
        let channels = self.channels.iter().skip(1).cloned().collect::<Vec<_>>();
        let mut run = MeasurementRun::default();
        for channel in channels {
            for case in &cases {
                let coordinate = MeasurementCoordinate {
                    phase: MeasurementPhase::CandidateSmoke,
                    round: 0,
                    row: 0,
                    case_id: case.id.clone(),
                    plan_digest: channel.plan_digest,
                    call: 1,
                };
                if self.rejected.contains(&channel.plan_digest) {
                    run.events.push(skipped(coordinate));
                    continue;
                }
                let iterations = self.iterations[&case.id];
                match invoke(&coordinate, case, &channel, iterations) {
                    Ok(result) => {
                        validate_result(&result, case, iterations)?;
                        run.events.push(completed(coordinate, result.elapsed_ns));
                    }
                    Err(RunnerFailure::CandidateTimeout(timeout)) => {
                        self.record_timeout(&mut run, &coordinate, &timeout)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        canonicalize_run(&mut run);
        Ok(run)
    }

    /// Runs three warmup and twenty measured search rows.
    pub fn run_search<F>(&mut self, invoke: F) -> Result<MeasurementRun, MeasurementFailure>
    where
        F: FnMut(
            &MeasurementCoordinate,
            &TuneCase,
            &MeasurementChannel,
            u64,
        ) -> Result<InvocationResult, RunnerFailure>,
    {
        self.run_matrix(
            TuneCaseRole::Search,
            MeasurementPhase::SearchWarmup,
            MeasurementPhase::SearchMeasured,
            0,
            self.channels.clone(),
            invoke,
        )
    }

    /// Runs one of the two complete validation matrices over baseline plus entrants.
    pub fn run_validation_round<F>(
        &mut self,
        round: u8,
        entrants: &[[u8; 32]],
        invoke: F,
    ) -> Result<MeasurementRun, MeasurementFailure>
    where
        F: FnMut(
            &MeasurementCoordinate,
            &TuneCase,
            &MeasurementChannel,
            u64,
        ) -> Result<InvocationResult, RunnerFailure>,
    {
        let (warmup, measured) = match round {
            1 => (
                MeasurementPhase::ValidationOneWarmup,
                MeasurementPhase::ValidationOneMeasured,
            ),
            2 => (
                MeasurementPhase::ValidationTwoWarmup,
                MeasurementPhase::ValidationTwoMeasured,
            ),
            _ => {
                return Err(MeasurementFailure::InvalidConfiguration(
                    "validation round must be one or two".into(),
                ));
            }
        };
        if entrants.windows(2).any(|items| items[0] >= items[1]) {
            return Err(MeasurementFailure::InvalidConfiguration(
                "validation entrants must be unique and sorted".into(),
            ));
        }
        let mut channels = vec![self.channels[0].clone()];
        for digest in entrants {
            let channel = self
                .channels
                .iter()
                .find(|channel| !channel.is_baseline && channel.plan_digest == *digest)
                .ok_or_else(|| {
                    MeasurementFailure::InvalidConfiguration(
                        "validation entrant is not a frozen finalist".into(),
                    )
                })?;
            channels.push(channel.clone());
        }
        self.run_matrix(
            TuneCaseRole::Validation,
            warmup,
            measured,
            round,
            channels,
            invoke,
        )
    }

    fn run_matrix<F>(
        &mut self,
        role: TuneCaseRole,
        warmup_phase: MeasurementPhase,
        measured_phase: MeasurementPhase,
        round: u8,
        channels: Vec<MeasurementChannel>,
        mut invoke: F,
    ) -> Result<MeasurementRun, MeasurementFailure>
    where
        F: FnMut(
            &MeasurementCoordinate,
            &TuneCase,
            &MeasurementChannel,
            u64,
        ) -> Result<InvocationResult, RunnerFailure>,
    {
        let cases = self.partition_cases(role);
        let mut run = MeasurementRun::default();
        self.run_rows(
            &cases,
            &channels,
            warmup_phase,
            round,
            WARMUP_ROWS,
            1,
            &mut invoke,
            &mut run,
            None,
        )?;
        let mut rows = RowAccumulator::new();
        self.run_rows(
            &cases,
            &channels,
            measured_phase,
            round,
            MEASURED_ROWS,
            MEASURED_CALLS,
            &mut invoke,
            &mut run,
            Some(&mut rows),
        )?;
        for case in &cases {
            for channel in &channels {
                if let Some(measured_rows) = rows.remove(&(case.id.clone(), channel.plan_digest))
                    && measured_rows.len() == usize::try_from(MEASURED_ROWS).unwrap_or(20)
                {
                    run.streams.push(MeasurementStream {
                        phase: measured_phase,
                        round,
                        case_id: case.id.clone(),
                        plan_digest: channel.plan_digest,
                        iterations: self.iterations[&case.id],
                        rows: measured_rows,
                        correctness_digest: case.expected_digest,
                    });
                }
            }
        }
        canonicalize_run(&mut run);
        Ok(run)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_rows<F>(
        &mut self,
        cases: &[TuneCase],
        channels: &[MeasurementChannel],
        phase: MeasurementPhase,
        round: u8,
        row_count: u32,
        call_count: u8,
        invoke: &mut F,
        run: &mut MeasurementRun,
        mut measured_rows: Option<&mut RowAccumulator>,
    ) -> Result<(), MeasurementFailure>
    where
        F: FnMut(
            &MeasurementCoordinate,
            &TuneCase,
            &MeasurementChannel,
            u64,
        ) -> Result<InvocationResult, RunnerFailure>,
    {
        for row in 0..row_count {
            let mut ordered_cases = cases.to_vec();
            rotate(&mut ordered_cases, self.case_rotation(phase, round, row));
            for case in ordered_cases {
                let key = self.permutation_key(phase, round, row, &case.id);
                let mut ordered_channels = channels.to_vec();
                let channel_rotation = rotation(&key, ordered_channels.len());
                rotate(&mut ordered_channels, channel_rotation);
                for channel in ordered_channels {
                    let mut calls = Vec::with_capacity(usize::from(call_count));
                    for call in 1..=call_count {
                        let coordinate = MeasurementCoordinate {
                            phase,
                            round,
                            row,
                            case_id: case.id.clone(),
                            plan_digest: channel.plan_digest,
                            call,
                        };
                        if self.rejected.contains(&channel.plan_digest) {
                            run.events.push(skipped(coordinate));
                            continue;
                        }
                        let iterations = self.iterations[&case.id];
                        match invoke(&coordinate, &case, &channel, iterations) {
                            Ok(result) => {
                                validate_result(&result, &case, iterations)?;
                                calls.push(result.elapsed_ns);
                                run.events.push(completed(coordinate, result.elapsed_ns));
                            }
                            Err(RunnerFailure::CandidateTimeout(timeout))
                                if !channel.is_baseline =>
                            {
                                self.record_timeout(run, &coordinate, &timeout)?;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    if call_count == MEASURED_CALLS && calls.len() == usize::from(MEASURED_CALLS) {
                        let minimum = *calls.iter().min().ok_or_else(|| {
                            MeasurementFailure::InvalidConfiguration("missing measured call".into())
                        })?;
                        measured_rows
                            .as_deref_mut()
                            .expect("measured row map")
                            .entry((case.id.clone(), channel.plan_digest))
                            .or_default()
                            .push(MeasurementRow {
                                ordinal: row,
                                permutation_key: key,
                                calls_ns: calls,
                                stored_minimum_ns: minimum,
                            });
                    }
                }
            }
        }
        Ok(())
    }

    fn record_timeout(
        &mut self,
        run: &mut MeasurementRun,
        coordinate: &MeasurementCoordinate,
        timeout: &super::CanonicalCandidateTimeout,
    ) -> Result<(), MeasurementFailure> {
        if timeout.case_id != coordinate.case_id
            || timeout.iterations != self.iterations[&coordinate.case_id]
            || timeout.timeout_ms == 0
            || timeout.elapsed_ns == 0
        {
            return Err(MeasurementFailure::InvalidConfiguration(
                "candidate timeout does not match its invocation coordinate".into(),
            ));
        }
        run.events.push(MeasurementEvent {
            coordinate: coordinate.clone(),
            outcome: MeasurementEventOutcome::TimedOut(timeout.elapsed_ns),
        });
        self.rejected.insert(coordinate.plan_digest);
        run.timeouts.push(TimeoutRecord {
            phase: coordinate.phase,
            round: coordinate.round,
            row: coordinate.row,
            case_id: coordinate.case_id.clone(),
            plan_digest: coordinate.plan_digest,
            call: coordinate.call,
            elapsed_ns: timeout.elapsed_ns,
        });
        Ok(())
    }

    fn partition_cases(&self, role: TuneCaseRole) -> Vec<TuneCase> {
        self.cases
            .iter()
            .filter(|case| case.role == role)
            .cloned()
            .collect()
    }

    fn permutation_key(
        &self,
        phase: MeasurementPhase,
        round: u8,
        row: u32,
        case_id: &str,
    ) -> [u8; 32] {
        order_digest(self.session_digest, phase, round, row, Some(case_id), false)
    }

    fn case_rotation(&self, phase: MeasurementPhase, round: u8, row: u32) -> usize {
        let key = order_digest(self.session_digest, phase, round, row, None, true);
        rotation(
            &key,
            self.partition_cases(match phase {
                MeasurementPhase::CandidateSmoke
                | MeasurementPhase::SearchWarmup
                | MeasurementPhase::SearchMeasured => TuneCaseRole::Search,
                _ => TuneCaseRole::Validation,
            })
            .len(),
        )
    }
}

/// Recomputes a search event log and its complete-stream set from immutable inputs.
///
/// Deleted, inserted, reordered, or forged slots are rejected even if a persisted
/// stream list was changed to agree with the mutation.
pub fn verify_search_measurement_run(
    session_digest: [u8; 32],
    channels: Vec<MeasurementChannel>,
    cases: Vec<TuneCase>,
    calibrations: &[(&str, u64)],
    run: &MeasurementRun,
) -> Result<(), MeasurementFailure> {
    let mut outcomes = BTreeMap::new();
    for event in &run.events {
        if outcomes
            .insert(event.coordinate.clone(), event.outcome)
            .is_some()
        {
            return Err(MeasurementFailure::InvalidConfiguration(
                "duplicate event-log coordinate".into(),
            ));
        }
    }
    let mut scheduler = MeasurementScheduler::new(session_digest, channels, cases, calibrations)?;
    let reconstructed = scheduler
        .run_search(
            |coordinate, case, _channel, iterations| match outcomes.get(coordinate) {
                Some(MeasurementEventOutcome::Completed(elapsed_ns)) => Ok(InvocationResult {
                    elapsed_ns: *elapsed_ns,
                    completed: iterations,
                    digest: case.expected_digest,
                }),
                Some(MeasurementEventOutcome::TimedOut(elapsed_ns)) => Err(
                    RunnerFailure::CandidateTimeout(super::CanonicalCandidateTimeout {
                        case_id: case.id.clone(),
                        iterations,
                        timeout_ms: 1,
                        elapsed_ns: *elapsed_ns,
                    }),
                ),
                Some(MeasurementEventOutcome::Skipped) | None => Err(RunnerFailure::Protocol),
            },
        )
        .map_err(|_| {
            MeasurementFailure::InvalidConfiguration(
                "event log cannot replay the immutable schedule".into(),
            )
        })?;
    if reconstructed != *run {
        return Err(MeasurementFailure::InvalidConfiguration(
            "measurement streams do not match the event log".into(),
        ));
    }
    Ok(())
}

fn validate_result(
    result: &InvocationResult,
    case: &TuneCase,
    iterations: u64,
) -> Result<(), MeasurementFailure> {
    if result.elapsed_ns == 0
        || result.completed != iterations
        || result.digest != case.expected_digest
    {
        return Err(MeasurementFailure::Correctness);
    }
    Ok(())
}

fn completed(coordinate: MeasurementCoordinate, elapsed_ns: u64) -> MeasurementEvent {
    MeasurementEvent {
        coordinate,
        outcome: MeasurementEventOutcome::Completed(elapsed_ns),
    }
}

fn skipped(coordinate: MeasurementCoordinate) -> MeasurementEvent {
    MeasurementEvent {
        coordinate,
        outcome: MeasurementEventOutcome::Skipped,
    }
}

fn canonicalize_run(run: &mut MeasurementRun) {
    run.streams.sort_by(|left, right| {
        (left.phase, left.round, &left.case_id, left.plan_digest).cmp(&(
            right.phase,
            right.round,
            &right.case_id,
            right.plan_digest,
        ))
    });
    run.timeouts.sort_by(|left, right| {
        (
            left.phase,
            left.round,
            left.row,
            &left.case_id,
            left.plan_digest,
            left.call,
        )
            .cmp(&(
                right.phase,
                right.round,
                right.row,
                &right.case_id,
                right.plan_digest,
                right.call,
            ))
    });
}

fn order_digest(
    session_digest: [u8; 32],
    phase: MeasurementPhase,
    round: u8,
    row: u32,
    case_id: Option<&str>,
    case_list: bool,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(ORDER_DOMAIN);
    hasher.update(session_digest);
    hasher.update([phase as u8]);
    hasher.update([round]);
    hasher.update(row.to_be_bytes());
    if let Some(case_id) = case_id {
        hasher.update(
            u32::try_from(case_id.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        hasher.update(case_id.as_bytes());
    }
    if case_list {
        hasher.update(1u32.to_be_bytes());
        hasher.update([0xff]);
    }
    hasher.finalize().into()
}

fn rotation(key: &[u8; 32], len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let value = u64::from_be_bytes(key[..8].try_into().expect("eight digest bytes"));
    usize::try_from(value % u64::try_from(len).unwrap_or(u64::MAX)).unwrap_or(0)
}

fn rotate<T>(values: &mut [T], amount: usize) {
    if !values.is_empty() {
        values.rotate_left(amount % values.len());
    }
}
