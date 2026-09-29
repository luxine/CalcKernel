use std::collections::{BTreeMap, BTreeSet};

use crate::*;

use super::{EmitWasmOptions, features::target_metadata, final_ir::FinalWasmModule};

pub fn emit_wat_kir_module(module: &KirModule, options: EmitWasmOptions) -> Result<String, String> {
    emit_wat_kir_module_with_contracts(module, None, options)
}

/// Emits WAT from an optimizer result only when its KIR and evidence are still verified.
pub fn emit_wat_kir_result(
    result: &KirPassManagerResult,
    options: EmitWasmOptions,
) -> Result<String, String> {
    let (module, contracts) = result.verified_artifact_and_contracts().ok_or_else(|| {
        "WebAssembly emission requires an unchanged verified KIR result".to_string()
    })?;
    emit_wat_kir_module_with_contracts(module, contracts, options)
}

fn emit_wat_kir_module_with_contracts(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    options: EmitWasmOptions,
) -> Result<String, String> {
    let features = validate_wasm_kir_module(module)?;
    let mir = adapt_unchecked_kir(module)?;
    let artifact = prepare_non_executable_artifact(&mir, MirArtifactConsumer::WebAssembly)
        .map_err(|error| error.to_string())?;
    let metadata = target_metadata(features, &module.profile.digest_hex());
    let mut lowered = emit_final_for_mir(module, &artifact, contracts, options)?;
    lowered.set_target_metadata(metadata.as_bytes().to_vec())?;
    let wat = lowered.to_wat();
    super::binary::validate_profile_wat(&wat, features, metadata.as_bytes())?;
    Ok(wat)
}

fn emit_final_for_mir(
    module: &KirModule,
    mir: &MirModule,
    contracts: Option<&ContractFactSet>,
    options: EmitWasmOptions,
) -> Result<FinalWasmModule, String> {
    let has_vector = module_has_vector_instructions(module) || module_has_vector_values(module);
    let has_version_predicate = module_has_version_predicates(module);
    let needs_typed_lowering = has_vector || has_version_predicate;
    if needs_typed_lowering {
        let lowered = super::lower::lower_wasm_module(module, contracts, mir)
            .map_err(|error| error.to_string())?;
        super::emit::emit_final_module_with_lowering(mir, &lowered, options)
    } else if options.opt_level >= 3 {
        match super::lower::lower_wasm_module(module, contracts, mir) {
            Ok(lowered) => super::emit::emit_final_module_with_lowering(mir, &lowered, options),
            Err(super::lower::WasmLoweringError::InvalidInput(message))
            | Err(super::lower::WasmLoweringError::InvariantFailure(message)) => Err(message),
            Err(error @ super::lower::WasmLoweringError::UnsupportedValueType(_)) => {
                Err(error.to_string())
            }
        }
    } else {
        super::lower::validate_wasm_scalar_value_types(module, mir)
            .map_err(|error| error.to_string())?;
        super::emit::emit_final_module_with_options(mir, options)
    }
}

pub fn emit_wasm_kir_module(
    module: &KirModule,
    options: EmitWasmOptions,
) -> Result<Vec<u8>, String> {
    emit_wasm_kir_module_with_contracts(module, None, options)
}

/// Emits a Wasm binary from an optimizer result only when its KIR and evidence are still verified.
pub fn emit_wasm_kir_result(
    result: &KirPassManagerResult,
    options: EmitWasmOptions,
) -> Result<Vec<u8>, String> {
    let (module, contracts) = result.verified_artifact_and_contracts().ok_or_else(|| {
        "WebAssembly emission requires an unchanged verified KIR result".to_string()
    })?;
    emit_wasm_kir_module_with_contracts(module, contracts, options)
}

fn emit_wasm_kir_module_with_contracts(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    options: EmitWasmOptions,
) -> Result<Vec<u8>, String> {
    let features = validate_wasm_kir_module(module)?;
    let mir = adapt_unchecked_kir(module)?;
    let artifact = prepare_non_executable_artifact(&mir, MirArtifactConsumer::WebAssembly)
        .map_err(|error| error.to_string())?;
    let metadata = target_metadata(features, &module.profile.digest_hex());
    let mut lowered = emit_final_for_mir(module, &artifact, contracts, options)?;
    lowered.set_target_metadata(metadata.as_bytes().to_vec())?;
    super::binary::encode_final_module(&lowered, features, metadata.as_bytes())
}

fn validate_wasm_kir_module(module: &KirModule) -> Result<KirWasmFeatures, String> {
    if module.profile.wasm_features().is_none() {
        reject_vector_values(module)?;
        reject_vector_instructions(module)?;
    }
    if module.config.overflow_mode != KirOverflowMode::Unchecked
        || module.config.bounds_mode != KirBoundsMode::Unchecked
    {
        return Err("WebAssembly KIR backend accepts only unchecked KIR".to_string());
    }
    if module.config.consumer != KirConsumer::WebAssembly
        || module.profile.consumer() != KirConsumer::WebAssembly
    {
        return Err("WebAssembly KIR backend requires a WebAssembly consumer and profile".into());
    }
    if module.config.sanitizer_mode != KirSanitizerMode::Disabled {
        return Err("WebAssembly KIR backend does not support KIR sanitizers".to_string());
    }
    let features = module.profile.wasm_features().ok_or_else(|| {
        "WebAssembly KIR backend requires a WebAssembly target profile".to_string()
    })?;
    module.profile.validate()?;
    validate_vector_kir(module, features)?;
    let validation = validate_kir_module(module);
    if let Some(error) = validation.errors.first() {
        return Err(error.message.clone());
    }
    Ok(features)
}

fn adapt_unchecked_kir(module: &KirModule) -> Result<MirModule, String> {
    if module.config.overflow_mode != KirOverflowMode::Unchecked
        || module.config.bounds_mode != KirBoundsMode::Unchecked
    {
        return Err("WebAssembly KIR backend accepts only unchecked KIR".to_string());
    }
    let reachable = reachable_wasm_functions(module)?;
    Ok(MirModule {
        entry: module.entry.clone(),
        structs: module.structs.clone(),
        functions: module
            .functions
            .iter()
            .filter(|function| reachable.contains(&function.name))
            .map(adapt_function)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn reachable_wasm_functions(module: &KirModule) -> Result<BTreeSet<String>, String> {
    let by_name = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect::<BTreeMap<_, _>>();
    let mut reachable = BTreeSet::<String>::new();
    let mut pending = module
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    while let Some(name) = pending.pop() {
        if !reachable.insert(name.to_string()) {
            continue;
        }
        let function = by_name
            .get(name)
            .ok_or_else(|| format!("WebAssembly export references unknown function {name}"))?;
        for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
            if let KirInstructionKind::Call { function_name, .. } = &instruction.kind {
                if !by_name.contains_key(function_name.as_str()) {
                    return Err(format!(
                        "WebAssembly call references unknown function {function_name}"
                    ));
                }
                pending.push(function_name);
            }
        }
    }
    Ok(reachable)
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
        .filter_map(|value| {
            types.get(value).map(|type_node| MirLocal {
                name: local_name(*value),
                type_node: type_node.clone(),
            })
        })
        .collect();
    let incoming_edges = incoming_edge_counts(function);
    let entry_block = function.blocks.first().map(|block| block.id);
    let force_synthetic_edges = function.blocks.len() == 1
        && !matches!(&function.blocks[0].terminator, KirTerminator::Return { .. });
    let adapt_context = KirAdaptContext {
        function,
        incoming_edges: &incoming_edges,
        force_synthetic_edges,
        types: &types,
        params: &function_params,
    };
    let mut branch_prefixes = BTreeMap::<BlockId, Vec<MirInstruction>>::new();
    for source in &function.blocks {
        let KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &source.terminator
        else {
            continue;
        };
        for (arm, edge) in [(0, then_edge), (1, else_edge)] {
            let target = function
                .blocks
                .iter()
                .find(|block| block.id == edge.target)
                .expect("validated target");
            if !force_synthetic_edges
                && !target.params.is_empty()
                && Some(edge.target) != entry_block
                && incoming_edges.get(&edge.target) == Some(&1)
            {
                let label = edge_label(source.id, edge.target, arm);
                branch_prefixes
                    .entry(edge.target)
                    .or_default()
                    .extend(edge_copy_instructions(
                        &label,
                        target,
                        edge,
                        &types,
                        &function_params,
                    ));
            }
        }
    }
    let mut blocks = Vec::new();
    let mut edge_blocks = Vec::new();
    for block in &function.blocks {
        let mut instructions = branch_prefixes.get(&block.id).cloned().unwrap_or_default();
        for instruction in &block.instructions {
            instructions.extend(adapt_instruction(instruction, &types, &function_params)?);
        }
        let terminator =
            adapt_terminator(&adapt_context, block, &mut instructions, &mut edge_blocks);
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

pub(super) fn adapt_instruction(
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
        KirInstructionKind::VersionPredicate { .. } => Vec::new(),
        KirInstructionKind::VectorSplat { .. }
        | KirInstructionKind::VectorLoad { .. }
        | KirInstructionKind::VectorStore { .. }
        | KirInstructionKind::VectorBinary { .. }
        | KirInstructionKind::VectorUnary { .. }
        | KirInstructionKind::VectorCompare { .. }
        | KirInstructionKind::VectorSelect { .. }
        | KirInstructionKind::VectorCast { .. }
        | KirInstructionKind::VectorReduce { .. } => Vec::new(),
        KirInstructionKind::VectorInsert { .. } | KirInstructionKind::VectorExtract { .. } => {
            return Err("WebAssembly KIR backend cannot lower vector instructions".to_string());
        }
    })
}

struct KirAdaptContext<'a> {
    function: &'a KirFunction,
    incoming_edges: &'a BTreeMap<BlockId, usize>,
    force_synthetic_edges: bool,
    types: &'a BTreeMap<ValueId, MirType>,
    params: &'a BTreeMap<ValueId, (String, MirType)>,
}

fn adapt_terminator(
    context: &KirAdaptContext<'_>,
    block: &KirBlock,
    instructions: &mut Vec<MirInstruction>,
    edge_blocks: &mut Vec<MirBlock>,
) -> MirTerminator {
    match &block.terminator {
        KirTerminator::Return { value, .. } => MirTerminator::Return {
            value: value.map(|value| mir_value(value, context.types, context.params)),
        },
        KirTerminator::Jump { edge } => {
            let target = context
                .function
                .blocks
                .iter()
                .find(|candidate| candidate.id == edge.target)
                .expect("validated target");
            if context.force_synthetic_edges {
                let label = edge_label(block.id, edge.target, 0);
                MirTerminator::Jump {
                    label: append_edge_block(
                        label,
                        target,
                        edge,
                        context.types,
                        context.params,
                        edge_blocks,
                    ),
                }
            } else {
                if !target.params.is_empty() {
                    let label = edge_label(block.id, edge.target, 0);
                    instructions.extend(edge_copy_instructions(
                        &label,
                        target,
                        edge,
                        context.types,
                        context.params,
                    ));
                }
                MirTerminator::Jump {
                    label: block_label(edge.target),
                }
            }
        }
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => MirTerminator::Branch {
            condition: mir_value(*condition, context.types, context.params),
            then_label: adapt_branch_edge(context, block.id, 0, then_edge, edge_blocks),
            else_label: adapt_branch_edge(context, block.id, 1, else_edge, edge_blocks),
        },
    }
}

fn adapt_branch_edge(
    context: &KirAdaptContext<'_>,
    source: BlockId,
    arm: u32,
    edge: &KirEdge,
    blocks: &mut Vec<MirBlock>,
) -> String {
    let target = context
        .function
        .blocks
        .iter()
        .find(|block| block.id == edge.target)
        .expect("validated target");
    if context.force_synthetic_edges {
        let label = edge_label(source, edge.target, arm);
        return append_edge_block(label, target, edge, context.types, context.params, blocks);
    }
    if target.params.is_empty() {
        return block_label(edge.target);
    }
    if Some(edge.target) != context.function.blocks.first().map(|block| block.id)
        && context.incoming_edges.get(&edge.target) == Some(&1)
    {
        return block_label(edge.target);
    }
    let label = edge_label(source, edge.target, arm);
    append_edge_block(label, target, edge, context.types, context.params, blocks)
}

fn append_edge_block(
    label: String,
    target: &KirBlock,
    edge: &KirEdge,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
    blocks: &mut Vec<MirBlock>,
) -> String {
    let instructions = edge_copy_instructions(&label, target, edge, types, params);
    blocks.push(MirBlock {
        label: label.clone(),
        instructions,
        terminator: MirTerminator::Jump {
            label: block_label(edge.target),
        },
    });
    label
}

pub(super) fn edge_copy_instructions(
    label: &str,
    target: &KirBlock,
    edge: &KirEdge,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
) -> Vec<MirInstruction> {
    let mut instructions = Vec::new();
    for (index, (target, argument)) in target.params.iter().zip(&edge.args).enumerate() {
        let Some(type_node) = target.type_node.as_scalar() else {
            // The typed emitter copies v128 values directly from the original edge.
            continue;
        };
        instructions.push(MirInstruction::Move {
            target: MirValue::Temp {
                name: format!("edge_{}_{}", label, index),
                type_node: type_node.clone(),
            },
            value: mir_value(*argument, types, params),
        });
    }
    for (index, target) in target.params.iter().enumerate() {
        let Some(type_node) = target.type_node.as_scalar() else {
            continue;
        };
        instructions.push(MirInstruction::Move {
            target: mir_value(target.value, types, params),
            value: MirValue::Temp {
                name: format!("edge_{}_{}", label, index),
                type_node: type_node.clone(),
            },
        });
    }
    instructions
}

fn incoming_edge_counts(function: &KirFunction) -> BTreeMap<BlockId, usize> {
    let mut incoming = BTreeMap::new();
    for block in &function.blocks {
        match &block.terminator {
            KirTerminator::Return { .. } => {}
            KirTerminator::Jump { edge } => *incoming.entry(edge.target).or_insert(0) += 1,
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                *incoming.entry(then_edge.target).or_insert(0) += 1;
                *incoming.entry(else_edge.target).or_insert(0) += 1;
            }
        }
    }
    incoming
}

pub(super) fn edge_label(source: BlockId, target: BlockId, arm: u32) -> String {
    format!("edge_{}_{}_{}", source.index(), target.index(), arm)
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

pub(super) fn mir_value(
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

pub(super) fn value_types(function: &KirFunction) -> BTreeMap<ValueId, MirType> {
    function
        .params
        .iter()
        .map(|param| (param.value, param.type_node.clone()))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .filter_map(|param| {
                    param
                        .type_node
                        .as_scalar()
                        .map(|type_node| (param.value, type_node.clone()))
                })
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction.results.iter().filter_map(|result| {
                        result
                            .type_node
                            .as_scalar()
                            .map(|type_node| (result.value, type_node.clone()))
                    })
                }))
        }))
        .collect()
}

pub(super) fn value_kir_types(function: &KirFunction) -> BTreeMap<ValueId, KirValueType> {
    function
        .params
        .iter()
        .map(|param| (param.value, KirValueType::Scalar(param.type_node.clone())))
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

fn supported_vector_shape(lane: KirLaneType, lanes: u16) -> bool {
    matches!(
        (lane, lanes),
        (KirLaneType::F64, 2) | (KirLaneType::I32 | KirLaneType::U32, 4)
    )
}

fn supported_narrow_load_shape(lane: KirLaneType, lanes: u16) -> bool {
    matches!((lane, lanes), (KirLaneType::I32 | KirLaneType::U32, 2))
}

fn validate_vector_access(access: &KirVectorMemoryAccess, store: bool) -> Result<(), String> {
    let (footprint, alignment) = match (access.lane, access.lanes, store) {
        (KirLaneType::F64, 2, _) => (16, 8),
        (KirLaneType::I32 | KirLaneType::U32, 4, _) => (16, 4),
        (KirLaneType::I32 | KirLaneType::U32, 2, false) => (8, 4),
        _ => return Err("WebAssembly SIMD128 cannot lower this vector memory shape".into()),
    };
    if access.byte_footprint != footprint
        || access.known_alignment != alignment
        || access.required_alignment != alignment
    {
        return Err(
            "WebAssembly SIMD128 vector access footprint or natural lane alignment is invalid"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn validate_vector_kir(
    module: &KirModule,
    features: KirWasmFeatures,
) -> Result<(), String> {
    if features == KirWasmFeatures::Baseline {
        reject_vector_values(module)?;
        reject_vector_instructions(module)?;
        if module_has_version_predicates(module) {
            return Err("WebAssembly baseline cannot lower runtime version predicates".into());
        }
        return Ok(());
    }

    for function in &module.functions {
        let types = value_kir_types(function);
        for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
            if let KirInstructionKind::VersionPredicate { predicate } = &instruction.kind {
                validate_wasm_version_predicate(
                    function,
                    instruction,
                    predicate,
                    &types,
                    module.profile.layout(),
                )?;
            }
        }
        let has_vector = function.blocks.iter().any(|block| {
            block
                .params
                .iter()
                .any(|param| param.type_node.as_scalar().is_none())
                || block.instructions.iter().any(|instruction| {
                    is_vector_instruction(&instruction.kind)
                        || instruction
                            .results
                            .iter()
                            .any(|result| result.type_node.as_scalar().is_none())
                })
        });
        if !has_vector {
            continue;
        }
        for block in &function.blocks {
            for param in &block.params {
                let supported = match param.type_node {
                    KirValueType::Scalar(_) => true,
                    KirValueType::FixedVector { lane, lanes } => {
                        supported_vector_shape(lane, lanes)
                            || supported_narrow_load_shape(lane, lanes)
                    }
                    KirValueType::Mask { lanes } => matches!(lanes, 2 | 4),
                };
                if !supported {
                    return Err("WebAssembly SIMD128 cannot lower this block parameter type".into());
                }
            }
            for instruction in &block.instructions {
                for result in &instruction.results {
                    match (&instruction.kind, &result.type_node) {
                        (_, KirValueType::Scalar(_)) => {}
                        (_, KirValueType::FixedVector { lane, lanes })
                            if supported_vector_shape(*lane, *lanes) => {}
                        (
                            KirInstructionKind::VectorLoad { .. }
                            | KirInstructionKind::VectorSplat { .. },
                            KirValueType::FixedVector { lane, lanes },
                        ) if supported_narrow_load_shape(*lane, *lanes) => {}
                        (
                            KirInstructionKind::VectorCompare { .. },
                            KirValueType::Mask { lanes: 2 | 4 },
                        ) => {}
                        (_, KirValueType::FixedVector { .. } | KirValueType::Mask { .. }) => {
                            return Err("WebAssembly SIMD128 cannot lower this vector value".into());
                        }
                    }
                }
                let supported = match &instruction.kind {
                    KirInstructionKind::VectorSplat { .. } => true,
                    KirInstructionKind::VectorLoad { access, .. } => {
                        let [result] = instruction.results.as_slice() else {
                            return Err(
                                "WebAssembly SIMD128 vector load result is malformed".into()
                            );
                        };
                        let result_matches = result.type_node
                            == KirValueType::FixedVector {
                                lane: access.lane,
                                lanes: access.lanes,
                            };
                        if result_matches {
                            validate_vector_access(access, false)?;
                        }
                        result_matches
                    }
                    KirInstructionKind::VectorStore { access, value, .. } => {
                        let value_matches = types.get(value)
                            == Some(&KirValueType::FixedVector {
                                lane: access.lane,
                                lanes: access.lanes,
                            });
                        let supported = instruction.results.is_empty() && value_matches;
                        if supported {
                            validate_vector_access(access, true)?;
                        }
                        supported
                    }
                    KirInstructionKind::VectorBinary {
                        op,
                        semantics,
                        no_failure_proof,
                        ..
                    } => {
                        let Some(KirResult {
                            type_node: KirValueType::FixedVector { lane, lanes },
                            ..
                        }) = instruction.results.first()
                        else {
                            return Err(
                                "WebAssembly SIMD128 vector binary result is missing".into()
                            );
                        };
                        supported_vector_shape(*lane, *lanes)
                            && no_failure_proof.is_none()
                            && match lane {
                                KirLaneType::F64 => {
                                    *semantics == KirArithmeticSemantics::StrictFloat
                                        && matches!(
                                            *op,
                                            KirVectorBinaryOp::Add
                                                | KirVectorBinaryOp::Subtract
                                                | KirVectorBinaryOp::Multiply
                                                | KirVectorBinaryOp::Divide
                                        )
                                }
                                KirLaneType::I32 | KirLaneType::U32 => {
                                    *semantics == KirArithmeticSemantics::Modular
                                        && matches!(
                                            *op,
                                            KirVectorBinaryOp::Add
                                                | KirVectorBinaryOp::Subtract
                                                | KirVectorBinaryOp::Multiply
                                        )
                                }
                                KirLaneType::I64 | KirLaneType::U64 => false,
                            }
                    }
                    KirInstructionKind::VectorUnary {
                        op,
                        semantics,
                        no_failure_proof,
                        ..
                    } => {
                        let Some(KirResult {
                            type_node: KirValueType::FixedVector { lane, lanes },
                            ..
                        }) = instruction.results.first()
                        else {
                            return Err("WebAssembly SIMD128 vector unary result is missing".into());
                        };
                        supported_vector_shape(*lane, *lanes)
                            && *op == KirVectorUnaryOp::Negate
                            && no_failure_proof.is_none()
                            && match lane {
                                KirLaneType::F64 => {
                                    *semantics == KirArithmeticSemantics::StrictFloat
                                }
                                KirLaneType::I32 | KirLaneType::U32 => {
                                    *semantics == KirArithmeticSemantics::Modular
                                }
                                KirLaneType::I64 | KirLaneType::U64 => false,
                            }
                    }
                    KirInstructionKind::VectorCompare { left, right, .. } => {
                        let [result] = instruction.results.as_slice() else {
                            return Err(
                                "WebAssembly SIMD128 vector compare result is malformed".into()
                            );
                        };
                        let KirValueType::Mask { lanes } = &result.type_node else {
                            return Err(
                                "WebAssembly SIMD128 vector compare must produce a mask".into()
                            );
                        };
                        match (types.get(left), types.get(right)) {
                            (
                                Some(KirValueType::FixedVector {
                                    lane,
                                    lanes: left_lanes,
                                }),
                                Some(right_type),
                            ) => {
                                supported_vector_shape(*lane, *left_lanes)
                                    && left_lanes == lanes
                                    && right_type
                                        == &KirValueType::FixedVector {
                                            lane: *lane,
                                            lanes: *left_lanes,
                                        }
                            }
                            _ => false,
                        }
                    }
                    KirInstructionKind::VectorSelect {
                        mask,
                        when_true,
                        when_false,
                        ..
                    } => {
                        let [result] = instruction.results.as_slice() else {
                            return Err(
                                "WebAssembly SIMD128 vector select result is malformed".into()
                            );
                        };
                        let KirValueType::FixedVector { lane, lanes } = &result.type_node else {
                            return Err(
                                "WebAssembly SIMD128 vector select result is malformed".into()
                            );
                        };
                        let expected = KirValueType::FixedVector {
                            lane: *lane,
                            lanes: *lanes,
                        };
                        supported_vector_shape(*lane, *lanes)
                            && types.get(mask) == Some(&KirValueType::Mask { lanes: *lanes })
                            && types.get(when_true) == Some(&expected)
                            && types.get(when_false) == Some(&expected)
                    }
                    KirInstructionKind::VectorCast { op, value, .. } => {
                        let [result] = instruction.results.as_slice() else {
                            return Err(
                                "WebAssembly SIMD128 vector cast result is malformed".into()
                            );
                        };
                        let source_lane = match op {
                            KirVectorCastOp::I32ToF64 => KirLaneType::I32,
                            KirVectorCastOp::U32ToF64 => KirLaneType::U32,
                        };
                        result.type_node
                            == KirValueType::FixedVector {
                                lane: KirLaneType::F64,
                                lanes: 2,
                            }
                            && types.get(value)
                                == Some(&KirValueType::FixedVector {
                                    lane: source_lane,
                                    lanes: 2,
                                })
                    }
                    KirInstructionKind::VectorReduce {
                        op,
                        vector,
                        semantics,
                        ..
                    } => {
                        let [result] = instruction.results.as_slice() else {
                            return Err(
                                "WebAssembly SIMD128 vector reduction result is malformed".into()
                            );
                        };
                        let expected_result = match (op, types.get(vector)) {
                            (
                                KirVectorReductionOp::ModularAdd
                                | KirVectorReductionOp::ModularMultiply,
                                Some(KirValueType::FixedVector {
                                    lane: KirLaneType::I32,
                                    lanes: 4,
                                }),
                            ) => Some(MirType::Primitive(MirPrimitiveTypeName::I32)),
                            (
                                KirVectorReductionOp::ModularAdd
                                | KirVectorReductionOp::ModularMultiply,
                                Some(KirValueType::FixedVector {
                                    lane: KirLaneType::U32,
                                    lanes: 4,
                                }),
                            ) => Some(MirType::Primitive(MirPrimitiveTypeName::U32)),
                            _ => None,
                        };
                        expected_result.is_some_and(|result_type| {
                            *semantics == KirArithmeticSemantics::Modular
                                && result.type_node == KirValueType::Scalar(result_type)
                        })
                    }
                    _ if is_vector_instruction(&instruction.kind) => false,
                    _ => instruction
                        .results
                        .iter()
                        .all(|result| result.type_node.as_scalar().is_some()),
                };
                if !supported {
                    return Err("WebAssembly SIMD128 cannot lower this vector instruction".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_wasm_version_predicate(
    function: &KirFunction,
    instruction: &KirInstruction,
    predicate: &KirVersionPredicate,
    types: &BTreeMap<ValueId, KirValueType>,
    layout: KirProfileLayout,
) -> Result<(), String> {
    let bool_type = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::Bool));
    let u32_type = MirType::Primitive(MirPrimitiveTypeName::U32);
    let (thresholds, nontrip) =
        predicate
            .conjuncts
            .iter()
            .fold((0, 0), |counts, item| match item {
                KirVersionPredicateConjunct::TripThreshold { .. } => (counts.0 + 1, counts.1),
                KirVersionPredicateConjunct::AddressIntervalsDisjoint { .. }
                | KirVersionPredicateConjunct::WasmSliceRange { .. } => (counts.0, counts.1 + 1),
            });
    if predicate.address_bits != 32
        || !matches!(
            layout,
            KirProfileLayout::Known {
                pointer_width_bits: 32,
                ..
            }
        )
        || instruction.results.len() != 1
        || instruction.results[0].type_node != bool_type
        || instruction.memory.is_some()
        || instruction.effect.is_some()
        || predicate.conjuncts.is_empty()
        || predicate.conjuncts.len() > 4
        || thresholds > 1
        || nontrip > 3
    {
        return Err(format!(
            "WebAssembly SIMD128 cannot lower this version predicate in {}",
            function.name
        ));
    }

    let slice_element_bytes = |value: &ValueId| {
        types
            .get(value)
            .and_then(KirValueType::as_scalar)
            .and_then(|type_node| match type_node {
                MirType::Slice(element) => match element.as_ref() {
                    MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
                        Some(4)
                    }
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
    for conjunct in &predicate.conjuncts {
        let valid = match conjunct {
            KirVersionPredicateConjunct::WasmSliceRange {
                slice,
                start,
                count,
                element_bytes,
            } => {
                slice_element_bytes(slice) == Some(*element_bytes)
                    && types.get(start).and_then(KirValueType::as_scalar) == Some(&u32_type)
                    && types.get(count).and_then(KirValueType::as_scalar) == Some(&u32_type)
            }
            KirVersionPredicateConjunct::TripThreshold { value, minimum } => {
                *minimum > 0
                    && types.get(value).and_then(KirValueType::as_scalar) == Some(&u32_type)
            }
            KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                left,
                left_count,
                left_element_bytes,
                right,
                right_count,
                right_element_bytes,
            } => {
                left != right
                    && slice_element_bytes(left) == Some(*left_element_bytes)
                    && slice_element_bytes(right) == Some(*right_element_bytes)
                    && types.get(left_count).and_then(KirValueType::as_scalar) == Some(&u32_type)
                    && types.get(right_count).and_then(KirValueType::as_scalar) == Some(&u32_type)
            }
        };
        if !valid {
            return Err(format!(
                "WebAssembly SIMD128 cannot lower an unsupported version-predicate conjunct in {}",
                function.name
            ));
        }
    }
    Ok(())
}

fn is_vector_instruction(kind: &KirInstructionKind) -> bool {
    matches!(
        kind,
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
    )
}

fn module_has_vector_instructions(module: &KirModule) -> bool {
    module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .any(|instruction| is_vector_instruction(&instruction.kind))
}

pub(super) fn module_has_version_predicates(module: &KirModule) -> bool {
    module.functions.iter().any(|function| {
        function.blocks.iter().any(|block| {
            block.instructions.iter().any(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::VersionPredicate { .. }
                )
            })
        })
    })
}

fn module_has_vector_values(module: &KirModule) -> bool {
    module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| {
            block.params.iter().map(|param| &param.type_node).chain(
                block.instructions.iter().flat_map(|instruction| {
                    instruction.results.iter().map(|result| &result.type_node)
                }),
            )
        })
        .any(|type_node| type_node.as_scalar().is_none())
}

pub(super) fn reject_vector_values(module: &KirModule) -> Result<(), String> {
    if module_has_vector_values(module) {
        return Err("WebAssembly KIR backend cannot lower vector values".to_string());
    }
    Ok(())
}

fn reject_vector_instructions(module: &KirModule) -> Result<(), String> {
    if module_has_vector_instructions(module) {
        return Err("WebAssembly KIR backend cannot lower vector instructions".to_string());
    }
    Ok(())
}

pub(super) fn local_name(value: ValueId) -> String {
    format!("v{}", value.index())
}

pub(super) fn block_label(block: BlockId) -> String {
    format!("b{}", block.index())
}

#[cfg(test)]
mod tests {
    use crate::{
        EmitWasmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode,
        KirSanitizerMode, SourceFile, build_kir_module, check, emit_wasm_kir_module,
        emit_wat_kir_module, lower_to_mir,
    };

    use super::emit_final_for_mir;

    #[test]
    fn o3_scalar_should_propagate_typed_lowering_invariant_failures() {
        let checked = check(&SourceFile::new(
            "wasm-lowering-invariant.ck",
            "export fn choose(flag: bool, value: i32) -> i32 { if flag { return value + 1; } return value; }",
        ));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let mut module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR");
        let branch = module.functions[0]
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, crate::KirTerminator::Branch { .. }))
            .expect("conditional branch");
        let crate::KirTerminator::Branch { then_edge, .. } = &mut branch.terminator else {
            unreachable!();
        };
        then_edge.target = crate::BlockId::from_index(u32::MAX);

        let error = emit_final_for_mir(&module, &mir, None, EmitWasmOptions { opt_level: 3 })
            .expect_err("typed lowering invariant failures must not be swallowed");
        assert!(error.contains("unknown block"), "{error}");
    }

    #[test]
    fn wasm_emission_should_report_unsupported_aggregate_values_without_mir_fallback_at_all_levels()
    {
        let checked = check(&SourceFile::new(
            "wasm-unsupported-aggregate.ck",
            "struct Item { value: i32; } export fn read(item: Item) -> i32 { return item.value; }",
        ));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR");

        for opt_level in 0..=3 {
            let options = EmitWasmOptions { opt_level };
            let wat = std::panic::catch_unwind(|| emit_wat_kir_module(&module, options))
                .expect("unsupported value should be reported instead of panicking");
            assert!(
                wat.expect_err("aggregate value is unsupported by the scalar WASM ABI")
                    .contains("cannot lower value type"),
                "O{opt_level} WAT"
            );
            let binary = std::panic::catch_unwind(|| emit_wasm_kir_module(&module, options))
                .expect("unsupported value should be reported instead of panicking");
            assert!(
                binary
                    .expect_err("aggregate value is unsupported by the scalar WASM ABI")
                    .contains("cannot lower value type"),
                "O{opt_level} binary"
            );
        }
    }

    #[test]
    fn o3_wasm_emission_should_keep_dispatcher_for_an_irreducible_cfg() {
        let checked = check(&SourceFile::new(
            "wasm-irreducible-dispatcher.ck",
            "export fn cycle(flag: bool) -> void { if flag { return; } return; }",
        ));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let mut module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR");
        let function = &mut module.functions[0];
        assert_eq!(function.blocks.len(), 3, "entry and both branch arms");
        let first = function.blocks[1].id;
        let second = function.blocks[2].id;
        let first_param = function.blocks[1].params[0].value;
        let second_param = function.blocks[2].params[0].value;
        let first_memory = function.blocks[1].memory_params[0].version;
        let second_memory = function.blocks[2].memory_params[0].version;
        function.blocks[1].terminator = crate::KirTerminator::Jump {
            edge: crate::KirEdge {
                target: second,
                args: vec![first_param],
                memory_args: vec![first_memory],
            },
        };
        function.blocks[2].terminator = crate::KirTerminator::Jump {
            edge: crate::KirEdge {
                target: first,
                args: vec![second_param],
                memory_args: vec![second_memory],
            },
        };
        let validation = crate::validate_kir_module(&module);
        assert!(validation.errors.is_empty(), "{:#?}", validation.errors);

        let options = EmitWasmOptions { opt_level: 3 };
        let wat = emit_wat_kir_module(&module, options).expect("irreducible WAT");
        assert!(
            wat.contains("br_table"),
            "dispatcher missing from WAT: {wat}"
        );
        let wasm = emit_wasm_kir_module(&module, options).expect("irreducible binary");
        wasmparser::Validator::new_with_features(
            wasmparser::WasmFeatures::MVP | wasmparser::WasmFeatures::MULTI_VALUE,
        )
        .validate_all(&wasm)
        .expect("dispatcher binary validates");
    }
}
