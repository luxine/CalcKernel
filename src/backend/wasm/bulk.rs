use crate::{
    KirArithmeticSemantics, KirBlock, KirFunction, KirInstructionKind, KirPlace, KirTerminator,
    KirValueType, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, ValueId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WasmBulkKind {
    Copy,
    Fill { byte: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WasmBulkProposal {
    kind: WasmBulkKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CheckedWasmBulkMemory {
    pub kind: WasmBulkKind,
    pub destination: ValueId,
    pub source: Option<ValueId>,
    pub start: ValueId,
    pub end: ValueId,
    pub element_bytes: u32,
}

/// Makes a deliberately loose proposal. The checker below owns all structural and type checks.
pub(super) fn propose_wasm_bulk_memory(function: &KirFunction) -> Option<WasmBulkProposal> {
    if function.return_type != MirType::Void || function.blocks.len() != 4 {
        return None;
    }

    let has_indexed_store = function.blocks.iter().any(|block| {
        block.instructions.iter().any(|instruction| {
            matches!(
                &instruction.kind,
                KirInstructionKind::Store { place, .. }
                    if matches!(place.as_ref(), KirPlace::Index { .. })
            )
        })
    });
    if !has_indexed_store {
        return None;
    }

    let kind = if function.blocks.iter().any(|block| {
        block.instructions.iter().any(|instruction| {
            matches!(
                &instruction.kind,
                KirInstructionKind::Load { place }
                    if matches!(place.as_ref(), KirPlace::Index { .. })
            )
        })
    }) {
        WasmBulkKind::Copy
    } else {
        WasmBulkKind::Fill { byte: 0 }
    };
    Some(WasmBulkProposal { kind })
}

/// Independently validates a proposal against the complete KIR function and builds its checked form.
pub(super) fn check_wasm_bulk_memory_candidate(
    function: &KirFunction,
    proposal: WasmBulkProposal,
) -> Option<CheckedWasmBulkMemory> {
    if function.return_type != MirType::Void || function.blocks.len() != 4 {
        return None;
    }

    let entry = function.blocks.first()?;
    if !entry.params.is_empty() {
        return None;
    }
    let KirTerminator::Jump { edge: entry_edge } = &entry.terminator else {
        return None;
    };
    let header = find_block(function, entry_edge.target)?;
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    let body = find_block(function, then_edge.target)?;
    let exit = find_block(function, else_edge.target)?;
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return None;
    };

    if body.id == header.id
        || exit.id == header.id
        || body.id == exit.id
        || backedge.target != header.id
        || function.blocks.iter().any(|block| {
            block.id != entry.id
                && block.id != header.id
                && block.id != body.id
                && block.id != exit.id
        })
        || !exit.instructions.is_empty()
        || !matches!(exit.terminator, KirTerminator::Return { value: None, .. })
        || entry_edge.args.len() != header.params.len()
        || then_edge.args.len() != header.params.len()
        || !else_edge.args.is_empty()
        || backedge.args.len() != header.params.len()
        || body.params.len() != header.params.len()
        || then_edge.args
            != header
                .params
                .iter()
                .map(|param| param.value)
                .collect::<Vec<_>>()
    {
        return None;
    }

    let is_copy = matches!(proposal.kind, WasmBulkKind::Copy);
    let expected_param_count = if is_copy { 4 } else { 3 };
    if header.params.len() != expected_param_count
        || function.params.len() != expected_param_count
        || entry.instructions.len() != if is_copy { 1 } else { 2 }
        || header.instructions.len() != 1
    {
        return None;
    }

    let compare = &header.instructions[0];
    let [compare_result] = compare.results.as_slice() else {
        return None;
    };
    let KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left: index,
        right: end_param,
    } = compare.kind
    else {
        return None;
    };
    if compare_result.value != *condition
        || scalar_type(function, compare_result.value)
            != Some(&MirType::Primitive(MirPrimitiveTypeName::Bool))
    {
        return None;
    }

    let index_slot = header
        .params
        .iter()
        .position(|param| param.value == index)?;
    let end_slot = header
        .params
        .iter()
        .position(|param| param.value == end_param)?;
    if index_slot == end_slot
        || scalar_type(function, index) != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        || scalar_type(function, end_param) != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
    {
        return None;
    }

    let destination_slot;
    let source_slot;
    let element_type;
    let store_value;
    let increment_value;
    let fill_byte;

    if is_copy {
        if body.instructions.len() != 3 {
            return None;
        }
        let load = body
            .instructions
            .iter()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))?;
        let store = body
            .instructions
            .iter()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))?;
        let load_position = body
            .instructions
            .iter()
            .position(|instruction| std::ptr::eq(instruction, load))?;
        let store_position = body
            .instructions
            .iter()
            .position(|instruction| std::ptr::eq(instruction, store))?;
        let increment = body
            .instructions
            .iter()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))?;
        if load_position >= store_position {
            return None;
        }
        let [loaded] = load.results.as_slice() else {
            return None;
        };
        let KirInstructionKind::Load {
            place: source_place,
        } = &load.kind
        else {
            return None;
        };
        let KirInstructionKind::Store {
            place: destination_place,
            value,
        } = &store.kind
        else {
            return None;
        };
        if *value != loaded.value {
            return None;
        }
        let (source_pointer_slot, source_index, source_element) =
            indexed_pointer_slot(source_place, &body.params)?;
        let (destination_pointer_slot, destination_index, destination_element) =
            indexed_pointer_slot(destination_place, &body.params)?;
        if source_index != body.params[index_slot].value
            || destination_index != body.params[index_slot].value
            || source_element != destination_element
            || !is_supported_word(&source_element)
            || scalar_type(function, loaded.value) != Some(&source_element)
        {
            return None;
        }
        destination_slot = destination_pointer_slot;
        source_slot = Some(source_pointer_slot);
        element_type = source_element;
        store_value = None;
        fill_byte = None;
        increment_value = increment;
    } else {
        if body.instructions.len() != 2 {
            return None;
        }
        let store = body
            .instructions
            .iter()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))?;
        let increment = body
            .instructions
            .iter()
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))?;
        let KirInstructionKind::Store { place, value } = &store.kind else {
            return None;
        };
        let (destination_pointer_slot, store_index, destination_element) =
            indexed_pointer_slot(place, &body.params)?;
        if store_index != body.params[index_slot].value || !is_supported_word(&destination_element)
        {
            return None;
        }
        destination_slot = destination_pointer_slot;
        source_slot = None;
        element_type = destination_element;
        store_value = Some(*value);
        fill_byte = Some(*value);
        increment_value = increment;
    }

    let increment = increment_value;
    let [increment_result] = increment.results.as_slice() else {
        return None;
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left: increment_index,
        right: step,
        semantics: KirArithmeticSemantics::Modular,
    } = increment.kind
    else {
        return None;
    };
    if increment_index != body.params[index_slot].value
        || !entry.instructions.iter().any(|instruction| {
            matches!(
                &instruction.kind,
                KirInstructionKind::ConstInt { value }
                    if instruction.results.as_slice() == [
                        crate::KirResult {
                            value: step,
                            type_node: KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)),
                        }
                    ] && value == "1"
            )
        })
        || scalar_type(function, increment_result.value)
            != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
    {
        return None;
    }

    let mut original_params = Vec::with_capacity(header.params.len());
    for argument in &entry_edge.args {
        if !function.params.iter().any(|param| param.value == *argument) {
            return None;
        }
        original_params.push(*argument);
    }
    let destination = original_params[destination_slot];
    let source = source_slot.map(|slot| original_params[slot]);
    let start = original_params[index_slot];
    let end = original_params[end_slot];
    if destination == start || destination == end || start == end || source == Some(destination) {
        return None;
    }
    if !matches!(scalar_type(function, destination), Some(MirType::Pointer(pointee)) if pointee.as_ref() == &element_type)
        || !matches!(scalar_type(function, start), Some(MirType::Primitive(MirPrimitiveTypeName::U32)))
        || !matches!(scalar_type(function, end), Some(MirType::Primitive(MirPrimitiveTypeName::U32)))
        || source.is_some_and(|value| {
            !matches!(scalar_type(function, value), Some(MirType::Pointer(pointee)) if pointee.as_ref() == &element_type)
        })
    {
        return None;
    }

    for (slot, argument) in backedge.args.iter().enumerate() {
        let expected = if slot == index_slot {
            increment_result.value
        } else {
            body.params[slot].value
        };
        if *argument != expected {
            return None;
        }
    }

    let kind = if let Some(value) = store_value {
        let literal = entry.instructions.iter().find_map(|instruction| {
            let [result] = instruction.results.as_slice() else {
                return None;
            };
            if result.value != value
                || result.type_node != KirValueType::Scalar(element_type.clone())
            {
                return None;
            }
            let KirInstructionKind::ConstInt { value } = &instruction.kind else {
                return None;
            };
            Some(value.as_str())
        })?;
        let word = literal.parse::<i64>().ok()? as u32;
        let byte = word as u8;
        if word != u32::from_le_bytes([byte; 4]) {
            return None;
        }
        let checked_byte = fill_byte?;
        if checked_byte != value {
            return None;
        }
        WasmBulkKind::Fill { byte }
    } else {
        if proposal.kind != WasmBulkKind::Copy {
            return None;
        }
        WasmBulkKind::Copy
    };

    let actual_proposal = match kind {
        WasmBulkKind::Copy => WasmBulkKind::Copy,
        WasmBulkKind::Fill { .. } => WasmBulkKind::Fill { byte: 0 },
    };
    if proposal.kind != actual_proposal {
        return None;
    }

    Some(CheckedWasmBulkMemory {
        kind,
        destination,
        source,
        start,
        end,
        element_bytes: 4,
    })
}

pub(super) fn checked_wasm_bulk_memory_candidate(
    function: &KirFunction,
) -> Option<CheckedWasmBulkMemory> {
    let proposal = propose_wasm_bulk_memory(function)?;
    check_wasm_bulk_memory_candidate(function, proposal)
}

fn find_block(function: &KirFunction, id: crate::BlockId) -> Option<&KirBlock> {
    function.blocks.iter().find(|block| block.id == id)
}

fn scalar_type(function: &KirFunction, value: ValueId) -> Option<&MirType> {
    for param in &function.params {
        if param.value == value {
            return Some(&param.type_node);
        }
    }
    for block in &function.blocks {
        for param in &block.params {
            if param.value == value {
                return param.type_node.as_scalar();
            }
        }
        for instruction in &block.instructions {
            for result in &instruction.results {
                if result.value == value {
                    return result.type_node.as_scalar();
                }
            }
        }
    }
    None
}

fn indexed_pointer_slot(
    place: &KirPlace,
    block_params: &[crate::KirBlockParam],
) -> Option<(usize, ValueId, MirType)> {
    let KirPlace::Index {
        base,
        index,
        type_node,
        ..
    } = place
    else {
        return None;
    };
    let KirPlace::Value {
        value: pointer,
        type_node: pointer_type,
        ..
    } = base.as_ref()
    else {
        return None;
    };
    let MirType::Pointer(pointee) = pointer_type else {
        return None;
    };
    if pointee.as_ref() != type_node {
        return None;
    }
    let slot = block_params
        .iter()
        .position(|param| param.value == *pointer)?;
    Some((slot, *index, type_node.clone()))
}

fn is_supported_word(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
    )
}
