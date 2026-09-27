use std::collections::{HashMap, HashSet};

use crate::{MirBinaryOp, MirPrimitiveTypeName, MirType, MirUnaryOp};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueDefinition {
    FunctionParam,
    BlockParam(BlockId),
    Instruction(BlockId, usize),
}

#[derive(Debug, Clone, Copy)]
enum MemoryDefinition {
    Initial,
    BlockParam(BlockId),
    Instruction(BlockId, usize),
}

#[derive(Clone)]
struct ValueRecord {
    definition: Option<ValueDefinition>,
    type_node: KirValueType,
}

/// Validation-local queries, rebuilt from the actual function on every check.
/// Dense storage is bounded by definition count, never by an untrusted max ID.
/// Sparse identities and any later inserts use the same exact lookup semantics.
struct ValueTable {
    base: u32,
    dense: Vec<Option<ValueRecord>>,
    sparse: HashMap<ValueId, ValueRecord>,
}

impl ValueTable {
    fn new(mut ids: impl Iterator<Item = ValueId>, capacity: usize) -> Self {
        let (base, size) = ids.next().map_or((0, 0), |first| {
            let (min, max) = ids.fold((first.index(), first.index()), |(min, max), id| {
                (min.min(id.index()), max.max(id.index()))
            });
            let span = u64::from(max) - u64::from(min) + 1;
            let size = usize::try_from(span)
                .ok()
                .filter(|span| *span <= capacity.saturating_mul(4))
                .unwrap_or(0);
            (min, size)
        });
        Self {
            base,
            dense: vec![None; size],
            sparse: HashMap::with_capacity(if size == 0 { capacity } else { 0 }),
        }
    }

    fn dense_index(&self, value: ValueId) -> Option<usize> {
        value
            .index()
            .checked_sub(self.base)
            .map(|offset| offset as usize)
            .filter(|index| *index < self.dense.len())
    }

    fn get(&self, value: ValueId) -> Option<&ValueRecord> {
        self.dense_index(value)
            .and_then(|index| self.dense[index].as_ref())
            .or_else(|| self.sparse.get(&value))
    }

    fn definition(&self, value: ValueId) -> Option<ValueDefinition> {
        self.get(value).and_then(|record| record.definition)
    }

    fn type_of(&self, value: ValueId) -> Option<&KirValueType> {
        self.get(value).map(|record| &record.type_node)
    }

    fn record(
        &mut self,
        value: ValueId,
        type_node: &KirValueType,
        definition: Option<ValueDefinition>,
    ) -> Option<ValueDefinition> {
        let empty = ValueRecord {
            definition: None,
            type_node: type_node.clone(),
        };
        let record = if let Some(index) = self.dense_index(value) {
            self.dense[index].get_or_insert(empty)
        } else {
            self.sparse.entry(value).or_insert(empty)
        };
        // Legacy validation retains the first definition after a module-wide ID
        // collision, but still records the last type for subsequent diagnostics.
        record.type_node.clone_from(type_node);
        definition.and_then(|definition| record.definition.replace(definition))
    }
}

#[must_use]
pub fn validate_kir_module(module: &KirModule) -> KirValidationResult {
    validate_kir_module_with_previous(module, None)
}

#[must_use]
pub(crate) fn validate_kir_module_incremental(
    module: &KirModule,
    previous: &KirModule,
) -> KirValidationResult {
    if module.config != previous.config || module.profile != previous.profile {
        validate_kir_module(module)
    } else {
        validate_kir_module_with_previous(module, Some(previous))
    }
}

fn validate_kir_module_with_previous(
    module: &KirModule,
    previous: Option<&KirModule>,
) -> KirValidationResult {
    let mut errors = Vec::new();
    if let Err(message) = module.profile.validate() {
        errors.push(error(message, None, None, None));
    } else if module.profile.consumer() != module.config.consumer {
        errors.push(error(
            "KIR target profile consumer does not match module consumer",
            None,
            None,
            None,
        ));
    }
    let block_capacity = module
        .functions
        .iter()
        .map(|function| function.blocks.len())
        .sum();
    let instruction_capacity = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .map(|block| block.instructions.len())
        .sum();
    let value_capacity = module
        .functions
        .iter()
        .map(|function| {
            function.params.len()
                + function
                    .blocks
                    .iter()
                    .map(|block| {
                        block.params.len()
                            + block
                                .instructions
                                .iter()
                                .map(|instruction| instruction.results.len())
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        })
        .sum();
    let region_capacity = module
        .functions
        .iter()
        .map(|function| function.regions.len())
        .sum();
    let memory_capacity = module
        .functions
        .iter()
        .map(|function| {
            function.initial_memory.len()
                + function
                    .blocks
                    .iter()
                    .map(|block| {
                        block.memory_params.len()
                            + block
                                .instructions
                                .iter()
                                .filter(|instruction| {
                                    instruction
                                        .memory
                                        .as_ref()
                                        .and_then(|memory| memory.output)
                                        .is_some()
                                })
                                .count()
                    })
                    .sum::<usize>()
        })
        .sum();
    let mut function_ids = HashSet::with_capacity(module.functions.len());
    let mut block_ids = HashSet::with_capacity(block_capacity);
    let mut instruction_ids = HashSet::with_capacity(instruction_capacity);
    let mut module_value_ids = HashSet::with_capacity(value_capacity);
    let mut module_region_ids = HashSet::with_capacity(region_capacity);
    let mut module_memory_ids = HashSet::with_capacity(memory_capacity);

    for function in &module.functions {
        if !function_ids.insert(function.id) {
            errors.push(error(
                "duplicate function id",
                Some(function.id),
                None,
                None,
            ));
        }
        let unchanged = previous.is_some_and(|previous| {
            previous
                .functions
                .iter()
                .find(|candidate| candidate.id == function.id)
                == Some(function)
        });
        if unchanged {
            record_verified_function_identities(
                function,
                &mut block_ids,
                &mut instruction_ids,
                &mut module_value_ids,
                &mut module_region_ids,
                &mut module_memory_ids,
                &mut errors,
            );
        } else {
            validate_function(
                function,
                &module.profile,
                &mut block_ids,
                &mut instruction_ids,
                &mut module_value_ids,
                &mut module_region_ids,
                &mut module_memory_ids,
                &mut errors,
            );
        }
    }
    KirValidationResult { errors }
}

#[allow(clippy::too_many_arguments)]
fn record_verified_function_identities(
    function: &KirFunction,
    module_block_ids: &mut HashSet<BlockId>,
    module_instruction_ids: &mut HashSet<InstructionId>,
    module_value_ids: &mut HashSet<ValueId>,
    module_region_ids: &mut HashSet<MemoryRegionId>,
    module_memory_ids: &mut HashSet<MemoryVersionId>,
    errors: &mut Vec<KirValidationError>,
) {
    let mut duplicate = |inserted: bool,
                         message: &'static str,
                         block: Option<BlockId>,
                         instruction: Option<InstructionId>| {
        if !inserted {
            errors.push(error(message, Some(function.id), block, instruction));
        }
    };
    for region in &function.regions {
        duplicate(
            module_region_ids.insert(region.id),
            "duplicate memory region id",
            None,
            None,
        );
    }
    for initial in &function.initial_memory {
        duplicate(
            module_memory_ids.insert(initial.version),
            "duplicate memory version id",
            None,
            None,
        );
    }
    for param in &function.params {
        duplicate(
            module_value_ids.insert(param.value),
            "duplicate value id",
            None,
            None,
        );
    }
    for block in &function.blocks {
        duplicate(
            module_block_ids.insert(block.id),
            "duplicate block id",
            Some(block.id),
            None,
        );
        for param in &block.params {
            duplicate(
                module_value_ids.insert(param.value),
                "duplicate value id",
                Some(block.id),
                None,
            );
        }
        for param in &block.memory_params {
            duplicate(
                module_memory_ids.insert(param.version),
                "duplicate memory version id",
                Some(block.id),
                None,
            );
        }
        for instruction in &block.instructions {
            duplicate(
                module_instruction_ids.insert(instruction.id),
                "duplicate instruction id",
                Some(block.id),
                Some(instruction.id),
            );
            for result in &instruction.results {
                duplicate(
                    module_value_ids.insert(result.value),
                    "duplicate value id",
                    Some(block.id),
                    Some(instruction.id),
                );
            }
            if let Some(output) = instruction.memory.as_ref().and_then(|memory| memory.output) {
                duplicate(
                    module_memory_ids.insert(output),
                    "duplicate memory version id",
                    Some(block.id),
                    Some(instruction.id),
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_function(
    function: &KirFunction,
    profile: &KirTargetProfile,
    module_block_ids: &mut HashSet<BlockId>,
    module_instruction_ids: &mut HashSet<InstructionId>,
    module_value_ids: &mut HashSet<ValueId>,
    module_region_ids: &mut HashSet<MemoryRegionId>,
    module_memory_ids: &mut HashSet<MemoryVersionId>,
    errors: &mut Vec<KirValidationError>,
) {
    let value_capacity = function.params.len()
        + function
            .blocks
            .iter()
            .map(|block| {
                block.params.len()
                    + block
                        .instructions
                        .iter()
                        .map(|instruction| instruction.results.len())
                        .sum::<usize>()
            })
            .sum::<usize>();
    let memory_capacity = function.initial_memory.len()
        + function
            .blocks
            .iter()
            .map(|block| {
                block.memory_params.len()
                    + block
                        .instructions
                        .iter()
                        .filter(|instruction| {
                            instruction
                                .memory
                                .as_ref()
                                .and_then(|memory| memory.output)
                                .is_some()
                        })
                        .count()
            })
            .sum::<usize>();
    let value_ids =
        function
            .params
            .iter()
            .map(|param| param.value)
            .chain(function.blocks.iter().flat_map(|block| {
                block.params.iter().map(|param| param.value).chain(
                    block.instructions.iter().flat_map(|instruction| {
                        instruction.results.iter().map(|result| result.value)
                    }),
                )
            }));
    let mut values = ValueTable::new(value_ids, value_capacity);
    let mut memory_definitions = HashMap::with_capacity(memory_capacity);
    let mut memory_regions = HashMap::with_capacity(memory_capacity);
    let mut function_region_ids = HashSet::with_capacity(function.regions.len());
    validate_vector_regions(function, errors);
    for region in &function.regions {
        function_region_ids.insert(region.id);
        if !module_region_ids.insert(region.id) {
            errors.push(error(
                "duplicate memory region id",
                Some(function.id),
                None,
                None,
            ));
        }
    }
    for region in &function.regions {
        if !function_region_ids.contains(&region.partition) {
            errors.push(error(
                format!(
                    "memory region r{} names undefined partition r{}",
                    region.id.index(),
                    region.partition.index()
                ),
                Some(function.id),
                None,
                None,
            ));
        }
        if let Some(parent) = region.parent
            && !function_region_ids.contains(&parent)
        {
            errors.push(error(
                format!(
                    "memory region r{} names undefined parent r{}",
                    region.id.index(),
                    parent.index()
                ),
                Some(function.id),
                None,
                None,
            ));
        }
    }
    for initial in &function.initial_memory {
        memory_regions.insert(initial.version, initial.region);
        define_memory(
            function,
            initial.version,
            MemoryDefinition::Initial,
            None,
            None,
            module_memory_ids,
            &mut memory_definitions,
            errors,
        );
    }
    for param in &function.params {
        define_value(
            function,
            param.value,
            &KirValueType::Scalar(param.type_node.clone()),
            ValueDefinition::FunctionParam,
            None,
            None,
            module_value_ids,
            &mut values,
            errors,
        );
    }
    for block in &function.blocks {
        if !module_block_ids.insert(block.id) {
            errors.push(error(
                "duplicate block id",
                Some(function.id),
                Some(block.id),
                None,
            ));
        }
        for param in &block.params {
            validate_value_type(
                function,
                profile,
                &param.type_node,
                Some(block.id),
                None,
                errors,
            );
            define_value(
                function,
                param.value,
                &param.type_node,
                ValueDefinition::BlockParam(block.id),
                Some(block.id),
                None,
                module_value_ids,
                &mut values,
                errors,
            );
        }
        for param in &block.memory_params {
            memory_regions.insert(param.version, param.region);
            define_memory(
                function,
                param.version,
                MemoryDefinition::BlockParam(block.id),
                Some(block.id),
                None,
                module_memory_ids,
                &mut memory_definitions,
                errors,
            );
        }
        for (index, instruction) in block.instructions.iter().enumerate() {
            if !module_instruction_ids.insert(instruction.id) {
                errors.push(error(
                    "duplicate instruction id",
                    Some(function.id),
                    Some(block.id),
                    Some(instruction.id),
                ));
            }
            for result in &instruction.results {
                validate_value_type(
                    function,
                    profile,
                    &result.type_node,
                    Some(block.id),
                    Some(instruction.id),
                    errors,
                );
                define_value(
                    function,
                    result.value,
                    &result.type_node,
                    ValueDefinition::Instruction(block.id, index),
                    Some(block.id),
                    Some(instruction.id),
                    module_value_ids,
                    &mut values,
                    errors,
                );
            }
            if let Some(output) = instruction.memory.as_ref().and_then(|memory| memory.output) {
                if let Some(memory) = &instruction.memory {
                    memory_regions.insert(output, memory.region);
                }
                define_memory(
                    function,
                    output,
                    MemoryDefinition::Instruction(block.id, index),
                    Some(block.id),
                    Some(instruction.id),
                    module_memory_ids,
                    &mut memory_definitions,
                    errors,
                );
            }
        }
    }

    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<HashMap<_, _>>();
    let dominators = compute_kir_dominators(function);
    let mut effect_orders = HashSet::with_capacity(
        function
            .blocks
            .iter()
            .map(|block| block.instructions.len() + 1)
            .sum(),
    );
    for block in &function.blocks {
        let mut previous_effect_order = None;
        for (index, instruction) in block.instructions.iter().enumerate() {
            visit_instruction_uses(instruction, |used| {
                validate_use(
                    function,
                    block.id,
                    index,
                    used,
                    Some(instruction.id),
                    &values,
                    &dominators,
                    errors,
                );
            });
            validate_instruction_structure(function, profile, block, index, &values, errors);
            if let Some(effect) = &instruction.effect {
                if previous_effect_order.is_some_and(|previous| effect.order <= previous)
                    || !effect_orders.insert(effect.order)
                {
                    errors.push(error(
                        "ordered effect sequence must be strictly increasing and unique",
                        Some(function.id),
                        Some(block.id),
                        Some(instruction.id),
                    ));
                }
                previous_effect_order = Some(effect.order);
            }
            if let Some(memory) = &instruction.memory {
                validate_memory_use(
                    function,
                    block.id,
                    index,
                    memory.input,
                    Some(instruction.id),
                    &memory_definitions,
                    &dominators,
                    errors,
                );
                if memory_regions.get(&memory.input) != Some(&memory.region) {
                    errors.push(error(
                        "memory input version partition does not match instruction access",
                        Some(function.id),
                        Some(block.id),
                        Some(instruction.id),
                    ));
                }
            }
        }
        let terminator_index = block.instructions.len();
        for used in terminator_uses(&block.terminator) {
            validate_use(
                function,
                block.id,
                terminator_index,
                used,
                None,
                &values,
                &dominators,
                errors,
            );
        }
        validate_terminator_types(function, block, &values, errors);
        for edge in terminator_edges(&block.terminator) {
            validate_edge(
                function,
                block.id,
                edge,
                &blocks,
                &values,
                &memory_regions,
                errors,
            );
            for version in &edge.memory_args {
                validate_memory_use(
                    function,
                    block.id,
                    terminator_index,
                    *version,
                    None,
                    &memory_definitions,
                    &dominators,
                    errors,
                );
            }
        }
        if let KirTerminator::Return { memory, .. } = &block.terminator {
            for (region, version) in memory {
                validate_memory_use(
                    function,
                    block.id,
                    terminator_index,
                    *version,
                    None,
                    &memory_definitions,
                    &dominators,
                    errors,
                );
                if memory_regions.get(version) != Some(region) {
                    errors.push(error(
                        "return memory version partition does not match returned region",
                        Some(function.id),
                        Some(block.id),
                        None,
                    ));
                }
            }
        }
        if let KirTerminator::Return { effect_order, .. } = &block.terminator
            && (previous_effect_order.is_some_and(|previous| *effect_order <= previous)
                || !effect_orders.insert(*effect_order))
        {
            errors.push(error(
                "ordered effect sequence must be strictly increasing and unique",
                Some(function.id),
                Some(block.id),
                None,
            ));
        }
    }
}

fn validate_instruction_structure(
    function: &KirFunction,
    profile: &KirTargetProfile,
    block: &KirBlock,
    index: usize,
    values: &ValueTable,
    errors: &mut Vec<KirValidationError>,
) {
    let instruction = &block.instructions[index];
    if let KirInstructionKind::Binary { left, right, .. } = instruction.kind {
        let result_type = instruction.results.first().map(|result| &result.type_node);
        if values.type_of(left) != result_type || values.type_of(right) != result_type {
            errors.push(error(
                "binary operands and value result must have one type",
                Some(function.id),
                Some(block.id),
                Some(instruction.id),
            ));
        }
    }
    let required_guard = match &instruction.kind {
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            semantics: KirArithmeticSemantics::Checked,
            ..
        }
        | KirInstructionKind::Unary {
            op: MirUnaryOp::Neg,
            semantics: KirArithmeticSemantics::Checked,
            ..
        } if instruction.results.len() == 2 => Some((
            instruction.results[1].value,
            KirFailureKind::Overflow,
            "checked arithmetic result is not followed by its required overflow guard",
        )),
        KirInstructionKind::CheckCondition { kind, .. } if instruction.results.len() == 1 => {
            let failure = match kind {
                KirCheckConditionKind::ArithmeticOverflow
                | KirCheckConditionKind::SignedDivisionOverflow => KirFailureKind::Overflow,
                KirCheckConditionKind::DivisionByZero => KirFailureKind::DivisionByZero,
                KirCheckConditionKind::SliceOutOfBounds
                | KirCheckConditionKind::InvalidSubslice => KirFailureKind::OutOfBounds,
            };
            Some((
                instruction.results[0].value,
                failure,
                "check condition is not followed by its required guard",
            ))
        }
        _ => None,
    };
    if let Some((condition, failure, message)) = required_guard
        && !block.instructions.get(index + 1).is_some_and(|next| {
            matches!(
                next.kind,
                KirInstructionKind::Guard {
                    condition: actual,
                    failure: actual_failure,
                } if actual == condition && actual_failure == failure
            )
        })
    {
        errors.push(error(
            message,
            Some(function.id),
            Some(block.id),
            Some(instruction.id),
        ));
    }
    if let KirInstructionKind::Guard { condition, .. } = instruction.kind {
        if !instruction
            .effect
            .as_ref()
            .is_some_and(|effect| effect.kind == KirEffectKind::MayFail)
        {
            errors.push(error(
                "guard must carry an ordered may-fail effect",
                Some(function.id),
                Some(block.id),
                Some(instruction.id),
            ));
        }
        if !values.type_of(condition).is_some_and(|type_node| {
            matches!(
                type_node,
                KirValueType::Scalar(MirType::Primitive(crate::MirPrimitiveTypeName::Bool))
            )
        }) {
            errors.push(error(
                "guard condition must have bool type",
                Some(function.id),
                Some(block.id),
                Some(instruction.id),
            ));
        }
    }
    if let KirInstructionKind::VersionPredicate { predicate } = &instruction.kind {
        validate_version_predicate(
            function,
            profile,
            block,
            instruction,
            predicate,
            values,
            errors,
        );
    }
    validate_vector_instruction(function, profile, block, instruction, values, errors);
}

fn validate_version_predicate(
    function: &KirFunction,
    profile: &KirTargetProfile,
    block: &KirBlock,
    instruction: &KirInstruction,
    predicate: &KirVersionPredicate,
    values: &ValueTable,
    errors: &mut Vec<KirValidationError>,
) {
    let bool_type = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::Bool));
    let u32_type = MirType::Primitive(MirPrimitiveTypeName::U32);
    let valid_result = instruction.results.as_slice()
        == [KirResult {
            value: instruction
                .results
                .first()
                .map_or(ValueId::from_index(u32::MAX), |result| result.value),
            type_node: bool_type,
        }];
    let layout_matches = matches!(
        profile.layout(),
        KirProfileLayout::Known { pointer_width_bits, .. }
            if pointer_width_bits == u16::from(predicate.address_bits)
    );
    if !valid_result
        || instruction.memory.is_some()
        || instruction.effect.is_some()
        || !matches!(
            profile.consumer(),
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
        )
        || !layout_matches
        || predicate.conjuncts.is_empty()
        || predicate.conjuncts.len() > 4
    {
        errors.push(error(
            "version predicate result, target layout, or conjunct count is invalid",
            Some(function.id),
            Some(block.id),
            Some(instruction.id),
        ));
        return;
    }
    for conjunct in &predicate.conjuncts {
        let valid = match conjunct {
            KirVersionPredicateConjunct::TripThreshold { value, minimum } => {
                *minimum > 0
                    && values.type_of(*value).and_then(KirValueType::as_scalar) == Some(&u32_type)
            }
            KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                left,
                left_count,
                left_element_bytes,
                right,
                right_count,
                right_element_bytes,
            } => {
                let slice_bytes = |value: ValueId| {
                    values
                        .type_of(value)
                        .and_then(KirValueType::as_scalar)
                        .and_then(|type_node| match type_node {
                            MirType::Slice(element) => match element.as_ref() {
                                MirType::Primitive(
                                    MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32,
                                ) => Some(4),
                                MirType::Primitive(
                                    MirPrimitiveTypeName::I64
                                    | MirPrimitiveTypeName::U64
                                    | MirPrimitiveTypeName::F64,
                                ) => Some(8),
                                _ => None,
                            },
                            _ => None,
                        })
                };
                left != right
                    && slice_bytes(*left) == Some(*left_element_bytes)
                    && slice_bytes(*right) == Some(*right_element_bytes)
                    && values
                        .type_of(*left_count)
                        .and_then(KirValueType::as_scalar)
                        == Some(&u32_type)
                    && values
                        .type_of(*right_count)
                        .and_then(KirValueType::as_scalar)
                        == Some(&u32_type)
            }
        };
        if !valid {
            errors.push(error(
                "version predicate conjunct is not total and well typed",
                Some(function.id),
                Some(block.id),
                Some(instruction.id),
            ));
        }
    }
}

fn validate_vector_regions(function: &KirFunction, errors: &mut Vec<KirValidationError>) {
    let blocks = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<HashSet<_>>();
    let mut ids = HashSet::new();
    let mut owners = HashMap::new();
    for region in &function.vector_regions {
        if !ids.insert(region.id) {
            errors.push(error(
                "duplicate vector region id",
                Some(function.id),
                None,
                None,
            ));
        }
        let mut previous = None;
        for &block in &region.blocks {
            if !blocks.contains(&block) {
                errors.push(error(
                    "vector region names an undefined block",
                    Some(function.id),
                    Some(block),
                    None,
                ));
            }
            if previous.is_some_and(|previous| previous >= block) {
                errors.push(error(
                    "vector region blocks must be strictly ordered and unique",
                    Some(function.id),
                    Some(block),
                    None,
                ));
            }
            previous = Some(block);
            if owners.insert(block, region.id).is_some() {
                errors.push(error(
                    "a block cannot belong to more than one vector region",
                    Some(function.id),
                    Some(block),
                    None,
                ));
            }
        }
    }
}

fn validate_value_type(
    function: &KirFunction,
    profile: &KirTargetProfile,
    type_node: &KirValueType,
    block: Option<BlockId>,
    instruction: Option<InstructionId>,
    errors: &mut Vec<KirValidationError>,
) {
    let valid = match type_node {
        KirValueType::Scalar(_) => true,
        KirValueType::FixedVector { lane, lanes } => {
            profile.vector_operations_enabled() && profile.supports_vector_shape(*lane, *lanes)
        }
        KirValueType::Mask { lanes } => {
            profile.vector_operations_enabled() && profile.supports_mask_lanes(*lanes)
        }
    };
    if !valid {
        let message = if profile.vector_operations_enabled() {
            "KIR value uses a vector lane/count unsupported by the target profile"
        } else {
            "KIR target profile vector operations are disabled"
        };
        errors.push(error(message, Some(function.id), block, instruction));
    }
}

fn validate_vector_instruction(
    function: &KirFunction,
    profile: &KirTargetProfile,
    block: &KirBlock,
    instruction: &KirInstruction,
    values: &ValueTable,
    errors: &mut Vec<KirValidationError>,
) {
    let vector_kind = matches!(
        instruction.kind,
        KirInstructionKind::VectorSplat { .. }
            | KirInstructionKind::VectorLoad { .. }
            | KirInstructionKind::VectorStore { .. }
            | KirInstructionKind::VectorBinary { .. }
            | KirInstructionKind::VectorUnary { .. }
            | KirInstructionKind::VectorCompare { .. }
            | KirInstructionKind::VectorSelect { .. }
            | KirInstructionKind::VectorCast { .. }
            | KirInstructionKind::VectorInsert { .. }
            | KirInstructionKind::VectorExtract { .. }
            | KirInstructionKind::VectorReduce { .. }
    );
    if !vector_kind {
        if instruction
            .results
            .iter()
            .any(|result| result.type_node.as_scalar().is_none())
        {
            vector_error(
                function,
                block,
                instruction,
                "scalar KIR instruction cannot produce a vector or mask value",
                errors,
            );
        }
        return;
    }
    let region = vector_instruction_region(&instruction.kind)
        .expect("every closed vector instruction has a region");
    if !function
        .vector_regions
        .iter()
        .any(|candidate| candidate.id == region && candidate.blocks.contains(&block.id))
    {
        vector_error(
            function,
            block,
            instruction,
            "vector instruction is outside its declared vector region",
            errors,
        );
    }
    match &instruction.kind {
        KirInstructionKind::VectorSplat { scalar, .. } => {
            let Some((lane, lanes)) = one_vector_result(instruction) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector splat result is malformed",
                    errors,
                );
                return;
            };
            if values.type_of(*scalar).and_then(KirValueType::as_scalar)
                != Some(&lane_mir_type(lane))
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector splat scalar lane type mismatch",
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                KirProfileOperation::Splat,
                lane,
                lanes,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorLoad { access, .. } => {
            if one_vector_result(instruction) != Some((access.lane, access.lanes)) {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector load result type mismatch",
                    errors,
                );
            }
            validate_vector_memory(
                function,
                profile,
                block,
                instruction,
                access,
                false,
                values,
                errors,
            );
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            if !instruction.results.is_empty()
                || values.type_of(*value)
                    != Some(&KirValueType::FixedVector {
                        lane: access.lane,
                        lanes: access.lanes,
                    })
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector store value type mismatch",
                    errors,
                );
            }
            validate_vector_memory(
                function,
                profile,
                block,
                instruction,
                access,
                true,
                values,
                errors,
            );
        }
        KirInstructionKind::VectorBinary {
            op,
            left,
            right,
            semantics,
            no_failure_proof,
            ..
        } => {
            let Some((lane, lanes)) = one_vector_result(instruction) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector binary result is malformed",
                    errors,
                );
                return;
            };
            let expected = KirValueType::FixedVector { lane, lanes };
            if values.type_of(*left) != Some(&expected) || values.type_of(*right) != Some(&expected)
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector binary operand/result lane mismatch",
                    errors,
                );
            }
            let semantic_valid = if lane == KirLaneType::F64 {
                *semantics == KirArithmeticSemantics::StrictFloat
                    && *op != KirVectorBinaryOp::Remainder
            } else {
                *semantics != KirArithmeticSemantics::StrictFloat
            };
            if !semantic_valid {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector binary arithmetic semantics are invalid",
                    errors,
                );
            }
            let proof_required = lane != KirLaneType::F64
                && (*semantics == KirArithmeticSemantics::Checked
                    || matches!(op, KirVectorBinaryOp::Divide | KirVectorBinaryOp::Remainder));
            if proof_required != no_failure_proof.is_some() {
                vector_error(
                    function,
                    block,
                    instruction,
                    if proof_required {
                        "failing integer vector arithmetic requires a no-failure proof"
                    } else {
                        "infallible vector arithmetic has an unexpected no-failure proof"
                    },
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                vector_binary_profile_operation(*op),
                lane,
                lanes,
                cost_semantics(*semantics),
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorUnary {
            op,
            operand,
            semantics,
            no_failure_proof,
            ..
        } => match op {
            KirVectorUnaryOp::Negate => {
                let Some((lane, lanes)) = one_vector_result(instruction) else {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "vector unary result is malformed",
                        errors,
                    );
                    return;
                };
                if values.type_of(*operand) != Some(&KirValueType::FixedVector { lane, lanes })
                    || (lane == KirLaneType::F64
                        && *semantics != KirArithmeticSemantics::StrictFloat)
                    || (lane != KirLaneType::F64
                        && *semantics == KirArithmeticSemantics::StrictFloat)
                {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "vector unary lane or semantics mismatch",
                        errors,
                    );
                }
                let proof_required =
                    lane != KirLaneType::F64 && *semantics == KirArithmeticSemantics::Checked;
                if proof_required != no_failure_proof.is_some() {
                    vector_error(
                        function,
                        block,
                        instruction,
                        if proof_required {
                            "checked integer vector negate requires a no-failure proof"
                        } else {
                            "vector negate has an unexpected no-failure proof"
                        },
                        errors,
                    );
                }
                require_vector_operation(
                    function,
                    profile,
                    block,
                    instruction,
                    KirProfileOperation::Negate,
                    lane,
                    lanes,
                    cost_semantics(*semantics),
                    KirAlignmentClass::NotApplicable,
                    errors,
                );
            }
            KirVectorUnaryOp::MaskNot => {
                let [result] = instruction.results.as_slice() else {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "mask not result is malformed",
                        errors,
                    );
                    return;
                };
                let KirValueType::Mask { lanes } = result.type_node else {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "mask not must produce a mask",
                        errors,
                    );
                    return;
                };
                if values.type_of(*operand) != Some(&KirValueType::Mask { lanes }) {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "mask not operand lane mismatch",
                        errors,
                    );
                }
                if *semantics != KirArithmeticSemantics::Modular {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "mask not must use canonical modular semantics",
                        errors,
                    );
                }
                if no_failure_proof.is_some() {
                    vector_error(
                        function,
                        block,
                        instruction,
                        "mask not cannot carry a no-failure proof",
                        errors,
                    );
                }
                require_vector_operation(
                    function,
                    profile,
                    block,
                    instruction,
                    KirProfileOperation::MaskNot,
                    KIR_MASK_COST_LANE,
                    lanes,
                    KirCostSemantics::NotApplicable,
                    KirAlignmentClass::NotApplicable,
                    errors,
                );
            }
        },
        KirInstructionKind::VectorCompare { left, right, .. } => {
            let [result] = instruction.results.as_slice() else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector compare result is malformed",
                    errors,
                );
                return;
            };
            let KirValueType::Mask { lanes } = result.type_node else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector compare must produce a mask",
                    errors,
                );
                return;
            };
            let Some(KirValueType::FixedVector {
                lane,
                lanes: operand_lanes,
            }) = values.type_of(*left)
            else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector compare operands must be vectors",
                    errors,
                );
                return;
            };
            if *operand_lanes != lanes || values.type_of(*right) != values.type_of(*left) {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector compare lane mismatch",
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                KirProfileOperation::Compare,
                *lane,
                lanes,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => {
            let Some((lane, lanes)) = one_vector_result(instruction) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector select result is malformed",
                    errors,
                );
                return;
            };
            let expected = KirValueType::FixedVector { lane, lanes };
            if values.type_of(*mask) != Some(&KirValueType::Mask { lanes })
                || values.type_of(*when_true) != Some(&expected)
                || values.type_of(*when_false) != Some(&expected)
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector select mask or lane mismatch",
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                KirProfileOperation::Select,
                lane,
                lanes,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorCast { op, value, .. } => {
            let Some((target_lane, lanes)) = one_vector_result(instruction) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector cast result is malformed",
                    errors,
                );
                return;
            };
            let source_lane = match op {
                KirVectorCastOp::I32ToF64 => KirLaneType::I32,
                KirVectorCastOp::U32ToF64 => KirLaneType::U32,
            };
            if target_lane != KirLaneType::F64
                || values.type_of(*value)
                    != Some(&KirValueType::FixedVector {
                        lane: source_lane,
                        lanes,
                    })
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "unsupported vector cast lane mapping",
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                KirProfileOperation::Cast,
                source_lane,
                lanes,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorInsert {
            vector,
            scalar,
            lane_index,
            ..
        } => {
            let Some((lane, lanes)) = one_vector_result(instruction) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector insert result is malformed",
                    errors,
                );
                return;
            };
            if *lane_index >= lanes
                || values.type_of(*vector) != Some(&KirValueType::FixedVector { lane, lanes })
                || values.type_of(*scalar).and_then(KirValueType::as_scalar)
                    != Some(&lane_mir_type(lane))
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector insert lane mapping is invalid",
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                KirProfileOperation::Insert,
                lane,
                lanes,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorExtract {
            vector, lane_index, ..
        } => {
            let Some(KirValueType::FixedVector { lane, lanes }) = values.type_of(*vector) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector extract operand is not a vector",
                    errors,
                );
                return;
            };
            if *lane_index >= *lanes
                || instruction
                    .results
                    .as_slice()
                    .first()
                    .and_then(|result| result.type_node.as_scalar())
                    != Some(&lane_mir_type(*lane))
                || instruction.results.len() != 1
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector extract lane mapping is invalid",
                    errors,
                );
            }
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                KirProfileOperation::Extract,
                *lane,
                *lanes,
                KirCostSemantics::NotApplicable,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        KirInstructionKind::VectorReduce {
            op,
            vector,
            semantics,
            ..
        } => {
            let Some(KirValueType::FixedVector { lane, lanes }) = values.type_of(*vector) else {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector reduction operand is not a vector",
                    errors,
                );
                return;
            };
            if *lane == KirLaneType::F64
                || *semantics != KirArithmeticSemantics::Modular
                || instruction.results.len() != 1
                || instruction.results[0].type_node.as_scalar() != Some(&lane_mir_type(*lane))
            {
                vector_error(
                    function,
                    block,
                    instruction,
                    "vector reduction must be exact modular integer reduction",
                    errors,
                );
            }
            let operation = match op {
                KirVectorReductionOp::ModularAdd => KirProfileOperation::ReduceAdd,
                KirVectorReductionOp::ModularMultiply => KirProfileOperation::ReduceMultiply,
                KirVectorReductionOp::ModularMin => KirProfileOperation::ReduceMin,
                KirVectorReductionOp::ModularMax => KirProfileOperation::ReduceMax,
            };
            require_vector_operation(
                function,
                profile,
                block,
                instruction,
                operation,
                *lane,
                *lanes,
                KirCostSemantics::Modular,
                KirAlignmentClass::NotApplicable,
                errors,
            );
        }
        _ => unreachable!("non-vector instruction returned above"),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_vector_memory(
    function: &KirFunction,
    profile: &KirTargetProfile,
    block: &KirBlock,
    instruction: &KirInstruction,
    access: &KirVectorMemoryAccess,
    store: bool,
    values: &ValueTable,
    errors: &mut Vec<KirValidationError>,
) {
    let expected_bytes = u32::from(access.lane.bit_width() / 8) * u32::from(access.lanes);
    if access.byte_footprint != expected_bytes {
        vector_error(
            function,
            block,
            instruction,
            "vector memory byte footprint is not exact",
            errors,
        );
    }
    if access.known_alignment == 0
        || !access.known_alignment.is_power_of_two()
        || access.required_alignment == 0
        || !access.required_alignment.is_power_of_two()
        || access.known_alignment < access.required_alignment
    {
        vector_error(
            function,
            block,
            instruction,
            "vector memory alignment is invalid or unproven",
            errors,
        );
    }
    let u32_type = MirType::Primitive(crate::MirPrimitiveTypeName::U32);
    if values
        .type_of(access.start)
        .and_then(KirValueType::as_scalar)
        != Some(&u32_type)
        || values.type_of(access.end).and_then(KirValueType::as_scalar) != Some(&u32_type)
    {
        vector_error(
            function,
            block,
            instruction,
            "vector memory start/end must be u32",
            errors,
        );
    }
    let expected_slice = MirType::Slice(Box::new(lane_mir_type(access.lane)));
    if values
        .type_of(access.slice)
        .and_then(KirValueType::as_scalar)
        != Some(&expected_slice)
    {
        vector_error(
            function,
            block,
            instruction,
            "vector memory slice lane type mismatch",
            errors,
        );
    }
    let valid_memory = instruction.memory.as_ref().is_some_and(|memory| {
        function
            .regions
            .iter()
            .any(|region| region.id == memory.region && region.partition == memory.region)
            && function.regions.iter().any(|region| {
                region.partition == memory.region
                    && matches!(
                        region.origin,
                        KirMemoryRegionOrigin::Parameter(value)
                            | KirMemoryRegionOrigin::RawSlice(value)
                            | KirMemoryRegionOrigin::Subslice(value)
                            if value == access.slice
                    )
            })
            && if store {
                memory.output.is_some()
                    && instruction
                        .effect
                        .as_ref()
                        .is_some_and(|effect| effect.kind == KirEffectKind::WriteMemory)
            } else {
                memory.output.is_none()
                    && instruction
                        .effect
                        .as_ref()
                        .is_some_and(|effect| effect.kind == KirEffectKind::ReadMemory)
            }
    });
    if !valid_memory {
        vector_error(
            function,
            block,
            instruction,
            "vector memory instruction has invalid Memory SSA or effect",
            errors,
        );
    }
    require_vector_operation(
        function,
        profile,
        block,
        instruction,
        if store {
            KirProfileOperation::Store
        } else {
            KirProfileOperation::Load
        },
        access.lane,
        access.lanes,
        KirCostSemantics::NotApplicable,
        KirAlignmentClass::Bytes(access.required_alignment),
        errors,
    );
}

#[allow(clippy::too_many_arguments)]
fn require_vector_operation(
    function: &KirFunction,
    profile: &KirTargetProfile,
    block: &KirBlock,
    instruction: &KirInstruction,
    operation: KirProfileOperation,
    lane: KirLaneType,
    lanes: u16,
    semantics: KirCostSemantics,
    alignment: KirAlignmentClass,
    errors: &mut Vec<KirValidationError>,
) {
    let legal = u8::try_from(lanes).ok().is_some_and(|lanes| {
        matches!(
            profile.operation_availability(&KirCostKey {
                operation,
                lane,
                lanes,
                semantics,
                alignment,
            }),
            Some(KirOperationAvailability::Legal(_))
        )
    });
    if !legal {
        vector_error(
            function,
            block,
            instruction,
            "vector instruction is unavailable in the exact target profile",
            errors,
        );
    }
}

fn one_vector_result(instruction: &KirInstruction) -> Option<(KirLaneType, u16)> {
    let [result] = instruction.results.as_slice() else {
        return None;
    };
    match result.type_node {
        KirValueType::FixedVector { lane, lanes } => Some((lane, lanes)),
        KirValueType::Scalar(_) | KirValueType::Mask { .. } => None,
    }
}

const fn vector_instruction_region(kind: &KirInstructionKind) -> Option<VectorRegionId> {
    match kind {
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
        | KirInstructionKind::VectorReduce { region, .. } => Some(*region),
        _ => None,
    }
}

const fn vector_binary_profile_operation(op: KirVectorBinaryOp) -> KirProfileOperation {
    match op {
        KirVectorBinaryOp::Add => KirProfileOperation::Add,
        KirVectorBinaryOp::Subtract => KirProfileOperation::Subtract,
        KirVectorBinaryOp::Multiply => KirProfileOperation::Multiply,
        KirVectorBinaryOp::Divide => KirProfileOperation::Divide,
        KirVectorBinaryOp::Remainder => KirProfileOperation::Remainder,
    }
}

const fn cost_semantics(semantics: KirArithmeticSemantics) -> KirCostSemantics {
    match semantics {
        KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
        KirArithmeticSemantics::Checked => KirCostSemantics::Checked,
        KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
    }
}

fn lane_mir_type(lane: KirLaneType) -> MirType {
    MirType::Primitive(match lane {
        KirLaneType::I32 => crate::MirPrimitiveTypeName::I32,
        KirLaneType::I64 => crate::MirPrimitiveTypeName::I64,
        KirLaneType::U32 => crate::MirPrimitiveTypeName::U32,
        KirLaneType::U64 => crate::MirPrimitiveTypeName::U64,
        KirLaneType::F64 => crate::MirPrimitiveTypeName::F64,
    })
}

fn vector_error(
    function: &KirFunction,
    block: &KirBlock,
    instruction: &KirInstruction,
    message: &str,
    errors: &mut Vec<KirValidationError>,
) {
    errors.push(error(
        message,
        Some(function.id),
        Some(block.id),
        Some(instruction.id),
    ));
}

#[allow(clippy::too_many_arguments)]
fn define_memory(
    function: &KirFunction,
    version: MemoryVersionId,
    definition: MemoryDefinition,
    block: Option<BlockId>,
    instruction: Option<InstructionId>,
    module_memory_ids: &mut HashSet<MemoryVersionId>,
    definitions: &mut HashMap<MemoryVersionId, MemoryDefinition>,
    errors: &mut Vec<KirValidationError>,
) {
    if !module_memory_ids.insert(version) || definitions.insert(version, definition).is_some() {
        errors.push(error(
            "duplicate memory version definition",
            Some(function.id),
            block,
            instruction,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn define_value(
    function: &KirFunction,
    value: ValueId,
    type_node: &KirValueType,
    definition: ValueDefinition,
    block: Option<BlockId>,
    instruction: Option<InstructionId>,
    module_value_ids: &mut HashSet<ValueId>,
    values: &mut ValueTable,
    errors: &mut Vec<KirValidationError>,
) {
    let fresh = module_value_ids.insert(value);
    let previous = values.record(value, type_node, fresh.then_some(definition));
    if !fresh || previous.is_some() {
        errors.push(error(
            "duplicate value definition",
            Some(function.id),
            block,
            instruction,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_use(
    function: &KirFunction,
    use_block: BlockId,
    use_index: usize,
    value: ValueId,
    instruction: Option<InstructionId>,
    values: &ValueTable,
    dominators: &KirDominators,
    errors: &mut Vec<KirValidationError>,
) {
    let Some(definition) = values.definition(value) else {
        errors.push(error(
            format!("value v{} is not defined", value.index()),
            Some(function.id),
            Some(use_block),
            instruction,
        ));
        return;
    };
    let dominates = match definition {
        ValueDefinition::FunctionParam => true,
        ValueDefinition::BlockParam(def_block) => {
            def_block == use_block || dominators.dominates(def_block, use_block)
        }
        ValueDefinition::Instruction(def_block, def_index) => {
            if def_block == use_block {
                def_index < use_index
            } else {
                dominators.dominates(def_block, use_block)
            }
        }
    };
    if !dominates {
        errors.push(error(
            format!(
                "value v{} definition does not dominate its use",
                value.index()
            ),
            Some(function.id),
            Some(use_block),
            instruction,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_memory_use(
    function: &KirFunction,
    use_block: BlockId,
    use_index: usize,
    version: MemoryVersionId,
    instruction: Option<InstructionId>,
    definitions: &HashMap<MemoryVersionId, MemoryDefinition>,
    dominators: &KirDominators,
    errors: &mut Vec<KirValidationError>,
) {
    let Some(definition) = definitions.get(&version) else {
        errors.push(error(
            format!("memory version m{} is not defined", version.index()),
            Some(function.id),
            Some(use_block),
            instruction,
        ));
        return;
    };
    let dominates = match definition {
        MemoryDefinition::Initial => true,
        MemoryDefinition::BlockParam(def_block) => {
            *def_block == use_block || dominators.dominates(*def_block, use_block)
        }
        MemoryDefinition::Instruction(def_block, def_index) => {
            if *def_block == use_block {
                *def_index < use_index
            } else {
                dominators.dominates(*def_block, use_block)
            }
        }
    };
    if !dominates {
        errors.push(error(
            format!(
                "memory version m{} definition does not dominate its use",
                version.index()
            ),
            Some(function.id),
            Some(use_block),
            instruction,
        ));
    }
}

fn validate_edge(
    function: &KirFunction,
    source: BlockId,
    edge: &KirEdge,
    blocks: &HashMap<BlockId, &KirBlock>,
    values: &ValueTable,
    memory_regions: &HashMap<MemoryVersionId, MemoryRegionId>,
    errors: &mut Vec<KirValidationError>,
) {
    let Some(target) = blocks.get(&edge.target) else {
        errors.push(error(
            format!(
                "edge from b{} names missing block b{}",
                source.index(),
                edge.target.index()
            ),
            Some(function.id),
            Some(source),
            None,
        ));
        return;
    };
    if edge.args.len() != target.params.len() {
        errors.push(error(
            format!(
                "edge to b{} has block argument arity {}, expected {}",
                edge.target.index(),
                edge.args.len(),
                target.params.len()
            ),
            Some(function.id),
            Some(source),
            None,
        ));
    }
    if edge.memory_args.len() != target.memory_params.len() {
        errors.push(error(
            format!(
                "edge to b{} has memory argument arity {}, expected {}",
                edge.target.index(),
                edge.memory_args.len(),
                target.memory_params.len()
            ),
            Some(function.id),
            Some(source),
            None,
        ));
    }
    for (argument, param) in edge.args.iter().zip(&target.params) {
        if let Some(argument_type) = values.type_of(*argument)
            && argument_type != &param.type_node
        {
            errors.push(error(
                format!(
                    "edge to b{} passes v{} with the wrong block argument type",
                    edge.target.index(),
                    argument.index()
                ),
                Some(function.id),
                Some(source),
                None,
            ));
        }
        if values
            .type_of(*argument)
            .is_some_and(|type_node| type_node.as_scalar().is_none())
            || param.type_node.as_scalar().is_none()
        {
            let source_region = vector_region_for_block(function, source);
            let target_region = vector_region_for_block(function, edge.target);
            if source_region.is_none() || source_region != target_region {
                errors.push(error(
                    "vector or mask value escapes its vector region on a block edge",
                    Some(function.id),
                    Some(source),
                    None,
                ));
            }
        }
    }
    for (argument, param) in edge.memory_args.iter().zip(&target.memory_params) {
        if memory_regions.get(argument) != Some(&param.region) {
            errors.push(error(
                "memory edge argument partition does not match target phi",
                Some(function.id),
                Some(source),
                None,
            ));
        }
    }
}

fn vector_region_for_block(function: &KirFunction, block: BlockId) -> Option<VectorRegionId> {
    function
        .vector_regions
        .iter()
        .find(|region| region.blocks.contains(&block))
        .map(|region| region.id)
}

fn validate_terminator_types(
    function: &KirFunction,
    block: &KirBlock,
    values: &ValueTable,
    errors: &mut Vec<KirValidationError>,
) {
    match &block.terminator {
        KirTerminator::Return { value, .. } => {
            let valid = match (value, &function.return_type) {
                (None, MirType::Void) => true,
                (Some(value), expected) => {
                    values.type_of(*value) == Some(&KirValueType::Scalar(expected.clone()))
                }
                (None, _) => false,
            };
            if !valid {
                errors.push(error(
                    "return value cannot expose a vector or mismatch the scalar function ABI",
                    Some(function.id),
                    Some(block.id),
                    None,
                ));
            }
        }
        KirTerminator::Branch { condition, .. } => {
            if values.type_of(*condition)
                != Some(&KirValueType::Scalar(MirType::Primitive(
                    crate::MirPrimitiveTypeName::Bool,
                )))
            {
                errors.push(error(
                    "branch condition must be scalar bool, not a vector mask",
                    Some(function.id),
                    Some(block.id),
                    None,
                ));
            }
        }
        KirTerminator::Jump { .. } => {}
    }
}

fn visit_instruction_uses(instruction: &KirInstruction, mut visit: impl FnMut(ValueId)) {
    match &instruction.kind {
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => {}
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => {
            visit(*value)
        }
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => {
            visit(*left);
            visit(*right);
        }
        KirInstructionKind::Unary { operand, .. } => visit(*operand),
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.iter().copied().for_each(visit),
        KirInstructionKind::Guard { condition, .. } => visit(*condition),
        KirInstructionKind::Address { place } | KirInstructionKind::Load { place } => {
            visit_place_uses(place, &mut visit);
        }
        KirInstructionKind::Store { place, value } => {
            visit_place_uses(place, &mut visit);
            visit(*value);
        }
        KirInstructionKind::MakeSlice { data, len } => {
            visit(*data);
            visit(*len);
        }
        KirInstructionKind::SliceData { slice } | KirInstructionKind::SliceLen { slice } => {
            visit(*slice)
        }
        KirInstructionKind::Subslice { slice, start, end } => {
            visit(*slice);
            visit(*start);
            visit(*end);
        }
        KirInstructionKind::VersionPredicate { predicate } => {
            for conjunct in &predicate.conjuncts {
                match conjunct {
                    KirVersionPredicateConjunct::TripThreshold { value, .. } => visit(*value),
                    KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left,
                        left_count,
                        right,
                        right_count,
                        ..
                    } => {
                        visit(*left);
                        visit(*left_count);
                        visit(*right);
                        visit(*right_count);
                    }
                }
            }
        }
        KirInstructionKind::VectorSplat { scalar, .. } => visit(*scalar),
        KirInstructionKind::VectorLoad { access, .. } => {
            visit(access.slice);
            visit(access.start);
            visit(access.end);
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            visit(access.slice);
            visit(access.start);
            visit(access.end);
            visit(*value);
        }
        KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => {
            visit(*left);
            visit(*right);
        }
        KirInstructionKind::VectorUnary { operand, .. } => visit(*operand),
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => {
            visit(*mask);
            visit(*when_true);
            visit(*when_false);
        }
        KirInstructionKind::VectorCast { value, .. } => visit(*value),
        KirInstructionKind::VectorInsert { vector, scalar, .. } => {
            visit(*vector);
            visit(*scalar);
        }
        KirInstructionKind::VectorExtract { vector, .. }
        | KirInstructionKind::VectorReduce { vector, .. } => visit(*vector),
    }
}

fn visit_place_uses(place: &KirPlace, visit: &mut impl FnMut(ValueId)) {
    match place {
        KirPlace::Value { value, .. } => visit(*value),
        KirPlace::Deref { pointer, .. } => visit(*pointer),
        KirPlace::Index { base, index, .. } => {
            visit_place_uses(base, visit);
            visit(*index);
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            visit(*slice);
            visit(*index);
        }
        KirPlace::Field { base, .. } => visit_place_uses(base, visit),
    }
}

#[cfg(test)]
fn instruction_uses(instruction: &KirInstruction) -> Vec<ValueId> {
    match &instruction.kind {
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => Vec::new(),
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => vec![*value],
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => vec![*left, *right],
        KirInstructionKind::Unary { operand, .. } => vec![*operand],
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.clone(),
        KirInstructionKind::Guard { condition, .. } => vec![*condition],
        KirInstructionKind::Address { place } | KirInstructionKind::Load { place } => {
            place_uses(place)
        }
        KirInstructionKind::Store { place, value } => {
            let mut values = place_uses(place);
            values.push(*value);
            values
        }
        KirInstructionKind::MakeSlice { data, len } => vec![*data, *len],
        KirInstructionKind::SliceData { slice } | KirInstructionKind::SliceLen { slice } => {
            vec![*slice]
        }
        KirInstructionKind::Subslice { slice, start, end } => vec![*slice, *start, *end],
        KirInstructionKind::VersionPredicate { predicate } => predicate
            .conjuncts
            .iter()
            .flat_map(|conjunct| match conjunct {
                KirVersionPredicateConjunct::TripThreshold { value, .. } => vec![*value],
                KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                    left,
                    left_count,
                    right,
                    right_count,
                    ..
                } => vec![*left, *left_count, *right, *right_count],
            })
            .collect(),
        KirInstructionKind::VectorSplat { scalar, .. } => vec![*scalar],
        KirInstructionKind::VectorLoad { access, .. } => {
            vec![access.slice, access.start, access.end]
        }
        KirInstructionKind::VectorStore { access, value, .. } => {
            vec![access.slice, access.start, access.end, *value]
        }
        KirInstructionKind::VectorBinary { left, right, .. }
        | KirInstructionKind::VectorCompare { left, right, .. } => vec![*left, *right],
        KirInstructionKind::VectorUnary { operand, .. } => vec![*operand],
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            ..
        } => vec![*mask, *when_true, *when_false],
        KirInstructionKind::VectorCast { value, .. } => vec![*value],
        KirInstructionKind::VectorInsert { vector, scalar, .. } => vec![*vector, *scalar],
        KirInstructionKind::VectorExtract { vector, .. }
        | KirInstructionKind::VectorReduce { vector, .. } => vec![*vector],
    }
}

#[cfg(test)]
fn place_uses(place: &KirPlace) -> Vec<ValueId> {
    match place {
        KirPlace::Value { value, .. } => vec![*value],
        KirPlace::Deref { pointer, .. } => vec![*pointer],
        KirPlace::Index { base, index, .. } => {
            let mut values = place_uses(base);
            values.push(*index);
            values
        }
        KirPlace::SliceIndex { slice, index, .. } => vec![*slice, *index],
        KirPlace::Field { base, .. } => place_uses(base),
    }
}

fn terminator_uses(terminator: &KirTerminator) -> impl Iterator<Item = ValueId> {
    let value = match terminator {
        KirTerminator::Return { value, .. } => *value,
        KirTerminator::Jump { .. } => None,
        KirTerminator::Branch { condition, .. } => Some(*condition),
    };
    value
        .into_iter()
        .chain(terminator_edges(terminator).flat_map(|edge| edge.args.iter().copied()))
}

fn terminator_edges(terminator: &KirTerminator) -> impl Iterator<Item = &KirEdge> {
    let edges = match terminator {
        KirTerminator::Return { .. } => [None, None],
        KirTerminator::Jump { edge } => [Some(edge), None],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
    };
    edges.into_iter().flatten()
}

fn error(
    message: impl Into<String>,
    function: Option<FunctionId>,
    block: Option<BlockId>,
    instruction: Option<InstructionId>,
) -> KirValidationError {
    KirValidationError {
        message: message.into(),
        function,
        block,
        instruction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_parameter_uses_should_match_full_dominance_including_unreachable_blocks() {
        let blocks = [0, 2, 3].map(BlockId::from_index);
        let function = KirFunction {
            id: FunctionId::from_index(0),
            name: "dominance".into(),
            exported: false,
            params: vec![],
            return_type: MirType::Void,
            regions: vec![],
            initial_memory: vec![],
            vector_regions: vec![],
            blocks: blocks
                .iter()
                .map(|&id| KirBlock {
                    id,
                    label: id.index().to_string(),
                    params: vec![],
                    memory_params: vec![],
                    instructions: vec![],
                    terminator: if id == blocks[0] {
                        KirTerminator::Jump {
                            edge: KirEdge {
                                target: blocks[1],
                                args: vec![],
                                memory_args: vec![],
                            },
                        }
                    } else {
                        KirTerminator::Return {
                            value: None,
                            memory: vec![],
                            effect_order: id.index(),
                        }
                    },
                })
                .collect(),
        };
        let dominators = compute_kir_dominators(&function);
        let value = ValueId::from_index(0);
        let memory = MemoryVersionId::from_index(0);
        for definition in blocks.into_iter().chain([BlockId::from_index(u32::MAX)]) {
            let mut values = ValueTable::new(std::iter::once(value), 1);
            values.record(
                value,
                &KirValueType::Scalar(MirType::Void),
                Some(ValueDefinition::BlockParam(definition)),
            );
            let memories = HashMap::from([(memory, MemoryDefinition::BlockParam(definition))]);
            for used in blocks {
                let expected = dominators.dominates(definition, used);
                let mut errors = Vec::new();
                validate_use(
                    &function,
                    used,
                    0,
                    value,
                    None,
                    &values,
                    &dominators,
                    &mut errors,
                );
                assert_eq!(errors.is_empty(), expected);
                errors.clear();
                validate_memory_use(
                    &function,
                    used,
                    0,
                    memory,
                    None,
                    &memories,
                    &dominators,
                    &mut errors,
                );
                assert_eq!(errors.is_empty(), expected);
            }
        }
    }

    #[test]
    fn value_table_should_match_legacy_queries_without_sparse_id_allocation() {
        let types = [
            KirValueType::Scalar(MirType::Void),
            KirValueType::Scalar(MirType::Primitive(crate::MirPrimitiveTypeName::I32)),
        ];
        for ids in [
            vec![],
            vec![0, 1, 2, 3],
            vec![u32::MAX - 2, u32::MAX - 1, u32::MAX],
            vec![0, u32::MAX],
        ] {
            let mut table =
                ValueTable::new(ids.iter().copied().map(ValueId::from_index), ids.len());
            assert!(table.dense.len() <= ids.len().saturating_mul(4));
            if ids == [0, u32::MAX] {
                assert!(
                    table.dense.is_empty(),
                    "a sparse identity must not size a dense allocation"
                );
            }
            let mut old_definitions = HashMap::new();
            let mut old_types = HashMap::new();
            // Simulate a collision with a value already defined in another function.
            let mut module_ids = HashSet::from([ValueId::from_index(1)]);
            for round in 0..3 {
                for &id in ids.iter().chain(&[1, 8, u32::MAX]) {
                    let value = ValueId::from_index(id);
                    let definition = match round {
                        0 => ValueDefinition::FunctionParam,
                        1 => ValueDefinition::BlockParam(BlockId::from_index(7)),
                        _ => ValueDefinition::Instruction(BlockId::from_index(9), 3),
                    };
                    let fresh = module_ids.insert(value);
                    let previous = if fresh {
                        old_definitions.insert(value, definition)
                    } else {
                        None
                    };
                    old_types.insert(value, types[round % types.len()].clone());
                    let actual = table.record(
                        value,
                        &types[round % types.len()],
                        fresh.then_some(definition),
                    );
                    assert_eq!(actual, previous);
                    for probe in ids.iter().copied().chain([0, 1, 7, 8, 9, u32::MAX]) {
                        let probe = ValueId::from_index(probe);
                        assert_eq!(table.type_of(probe), old_types.get(&probe));
                        assert_eq!(
                            table.definition(probe),
                            old_definitions.get(&probe).copied()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn operand_visitor_should_match_legacy_collection_for_every_instruction_and_place() {
        let a = ValueId::from_index(1);
        let b = ValueId::from_index(7);
        let c = ValueId::from_index(u32::MAX);
        let region = MemoryRegionId::from_index(0);
        let ty = MirType::Primitive(crate::MirPrimitiveTypeName::I32);
        let leaf = KirPlace::Value {
            value: a,
            type_node: ty.clone(),
            region,
        };
        let indexed = KirPlace::Index {
            base: Box::new(leaf.clone()),
            index: b,
            type_node: ty.clone(),
            region,
        };
        let places = [
            leaf,
            KirPlace::Deref {
                pointer: c,
                type_node: ty.clone(),
                region,
            },
            indexed.clone(),
            KirPlace::SliceIndex {
                slice: a,
                index: b,
                type_node: ty.clone(),
                region,
            },
            KirPlace::Field {
                base: Box::new(indexed),
                field_name: "x".into(),
                type_node: ty.clone(),
                region,
            },
        ];
        let mut kinds = vec![
            KirInstructionKind::Undef { slot: "x".into() },
            KirInstructionKind::ConstInt { value: "1".into() },
            KirInstructionKind::ConstFloat {
                value: "1.5".into(),
            },
            KirInstructionKind::ConstBool { value: true },
            KirInstructionKind::Copy { value: a },
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: a,
                right: b,
                semantics: KirArithmeticSemantics::Modular,
            },
            KirInstructionKind::Unary {
                op: MirUnaryOp::Neg,
                operand: c,
                semantics: KirArithmeticSemantics::Checked,
            },
            KirInstructionKind::Compare {
                op: crate::MirCompareOp::Lt,
                left: a,
                right: b,
            },
            KirInstructionKind::Cast {
                op: crate::MirCastOp::I32ToF64,
                value: a,
            },
            KirInstructionKind::CheckCondition {
                kind: KirCheckConditionKind::InvalidSubslice,
                args: vec![a, b, c, a],
            },
            KirInstructionKind::Guard {
                condition: b,
                failure: KirFailureKind::OutOfBounds,
            },
            KirInstructionKind::MakeSlice { data: a, len: b },
            KirInstructionKind::SliceData { slice: a },
            KirInstructionKind::SliceLen { slice: c },
            KirInstructionKind::Subslice {
                slice: a,
                start: b,
                end: c,
            },
            KirInstructionKind::Call {
                function_name: "callee".into(),
                args: vec![a, b, c, a],
            },
            KirInstructionKind::RuntimeCall {
                intrinsic: crate::MirRuntimeIntrinsic::PrintI32,
                args: vec![a],
            },
            KirInstructionKind::RuntimeCall {
                intrinsic: crate::MirRuntimeIntrinsic::PrintNewline,
                args: vec![],
            },
        ];
        for place in places {
            kinds.push(KirInstructionKind::Address {
                place: Box::new(place.clone()),
            });
            kinds.push(KirInstructionKind::Load {
                place: Box::new(place.clone()),
            });
            kinds.push(KirInstructionKind::Store {
                place: Box::new(place),
                value: c,
            });
        }
        for kind in kinds {
            let instruction = KirInstruction {
                id: InstructionId::from_index(0),
                results: vec![],
                kind,
                memory: None,
                effect: None,
            };
            let mut actual = Vec::new();
            visit_instruction_uses(&instruction, |value| actual.push(value));
            assert_eq!(
                actual,
                instruction_uses(&instruction),
                "{:?}",
                instruction.kind
            );
        }
    }

    #[test]
    fn terminator_visitors_should_preserve_order_and_parallel_edges() {
        let a = ValueId::from_index(1);
        let b = ValueId::from_index(7);
        let edge = KirEdge {
            target: BlockId::from_index(4),
            args: vec![a, b, a],
            memory_args: vec![],
        };
        let cases = [
            (
                KirTerminator::Return {
                    value: None,
                    memory: vec![],
                    effect_order: 0,
                },
                vec![],
                0,
            ),
            (
                KirTerminator::Return {
                    value: Some(a),
                    memory: vec![],
                    effect_order: 0,
                },
                vec![a],
                0,
            ),
            (KirTerminator::Jump { edge: edge.clone() }, vec![a, b, a], 1),
            (
                KirTerminator::Branch {
                    condition: b,
                    then_edge: edge.clone(),
                    else_edge: edge,
                },
                vec![b, a, b, a, a, b, a],
                2,
            ),
        ];
        for (terminator, expected, count) in cases {
            assert_eq!(terminator_uses(&terminator).collect::<Vec<_>>(), expected);
            assert_eq!(terminator_edges(&terminator).count(), count);
        }
    }
}
