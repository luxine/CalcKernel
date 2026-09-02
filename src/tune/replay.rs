use crate::{
    KirVerifiedProgramState, TuneAlternativeClass, TuneBudget, TuningPlan, TuningSpace,
    apply_tuning_plan, check_tuning_plan, run_deterministic_search,
};

use super::NonPublishableTuneTrial;

/// Complete size-gate result before measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneFinalistSelection {
    pub eligible: Vec<NonPublishableTuneTrial>,
    pub size_rejected: Vec<NonPublishableTuneTrial>,
}

/// Recomputes the compile selection and verifies every isolated trial.
pub fn verify_tune_trials_with_source(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    budget: TuneBudget,
    trials: &[NonPublishableTuneTrial],
) -> Result<(), String> {
    let search =
        run_deterministic_search(state, space, budget).map_err(|error| error.to_string())?;
    let mut expected = search
        .compile_selection
        .iter()
        .map(|plan| plan.digest)
        .collect::<Vec<_>>();
    expected.sort();
    let actual = trials
        .iter()
        .map(NonPublishableTuneTrial::plan_digest)
        .collect::<Vec<_>>();
    if actual != expected {
        return Err("tuning trial set does not equal the compile selection".to_string());
    }
    for trial in trials {
        check_tuning_plan(state, space, trial.plan()).map_err(|error| error.to_string())?;
        let replayed =
            apply_tuning_plan(state, space, trial.plan()).map_err(|error| error.to_string())?;
        if replayed.kir_digest() != trial.post_state_digest() {
            return Err("tuning trial post-state mismatch".to_string());
        }
        trial.verify_internal_identity()?;
    }
    Ok(())
}

/// Applies the exact 110% primary-size gate and frozen diversity bound.
pub fn select_size_valid_finalists(
    baseline: &NonPublishableTuneTrial,
    trials: &[NonPublishableTuneTrial],
    budget: TuneBudget,
) -> Result<TuneFinalistSelection, String> {
    let baseline_size = u128::from(baseline.primary_size());
    let threshold = baseline_size
        .checked_mul(110)
        .ok_or("baseline size multiplication overflow")?;
    let mut eligible = Vec::new();
    let mut size_rejected = Vec::new();
    for trial in trials {
        let scaled = u128::from(trial.primary_size())
            .checked_mul(100)
            .ok_or("tuning size multiplication overflow")?;
        if scaled <= threshold {
            eligible.push(trial.clone());
        } else {
            size_rejected.push(trial.clone());
        }
    }
    eligible.sort_by(trial_rank);
    eligible = diversity_truncate(eligible, budget.contract().measured_finalist_limit)?;
    size_rejected.sort_by_key(NonPublishableTuneTrial::plan_digest);
    Ok(TuneFinalistSelection {
        eligible,
        size_rejected,
    })
}

fn diversity_truncate(
    plans: Vec<NonPublishableTuneTrial>,
    limit: u32,
) -> Result<Vec<NonPublishableTuneTrial>, String> {
    let limit = usize::try_from(limit).map_err(|_| "finalist limit overflow")?;
    let priority = [
        TuneAlternativeClass::Inlining,
        TuneAlternativeClass::Specialization,
        TuneAlternativeClass::Unrolling,
        TuneAlternativeClass::LoopSimd,
        TuneAlternativeClass::Slp,
        TuneAlternativeClass::ShortSliceVersioning,
        TuneAlternativeClass::Layout,
    ];
    let mut selected = Vec::new();
    for class in priority {
        if selected.len() == limit {
            break;
        }
        if let Some(trial) = plans.iter().find(|trial| {
            trial
                .plan()
                .choices
                .last()
                .is_some_and(|choice| choice.class == class)
                && !selected
                    .iter()
                    .any(|item: &NonPublishableTuneTrial| item.plan_digest() == trial.plan_digest())
        }) {
            selected.push(trial.clone());
        }
    }
    for trial in plans {
        if selected.len() == limit {
            break;
        }
        if !selected
            .iter()
            .any(|item| item.plan_digest() == trial.plan_digest())
        {
            selected.push(trial);
        }
    }
    selected.sort_by(trial_rank);
    Ok(selected)
}

fn trial_rank(
    left: &NonPublishableTuneTrial,
    right: &NonPublishableTuneTrial,
) -> std::cmp::Ordering {
    plan_actual_rank(left.plan(), left.primary_size())
        .cmp(&plan_actual_rank(right.plan(), right.primary_size()))
}

fn plan_actual_rank(plan: &TuningPlan, actual_size: u64) -> impl Ord {
    let classes = plan
        .choices
        .iter()
        .map(|choice| choice.class as u8)
        .collect::<Vec<_>>();
    let pairs = plan
        .choices
        .iter()
        .map(|choice| (choice.unit_id, choice.variant_id))
        .collect::<Vec<_>>();
    (
        plan.predicted_dynamic,
        plan.predicted_static,
        actual_size,
        plan.choices.len(),
        classes,
        pairs,
        plan.digest,
    )
}
