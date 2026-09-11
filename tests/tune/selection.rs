use calckernel::{
    CalibrationRecord, CandidateOutcome, CandidateRank, MeasurementPhase, MeasurementRow,
    MeasurementStream, RoundPlan, SelectionEntrant, SelectionError, SelectionReason, TuneCase,
    TuneCaseRole, derive_round_summary, derive_search_entrants, derive_selection,
    stream_statistics,
};

fn case(id: &str, role: TuneCaseRole, weight: u32) -> TuneCase {
    TuneCase {
        id: id.into(),
        role,
        seed: 1,
        weight,
        expected_digest: [7; 32],
    }
}

fn stream(
    phase: MeasurementPhase,
    round: u8,
    id: &str,
    plan: [u8; 32],
    values: [u64; 20],
) -> MeasurementStream {
    MeasurementStream {
        phase,
        round,
        case_id: id.into(),
        plan_digest: plan,
        iterations: 1,
        rows: values
            .into_iter()
            .enumerate()
            .map(|(ordinal, value)| MeasurementRow {
                ordinal: ordinal as u32,
                permutation_key: [ordinal as u8; 32],
                calls_ns: vec![value + 2, value, value + 1],
                stored_minimum_ns: value,
            })
            .collect(),
        correctness_digest: [7; 32],
    }
}

#[test]
fn selection_stream_statistics_use_upper_median_and_inclusive_stability() {
    let mut values = [100; 20];
    values[0] = 80;
    values[1] = 120;
    values[2] = 79;
    values[3] = 121;
    let stats = stream_statistics(&stream(
        MeasurementPhase::SearchMeasured,
        0,
        "s",
        [0; 32],
        values,
    ))
    .expect("stable");
    assert_eq!(stats.upper_median_ns, 100);
    assert_eq!(stats.in_range_samples, 18);

    values[4] = 121;
    values[5] = 79;
    values[6] = 121;
    assert!(
        stream_statistics(&stream(
            MeasurementPhase::SearchMeasured,
            0,
            "s",
            [0; 32],
            values
        ))
        .is_err()
    );
}

#[test]
fn selection_error_context_preserves_all_raw_calls_and_matching_calibration() {
    let mut minima = [100_000_000; 20];
    minima[..5].fill(70_000_000);
    let mut rejected = stream(
        MeasurementPhase::ValidationTwoMeasured,
        2,
        "case",
        [9; 32],
        minima,
    );
    rejected.iterations = 32;
    let before = rejected.clone();
    let calibration = CalibrationRecord {
        case_id: "case".into(),
        iterations: 32,
        attempts: 6,
        elapsed_ns: 631_114_667,
        confirmation_elapsed_ns: 314_409_459,
        overshoot: true,
    };
    let error = stream_statistics(&rejected).expect_err("fifteen in-range rows must fail");
    assert_eq!(error, SelectionError::Unstable);
    assert_eq!(error.to_string(), "unstable measurement stream");
    let unrelated_calibration = CalibrationRecord {
        case_id: "unrelated".into(),
        iterations: 999,
        ..calibration.clone()
    };
    let description = error.describe_with_measurements(
        std::slice::from_ref(&rejected),
        &[unrelated_calibration, calibration],
    );
    let calls = rejected
        .rows
        .iter()
        .map(|row| row.calls_ns.clone())
        .collect::<Vec<_>>();
    for expected in [
        "phase=7 round=2 case=\"case\" caseBytes=4 caseTruncated=false".to_string(),
        format!("plan={} iterations=32", "09".repeat(32)),
        "inRange=15/20 required=16 upperMedianNs=100000000".to_string(),
        format!("minimaNs={minima:?} callsNs={calls:?}"),
        "calibrationIterations=32 calibrationAttempts=6 calibrationElapsedNs=631114667 calibrationConfirmationNs=314409459 calibrationOvershoot=true".to_string(),
    ] {
        assert!(description.contains(&expected), "missing {expected:?} in {description}");
    }
    assert_eq!(
        rejected, before,
        "diagnostics must not change measurement evidence"
    );
}

#[test]
fn selection_error_context_is_bounded_escaped_and_exact_at_u64_limits() {
    let mut values = [u64::MAX - 2; 20];
    values[..5].fill(1);
    let id = format!("{}PRIVATE_SUFFIX", "é\n".repeat(8192));
    let rejected = stream(MeasurementPhase::SearchMeasured, 0, &id, [255; 32], values);
    let error = stream_statistics(&rejected).expect_err("unstable, not overflowing");
    assert_eq!(error, SelectionError::Unstable);
    let description = error.describe_with_measurements(&[rejected], &[]);
    assert!(description.contains(&format!("caseBytes={} caseTruncated=true", id.len())));
    assert!(description.contains("\\n"));
    assert!(!description.contains('\n'));
    assert!(!description.contains("PRIVATE_SUFFIX"));
    assert!(description.contains(&format!("upperMedianNs={}", u64::MAX - 2)));
    assert!(description.contains(&u64::MAX.to_string()));
    assert!(description.contains("calibration=unavailable"));
    assert!(
        description.len() < 4096,
        "diagnostic unexpectedly unbounded"
    );
}

#[test]
fn selection_error_context_does_not_invent_rows_for_other_errors_or_invalid_streams() {
    let stable = stream(
        MeasurementPhase::SearchMeasured,
        0,
        "stable",
        [1; 32],
        [100; 20],
    );
    let mut invalid = stable.clone();
    invalid.rows[0].calls_ns.pop();
    for error in [
        SelectionError::InvalidEvidence("row"),
        SelectionError::Overflow,
    ] {
        assert_eq!(
            error.describe_with_measurements(std::slice::from_ref(&stable), &[]),
            error.to_string()
        );
    }
    for streams in [vec![], vec![stable], vec![invalid]] {
        assert_eq!(
            SelectionError::Unstable.describe_with_measurements(&streams, &[]),
            "unstable measurement stream"
        );
    }
}

#[test]
fn selection_search_entrants_use_checked_q32_and_total_rank() {
    let cases = vec![
        case("a", TuneCaseRole::Search, 1),
        case("b", TuneCaseRole::Search, 3),
    ];
    let baseline = [0; 32];
    let a = [1; 32];
    let b = [2; 32];
    let mut streams = vec![
        stream(
            MeasurementPhase::SearchMeasured,
            0,
            "a",
            baseline,
            [100; 20],
        ),
        stream(
            MeasurementPhase::SearchMeasured,
            0,
            "b",
            baseline,
            [200; 20],
        ),
        stream(MeasurementPhase::SearchMeasured, 0, "a", a, [90; 20]),
        stream(MeasurementPhase::SearchMeasured, 0, "b", a, [180; 20]),
        stream(MeasurementPhase::SearchMeasured, 0, "a", b, [90; 20]),
        stream(MeasurementPhase::SearchMeasured, 0, "b", b, [180; 20]),
    ];
    streams.sort_by_key(|item| {
        (
            item.phase as u8,
            item.round,
            item.case_id.clone(),
            item.plan_digest,
        )
    });
    let ranks = vec![
        CandidateRank {
            plan_digest: a,
            primary_artifact_bytes: 4_000,
            choice_count: 2,
        },
        CandidateRank {
            plan_digest: b,
            primary_artifact_bytes: 3_900,
            choice_count: 3,
        },
    ];
    let entrants = derive_search_entrants(baseline, &ranks, &cases, &streams, 2).expect("entrants");
    assert_eq!(
        entrants
            .iter()
            .map(|entry| entry.plan_digest)
            .collect::<Vec<_>>(),
        vec![b, a]
    );
    assert_eq!(entrants[0].score_q32, (9u128 << 32).div_ceil(10) as u64);

    let mut incomplete = streams.clone();
    incomplete.pop();
    assert!(derive_search_entrants(baseline, &ranks, &cases, &incomplete, 2).is_err());

    let overflow_streams = vec![
        stream(MeasurementPhase::SearchMeasured, 0, "a", baseline, [1; 20]),
        stream(MeasurementPhase::SearchMeasured, 0, "b", baseline, [1; 20]),
        stream(
            MeasurementPhase::SearchMeasured,
            0,
            "a",
            a,
            [u64::MAX - 2; 20],
        ),
        stream(
            MeasurementPhase::SearchMeasured,
            0,
            "b",
            a,
            [u64::MAX - 2; 20],
        ),
    ];
    assert!(derive_search_entrants(baseline, &ranks[..1], &cases, &overflow_streams, 1).is_err());
}

#[test]
fn selection_search_rank_is_stable_in_the_same_percent_ceiling_bucket() {
    let cases = vec![case("s", TuneCaseRole::Search, 1)];
    let baseline = [0; 32];
    let larger = [1; 32];
    let smaller = [2; 32];
    let ranks = vec![
        CandidateRank {
            plan_digest: larger,
            primary_artifact_bytes: 4_000,
            choice_count: 2,
        },
        CandidateRank {
            plan_digest: smaller,
            primary_artifact_bytes: 3_900,
            choice_count: 3,
        },
    ];
    let session = |larger_ns, smaller_ns| {
        derive_search_entrants(
            baseline,
            &ranks,
            &cases,
            &[
                stream(
                    MeasurementPhase::SearchMeasured,
                    0,
                    "s",
                    baseline,
                    [10_000; 20],
                ),
                stream(
                    MeasurementPhase::SearchMeasured,
                    0,
                    "s",
                    larger,
                    [larger_ns; 20],
                ),
                stream(
                    MeasurementPhase::SearchMeasured,
                    0,
                    "s",
                    smaller,
                    [smaller_ns; 20],
                ),
            ],
            2,
        )
        .expect("entrants")
        .into_iter()
        .map(|entry| entry.plan_digest)
        .collect::<Vec<_>>()
    };

    assert_eq!(session(8_910, 8_920), vec![smaller, larger]);
    assert_eq!(session(8_920, 8_910), vec![smaller, larger]);
}

#[test]
fn selection_validation_rank_is_stable_in_the_same_percent_ceiling_bucket() {
    let cases = vec![case("v", TuneCaseRole::Validation, 1)];
    let baseline = [0; 32];
    let larger = [1; 32];
    let smaller = [2; 32];
    let ranks = vec![
        CandidateRank {
            plan_digest: larger,
            primary_artifact_bytes: 4_000,
            choice_count: 2,
        },
        CandidateRank {
            plan_digest: smaller,
            primary_artifact_bytes: 3_900,
            choice_count: 3,
        },
    ];
    let round = |number, phase, larger_ns, smaller_ns| {
        derive_round_summary(
            number,
            baseline,
            &ranks,
            &cases,
            &[
                stream(phase, number, "v", baseline, [10_000; 20]),
                stream(phase, number, "v", larger, [larger_ns; 20]),
                stream(phase, number, "v", smaller, [smaller_ns; 20]),
            ],
        )
        .expect("round")
    };
    let one = round(1, MeasurementPhase::ValidationOneMeasured, 8_910, 8_920);
    let two = round(2, MeasurementPhase::ValidationTwoMeasured, 8_920, 8_910);

    assert_eq!(one.ranked_plan_digests, vec![smaller, larger]);
    assert_eq!(two.ranked_plan_digests, vec![smaller, larger]);
    let selected = derive_selection(
        baseline,
        &[
            SelectionEntrant::active(larger),
            SelectionEntrant::active(smaller),
        ],
        &one,
        &two,
    )
    .expect("selection");
    assert_eq!(selected.reason, SelectionReason::Tuned);
    assert_eq!(selected.selected_plan_digest, smaller);
}

#[test]
fn selection_validation_rederives_thresholds_paired_wins_and_four_row_table() {
    let cases = vec![case("v", TuneCaseRole::Validation, 1)];
    let baseline = [0; 32];
    let winner = [1; 32];
    let loser = [2; 32];
    let ranks = vec![
        CandidateRank {
            plan_digest: winner,
            primary_artifact_bytes: 4_000,
            choice_count: 1,
        },
        CandidateRank {
            plan_digest: loser,
            primary_artifact_bytes: 3_900,
            choice_count: 2,
        },
    ];
    let round = |number, phase| {
        derive_round_summary(
            number,
            baseline,
            &ranks,
            &cases,
            &[
                stream(phase, number, "v", baseline, [100; 20]),
                stream(phase, number, "v", winner, [96; 20]),
                stream(phase, number, "v", loser, [100; 20]),
            ],
        )
        .expect("round")
    };
    let one = round(1, MeasurementPhase::ValidationOneMeasured);
    let two = round(2, MeasurementPhase::ValidationTwoMeasured);
    assert_eq!(one.plans[0].paired_wins, 20);
    assert!(one.plans[0].threshold_passed);
    assert!(!one.plans[1].threshold_passed);

    let entrants = vec![
        SelectionEntrant::active(winner),
        SelectionEntrant::active(loser),
    ];
    let tuned = derive_selection(baseline, &entrants, &one, &two).expect("tuned");
    assert_eq!(
        (
            tuned.reason,
            tuned.selected_plan_digest,
            tuned.certificate_plan_digest
        ),
        (SelectionReason::Tuned, winner, Some(winner))
    );
    assert_eq!(tuned.outcomes[&winner], CandidateOutcome::Selected);
    assert_eq!(
        tuned.outcomes[&loser],
        CandidateOutcome::ValidationNonwinner
    );

    let none =
        derive_selection(baseline, &[], &empty_round(1), &empty_round(2)).expect("no candidate");
    assert_eq!(none.reason, SelectionReason::NoCandidate);

    let threshold = derive_selection(
        baseline,
        &[SelectionEntrant::active(loser)],
        &rejected_round(1, loser),
        &rejected_round(2, loser),
    )
    .expect("threshold");
    assert_eq!(threshold.reason, SelectionReason::ValidationThreshold);
    assert_eq!(
        threshold.outcomes[&loser],
        CandidateOutcome::ValidationThreshold
    );

    let disagreement = derive_selection(baseline, &entrants, &one, &round_with_rank(2, loser))
        .expect("disagreement");
    assert_eq!(disagreement.reason, SelectionReason::ValidationDisagreement);

    let timeout = derive_selection(
        baseline,
        &[SelectionEntrant::timed_out(winner)],
        &empty_round(1),
        &empty_round(2),
    )
    .expect("timed-out entrant remains timed-out");
    assert_eq!(timeout.reason, SelectionReason::ValidationThreshold);
    assert_eq!(timeout.outcomes[&winner], CandidateOutcome::TimedOut);
}

fn empty_round(round: u8) -> calckernel::RoundSummary {
    calckernel::RoundSummary {
        round,
        plans: Vec::new(),
        ranked_plan_digests: Vec::new(),
    }
}

fn round_with_rank(round: u8, digest: [u8; 32]) -> calckernel::RoundSummary {
    let mut summary = rejected_round(round, [1; 32]);
    summary.plans.push(RoundPlan {
        plan_digest: [2; 32],
        case_medians: Vec::new(),
        aggregate_ratio_q32: 1u64 << 32,
        stable: true,
        threshold_passed: false,
        paired_wins: 20,
    });
    summary.plans.sort_by_key(|plan| plan.plan_digest);
    for plan in &mut summary.plans {
        plan.threshold_passed = plan.plan_digest == digest;
    }
    summary.ranked_plan_digests = vec![digest];
    summary
}

fn rejected_round(round: u8, digest: [u8; 32]) -> calckernel::RoundSummary {
    calckernel::RoundSummary {
        round,
        plans: vec![RoundPlan {
            plan_digest: digest,
            case_medians: Vec::new(),
            aggregate_ratio_q32: 1u64 << 32,
            stable: true,
            threshold_passed: false,
            paired_wins: 0,
        }],
        ranked_plan_digests: Vec::new(),
    }
}
