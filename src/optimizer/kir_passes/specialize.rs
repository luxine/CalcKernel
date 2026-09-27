use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, FactUseSite, KirEffectKind, KirInstruction, KirInstructionKind, KirMemoryRegionOrigin,
    KirOrderedEffect, KirPlace, KirResult, KirTerminator, KirValueType, MemoryRegionId,
    MemoryVersionId, ProofStep, ProofStepId, ScalarClaim, ScalarFailure, ScalarInterval,
    SpecializationCandidate, SpecializationFactSource, SpecializationFactValue,
    SpecializationIdMapping, SpecializationPlan, ValueId, VectorRegionId,
};

use super::{
    super::{
        CandidateBudgetCharge, KirGuardElimination, KirPreStateIdentity, KirVerifiedProgramState,
        ProofArena, VectorPlanGrowth, kir_function_units,
    },
    rewrite::{remap_instruction_values, remap_terminator_values, replace_value_uses},
};

#[derive(Debug, Clone)]
pub(crate) struct MaterializedSpecialization {
    pub trial: KirVerifiedProgramState,
    pub plan: SpecializationPlan,
    pub charge: CandidateBudgetCharge,
}

pub(crate) fn materialize_specialization_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &SpecializationCandidate,
    clone_ordinal: u8,
) -> Result<MaterializedSpecialization, String> {
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.callee)
        .ok_or_else(|| "specialization callee is missing".to_string())?
        .clone();
    let module_before_units = module_units(pre_state.module());
    let original_units = kir_function_units(&original);
    let clone_name =
        specialization_clone_name(&original.name, original.id, &candidate.fact_set_digest);
    if let Some(existing) = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.name == clone_name)
    {
        let existing = existing.clone();
        let mut trial = pre_state.clone();
        redirect_call(&mut trial, candidate, &clone_name)?;
        let (argument_mapping_proof, fact_scope_proof) =
            append_specialization_proofs(&mut trial, candidate)?;
        let mapping = SpecializationIdMapping {
            parameters: original
                .params
                .iter()
                .zip(&existing.params)
                .map(|(source, target)| (source.value, target.value))
                .collect(),
            blocks: Vec::new(),
            instructions: Vec::new(),
            values: Vec::new(),
            memory_regions: Vec::new(),
            memory_versions: Vec::new(),
            vector_regions: Vec::new(),
        };
        let plan = SpecializationPlan {
            pre_state: KirPreStateIdentity {
                function: candidate.callee,
                kir_digest: pre_state.kir_digest(),
                profile_digest: pre_state.module().profile.digest_hex(),
                evidence_generation: pre_state.evidence_generation(),
                frozen_kir_units: original_units,
            },
            caller: candidate.caller,
            call: candidate.call,
            callee: candidate.callee,
            fact_set_digest: candidate.fact_set_digest.clone(),
            clone_ordinal,
            clone: existing.id,
            clone_name,
            reused: true,
            o3_entry_module_units: pre_state.optimization_entry_module_units(),
            facts: candidate.facts.clone(),
            mapping,
            cost: crate::KirCostEstimate::new(original_units, kir_function_units(&existing), 0, 0),
            growth: VectorPlanGrowth::new(
                original_units,
                kir_function_units(&existing),
                module_before_units,
                module_before_units,
            ),
            argument_mapping_proof,
            fact_scope_proof,
        };
        let charge = specialization_charge(&plan);
        return Ok(MaterializedSpecialization {
            trial,
            plan,
            charge,
        });
    }

    let mut trial = pre_state.clone();
    let (mut cloned, mut mapping) =
        clone_with_fresh_ids(&mut trial, &original, clone_name.clone())?;
    restore_eliminated_guards(
        &mut trial,
        &mut cloned,
        &mapping,
        pre_state.eliminated_guards(),
        original.id,
    )?;
    substitute_facts(&mut trial, &mut cloned, &mapping, &candidate.facts)?;
    finalize_clone_locally(&mut trial, &mut cloned)?;
    retain_live_mapping(&mut mapping, &cloned);
    let clone_id = cloned.id;
    let clone_units = kir_function_units(&cloned);
    trial.module_mut().functions.push(cloned);
    redirect_call(&mut trial, candidate, &clone_name)?;
    let (argument_mapping_proof, fact_scope_proof) =
        append_specialization_proofs(&mut trial, candidate)?;

    let module_after_units = module_units(trial.module());
    let plan = SpecializationPlan {
        pre_state: KirPreStateIdentity {
            function: candidate.callee,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: original_units,
        },
        caller: candidate.caller,
        call: candidate.call,
        callee: candidate.callee,
        fact_set_digest: candidate.fact_set_digest.clone(),
        clone_ordinal,
        clone: clone_id,
        clone_name,
        reused: false,
        o3_entry_module_units: pre_state.optimization_entry_module_units(),
        facts: candidate.facts.clone(),
        mapping,
        cost: crate::KirCostEstimate::new(original_units, clone_units, 0, 0),
        growth: VectorPlanGrowth::new(
            original_units,
            clone_units,
            module_before_units,
            module_after_units,
        ),
        argument_mapping_proof,
        fact_scope_proof,
    };
    let charge = specialization_charge(&plan);
    Ok(MaterializedSpecialization {
        trial,
        plan,
        charge,
    })
}

#[must_use]
pub(crate) fn specialization_clone_name(
    original: &str,
    id: crate::FunctionId,
    digest: &str,
) -> String {
    format!("__ck_spec_{original}_f{}_{digest}", id.index())
}

#[must_use]
pub(crate) fn specialization_charge(plan: &SpecializationPlan) -> CandidateBudgetCharge {
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

fn clone_with_fresh_ids(
    state: &mut KirVerifiedProgramState,
    original: &crate::KirFunction,
    name: String,
) -> Result<(crate::KirFunction, SpecializationIdMapping), String> {
    let clone_id = state.fresh_function()?;
    let mut blocks = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut instructions = BTreeMap::new();
    let mut regions = BTreeMap::new();
    let mut memories = BTreeMap::new();
    let mut vector_regions = BTreeMap::new();

    for parameter in &original.params {
        values.insert(parameter.value, state.fresh_value()?);
    }
    for region in &original.regions {
        regions.insert(region.id, state.fresh_memory_region()?);
    }
    for region in &original.vector_regions {
        vector_regions.insert(region.id, state.fresh_vector_region()?);
    }
    for initial in &original.initial_memory {
        memories.insert(initial.version, state.fresh_memory_version()?);
    }
    for block in &original.blocks {
        blocks.insert(block.id, state.fresh_block()?);
        for parameter in &block.params {
            values.insert(parameter.value, state.fresh_value()?);
        }
        for memory in &block.memory_params {
            memories
                .entry(memory.version)
                .or_insert(state.fresh_memory_version()?);
        }
        for instruction in &block.instructions {
            instructions.insert(instruction.id, state.fresh_instruction()?);
            for result in &instruction.results {
                values.insert(result.value, state.fresh_value()?);
            }
            if let Some(memory) = &instruction.memory {
                memories
                    .entry(memory.input)
                    .or_insert(state.fresh_memory_version()?);
                if let Some(output) = memory.output {
                    memories
                        .entry(output)
                        .or_insert(state.fresh_memory_version()?);
                }
            }
        }
    }

    let mut cloned = original.clone();
    cloned.id = clone_id;
    cloned.name = name;
    cloned.exported = false;
    for parameter in &mut cloned.params {
        parameter.value = values[&parameter.value];
    }
    for region in &mut cloned.regions {
        region.id = regions[&region.id];
        region.parent = region.parent.map(|id| regions[&id]);
        region.partition = regions[&region.partition];
        match &mut region.origin {
            KirMemoryRegionOrigin::Parameter(value)
            | KirMemoryRegionOrigin::RawSlice(value)
            | KirMemoryRegionOrigin::Subslice(value) => *value = values[value],
            KirMemoryRegionOrigin::Conservative => {}
        }
        if let Some(interval) = &mut region.byte_interval {
            interval.start = values[&interval.start];
            interval.end = values[&interval.end];
        }
    }
    for initial in &mut cloned.initial_memory {
        initial.region = regions[&initial.region];
        initial.version = memories[&initial.version];
    }
    for vector_region in &mut cloned.vector_regions {
        vector_region.id = vector_regions[&vector_region.id];
        for block in &mut vector_region.blocks {
            *block = blocks[block];
        }
    }
    for block in &mut cloned.blocks {
        block.id = blocks[&block.id];
        for parameter in &mut block.params {
            parameter.value = values[&parameter.value];
        }
        for memory in &mut block.memory_params {
            memory.version = memories[&memory.version];
            memory.region = regions[&memory.region];
        }
        for instruction in &mut block.instructions {
            instruction.id = instructions[&instruction.id];
            for result in &mut instruction.results {
                result.value = values[&result.value];
            }
            remap_instruction_values(instruction, &values);
            remap_instruction_regions(instruction, &regions, &vector_regions);
            if let Some(memory) = &mut instruction.memory {
                memory.region = regions[&memory.region];
                memory.input = memories[&memory.input];
                memory.output = memory.output.map(|version| memories[&version]);
            }
        }
        remap_terminator_values(&mut block.terminator, &values);
        remap_terminator(&mut block.terminator, &blocks, &regions, &memories);
    }

    let mapping = SpecializationIdMapping {
        parameters: original
            .params
            .iter()
            .map(|parameter| (parameter.value, values[&parameter.value]))
            .collect(),
        blocks: blocks.into_iter().collect(),
        instructions: instructions.into_iter().collect(),
        values: values.into_iter().collect(),
        memory_regions: regions.into_iter().collect(),
        memory_versions: memories.into_iter().collect(),
        vector_regions: vector_regions.into_iter().collect(),
    };
    Ok((cloned, mapping))
}

fn restore_eliminated_guards(
    state: &mut KirVerifiedProgramState,
    cloned: &mut crate::KirFunction,
    mapping: &SpecializationIdMapping,
    eliminations: &[KirGuardElimination],
    original: crate::FunctionId,
) -> Result<(), String> {
    let instruction_map = mapping
        .instructions
        .iter()
        .copied()
        .collect::<BTreeMap<_, _>>();
    for elimination in eliminations
        .iter()
        .filter(|elimination| elimination.function == original)
    {
        let Some(condition_id) = instruction_map
            .get(&elimination.condition_instruction)
            .copied()
        else {
            continue;
        };
        let Some((block_index, instruction_index)) =
            cloned
                .blocks
                .iter()
                .enumerate()
                .find_map(|(block_index, block)| {
                    block
                        .instructions
                        .iter()
                        .position(|instruction| instruction.id == condition_id)
                        .map(|instruction_index| (block_index, instruction_index))
                })
        else {
            return Err("specialization guard condition mapping is missing".to_string());
        };
        let Some((condition, failure)) =
            restored_guard(&cloned.blocks[block_index].instructions[instruction_index])
        else {
            return Err("specialization cannot reconstruct an eliminated guard".to_string());
        };
        let guard = KirInstruction {
            id: state.fresh_instruction()?,
            results: Vec::new(),
            kind: KirInstructionKind::Guard { condition, failure },
            memory: None,
            effect: Some(KirOrderedEffect {
                order: u32::MAX,
                kind: KirEffectKind::MayFail,
            }),
        };
        cloned.blocks[block_index]
            .instructions
            .insert(instruction_index + 1, guard);
    }
    let mut temporary = module_with_function(state.module(), cloned.clone());
    super::run_cleanup(&mut temporary);
    *cloned = temporary.functions.remove(0);
    Ok(())
}

fn restored_guard(instruction: &KirInstruction) -> Option<(ValueId, crate::KirFailureKind)> {
    match &instruction.kind {
        KirInstructionKind::Binary { .. } | KirInstructionKind::Unary { .. } => instruction
            .results
            .get(1)
            .map(|result| (result.value, crate::KirFailureKind::Overflow)),
        KirInstructionKind::CheckCondition { kind, .. } => {
            let failure = match kind {
                crate::KirCheckConditionKind::ArithmeticOverflow
                | crate::KirCheckConditionKind::SignedDivisionOverflow => {
                    crate::KirFailureKind::Overflow
                }
                crate::KirCheckConditionKind::DivisionByZero => {
                    crate::KirFailureKind::DivisionByZero
                }
                crate::KirCheckConditionKind::SliceOutOfBounds
                | crate::KirCheckConditionKind::InvalidSubslice => {
                    crate::KirFailureKind::OutOfBounds
                }
            };
            instruction
                .results
                .first()
                .map(|result| (result.value, failure))
        }
        _ => None,
    }
}

fn substitute_facts(
    state: &mut KirVerifiedProgramState,
    cloned: &mut crate::KirFunction,
    mapping: &SpecializationIdMapping,
    facts: &[crate::SpecializationFact],
) -> Result<(), String> {
    let parameter_map = mapping
        .parameters
        .iter()
        .copied()
        .collect::<BTreeMap<_, _>>();
    for fact in facts {
        let index = usize::try_from(fact.parameter_index)
            .map_err(|_| "specialization parameter index is invalid")?;
        let original_parameter = mapping
            .parameters
            .get(index)
            .map(|(original, _)| *original)
            .ok_or_else(|| "specialization fact parameter is missing".to_string())?;
        let cloned_parameter = parameter_map[&original_parameter];
        match &fact.value {
            SpecializationFactValue::Integer { value } => {
                let result = state.fresh_value()?;
                let instruction = state.fresh_instruction()?;
                let type_node = cloned.params[index].type_node.clone();
                replace_value_uses(cloned, cloned_parameter, result);
                cloned.blocks[0].instructions.insert(
                    0,
                    KirInstruction {
                        id: instruction,
                        results: vec![KirResult {
                            value: result,
                            type_node: KirValueType::Scalar(type_node),
                        }],
                        kind: KirInstructionKind::ConstInt {
                            value: value.clone(),
                        },
                        memory: None,
                        effect: None,
                    },
                );
            }
            SpecializationFactValue::Boolean { value } => {
                let result = state.fresh_value()?;
                let instruction = state.fresh_instruction()?;
                let type_node = cloned.params[index].type_node.clone();
                replace_value_uses(cloned, cloned_parameter, result);
                cloned.blocks[0].instructions.insert(
                    0,
                    KirInstruction {
                        id: instruction,
                        results: vec![KirResult {
                            value: result,
                            type_node: KirValueType::Scalar(type_node),
                        }],
                        kind: KirInstructionKind::ConstBool { value: *value },
                        memory: None,
                        effect: None,
                    },
                );
            }
            SpecializationFactValue::Float { value } => {
                let result = state.fresh_value()?;
                let instruction = state.fresh_instruction()?;
                let type_node = cloned.params[index].type_node.clone();
                replace_value_uses(cloned, cloned_parameter, result);
                cloned.blocks[0].instructions.insert(
                    0,
                    KirInstruction {
                        id: instruction,
                        results: vec![KirResult {
                            value: result,
                            type_node: KirValueType::Scalar(type_node),
                        }],
                        kind: KirInstructionKind::ConstFloat {
                            value: value.clone(),
                        },
                        memory: None,
                        effect: None,
                    },
                );
            }
            SpecializationFactValue::SliceLength { length } => {
                for instruction in cloned
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.instructions)
                {
                    if matches!(instruction.kind, KirInstructionKind::SliceLen { slice } if slice == cloned_parameter)
                    {
                        instruction.kind = KirInstructionKind::ConstInt {
                            value: length.to_string(),
                        };
                    }
                }
            }
        }
    }
    Ok(())
}

fn finalize_clone_locally(
    state: &mut KirVerifiedProgramState,
    cloned: &mut crate::KirFunction,
) -> Result<(), String> {
    let mut module = module_with_function(state.module(), cloned.clone());
    let empty = ProofArena::new(state.evidence_generation());
    for _ in 0..8 {
        let folded = super::run_integer_constant_folding(&mut module, None, &empty)?;
        let cfg = super::run_cfg_canonicalize(&mut module, None);
        if !folded && !cfg {
            break;
        }
    }
    let mut local_proofs = ProofArena::new(state.evidence_generation());
    let mut local_eliminations = Vec::new();
    let mut local_explanations = Vec::new();
    super::run_check_elimination(
        &mut module,
        None,
        &mut local_proofs,
        &mut local_eliminations,
        &mut local_explanations,
        state.evidence_generation(),
        false,
    )?;
    let protected = local_proofs.instruction_dependencies();
    super::run_dead_code_elimination(&mut module, &protected);
    super::run_cleanup(&mut module);

    let mut proof_map = BTreeMap::new();
    for proof in local_proofs.proofs() {
        let global = state
            .proofs_mut()
            .try_insert(proof.use_site, proof.steps.clone(), proof.root)
            .map_err(|error| error.to_string())?;
        proof_map.insert(proof.id, global);
    }
    for mut elimination in local_eliminations {
        elimination.proof = elimination.proof.map(|proof| proof_map[&proof]);
        state.eliminated_guards_mut().push(elimination);
    }
    *cloned = module.functions.remove(0);
    Ok(())
}

fn append_specialization_proofs(
    state: &mut KirVerifiedProgramState,
    candidate: &SpecializationCandidate,
) -> Result<(crate::ProofId, crate::ProofId), String> {
    let mut steps = Vec::new();
    let mut contract_instance = None;
    let trusted = candidate.facts.iter().any(|fact| {
        matches!(
            fact.source,
            SpecializationFactSource::TrustedContract { .. }
        )
    });
    for fact in &candidate.facts {
        match fact.source {
            SpecializationFactSource::Constant { instruction } => {
                if trusted {
                    continue;
                }
                let source = state
                    .module()
                    .functions
                    .iter()
                    .flat_map(|function| &function.blocks)
                    .flat_map(|block| &block.instructions)
                    .find(|item| item.id == instruction)
                    .ok_or_else(|| "specialization constant source is missing".to_string())?;
                let result = source
                    .results
                    .first()
                    .ok_or_else(|| "specialization constant source has no result".to_string())?;
                match (&fact.value, &source.kind) {
                    (
                        SpecializationFactValue::Integer { value },
                        KirInstructionKind::ConstInt { .. },
                    ) => {
                        let point = value
                            .parse::<BigInt>()
                            .map_err(|_| "specialization integer fact is malformed")?;
                        steps.push(ProofStep::Constant {
                            instruction,
                            claim: ScalarClaim::new(
                                result.value,
                                ScalarInterval::new(point.clone(), point)
                                    .map_err(|error| error.to_string())?,
                                ScalarFailure::None,
                            ),
                        });
                    }
                    (
                        SpecializationFactValue::SliceLength { length },
                        KirInstructionKind::ConstInt { .. },
                    ) => {
                        let point = BigInt::from(*length);
                        steps.push(ProofStep::Constant {
                            instruction,
                            claim: ScalarClaim::new(
                                result.value,
                                ScalarInterval::new(point.clone(), point)
                                    .map_err(|error| error.to_string())?,
                                ScalarFailure::None,
                            ),
                        });
                    }
                    (
                        SpecializationFactValue::Boolean { value },
                        KirInstructionKind::ConstBool { .. },
                    ) => steps.push(ProofStep::BooleanTransfer {
                        instruction,
                        inputs: Vec::new(),
                        value: result.value,
                        result: *value,
                    }),
                    _ => {
                        return Err(
                            "specialization fact does not match its constant source".to_string()
                        );
                    }
                }
            }
            SpecializationFactSource::TrustedContract { instance, fact } => {
                if contract_instance
                    .replace(instance)
                    .is_some_and(|old| old != instance)
                {
                    return Err("specialization facts cross contract instances".to_string());
                }
                steps.push(ProofStep::FactLeaf { fact });
            }
        }
    }
    if steps.is_empty() {
        return Err("specialization proof has no facts".to_string());
    }
    let root = ProofStepId::from_index(
        u32::try_from(steps.len() - 1)
            .map_err(|_| "specialization proof exceeds u32 identity space")?,
    );
    let use_site = if trusted {
        let block = state
            .module()
            .functions
            .iter()
            .find(|function| function.id == candidate.callee)
            .and_then(|function| function.blocks.first())
            .map(|block| block.id)
            .ok_or_else(|| "specialization contract proof callee entry is missing".to_string())?;
        FactUseSite {
            function: candidate.callee,
            block,
            instruction: None,
            contract_instance,
        }
    } else {
        FactUseSite {
            function: candidate.caller,
            block: candidate.block,
            instruction: Some(candidate.call),
            contract_instance,
        }
    };
    let argument = state
        .proofs_mut()
        .try_insert(use_site, steps.clone(), root)
        .map_err(|error| error.to_string())?;
    let scope = state
        .proofs_mut()
        .try_insert(use_site, steps, root)
        .map_err(|error| error.to_string())?;
    Ok((argument, scope))
}

fn redirect_call(
    state: &mut KirVerifiedProgramState,
    candidate: &SpecializationCandidate,
    clone_name: &str,
) -> Result<(), String> {
    let instruction = state
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.caller)
        .and_then(|function| {
            function
                .blocks
                .iter_mut()
                .find(|block| block.id == candidate.block)
        })
        .and_then(|block| {
            block
                .instructions
                .iter_mut()
                .find(|instruction| instruction.id == candidate.call)
        })
        .ok_or_else(|| "specialization call site is missing".to_string())?;
    let KirInstructionKind::Call { function_name, .. } = &mut instruction.kind else {
        return Err("specialization site is no longer a direct call".to_string());
    };
    *function_name = clone_name.to_string();
    Ok(())
}

fn module_with_function(
    template: &crate::KirModule,
    function: crate::KirFunction,
) -> crate::KirModule {
    crate::KirModule {
        config: template.config,
        profile: template.profile.clone(),
        entry: None,
        structs: template.structs.clone(),
        functions: vec![function],
    }
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}

fn retain_live_mapping(mapping: &mut SpecializationIdMapping, clone: &crate::KirFunction) {
    let blocks = clone
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let instructions = clone
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id)
        .collect::<BTreeSet<_>>();
    let values = clone
        .params
        .iter()
        .map(|parameter| parameter.value)
        .chain(clone.blocks.iter().flat_map(|block| {
            block.params.iter().map(|parameter| parameter.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        }))
        .collect::<BTreeSet<_>>();
    let regions = clone
        .regions
        .iter()
        .map(|region| region.id)
        .collect::<BTreeSet<_>>();
    let memories = clone
        .initial_memory
        .iter()
        .map(|memory| memory.version)
        .chain(clone.blocks.iter().flat_map(|block| {
            block
                .memory_params
                .iter()
                .map(|memory| memory.version)
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction.memory.iter().flat_map(|memory| {
                        [Some(memory.input), memory.output].into_iter().flatten()
                    })
                }))
        }))
        .collect::<BTreeSet<_>>();
    let vector_regions = clone
        .vector_regions
        .iter()
        .map(|region| region.id)
        .collect::<BTreeSet<_>>();
    mapping.blocks.retain(|(_, target)| blocks.contains(target));
    mapping
        .instructions
        .retain(|(_, target)| instructions.contains(target));
    mapping.values.retain(|(_, target)| values.contains(target));
    mapping
        .memory_regions
        .retain(|(_, target)| regions.contains(target));
    mapping
        .memory_versions
        .retain(|(_, target)| memories.contains(target));
    mapping
        .vector_regions
        .retain(|(_, target)| vector_regions.contains(target));
}

fn remap_instruction_regions(
    instruction: &mut KirInstruction,
    regions: &BTreeMap<MemoryRegionId, MemoryRegionId>,
    vector_regions: &BTreeMap<VectorRegionId, VectorRegionId>,
) {
    match &mut instruction.kind {
        KirInstructionKind::Address { place }
        | KirInstructionKind::Load { place }
        | KirInstructionKind::Store { place, .. } => remap_place_regions(place, regions),
        KirInstructionKind::VectorSplat { region, .. }
        | KirInstructionKind::VectorLoad { region, .. }
        | KirInstructionKind::VectorStore { region, .. }
        | KirInstructionKind::VectorBinary { region, .. }
        | KirInstructionKind::VectorUnary { region, .. }
        | KirInstructionKind::VectorCompare { region, .. }
        | KirInstructionKind::VectorSelect { region, .. }
        | KirInstructionKind::VectorCast { region, .. }
        | KirInstructionKind::VectorInsert { region, .. }
        | KirInstructionKind::VectorExtract { region, .. }
        | KirInstructionKind::VectorReduce { region, .. } => *region = vector_regions[region],
        _ => {}
    }
}

fn remap_place_regions(place: &mut KirPlace, regions: &BTreeMap<MemoryRegionId, MemoryRegionId>) {
    match place {
        KirPlace::Value { region, .. }
        | KirPlace::Deref { region, .. }
        | KirPlace::SliceIndex { region, .. } => *region = regions[region],
        KirPlace::Index { base, region, .. } | KirPlace::Field { base, region, .. } => {
            *region = regions[region];
            remap_place_regions(base, regions);
        }
    }
}

fn remap_terminator(
    terminator: &mut KirTerminator,
    blocks: &BTreeMap<BlockId, BlockId>,
    regions: &BTreeMap<MemoryRegionId, MemoryRegionId>,
    memories: &BTreeMap<MemoryVersionId, MemoryVersionId>,
) {
    let remap_edge = |edge: &mut crate::KirEdge| {
        edge.target = blocks[&edge.target];
        for memory in &mut edge.memory_args {
            *memory = memories[memory];
        }
    };
    match terminator {
        KirTerminator::Return { memory, .. } => {
            for (region, version) in memory {
                *region = regions[region];
                *version = memories[version];
            }
        }
        KirTerminator::Jump { edge } => remap_edge(edge),
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            remap_edge(then_edge);
            remap_edge(else_edge);
        }
    }
}
