use std::collections::{HashMap, HashSet};

use crate::{MirBinaryOp, MirType, MirUnaryOp};

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

#[derive(Clone, Copy)]
struct ValueRecord<'a> {
    definition: Option<ValueDefinition>,
    type_node: &'a MirType,
}

/// Validation-local queries, rebuilt from the actual function on every check.
/// Dense storage is bounded by definition count, never by an untrusted max ID.
/// Sparse identities and any later inserts use the same exact lookup semantics.
struct ValueTable<'a> {
    base: u32,
    dense: Vec<Option<ValueRecord<'a>>>,
    sparse: HashMap<ValueId, ValueRecord<'a>>,
}

impl<'a> ValueTable<'a> {
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

    fn get(&self, value: ValueId) -> Option<&ValueRecord<'a>> {
        self.dense_index(value)
            .and_then(|index| self.dense[index].as_ref())
            .or_else(|| self.sparse.get(&value))
    }

    fn definition(&self, value: ValueId) -> Option<ValueDefinition> {
        self.get(value).and_then(|record| record.definition)
    }

    fn type_of(&self, value: ValueId) -> Option<&'a MirType> {
        self.get(value).map(|record| record.type_node)
    }

    fn record(
        &mut self,
        value: ValueId,
        type_node: &'a MirType,
        definition: Option<ValueDefinition>,
    ) -> Option<ValueDefinition> {
        let empty = ValueRecord {
            definition: None,
            type_node,
        };
        let record = if let Some(index) = self.dense_index(value) {
            self.dense[index].get_or_insert(empty)
        } else {
            self.sparse.entry(value).or_insert(empty)
        };
        // Legacy validation retains the first definition after a module-wide ID
        // collision, but still records the last type for subsequent diagnostics.
        record.type_node = type_node;
        definition.and_then(|definition| record.definition.replace(definition))
    }
}

#[must_use]
pub fn validate_kir_module(module: &KirModule) -> KirValidationResult {
    let mut errors = Vec::new();
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
        validate_function(
            function,
            &mut block_ids,
            &mut instruction_ids,
            &mut module_value_ids,
            &mut module_region_ids,
            &mut module_memory_ids,
            &mut errors,
        );
    }
    KirValidationResult { errors }
}

fn validate_function(
    function: &KirFunction,
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
            &param.type_node,
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
            validate_instruction_structure(function, block, index, &values, errors);
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
    block: &KirBlock,
    index: usize,
    values: &ValueTable<'_>,
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
                MirType::Primitive(crate::MirPrimitiveTypeName::Bool)
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
fn define_value<'a>(
    function: &KirFunction,
    value: ValueId,
    type_node: &'a MirType,
    definition: ValueDefinition,
    block: Option<BlockId>,
    instruction: Option<InstructionId>,
    module_value_ids: &mut HashSet<ValueId>,
    values: &mut ValueTable<'a>,
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
    values: &ValueTable<'_>,
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
    values: &ValueTable<'_>,
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
                &MirType::Void,
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
            MirType::Void,
            MirType::Primitive(crate::MirPrimitiveTypeName::I32),
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
                    old_types.insert(value, &types[round % types.len()]);
                    let actual = table.record(
                        value,
                        &types[round % types.len()],
                        fresh.then_some(definition),
                    );
                    assert_eq!(actual, previous);
                    for probe in ids.iter().copied().chain([0, 1, 7, 8, 9, u32::MAX]) {
                        let probe = ValueId::from_index(probe);
                        assert_eq!(table.type_of(probe), old_types.get(&probe).copied());
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
