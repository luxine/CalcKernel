use std::collections::HashMap;

use crate::{
    MirArtifactConsumer, MirBinaryOp, MirFunction, MirInstruction, MirModule, MirPlace,
    MirPrimitiveTypeName, MirTerminator, MirType, MirUnaryOp, MirValue,
    prepare_artifact_for_consumer, print_mir_type,
};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MirValueName {
    Param(String),
    Local(String),
    Temp(String),
    ConstInt(String, MirType),
    ConstFloat(String, MirType),
    ConstBool(bool, MirType),
}

#[derive(Default)]
struct KirIds {
    function: u32,
    block: u32,
    value: u32,
    instruction: u32,
    region: u32,
    memory: u32,
}

impl KirIds {
    fn function(&mut self) -> FunctionId {
        let id = FunctionId::from_index(self.function);
        self.function += 1;
        id
    }

    fn block(&mut self) -> BlockId {
        let id = BlockId::from_index(self.block);
        self.block += 1;
        id
    }

    fn value(&mut self) -> ValueId {
        let id = ValueId::from_index(self.value);
        self.value += 1;
        id
    }

    fn instruction(&mut self) -> InstructionId {
        let id = InstructionId::from_index(self.instruction);
        self.instruction += 1;
        id
    }

    fn region(&mut self) -> MemoryRegionId {
        let id = MemoryRegionId::from_index(self.region);
        self.region += 1;
        id
    }

    fn memory(&mut self) -> MemoryVersionId {
        let id = MemoryVersionId::from_index(self.memory);
        self.memory += 1;
        id
    }
}

#[must_use = "KIR construction errors must be handled"]
pub fn build_kir_module(
    module: &MirModule,
    config: KirBuildConfig,
) -> Result<KirModule, KirBuildError> {
    build_kir_module_with_profile(
        module,
        config,
        KirTargetProfile::for_consumer(config.consumer),
    )
}

#[must_use = "KIR construction errors must be handled"]
pub fn build_kir_module_with_profile(
    module: &MirModule,
    config: KirBuildConfig,
    profile: KirTargetProfile,
) -> Result<KirModule, KirBuildError> {
    validate_build_config(config)?;
    profile.validate().map_err(KirBuildError::new)?;
    if profile.consumer() != config.consumer {
        return Err(KirBuildError::new(
            "KIR target profile consumer does not match module consumer",
        ));
    }
    let artifact = prepare_artifact_for_consumer(module, mir_consumer(config.consumer))
        .map_err(|error| KirBuildError::new(error.message))?;
    let mut ids = KirIds::default();
    let functions = artifact
        .functions
        .iter()
        .map(|function| build_function(function, config, &mut ids))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(KirModule {
        config,
        profile,
        entry: artifact.entry,
        structs: artifact.structs,
        functions,
        tune_layout: None,
    })
}

fn validate_build_config(config: KirBuildConfig) -> Result<(), KirBuildError> {
    if config.consumer == KirConsumer::WebAssembly
        && config.overflow_mode == KirOverflowMode::Checked
    {
        return Err(KirBuildError::new(
            "WebAssembly KIR consumer does not support checked overflow mode.",
        ));
    }
    if config.consumer == KirConsumer::WebAssembly && config.bounds_mode == KirBoundsMode::Checked {
        return Err(KirBuildError::new(
            "WebAssembly KIR consumer does not support checked bounds mode.",
        ));
    }
    if config.sanitizer_mode == KirSanitizerMode::Contracts
        && config.consumer != KirConsumer::NativeExecutable
    {
        return Err(KirBuildError::new(
            "Contract sanitizer KIR is supported only for native executable consumers.",
        ));
    }
    Ok(())
}

const fn mir_consumer(consumer: KirConsumer) -> MirArtifactConsumer {
    match consumer {
        KirConsumer::C => MirArtifactConsumer::C,
        KirConsumer::WebAssembly => MirArtifactConsumer::WebAssembly,
        KirConsumer::NativeLibrary => MirArtifactConsumer::NativeLibrary,
        KirConsumer::NativeExecutable => MirArtifactConsumer::NativeExecutable,
        KirConsumer::Inspection => MirArtifactConsumer::Inspection,
    }
}

fn build_function(
    function: &MirFunction,
    config: KirBuildConfig,
    ids: &mut KirIds,
) -> Result<KirFunction, KirBuildError> {
    if function.blocks.is_empty() {
        return Err(KirBuildError::new(format!(
            "MIR function '{}' has no entry block",
            function.name
        )));
    }
    let function_id = ids.function();
    let conservative_region = ids.region();
    let initial_memory_version = ids.memory();
    let mut regions = vec![KirMemoryRegion {
        id: conservative_region,
        origin: KirMemoryRegionOrigin::Conservative,
        parent: None,
        partition: conservative_region,
        byte_interval: None,
    }];
    let initial_memory = vec![KirInitialMemory {
        region: conservative_region,
        version: initial_memory_version,
    }];
    let mut values = HashMap::new();
    let mut value_regions = HashMap::new();
    let mut params = Vec::new();
    for param in &function.params {
        let value = ids.value();
        let name = MirValueName::Param(param.name.clone());
        values.insert(name.clone(), value);
        if is_memory_origin_type(&param.type_node) {
            let region = ids.region();
            regions.push(KirMemoryRegion {
                id: region,
                origin: KirMemoryRegionOrigin::Parameter(value),
                parent: None,
                partition: conservative_region,
                byte_interval: None,
            });
            value_regions.insert(name, region);
        }
        params.push(KirParam {
            value,
            name: param.name.clone(),
            type_node: param.type_node.clone(),
        });
    }
    let slots = function
        .params
        .iter()
        .map(|param| {
            (
                MirValueName::Param(param.name.clone()),
                param.name.clone(),
                param.type_node.clone(),
            )
        })
        .chain(function.locals.iter().map(|local| {
            (
                MirValueName::Local(local.name.clone()),
                local.name.clone(),
                local.type_node.clone(),
            )
        }))
        .collect::<Vec<_>>();
    let mut entry_prefix = Vec::new();
    for local in &function.locals {
        let value = ids.value();
        values.insert(MirValueName::Local(local.name.clone()), value);
        entry_prefix.push(KirInstruction {
            id: ids.instruction(),
            results: vec![KirResult {
                value,
                type_node: local.type_node.clone().into(),
            }],
            kind: KirInstructionKind::Undef {
                slot: local.name.clone(),
            },
            memory: None,
            effect: None,
        });
    }
    for constant in inline_constants(function) {
        let value = ids.value();
        values.insert(value_name(&constant), value);
        let kind = match &constant {
            MirValue::ConstInt { text, .. } => KirInstructionKind::ConstInt {
                value: text.clone(),
            },
            MirValue::ConstFloat { text, .. } => KirInstructionKind::ConstFloat {
                value: text.clone(),
            },
            MirValue::ConstBool { value, .. } => KirInstructionKind::ConstBool { value: *value },
            MirValue::Param { .. } | MirValue::Local { .. } | MirValue::Temp { .. } => {
                return Err(KirBuildError::new(
                    "inline constant collector returned a named MIR value",
                ));
            }
        };
        entry_prefix.push(KirInstruction {
            id: ids.instruction(),
            results: vec![KirResult {
                value,
                type_node: mir_value_type(&constant).clone().into(),
            }],
            kind,
            memory: None,
            effect: None,
        });
    }
    let block_ids = function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), ids.block()))
        .collect::<HashMap<_, _>>();
    let block_params = function
        .blocks
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index == 0 {
                return Vec::new();
            }
            slots
                .iter()
                .map(|(_, slot, type_node)| KirBlockParam {
                    value: ids.value(),
                    slot: slot.clone(),
                    type_node: type_node.clone().into(),
                })
                .collect()
        })
        .collect::<Vec<Vec<KirBlockParam>>>();
    let memory_params = function
        .blocks
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index == 0 {
                Vec::new()
            } else {
                vec![KirMemoryBlockParam {
                    version: ids.memory(),
                    region: conservative_region,
                }]
            }
        })
        .collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut current_memory = initial_memory_version;
    let mut effect_order = 0_u32;
    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            for ((key, _, _), param) in slots.iter().zip(&block_params[index]) {
                values.insert(key.clone(), param.value);
            }
            current_memory = memory_params[index][0].version;
        }
        let mut instructions = if index == 0 {
            std::mem::take(&mut entry_prefix)
        } else {
            Vec::new()
        };
        {
            let mut builder = InstructionBuilder {
                config,
                ids,
                values: &mut values,
                value_regions: &mut value_regions,
                regions: &mut regions,
                conservative_region,
                current_memory: &mut current_memory,
                effect_order: &mut effect_order,
                instructions: &mut instructions,
            };
            for instruction in &block.instructions {
                builder.build(instruction)?;
            }
        }
        let terminator = build_terminator(
            &block.terminator,
            &values,
            &slots,
            &block_ids,
            conservative_region,
            current_memory,
            &mut effect_order,
        )?;
        let block_id = block_ids.get(&block.label).copied().ok_or_else(|| {
            KirBuildError::new(format!("missing KIR block id for '{}'", block.label))
        })?;
        blocks.push(KirBlock {
            id: block_id,
            label: block.label.clone(),
            params: block_params[index].clone(),
            memory_params: memory_params[index].clone(),
            instructions,
            terminator,
        });
    }
    Ok(KirFunction {
        id: function_id,
        name: function.name.clone(),
        exported: function.exported,
        params,
        return_type: function.return_type.clone(),
        regions,
        initial_memory,
        vector_regions: Vec::new(),
        blocks,
    })
}

struct InstructionBuilder<'build> {
    config: KirBuildConfig,
    ids: &'build mut KirIds,
    values: &'build mut HashMap<MirValueName, ValueId>,
    value_regions: &'build mut HashMap<MirValueName, MemoryRegionId>,
    regions: &'build mut Vec<KirMemoryRegion>,
    conservative_region: MemoryRegionId,
    current_memory: &'build mut MemoryVersionId,
    effect_order: &'build mut u32,
    instructions: &'build mut Vec<KirInstruction>,
}

impl InstructionBuilder<'_> {
    fn build(&mut self, instruction: &MirInstruction) -> Result<(), KirBuildError> {
        let config = self.config;
        let ids = &mut *self.ids;
        let values = &mut *self.values;
        let value_regions = &mut *self.value_regions;
        let regions = &mut *self.regions;
        let conservative_region = self.conservative_region;
        let current_memory = &mut *self.current_memory;
        let effect_order = &mut *self.effect_order;
        let instructions = &mut *self.instructions;
        match instruction {
            MirInstruction::ConstInt { target, value } => {
                define_instruction_result(
                    target,
                    KirInstructionKind::ConstInt {
                        value: value.clone(),
                    },
                    ids,
                    values,
                    instructions,
                );
                Ok(())
            }
            MirInstruction::ConstFloat { target, value } => {
                define_instruction_result(
                    target,
                    KirInstructionKind::ConstFloat {
                        value: value.clone(),
                    },
                    ids,
                    values,
                    instructions,
                );
                Ok(())
            }
            MirInstruction::ConstBool { target, value } => {
                define_instruction_result(
                    target,
                    KirInstructionKind::ConstBool { value: *value },
                    ids,
                    values,
                    instructions,
                );
                Ok(())
            }
            MirInstruction::Move { target, value } => {
                let source = lookup_value(value, values)?;
                let source_region = value_regions.get(&value_name(value)).copied();
                if matches!(target, MirValue::Param { .. } | MirValue::Local { .. }) {
                    let target_name = value_name(target);
                    values.insert(target_name.clone(), source);
                    update_value_region(&target_name, source_region, value_regions);
                } else {
                    let result = define_instruction_result(
                        target,
                        KirInstructionKind::Copy { value: source },
                        ids,
                        values,
                        instructions,
                    );
                    update_value_region(&value_name(target), source_region, value_regions);
                    let _ = result;
                }
                Ok(())
            }
            MirInstruction::Unary {
                target,
                op,
                operand,
            } => {
                let operand = lookup_value(operand, values)?;
                let semantics = arithmetic_semantics(mir_value_type(target), config);
                if semantics == KirArithmeticSemantics::Checked
                    && *op == MirUnaryOp::Neg
                    && is_integer_type(mir_value_type(target))
                {
                    let result = ids.value();
                    let overflow = ids.value();
                    values.insert(value_name(target), result);
                    instructions.push(KirInstruction {
                        id: ids.instruction(),
                        results: vec![
                            KirResult {
                                value: result,
                                type_node: mir_value_type(target).clone().into(),
                            },
                            KirResult {
                                value: overflow,
                                type_node: bool_type().into(),
                            },
                        ],
                        kind: KirInstructionKind::Unary {
                            op: *op,
                            operand,
                            semantics,
                        },
                        memory: None,
                        effect: None,
                    });
                    push_guard(
                        overflow,
                        KirFailureKind::Overflow,
                        ids,
                        effect_order,
                        instructions,
                    );
                } else {
                    define_instruction_result(
                        target,
                        KirInstructionKind::Unary {
                            op: *op,
                            operand,
                            semantics,
                        },
                        ids,
                        values,
                        instructions,
                    );
                }
                Ok(())
            }
            MirInstruction::Compare {
                target,
                op,
                left,
                right,
            } => {
                let left = lookup_value(left, values)?;
                let right = lookup_value(right, values)?;
                define_instruction_result(
                    target,
                    KirInstructionKind::Compare {
                        op: *op,
                        left,
                        right,
                    },
                    ids,
                    values,
                    instructions,
                );
                Ok(())
            }
            MirInstruction::Cast { target, op, value } => {
                let value = lookup_value(value, values)?;
                define_instruction_result(
                    target,
                    KirInstructionKind::Cast { op: *op, value },
                    ids,
                    values,
                    instructions,
                );
                Ok(())
            }
            MirInstruction::Binary {
                target,
                op,
                left,
                right,
            } => {
                let left = lookup_value(left, values)?;
                let right = lookup_value(right, values)?;
                let semantics = arithmetic_semantics(mir_value_type(target), config);
                if semantics == KirArithmeticSemantics::Checked
                    && is_integer_type(mir_value_type(target))
                {
                    match op {
                        MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul => {
                            let result = ids.value();
                            let overflow = ids.value();
                            values.insert(value_name(target), result);
                            instructions.push(KirInstruction {
                                id: ids.instruction(),
                                results: vec![
                                    KirResult {
                                        value: result,
                                        type_node: mir_value_type(target).clone().into(),
                                    },
                                    KirResult {
                                        value: overflow,
                                        type_node: bool_type().into(),
                                    },
                                ],
                                kind: KirInstructionKind::Binary {
                                    op: *op,
                                    left,
                                    right,
                                    semantics,
                                },
                                memory: None,
                                effect: None,
                            });
                            push_guard(
                                overflow,
                                KirFailureKind::Overflow,
                                ids,
                                effect_order,
                                instructions,
                            );
                        }
                        MirBinaryOp::Div | MirBinaryOp::Mod => {
                            let by_zero = push_check_condition(
                                KirCheckConditionKind::DivisionByZero,
                                vec![right],
                                ids,
                                instructions,
                            );
                            push_guard(
                                by_zero,
                                KirFailureKind::DivisionByZero,
                                ids,
                                effect_order,
                                instructions,
                            );
                            if is_signed_integer_type(mir_value_type(target)) {
                                let overflow = push_check_condition(
                                    KirCheckConditionKind::SignedDivisionOverflow,
                                    vec![left, right],
                                    ids,
                                    instructions,
                                );
                                push_guard(
                                    overflow,
                                    KirFailureKind::Overflow,
                                    ids,
                                    effect_order,
                                    instructions,
                                );
                            }
                            define_instruction_result(
                                target,
                                KirInstructionKind::Binary {
                                    op: *op,
                                    left,
                                    right,
                                    semantics,
                                },
                                ids,
                                values,
                                instructions,
                            );
                        }
                    }
                } else {
                    define_instruction_result(
                        target,
                        KirInstructionKind::Binary {
                            op: *op,
                            left,
                            right,
                            semantics,
                        },
                        ids,
                        values,
                        instructions,
                    );
                }
                Ok(())
            }
            MirInstruction::Address { target, place } => {
                let place = build_place(
                    place,
                    config,
                    ids,
                    values,
                    value_regions,
                    conservative_region,
                    effect_order,
                    instructions,
                )?;
                let region = place_region(&place);
                define_instruction_result(
                    target,
                    KirInstructionKind::Address {
                        place: Box::new(place),
                    },
                    ids,
                    values,
                    instructions,
                );
                value_regions.insert(value_name(target), region);
                Ok(())
            }
            MirInstruction::Load { target, place } => {
                let place = build_place(
                    place,
                    config,
                    ids,
                    values,
                    value_regions,
                    conservative_region,
                    effect_order,
                    instructions,
                )?;
                let memory = KirMemoryAccess {
                    region: conservative_region,
                    input: *current_memory,
                    output: None,
                };
                let result = ids.value();
                values.insert(value_name(target), result);
                instructions.push(KirInstruction {
                    id: ids.instruction(),
                    results: vec![KirResult {
                        value: result,
                        type_node: mir_value_type(target).clone().into(),
                    }],
                    kind: KirInstructionKind::Load {
                        place: Box::new(place),
                    },
                    memory: Some(memory),
                    effect: Some(next_effect(KirEffectKind::ReadMemory, effect_order)),
                });
                Ok(())
            }
            MirInstruction::Store { place, value } => {
                let place = build_place(
                    place,
                    config,
                    ids,
                    values,
                    value_regions,
                    conservative_region,
                    effect_order,
                    instructions,
                )?;
                let value = lookup_value(value, values)?;
                let output = ids.memory();
                instructions.push(KirInstruction {
                    id: ids.instruction(),
                    results: Vec::new(),
                    kind: KirInstructionKind::Store {
                        place: Box::new(place),
                        value,
                    },
                    memory: Some(KirMemoryAccess {
                        region: conservative_region,
                        input: *current_memory,
                        output: Some(output),
                    }),
                    effect: Some(next_effect(KirEffectKind::WriteMemory, effect_order)),
                });
                *current_memory = output;
                Ok(())
            }
            MirInstruction::MakeSlice { target, data, len } => {
                let data_value = lookup_value(data, values)?;
                let len = lookup_value(len, values)?;
                let result = define_instruction_result(
                    target,
                    KirInstructionKind::MakeSlice {
                        data: data_value,
                        len,
                    },
                    ids,
                    values,
                    instructions,
                );
                let parent = value_regions.get(&value_name(data)).copied();
                let region = ids.region();
                regions.push(KirMemoryRegion {
                    id: region,
                    origin: KirMemoryRegionOrigin::RawSlice(result),
                    parent,
                    partition: conservative_region,
                    byte_interval: None,
                });
                value_regions.insert(value_name(target), region);
                Ok(())
            }
            MirInstruction::SliceData { target, slice } => {
                let slice_value = lookup_value(slice, values)?;
                define_instruction_result(
                    target,
                    KirInstructionKind::SliceData { slice: slice_value },
                    ids,
                    values,
                    instructions,
                );
                update_value_region(
                    &value_name(target),
                    value_regions.get(&value_name(slice)).copied(),
                    value_regions,
                );
                Ok(())
            }
            MirInstruction::SliceLen { target, slice } => {
                let slice = lookup_value(slice, values)?;
                define_instruction_result(
                    target,
                    KirInstructionKind::SliceLen { slice },
                    ids,
                    values,
                    instructions,
                );
                Ok(())
            }
            MirInstruction::Subslice {
                target,
                slice,
                start,
                end,
            } => {
                let slice_value = lookup_value(slice, values)?;
                let start_value = lookup_value(start, values)?;
                let end_value = lookup_value(end, values)?;
                if config.bounds_mode == KirBoundsMode::Checked {
                    let invalid = push_check_condition(
                        KirCheckConditionKind::InvalidSubslice,
                        vec![slice_value, start_value, end_value],
                        ids,
                        instructions,
                    );
                    push_guard(
                        invalid,
                        KirFailureKind::OutOfBounds,
                        ids,
                        effect_order,
                        instructions,
                    );
                }
                let result = define_instruction_result(
                    target,
                    KirInstructionKind::Subslice {
                        slice: slice_value,
                        start: start_value,
                        end: end_value,
                    },
                    ids,
                    values,
                    instructions,
                );
                let parent = value_regions.get(&value_name(slice)).copied();
                let region = ids.region();
                regions.push(KirMemoryRegion {
                    id: region,
                    origin: KirMemoryRegionOrigin::Subslice(result),
                    parent,
                    partition: conservative_region,
                    byte_interval: Some(KirSymbolicByteInterval {
                        start: start_value,
                        end: end_value,
                        element_type: match mir_value_type(target) {
                            MirType::Slice(element_type) => (**element_type).clone(),
                            _ => {
                                return Err(KirBuildError::new(
                                    "MIR subslice result does not have slice type",
                                ));
                            }
                        },
                    }),
                });
                value_regions.insert(value_name(target), region);
                Ok(())
            }
            MirInstruction::Call {
                target,
                function_name,
                args,
            } => {
                let args = args
                    .iter()
                    .map(|arg| lookup_value(arg, values))
                    .collect::<Result<Vec<_>, _>>()?;
                let results = target.as_ref().map_or_else(Vec::new, |target| {
                    let value = ids.value();
                    values.insert(value_name(target), value);
                    vec![KirResult {
                        value,
                        type_node: mir_value_type(target).clone().into(),
                    }]
                });
                let output = ids.memory();
                instructions.push(KirInstruction {
                    id: ids.instruction(),
                    results,
                    kind: KirInstructionKind::Call {
                        function_name: function_name.clone(),
                        args,
                    },
                    memory: Some(KirMemoryAccess {
                        region: conservative_region,
                        input: *current_memory,
                        output: Some(output),
                    }),
                    effect: Some(next_effect(KirEffectKind::Call, effect_order)),
                });
                *current_memory = output;
                Ok(())
            }
            MirInstruction::RuntimeCall { intrinsic, args } => {
                let args = args
                    .iter()
                    .map(|arg| lookup_value(arg, values))
                    .collect::<Result<Vec<_>, _>>()?;
                instructions.push(KirInstruction {
                    id: ids.instruction(),
                    results: Vec::new(),
                    kind: KirInstructionKind::RuntimeCall {
                        intrinsic: *intrinsic,
                        args,
                    },
                    memory: None,
                    effect: Some(next_effect(KirEffectKind::Runtime, effect_order)),
                });
                Ok(())
            }
        }
    }
}

fn define_instruction_result(
    target: &MirValue,
    kind: KirInstructionKind,
    ids: &mut KirIds,
    values: &mut HashMap<MirValueName, ValueId>,
    instructions: &mut Vec<KirInstruction>,
) -> ValueId {
    let value = ids.value();
    values.insert(value_name(target), value);
    instructions.push(KirInstruction {
        id: ids.instruction(),
        results: vec![KirResult {
            value,
            type_node: mir_value_type(target).clone().into(),
        }],
        kind,
        memory: None,
        effect: None,
    });
    value
}

fn build_terminator(
    terminator: &MirTerminator,
    values: &HashMap<MirValueName, ValueId>,
    slots: &[(MirValueName, String, MirType)],
    block_ids: &HashMap<String, BlockId>,
    conservative_region: MemoryRegionId,
    current_memory: MemoryVersionId,
    effect_order: &mut u32,
) -> Result<KirTerminator, KirBuildError> {
    match terminator {
        MirTerminator::Return { value } => {
            let order = *effect_order;
            *effect_order += 1;
            Ok(KirTerminator::Return {
                value: value
                    .as_ref()
                    .map(|value| lookup_value(value, values))
                    .transpose()?,
                memory: vec![(conservative_region, current_memory)],
                effect_order: order,
            })
        }
        MirTerminator::Jump { label } => Ok(KirTerminator::Jump {
            edge: build_edge(label, values, slots, block_ids, current_memory)?,
        }),
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => Ok(KirTerminator::Branch {
            condition: lookup_value(condition, values)?,
            then_edge: build_edge(then_label, values, slots, block_ids, current_memory)?,
            else_edge: build_edge(else_label, values, slots, block_ids, current_memory)?,
        }),
    }
}

fn build_edge(
    label: &str,
    values: &HashMap<MirValueName, ValueId>,
    slots: &[(MirValueName, String, MirType)],
    block_ids: &HashMap<String, BlockId>,
    current_memory: MemoryVersionId,
) -> Result<KirEdge, KirBuildError> {
    let target = block_ids
        .get(label)
        .copied()
        .ok_or_else(|| KirBuildError::new(format!("MIR edge names missing block '{label}'")))?;
    let args = slots
        .iter()
        .map(|(key, name, _)| {
            values.get(key).copied().ok_or_else(|| {
                KirBuildError::new(format!(
                    "MIR slot '{name}' has no value on edge to '{label}'"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(KirEdge {
        target,
        args,
        memory_args: vec![current_memory],
    })
}

fn arithmetic_semantics(type_node: &MirType, config: KirBuildConfig) -> KirArithmeticSemantics {
    if matches!(
        type_node,
        MirType::Primitive(crate::MirPrimitiveTypeName::F64)
    ) {
        KirArithmeticSemantics::StrictFloat
    } else if config.overflow_mode == KirOverflowMode::Checked {
        KirArithmeticSemantics::Checked
    } else {
        KirArithmeticSemantics::Modular
    }
}

fn push_check_condition(
    kind: KirCheckConditionKind,
    args: Vec<ValueId>,
    ids: &mut KirIds,
    instructions: &mut Vec<KirInstruction>,
) -> ValueId {
    let condition = ids.value();
    instructions.push(KirInstruction {
        id: ids.instruction(),
        results: vec![KirResult {
            value: condition,
            type_node: bool_type().into(),
        }],
        kind: KirInstructionKind::CheckCondition { kind, args },
        memory: None,
        effect: None,
    });
    condition
}

fn push_guard(
    condition: ValueId,
    failure: KirFailureKind,
    ids: &mut KirIds,
    effect_order: &mut u32,
    instructions: &mut Vec<KirInstruction>,
) {
    instructions.push(KirInstruction {
        id: ids.instruction(),
        results: Vec::new(),
        kind: KirInstructionKind::Guard { condition, failure },
        memory: None,
        effect: Some(next_effect(KirEffectKind::MayFail, effect_order)),
    });
}

fn next_effect(kind: KirEffectKind, effect_order: &mut u32) -> KirOrderedEffect {
    let order = *effect_order;
    *effect_order += 1;
    KirOrderedEffect { order, kind }
}

#[allow(clippy::too_many_arguments)]
fn build_place(
    place: &MirPlace,
    config: KirBuildConfig,
    ids: &mut KirIds,
    values: &HashMap<MirValueName, ValueId>,
    value_regions: &HashMap<MirValueName, MemoryRegionId>,
    conservative_region: MemoryRegionId,
    effect_order: &mut u32,
    instructions: &mut Vec<KirInstruction>,
) -> Result<KirPlace, KirBuildError> {
    match place {
        MirPlace::Param { name, type_node } => {
            let key = MirValueName::Param(name.clone());
            Ok(KirPlace::Value {
                value: values.get(&key).copied().ok_or_else(|| {
                    KirBuildError::new(format!("MIR parameter place '{name}' has no SSA value"))
                })?,
                type_node: type_node.clone(),
                region: value_regions
                    .get(&key)
                    .copied()
                    .unwrap_or(conservative_region),
            })
        }
        MirPlace::Local { name, type_node } => {
            let key = MirValueName::Local(name.clone());
            Ok(KirPlace::Value {
                value: values.get(&key).copied().ok_or_else(|| {
                    KirBuildError::new(format!("MIR local place '{name}' has no SSA value"))
                })?,
                type_node: type_node.clone(),
                region: value_regions
                    .get(&key)
                    .copied()
                    .unwrap_or(conservative_region),
            })
        }
        MirPlace::Deref { pointer, type_node } => Ok(KirPlace::Deref {
            pointer: lookup_value(pointer, values)?,
            type_node: type_node.clone(),
            region: value_regions
                .get(&value_name(pointer))
                .copied()
                .unwrap_or(conservative_region),
        }),
        MirPlace::Index {
            base,
            index,
            type_node,
        } => {
            let base = build_place(
                base,
                config,
                ids,
                values,
                value_regions,
                conservative_region,
                effect_order,
                instructions,
            )?;
            let region = place_region(&base);
            Ok(KirPlace::Index {
                base: Box::new(base),
                index: lookup_value(index, values)?,
                type_node: type_node.clone(),
                region,
            })
        }
        MirPlace::SliceIndex {
            slice,
            index,
            type_node,
        } => {
            let slice_value = lookup_value(slice, values)?;
            let index_value = lookup_value(index, values)?;
            if config.bounds_mode == KirBoundsMode::Checked {
                let invalid = push_check_condition(
                    KirCheckConditionKind::SliceOutOfBounds,
                    vec![slice_value, index_value],
                    ids,
                    instructions,
                );
                push_guard(
                    invalid,
                    KirFailureKind::OutOfBounds,
                    ids,
                    effect_order,
                    instructions,
                );
            }
            Ok(KirPlace::SliceIndex {
                slice: slice_value,
                index: index_value,
                type_node: type_node.clone(),
                region: value_regions
                    .get(&value_name(slice))
                    .copied()
                    .unwrap_or(conservative_region),
            })
        }
        MirPlace::Field {
            base,
            field_name,
            type_node,
        } => {
            let base = build_place(
                base,
                config,
                ids,
                values,
                value_regions,
                conservative_region,
                effect_order,
                instructions,
            )?;
            let region = place_region(&base);
            Ok(KirPlace::Field {
                base: Box::new(base),
                field_name: field_name.clone(),
                type_node: type_node.clone(),
                region,
            })
        }
    }
}

const fn place_region(place: &KirPlace) -> MemoryRegionId {
    match place {
        KirPlace::Value { region, .. }
        | KirPlace::Deref { region, .. }
        | KirPlace::Index { region, .. }
        | KirPlace::SliceIndex { region, .. }
        | KirPlace::Field { region, .. } => *region,
    }
}

fn update_value_region(
    name: &MirValueName,
    region: Option<MemoryRegionId>,
    value_regions: &mut HashMap<MirValueName, MemoryRegionId>,
) {
    if let Some(region) = region {
        value_regions.insert(name.clone(), region);
    } else {
        value_regions.remove(name);
    }
}

const fn bool_type() -> MirType {
    MirType::Primitive(MirPrimitiveTypeName::Bool)
}

const fn is_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(
            MirPrimitiveTypeName::I32
                | MirPrimitiveTypeName::I64
                | MirPrimitiveTypeName::U32
                | MirPrimitiveTypeName::U64
        )
    )
}

const fn is_signed_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)
    )
}

const fn is_memory_origin_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Pointer(_) | MirType::Slice(_))
}

fn lookup_value(
    value: &MirValue,
    values: &HashMap<MirValueName, ValueId>,
) -> Result<ValueId, KirBuildError> {
    values.get(&value_name(value)).copied().ok_or_else(|| {
        KirBuildError::new(format!(
            "MIR value '{}' is used before its KIR definition",
            print_value_name(value)
        ))
    })
}

fn value_name(value: &MirValue) -> MirValueName {
    match value {
        MirValue::Param { name, .. } => MirValueName::Param(name.clone()),
        MirValue::Local { name, .. } => MirValueName::Local(name.clone()),
        MirValue::Temp { name, .. } => MirValueName::Temp(name.clone()),
        MirValue::ConstInt { text, type_node } => {
            MirValueName::ConstInt(text.clone(), type_node.clone())
        }
        MirValue::ConstFloat { text, type_node } => {
            MirValueName::ConstFloat(text.clone(), type_node.clone())
        }
        MirValue::ConstBool { value, type_node } => {
            MirValueName::ConstBool(*value, type_node.clone())
        }
    }
}

fn inline_constants(function: &MirFunction) -> Vec<MirValue> {
    let mut constants = Vec::new();
    let mut seen = HashMap::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            visit_instruction_values(instruction, &mut |value| {
                if matches!(
                    value,
                    MirValue::ConstInt { .. }
                        | MirValue::ConstFloat { .. }
                        | MirValue::ConstBool { .. }
                ) && seen.insert(value_name(value), ()).is_none()
                {
                    constants.push(value.clone());
                }
            });
        }
        match &block.terminator {
            MirTerminator::Return { value } => {
                if let Some(value) = value {
                    visit_value(value, &mut |value| {
                        if matches!(
                            value,
                            MirValue::ConstInt { .. }
                                | MirValue::ConstFloat { .. }
                                | MirValue::ConstBool { .. }
                        ) && seen.insert(value_name(value), ()).is_none()
                        {
                            constants.push(value.clone());
                        }
                    });
                }
            }
            MirTerminator::Jump { .. } => {}
            MirTerminator::Branch { condition, .. } => {
                visit_value(condition, &mut |value| {
                    if matches!(
                        value,
                        MirValue::ConstInt { .. }
                            | MirValue::ConstFloat { .. }
                            | MirValue::ConstBool { .. }
                    ) && seen.insert(value_name(value), ()).is_none()
                    {
                        constants.push(value.clone());
                    }
                });
            }
        }
    }
    constants
}

fn visit_instruction_values(instruction: &MirInstruction, visitor: &mut impl FnMut(&MirValue)) {
    match instruction {
        MirInstruction::ConstInt { target, .. }
        | MirInstruction::ConstFloat { target, .. }
        | MirInstruction::ConstBool { target, .. } => visit_value(target, visitor),
        MirInstruction::Move { target, value }
        | MirInstruction::Cast { target, value, .. }
        | MirInstruction::SliceData {
            target,
            slice: value,
        }
        | MirInstruction::SliceLen {
            target,
            slice: value,
        } => {
            visit_value(target, visitor);
            visit_value(value, visitor);
        }
        MirInstruction::Binary {
            target,
            left,
            right,
            ..
        }
        | MirInstruction::Compare {
            target,
            left,
            right,
            ..
        } => {
            visit_value(target, visitor);
            visit_value(left, visitor);
            visit_value(right, visitor);
        }
        MirInstruction::Unary {
            target, operand, ..
        } => {
            visit_value(target, visitor);
            visit_value(operand, visitor);
        }
        MirInstruction::Address { target, place } | MirInstruction::Load { target, place } => {
            visit_value(target, visitor);
            visit_place_values(place, visitor);
        }
        MirInstruction::Store { place, value } => {
            visit_place_values(place, visitor);
            visit_value(value, visitor);
        }
        MirInstruction::MakeSlice { target, data, len } => {
            visit_value(target, visitor);
            visit_value(data, visitor);
            visit_value(len, visitor);
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            visit_value(target, visitor);
            visit_value(slice, visitor);
            visit_value(start, visitor);
            visit_value(end, visitor);
        }
        MirInstruction::Call { target, args, .. } => {
            if let Some(target) = target {
                visit_value(target, visitor);
            }
            for arg in args {
                visit_value(arg, visitor);
            }
        }
        MirInstruction::RuntimeCall { args, .. } => {
            for arg in args {
                visit_value(arg, visitor);
            }
        }
    }
}

fn visit_place_values(place: &MirPlace, visitor: &mut impl FnMut(&MirValue)) {
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => {}
        MirPlace::Deref { pointer, .. } => visit_value(pointer, visitor),
        MirPlace::Index { base, index, .. } => {
            visit_place_values(base, visitor);
            visit_value(index, visitor);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            visit_value(slice, visitor);
            visit_value(index, visitor);
        }
        MirPlace::Field { base, .. } => visit_place_values(base, visitor),
    }
}

fn visit_value(value: &MirValue, visitor: &mut impl FnMut(&MirValue)) {
    visitor(value);
}

fn print_value_name(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => name.clone(),
        _ => format!("constant {}", print_mir_type(mir_value_type(value))),
    }
}

fn mir_value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}
