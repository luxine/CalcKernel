use calckernel::{
    CandidateOutcome, CandidateRank, MeasurementPhase, MeasurementRow, MeasurementStream,
    RoundPlan, SelectionEntrant, SelectionReason, TuneCase, TuneCaseRole, derive_round_summary,
    derive_search_entrants, derive_selection, stream_statistics,
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
