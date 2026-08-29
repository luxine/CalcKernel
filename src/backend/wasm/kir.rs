use std::collections::{BTreeMap, BTreeSet};

use crate::*;

use super::{EmitWasmOptions, emit_wasm_module_with_options, emit_wat_module_with_options};

pub fn emit_wat_kir_module(module: &KirModule, options: EmitWasmOptions) -> Result<String, String> {
    let mir = adapt_unchecked_kir(module)?;
    Ok(emit_wat_module_with_options(&mir, options))
}

pub fn emit_wasm_kir_module(
    module: &KirModule,
    options: EmitWasmOptions,
) -> Result<Vec<u8>, String> {
    let mir = adapt_unchecked_kir(module)?;
    emit_wasm_module_with_options(&mir, options)
}

fn adapt_unchecked_kir(module: &KirModule) -> Result<MirModule, String> {
    if module.config.overflow_mode != KirOverflowMode::Unchecked
        || module.config.bounds_mode != KirBoundsMode::Unchecked
    {
        return Err("WebAssembly KIR backend accepts only unchecked KIR".to_string());
    }
    Ok(MirModule {
        entry: module.entry.clone(),
        structs: module.structs.clone(),
        functions: module
            .functions
            .iter()
            .map(adapt_function)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn adapt_function(function: &KirFunction) -> Result<MirFunction, String> {
    let types = value_types(function);
    let function_params = function
        .params
        .iter()
        .map(|param| (param.value, (param.name.clone(), param.type_node.clone())))
        .collect::<BTreeMap<_, _>>();
    let local_values = function
        .blocks
        .iter()
        .flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        })
        .collect::<BTreeSet<_>>();
    let locals = local_values
        .iter()
        .map(|value| MirLocal {
            name: local_name(*value),
            type_node: types[value].clone(),
        })
        .collect();
    let mut blocks = Vec::new();
    let mut edge_blocks = Vec::new();
    for block in &function.blocks {
        let mut instructions = Vec::new();
        for instruction in &block.instructions {
            instructions.extend(adapt_instruction(instruction, &types, &function_params)?);
        }
        let terminator =
            adapt_terminator(function, block, &types, &function_params, &mut edge_blocks);
        blocks.push(MirBlock {
            label: block_label(block.id),
            instructions,
            terminator,
        });
    }
    blocks.extend(edge_blocks);
    Ok(MirFunction {
        name: function.name.clone(),
        exported: function.exported,
        params: function
            .params
            .iter()
            .map(|param| MirParam {
                name: param.name.clone(),
                type_node: param.type_node.clone(),
            })
            .collect(),
        return_type: function.return_type.clone(),
        locals,
        blocks,
    })
}

fn adapt_instruction(
    instruction: &KirInstruction,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
) -> Result<Vec<MirInstruction>, String> {
    let result = |index: usize| mir_value(instruction.results[index].value, types, params);
    let value = |value: ValueId| mir_value(value, types, params);
    Ok(match &instruction.kind {
        KirInstructionKind::Undef { .. } => Vec::new(),
        KirInstructionKind::ConstInt { value: constant } => vec![MirInstruction::ConstInt {
            target: result(0),
            value: constant.clone(),
        }],
        KirInstructionKind::ConstFloat { value: constant } => vec![MirInstruction::ConstFloat {
            target: result(0),
            value: constant.clone(),
        }],
        KirInstructionKind::ConstBool { value: constant } => vec![MirInstruction::ConstBool {
            target: result(0),
            value: *constant,
        }],
        KirInstructionKind::Copy { value: source } => vec![MirInstruction::Move {
            target: result(0),
            value: value(*source),
        }],
        KirInstructionKind::Binary {
            op, left, right, ..
        } => vec![MirInstruction::Binary {
            target: result(0),
            op: *op,
            left: value(*left),
            right: value(*right),
        }],
        KirInstructionKind::Unary { op, operand, .. } => vec![MirInstruction::Unary {
            target: result(0),
            op: *op,
            operand: value(*operand),
        }],
        KirInstructionKind::Compare { op, left, right } => vec![MirInstruction::Compare {
            target: result(0),
            op: *op,
            left: value(*left),
            right: value(*right),
        }],
        KirInstructionKind::Cast { op, value: source } => vec![MirInstruction::Cast {
            target: result(0),
            op: *op,
            value: value(*source),
        }],
        KirInstructionKind::Address { place } => vec![MirInstruction::Address {
            target: result(0),
            place: adapt_place(place, types, params),
        }],
        KirInstructionKind::Load { place } => vec![MirInstruction::Load {
            target: result(0),
            place: adapt_place(place, types, params),
        }],
        KirInstructionKind::Store {
            place,
            value: source,
        } => vec![MirInstruction::Store {
            place: adapt_place(place, types, params),
            value: value(*source),
        }],
        KirInstructionKind::MakeSlice { data, len } => vec![MirInstruction::MakeSlice {
            target: result(0),
            data: value(*data),
            len: value(*len),
        }],
        KirInstructionKind::SliceData { slice } => vec![MirInstruction::SliceData {
            target: result(0),
            slice: value(*slice),
        }],
        KirInstructionKind::SliceLen { slice } => vec![MirInstruction::SliceLen {
            target: result(0),
            slice: value(*slice),
        }],
        KirInstructionKind::Subslice { slice, start, end } => vec![MirInstruction::Subslice {
            target: result(0),
            slice: value(*slice),
            start: value(*start),
            end: value(*end),
        }],
        KirInstructionKind::Call {
            function_name,
            args,
        } => vec![MirInstruction::Call {
            target: instruction.results.first().map(|_| result(0)),
            function_name: function_name.clone(),
            args: args.iter().map(|arg| value(*arg)).collect(),
        }],
        KirInstructionKind::CheckCondition { .. } | KirInstructionKind::Guard { .. } => {
            return Err("unchecked WebAssembly KIR contains a safety guard".to_string());
        }
        KirInstructionKind::RuntimeCall { .. } => {
            return Err("WebAssembly KIR cannot lower native runtime calls".to_string());
        }
    })
}

fn adapt_terminator(
    function: &KirFunction,
    block: &KirBlock,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
    edge_blocks: &mut Vec<MirBlock>,
) -> MirTerminator {
    match &block.terminator {
        KirTerminator::Return { value, .. } => MirTerminator::Return {
            value: value.map(|value| mir_value(value, types, params)),
        },
        KirTerminator::Jump { edge } => MirTerminator::Jump {
            label: append_edge_block(function, block.id, 0, edge, types, params, edge_blocks),
        },
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => MirTerminator::Branch {
            condition: mir_value(*condition, types, params),
            then_label: append_edge_block(
                function,
                block.id,
                0,
                then_edge,
                types,
                params,
                edge_blocks,
            ),
            else_label: append_edge_block(
                function,
                block.id,
                1,
                else_edge,
                types,
                params,
                edge_blocks,
            ),
        },
    }
}

fn append_edge_block(
    function: &KirFunction,
    source: BlockId,
    arm: u32,
    edge: &KirEdge,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
    blocks: &mut Vec<MirBlock>,
) -> String {
    let label = format!("edge_{}_{}_{}", source.index(), edge.target.index(), arm);
    let target = function
        .blocks
        .iter()
        .find(|block| block.id == edge.target)
        .expect("validated target");
    let mut instructions = Vec::new();
    for (index, (target, argument)) in target.params.iter().zip(&edge.args).enumerate() {
        instructions.push(MirInstruction::Move {
            target: MirValue::Temp {
                name: format!("edge_{}_{}", label, index),
                type_node: target.type_node.clone(),
            },
            value: mir_value(*argument, types, params),
        });
    }
    for (index, target) in target.params.iter().enumerate() {
        instructions.push(MirInstruction::Move {
            target: mir_value(target.value, types, params),
            value: MirValue::Temp {
                name: format!("edge_{}_{}", label, index),
                type_node: target.type_node.clone(),
            },
        });
    }
    blocks.push(MirBlock {
        label: label.clone(),
        instructions,
        terminator: MirTerminator::Jump {
            label: block_label(edge.target),
        },
    });
    label
}

fn adapt_place(
    place: &KirPlace,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
) -> MirPlace {
    match place {
        KirPlace::Value {
            value, type_node, ..
        } => params.get(value).map_or_else(
            || MirPlace::Local {
                name: local_name(*value),
                type_node: type_node.clone(),
            },
            |(name, _)| MirPlace::Param {
                name: name.clone(),
                type_node: type_node.clone(),
            },
        ),
        KirPlace::Deref {
            pointer, type_node, ..
        } => MirPlace::Deref {
            pointer: mir_value(*pointer, types, params),
            type_node: type_node.clone(),
        },
        KirPlace::Index {
            base,
            index,
            type_node,
            ..
        } => MirPlace::Index {
            base: Box::new(adapt_place(base, types, params)),
            index: mir_value(*index, types, params),
            type_node: type_node.clone(),
        },
        KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            ..
        } => MirPlace::SliceIndex {
            slice: mir_value(*slice, types, params),
            index: mir_value(*index, types, params),
            type_node: type_node.clone(),
        },
        KirPlace::Field {
            base,
            field_name,
            type_node,
            ..
        } => MirPlace::Field {
            base: Box::new(adapt_place(base, types, params)),
            field_name: field_name.clone(),
            type_node: type_node.clone(),
        },
    }
}

fn mir_value(
    value: ValueId,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
) -> MirValue {
    params.get(&value).map_or_else(
        || MirValue::Local {
            name: local_name(value),
            type_node: types[&value].clone(),
        },
        |(name, type_node)| MirValue::Param {
            name: name.clone(),
            type_node: type_node.clone(),
        },
    )
}

fn value_types(function: &KirFunction) -> BTreeMap<ValueId, MirType> {
    function
        .params
        .iter()
        .map(|param| (param.value, param.type_node.clone()))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, param.type_node.clone()))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, result.type_node.clone()))
                }))
        }))
        .collect()
}

fn local_name(value: ValueId) -> String {
    format!("v{}", value.index())
}

fn block_label(block: BlockId) -> String {
    format!("b{}", block.index())
}
