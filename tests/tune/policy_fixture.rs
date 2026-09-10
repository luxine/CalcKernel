//! Test-only synthetic measurements and bytes; no artifact or runner is executed.

use std::{collections::BTreeMap, fs, path::PathBuf};

use calckernel::*;
use sha2::{Digest, Sha256};

use super::super::{support, temp};

pub fn encoded() -> Vec<u8> {
    encoded_for_reason(SelectionReason::Tuned)
}

pub fn encoded_for_reason(reason: SelectionReason) -> Vec<u8> {
    let state = super::super::trial::state();
    let space = enumerate_tuning_space(&state).expect("space");
    let budget = TuneBudget::Standard;
    let frontier = run_deterministic_search(&state, &space, budget).expect("frontier");
    assert!(
        frontier.compile_selection.len() >= 2,
        "fixture needs competing plans"
    );
    let request = |index: usize| {
        TuneTrialBuildRequest::new(
            TuneArtifactKind::Executable,
            vec![
                index as u8;
                if reason == SelectionReason::NoCandidate && index > 0 {
                    2048
                } else {
                    1024 - index
                }
            ],
            None,
            None,
            vec![("program.o".into(), vec![index as u8; 8])],
            vec!["link".into()],
        )
    };
    let baseline = compile_tune_trial(&state, &space, &TuningPlan::baseline(), request(0))
        .expect("baseline trial");
    let mut trials = frontier
        .compile_selection
        .iter()
        .enumerate()
        .map(|(index, plan)| {
            compile_tune_trial(&state, &space, plan, request(index + 1)).expect("trial")
        })
        .collect::<Vec<_>>();
    trials.sort_by_key(NonPublishableTuneTrial::plan_digest);
    let finalists = select_size_valid_finalists(&baseline, &trials, budget).expect("finalists");
    let size_rejected = finalists
        .size_rejected
        .iter()
        .map(NonPublishableTuneTrial::plan_digest)
        .collect::<Vec<_>>();
    let mut eligible = finalists.eligible;
    eligible.sort_by_key(NonPublishableTuneTrial::plan_digest);
    let ranks = eligible
        .iter()
        .map(|trial| CandidateRank {
            plan_digest: trial.plan_digest(),
            primary_artifact_bytes: trial.primary_size(),
            choice_count: frontier
                .compile_selection
                .iter()
                .find(|plan| plan.digest == trial.plan_digest())
                .expect("plan")
                .choices
                .len() as u32,
        })
        .collect::<Vec<_>>();
    let workload = workload();
    let cases = workload.case_identities();
    let identity = TuneDecisionIdentity {
        compiler_source: [2; 32],
        llvm_bridge: [3; 32],
        source_digest: [4; 32],
        semantic_contract_digest: [5; 32],
        pre_tune_kir_digest: tuning_pre_kir_digest(&state).expect("pre-KIR digest"),
        compilation_mode_digest: [6; 32],
        output_kind: TuneArtifactKind::Executable,
        target_triple: "portable-test".into(),
        target_cpu: "test".into(),
        target_features: vec![],
        target_profile: "test-profile".into(),
        profile_digest: None,
    };
    let session =
        derive_tune_session_digest(&identity, budget, &workload, &space, &frontier, &baseline)
            .expect("session");
    let mut channels = vec![MeasurementChannel::baseline(
        baseline.plan_digest(),
        baseline.primary_size(),
    )];
    channels.extend(ranks.iter().map(|rank| {
        MeasurementChannel::candidate(
            rank.plan_digest,
            rank.primary_artifact_bytes,
            rank.choice_count,
        )
    }));
    let timings = ranks
        .iter()
        .enumerate()
        .map(|(index, rank)| (rank.plan_digest, 21_900_000 + index as u64 * 100_000))
        .collect::<BTreeMap<_, _>>();
    let iterations = cases
        .iter()
        .map(|case| (case.id.as_str(), 1))
        .collect::<Vec<_>>();
    let mut scheduler = MeasurementScheduler::new(session, channels, cases.to_vec(), &iterations)
        .expect("scheduler");
    let invoke = |coordinate: &MeasurementCoordinate,
                  case: &TuneCase,
                  channel: &MeasurementChannel,
                  iterations| {
        let validation_ns = if coordinate.round > 0 {
            match reason {
                SelectionReason::ValidationThreshold => Some(110_000_000),
                SelectionReason::ValidationDisagreement => {
                    let best = if coordinate.round == 1 {
                        ranks.first()
                    } else {
                        ranks.last()
                    }
                    .expect("competing entrants");
                    // Distinct per-case times also exercise weighted normalization.
                    Some(if channel.plan_digest == best.plan_digest {
                        if case.id.ends_with('b') {
                            30_000_000
                        } else {
                            20_000_000
                        }
                    } else {
                        50_000_000
                    })
                }
                _ => None,
            }
        } else {
            None
        };
        Ok(InvocationResult {
            elapsed_ns: if channel.is_baseline {
                100_000_000
            } else {
                validation_ns.unwrap_or_else(|| timings[&channel.plan_digest])
            },
            completed: iterations,
            digest: case.expected_digest,
        })
    };
    scheduler.run_smoke(invoke).expect("smoke");
    let search = scheduler.run_search(invoke).expect("search");
    let entrants = derive_search_entrants(
        baseline.plan_digest(),
        &ranks,
        cases,
        &search.streams,
        budget.contract().validation_entrant_limit,
    )
    .expect("entrants");
    let mut entrant_ids = entrants
        .iter()
        .map(|entry| entry.plan_digest)
        .collect::<Vec<_>>();
    entrant_ids.sort();
    let entrant_ranks = ranks
        .iter()
        .copied()
        .filter(|rank| entrant_ids.contains(&rank.plan_digest))
        .collect::<Vec<_>>();
    let first = scheduler
        .run_validation_round(1, &entrant_ids, invoke)
        .expect("first validation");
    let second = scheduler
        .run_validation_round(2, &entrant_ids, invoke)
        .expect("second validation");
    let one = derive_round_summary(
        1,
        baseline.plan_digest(),
        &entrant_ranks,
        cases,
        &first.streams,
    )
    .expect("one");
    let two = derive_round_summary(
        2,
        baseline.plan_digest(),
        &entrant_ranks,
        cases,
        &second.streams,
    )
    .expect("two");
    let selection = derive_selection(
        baseline.plan_digest(),
        &entrant_ids
            .iter()
            .copied()
            .map(SelectionEntrant::active)
            .collect::<Vec<_>>(),
        &one,
        &two,
    )
    .expect("selection");
    assert_eq!(selection.reason, reason);
    let streams = search
        .streams
        .into_iter()
        .chain(first.streams)
        .chain(second.streams)
        .collect::<Vec<_>>();
    let selected = std::iter::once(&baseline)
        .chain(&trials)
        .find(|trial| trial.plan_digest() == selection.selected_plan_digest)
        .expect("selected");
    let primary = &selected.identity().roles[0];
    let calibrations = cases
        .iter()
        .map(|case| CalibrationRecord {
            case_id: case.id.clone(),
            iterations: 1,
            attempts: 1,
            elapsed_ns: 100_000_000,
            confirmation_elapsed_ns: 100_000_000,
            overshoot: false,
        })
        .collect::<Vec<_>>();
    encode_completed_tune_decision(&TuneDecisionBuildInput {
        identity,
        budget,
        workload: &workload,
        calibrations: &calibrations,
        space: &space,
        frontier: &frontier,
        baseline: &baseline,
        baseline_streams: streams
            .iter()
            .filter(|stream| stream.plan_digest == baseline.plan_digest())
            .cloned()
            .collect(),
        baseline_compile_reused: false,
        candidates: trials
            .iter()
            .map(|trial| {
                let plan = frontier
                    .compile_selection
                    .iter()
                    .find(|plan| plan.digest == trial.plan_digest())
                    .expect("plan");
                TuneDecisionCandidate {
                    plan,
                    trial,
                    outcome: selection
                        .outcomes
                        .get(&plan.digest)
                        .copied()
                        .unwrap_or_else(|| {
                            if size_rejected.contains(&plan.digest) {
                                CandidateOutcome::SizeRejected
                            } else if ranks.iter().any(|rank| rank.plan_digest == plan.digest) {
                                CandidateOutcome::SearchNonwinner
                            } else {
                                CandidateOutcome::CompiledUnmeasured
                            }
                        }),
                    streams: streams
                        .iter()
                        .filter(|stream| stream.plan_digest == plan.digest)
                        .cloned()
                        .collect(),
                    timeout: None,
                    compile_reused: false,
                }
            })
            .collect(),
        round_one: &one,
        round_two: &two,
        selection: &selection,
        measurement_reused: false,
        measurement_cache_salt_digest: [9; 32],
        outputs: vec![TuneDecisionOutput {
            role: TuneArtifactRole::Primary,
            logical_basename: "program".into(),
            content_digest: primary.digest,
            content_bytes: primary.size,
        }],
    })
    .expect("encoded complete synthetic decision")
}

fn workload() -> CapturedWorkload {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/tune-policy-tests")
        .join(format!("fixture-{}", temp::unique_id()));
    fs::create_dir_all(&root).expect("fixture directory");
    let runner = root.join("runner");
    let magic: &[u8] = if cfg!(target_os = "macos") {
        b"\xcf\xfa\xed\xfe"
    } else if cfg!(windows) {
        b"MZ"
    } else {
        b"\x7fELF"
    };
    fs::write(&runner, magic).expect("non-executed runner identity");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).expect("mode");
    }
    let bytes = b"schema=1\n[runner]\npath=\"runner\"\n[[case]]\nid=\"search\"\nrole=\"search\"\nseed=1\nweight=1\nexpected_digest=\"1111111111111111111111111111111111111111111111111111111111111111\"\n[[case]]\nid=\"validation-a\"\nrole=\"validation\"\nseed=2\nweight=1\nexpected_digest=\"2222222222222222222222222222222222222222222222222222222222222222\"\n[[case]]\nid=\"validation-b\"\nrole=\"validation\"\nseed=3\nweight=3\nexpected_digest=\"3333333333333333333333333333333333333333333333333333333333333333\"\n";
    let manifest =
        TuneManifest::parse(bytes, &root.join("workload.cktune.toml")).expect("manifest");
    let captured = capture_workload(&manifest).expect("captured fixture");
    fs::remove_dir_all(&root).expect("remove owned non-executed fixture");
    captured
}

pub fn fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut rest = bytes;
    let mut fields = Vec::new();
    while !rest.is_empty() {
        assert_eq!(
            u16::from_be_bytes(rest[..2].try_into().expect("tag")) as usize,
            fields.len() + 1
        );
        let len = u32::from_be_bytes(rest[2..6].try_into().expect("length")) as usize;
        fields.push(rest[6..6 + len].to_vec());
        rest = &rest[6 + len..];
    }
    fields
}

pub fn pack(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut result = Vec::new();
    for (index, value) in fields.iter().enumerate() {
        support::field(&mut result, index as u16 + 1, value);
    }
    result
}

pub fn record_fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    fields(&bytes[4..])
}
pub fn record(fields: &[Vec<u8>]) -> Vec<u8> {
    support::record(&pack(fields))
}

pub fn records(bytes: &[u8]) -> Vec<Vec<Vec<u8>>> {
    let count = u32::from_be_bytes(bytes[..4].try_into().expect("list count")) as usize;
    let mut rest = &bytes[4..];
    let mut result = Vec::new();
    for _ in 0..count {
        let len = u32::from_be_bytes(rest[..4].try_into().expect("record length")) as usize;
        result.push(fields(&rest[4..4 + len]));
        rest = &rest[4 + len..];
    }
    assert!(rest.is_empty());
    result
}

pub fn record_list(records: &[Vec<Vec<u8>>]) -> Vec<u8> {
    support::list(
        &records
            .iter()
            .map(|fields| record(fields))
            .collect::<Vec<_>>(),
    )
}

pub fn hash(domain: &[u8], bytes: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(bytes);
    hash.finalize().to_vec()
}

pub fn top() -> Vec<Vec<u8>> {
    let encoded = encoded();
    fields(&encoded[12..encoded.len() - 32])
}

// Repair only enclosing receipts; leave the semantic mutation under test intact.
pub fn reframe(mut top: Vec<Vec<u8>>) -> Vec<u8> {
    let mut selection = fields(&top[6]);
    if selection[4][0] == 1 {
        let mut certificate = record_fields(&selection[4][1..]);
        certificate[2] = fields(&top[1])[31].clone();
        certificate[3] = hash(b"CK-TUNE-VALIDATION-ROUND\0", &selection[0]);
        certificate[4] = hash(b"CK-TUNE-VALIDATION-ROUND\0", &selection[1]);
        selection[4] = support::optional(Some(&record(&certificate)));
        top[6] = pack(&selection);
    }
    let mut replay = fields(&top[7]);
    let mut origin = record_fields(&replay[7]);
    origin[2] = hash(
        b"CK-TUNE-MEASUREMENT-ENTRY\0",
        &record(&[
            origin[1].clone(),
            support::record(&top[5]),
            support::record(&top[6]),
        ]),
    );
    replay[7] = record(&origin);
    top[7] = pack(&replay);
    support::outer_decision(&top)
}
