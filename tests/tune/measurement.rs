use calckernel::{
    BaselineSessionSeed, MeasurementChannel, MeasurementEventOutcome, MeasurementPhase,
    MeasurementScheduler, SessionDigestMaterial, TuneCase, TuneCaseRole, derive_session_digest,
    verify_search_measurement_run,
};
use sha2::{Digest, Sha256};

fn cases() -> Vec<TuneCase> {
    vec![
        TuneCase {
            id: "a".into(),
            role: TuneCaseRole::Search,
            seed: 1,
            weight: 1,
            expected_digest: [1; 32],
        },
        TuneCase {
            id: "b".into(),
            role: TuneCaseRole::Search,
            seed: 2,
            weight: 3,
            expected_digest: [2; 32],
        },
        TuneCase {
            id: "v".into(),
            role: TuneCaseRole::Validation,
            seed: 3,
            weight: 1,
            expected_digest: [3; 32],
        },
    ]
}

fn channels() -> Vec<MeasurementChannel> {
    vec![
        MeasurementChannel::baseline([0; 32], 4_096),
        MeasurementChannel::candidate([1; 32], 4_000, 2),
        MeasurementChannel::candidate([2; 32], 3_900, 1),
    ]
}

#[test]
fn measurement_session_digest_uses_exact_canonical_material() {
    let identity = record(&[1, 2]);
    let contract = record(&[3]);
    let workload = record(&[4, 5]);
    let environment = record(&[6]);
    let frontier = record(&[7, 8]);
    let material = SessionDigestMaterial {
        identity_record: &identity,
        contract_record: &contract,
        workload_record: &workload,
        environment_seed_record: &environment,
        frontier_record: &frontier,
        baseline: BaselineSessionSeed {
            plan_digest: [9; 32],
            object_graph_digest: [10; 32],
            link_recipe_digest: [11; 32],
            primary_artifact_bytes: 12,
        },
    };
    let first = derive_session_digest(&material).expect("bounded material");
    let second = derive_session_digest(&material).expect("same material");
    assert_eq!(first, second);
    let unrelated: [u8; 32] = Sha256::digest(b"uncanonical").into();
    assert_ne!(first, unrelated);
    assert_eq!(
        first
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "94aadc0fd528d73de8cb5050e520dcf5874d25bbeb11cbf84368d3ec96f44e6e"
    );
    let malformed = SessionDigestMaterial {
        identity_record: &[1, 2],
        ..material
    };
    assert!(derive_session_digest(&malformed).is_err());
}

#[test]
fn measurement_schedule_is_deterministic_complete_and_rotated() {
    let mut scheduler = MeasurementScheduler::new(
        [7; 32],
        channels(),
        cases(),
        &[("a", 10), ("b", 20), ("v", 30)],
    )
    .expect("scheduler");
    let mut calls = 0u64;
    let run = scheduler
        .run_search(|coordinate, case, _channel, iterations| {
            calls += 1;
            Ok(calckernel::InvocationResult {
                elapsed_ns: 100 + u64::from(coordinate.call),
                completed: iterations,
                digest: case.expected_digest,
            })
        })
        .expect("complete search");

    assert_eq!(calls, 2 * 3 * (3 + 20 * 3));
    assert_eq!(run.streams.len(), 6);
    assert!(run.streams.iter().all(|stream| stream.rows.len() == 20));
    assert!(run.streams.iter().all(|stream| {
        stream
            .rows
            .iter()
            .all(|row| row.calls_ns.len() == 3 && row.stored_minimum_ns == 101)
    }));
    let first_cases = run
        .events
        .iter()
        .filter(|event| {
            event.coordinate.phase == MeasurementPhase::SearchMeasured
                && event.coordinate.row == 0
                && event.coordinate.call == 1
        })
        .map(|event| event.coordinate.case_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(first_cases.len(), 6);
    assert_ne!(&first_cases[..3], &first_cases[3..]);
    let key = run
        .streams
        .iter()
        .find(|stream| stream.case_id == "a" && stream.plan_digest == [0; 32])
        .expect("baseline a stream")
        .rows[0]
        .permutation_key;
    assert_eq!(
        key.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "34e4ec414c144e81dc2b96955ca5fd830f67316f28df9b0b0da2ddfbfa9b64ad"
    );
    verify_search_measurement_run(
        [7; 32],
        channels(),
        cases(),
        &[("a", 10), ("b", 20), ("v", 30)],
        &run,
    )
    .expect("event log and streams replay exactly");

    let mut deleted = run.clone();
    deleted.streams[0].rows.remove(0);
    assert!(
        verify_search_measurement_run(
            [7; 32],
            channels(),
            cases(),
            &[("a", 10), ("b", 20), ("v", 30)],
            &deleted,
        )
        .is_err()
    );

    let mut swapped = run.clone();
    swapped.events.swap(0, 1);
    assert!(
        verify_search_measurement_run(
            [7; 32],
            channels(),
            cases(),
            &[("a", 10), ("b", 20), ("v", 30)],
            &swapped,
        )
        .is_err()
    );

    let mut inserted = run.clone();
    inserted.events.push(inserted.events[0].clone());
    assert!(
        verify_search_measurement_run(
            [7; 32],
            channels(),
            cases(),
            &[("a", 10), ("b", 20), ("v", 30)],
            &inserted,
        )
        .is_err()
    );
}

#[test]
fn measurement_timeout_retains_exact_coordinate_complete_stream_set_and_skips_later_slots() {
    let mut scheduler = MeasurementScheduler::new(
        [8; 32],
        channels(),
        cases(),
        &[("a", 10), ("b", 20), ("v", 30)],
    )
    .expect("scheduler");
    let run = scheduler
        .run_search(|coordinate, case, channel, iterations| {
            if channel.plan_digest == [1; 32]
                && coordinate.phase == MeasurementPhase::SearchMeasured
                && coordinate.row == 4
                && coordinate.call == 2
            {
                return Err(calckernel::RunnerFailure::CandidateTimeout(
                    calckernel::CanonicalCandidateTimeout {
                        case_id: case.id.clone(),
                        iterations,
                        timeout_ms: 100,
                        elapsed_ns: 100_123_456,
                    },
                ));
            }
            Ok(calckernel::InvocationResult {
                elapsed_ns: 100,
                completed: iterations,
                digest: case.expected_digest,
            })
        })
        .expect("candidate timeout is a recorded rejection");

    assert_eq!(run.timeouts.len(), 1);
    let timeout = &run.timeouts[0];
    assert_eq!(
        (
            timeout.phase,
            timeout.row,
            timeout.call,
            timeout.plan_digest
        ),
        (MeasurementPhase::SearchMeasured, 4, 2, [1; 32])
    );
    assert!(
        run.streams
            .iter()
            .all(|stream| stream.plan_digest != [1; 32])
    );
    assert!(
        run.events
            .iter()
            .any(|event| event.coordinate.plan_digest == [1; 32]
                && event.coordinate.row > 4
                && event.outcome == MeasurementEventOutcome::Skipped)
    );
    verify_search_measurement_run(
        [8; 32],
        channels(),
        cases(),
        &[("a", 10), ("b", 20), ("v", 30)],
        &run,
    )
    .expect("timeout stream set must be exactly recomputable");

    let mut forged = run.clone();
    forged.timeouts[0].row += 1;
    assert!(
        verify_search_measurement_run(
            [8; 32],
            channels(),
            cases(),
            &[("a", 10), ("b", 20), ("v", 30)],
            &forged,
        )
        .is_err()
    );
}

#[test]
fn measurement_validation_rounds_have_distinct_order_domains() {
    let mut scheduler = MeasurementScheduler::new(
        [9; 32],
        channels(),
        cases(),
        &[("a", 10), ("b", 20), ("v", 30)],
    )
    .expect("scheduler");
    let mut invoke = |_: &_, case: &TuneCase, _: &_, iterations: u64| {
        Ok(calckernel::InvocationResult {
            elapsed_ns: 100,
            completed: iterations,
            digest: case.expected_digest,
        })
    };
    let one = scheduler
        .run_validation_round(1, &[[1; 32]], &mut invoke)
        .expect("round one");
    let two = scheduler
        .run_validation_round(2, &[[1; 32]], &mut invoke)
        .expect("round two");
    assert_eq!(one.streams.len(), 2);
    assert_eq!(two.streams.len(), 2);
    assert_ne!(
        one.streams[0].rows[0].permutation_key,
        two.streams[0].rows[0].permutation_key
    );
}

#[test]
fn measurement_failures_abort_without_partial_evidence() {
    let mut scheduler = MeasurementScheduler::new(
        [10; 32],
        channels(),
        cases(),
        &[("a", 10), ("b", 20), ("v", 30)],
    )
    .expect("scheduler");
    assert!(
        scheduler
            .run_search(|_, _, _, _| Err(calckernel::RunnerFailure::WallBudgetAdmission))
            .is_err()
    );
}

fn record(payload: &[u8]) -> Vec<u8> {
    let mut output = u32::try_from(payload.len())
        .expect("test record length")
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(payload);
    output
}
