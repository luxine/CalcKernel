use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::{
    CandidateBudgetCharge, ContractInstanceSource, KirInstructionKind, KirVerifiedProgramState,
    SpecializationFactSource, SpecializationFactValue, SpecializationPlan, TransactionCheckError,
    ValueId, kir_function_units,
};

#[must_use]
pub const fn specialization_profitability_threshold(
    original_cost: u32,
    transformed_cost: u32,
) -> bool {
    original_cost >= transformed_cost.saturating_add(2)
        && (transformed_cost as u64).saturating_mul(100)
            <= (original_cost as u64).saturating_mul(90)
}

/// Checks a materialized specialization without invoking discovery, proposal,
/// scalar optimization, or a proposal-side profitability routine.
pub fn check_specialization_plan_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &SpecializationPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_specialization_plan(pre_state, trial, plan, charge, true)
}

/// Checks the same closed specialization transaction for offline tuning while
/// deliberately leaving profitability to measurement and selection.
pub fn check_tuned_specialization_plan_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &SpecializationPlan,
    charge: &CandidateBudgetCharge,
) -> Result<(), TransactionCheckError> {
    check_specialization_plan(pre_state, trial, plan, charge, false)
}

fn check_specialization_plan(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &SpecializationPlan,
    charge: &CandidateBudgetCharge,
    require_profitability: bool,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    if plan.pre_state.kir_digest != pre_state.kir_digest()
        || plan.pre_state.profile_digest != pre_state.module().profile.digest_hex()
        || plan.pre_state.evidence_generation != pre_state.evidence_generation()
        || plan.pre_state.function != plan.callee
        || plan.o3_entry_module_units != pre_state.optimization_entry_module_units()
    {
        return malformed("specialization pre-state identity is stale");
    }
    let Some(original) = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.callee)
    else {
        return malformed("specialization original callee is missing");
    };
    let Some(pre_caller) = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.caller)
    else {
        return malformed("specialization caller is missing");
    };
    let Some((pre_block, pre_call)) = call(pre_caller, plan.call) else {
        return malformed("specialization direct call is missing");
    };
    let KirInstructionKind::Call {
        function_name: original_target,
        args,
    } = &pre_call.kind
    else {
        return malformed("specialization site is not a direct call");
    };
    if original_target != &original.name {
        return malformed("specialization call target or proof site is false");
    }
    if plan.pre_state.frozen_kir_units != kir_function_units(original) {
        return malformed("specialization frozen callee size is false");
    }
    verify_facts(pre_state, pre_block.id, args, plan)?;
    if canonical_fact_digest(plan) != plan.fact_set_digest {
        return malformed("specialization fact-set digest is false");
    }

    let expected_name = format!(
        "__ck_spec_{}_f{}_{}",
        original.name,
        original.id.index(),
        plan.fact_set_digest
    );
    if plan.clone_name != expected_name || plan.clone_ordinal >= 3 {
        return malformed("specialization clone identity or ordinal is false");
    }
    let Some(clone) = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.clone)
    else {
        return malformed("specialization clone is missing from the trial");
    };
    if clone.name != plan.clone_name
        || clone.exported
        || clone.params.len() != original.params.len()
        || clone
            .params
            .iter()
            .zip(&original.params)
            .any(|(cloned, source)| {
                cloned.name != source.name || cloned.type_node != source.type_node
            })
        || clone.return_type != original.return_type
    {
        return malformed("specialization clone changes the internal signature");
    }
    let Some(trial_original) = trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == plan.callee)
    else {
        return malformed("specialization trial removed the generic body");
    };
    if trial_original != original {
        return malformed("specialization trial mutated the generic body");
    }
    verify_only_call_and_clone_changed(pre_state, trial, plan)?;
    verify_mapping(pre_state, original, clone, plan)?;

    let original_units = kir_function_units(original);
    let clone_units = kir_function_units(clone);
    if plan.cost.scalar != original_units
        || plan.cost.transformed_body != clone_units
        || plan.cost.predicates != 0
        || plan.cost.epilogue != 0
        || plan.cost.total != clone_units
    {
        return malformed("specialization scalar cost record is false");
    }
    if require_profitability && !specialization_profitability_threshold(original_units, clone_units)
    {
        return Err(TransactionCheckError::reject(
            "specialization-profitability-threshold-not-met",
        ));
    }
    verify_growth(pre_state, trial, original_units, clone_units, plan)?;
    verify_proofs(trial, pre_block.id, plan)?;
    if charge != &recompute_charge(plan) {
        return malformed("specialization caller/callee budget charge is false");
    }
    Ok(())
}

fn verify_facts(
    pre_state: &KirVerifiedProgramState,
    block: crate::BlockId,
    args: &[ValueId],
    plan: &SpecializationPlan,
) -> Result<(), TransactionCheckError> {
    let malformed = |message: &str| Err(TransactionCheckError::compiler(message));
    let mut previous = None;
    for fact in &plan.facts {
        let stable = fact.stable_text();
        if previous.as_ref().is_some_and(|value| value >= &stable) {
            return malformed("specialization facts are not in canonical strict order");
        }
        previous = Some(stable);
        let index = usize::try_from(fact.parameter_index)
            .map_err(|_| TransactionCheckError::compiler("fact parameter index is invalid"))?;
        let Some(argument) = args.get(index).copied() else {
            return malformed("specialization fact parameter is absent at the call");
        };
        match fact.source {
            SpecializationFactSource::Constant {
                instruction: source_id,
            } => {
                let Some((source_block, source)) = pre_state
                    .module()
                    .functions
                    .iter()
                    .find(|function| function.id == plan.caller)
                    .and_then(|function| instruction(function, source_id))
                else {
                    return malformed("specialization constant source is missing");
                };
                if source_block.id == block
                    && source.id.index() >= plan.call.index()
                    && source.id != plan.call
                {
                    return malformed("specialization constant does not dominate the call");
                }
                let source_value = source.results.first().map(|result| result.value);
                let caller = pre_state
                    .module()
                    .functions
                    .iter()
                    .find(|function| function.id == plan.caller)
                    .expect("caller established");
                let valid = match (&fact.value, &source.kind) {
                    (
                        SpecializationFactValue::Integer { value },
                        KirInstructionKind::ConstInt { value: actual },
                    ) => {
                        source_value.is_some_and(|value| resolves_to(caller, argument, value))
                            && actual == value
                    }
                    (
                        SpecializationFactValue::Boolean { value },
                        KirInstructionKind::ConstBool { value: actual },
                    ) => {
                        source_value.is_some_and(|value| resolves_to(caller, argument, value))
                            && actual == value
                    }
                    (
                        SpecializationFactValue::Float { value },
                        KirInstructionKind::ConstFloat { value: actual },
                    ) => {
                        source_value.is_some_and(|value| resolves_to(caller, argument, value))
                            && actual == value
                    }
                    (
                        SpecializationFactValue::SliceLength { length },
                        KirInstructionKind::ConstInt { value },
                    ) => defining_instruction_following_copies(caller, argument).is_some_and(
                        |make_slice| {
                            matches!(
                                make_slice.kind,
                                KirInstructionKind::MakeSlice { len, .. }
                                    if source_value.is_some_and(|source| resolves_to(caller, len, source))
                                        && value.parse::<u32>().ok() == Some(*length)
                            )
                        },
                    ),
                    _ => false,
                };
                if !valid {
                    return malformed("specialization constant fact is false at the call");
                }
            }
            SpecializationFactSource::TrustedContract { instance, fact } => {
                let Some(contracts) = pre_state.contract_facts() else {
                    return malformed("specialization trusted fact table is missing");
                };
                let Some(record) = contracts
                    .instances()
                    .iter()
                    .find(|item| item.id == instance)
                else {
                    return malformed("specialization trusted instance is missing");
                };
                if !record.facts.contains(&fact)
                    || !matches!(
                        record.source,
                        ContractInstanceSource::Call {
                            caller,
                            block: source_block,
                            instruction,
                        } if caller == plan.caller && source_block == block && instruction == plan.call
                    )
                {
                    return malformed("specialization trusted fact crosses a call instance");
                }
            }
        }
    }
    Ok(())
}

fn verify_only_call_and_clone_changed(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    plan: &SpecializationPlan,
) -> Result<(), TransactionCheckError> {
    for source in &pre_state.module().functions {
        let Some(target) = trial
            .module()
            .functions
            .iter()
            .find(|function| function.id == source.id)
        else {
            return Err(TransactionCheckError::compiler(
                "specialization removed a pre-state function",
            ));
        };
        if source.id != plan.caller {
            if target != source {
                return Err(TransactionCheckError::compiler(
                    "specialization mutated an unrelated function",
                ));
            }
            continue;
        }
        let mut restored = target.clone();
        let Some(call) = call_mut(&mut restored, plan.call) else {
            return Err(TransactionCheckError::compiler(
                "specialization redirected call is missing",
            ));
        };
        let KirInstructionKind::Call { function_name, .. } = &mut call.kind else {
            return Err(TransactionCheckError::compiler(
                "specialization redirected a non-call",
            ));
        };
        if function_name != &plan.clone_name {
            return Err(TransactionCheckError::compiler(
                "specialization call does not target its clone",
            ));
        }
        let original_name = pre_state
            .module()
            .functions
            .iter()
            .find(|function| function.id == plan.callee)
            .map(|function| function.name.clone())
            .expect("callee established");
        *function_name = original_name;
        if &restored != source {
            return Err(TransactionCheckError::compiler(
                "specialization changed caller state beyond one call target",
            ));
        }
    }
    Ok(())
}

fn verify_mapping(
    pre_state: &KirVerifiedProgramState,
    original: &crate::KirFunction,
    clone: &crate::KirFunction,
    plan: &SpecializationPlan,
) -> Result<(), TransactionCheckError> {
    let original_blocks = original
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let clone_blocks = clone
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let original_instructions = original
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id)
        .collect::<BTreeSet<_>>();
    let clone_instructions = clone
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id)
        .collect::<BTreeSet<_>>();
    if plan.mapping.parameters.len() != original.params.len()
        || plan
            .mapping
            .parameters
            .iter()
            .zip(original.params.iter().zip(&clone.params))
            .any(|((source, target), (original, cloned))| {
                *source != original.value
                    || *target != cloned.value
                    || (!plan.reused && target.index() < pre_state.ids().next_value)
                    || (plan.reused && target.index() >= pre_state.ids().next_value)
            })
        || (!plan.reused
            && plan.mapping.blocks.iter().any(|(source, target)| {
                !original_blocks.contains(source)
                    || !clone_blocks.contains(target)
                    || target.index() < pre_state.ids().next_block
            }))
        || (!plan.reused
            && plan.mapping.instructions.iter().any(|(source, target)| {
                !original_instructions.contains(source)
                    || !clone_instructions.contains(target)
                    || target.index() < pre_state.ids().next_instruction
            }))
        || !mapping_is_strict_and_unique(&plan.mapping.blocks)
        || !mapping_is_strict_and_unique(&plan.mapping.instructions)
        || (!plan.reused && plan.clone.index() < pre_state.ids().next_function)
        || (plan.reused && plan.clone.index() >= pre_state.ids().next_function)
    {
        return Err(TransactionCheckError::compiler(
            "specialization original-to-clone ID mapping is false",
        ));
    }
    Ok(())
}

fn mapping_is_strict_and_unique<Left: Ord + Copy, Right: Ord + Copy>(
    mapping: &[(Left, Right)],
) -> bool {
    mapping.windows(2).all(|pair| pair[0].0 < pair[1].0)
        && mapping
            .iter()
            .map(|(_, target)| *target)
            .collect::<BTreeSet<_>>()
            .len()
            == mapping.len()
}

fn verify_growth(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    original_units: u32,
    clone_units: u32,
    plan: &SpecializationPlan,
) -> Result<(), TransactionCheckError> {
    let before = module_units(pre_state);
    let after = module_units(trial);
    let entry = pre_state.optimization_entry_module_units();
    let allowance = entry.div_ceil(4).clamp(64, 4096);
    if plan.growth.original_units != original_units
        || plan.growth.transformed_units != clone_units
        || plan.growth.module_before_units != before
        || plan.growth.module_after_units != after
    {
        return Err(TransactionCheckError::compiler(
            "specialization growth record is false",
        ));
    }
    if after.saturating_sub(entry) > allowance || after > entry.saturating_mul(2) {
        return Err(TransactionCheckError::reject(
            "specialization-code-growth-limit",
        ));
    }
    let clones = trial
        .module()
        .functions
        .iter()
        .filter(|function| function.name.starts_with("__ck_spec_"))
        .count();
    if clones > 24 {
        return Err(TransactionCheckError::reject(
            "specialization-module-clone-limit",
        ));
    }
    Ok(())
}

fn verify_proofs(
    trial: &KirVerifiedProgramState,
    block: crate::BlockId,
    plan: &SpecializationPlan,
) -> Result<(), TransactionCheckError> {
    if plan.argument_mapping_proof == plan.fact_scope_proof {
        return Err(TransactionCheckError::compiler(
            "specialization proof roots are not distinct",
        ));
    }
    for proof in [plan.argument_mapping_proof, plan.fact_scope_proof] {
        let Some(certificate) = trial.proofs().get(proof) else {
            return Err(TransactionCheckError::compiler(
                "specialization proof root is missing",
            ));
        };
        let trusted_instance = plan.facts.iter().find_map(|fact| match fact.source {
            SpecializationFactSource::TrustedContract { instance, .. } => Some(instance),
            _ => None,
        });
        let site_matches = if let Some(instance) = trusted_instance {
            certificate.use_site.function == plan.callee
                && certificate.use_site.instruction.is_none()
                && certificate.use_site.contract_instance == Some(instance)
        } else {
            certificate.use_site.function == plan.caller
                && certificate.use_site.block == block
                && certificate.use_site.instruction == Some(plan.call)
                && certificate.use_site.contract_instance.is_none()
        };
        if !site_matches || certificate.generation != trial.evidence_generation() {
            return Err(TransactionCheckError::compiler(
                "specialization proof root has a false use site",
            ));
        }
    }
    Ok(())
}

fn canonical_fact_digest(plan: &SpecializationPlan) -> String {
    let mut facts = plan
        .facts
        .iter()
        .map(crate::SpecializationFact::semantic_text)
        .collect::<Vec<_>>();
    facts.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"ck-specialization-facts-v1\0");
    for fact in facts {
        hasher.update((fact.len() as u64).to_le_bytes());
        hasher.update(fact.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn recompute_charge(plan: &SpecializationPlan) -> CandidateBudgetCharge {
    let facts = u32::try_from(plan.facts.len()).unwrap_or(u32::MAX);
    let mappings = [
        plan.mapping.parameters.len(),
        plan.mapping.blocks.len(),
        plan.mapping.instructions.len(),
        plan.mapping.values.len(),
        plan.mapping.memory_regions.len(),
        plan.mapping.memory_versions.len(),
        plan.mapping.vector_regions.len(),
    ]
    .into_iter()
    .fold(0_u32, |total, count| {
        total.saturating_add(u32::try_from(count).unwrap_or(u32::MAX))
    });
    CandidateBudgetCharge {
        functions: vec![plan.caller, plan.callee],
        proposer_units: 8_u32
            .saturating_add(plan.pre_state.frozen_kir_units)
            .saturating_add(facts.saturating_mul(4)),
        checker_units: 16_u32
            .saturating_add(plan.pre_state.frozen_kir_units)
            .saturating_add(plan.growth.transformed_units)
            .saturating_add(facts.saturating_mul(6))
            .saturating_add(mappings),
    }
}

fn module_units(state: &KirVerifiedProgramState) -> u32 {
    state
        .module()
        .functions
        .iter()
        .fold(0_u32, |total, function| {
            total.saturating_add(kir_function_units(function))
        })
}

fn call(
    function: &crate::KirFunction,
    id: crate::InstructionId,
) -> Option<(&crate::KirBlock, &crate::KirInstruction)> {
    function.blocks.iter().find_map(|block| {
        block
            .instructions
            .iter()
            .find(|instruction| instruction.id == id)
            .map(|instruction| (block, instruction))
    })
}

fn call_mut(
    function: &mut crate::KirFunction,
    id: crate::InstructionId,
) -> Option<&mut crate::KirInstruction> {
    function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == id)
}

fn instruction(
    function: &crate::KirFunction,
    id: crate::InstructionId,
) -> Option<(&crate::KirBlock, &crate::KirInstruction)> {
    call(function, id)
}

fn defining_instruction(
    function: &crate::KirFunction,
    value: ValueId,
) -> Option<&crate::KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
}

fn defining_instruction_following_copies(
    function: &crate::KirFunction,
    mut value: ValueId,
) -> Option<&crate::KirInstruction> {
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(value) {
            return None;
        }
        let instruction = defining_instruction(function, value)?;
        if let KirInstructionKind::Copy { value: input } = instruction.kind {
            value = input;
        } else {
            return Some(instruction);
        }
    }
}

fn resolves_to(function: &crate::KirFunction, mut value: ValueId, source: ValueId) -> bool {
    let mut visited = BTreeSet::new();
    loop {
        if value == source {
            return true;
        }
        if !visited.insert(value) {
            return false;
        }
        let Some(instruction) = defining_instruction(function, value) else {
            return false;
        };
        let KirInstructionKind::Copy { value: input } = instruction.kind else {
            return false;
        };
        value = input;
    }
}
