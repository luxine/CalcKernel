use calckernel::{
    TuneArtifactKind, TuneBudget, TuneTrialBuildRequest, compile_tune_trial,
    enumerate_tuning_space, run_deterministic_search, select_size_valid_finalists,
    verify_tune_trials_with_source,
};

use super::trial::state;

#[test]
fn replay_requires_the_complete_compile_selection_in_plan_digest_order() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let search = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");
    let mut trials = search
        .compile_selection
        .iter()
        .map(|plan| trial(&state, &space, plan, 100))
        .collect::<Vec<_>>();
    trials.sort_by_key(|trial| trial.plan_digest());
    verify_tune_trials_with_source(&state, &space, TuneBudget::Quick, &trials)
        .expect("complete set");

    trials.pop();
    assert!(verify_tune_trials_with_source(&state, &space, TuneBudget::Quick, &trials).is_err());
}

#[test]
fn replay_size_gate_uses_checked_ten_percent_and_actual_primary_bytes() {
    let state = state();
    let space = enumerate_tuning_space(&state).expect("space");
    let search = run_deterministic_search(&state, &space, TuneBudget::Quick).expect("search");
    let baseline = trial(&state, &space, &calckernel::TuningPlan::baseline(), 100);
    let mut trials = search
        .compile_selection
        .iter()
        .take(2)
        .enumerate()
        .map(|(index, plan)| trial(&state, &space, plan, if index == 0 { 110 } else { 111 }))
        .collect::<Vec<_>>();
    trials.sort_by_key(|trial| trial.plan_digest());
    let selected = select_size_valid_finalists(&baseline, &trials, TuneBudget::Quick)
        .expect("bounded selection");

    assert_eq!(selected.eligible.len(), 1);
    assert_eq!(selected.size_rejected.len(), 1);
    assert_eq!(selected.eligible[0].primary_size(), 110);
}

fn trial(
    state: &calckernel::KirVerifiedProgramState,
    space: &calckernel::TuningSpace,
    plan: &calckernel::TuningPlan,
    size: usize,
) -> calckernel::NonPublishableTuneTrial {
    compile_tune_trial(
        state,
        space,
        plan,
        TuneTrialBuildRequest::new(
            TuneArtifactKind::Executable,
            vec![0x5a; size],
            None,
            None,
            vec![("program.o".to_string(), vec![0x31; 8])],
            vec!["embedded-lld".to_string()],
        ),
    )
    .expect("trial")
}
