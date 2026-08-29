use std::collections::{BTreeMap, BTreeSet};

use crate::{MirBinaryOp, MirType, MirUnaryOp};

use super::*;

#[derive(Debug, Clone, Copy)]
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

#[must_use]
pub fn validate_kir_module(module: &KirModule) -> KirValidationResult {
    let mut errors = Vec::new();
    let mut function_ids = BTreeSet::new();
    let mut block_ids = BTreeSet::new();
    let mut instruction_ids = BTreeSet::new();
    let mut module_value_ids = BTreeSet::new();
    let mut module_region_ids = BTreeSet::new();
    let mut module_memory_ids = BTreeSet::new();

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
    module_block_ids: &mut BTreeSet<BlockId>,
    module_instruction_ids: &mut BTreeSet<InstructionId>,
    module_value_ids: &mut BTreeSet<ValueId>,
    module_region_ids: &mut BTreeSet<MemoryRegionId>,
    module_memory_ids: &mut BTreeSet<MemoryVersionId>,
    errors: &mut Vec<KirValidationError>,
) {
    let mut definitions = BTreeMap::new();
    let mut types = BTreeMap::new();
    let mut memory_definitions = BTreeMap::new();
    let mut memory_regions = BTreeMap::new();
    let mut function_region_ids = BTreeSet::new();
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
            &mut definitions,
            &mut types,
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
                &mut definitions,
                &mut types,
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
                    &mut definitions,
                    &mut types,
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
        .collect::<BTreeMap<_, _>>();
    let dominators = compute_kir_dominators(function);
    let mut effect_orders = BTreeSet::new();
    for block in &function.blocks {
        let mut previous_effect_order = None;
        for (index, instruction) in block.instructions.iter().enumerate() {
            for used in instruction_uses(instruction) {
                validate_use(
                    function,
                    block.id,
                    index,
                    used,
                    Some(instruction.id),
                    &definitions,
                    &dominators,
                    errors,
                );
            }
            validate_instruction_structure(function, block, index, &types, errors);
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
                &definitions,
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
                &types,
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
    types: &BTreeMap<ValueId, MirType>,
    errors: &mut Vec<KirValidationError>,
) {
    let instruction = &block.instructions[index];
    if let KirInstructionKind::Binary { left, right, .. } = instruction.kind {
        let result_type = instruction.results.first().map(|result| &result.type_node);
        if types.get(&left) != result_type || types.get(&right) != result_type {
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
        if !types.get(&condition).is_some_and(|type_node| {
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
    module_memory_ids: &mut BTreeSet<MemoryVersionId>,
    definitions: &mut BTreeMap<MemoryVersionId, MemoryDefinition>,
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
    type_node: &MirType,
    definition: ValueDefinition,
    block: Option<BlockId>,
    instruction: Option<InstructionId>,
    module_value_ids: &mut BTreeSet<ValueId>,
    definitions: &mut BTreeMap<ValueId, ValueDefinition>,
    types: &mut BTreeMap<ValueId, MirType>,
    errors: &mut Vec<KirValidationError>,
) {
    if !module_value_ids.insert(value) || definitions.insert(value, definition).is_some() {
        errors.push(error(
            "duplicate value definition",
            Some(function.id),
            block,
            instruction,
        ));
    }
    types.insert(value, type_node.clone());
}

#[allow(clippy::too_many_arguments)]
fn validate_use(
    function: &KirFunction,
    use_block: BlockId,
    use_index: usize,
    value: ValueId,
    instruction: Option<InstructionId>,
    definitions: &BTreeMap<ValueId, ValueDefinition>,
    dominators: &KirDominators,
    errors: &mut Vec<KirValidationError>,
) {
    let Some(definition) = definitions.get(&value) else {
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
        ValueDefinition::BlockParam(def_block) => dominators.dominates(*def_block, use_block),
        ValueDefinition::Instruction(def_block, def_index) => {
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
    definitions: &BTreeMap<MemoryVersionId, MemoryDefinition>,
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
        MemoryDefinition::BlockParam(def_block) => dominators.dominates(*def_block, use_block),
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
    blocks: &BTreeMap<BlockId, &KirBlock>,
    types: &BTreeMap<ValueId, MirType>,
    memory_regions: &BTreeMap<MemoryVersionId, MemoryRegionId>,
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
        if let Some(argument_type) = types.get(argument)
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

fn terminator_uses(terminator: &KirTerminator) -> Vec<ValueId> {
    let mut values = match terminator {
        KirTerminator::Return { value, .. } => value.iter().copied().collect(),
        KirTerminator::Jump { .. } => Vec::new(),
        KirTerminator::Branch { condition, .. } => vec![*condition],
    };
    for edge in terminator_edges(terminator) {
        values.extend(edge.args.iter().copied());
    }
    values
}

fn terminator_edges(terminator: &KirTerminator) -> Vec<&KirEdge> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
    }
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
