use calckernel::{
    CandidateRank, MeasurementPhase, MeasurementRow, MeasurementStream, SelectionEntrant,
    SelectionReason, TuneCase, TuneCaseRole, derive_round_summary, derive_search_entrants,
    derive_selection,
};

const BASELINE: [u8; 32] = [0; 32];
const A: [u8; 32] = [1; 32];
const B: [u8; 32] = [2; 32];
const Q32: u64 = 1 << 32;
// Retained CI medians: two cold sessions, each with two validation rounds.
const OBSERVATIONS: [(u64, u64, u64); 4] = [
    (52_881_338, 11_565_700, 11_475_805),
    (53_031_075, 11_731_577, 11_521_129),
    (53_069_070, 11_619_618, 11_561_496),
    (52_744_403, 11_575_849, 11_445_763),
];

fn case(role: TuneCaseRole) -> TuneCase {
    TuneCase {
        id: "case".into(),
        role,
        seed: 1,
        weight: 1,
        expected_digest: [7; 32],
    }
}

fn stream(phase: MeasurementPhase, round: u8, plan: [u8; 32], ns: u64) -> MeasurementStream {
    MeasurementStream {
        phase,
        round,
        case_id: "case".into(),
        plan_digest: plan,
        iterations: 1,
        rows: (0..20)
            .map(|ordinal| MeasurementRow {
                ordinal,
                permutation_key: [ordinal as u8; 32],
                calls_ns: vec![ns + 2, ns, ns + 1],
                stored_minimum_ns: ns,
            })
            .collect(),
        correctness_digest: [7; 32],
    }
}

fn rank(plan: [u8; 32], bytes: u64, choices: u32) -> CandidateRank {
    CandidateRank {
        plan_digest: plan,
        primary_artifact_bytes: bytes,
        choice_count: choices,
    }
}

fn search(baseline_ns: u64, inputs: &[(CandidateRank, u64)], limit: u32) -> Vec<[u8; 32]> {
    let mut streams = vec![stream(
        MeasurementPhase::SearchMeasured,
        0,
        BASELINE,
        baseline_ns,
    )];
    streams.extend(inputs.iter().map(|(candidate, ns)| {
        stream(
            MeasurementPhase::SearchMeasured,
            0,
            candidate.plan_digest,
            *ns,
        )
    }));
    let ranks = inputs
        .iter()
        .map(|(candidate, _)| *candidate)
        .collect::<Vec<_>>();
    derive_search_entrants(
        BASELINE,
        &ranks,
        &[case(TuneCaseRole::Search)],
        &streams,
        limit,
    )
    .expect("complete stable search")
    .into_iter()
    .map(|entry| entry.plan_digest)
    .collect()
}

#[test]
fn selection_groups_search_keeps_near_scores_together_across_absolute_grid_edges() {
    for (baseline, a, b) in OBSERVATIONS {
        assert_eq!(
            search(baseline, &[(rank(A, 1640, 2), a), (rank(B, 1720, 1), b)], 2),
            [A, B],
            "observed medians {baseline}/{a}/{b}"
        );
    }
}

#[test]
fn selection_groups_validation_agrees_across_both_retained_cold_sessions() {
    let ranks = [rank(A, 1640, 2), rank(B, 1720, 1)];
    for observed in OBSERVATIONS.chunks_exact(2) {
        let rounds = observed
            .iter()
            .enumerate()
            .map(|(index, &(baseline, a, b))| {
                let (round, phase) = if index == 0 {
                    (1, MeasurementPhase::ValidationOneMeasured)
                } else {
                    (2, MeasurementPhase::ValidationTwoMeasured)
                };
                derive_round_summary(
                    round,
                    BASELINE,
                    &ranks,
                    &[case(TuneCaseRole::Validation)],
                    &[
                        stream(phase, round, BASELINE, baseline),
                        stream(phase, round, A, a),
                        stream(phase, round, B, b),
                    ],
                )
                .expect("complete stable validation")
            })
            .collect::<Vec<_>>();
        let selected = derive_selection(
            BASELINE,
            &[SelectionEntrant::active(A), SelectionEntrant::active(B)],
            &rounds[0],
            &rounds[1],
        )
        .expect("selection");
        assert_eq!(
            (selected.reason, selected.selected_plan_digest),
            (SelectionReason::Tuned, A)
        );
    }
}

#[test]
fn selection_groups_accepts_largest_q32_span_inside_one_percentage_point() {
    assert_eq!(
        search(
            Q32,
            &[
                (rank(A, 200, 1), 900_000_000),
                (rank(B, 100, 1), 900_000_000 + Q32 / 100)
            ],
            2
        ),
        [B, A]
    );
}

#[test]
fn selection_groups_rejects_one_q32_unit_beyond_the_group_span() {
    assert_eq!(
        search(
            Q32,
            &[
                (rank(A, 200, 1), 900_000_000),
                (rank(B, 100, 1), 900_000_001 + Q32 / 100)
            ],
            2
        ),
        [A, B]
    );
}

#[test]
fn selection_groups_never_chains_near_neighbors_beyond_the_fastest_anchor() {
    let c = [3; 32];
    assert_eq!(
        search(
            10_000,
            &[
                (rank(A, 300, 1), 2175),
                (rank(B, 200, 1), 2250),
                (rank(c, 100, 1), 2325)
            ],
            3
        ),
        [B, A, c]
    );
}

#[test]
fn selection_groups_total_tie_keys_are_independent_of_input_order() {
    let c = [3; 32];
    let inputs = [
        (rank(A, 200, 2), 2175),
        (rank(B, 200, 1), 2200),
        (rank(c, 200, 1), 2250),
    ];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        assert_eq!(
            search(10_000, &order.map(|index| inputs[index]), 3),
            [B, c, A]
        );
    }
}

#[test]
fn selection_groups_truncates_only_after_resolving_the_whole_group() {
    assert_eq!(
        search(
            10_000,
            &[(rank(A, 200, 1), 2199), (rank(B, 100, 1), 2201)],
            1
        ),
        [B]
    );
}

#[test]
fn selection_groups_handles_empty_candidates_and_zero_limit() {
    assert!(search(10_000, &[], 3).is_empty());
    assert!(search(10_000, &[(rank(A, 200, 1), 2199)], 0).is_empty());
}

#[test]
fn selection_groups_uses_checked_wide_arithmetic_for_large_q32_scores() {
    assert_eq!(
        search(
            Q32,
            &[
                (rank(A, 200, 1), u64::MAX - 100),
                (rank(B, 100, 1), u64::MAX - 2)
            ],
            2
        ),
        [B, A]
    );
}
