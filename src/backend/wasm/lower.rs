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

pub(super) fn lower_wasm_module<'a>(
    module: &'a KirModule,
    contracts: Option<&ContractFactSet>,
    mir: &MirModule,
) -> Result<WasmLoweredModule<'a>, String> {
    if module.config.overflow_mode != KirOverflowMode::Unchecked
        || module.config.bounds_mode != KirBoundsMode::Unchecked
    {
        return Err("WebAssembly KIR backend accepts only unchecked KIR".to_string());
    }
    let features = module.profile.wasm_features().ok_or_else(|| {
        "WebAssembly KIR backend requires a WebAssembly target profile".to_string()
    })?;
    validate_vector_kir(module, features)?;
    let layout = WasmStructLayout::new(mir);
    Ok(WasmLoweredModule {
        source: module,
        functions: module
            .functions
            .iter()
            .map(|function| lower_wasm_function(function, contracts, &layout))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn lower_wasm_function<'a>(
    function: &'a KirFunction,
    contracts: Option<&ContractFactSet>,
    layout: &WasmStructLayout,
) -> Result<WasmLoweredFunction<'a>, String> {
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
            let kir_type = kir_types
                .get(&id)
                .ok_or_else(|| format!("WebAssembly KIR has no type for value {}", id.index()))?;
            let operand = match source_type {
                WasmSourceType::Mir(_) | WasmSourceType::Kir(KirValueType::Scalar(_)) => {
                    Some(super::kir::mir_value(id, &types, &params))
                }
                WasmSourceType::Kir(KirValueType::FixedVector { .. }) => None,
                WasmSourceType::Kir(KirValueType::Mask { .. }) => None,
            };
            Ok((
                id,
                WasmTypedValue {
                    value: id,
                    source_type,
                    operand,
                    physical: wasm_physical_type(source_type, kir_type)?,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
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
            let leaves = adapt_instruction(instruction, &types, &params)?;
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
) -> Result<WasmLoweredEdge<'a>, String> {
    let target = blocks.get(&edge.target).ok_or_else(|| {
        format!(
            "WebAssembly KIR edge targets unknown block {}",
            edge.target.index()
        )
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
) -> Result<WasmPhysicalType, String> {
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
        _ => Err("WebAssembly KIR typed value source metadata is inconsistent".to_string()),
    }
}

fn wasm_scalar_physical_type(type_node: &MirType) -> Result<WasmPhysicalType, String> {
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
        MirType::Struct(_) | MirType::Void => Err(format!(
            "WebAssembly KIR backend cannot lower value type {type_node:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode,
        KirValueType, MirType, SourceFile, build_kir_module, check, lower_to_mir,
    };

    use super::{WasmPhysicalType, WasmSourceType, lower_wasm_module};

    fn test_kir() -> crate::KirModule {
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
        build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("valid unchecked WebAssembly KIR")
    }

    #[test]
    fn wasm_lowering_should_retain_typed_values_sources_effects_edges_and_order() {
        let module = test_kir();
        let mir = crate::MirModule {
            entry: module.entry.clone(),
            structs: module.structs.clone(),
            functions: Vec::new(),
        };
        let lowered = lower_wasm_module(&module, None, &mir).expect("typed lowering");
        assert_eq!(lowered.source.profile, module.profile);
        assert_eq!(lowered.source.config, module.config);

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
