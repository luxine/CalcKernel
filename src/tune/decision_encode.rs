use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::{
    BaselineSessionSeed, CalibrationRecord, CandidateOutcome, CapturedWorkload, MeasurementStream,
    NonPublishableTuneTrial, RoundSummary, SearchFrontier, Selection, SessionDigestMaterial,
    TimeoutRecord, TuneArtifactKind, TuneArtifactRole, TuneBudget, TuneDecisionError,
    TunePlanChoice, TuningPlan, decode_tune_decision, derive_session_digest,
};
use crate::{TuningSpace, canonical_site, canonical_unit};

/// Compiler/source/target identity frozen into one completed decision.
#[derive(Debug, Clone)]
pub struct TuneDecisionIdentity {
    pub compiler_source: [u8; 32],
    pub llvm_bridge: [u8; 32],
    pub source_digest: [u8; 32],
    pub semantic_contract_digest: [u8; 32],
    pub pre_tune_kir_digest: [u8; 32],
    pub compilation_mode_digest: [u8; 32],
    pub output_kind: TuneArtifactKind,
    pub target_triple: String,
    pub target_cpu: String,
    pub target_features: Vec<String>,
    pub target_profile: String,
    pub profile_digest: Option<[u8; 32]>,
}

/// Immutable compile or measurement cache receipt recorded in a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuneRecordedCacheOrigin {
    pub reused: bool,
    pub key_digest: [u8; 32],
    pub entry_digest: [u8; 32],
}

/// One nonbaseline trial and its complete terminal evidence.
pub struct TuneDecisionCandidate<'a> {
    pub plan: &'a TuningPlan,
    pub trial: &'a NonPublishableTuneTrial,
    pub outcome: CandidateOutcome,
    pub streams: Vec<MeasurementStream>,
    pub timeout: Option<TimeoutRecord>,
    pub compile_reused: bool,
}

/// One role-tagged final output identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneDecisionOutput {
    pub role: TuneArtifactRole,
    pub logical_basename: String,
    pub content_digest: [u8; 32],
    pub content_bytes: u64,
}

/// Complete, already-derived material for canonical CKTUNE01 assembly.
pub struct TuneDecisionBuildInput<'a> {
    pub identity: TuneDecisionIdentity,
    pub budget: TuneBudget,
    pub workload: &'a CapturedWorkload,
    pub calibrations: &'a [CalibrationRecord],
    pub space: &'a TuningSpace,
    pub frontier: &'a SearchFrontier,
    pub baseline: &'a NonPublishableTuneTrial,
    pub baseline_streams: Vec<MeasurementStream>,
    pub baseline_compile_reused: bool,
    pub candidates: Vec<TuneDecisionCandidate<'a>>,
    pub round_one: &'a RoundSummary,
    pub round_two: &'a RoundSummary,
    pub selection: &'a Selection,
    pub measurement_reused: bool,
    pub measurement_cache_salt_digest: [u8; 32],
    pub outputs: Vec<TuneDecisionOutput>,
}

/// Encodes and immediately self-validates one completed decision.
pub fn encode_completed_tune_decision(
    input: &TuneDecisionBuildInput<'_>,
) -> Result<Vec<u8>, TuneDecisionError> {
    let mut candidates = input.candidates.iter().collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| candidate.plan.digest);
    let selected = if input.selection.selected_plan_digest == input.baseline.plan_digest() {
        input.baseline
    } else {
        candidates
            .iter()
            .find(|candidate| candidate.plan.digest == input.selection.selected_plan_digest)
            .map(|candidate| candidate.trial)
            .ok_or(TuneDecisionError::InvalidValue(
                "selected decision candidate",
            ))?
    };
    let frontier_digest = super::canonical_frontier_digest(input.space, input.frontier);
    let session_digest = derive_tune_session_digest(
        &input.identity,
        input.budget,
        input.workload,
        input.space,
        input.frontier,
        input.baseline,
    )
    .map_err(|_| TuneDecisionError::InvalidValue("Environment.sessionDigest"))?;
    let identity = identity_record(&input.identity);
    let contract = contract_record(input.budget);
    let workload = workload_record(input.workload);
    let frontier = frontier_record(input.space, input.frontier);
    let candidates_record = candidates_record(input, &identity, &candidates);
    let selection = selection_record(input, frontier_digest);
    let measurement_origin = derive_measurement_cache_origin(
        session_digest,
        input.measurement_cache_salt_digest,
        &candidates_record,
        &selection,
        input.measurement_reused,
    );
    let payloads = [
        identity,
        contract,
        workload,
        environment_record(input, session_digest),
        frontier,
        candidates_record,
        selection,
        replay_record(input, selected, frontier_digest, measurement_origin),
    ];
    let mut bytes = b"CKTUNE01".to_vec();
    bytes.extend_from_slice(&1u32.to_be_bytes());
    for (index, payload) in payloads.iter().enumerate() {
        field(
            &mut bytes,
            u16::try_from(index + 1).unwrap_or(u16::MAX),
            payload,
        );
    }
    let digest = hash_parts(b"CK-TUNING-DECISION\0", &[&bytes]);
    bytes.extend_from_slice(&digest);
    decode_tune_decision(&bytes)?;
    Ok(bytes)
}

/// Derives the exact measurement-order seed from the canonical schema-1
/// records, excluding calibration, measurements, paths, and cache origin.
pub fn derive_tune_session_digest(
    identity: &TuneDecisionIdentity,
    budget: TuneBudget,
    workload: &CapturedWorkload,
    space: &TuningSpace,
    frontier: &SearchFrontier,
    baseline: &NonPublishableTuneTrial,
) -> Result<[u8; 32], String> {
    let identity_record_bytes = record(&identity_record(identity));
    let contract_record_bytes = record(&contract_record(budget));
    let workload_record_bytes = record(&workload_record(workload));
    let environment_record_bytes = record(&environment_seed_record(identity));
    let frontier_record_bytes = record(&frontier_record(space, frontier));
    derive_session_digest(&SessionDigestMaterial {
        identity_record: &identity_record_bytes,
        contract_record: &contract_record_bytes,
        workload_record: &workload_record_bytes,
        environment_seed_record: &environment_record_bytes,
        frontier_record: &frontier_record_bytes,
        baseline: BaselineSessionSeed {
            plan_digest: baseline.plan_digest(),
            object_graph_digest: baseline.identity().object_graph_digest,
            link_recipe_digest: baseline.identity().link_recipe_digest,
            primary_artifact_bytes: baseline.primary_size(),
        },
    })
}

fn identity_record(identity: &TuneDecisionIdentity) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &text(env!("CARGO_PKG_VERSION")));
    field(&mut out, 2, &identity.compiler_source);
    field(&mut out, 3, &text("rustc-stable"));
    field(&mut out, 4, &text("LLVM 22.1.8"));
    field(&mut out, 5, &identity.llvm_bridge);
    for (tag, value) in (6u16..=15).zip([1u32, 1, 2, 3, 3, 3, 1, 5, 1, 1]) {
        field(&mut out, tag, &value.to_be_bytes());
    }
    for (tag, value) in [
        (16, identity.source_digest),
        (17, identity.semantic_contract_digest),
        (18, identity.pre_tune_kir_digest),
        (19, identity.compilation_mode_digest),
    ] {
        field(&mut out, tag, &value);
    }
    field(&mut out, 20, &[identity.output_kind as u8]);
    let mut target = Vec::new();
    field(&mut target, 1, &text(&identity.target_triple));
    field(&mut target, 2, &text(&identity.target_cpu));
    let mut features = identity.target_features.clone();
    features.sort();
    features.dedup();
    field(
        &mut target,
        3,
        &list(&features.iter().map(|value| text(value)).collect::<Vec<_>>()),
    );
    field(&mut target, 4, &text(&identity.target_profile));
    field(&mut out, 21, &record(&target));
    let profile = identity.profile_digest.map(|digest| {
        let mut profile = Vec::new();
        field(&mut profile, 1, &1u32.to_be_bytes());
        field(&mut profile, 2, &identity.compiler_source);
        field(&mut profile, 3, &identity.source_digest);
        field(&mut profile, 4, &identity.compilation_mode_digest);
        field(&mut profile, 5, &digest);
        field(&mut profile, 6, &32u64.to_be_bytes());
        record(&profile)
    });
    field(&mut out, 22, &optional(profile.as_deref()));
    out
}

fn contract_record(budget: TuneBudget) -> Vec<u8> {
    let mut out = Vec::new();
    for tag in 1..=5 {
        field(&mut out, tag, &1u32.to_be_bytes());
    }
    let preset = match budget {
        TuneBudget::Quick => 1,
        TuneBudget::Standard => 2,
        TuneBudget::Thorough => 3,
    };
    field(&mut out, 6, &[preset]);
    let contract = budget.contract();
    for (tag, value) in (7u16..=11).zip([
        contract.beam_width,
        contract.expansion_limit,
        contract.compile_attempt_limit,
        contract.measured_finalist_limit,
        contract.validation_entrant_limit,
    ]) {
        field(&mut out, tag, &value.to_be_bytes());
    }
    field(&mut out, 12, &contract.wall_clock_ms.to_be_bytes());
    for (tag, value) in (13u16..=14).zip([11u32, 10]) {
        field(&mut out, tag, &value.to_be_bytes());
    }
    for (tag, value) in (15u16..=16).zip([50_000_000u64, 250_000_000]) {
        field(&mut out, tag, &value.to_be_bytes());
    }
    for (tag, value) in (17u16..=31).zip([
        32u32, 3, 20, 3, 2_250, 4, 5, 6, 5, 16, 97, 100, 102, 100, 16,
    ]) {
        field(&mut out, tag, &value.to_be_bytes());
    }
    let digest = hash_parts(b"CK-TUNE-POLICY\0", &[&record(&out)]);
    field(&mut out, 32, &digest);
    out
}

fn workload_record(workload: &CapturedWorkload) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &workload.manifest_digest());
    field(&mut out, 2, &workload.runner_digest());
    field(
        &mut out,
        3,
        &u64::try_from(workload.runner_bytes().len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    field(
        &mut out,
        4,
        &list(
            &workload
                .runner_args()
                .iter()
                .map(|value| text(value))
                .collect::<Vec<_>>(),
        ),
    );
    let environment = workload
        .environment_identities()
        .into_iter()
        .map(|entry| {
            let mut item = Vec::new();
            field(&mut item, 1, &text(&entry.name));
            field(&mut item, 2, &entry.value_bytes.to_be_bytes());
            field(&mut item, 3, &entry.value_digest);
            record(&item)
        })
        .collect::<Vec<_>>();
    field(&mut out, 5, &list(&environment));
    field(&mut out, 6, &workload.invocation_timeout_ms().to_be_bytes());
    let inputs = workload
        .input_identities()
        .into_iter()
        .map(|entry| {
            let mut item = Vec::new();
            field(&mut item, 1, &text(&entry.logical_path));
            field(&mut item, 2, &entry.digest);
            field(&mut item, 3, &entry.bytes.to_be_bytes());
            record(&item)
        })
        .collect::<Vec<_>>();
    field(&mut out, 7, &list(&inputs));
    let cases = workload
        .case_identities()
        .iter()
        .map(|case| {
            let mut item = Vec::new();
            field(&mut item, 1, &text(&case.id));
            field(
                &mut item,
                2,
                &[match case.role {
                    super::TuneCaseRole::Search => 1,
                    super::TuneCaseRole::Validation => 2,
                }],
            );
            field(&mut item, 3, &case.seed.to_be_bytes());
            field(&mut item, 4, &case.weight.to_be_bytes());
            field(&mut item, 5, &case.expected_digest);
            record(&item)
        })
        .collect::<Vec<_>>();
    field(&mut out, 8, &list(&cases));
    out
}

fn environment_record(input: &TuneDecisionBuildInput<'_>, session: [u8; 32]) -> Vec<u8> {
    let mut out = environment_seed_record(&input.identity);
    let calibrations = input
        .calibrations
        .iter()
        .map(|calibration| {
            let mut item = Vec::new();
            field(&mut item, 1, &text(&calibration.case_id));
            field(&mut item, 2, &calibration.iterations.to_be_bytes());
            field(&mut item, 3, &u32::from(calibration.attempts).to_be_bytes());
            field(&mut item, 4, &calibration.elapsed_ns.to_be_bytes());
            field(
                &mut item,
                5,
                &calibration.confirmation_elapsed_ns.to_be_bytes(),
            );
            field(&mut item, 6, &[u8::from(calibration.overshoot)]);
            record(&item)
        })
        .collect::<Vec<_>>();
    field(&mut out, 17, &list(&calibrations));
    field(&mut out, 18, &session);
    field(&mut out, 19, &input.measurement_cache_salt_digest);
    out
}

fn environment_seed_record(identity: &TuneDecisionIdentity) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, value) in (1u16..=9).zip([
        std::env::consts::OS,
        "unavailable",
        "unavailable",
        std::env::consts::ARCH,
        "unavailable",
        "unavailable",
        "unavailable",
        "unavailable",
        "unavailable",
    ]) {
        field(&mut out, tag, &text(value));
    }
    field(
        &mut out,
        10,
        &list(
            &identity
                .target_features
                .iter()
                .map(|feature| text(feature))
                .collect::<Vec<_>>(),
        ),
    );
    field(&mut out, 11, &optional(None));
    field(
        &mut out,
        12,
        &std::thread::available_parallelism()
            .map(|value| u32::try_from(value.get()).unwrap_or(u32::MAX))
            .unwrap_or(1)
            .to_be_bytes(),
    );
    field(&mut out, 13, &optional(None));
    field(&mut out, 14, &text("monotonic"));
    field(&mut out, 15, &1u64.to_be_bytes());
    field(&mut out, 16, &text("default"));
    out
}

fn frontier_record(space: &TuningSpace, frontier: &SearchFrontier) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &space.digest);
    let sites = space
        .sites
        .iter()
        .map(|site| record(&canonical_site(site)))
        .collect::<Vec<_>>();
    field(&mut out, 2, &list(&sites));
    let units = space
        .units
        .iter()
        .map(|unit| record(&canonical_unit(unit, space.pre_state_digest)))
        .collect::<Vec<_>>();
    field(&mut out, 3, &list(&units));
    let expansions = frontier
        .expansions
        .iter()
        .map(|expansion| record(&super::frontier::canonical_expansion(expansion)))
        .collect::<Vec<_>>();
    field(&mut out, 4, &list(&expansions));
    out
}

fn candidates_record(
    input: &TuneDecisionBuildInput<'_>,
    identity: &[u8],
    candidates: &[&TuneDecisionCandidate<'_>],
) -> Vec<u8> {
    let mut out = Vec::new();
    let baseline_plan = TuningPlan::baseline();
    let baseline_origin = derive_compile_cache_origin_from_record(
        identity,
        baseline_plan.digest,
        input.baseline,
        input.baseline_compile_reused,
    );
    field(
        &mut out,
        1,
        &record(&candidate_record(
            &baseline_plan,
            input.baseline,
            CandidateOutcome::Baseline,
            &input.baseline_streams,
            None,
            baseline_origin,
        )),
    );
    let trials = candidates
        .iter()
        .map(|candidate| {
            record(&candidate_record(
                candidate.plan,
                candidate.trial,
                candidate.outcome,
                &candidate.streams,
                candidate.timeout.as_ref(),
                derive_compile_cache_origin_from_record(
                    identity,
                    candidate.plan.digest,
                    candidate.trial,
                    candidate.compile_reused,
                ),
            ))
        })
        .collect::<Vec<_>>();
    field(&mut out, 2, &list(&trials));
    out
}

fn candidate_record(
    plan: &TuningPlan,
    trial: &NonPublishableTuneTrial,
    outcome: CandidateOutcome,
    streams: &[MeasurementStream],
    timeout: Option<&TimeoutRecord>,
    origin: TuneRecordedCacheOrigin,
) -> Vec<u8> {
    let identity = trial.identity();
    let primary = identity
        .roles
        .iter()
        .find(|role| role.role == TuneArtifactRole::Primary)
        .expect("trial primary identity");
    let mut out = Vec::new();
    field(&mut out, 1, &plan.digest);
    field(
        &mut out,
        2,
        &list(&plan.choices.iter().map(plan_choice).collect::<Vec<_>>()),
    );
    field(&mut out, 3, &identity.object_graph_digest);
    field(&mut out, 4, &identity.link_recipe_digest);
    field(&mut out, 5, &primary.size.to_be_bytes());
    field(&mut out, 6, &[outcome as u8]);
    let diagnostic = match outcome {
        CandidateOutcome::SizeRejected => 3,
        CandidateOutcome::TimedOut => 4,
        _ => 0,
    };
    field(&mut out, 7, &(diagnostic as u16).to_be_bytes());
    let correctness = (!streams.is_empty()).then(|| correctness_digest(streams));
    field(
        &mut out,
        8,
        &optional(correctness.as_ref().map(<[u8; 32]>::as_slice)),
    );
    let mut streams = streams.to_vec();
    streams.sort_by_key(|stream| {
        (
            stream.phase,
            stream.round,
            stream.case_id.clone(),
            stream.plan_digest,
        )
    });
    field(
        &mut out,
        9,
        &list(&streams.iter().map(measurement_stream).collect::<Vec<_>>()),
    );
    field(&mut out, 10, &record(&cache_origin(origin)));
    let timeout = timeout.map(|timeout| record(&timeout_record(timeout)));
    field(&mut out, 11, &optional(timeout.as_deref()));
    field(&mut out, 12, &primary.digest);
    out
}

fn plan_choice(choice: &TunePlanChoice) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &choice.unit_id);
    field(&mut out, 2, &choice.variant_id);
    field(&mut out, 3, &[choice.class as u8]);
    field(&mut out, 4, &choice.pre_state_digest);
    field(&mut out, 5, &choice.post_state_digest);
    record(&out)
}

fn measurement_stream(stream: &MeasurementStream) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &[stream.phase as u8]);
    field(&mut out, 2, &[stream.round]);
    field(&mut out, 3, &text(&stream.case_id));
    field(&mut out, 4, &stream.plan_digest);
    field(&mut out, 5, &stream.iterations.to_be_bytes());
    let rows = stream
        .rows
        .iter()
        .map(|row| {
            let mut item = Vec::new();
            field(&mut item, 1, &row.ordinal.to_be_bytes());
            field(&mut item, 2, &row.permutation_key);
            let mut calls = u32::try_from(row.calls_ns.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes()
                .to_vec();
            for call in &row.calls_ns {
                calls.extend_from_slice(&call.to_be_bytes());
            }
            field(&mut item, 3, &calls);
            field(&mut item, 4, &row.stored_minimum_ns.to_be_bytes());
            record(&item)
        })
        .collect::<Vec<_>>();
    field(&mut out, 6, &list(&rows));
    field(&mut out, 7, &stream.correctness_digest);
    record(&out)
}

fn timeout_record(timeout: &TimeoutRecord) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &[timeout.phase as u8]);
    field(&mut out, 2, &[timeout.round]);
    field(&mut out, 3, &timeout.row.to_be_bytes());
    field(&mut out, 4, &text(&timeout.case_id));
    field(&mut out, 5, &[timeout.call]);
    field(&mut out, 6, &timeout.elapsed_ns.to_be_bytes());
    out
}

fn selection_record(input: &TuneDecisionBuildInput<'_>, frontier: [u8; 32]) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &record(&round_summary(input.round_one)));
    field(&mut out, 2, &record(&round_summary(input.round_two)));
    field(&mut out, 3, &input.selection.selected_plan_digest);
    field(&mut out, 4, &[input.selection.reason as u8]);
    let certificate = input.selection.certificate_plan_digest.map(|plan| {
        let correctness = correctness_digest(
            &input
                .candidates
                .iter()
                .flat_map(|candidate| candidate.streams.clone())
                .collect::<Vec<_>>(),
        );
        let selected = input
            .candidates
            .iter()
            .find(|candidate| candidate.plan.digest == plan)
            .expect("selected candidate");
        let mut value = Vec::new();
        for (tag, digest) in [
            (1, plan),
            (2, frontier),
            (3, policy_digest(input.budget)),
            (4, round_digest(input.round_one)),
            (5, round_digest(input.round_two)),
            (6, correctness),
            (7, selected.trial.identity().object_graph_digest),
            (8, selected.trial.identity().link_recipe_digest),
        ] {
            field(&mut value, tag, &digest);
        }
        record(&value)
    });
    field(&mut out, 5, &optional(certificate.as_deref()));
    out
}

fn round_summary(summary: &RoundSummary) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &[summary.round]);
    let plans = summary
        .plans
        .iter()
        .map(|plan| {
            let mut item = Vec::new();
            field(&mut item, 1, &plan.plan_digest);
            let medians = plan
                .case_medians
                .iter()
                .map(|median| {
                    let mut value = Vec::new();
                    field(&mut value, 1, &text(&median.case_id));
                    field(&mut value, 2, &median.baseline_ns.to_be_bytes());
                    field(&mut value, 3, &median.candidate_ns.to_be_bytes());
                    field(&mut value, 4, &median.ratio_q32.to_be_bytes());
                    record(&value)
                })
                .collect::<Vec<_>>();
            field(&mut item, 2, &list(&medians));
            field(&mut item, 3, &plan.aggregate_ratio_q32.to_be_bytes());
            field(&mut item, 4, &[u8::from(plan.stable)]);
            field(&mut item, 5, &[u8::from(plan.threshold_passed)]);
            field(&mut item, 6, &plan.paired_wins.to_be_bytes());
            record(&item)
        })
        .collect::<Vec<_>>();
    field(&mut out, 2, &list(&plans));
    field(
        &mut out,
        3,
        &list(
            &summary
                .ranked_plan_digests
                .iter()
                .map(|digest| digest.to_vec())
                .collect::<Vec<_>>(),
        ),
    );
    out
}

fn replay_record(
    input: &TuneDecisionBuildInput<'_>,
    selected: &NonPublishableTuneTrial,
    frontier: [u8; 32],
    measurement_origin: TuneRecordedCacheOrigin,
) -> Vec<u8> {
    let identity = selected.identity();
    let plan = input
        .candidates
        .iter()
        .find(|candidate| candidate.plan.digest == input.selection.selected_plan_digest)
        .map_or_else(TuningPlan::baseline, |candidate| candidate.plan.clone());
    let mut out = Vec::new();
    field(&mut out, 1, &frontier);
    field(
        &mut out,
        2,
        &plan
            .choices
            .first()
            .map_or(input.space.pre_state_digest, |choice| {
                choice.pre_state_digest
            }),
    );
    field(
        &mut out,
        3,
        &plan.choices.last().map_or_else(
            || selected.post_state_digest_bytes(),
            |choice| choice.post_state_digest,
        ),
    );
    field(&mut out, 4, &identity.object_graph_digest);
    field(&mut out, 5, &identity.link_recipe_digest);
    let mut outputs = input.outputs.clone();
    outputs.sort_by_key(|output| output.role);
    field(
        &mut out,
        6,
        &list(
            &outputs
                .iter()
                .map(|output| {
                    let mut item = Vec::new();
                    field(&mut item, 1, &[output.role as u8]);
                    field(&mut item, 2, &text(&output.logical_basename));
                    field(&mut item, 3, &output.content_digest);
                    field(&mut item, 4, &output.content_bytes.to_be_bytes());
                    record(&item)
                })
                .collect::<Vec<_>>(),
        ),
    );
    let identity_record = identity_record(&input.identity);
    let compile = if input.selection.selected_plan_digest == input.baseline.plan_digest() {
        derive_compile_cache_origin_from_record(
            &identity_record,
            input.baseline.plan_digest(),
            input.baseline,
            input.baseline_compile_reused,
        )
    } else {
        let candidate = input
            .candidates
            .iter()
            .find(|candidate| candidate.plan.digest == input.selection.selected_plan_digest)
            .expect("selected candidate");
        derive_compile_cache_origin_from_record(
            &identity_record,
            candidate.plan.digest,
            candidate.trial,
            candidate.compile_reused,
        )
    };
    field(&mut out, 7, &record(&cache_origin(compile)));
    field(&mut out, 8, &record(&cache_origin(measurement_origin)));
    let replay_digest = hash_parts(
        b"CK-TUNE-REPLAY-RESULT\0",
        &[
            &frontier,
            &plan.digest,
            &identity.object_graph_digest,
            &identity.link_recipe_digest,
        ],
    );
    field(&mut out, 9, &replay_digest);
    let choice = hash_parts(
        b"CK-TUNE-CHOICE\0",
        &[
            &input.identity.source_digest,
            &input.workload.manifest_digest(),
            &frontier,
            &[input.selection.reason as u8],
            &plan.digest,
            &identity.chosen_code_digest,
        ],
    );
    field(&mut out, 10, &choice);
    out
}

fn cache_origin(origin: TuneRecordedCacheOrigin) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &[if origin.reused { 2 } else { 1 }]);
    field(&mut out, 2, &origin.key_digest);
    field(&mut out, 3, &origin.entry_digest);
    out
}

fn derive_compile_cache_origin_from_record(
    identity: &[u8],
    plan_digest: [u8; 32],
    trial: &NonPublishableTuneTrial,
    reused: bool,
) -> TuneRecordedCacheOrigin {
    let mut key_material = Vec::new();
    field(&mut key_material, 1, &1u32.to_be_bytes());
    field(&mut key_material, 2, &record(identity));
    field(&mut key_material, 3, &plan_digest);
    let key_digest = hash_parts(b"CK-TUNE-COMPILE-KEY\0", &[&record(&key_material)]);
    let primary = trial
        .identity()
        .roles
        .iter()
        .find(|role| role.role == TuneArtifactRole::Primary)
        .expect("verified trial primary identity");
    let mut entry_material = Vec::new();
    field(&mut entry_material, 1, &key_digest);
    field(&mut entry_material, 2, &primary.digest);
    field(&mut entry_material, 3, &primary.size.to_be_bytes());
    field(
        &mut entry_material,
        4,
        &trial.identity().object_graph_digest,
    );
    field(&mut entry_material, 5, &trial.identity().link_recipe_digest);
    TuneRecordedCacheOrigin {
        reused,
        key_digest,
        entry_digest: hash_parts(b"CK-TUNE-COMPILE-ENTRY\0", &[&record(&entry_material)]),
    }
}

fn derive_measurement_cache_origin(
    session_digest: [u8; 32],
    salt_digest: [u8; 32],
    candidates: &[u8],
    selection: &[u8],
    reused: bool,
) -> TuneRecordedCacheOrigin {
    let mut key_material = Vec::new();
    field(&mut key_material, 1, &1u32.to_be_bytes());
    field(&mut key_material, 2, &session_digest);
    field(&mut key_material, 3, &salt_digest);
    let key_digest = hash_parts(b"CK-TUNE-MEASUREMENT-KEY\0", &[&record(&key_material)]);
    let mut entry_material = Vec::new();
    field(&mut entry_material, 1, &key_digest);
    field(&mut entry_material, 2, &record(candidates));
    field(&mut entry_material, 3, &record(selection));
    TuneRecordedCacheOrigin {
        reused,
        key_digest,
        entry_digest: hash_parts(b"CK-TUNE-MEASUREMENT-ENTRY\0", &[&record(&entry_material)]),
    }
}

fn correctness_digest(streams: &[MeasurementStream]) -> [u8; 32] {
    let mut by_case = BTreeMap::new();
    for stream in streams {
        by_case.insert(stream.case_id.as_bytes(), stream.correctness_digest);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-CORRECTNESS\0");
    hasher.update(
        u32::try_from(by_case.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for (case, digest) in by_case {
        hasher.update(u32::try_from(case.len()).unwrap_or(u32::MAX).to_be_bytes());
        hasher.update(case);
        hasher.update(digest);
    }
    hasher.finalize().into()
}

fn round_digest(summary: &RoundSummary) -> [u8; 32] {
    hash_parts(
        b"CK-TUNE-VALIDATION-ROUND\0",
        &[&record(&round_summary(summary))],
    )
}

fn policy_digest(budget: TuneBudget) -> [u8; 32] {
    let contract = contract_record(budget);
    let fields_without_digest = &contract[..contract.len() - 38];
    hash_parts(b"CK-TUNE-POLICY\0", &[&record(fields_without_digest)])
}

fn field(output: &mut Vec<u8>, tag: u16, payload: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
}

fn text(value: &str) -> Vec<u8> {
    let mut bytes = u32::try_from(value.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(value.as_bytes());
    bytes
}

fn record(fields: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(fields.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(fields);
    bytes
}

fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = u32::try_from(items.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    for item in items {
        bytes.extend_from_slice(item);
    }
    bytes
}

fn optional(value: Option<&[u8]>) -> Vec<u8> {
    let mut bytes = vec![u8::from(value.is_some())];
    if let Some(value) = value {
        bytes.extend_from_slice(value);
    }
    bytes
}

fn hash_parts(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}
