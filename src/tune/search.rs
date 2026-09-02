use std::collections::{BTreeMap, BTreeSet};

use crate::{
    KirVerifiedProgramState, TuneAlternativeClass, TuneBudget, TunePlanChoice, TuneUnit,
    TuningPlan, TuningPlanError, TuningSpace, apply_tuning_plan,
};

/// Closed result of one attempted plan expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionDisposition {
    Legal,
    Duplicate,
}

/// One zero-based deterministic expansion trace record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionRecord {
    pub ordinal: u32,
    pub parent_plan_digest: [u8; 32],
    pub unit_id: [u8; 32],
    pub variant_id: [u8; 32],
    pub disposition: ExpansionDisposition,
    pub result_plan_digest: [u8; 32],
    pub whole_plan_dynamic: u64,
    pub whole_plan_static: u64,
    pub whole_plan_kir_bytes: u64,
}

/// Complete deterministic search output before compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFrontier {
    pub expansions: Vec<ExpansionRecord>,
    pub frontier: Vec<TuningPlan>,
    pub compile_selection: Vec<TuningPlan>,
}

/// Runs the frozen bounded beam and diversity search.
///
/// # Errors
///
/// Returns an independent plan/replay failure or checked arithmetic failure.
pub fn run_deterministic_search(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    budget: TuneBudget,
) -> Result<SearchFrontier, TuningPlanError> {
    let contract = budget.contract();
    let baseline = metrics_for(state, TuningPlan::baseline())?;
    let mut beam = vec![baseline.clone()];
    let mut expansions = Vec::new();
    'units: for unit in &space.units {
        let mut pool = beam.clone();
        let mut ranked = beam.clone();
        ranked.sort_by(plan_rank);
        for parent in ranked {
            for variant in &unit.variants {
                if expansions.len()
                    == usize::try_from(contract.expansion_limit)
                        .map_err(|_| TuningPlanError::ResourceLimit)?
                {
                    break 'units;
                }
                let ordinal =
                    u32::try_from(expansions.len()).map_err(|_| TuningPlanError::ResourceLimit)?;
                let derived = extend_plan(state, space, &parent, unit, variant.variant_id)?;
                let duplicate = pool.iter().any(|plan| plan.digest == derived.digest);
                expansions.push(ExpansionRecord {
                    ordinal,
                    parent_plan_digest: parent.digest,
                    unit_id: unit.unit_id,
                    variant_id: variant.variant_id,
                    disposition: if duplicate {
                        ExpansionDisposition::Duplicate
                    } else {
                        ExpansionDisposition::Legal
                    },
                    result_plan_digest: derived.digest,
                    whole_plan_dynamic: derived.predicted_dynamic,
                    whole_plan_static: derived.predicted_static,
                    whole_plan_kir_bytes: derived.kir_bytes,
                });
                if !duplicate {
                    pool.push(derived);
                }
            }
        }
        let unique: Vec<_> = deduplicate(pool)
            .into_iter()
            .filter(|plan| !plan.choices.is_empty())
            .collect();
        beam = vec![baseline.clone()];
        beam.extend(diversity_truncate(unique, contract.beam_width)?);
    }
    let frontier: Vec<_> = beam
        .into_iter()
        .filter(|plan| !plan.choices.is_empty())
        .collect();
    let compile_selection = diversity_truncate(frontier.clone(), contract.compile_attempt_limit)?;
    Ok(SearchFrontier {
        expansions,
        frontier,
        compile_selection,
    })
}

fn extend_plan(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    parent: &TuningPlan,
    unit: &TuneUnit,
    variant_id: [u8; 32],
) -> Result<TuningPlan, TuningPlanError> {
    let mut choices = parent.choices.clone();
    choices.push(TunePlanChoice {
        unit_id: unit.unit_id,
        variant_id,
        class: unit.class,
    });
    let digest = super::plan::plan_digest(&choices);
    let plan = TuningPlan {
        choices,
        predicted_dynamic: 0,
        predicted_static: 0,
        kir_bytes: 0,
        digest,
    };
    apply_tuning_plan(state, space, &plan)?;
    metrics_for(state, plan)
}

fn metrics_for(
    state: &KirVerifiedProgramState,
    mut plan: TuningPlan,
) -> Result<TuningPlan, TuningPlanError> {
    let instruction_count = state
        .module()
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .map(|block| block.instructions.len())
        .sum::<usize>();
    let base = u64::try_from(instruction_count).map_err(|_| TuningPlanError::ResourceLimit)?;
    let choice_weight = plan.choices.iter().try_fold(0u64, |total, choice| {
        total
            .checked_add(u64::from(choice.class.replay_phase()))
            .ok_or(TuningPlanError::ResourceLimit)
    })?;
    plan.predicted_dynamic = base.saturating_mul(100).saturating_sub(choice_weight);
    plan.predicted_static = base.saturating_add(choice_weight);
    plan.kir_bytes = u64::try_from(state.kir_digest().len())
        .map_err(|_| TuningPlanError::ResourceLimit)?
        .saturating_add(choice_weight);
    Ok(plan)
}

fn deduplicate(plans: Vec<TuningPlan>) -> Vec<TuningPlan> {
    let mut ranked = plans;
    ranked.sort_by(plan_rank);
    let mut unique = BTreeMap::new();
    for plan in ranked {
        unique.entry(plan.digest).or_insert(plan);
    }
    unique.into_values().collect()
}

fn diversity_truncate(
    mut plans: Vec<TuningPlan>,
    limit: u32,
) -> Result<Vec<TuningPlan>, TuningPlanError> {
    plans.sort_by(plan_rank);
    let limit = usize::try_from(limit).map_err(|_| TuningPlanError::ResourceLimit)?;
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
    let mut digests = BTreeSet::new();
    for class in priority {
        if selected.len() == limit {
            break;
        }
        if let Some(plan) = plans.iter().find(|plan| {
            plan.choices
                .last()
                .is_some_and(|choice| choice.class == class)
                && !digests.contains(&plan.digest)
        }) {
            digests.insert(plan.digest);
            selected.push(plan.clone());
        }
    }
    for plan in plans {
        if selected.len() == limit {
            break;
        }
        if digests.insert(plan.digest) {
            selected.push(plan);
        }
    }
    selected.sort_by(plan_rank);
    Ok(selected)
}

fn plan_rank(left: &TuningPlan, right: &TuningPlan) -> std::cmp::Ordering {
    let left_classes: Vec<_> = left
        .choices
        .iter()
        .map(|choice| choice.class as u8)
        .collect();
    let right_classes: Vec<_> = right
        .choices
        .iter()
        .map(|choice| choice.class as u8)
        .collect();
    let left_pairs: Vec<_> = left
        .choices
        .iter()
        .map(|choice| (choice.unit_id, choice.variant_id))
        .collect();
    let right_pairs: Vec<_> = right
        .choices
        .iter()
        .map(|choice| (choice.unit_id, choice.variant_id))
        .collect();
    (
        left.predicted_dynamic,
        left.predicted_static,
        left.kir_bytes,
        left.choices.len(),
        left_classes,
        left_pairs,
        left.digest,
    )
        .cmp(&(
            right.predicted_dynamic,
            right.predicted_static,
            right.kir_bytes,
            right.choices.len(),
            right_classes,
            right_pairs,
            right.digest,
        ))
}
