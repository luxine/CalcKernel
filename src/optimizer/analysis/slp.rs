use std::collections::BTreeSet;

use crate::{
    BlockId, CandidateKey, FunctionId, InstructionId, KirArithmeticSemantics, KirCostSemantics,
    KirInstruction, KirInstructionKind, KirLaneType, KirPlace, KirProfileOperation, KirValueType,
    MirBinaryOp, MirPrimitiveTypeName, MirType, ValueId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpCandidate {
    pub key: CandidateKey,
    pub function: FunctionId,
    pub block: BlockId,
    pub root: InstructionId,
    pub lanes: u16,
    pub lane_type: KirLaneType,
    pub semantics: KirCostSemantics,
    pub operation: KirProfileOperation,
    pub scalar_instructions: Vec<InstructionId>,
    pub left: Vec<ValueId>,
    pub right: Vec<ValueId>,
    pub results: Vec<ValueId>,
    pub memory: Option<SlpMemoryCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpMemoryCandidate {
    pub left_loads: Vec<InstructionId>,
    pub right_loads: Vec<InstructionId>,
    pub stores: Vec<InstructionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlpFallback {
    pub function: FunctionId,
    pub block: BlockId,
    pub root: Option<InstructionId>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlpDiscovery {
    pub candidates: Vec<SlpCandidate>,
    pub fallbacks: Vec<SlpFallback>,
}

#[must_use]
pub fn discover_slp_candidates(
    function: &crate::KirFunction,
    certificate_dependencies: &BTreeSet<InstructionId>,
) -> SlpDiscovery {
    let mut result = SlpDiscovery::default();
    for block in &function.blocks {
        let mut run = Vec::<SlpScalarOperation>::new();
        for instruction in &block.instructions {
            if is_slp_barrier(instruction, certificate_dependencies) {
                flush_run(function.id, block.id, &mut run, &mut result);
                result.fallbacks.push(SlpFallback {
                    function: function.id,
                    block: block.id,
                    root: Some(instruction.id),
                    reason: "slp-barrier".to_string(),
                });
                continue;
            }
            if let Some(operation) = scalar_operation(instruction) {
                if run.last().is_some_and(|previous| {
                    previous.operation != operation.operation
                        || previous.lane_type != operation.lane_type
                        || previous.semantics != operation.semantics
                }) {
                    flush_run(function.id, block.id, &mut run, &mut result);
                }
                run.push(operation);
            }
        }
        flush_run(function.id, block.id, &mut run, &mut result);
        discover_memory_candidates(function, block, certificate_dependencies, &mut result);
    }
    result
        .candidates
        .sort_by(|left, right| left.key.cmp(&right.key));
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.block, left.root, &left.reason).cmp(&(
            right.function,
            right.block,
            right.root,
            &right.reason,
        ))
    });
    result.fallbacks.dedup();
    result
}

#[derive(Debug, Clone)]
struct SlpMemoryLane {
    operation: SlpScalarOperation,
    left_load: InstructionId,
    right_load: InstructionId,
    store: InstructionId,
    left_slice: ValueId,
    right_slice: ValueId,
    output_slice: ValueId,
    index: u32,
}

fn discover_memory_candidates(
    function: &crate::KirFunction,
    block: &crate::KirBlock,
    certificate_dependencies: &BTreeSet<InstructionId>,
    result: &mut SlpDiscovery,
) {
    let constants = block
        .instructions
        .iter()
        .filter_map(|instruction| {
            let KirInstructionKind::ConstInt { value } = &instruction.kind else {
                return None;
            };
            Some((
                instruction.results.first()?.value,
                value.parse::<u32>().ok()?,
            ))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let definition = |value: ValueId| {
        block.instructions.iter().find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
    };
    let mut lanes = Vec::new();
    for instruction in &block.instructions {
        let Some(operation) = scalar_operation(instruction) else {
            continue;
        };
        let (Some(left), Some(right)) = (definition(operation.left), definition(operation.right))
        else {
            continue;
        };
        let (Some((left_slice, left_index)), Some((right_slice, right_index))) =
            (load_slice_index(left), load_slice_index(right))
        else {
            continue;
        };
        let Some(store) = block.instructions.iter().find(|candidate| {
            matches!(candidate.kind, KirInstructionKind::Store { value, .. } if value == operation.result)
        }) else {
            continue;
        };
        let Some((output_slice, output_index)) = store_slice_index(store) else {
            continue;
        };
        let (Some(left_index), Some(right_index), Some(output_index)) = (
            constants.get(&left_index).copied(),
            constants.get(&right_index).copied(),
            constants.get(&output_index).copied(),
        ) else {
            continue;
        };
        if left_index != right_index
            || left_index != output_index
            || left_slice == right_slice
            || left_slice == output_slice
            || right_slice == output_slice
        {
            continue;
        }
        lanes.push(SlpMemoryLane {
            operation,
            left_load: left.id,
            right_load: right.id,
            store: store.id,
            left_slice,
            right_slice,
            output_slice,
            index: left_index,
        });
    }
    let mut offset = 0_usize;
    while lanes.len().saturating_sub(offset) >= 2 {
        let remaining = lanes.len() - offset;
        if remaining >= 4 {
            push_memory_group(
                function,
                block,
                &lanes[offset..offset + 4],
                certificate_dependencies,
                result,
            );
        }
        push_memory_group(
            function,
            block,
            &lanes[offset..offset + 2],
            certificate_dependencies,
            result,
        );
        offset += 2;
    }
}

fn push_memory_group(
    function: &crate::KirFunction,
    block: &crate::KirBlock,
    group: &[SlpMemoryLane],
    certificate_dependencies: &BTreeSet<InstructionId>,
    result: &mut SlpDiscovery,
) {
    let first = &group[0];
    if group.iter().enumerate().any(|(lane, item)| {
        item.operation.operation != first.operation.operation
            || item.operation.lane_type != first.operation.lane_type
            || item.operation.semantics != first.operation.semantics
            || item.left_slice != first.left_slice
            || item.right_slice != first.right_slice
            || item.output_slice != first.output_slice
            || item.index
                != first
                    .index
                    .saturating_add(u32::try_from(lane).unwrap_or(u32::MAX))
    }) {
        return;
    }
    let selected = group
        .iter()
        .flat_map(|lane| {
            [
                lane.left_load,
                lane.right_load,
                lane.operation.instruction,
                lane.store,
            ]
        })
        .collect::<BTreeSet<_>>();
    if selected
        .iter()
        .any(|instruction| certificate_dependencies.contains(instruction))
    {
        return;
    }
    let positions = selected
        .iter()
        .filter_map(|id| {
            block
                .instructions
                .iter()
                .position(|instruction| instruction.id == *id)
        })
        .collect::<Vec<_>>();
    let (Some(first_position), Some(last_position)) = (
        positions.iter().min().copied(),
        positions.iter().max().copied(),
    ) else {
        return;
    };
    if block.instructions[first_position..=last_position]
        .iter()
        .any(|instruction| {
            (instruction.memory.is_some() || instruction.effect.is_some())
                && !selected.contains(&instruction.id)
        })
    {
        return;
    }
    let lanes = u16::try_from(group.len()).expect("closed SLP memory lanes fit u16");
    result.candidates.push(SlpCandidate {
        key: CandidateKey::ResidualSlp {
            function: function.id,
            block: block.id,
            root: first.operation.instruction,
            lanes,
        },
        function: function.id,
        block: block.id,
        root: first.operation.instruction,
        lanes,
        lane_type: first.operation.lane_type,
        semantics: first.operation.semantics,
        operation: first.operation.operation,
        scalar_instructions: group
            .iter()
            .map(|lane| lane.operation.instruction)
            .collect(),
        left: group.iter().map(|lane| lane.operation.left).collect(),
        right: group.iter().map(|lane| lane.operation.right).collect(),
        results: group.iter().map(|lane| lane.operation.result).collect(),
        memory: Some(SlpMemoryCandidate {
            left_loads: group.iter().map(|lane| lane.left_load).collect(),
            right_loads: group.iter().map(|lane| lane.right_load).collect(),
            stores: group.iter().map(|lane| lane.store).collect(),
        }),
    });
}

fn load_slice_index(instruction: &KirInstruction) -> Option<(ValueId, ValueId)> {
    let KirInstructionKind::Load { place } = &instruction.kind else {
        return None;
    };
    slice_index(place)
}

fn store_slice_index(instruction: &KirInstruction) -> Option<(ValueId, ValueId)> {
    let KirInstructionKind::Store { place, .. } = &instruction.kind else {
        return None;
    };
    slice_index(place)
}

fn slice_index(place: &KirPlace) -> Option<(ValueId, ValueId)> {
    let KirPlace::SliceIndex { slice, index, .. } = place else {
        return None;
    };
    Some((*slice, *index))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlpScalarOperation {
    instruction: InstructionId,
    operation: KirProfileOperation,
    lane_type: KirLaneType,
    semantics: KirCostSemantics,
    left: ValueId,
    right: ValueId,
    result: ValueId,
}

fn flush_run(
    function: FunctionId,
    block: BlockId,
    run: &mut Vec<SlpScalarOperation>,
    result: &mut SlpDiscovery,
) {
    let mut offset = 0_usize;
    while run.len().saturating_sub(offset) >= 2 {
        let remaining = run.len() - offset;
        if remaining >= 4 {
            push_group(function, block, &run[offset..offset + 4], result);
        }
        push_group(function, block, &run[offset..offset + 2], result);
        offset += 2;
    }
    run.clear();
}

fn push_group(
    function: FunctionId,
    block: BlockId,
    group: &[SlpScalarOperation],
    result: &mut SlpDiscovery,
) {
    let definitions = group
        .iter()
        .map(|item| item.result)
        .collect::<BTreeSet<_>>();
    if group
        .iter()
        .any(|item| definitions.contains(&item.left) || definitions.contains(&item.right))
    {
        result.fallbacks.push(SlpFallback {
            function,
            block,
            root: Some(group[0].instruction),
            reason: "slp-lane-dependence".to_string(),
        });
        return;
    }
    let lanes = u16::try_from(group.len()).expect("closed SLP lane counts fit u16");
    result.candidates.push(SlpCandidate {
        key: CandidateKey::ResidualSlp {
            function,
            block,
            root: group[0].instruction,
            lanes,
        },
        function,
        block,
        root: group[0].instruction,
        lanes,
        lane_type: group[0].lane_type,
        semantics: group[0].semantics,
        operation: group[0].operation,
        scalar_instructions: group.iter().map(|item| item.instruction).collect(),
        left: group.iter().map(|item| item.left).collect(),
        right: group.iter().map(|item| item.right).collect(),
        results: group.iter().map(|item| item.result).collect(),
        memory: None,
    });
}

fn scalar_operation(instruction: &KirInstruction) -> Option<SlpScalarOperation> {
    if instruction.memory.is_some()
        || instruction.effect.is_some()
        || instruction.results.len() != 1
    {
        return None;
    }
    let KirInstructionKind::Binary {
        op,
        left,
        right,
        semantics,
    } = instruction.kind
    else {
        return None;
    };
    if semantics == KirArithmeticSemantics::Checked
        || op == MirBinaryOp::Mod
        || (op == MirBinaryOp::Div && semantics != KirArithmeticSemantics::StrictFloat)
    {
        return None;
    }
    let lane_type = lane_type(&instruction.results[0].type_node)?;
    let operation = match op {
        MirBinaryOp::Add => KirProfileOperation::Add,
        MirBinaryOp::Sub => KirProfileOperation::Subtract,
        MirBinaryOp::Mul => KirProfileOperation::Multiply,
        MirBinaryOp::Div => KirProfileOperation::Divide,
        MirBinaryOp::Mod => return None,
    };
    let semantics = match semantics {
        KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
        KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
        KirArithmeticSemantics::Checked => return None,
    };
    Some(SlpScalarOperation {
        instruction: instruction.id,
        operation,
        lane_type,
        semantics,
        left,
        right,
        result: instruction.results[0].value,
    })
}

#[must_use]
pub(crate) fn is_slp_barrier(
    instruction: &KirInstruction,
    dependencies: &BTreeSet<InstructionId>,
) -> bool {
    dependencies.contains(&instruction.id)
        || instruction.effect.is_some()
        || instruction.memory.is_some()
        || matches!(
            instruction.kind,
            KirInstructionKind::Guard { .. }
                | KirInstructionKind::CheckCondition { .. }
                | KirInstructionKind::Call { .. }
                | KirInstructionKind::RuntimeCall { .. }
                | KirInstructionKind::Load { .. }
                | KirInstructionKind::Store { .. }
                | KirInstructionKind::VectorSplat { .. }
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
        )
}

fn lane_type(type_node: &KirValueType) -> Option<KirLaneType> {
    match type_node.as_scalar()? {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(KirLaneType::I64),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some(KirLaneType::U64),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        MirType::Primitive(MirPrimitiveTypeName::Bool)
        | MirType::Pointer(_)
        | MirType::Slice(_)
        | MirType::Struct(_)
        | MirType::Void => None,
    }
}
