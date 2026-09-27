use std::collections::BTreeSet;

use crate::{ContractInstanceSource, EffectSolveConfig, KirModule};

use super::super::{ContractFactSet, refine_memory_ssa_with_effects, solve_kir_effects};

pub(crate) fn run_memory_ssa_refine(
    module: &mut KirModule,
    contracts: Option<&ContractFactSet>,
) -> Result<bool, String> {
    let before = module.clone();
    let unsafe_functions = contracts.map_or_else(BTreeSet::new, |contracts| {
        contracts
            .instances()
            .iter()
            .filter(|instance| matches!(instance.source, ContractInstanceSource::FunctionEntry))
            .filter_map(|instance| {
                module
                    .functions
                    .iter()
                    .find(|function| function.id == instance.callee)
                    .map(|function| function.name.clone())
            })
            .collect()
    });
    let solved = solve_kir_effects(module, &unsafe_functions, EffectSolveConfig::default());
    refine_memory_ssa_with_effects(
        module,
        contracts.map(ContractFactSet::facts),
        &solved.summaries,
    )
    .map_err(|error| error.message)?;
    Ok(*module != before)
}
