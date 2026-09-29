use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::*;

use super::{
    ir::{
        WasmLoweredBlock, WasmLoweredEdge, WasmLoweredFunction, WasmLoweredInstruction,
        WasmLoweredModule, WasmPhysicalType, WasmSourceType, WasmTypedValue,
    },
    kir::{
        adapt_instruction, block_label, edge_copy_instructions, local_name, validate_vector_kir,
        value_kir_types, value_types,
    },
    layout::WasmStructLayout,
    memory::checked_wasm_memory_plan_with_evidence,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WasmLoweringError {
    InvalidInput(String),
    UnsupportedValueType(MirType),
    InvariantFailure(String),
}

impl std::fmt::Display for WasmLoweringError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::InvariantFailure(message) => {
                formatter.write_str(message)
            }
            Self::UnsupportedValueType(type_node) => write!(
                formatter,
                "WebAssembly KIR backend cannot lower value type {type_node:?}"
            ),
        }
    }
}

impl std::error::Error for WasmLoweringError {}

pub(super) fn lower_wasm_module<'a>(
    module: &'a KirModule,
    contracts: Option<&ContractFactSet>,
    mir: &MirModule,
) -> Result<WasmLoweredModule<'a>, WasmLoweringError> {
    if module.config.overflow_mode != KirOverflowMode::Unchecked
        || module.config.bounds_mode != KirBoundsMode::Unchecked
    {
        return Err(WasmLoweringError::InvalidInput(
            "WebAssembly KIR backend accepts only unchecked KIR".to_string(),
        ));
    }
    let features = module.profile.wasm_features().ok_or_else(|| {
        WasmLoweringError::InvalidInput(
            "WebAssembly KIR backend requires a WebAssembly target profile".to_string(),
        )
    })?;
    validate_vector_kir(module, features).map_err(WasmLoweringError::InvalidInput)?;
    let layout = WasmStructLayout::new(mir);
    let artifact_functions = mir
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<BTreeSet<_>>();
    Ok(WasmLoweredModule {
        functions: module
            .functions
            .iter()
            .filter(|function| artifact_functions.contains(function.name.as_str()))
            .map(|function| lower_wasm_function(function, contracts, &layout))
            .collect::<Result<Vec<_>, WasmLoweringError>>()?,
    })
}

/// Rejects scalar KIR values that the legacy MIR emitter cannot represent before
/// optimization levels that intentionally skip typed lowering reach that emitter.
pub(super) fn validate_wasm_scalar_value_types(
    module: &KirModule,
    mir: &MirModule,
) -> Result<(), WasmLoweringError> {
    let artifact_functions = mir
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<BTreeSet<_>>();

    for function in module
        .functions
        .iter()
        .filter(|function| artifact_functions.contains(function.name.as_str()))
    {
        if !matches!(function.return_type, MirType::Void) {
            wasm_scalar_physical_type(&function.return_type)?;
        }
        for param in &function.params {
            wasm_scalar_physical_type(&param.type_node)?;
        }
        for value_type in function.blocks.iter().flat_map(|block| {
            block.params.iter().map(|param| &param.type_node).chain(
                block.instructions.iter().flat_map(|instruction| {
                    instruction.results.iter().map(|result| &result.type_node)
                }),
            )
        }) {
            if let KirValueType::Scalar(type_node) = value_type {
                wasm_scalar_physical_type(type_node)?;
            }
        }
    }

    Ok(())
}

fn lower_wasm_function<'a>(
    function: &'a KirFunction,
    contracts: Option<&ContractFactSet>,
    layout: &WasmStructLayout,
) -> Result<WasmLoweredFunction<'a>, WasmLoweringError> {
    let types = value_types(function);
    let kir_types = value_kir_types(function);
    let params = function
        .params
        .iter()
        .map(|param| (param.value, (param.name.clone(), param.type_node.clone())))
        .collect::<BTreeMap<_, _>>();
    let source_types = function
        .params
        .iter()
        .map(|param| (param.value, WasmSourceType::Mir(&param.type_node)))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, WasmSourceType::Kir(&param.type_node)))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, WasmSourceType::Kir(&result.type_node)))
                }))
        }))
        .collect::<BTreeMap<_, _>>();
    let values = source_types
        .into_iter()
        .map(|(id, source_type)| {
            let kir_type = kir_types.get(&id).ok_or_else(|| {
                WasmLoweringError::InvariantFailure(format!(
                    "WebAssembly KIR has no type for value {}",
                    id.index()
                ))
            })?;
            let operand = match source_type {
                WasmSourceType::Mir(_) | WasmSourceType::Kir(KirValueType::Scalar(_)) => {
                    Some(super::kir::mir_value(id, &types, &params))
                }
                WasmSourceType::Kir(KirValueType::FixedVector { .. }) => None,
                WasmSourceType::Kir(KirValueType::Mask { .. }) => None,
            };
            let physical = wasm_physical_type(source_type, kir_type)?;
            Ok((
                id,
                WasmTypedValue {
                    value: id,
                    source_type,
                    operand,
                    physical,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, WasmLoweringError>>()?;
    if !matches!(function.return_type, MirType::Void) {
        wasm_scalar_physical_type(&function.return_type)?;
    }

    let blocks_by_id = function
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<HashMap<_, _>>();
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

    let mut lowered_blocks = Vec::with_capacity(function.blocks.len());
    let mut view_blocks = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        let mut lowered_instructions = Vec::with_capacity(block.instructions.len());
        let mut view_instructions = Vec::new();
        for instruction in &block.instructions {
            let leaves = adapt_instruction(instruction, &types, &params)
                .map_err(WasmLoweringError::InvalidInput)?;
            view_instructions.extend(leaves.iter().cloned());
            lowered_instructions.push(WasmLoweredInstruction {
                source: instruction,
                leaves,
            });
        }
        let mut edges = Vec::new();
        match &block.terminator {
            KirTerminator::Return { .. } => {}
            KirTerminator::Jump { edge } => {
                edges.push(lower_edge(block, 0, edge, &blocks_by_id, &types, &params)?);
            }
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                edges.push(lower_edge(
                    block,
                    0,
                    then_edge,
                    &blocks_by_id,
                    &types,
                    &params,
                )?);
                edges.push(lower_edge(
                    block,
                    1,
                    else_edge,
                    &blocks_by_id,
                    &types,
                    &params,
                )?);
            }
        }
        for edge in &edges {
            view_instructions.extend(edge.copies.iter().cloned());
        }
        let view_terminator = match &block.terminator {
            KirTerminator::Return { value, .. } => MirTerminator::Return {
                value: value.map(|value| super::kir::mir_value(value, &types, &params)),
            },
            KirTerminator::Jump { edge } => MirTerminator::Jump {
                label: block_label(edge.target),
            },
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => MirTerminator::Branch {
                condition: super::kir::mir_value(*condition, &types, &params),
                then_label: block_label(then_edge.target),
                else_label: block_label(else_edge.target),
            },
        };
        view_blocks.push(MirBlock {
            label: block_label(block.id),
            instructions: view_instructions,
            terminator: view_terminator,
        });
        lowered_blocks.push(WasmLoweredBlock {
            source: block,
            instructions: lowered_instructions,
            edges,
        });
    }

    Ok(WasmLoweredFunction {
        source: function,
        vector_values: values
            .iter()
            .filter_map(|(value, typed)| {
                (typed.physical == WasmPhysicalType::V128).then_some(*value)
            })
            .collect(),
        memory_plan: checked_wasm_memory_plan_with_evidence(function, contracts, Some(layout)),
        values,
        blocks: lowered_blocks,
        local_view: MirFunction {
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
            blocks: view_blocks,
        },
    })
}

fn lower_edge<'a>(
    source: &'a KirBlock,
    arm: u8,
    edge: &'a KirEdge,
    blocks: &HashMap<BlockId, &'a KirBlock>,
    types: &BTreeMap<ValueId, MirType>,
    params: &BTreeMap<ValueId, (String, MirType)>,
) -> Result<WasmLoweredEdge<'a>, WasmLoweringError> {
    let target = blocks.get(&edge.target).ok_or_else(|| {
        WasmLoweringError::InvariantFailure(format!(
            "WebAssembly KIR edge targets unknown block {}",
            edge.target.index()
        ))
    })?;
    let label = super::kir::edge_label(source.id, edge.target, u32::from(arm));
    Ok(WasmLoweredEdge {
        arm,
        source: edge,
        copies: edge_copy_instructions(&label, target, edge, types, params),
    })
}

fn wasm_physical_type(
    source_type: WasmSourceType<'_>,
    kir_type: &KirValueType,
) -> Result<WasmPhysicalType, WasmLoweringError> {
    match (source_type, kir_type) {
        (WasmSourceType::Mir(type_node), KirValueType::Scalar(source))
        | (WasmSourceType::Kir(KirValueType::Scalar(type_node)), KirValueType::Scalar(source))
            if type_node == source =>
        {
            wasm_scalar_physical_type(type_node)
        }
        (
            WasmSourceType::Kir(KirValueType::FixedVector { .. }),
            KirValueType::FixedVector { .. },
        ) => Ok(WasmPhysicalType::V128),
        (WasmSourceType::Kir(KirValueType::Mask { .. }), KirValueType::Mask { .. }) => {
            Ok(WasmPhysicalType::V128)
        }
        _ => Err(WasmLoweringError::InvariantFailure(
            "WebAssembly KIR typed value source metadata is inconsistent".to_string(),
        )),
    }
}

/// Returns the v128 assignments on one selected edge. Scalar assignments live in
/// the MIR leaf view; vector and mask values retain their exact KIR types here.
pub(super) fn vector_edge_copies(
    function: &WasmLoweredFunction<'_>,
    edge: &WasmLoweredEdge<'_>,
) -> Result<Vec<(ValueId, ValueId)>, WasmLoweringError> {
    let invalid = |message: &str| WasmLoweringError::InvariantFailure(message.to_string());
    let target = function
        .source
        .blocks
        .iter()
        .find(|block| block.id == edge.source.target)
        .ok_or_else(|| invalid("WebAssembly vector edge targets an unknown block"))?;
    if target.params.len() != edge.source.args.len() {
        return Err(invalid(
            "WebAssembly vector edge argument arity is inconsistent",
        ));
    }
    let mut copies = Vec::new();
    for (param, argument) in target.params.iter().zip(&edge.source.args) {
        if param.type_node.as_scalar().is_some() {
            continue;
        }
        for value in [param.value, *argument] {
            let typed = function
                .values
                .get(&value)
                .ok_or_else(|| invalid("WebAssembly vector edge value has no typed metadata"))?;
            if typed.physical != WasmPhysicalType::V128
                || !matches!(typed.source_type, WasmSourceType::Kir(type_node) if type_node == &param.type_node)
            {
                return Err(invalid(
                    "WebAssembly vector edge value type is inconsistent",
                ));
            }
        }
        copies.push((param.value, *argument));
    }
    Ok(copies)
}

pub(super) struct WasmConditionalIncrement<'lowered, 'source> {
    pub direct_edge: &'lowered WasmLoweredEdge<'source>,
    pub condition: ValueId,
    pub base: ValueId,
    pub result: ValueId,
    pub copy_index: usize,
    pub increment_on_true: bool,
}

/// Recognizes a closed, effect-free triangle without changing its typed CFG.
/// The direct join edge supplies all parallel assignments except one modular
/// increment. Values defined in the bypassed arm cannot escape through other uses.
pub(super) fn checked_conditional_increment<'lowered, 'source>(
    function: &'lowered WasmLoweredFunction<'source>,
    source: &'lowered WasmLoweredBlock<'source>,
) -> Option<WasmConditionalIncrement<'lowered, 'source>> {
    let KirTerminator::Branch { condition, .. } = source.source.terminator else {
        return None;
    };
    for update_arm in 0..=1 {
        let update_edge = source.edges.iter().find(|edge| edge.arm == update_arm)?;
        let direct_edge = source.edges.iter().find(|edge| edge.arm != update_arm)?;
        let Some(arm) = function
            .blocks
            .iter()
            .find(|block| block.source.id == update_edge.source.target)
        else {
            continue;
        };
        let Some(candidate) =
            check_increment_arm(function, source, arm, update_edge, direct_edge, condition)
        else {
            continue;
        };
        return Some(candidate);
    }
    None
}

fn check_increment_arm<'lowered, 'source>(
    function: &'lowered WasmLoweredFunction<'source>,
    source: &WasmLoweredBlock<'source>,
    arm: &WasmLoweredBlock<'source>,
    update_edge: &WasmLoweredEdge<'source>,
    direct_edge: &'lowered WasmLoweredEdge<'source>,
    condition: ValueId,
) -> Option<WasmConditionalIncrement<'lowered, 'source>> {
    if arm.source.id == source.source.id
        || arm.source.id == direct_edge.source.target
        || direct_edge.source.target == source.source.id
        || function
            .source
            .blocks
            .iter()
            .flat_map(|block| wasm_edges(&block.terminator))
            .flatten()
            .filter(|edge| edge.target == arm.source.id)
            .count()
            != 1
        || [
            (source.source.id, update_edge.arm),
            (source.source.id, direct_edge.arm),
            (arm.source.id, 0),
        ]
        .iter()
        .any(|edge| {
            function
                .memory_plan
                .edge_actions
                .get(edge)
                .is_some_and(|actions| !actions.is_empty())
        })
    {
        return None;
    }
    let KirTerminator::Jump { edge: join_edge } = &arm.source.terminator else {
        return None;
    };
    if join_edge.target != direct_edge.source.target
        || arm.source.params.len() != update_edge.source.args.len()
        || arm.source.memory_params.len() != update_edge.source.memory_args.len()
        || join_edge.args.len() != direct_edge.source.args.len()
        || join_edge.memory_args.len() != direct_edge.source.memory_args.len()
    {
        return None;
    }
    let resolve = |value: ValueId| {
        arm.source
            .params
            .iter()
            .position(|param| param.value == value)
            .map_or(value, |index| update_edge.source.args[index])
    };
    for (carried, direct) in join_edge
        .memory_args
        .iter()
        .zip(&direct_edge.source.memory_args)
    {
        let resolved = arm
            .source
            .memory_params
            .iter()
            .position(|param| param.version == *carried)
            .map_or(*carried, |index| update_edge.source.memory_args[index]);
        if resolved != *direct {
            return None;
        }
    }
    let mut add = None;
    for instruction in &arm.source.instructions {
        if instruction.memory.is_some()
            || instruction.effect.is_some()
            || instruction.results.len() != 1
        {
            return None;
        }
        match &instruction.kind {
            KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                semantics: KirArithmeticSemantics::Modular,
                ..
            } => {
                if add.replace(instruction).is_some() {
                    return None;
                }
            }
            KirInstructionKind::ConstInt { value } if value.parse::<u32>().ok() == Some(1) => {}
            _ => return None,
        }
    }
    let add = add?;
    let KirInstructionKind::Binary { left, right, .. } = add.kind else {
        return None;
    };
    let result = &add.results[0];
    if !matches!(
        result.type_node,
        KirValueType::Scalar(MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32
        ))
    ) {
        return None;
    }
    let is_one = |value| {
        function.source.blocks.iter().flat_map(|block| &block.instructions)
        .any(|instruction| matches!(&instruction.kind, KirInstructionKind::ConstInt { value: literal } if literal.parse::<u32>().ok() == Some(1))
            && instruction.results.as_slice().iter().any(|item| item.value == resolve(value) && item.type_node == result.type_node))
    };
    let (base, one) = if is_one(right) {
        (resolve(left), resolve(right))
    } else if is_one(left) {
        (resolve(right), resolve(left))
    } else {
        return None;
    };
    // Any local constant must be precisely the unit operand, with no extra work.
    if arm
        .source
        .instructions
        .iter()
        .filter(|instruction| instruction.id != add.id)
        .any(|instruction| {
            instruction.results[0].value != one
                || instruction.results[0].type_node != result.type_node
        })
    {
        return None;
    }
    let mut changed_index = None;
    for (index, (carried, direct)) in join_edge
        .args
        .iter()
        .zip(&direct_edge.source.args)
        .enumerate()
    {
        if *carried == result.value {
            if *direct != base || changed_index.replace(index).is_some() {
                return None;
            }
        } else if resolve(*carried) != *direct {
            return None;
        }
    }
    let changed_index = changed_index?;
    let definitions = arm
        .source
        .params
        .iter()
        .map(|param| param.value)
        .chain(
            arm.source
                .instructions
                .iter()
                .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
        )
        .collect::<BTreeSet<_>>();
    for block in &function.source.blocks {
        if block.id == arm.source.id {
            continue;
        }
        let mut external_use = false;
        for instruction in &block.instructions {
            crate::visit_instruction_uses(instruction, &mut |value| {
                external_use |= definitions.contains(&value)
            });
        }
        let control_value = match block.terminator {
            KirTerminator::Return { value, .. } => value,
            KirTerminator::Branch { condition, .. } => Some(condition),
            KirTerminator::Jump { .. } => None,
        };
        if external_use
            || control_value.is_some_and(|value| definitions.contains(&value))
            || wasm_edges(&block.terminator)
                .into_iter()
                .flatten()
                .flat_map(|edge| &edge.args)
                .any(|value| definitions.contains(value))
        {
            return None;
        }
    }
    let join = function
        .source
        .blocks
        .iter()
        .find(|block| block.id == join_edge.target)?;
    if join.params.get(changed_index)?.type_node != result.type_node {
        return None;
    }
    let copy_index = join.params[..changed_index]
        .iter()
        .filter(|param| param.type_node.as_scalar().is_some())
        .count();
    let MirInstruction::Move { value, .. } = direct_edge.copies.get(copy_index)? else {
        return None;
    };
    if Some(value) != function.values.get(&base)?.operand.as_ref() {
        return None;
    }
    Some(WasmConditionalIncrement {
        direct_edge,
        condition,
        base,
        result: result.value,
        copy_index,
        increment_on_true: update_edge.arm == 0,
    })
}

fn wasm_edges(terminator: &KirTerminator) -> [Option<&KirEdge>; 2] {
    match terminator {
        KirTerminator::Return { .. } => [None, None],
        KirTerminator::Jump { edge } => [Some(edge), None],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
    }
}

fn wasm_scalar_physical_type(type_node: &MirType) -> Result<WasmPhysicalType, WasmLoweringError> {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => Ok(WasmPhysicalType::I32),
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            Ok(WasmPhysicalType::I64)
        }
        MirType::Primitive(MirPrimitiveTypeName::F64) => Ok(WasmPhysicalType::F64),
        MirType::Slice(_) => Ok(WasmPhysicalType::I32Pair),
        MirType::Struct(_) | MirType::Void => {
            Err(WasmLoweringError::UnsupportedValueType(type_node.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
        KirValueType, MirType, SourceFile, build_kir_module, check, lower_to_mir,
    };

    use super::{WasmLoweringError, WasmPhysicalType, WasmSourceType, lower_wasm_module};

    fn test_kir() -> (crate::KirModule, crate::MirModule) {
        let checked = check(&SourceFile::new(
            "wasm-typed-lowering.ck",
            r#"
                export fn typed(out: ptr<i32>, i: i32, wide: i64, scale: f64, flag: bool,
                                values: slice<i32>) -> f64 {
                    let result: f64 = scale * 2.0;
                    if flag {
                        out[0] = i;
                    } else {
                        out[0] = i + 1;
                    }
                    return result;
                }
            "#,
        ));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("valid MIR");
        let kir = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR");
        (kir, mir)
    }

    #[test]
    fn wasm_lowering_should_classify_invalid_input_and_cfg_invariants() {
        let (mut module, mir) = test_kir();
        module.profile = crate::KirTargetProfile::portable_c();
        assert!(matches!(
            lower_wasm_module(&module, None, &mir),
            Err(WasmLoweringError::InvalidInput(_))
        ));

        let (mut module, mir) = test_kir();
        let branch = module.functions[0]
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, crate::KirTerminator::Branch { .. }))
            .expect("conditional branch");
        let crate::KirTerminator::Branch { then_edge, .. } = &mut branch.terminator else {
            unreachable!();
        };
        then_edge.target = crate::BlockId::from_index(u32::MAX);
        let error = lower_wasm_module(&module, None, &mir).expect_err("unknown edge target");
        assert!(matches!(error, WasmLoweringError::InvariantFailure(_)));
        assert!(
            error
                .to_string()
                .contains("WebAssembly KIR edge targets unknown block"),
            "{error}"
        );
    }

    #[test]
    fn wasm_lowering_should_classify_unsupported_value_types_and_preserve_display() {
        let checked = check(&SourceFile::new(
            "wasm-typed-unsupported.ck",
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

        let error = lower_wasm_module(&module, None, &mir).expect_err("struct values unsupported");
        assert!(matches!(
            &error,
            WasmLoweringError::UnsupportedValueType(MirType::Struct(name)) if name == "Item"
        ));
        assert_eq!(
            error.to_string(),
            "WebAssembly KIR backend cannot lower value type Struct(\"Item\")"
        );
    }

    #[test]
    fn wasm_lowering_should_retain_typed_values_sources_effects_edges_and_order() {
        let (module, mir) = test_kir();
        let lowered = lower_wasm_module(&module, None, &mir).expect("typed lowering");

        let source = &module.functions[0];
        let function = &lowered.functions[0];
        assert!(std::ptr::eq(function.source, source));
        for param in &source.params {
            let typed = &function.values[&param.value];
            assert!(
                matches!(typed.source_type, WasmSourceType::Mir(source_type) if source_type == &param.type_node)
            );
        }
        let by_name = source
            .params
            .iter()
            .map(|param| (param.name.as_str(), param.value))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            function.values[&by_name["out"]].physical,
            WasmPhysicalType::I32
        );
        assert_eq!(
            function.values[&by_name["i"]].physical,
            WasmPhysicalType::I32
        );
        assert_eq!(
            function.values[&by_name["wide"]].physical,
            WasmPhysicalType::I64
        );
        assert_eq!(
            function.values[&by_name["scale"]].physical,
            WasmPhysicalType::F64
        );
        assert_eq!(
            function.values[&by_name["flag"]].physical,
            WasmPhysicalType::I32
        );
        assert_eq!(
            function.values[&by_name["values"]].physical,
            WasmPhysicalType::I32Pair
        );

        assert_eq!(
            function
                .blocks
                .iter()
                .map(|block| block.source.id)
                .collect::<Vec<_>>(),
            source
                .blocks
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>()
        );
        for (lowered_block, source_block) in function.blocks.iter().zip(&source.blocks) {
            assert!(std::ptr::eq(lowered_block.source, source_block));
            assert_eq!(
                lowered_block
                    .instructions
                    .iter()
                    .map(|instruction| instruction.source.id)
                    .collect::<Vec<_>>(),
                source_block
                    .instructions
                    .iter()
                    .map(|instruction| instruction.id)
                    .collect::<Vec<_>>()
            );
            for instruction in &lowered_block.instructions {
                assert!(
                    source_block
                        .instructions
                        .iter()
                        .any(|original| std::ptr::eq(instruction.source, original))
                );
            }
        }

        let mut saw_effectful_store = false;
        let mut saw_strict_float = false;
        for block in &function.blocks {
            for instruction in &block.instructions {
                if matches!(
                    instruction.source.kind,
                    crate::KirInstructionKind::Store { .. }
                ) {
                    assert!(instruction.source.memory.is_some());
                    assert!(instruction.source.effect.is_some());
                    saw_effectful_store = true;
                }
                if matches!(
                    instruction.source.kind,
                    crate::KirInstructionKind::Binary {
                        semantics: crate::KirArithmeticSemantics::StrictFloat,
                        ..
                    }
                ) {
                    saw_strict_float = true;
                }
            }
            for edge in &block.edges {
                let source_edge = match &block.source.terminator {
                    crate::KirTerminator::Jump { edge } => edge,
                    crate::KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => match edge.arm {
                        0 => then_edge,
                        1 => else_edge,
                        _ => panic!("invalid lowered edge arm"),
                    },
                    crate::KirTerminator::Return { .. } => panic!("return has no edges"),
                };
                assert!(std::ptr::eq(edge.source, source_edge));
                assert_eq!(edge.source.memory_args, source_edge.memory_args);
            }
        }
        assert!(saw_effectful_store);
        assert!(saw_strict_float);
        assert!(function.values.values().any(|value| matches!(
            value.source_type,
            WasmSourceType::Mir(MirType::Slice(_))
                | WasmSourceType::Kir(KirValueType::Scalar(MirType::Slice(_)))
        )));
        assert!(function.values.values().any(|value| matches!(
            &value.source_type,
            WasmSourceType::Kir(KirValueType::Scalar(_))
        )));
    }
}
