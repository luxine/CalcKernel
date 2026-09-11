use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
};

use crate::{
    CheckedTuningSpace, KirVerifiedProgramState, TuneAlternativeClass, TuneBudget, TuneUnit,
    TuningPlan, TuningPlanError, TuningSpace,
};

/// Closed result of one attempted plan expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionDisposition {
    Legal,
    Illegal,
    Duplicate,
    GrowthRejected,
}

/// One zero-based deterministic expansion trace record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionRecord {
    pub ordinal: u32,
    pub parent_plan_digest: [u8; 32],
    pub unit_id: [u8; 32],
    pub variant_id: [u8; 32],
    pub disposition: ExpansionDisposition,
    pub result_plan_digest: Option<[u8; 32]>,
    pub diagnostic_code: u16,
    pub whole_plan_dynamic: Option<u64>,
    pub whole_plan_static: Option<u64>,
    pub whole_plan_kir_bytes: Option<u64>,
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
    let checked = CheckedTuningSpace::check(state, space)?;
    run_checked_tuning_search(&checked, budget)
}

/// Runs the unchanged bounded search while retaining source-backed space
/// authority across every expansion. Raw spaces must use the checked entry.
pub fn run_checked_tuning_search(
    checked: &CheckedTuningSpace<'_>,
    budget: TuneBudget,
) -> Result<SearchFrontier, TuningPlanError> {
    let mut replay = checked.search_replay();
    search_with_replay(checked.space(), budget, |selections| {
        replay.derive(selections)
    })
}

type Selections = [([u8; 32], [u8; 32])];
type ReplayResult<State> = Result<(TuningPlan, State), TuningPlanError>;

fn search_with_replay<State: Borrow<KirVerifiedProgramState>>(
    space: &TuningSpace,
    budget: TuneBudget,
    mut replay: impl FnMut(&Selections) -> ReplayResult<State>,
) -> Result<SearchFrontier, TuningPlanError> {
    let contract = budget.contract();
    let (baseline_plan, baseline_state) = replay(&[])?;
    let baseline = metrics_for(baseline_state.borrow(), baseline_plan)?;
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
                let derived = match extend_plan(&mut replay, &parent, unit, variant.variant_id) {
                    Ok(derived) => derived,
                    Err(TuningPlanError::IllegalAlternative(_)) => {
                        expansions.push(ExpansionRecord {
                            ordinal,
                            parent_plan_digest: parent.digest,
                            unit_id: unit.unit_id,
                            variant_id: variant.variant_id,
                            disposition: ExpansionDisposition::Illegal,
                            result_plan_digest: None,
                            diagnostic_code: 1,
                            whole_plan_dynamic: None,
                            whole_plan_static: None,
                            whole_plan_kir_bytes: None,
                        });
                        continue;
                    }
                    Err(TuningPlanError::GrowthRejected(_)) => {
                        expansions.push(ExpansionRecord {
                            ordinal,
                            parent_plan_digest: parent.digest,
                            unit_id: unit.unit_id,
                            variant_id: variant.variant_id,
                            disposition: ExpansionDisposition::GrowthRejected,
                            result_plan_digest: None,
                            diagnostic_code: 2,
                            whole_plan_dynamic: None,
                            whole_plan_static: None,
                            whole_plan_kir_bytes: None,
                        });
                        continue;
                    }
                    Err(error) => return Err(error),
                };
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
                    result_plan_digest: Some(derived.digest),
                    diagnostic_code: 0,
                    whole_plan_dynamic: Some(derived.predicted_dynamic),
                    whole_plan_static: Some(derived.predicted_static),
                    whole_plan_kir_bytes: Some(derived.kir_bytes),
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

fn extend_plan<State: Borrow<KirVerifiedProgramState>>(
    replay: &mut impl FnMut(&Selections) -> ReplayResult<State>,
    parent: &TuningPlan,
    unit: &TuneUnit,
    variant_id: [u8; 32],
) -> Result<TuningPlan, TuningPlanError> {
    let mut selections = parent
        .choices
        .iter()
        .map(|choice| (choice.unit_id, choice.variant_id))
        .collect::<Vec<_>>();
    selections.push((unit.unit_id, variant_id));
    let (plan, replayed) = replay(&selections)?;
    metrics_for(replayed.borrow(), plan)
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
    plan.predicted_dynamic = base;
    plan.predicted_static = base;
    plan.kir_bytes = u64::try_from(crate::print_kir_module(state.module()).len())
        .map_err(|_| TuningPlanError::ResourceLimit)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_state(source: &str) -> KirVerifiedProgramState {
        let checked = crate::check(&crate::SourceFile::new("search-equivalence.ck", source));
        assert!(checked.diagnostics.is_empty());
        let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR");
        #[cfg(feature = "native-toolchain")]
        let (consumer, profile) = {
            let consumer = crate::KirConsumer::NativeLibrary;
            let target =
                crate::NativeTarget::host_with_cpu(crate::NativeCpu::Native).expect("target");
            (
                consumer,
                Some(target.kir_profile(consumer).expect("profile")),
            )
        };
        #[cfg(not(feature = "native-toolchain"))]
        let (consumer, profile) = (crate::KirConsumer::C, None);
        let config = crate::KirBuildConfig {
            consumer,
            overflow_mode: crate::KirOverflowMode::Unchecked,
            bounds_mode: crate::KirBoundsMode::Unchecked,
            sanitizer_mode: crate::KirSanitizerMode::Disabled,
        };
        let module = match profile {
            Some(profile) => crate::build_kir_module_with_profile(&mir, config, profile),
            None => crate::build_kir_module(&mir, config),
        }
        .expect("KIR");
        let facts =
            crate::import_contract_facts(&module, &checked.checked_program, 0).expect("facts");
        crate::prepare_kir_pre_tune_state(module, Some(&facts)).expect("pre-tune")
    }

    #[test]
    fn cached_search_should_preserve_the_complete_uncached_frontier_in_every_budget() {
        for source in [
            include_str!("../../benches/fixtures/pgo/branch_layout.ck"),
            include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
        ] {
            let state = fixture_state(source);
            let checked = CheckedTuningSpace::enumerate(&state).expect("space");
            for budget in [
                TuneBudget::Quick,
                TuneBudget::Standard,
                TuneBudget::Thorough,
            ] {
                let cached = run_checked_tuning_search(&checked, budget).expect("cached search");
                let uncached = search_with_replay(checked.space(), budget, |selections| {
                    checked.derive(selections)
                })
                .expect("independent uncached search");
                assert_eq!(
                    cached, uncached,
                    "every expansion, metric, order and digest: {budget:?}"
                );
            }
        }
    }

    #[test]
    fn extending_a_final_prefix_should_preserve_independent_pre_and_post_states() {
        let state = fixture_state(include_str!(
            "../../benches/fixtures/pgo/call_constant_length.ck"
        ));
        let checked = CheckedTuningSpace::enumerate(&state).expect("space");
        let search = run_checked_tuning_search(&checked, TuneBudget::Thorough).expect("search");
        let mut replay = checked.search_replay();
        let mut saw_compound = false;
        for plan in search.compile_selection {
            let selections = plan
                .choices
                .iter()
                .map(|choice| (choice.unit_id, choice.variant_id))
                .collect::<Vec<_>>();
            saw_compound |= selections.len() > 1;
            for length in 0..=selections.len() {
                let actual = replay
                    .derive(&selections[..length])
                    .map(|(plan, state)| (plan, std::rc::Rc::unwrap_or_clone(state)))
                    .expect("retained prefix");
                let expected = checked
                    .derive(&selections[..length])
                    .expect("uncached prefix");
                assert_eq!(
                    actual, expected,
                    "including complete evidence/allocator state, prefix {length}"
                );
            }
            // Revisit shorter, previously final prefixes after their extension.
            for length in (0..selections.len()).rev() {
                assert_eq!(
                    replay
                        .derive(&selections[..length])
                        .map(|(plan, state)| (plan, std::rc::Rc::unwrap_or_clone(state))),
                    checked.derive(&selections[..length])
                );
            }
        }
        assert!(saw_compound, "exercise the last-choice suffix distinction");
    }
}
