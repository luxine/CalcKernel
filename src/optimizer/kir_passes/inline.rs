use std::collections::BTreeMap;

use crate::{
    BlockId, ContractInstanceSource, InstructionId, KirBlock, KirBlockParam, KirEdge,
    KirEffectKind, KirFailureKind, KirInstruction, KirInstructionKind, KirMemoryBlockParam,
    KirModule, KirOrderedEffect, KirResult, KirTerminator, MemoryVersionId, ValueId,
};

use super::{
    super::{
        ContractFactSet, ContractInstanceId, KirGuardElimination,
        clone_contract_instance_for_inline,
    },
    rewrite::{remap_instruction_values, remap_terminator_values},
};

const INLINE_CALLEE_BUDGET: usize = 32;
const INLINE_MODULE_BUDGET: u32 = 128;

#[derive(Debug, Clone)]
struct InlineCandidate {
    caller_index: usize,
    block_index: usize,
    call_index: usize,
    callee: crate::KirFunction,
    source_contract: Option<ContractInstanceId>,
}

#[derive(Debug)]
struct IdAllocator {
    block: u32,
    value: u32,
    instruction: u32,
    memory: u32,
}

impl IdAllocator {
    fn for_module(module: &KirModule) -> Self {
        Self {
            block: next_index(
                module
                    .functions
                    .iter()
                    .flat_map(|function| function.blocks.iter().map(|block| block.id.index())),
            ),
            value: next_index(module.functions.iter().flat_map(|function| {
                function
                    .params
                    .iter()
                    .map(|param| param.value.index())
                    .chain(function.blocks.iter().flat_map(|block| {
                        block.params.iter().map(|param| param.value.index()).chain(
                            block.instructions.iter().flat_map(|instruction| {
                                instruction
                                    .results
                                    .iter()
                                    .map(|result| result.value.index())
                            }),
                        )
                    }))
            })),
            instruction: next_index(module.functions.iter().flat_map(|function| {
                function.blocks.iter().flat_map(|block| {
                    block
                        .instructions
                        .iter()
                        .map(|instruction| instruction.id.index())
                })
            })),
            memory: next_index(module.functions.iter().flat_map(|function| {
                function
                    .initial_memory
                    .iter()
                    .map(|memory| memory.version.index())
                    .chain(function.blocks.iter().flat_map(|block| {
                        block
                            .memory_params
                            .iter()
                            .map(|param| param.version.index())
                            .chain(block.instructions.iter().flat_map(|instruction| {
                                instruction.memory.iter().flat_map(|memory| {
                                    std::iter::once(memory.input.index())
                                        .chain(memory.output.map(MemoryVersionId::index))
                                })
                            }))
                    }))
            })),
        }
    }

    fn block(&mut self) -> BlockId {
        let id = BlockId::from_index(self.block);
        self.block = self.block.saturating_add(1);
        id
    }

    fn value(&mut self) -> ValueId {
        let id = ValueId::from_index(self.value);
        self.value = self.value.saturating_add(1);
        id
    }

    fn instruction(&mut self) -> InstructionId {
        let id = InstructionId::from_index(self.instruction);
        self.instruction = self.instruction.saturating_add(1);
        id
    }

    fn memory(&mut self) -> MemoryVersionId {
        let id = MemoryVersionId::from_index(self.memory);
        self.memory = self.memory.saturating_add(1);
        id
    }
}

pub(crate) fn run_effect_aware_inline(
    module: &mut KirModule,
    contracts: &mut Option<ContractFactSet>,
    eliminations: &[KirGuardElimination],
    pgo: Option<&crate::CkPgoOptimizerPlan>,
) -> u32 {
    let mut allocator = IdAllocator::for_module(module);
    let mut inlined = 0_u32;
    loop {
        let Some(candidate) =
            find_candidate(module, contracts.as_ref(), eliminations, inlined, pgo)
        else {
            break;
        };
        if !inline_candidate(
            module,
            contracts,
            eliminations,
            candidate,
            &mut allocator,
            inlined,
        ) {
            break;
        }
        inlined = inlined.saturating_add(1);
    }
    inlined
}

fn find_candidate(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    eliminations: &[KirGuardElimination],
    already_inlined: u32,
    pgo: Option<&crate::CkPgoOptimizerPlan>,
) -> Option<InlineCandidate> {
    if already_inlined >= INLINE_MODULE_BUDGET {
        return None;
    }
    for (caller_index, caller) in module.functions.iter().enumerate() {
        for (block_index, block) in caller.blocks.iter().enumerate() {
            for (call_index, instruction) in block.instructions.iter().enumerate() {
                let KirInstructionKind::Call { function_name, .. } = &instruction.kind else {
                    continue;
                };
                let Some(callee) = module
                    .functions
                    .iter()
                    .find(|function| function.name == *function_name)
                else {
                    continue;
                };
                if pgo.is_some_and(|profile| {
                    profile.block_is_profile_cold(module, caller.id, block.id)
                }) {
                    continue;
                }
                let callee_budget = if pgo.is_some_and(|profile| {
                    profile.function_is_hot(caller.id) || profile.function_is_hot(callee.id)
                }) {
                    48
                } else {
                    INLINE_CALLEE_BUDGET
                };
                if callee.exported
                    || callee.id == caller.id
                    || callee
                        .blocks
                        .iter()
                        .map(|block| block.instructions.len())
                        .sum::<usize>()
                        > callee_budget
                    || !callee_is_supported(callee)
                    || block.instructions[call_index + 1..].iter().any(|after| {
                        eliminations.iter().any(|elimination| {
                            elimination.function == caller.id
                                && elimination.condition_instruction == after.id
                        })
                    })
                {
                    continue;
                }
                let entry_contract = contracts.and_then(|contracts| {
                    contracts.instances().iter().find(|instance| {
                        instance.callee == callee.id
                            && matches!(instance.source, ContractInstanceSource::FunctionEntry)
                    })
                });
                let source_contract = if entry_contract.is_some() {
                    contracts.and_then(|contracts| {
                        contracts
                            .instances()
                            .iter()
                            .find(|instance| {
                                instance.callee == callee.id
                                    && matches!(
                                        instance.source,
                                        ContractInstanceSource::Call {
                                            caller: source_caller,
                                            block: source_block,
                                            instruction: source_instruction,
                                        } if source_caller == caller.id
                                            && source_block == block.id
                                            && source_instruction == instruction.id
                                    )
                            })
                            .map(|instance| instance.id)
                    })
                } else {
                    None
                };
                if entry_contract.is_some() && source_contract.is_none() {
                    continue;
                }
                return Some(InlineCandidate {
                    caller_index,
                    block_index,
                    call_index,
                    callee: callee.clone(),
                    source_contract,
                });
            }
        }
    }
    None
}

fn callee_is_supported(callee: &crate::KirFunction) -> bool {
    callee.blocks.iter().all(|block| {
        block.instructions.iter().all(|instruction| {
            instruction.memory.is_none()
                && matches!(
                    instruction.kind,
                    KirInstructionKind::Undef { .. }
                        | KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::ConstFloat { .. }
                        | KirInstructionKind::ConstBool { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::Binary { .. }
                        | KirInstructionKind::Unary { .. }
                        | KirInstructionKind::Compare { .. }
                        | KirInstructionKind::Cast { .. }
                        | KirInstructionKind::CheckCondition { .. }
                        | KirInstructionKind::Guard { .. }
                        | KirInstructionKind::RuntimeCall { .. }
                )
        })
    })
}

fn inline_candidate(
    module: &mut KirModule,
    contracts: &mut Option<ContractFactSet>,
    eliminations: &[KirGuardElimination],
    candidate: InlineCandidate,
    allocator: &mut IdAllocator,
    clone_number: u32,
) -> bool {
    let call = module.functions[candidate.caller_index].blocks[candidate.block_index].instructions
        [candidate.call_index]
        .clone();
    let KirInstructionKind::Call { args, .. } = &call.kind else {
        return false;
    };
    let Some(call_memory) = call.memory.as_ref() else {
        return false;
    };
    let Some(call_memory_output) = call_memory.output else {
        return false;
    };
    if args.len() != candidate.callee.params.len() || call.results.len() > 1 {
        return false;
    }

    let mut block_map = BTreeMap::new();
    let mut value_map = candidate
        .callee
        .params
        .iter()
        .zip(args)
        .map(|(param, argument)| (param.value, *argument))
        .collect::<BTreeMap<_, _>>();
    let mut instruction_map = BTreeMap::new();
    for block in &candidate.callee.blocks {
        block_map.insert(block.id, allocator.block());
        for param in &block.params {
            value_map.insert(param.value, allocator.value());
        }
        for instruction in &block.instructions {
            instruction_map.insert(instruction.id, allocator.instruction());
            for result in &instruction.results {
                value_map.insert(result.value, allocator.value());
            }
        }
    }
    let continuation_id = allocator.block();
    let continuation_value = call.results.first().map(|_| allocator.value());
    let clone_memories = candidate
        .callee
        .blocks
        .iter()
        .map(|block| (block.id, allocator.memory()))
        .collect::<BTreeMap<_, _>>();
    let continuation_memory = allocator.memory();
    let clone_block_ids = block_map.values().copied().collect::<Vec<_>>();

    if let (Some(source), Some(imported)) = (candidate.source_contract, contracts.as_ref()) {
        let mut proposed = imported.clone();
        if clone_contract_instance_for_inline(
            &mut proposed,
            source,
            module.functions[candidate.caller_index].id,
            clone_number,
            clone_block_ids.clone(),
            &value_map,
        )
        .is_err()
        {
            return false;
        }
        *contracts = Some(proposed);
    }

    let caller_id = module.functions[candidate.caller_index].id;
    let original = module.functions[candidate.caller_index]
        .blocks
        .remove(candidate.block_index);
    let before = KirBlock {
        id: original.id,
        label: original.label,
        params: original.params,
        memory_params: original.memory_params,
        instructions: original.instructions[..candidate.call_index].to_vec(),
        terminator: KirTerminator::Jump {
            edge: KirEdge {
                target: block_map[&candidate.callee.blocks[0].id],
                args: Vec::new(),
                memory_args: vec![call_memory.input],
            },
        },
    };
    let mut after_values = BTreeMap::new();
    if let (Some(result), Some(continuation)) = (call.results.first(), continuation_value) {
        after_values.insert(result.value, continuation);
    }
    let mut after_instructions = original.instructions[candidate.call_index + 1..].to_vec();
    for instruction in &mut after_instructions {
        remap_instruction_values(instruction, &after_values);
        remap_memory_instruction(instruction, call_memory_output, continuation_memory);
    }
    let mut after_terminator = original.terminator;
    remap_terminator_values(&mut after_terminator, &after_values);
    remap_memory_terminator(
        &mut after_terminator,
        call_memory_output,
        continuation_memory,
    );

    let mut cloned_blocks = Vec::new();
    for source in &candidate.callee.blocks {
        let current_memory = clone_memories[&source.id];
        let mut instructions = Vec::new();
        for instruction in &source.instructions {
            let mut cloned = instruction.clone();
            cloned.id = instruction_map[&instruction.id];
            cloned.results = instruction
                .results
                .iter()
                .map(|result| KirResult {
                    value: value_map[&result.value],
                    type_node: result.type_node.clone(),
                })
                .collect();
            remap_instruction_values(&mut cloned, &value_map);
            instructions.push(cloned.clone());
            if eliminations.iter().any(|elimination| {
                elimination.function == candidate.callee.id
                    && elimination.condition_instruction == instruction.id
            }) && let Some((condition, failure)) = restored_guard(&cloned)
            {
                instructions.push(KirInstruction {
                    id: allocator.instruction(),
                    results: Vec::new(),
                    kind: KirInstructionKind::Guard { condition, failure },
                    memory: None,
                    effect: Some(KirOrderedEffect {
                        order: u32::MAX,
                        kind: KirEffectKind::MayFail,
                    }),
                });
            }
        }
        let mut terminator = source.terminator.clone();
        remap_terminator_values(&mut terminator, &value_map);
        terminator = match terminator {
            KirTerminator::Return { value, .. } => KirTerminator::Jump {
                edge: KirEdge {
                    target: continuation_id,
                    args: value.into_iter().collect(),
                    memory_args: vec![current_memory],
                },
            },
            KirTerminator::Jump { mut edge } => {
                edge.target = block_map[&edge.target];
                edge.memory_args = vec![current_memory];
                KirTerminator::Jump { edge }
            }
            KirTerminator::Branch {
                condition,
                mut then_edge,
                mut else_edge,
            } => {
                then_edge.target = block_map[&then_edge.target];
                else_edge.target = block_map[&else_edge.target];
                then_edge.memory_args = vec![current_memory];
                else_edge.memory_args = vec![current_memory];
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                }
            }
        };
        cloned_blocks.push(KirBlock {
            id: block_map[&source.id],
            label: format!("inline.{}.{}", candidate.callee.name, source.label),
            params: source
                .params
                .iter()
                .map(|param| KirBlockParam {
                    value: value_map[&param.value],
                    slot: param.slot.clone(),
                    type_node: param.type_node.clone(),
                })
                .collect(),
            memory_params: vec![KirMemoryBlockParam {
                version: current_memory,
                region: call_memory.region,
            }],
            instructions,
            terminator,
        });
    }
    let continuation = KirBlock {
        id: continuation_id,
        label: format!("inline.cont.{}", call.id.index()),
        params: call
            .results
            .first()
            .zip(continuation_value)
            .map(|(result, value)| KirBlockParam {
                value,
                slot: "inline.result".to_string(),
                type_node: result.type_node.clone(),
            })
            .into_iter()
            .collect(),
        memory_params: vec![KirMemoryBlockParam {
            version: continuation_memory,
            region: call_memory.region,
        }],
        instructions: after_instructions,
        terminator: after_terminator,
    };

    let caller = &mut module.functions[candidate.caller_index];
    caller.blocks.insert(candidate.block_index, before);
    for (offset, block) in cloned_blocks.into_iter().enumerate() {
        caller
            .blocks
            .insert(candidate.block_index + 1 + offset, block);
    }
    caller.blocks.insert(
        candidate.block_index + 1 + candidate.callee.blocks.len(),
        continuation,
    );
    debug_assert_eq!(caller.id, caller_id);
    true
}

fn restored_guard(instruction: &KirInstruction) -> Option<(ValueId, KirFailureKind)> {
    match &instruction.kind {
        KirInstructionKind::Binary { .. } | KirInstructionKind::Unary { .. } => instruction
            .results
            .get(1)
            .map(|result| (result.value, KirFailureKind::Overflow)),
        KirInstructionKind::CheckCondition { kind, .. } => {
            let failure = match kind {
                crate::KirCheckConditionKind::ArithmeticOverflow
                | crate::KirCheckConditionKind::SignedDivisionOverflow => KirFailureKind::Overflow,
                crate::KirCheckConditionKind::DivisionByZero => KirFailureKind::DivisionByZero,
                crate::KirCheckConditionKind::SliceOutOfBounds
                | crate::KirCheckConditionKind::InvalidSubslice => KirFailureKind::OutOfBounds,
            };
            instruction
                .results
                .first()
                .map(|result| (result.value, failure))
        }
        _ => None,
    }
}

fn remap_memory_instruction(
    instruction: &mut KirInstruction,
    old: MemoryVersionId,
    new: MemoryVersionId,
) {
    if let Some(memory) = &mut instruction.memory {
        if memory.input == old {
            memory.input = new;
        }
        if memory.output == Some(old) {
            memory.output = Some(new);
        }
    }
}

fn remap_memory_terminator(
    terminator: &mut KirTerminator,
    old: MemoryVersionId,
    new: MemoryVersionId,
) {
    let remap = |version: &mut MemoryVersionId| {
        if *version == old {
            *version = new;
        }
    };
    match terminator {
        KirTerminator::Return { memory, .. } => {
            for (_, version) in memory {
                remap(version);
            }
        }
        KirTerminator::Jump { edge } => {
            for version in &mut edge.memory_args {
                remap(version);
            }
        }
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            for version in then_edge
                .memory_args
                .iter_mut()
                .chain(&mut else_edge.memory_args)
            {
                remap(version);
            }
        }
    }
}

fn next_index(values: impl Iterator<Item = u32>) -> u32 {
    values.max().map_or(0, |value| value.saturating_add(1))
}
