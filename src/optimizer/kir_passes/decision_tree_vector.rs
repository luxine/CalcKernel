use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CandidateBudgetCharge, DecisionTreeSelectMapping, DecisionTreeVectorPlan, FactUseSite,
    KirAlignmentClass, KirArithmeticSemantics, KirBlock, KirBlockParam, KirCostSemantics, KirEdge,
    KirEffectKind, KirInstruction, KirInstructionKind, KirLaneType, KirMemoryAccess,
    KirMemoryBlockParam, KirOrderedEffect, KirPreStateIdentity, KirProfileOperation, KirResult,
    KirValueType, KirVectorBinaryOp, KirVectorMemoryAccess, KirVectorRegion, KirVectorUnaryOp,
    KirVerifiedProgramState, KirVersionPredicate, KirVersionPredicateConjunct, MemoryRegionId,
    MemoryVersionId, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, MirUnaryOp,
    ProofStep, ProofStepId, ScalarClaim, ScalarFailure, ScalarInterval, VectorEpilogue,
    VectorLaneMapping, VectorMemoryAccessKind, VectorMemoryGroup, VectorOperationMapping,
    VectorPlanGrowth, VectorPredicate, VectorProofRoots, VectorizationPlan,
    WasmDecisionTreeCandidate, WasmDecisionTreeNode, WasmRangeCount, kir_function_units,
};

#[derive(Debug, Clone)]
pub struct MaterializedDecisionTreeVector {
    pub trial: KirVerifiedProgramState,
    pub plan: DecisionTreeVectorPlan,
    pub charge: CandidateBudgetCharge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MappedValue {
    value: crate::ValueId,
    vector: bool,
}

type ChildMapState = (
    BTreeMap<crate::ValueId, MappedValue>,
    BTreeSet<crate::ValueId>,
    BTreeMap<crate::ValueId, crate::ValueId>,
    BTreeMap<MemoryVersionId, MemoryVersionId>,
);

#[derive(Debug, Clone)]
struct LeafState {
    invariant_origins: BTreeMap<crate::ValueId, crate::ValueId>,
    memory_origins: BTreeMap<MemoryVersionId, MemoryVersionId>,
    block: crate::BlockId,
    unroll_index: u8,
}

struct TreeEmitter<'a> {
    original: &'a crate::KirFunction,
    candidate: &'a WasmDecisionTreeCandidate,
    trial: &'a mut KirVerifiedProgramState,
    region: crate::VectorRegionId,
    body: Vec<KirInstruction>,
    scalar_splats: BTreeMap<crate::ValueId, crate::ValueId>,
    emitted_operations: BTreeMap<
        (crate::InstructionId, u8),
        (crate::InstructionId, KirProfileOperation, KirCostSemantics),
    >,
    emitted_selects: BTreeMap<(crate::BlockId, u8), crate::InstructionId>,
    vector_loads: Vec<crate::InstructionId>,
    vector_stores: Vec<crate::InstructionId>,
    unroll_index: u8,
    effect_order: u32,
    leaves: Vec<LeafState>,
}

/// Constructs a guarded VF2/UF1 strict SIMD trial for a closed decision tree.
///
/// The source scalar loop remains intact as both the failed-guard path and the
/// vector loop's scalar tail. Every unsupported shape fails closed.
pub fn prepare_decision_tree_vector_trial(
    pre_state: &KirVerifiedProgramState,
    candidate: &WasmDecisionTreeCandidate,
) -> Result<MaterializedDecisionTreeVector, String> {
    preflight(pre_state, candidate)?;
    let original = pre_state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "decision-tree function is missing".to_string())?
        .clone();
    let source_preheader = block(&original, candidate.preheader)?.clone();
    let source_header = block(&original, candidate.header)?.clone();
    let source_root = block(&original, candidate.root)?.clone();
    let input_access_slice = descriptor_origin_slice(
        &original,
        candidate.input_region,
        candidate.input_partition,
        candidate.input_slice,
    )?;
    let output_access_slice = descriptor_origin_slice(
        &original,
        candidate.output_region,
        candidate.output_partition,
        candidate.output_slice,
    )?;
    let range_requirements = normalized_range_requirements(
        &original,
        candidate,
        input_access_slice,
        output_access_slice,
    )?;
    let entry = match &source_preheader.terminator {
        crate::KirTerminator::Jump { edge } if edge.target == candidate.header => edge.clone(),
        _ => return Err("decision-tree preheader does not enter its source header".to_string()),
    };
    let body_edge = match &source_header.terminator {
        crate::KirTerminator::Branch { then_edge, .. } if then_edge.target == candidate.root => {
            then_edge.clone()
        }
        _ => return Err("decision-tree body is not the source loop's then edge".to_string()),
    };

    let mut trial = pre_state.clone();
    let mut transformed_preheader = source_preheader.clone();
    let entry_bound = materialize_entry_value(
        &original,
        &source_header,
        &source_preheader,
        &entry,
        candidate.bound,
        pre_state,
        &mut trial,
        &mut transformed_preheader,
        &mut BTreeSet::new(),
    )?;
    let mut entry_values = BTreeMap::new();
    let mut header_value_map = BTreeMap::new();
    let vector_header_id = trial.fresh_block()?;
    let vector_body_id = trial.fresh_block()?;
    let vector_region = trial.fresh_vector_region()?;

    let mut vector_header_params = Vec::with_capacity(source_header.params.len());
    for param in &source_header.params {
        let value = trial.fresh_value()?;
        header_value_map.insert(param.value, value);
        vector_header_params.push(KirBlockParam {
            value,
            slot: format!("simd_{}", param.slot),
            type_node: param.type_node.clone(),
        });
        let index = source_header
            .params
            .iter()
            .position(|candidate_param| candidate_param.value == param.value)
            .ok_or_else(|| "decision-tree header parameter index is missing".to_string())?;
        let source_entry_value = *entry
            .args
            .get(index)
            .ok_or_else(|| "decision-tree preheader value edge is incomplete".to_string())?;
        let mapped = materialize_source_edge_value(
            &original,
            &source_header,
            &source_preheader,
            &entry,
            source_entry_value,
            pre_state,
            &mut trial,
            &mut transformed_preheader,
        )?;
        entry_values.insert(param.value, mapped);
    }

    let mut vector_header_memory = Vec::with_capacity(source_header.memory_params.len());
    let mut header_memory_map = BTreeMap::new();
    for param in &source_header.memory_params {
        let version = trial.fresh_memory_version()?;
        header_memory_map.insert(param.version, version);
        vector_header_memory.push(KirMemoryBlockParam {
            version,
            region: param.region,
        });
    }

    let root_body_args = remap_edge_args(
        &original,
        &body_edge,
        &source_root,
        &header_value_map,
        &entry_values,
        &source_header,
        &source_preheader,
        pre_state,
        &mut trial,
        &mut transformed_preheader,
    )?;
    let mut vector_body_params = Vec::with_capacity(source_root.params.len());
    let mut root_values = BTreeMap::new();
    let mut root_iv_aliases = BTreeSet::new();
    let mut root_invariant_origins = BTreeMap::new();
    for (index, param) in source_root.params.iter().enumerate() {
        let value = trial.fresh_value()?;
        vector_body_params.push(KirBlockParam {
            value,
            slot: format!("simd_root_{}", param.slot),
            type_node: param.type_node.clone(),
        });
        let mapped = *root_body_args
            .get(index)
            .ok_or_else(|| "decision-tree root edge does not match its parameters".to_string())?;
        root_values.insert(
            param.value,
            MappedValue {
                value,
                vector: false,
            },
        );
        if source_value_is_header_iv(&source_header, &body_edge, index, candidate.induction) {
            root_iv_aliases.insert(param.value);
        }
        if let Some(origin) = source_value_header_origin(&source_header, &body_edge, index) {
            root_invariant_origins.insert(param.value, origin);
        }
        if let Some(source_header_param) = source_header
            .params
            .iter()
            .find(|header_param| body_edge.args.get(index) == Some(&header_param.value))
        {
            root_values.insert(
                source_header_param.value,
                MappedValue {
                    value,
                    vector: false,
                },
            );
            if source_header_param.value == candidate.induction {
                root_iv_aliases.insert(source_header_param.value);
            } else {
                root_invariant_origins.insert(source_header_param.value, source_header_param.value);
            }
        }
        if mapped.vector && param.value != candidate.root_induction {
            return Err("decision-tree root carries a vector value as scalar state".to_string());
        }
    }
    if !root_values.contains_key(&candidate.root_induction) {
        return Err("decision-tree root induction parameter is missing".to_string());
    }
    let body_induction = root_values[&candidate.root_induction].value;
    root_values.insert(
        candidate.induction,
        MappedValue {
            value: body_induction,
            vector: false,
        },
    );
    root_iv_aliases.insert(candidate.root_induction);
    root_iv_aliases.insert(candidate.induction);

    let header_index = source_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .ok_or_else(|| "decision-tree induction header parameter is missing".to_string())?;
    let root_induction_index = source_root
        .params
        .iter()
        .position(|param| param.value == candidate.root_induction)
        .ok_or_else(|| "decision-tree root induction parameter is missing".to_string())?;
    if body_edge.args.get(root_induction_index) != Some(&candidate.induction)
        || entry.args.get(header_index).is_none()
    {
        return Err("decision-tree induction is not forwarded exactly into its root".to_string());
    }

    let root_memory_origins = source_root
        .memory_params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let source = *body_edge
                .memory_args
                .get(index)
                .ok_or_else(|| "decision-tree root memory edge is incomplete".to_string())?;
            let mapped = header_memory_map.get(&source).copied().ok_or_else(|| {
                "decision-tree root memory input is not a vector-header parameter".to_string()
            })?;
            Ok((param.version, mapped))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let body = Vec::new();
    let zero = preheader_u32_constant(&mut trial, &mut transformed_preheader, 0)?;
    let _ = zero;
    let vector_width = 2_u32.saturating_mul(u32::from(candidate.uf));
    let stride = preheader_u32_constant(&mut trial, &mut transformed_preheader, vector_width)?;
    let vector_limit = trial.fresh_value()?;
    transformed_preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: vector_limit,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            left: entry_bound,
            right: stride,
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });

    let (loop_condition, loop_compare) = make_vector_loop_condition(
        &mut trial,
        header_value_map[&candidate.induction],
        vector_limit,
    )?;
    let transformed_header = KirBlock {
        id: vector_header_id,
        label: "decision_tree_simd_header".to_string(),
        params: vector_header_params,
        memory_params: vector_header_memory,
        instructions: vec![loop_compare],
        terminator: crate::KirTerminator::Branch {
            condition: loop_condition,
            then_edge: KirEdge {
                target: vector_body_id,
                args: root_body_args.iter().map(|mapped| mapped.value).collect(),
                memory_args: Vec::new(),
            },
            else_edge: KirEdge {
                target: candidate.header,
                args: source_header
                    .params
                    .iter()
                    .map(|param| header_value_map[&param.value])
                    .collect(),
                memory_args: source_header
                    .memory_params
                    .iter()
                    .map(|param| header_memory_map[&param.version])
                    .collect(),
            },
        },
    };
    let mut conjuncts = vec![KirVersionPredicateConjunct::TripThreshold {
        value: entry_bound,
        minimum: candidate.minimum_trip,
    }];
    for requirement in &range_requirements {
        let slice = materialize_entry_value(
            &original,
            &source_header,
            &source_preheader,
            &entry,
            requirement.slice,
            pre_state,
            &mut trial,
            &mut transformed_preheader,
            &mut BTreeSet::new(),
        )?;
        let start = match requirement.start {
            Some(value) => materialize_entry_value(
                &original,
                &source_header,
                &source_preheader,
                &entry,
                value,
                pre_state,
                &mut trial,
                &mut transformed_preheader,
                &mut BTreeSet::new(),
            )?,
            None => preheader_u32_constant(&mut trial, &mut transformed_preheader, 0)?,
        };
        let count = match requirement.count {
            WasmRangeCount::TripBound(bound) => {
                if bound != candidate.bound {
                    return Err("decision-tree range count differs from the loop bound".to_string());
                }
                entry_bound
            }
            WasmRangeCount::One => {
                preheader_u32_constant(&mut trial, &mut transformed_preheader, 1)?
            }
            WasmRangeCount::Invariant(_) | WasmRangeCount::ScaledInvariant { .. } => {
                return Err(
                    "decision-tree guard accepts only its exact trip-bound extent".to_string(),
                );
            }
        };
        conjuncts.push(KirVersionPredicateConjunct::WasmSliceRange {
            slice,
            start,
            count,
            element_bytes: requirement.element_bytes,
        });
    }
    if conjuncts.len() != 3 {
        return Err(
            "decision-tree guard must contain one threshold and two full ranges".to_string(),
        );
    }
    let guard = trial.fresh_value()?;
    transformed_preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: guard,
            type_node: MirType::Primitive(MirPrimitiveTypeName::Bool).into(),
        }],
        kind: KirInstructionKind::VersionPredicate {
            predicate: KirVersionPredicate {
                address_bits: 32,
                conjuncts,
            },
        },
        memory: None,
        effect: None,
    });

    let entry_header_args = source_header
        .params
        .iter()
        .map(|param| {
            let value = entry_values
                .get(&param.value)
                .copied()
                .ok_or_else(|| "decision-tree entry value mapping is incomplete".to_string())?;
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    transformed_preheader.terminator = crate::KirTerminator::Branch {
        condition: guard,
        then_edge: KirEdge {
            target: vector_header_id,
            args: entry_header_args,
            memory_args: entry.memory_args.clone(),
        },
        else_edge: entry,
    };

    let effect_order = original
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| instruction.effect.as_ref().map(|effect| effect.order))
        .chain(
            original
                .blocks
                .iter()
                .filter_map(|block| match block.terminator {
                    crate::KirTerminator::Return { effect_order, .. } => Some(effect_order),
                    _ => None,
                }),
        )
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let root_load = instruction(&original, candidate.root_load)?;
    let load_memory = root_load
        .memory
        .as_ref()
        .ok_or_else(|| "decision-tree root load lacks MemorySSA".to_string())?;
    let vector_load_memory = root_memory_origins
        .get(&load_memory.input)
        .copied()
        .or_else(|| header_memory_map.get(&load_memory.input).copied())
        .ok_or_else(|| {
            "decision-tree input memory does not map to the vector header".to_string()
        })?;
    let body_input_iv = root_values[&candidate.root_induction].value;
    let mut emitter = TreeEmitter {
        original: &original,
        candidate,
        trial: &mut trial,
        region: vector_region,
        body,
        scalar_splats: BTreeMap::new(),
        emitted_operations: BTreeMap::new(),
        emitted_selects: BTreeMap::new(),
        vector_loads: Vec::new(),
        vector_stores: Vec::new(),
        unroll_index: 0,
        effect_order,
        leaves: Vec::new(),
    };
    let output_header_memory = source_header
        .memory_params
        .iter()
        .find(|param| param.region == candidate.output_partition)
        .map(|param| header_memory_map[&param.version])
        .ok_or_else(|| "decision-tree output partition is not a header memory phi".to_string())?;
    let mut current_chunk_iv = body_input_iv;
    let mut output_memory = output_header_memory;
    for unroll_index in 0..candidate.uf {
        if unroll_index > 0 {
            let offset = emitter.trial.fresh_value()?;
            emitter.body.push(KirInstruction {
                id: emitter.trial.fresh_instruction()?,
                results: vec![KirResult {
                    value: offset,
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                }],
                kind: KirInstructionKind::ConstInt {
                    value: "2".to_string(),
                },
                memory: None,
                effect: None,
            });
            let next_chunk_iv = emitter.trial.fresh_value()?;
            emitter.body.push(KirInstruction {
                id: emitter.trial.fresh_instruction()?,
                results: vec![KirResult {
                    value: next_chunk_iv,
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
                }],
                kind: KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left: current_chunk_iv,
                    right: offset,
                    semantics: KirArithmeticSemantics::Modular,
                },
                memory: None,
                effect: None,
            });
            current_chunk_iv = next_chunk_iv;
        }
        emitter.unroll_index = unroll_index;
        let mut chunk_values = root_values.clone();
        chunk_values.insert(
            candidate.root_induction,
            MappedValue {
                value: current_chunk_iv,
                vector: false,
            },
        );
        chunk_values.insert(
            candidate.induction,
            MappedValue {
                value: current_chunk_iv,
                vector: false,
            },
        );
        let load_value = emitter.trial.fresh_value()?;
        let load_id = emitter.trial.fresh_instruction()?;
        emitter.body.push(KirInstruction {
            id: load_id,
            results: vec![KirResult {
                value: load_value,
                type_node: KirValueType::FixedVector {
                    lane: KirLaneType::F64,
                    lanes: 2,
                },
            }],
            kind: KirInstructionKind::VectorLoad {
                access: vector_access(input_access_slice, current_chunk_iv, entry_bound),
                region: vector_region,
            },
            memory: Some(KirMemoryAccess {
                region: candidate.input_partition,
                input: vector_load_memory,
                output: None,
            }),
            effect: Some(KirOrderedEffect {
                order: emitter.effect_order,
                kind: KirEffectKind::ReadMemory,
            }),
        });
        emitter.effect_order = emitter.effect_order.saturating_add(1);
        emitter.vector_loads.push(load_id);
        chunk_values.insert(
            candidate.root_load_value,
            MappedValue {
                value: load_value,
                vector: true,
            },
        );
        // Each unrolled chunk is priced independently; retain its own first-use
        // scalar splats instead of silently sharing setup across iterations.
        emitter.scalar_splats.clear();
        let tree_value = emitter.emit_node(
            &candidate.tree,
            chunk_values,
            root_iv_aliases.clone(),
            root_invariant_origins.clone(),
            root_memory_origins.clone(),
            &body_edge,
        )?;
        if !tree_value.vector {
            return Err("decision-tree did not produce vector values on all leaves".to_string());
        }
        let output_id = emitter.trial.fresh_instruction()?;
        let next_output_memory = emitter.trial.fresh_memory_version()?;
        emitter.body.push(KirInstruction {
            id: output_id,
            results: Vec::new(),
            kind: KirInstructionKind::VectorStore {
                access: vector_access(output_access_slice, current_chunk_iv, entry_bound),
                value: tree_value.value,
                region: vector_region,
            },
            memory: Some(KirMemoryAccess {
                region: candidate.output_partition,
                input: output_memory,
                output: Some(next_output_memory),
            }),
            effect: Some(KirOrderedEffect {
                order: emitter.effect_order,
                kind: KirEffectKind::WriteMemory,
            }),
        });
        emitter.effect_order = emitter.effect_order.saturating_add(1);
        emitter.vector_stores.push(output_id);
        output_memory = next_output_memory;
    }
    if emitter.leaves.is_empty() {
        return Err("decision-tree did not produce vector values on all leaves".to_string());
    }

    let next_induction = emitter.trial.fresh_value()?;
    let stride_body = emitter.trial.fresh_value()?;
    emitter.body.push(KirInstruction {
        id: emitter.trial.fresh_instruction()?,
        results: vec![KirResult {
            value: stride_body,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::ConstInt {
            value: vector_width.to_string(),
        },
        memory: None,
        effect: None,
    });
    emitter.body.push(KirInstruction {
        id: emitter.trial.fresh_instruction()?,
        results: vec![KirResult {
            value: next_induction,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: body_input_iv,
            right: stride_body,
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });

    let mut backedge_args = Vec::with_capacity(source_header.params.len());
    let mut backedge_memory = Vec::with_capacity(source_header.memory_params.len());
    let (source_join_edge, source_join) = join_shape(&original, candidate)?;
    let source_leaf_states = &emitter.leaves;
    check_leaf_backedges(
        &original,
        candidate,
        &source_header,
        source_join,
        source_join_edge,
        source_leaf_states,
        &header_memory_map,
    )?;
    for param in &source_header.params {
        if param.value == candidate.induction {
            backedge_args.push(next_induction);
        } else {
            backedge_args.push(header_value_map[&param.value]);
        }
    }
    for param in &source_header.memory_params {
        if param.region == candidate.output_partition {
            backedge_memory.push(output_memory);
        } else {
            backedge_memory.push(header_memory_map[&param.version]);
        }
    }

    let body_instructions = std::mem::take(&mut emitter.body);
    let vector_body = KirBlock {
        id: vector_body_id,
        label: "decision_tree_simd_body".to_string(),
        params: vector_body_params,
        memory_params: Vec::new(),
        instructions: body_instructions,
        terminator: crate::KirTerminator::Jump {
            edge: KirEdge {
                target: vector_header_id,
                args: backedge_args,
                memory_args: backedge_memory,
            },
        },
    };
    let emitted_operations = emitter.emitted_operations.clone();
    let emitted_selects = emitter.emitted_selects.clone();
    let vector_load_instructions = emitter.vector_loads.clone();
    let vector_store_instructions = emitter.vector_stores.clone();
    drop(emitter);
    let roots = insert_proofs(&mut trial, candidate)?;
    let operations = plan_operations(&original, candidate, &emitted_operations)?;
    let selects = emitted_selects
        .iter()
        .map(
            |((source_branch, unroll_index), vector_select)| DecisionTreeSelectMapping {
                source_branch: *source_branch,
                vector_select: *vector_select,
                unroll_index: *unroll_index,
            },
        )
        .collect::<Vec<_>>();
    if selects.len() != emitted_selects.len()
        || vector_load_instructions.len() != usize::from(candidate.uf)
        || vector_store_instructions.len() != usize::from(candidate.uf)
    {
        return Err("decision-tree vector instruction mapping is incomplete".to_string());
    }
    let stores = tree_store_ids(&candidate.tree);
    let memory_groups = (0..candidate.uf)
        .flat_map(|unroll_index| {
            [
                VectorMemoryGroup {
                    region: candidate.input_partition,
                    access: VectorMemoryAccessKind::Read,
                    scalar_instructions: vec![candidate.root_load],
                    vector_instruction: vector_load_instructions[usize::from(unroll_index)],
                    unroll_index,
                    footprint_proof: roots.operation_equivalence,
                },
                VectorMemoryGroup {
                    region: candidate.output_partition,
                    access: VectorMemoryAccessKind::Write,
                    scalar_instructions: stores.clone(),
                    vector_instruction: vector_store_instructions[usize::from(unroll_index)],
                    unroll_index,
                    footprint_proof: roots.operation_equivalence,
                },
            ]
        })
        .collect();
    let predicates = decision_tree_predicates(candidate, &range_requirements, &roots);
    let before_function = kir_function_units(&original);
    let before_module = module_units(pre_state.module());
    let transformed = trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "decision-tree trial function disappeared".to_string())?;
    let preheader = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .ok_or_else(|| "decision-tree preheader disappeared".to_string())?;
    *preheader = transformed_preheader;
    transformed.vector_regions.push(KirVectorRegion {
        id: vector_region,
        blocks: vec![vector_body_id],
    });
    transformed.blocks.push(transformed_header);
    transformed.blocks.push(vector_body);
    let after_function = kir_function_units(transformed);
    let after_module = before_module
        .saturating_sub(before_function)
        .saturating_add(after_function);
    let vector_plan = VectorizationPlan {
        pre_state: KirPreStateIdentity {
            function: candidate.function,
            kir_digest: pre_state.kir_digest(),
            profile_digest: pre_state.module().profile.digest_hex(),
            evidence_generation: pre_state.evidence_generation(),
            frozen_kir_units: before_function,
        },
        loop_id: candidate.loop_id,
        vf: 2,
        uf: candidate.uf,
        operations,
        memory_groups,
        broadcast_groups: Vec::new(),
        predicates,
        epilogue: VectorEpilogue::Scalar {
            start: candidate.induction,
            end: candidate.bound,
            coverage_proof: roots.trip_partition,
        },
        cost: candidate.predicted_cost,
        growth: VectorPlanGrowth::new(before_function, after_function, before_module, after_module),
        proofs: roots,
    };
    let plan = DecisionTreeVectorPlan {
        vector: vector_plan,
        selects,
    };
    let charge = decision_tree_charge(&plan);
    Ok(MaterializedDecisionTreeVector {
        trial,
        plan,
        charge,
    })
}

impl TreeEmitter<'_> {
    fn emit_node(
        &mut self,
        node: &WasmDecisionTreeNode,
        mut values: BTreeMap<crate::ValueId, MappedValue>,
        mut iv_aliases: BTreeSet<crate::ValueId>,
        mut invariant_origins: BTreeMap<crate::ValueId, crate::ValueId>,
        memory_origins: BTreeMap<MemoryVersionId, MemoryVersionId>,
        incoming: &KirEdge,
    ) -> Result<MappedValue, String> {
        let current_block = match node {
            WasmDecisionTreeNode::Branch { block, .. }
            | WasmDecisionTreeNode::Leaf { block, .. } => *block,
        };
        if current_block != incoming.target {
            return Err("decision-tree node does not match its predecessor edge".to_string());
        }
        let source_block = block(self.original, current_block)?.clone();
        let node_dag = self
            .candidate
            .ordered_tree_dag
            .iter()
            .filter(|id| {
                source_block
                    .instructions
                    .iter()
                    .any(|instruction| instruction.id == **id)
            })
            .copied()
            .collect::<Vec<_>>();
        let node_ids = node_dag.iter().copied().collect::<BTreeSet<_>>();
        for instruction in &source_block.instructions {
            if !node_ids.contains(&instruction.id) {
                continue;
            }
            self.emit_pure_instruction(instruction, &mut values)?;
            if let Some(result) = instruction.results.first().map(|result| result.value)
                && let KirInstructionKind::Copy { value } = &instruction.kind
            {
                if iv_aliases.contains(value) {
                    iv_aliases.insert(result);
                }
                if let Some(origin) = invariant_origins.get(value).copied() {
                    invariant_origins.insert(result, origin);
                }
            }
        }
        match node {
            WasmDecisionTreeNode::Branch {
                block: branch,
                condition,
                then_edge,
                else_edge,
                then_node,
                else_node,
                ..
            } => {
                let condition = values
                    .get(condition)
                    .copied()
                    .ok_or_else(|| "decision-tree vector comparison is unmapped".to_string())?;
                if !condition.vector {
                    return Err("decision-tree branch condition is not a vector mask".to_string());
                }
                let then_values = self.enter_child(
                    then_edge,
                    &values,
                    &iv_aliases,
                    &invariant_origins,
                    &memory_origins,
                )?;
                let when_true = self.emit_node(
                    then_node,
                    then_values.0,
                    then_values.1,
                    then_values.2,
                    then_values.3,
                    then_edge,
                )?;
                let else_values = self.enter_child(
                    else_edge,
                    &values,
                    &iv_aliases,
                    &invariant_origins,
                    &memory_origins,
                )?;
                let when_false = self.emit_node(
                    else_node,
                    else_values.0,
                    else_values.1,
                    else_values.2,
                    else_values.3,
                    else_edge,
                )?;
                if !when_true.vector || !when_false.vector {
                    return Err("decision-tree select arm is not a vector value".to_string());
                }
                let select_value = self.trial.fresh_value()?;
                let select_instruction = self.trial.fresh_instruction()?;
                self.body.push(KirInstruction {
                    id: select_instruction,
                    results: vec![KirResult {
                        value: select_value,
                        type_node: KirValueType::FixedVector {
                            lane: KirLaneType::F64,
                            lanes: 2,
                        },
                    }],
                    kind: KirInstructionKind::VectorSelect {
                        mask: condition.value,
                        when_true: when_true.value,
                        when_false: when_false.value,
                        region: self.region,
                    },
                    memory: None,
                    effect: None,
                });
                self.emitted_selects
                    .insert((*branch, self.unroll_index), select_instruction);
                Ok(MappedValue {
                    value: select_value,
                    vector: true,
                })
            }
            WasmDecisionTreeNode::Leaf {
                value,
                store,
                induction_update,
                induction_result,
                join_edge,
                ..
            } => {
                self.verify_leaf_induction(
                    *induction_update,
                    *induction_result,
                    &values,
                    &iv_aliases,
                    join_edge,
                )?;
                let mapped = values
                    .get(value)
                    .copied()
                    .or_else(|| self.map_invariant_value(*value, &values).ok())
                    .ok_or_else(|| "decision-tree leaf result is unmapped".to_string())?;
                if !mapped.vector {
                    return Err("decision-tree leaf result did not vectorize".to_string());
                }
                let source_store = instruction(self.original, *store)?;
                if !matches!(source_store.kind, KirInstructionKind::Store { .. })
                    || source_store
                        .memory
                        .as_ref()
                        .is_none_or(|memory| memory.output.is_none())
                {
                    return Err("decision-tree leaf store is malformed".to_string());
                }
                self.leaves.push(LeafState {
                    invariant_origins,
                    memory_origins,
                    block: current_block,
                    unroll_index: self.unroll_index,
                });
                Ok(mapped)
            }
        }
    }

    fn enter_child(
        &self,
        edge: &KirEdge,
        parent_values: &BTreeMap<crate::ValueId, MappedValue>,
        parent_ivs: &BTreeSet<crate::ValueId>,
        parent_origins: &BTreeMap<crate::ValueId, crate::ValueId>,
        parent_memories: &BTreeMap<MemoryVersionId, MemoryVersionId>,
    ) -> Result<ChildMapState, String> {
        let child = block(self.original, edge.target)?;
        if edge.args.len() != child.params.len()
            || edge.memory_args.len() != child.memory_params.len()
        {
            return Err("decision-tree child edge does not match its parameters".to_string());
        }
        let mut values = parent_values.clone();
        let mut ivs = parent_ivs.clone();
        let mut origins = parent_origins.clone();
        for (param, source) in child.params.iter().zip(&edge.args) {
            let mapped = parent_values
                .get(source)
                .copied()
                .or_else(|| self.map_invariant_value(*source, parent_values).ok())
                .ok_or_else(|| "decision-tree block argument is unavailable".to_string())?;
            values.insert(param.value, mapped);
            if parent_ivs.contains(source) {
                ivs.insert(param.value);
            }
            if let Some(origin) = parent_origins.get(source).copied() {
                origins.insert(param.value, origin);
            }
        }
        let mut memories = parent_memories.clone();
        for (param, source) in child.memory_params.iter().zip(&edge.memory_args) {
            let mapped = parent_memories
                .get(source)
                .copied()
                .ok_or_else(|| "decision-tree memory forwarding is not invariant".to_string())?;
            memories.insert(param.version, mapped);
        }
        Ok((values, ivs, origins, memories))
    }

    fn emit_pure_instruction(
        &mut self,
        instruction: &KirInstruction,
        values: &mut BTreeMap<crate::ValueId, MappedValue>,
    ) -> Result<(), String> {
        let result = match instruction.results.as_slice() {
            [result] => result,
            _ => return Err("decision-tree DAG instruction result arity differs".to_string()),
        };
        let mapped = match &instruction.kind {
            KirInstructionKind::ConstInt { value } => {
                let fresh = self.trial.fresh_value()?;
                self.body.push(KirInstruction {
                    id: self.trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: result.type_node.clone(),
                    }],
                    kind: KirInstructionKind::ConstInt {
                        value: value.clone(),
                    },
                    memory: None,
                    effect: None,
                });
                MappedValue {
                    value: fresh,
                    vector: false,
                }
            }
            KirInstructionKind::ConstFloat { value } => {
                let fresh = self.trial.fresh_value()?;
                self.body.push(KirInstruction {
                    id: self.trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: result.type_node.clone(),
                    }],
                    kind: KirInstructionKind::ConstFloat {
                        value: value.clone(),
                    },
                    memory: None,
                    effect: None,
                });
                MappedValue {
                    value: fresh,
                    vector: false,
                }
            }
            KirInstructionKind::ConstBool { value } => {
                let fresh = self.trial.fresh_value()?;
                self.body.push(KirInstruction {
                    id: self.trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: result.type_node.clone(),
                    }],
                    kind: KirInstructionKind::ConstBool { value: *value },
                    memory: None,
                    effect: None,
                });
                MappedValue {
                    value: fresh,
                    vector: false,
                }
            }
            KirInstructionKind::Copy { value } => self.mapped_scalar_or_vector(*value, values)?,
            KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics: KirArithmeticSemantics::StrictFloat,
            } => {
                let vector_op = vector_binary(*op)?;
                let left = self.vector_operand(*left, values)?;
                let right = self.vector_operand(*right, values)?;
                let fresh = self.trial.fresh_value()?;
                let id = self.trial.fresh_instruction()?;
                self.body.push(KirInstruction {
                    id,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: KirValueType::FixedVector {
                            lane: KirLaneType::F64,
                            lanes: 2,
                        },
                    }],
                    kind: KirInstructionKind::VectorBinary {
                        op: vector_op,
                        left,
                        right,
                        semantics: KirArithmeticSemantics::StrictFloat,
                        no_failure_proof: None,
                        region: self.region,
                    },
                    memory: None,
                    effect: None,
                });
                let profile_op = match op {
                    MirBinaryOp::Add => KirProfileOperation::Add,
                    MirBinaryOp::Sub => KirProfileOperation::Subtract,
                    MirBinaryOp::Mul => KirProfileOperation::Multiply,
                    _ => return Err("decision-tree operation is not allowed to divide".to_string()),
                };
                self.emitted_operations.insert(
                    (instruction.id, self.unroll_index),
                    (id, profile_op, KirCostSemantics::StrictFloat),
                );
                MappedValue {
                    value: fresh,
                    vector: true,
                }
            }
            KirInstructionKind::Unary {
                op: MirUnaryOp::Neg,
                operand,
                semantics: KirArithmeticSemantics::StrictFloat,
            } => {
                let operand = self.vector_operand(*operand, values)?;
                let fresh = self.trial.fresh_value()?;
                let id = self.trial.fresh_instruction()?;
                self.body.push(KirInstruction {
                    id,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: KirValueType::FixedVector {
                            lane: KirLaneType::F64,
                            lanes: 2,
                        },
                    }],
                    kind: KirInstructionKind::VectorUnary {
                        op: KirVectorUnaryOp::Negate,
                        operand,
                        semantics: KirArithmeticSemantics::StrictFloat,
                        no_failure_proof: None,
                        region: self.region,
                    },
                    memory: None,
                    effect: None,
                });
                self.emitted_operations.insert(
                    (instruction.id, self.unroll_index),
                    (
                        id,
                        KirProfileOperation::Negate,
                        KirCostSemantics::StrictFloat,
                    ),
                );
                MappedValue {
                    value: fresh,
                    vector: true,
                }
            }
            KirInstructionKind::Compare { op, left, right } => {
                let left = self.vector_operand(*left, values)?;
                let right = self.vector_operand(*right, values)?;
                let fresh = self.trial.fresh_value()?;
                let id = self.trial.fresh_instruction()?;
                self.body.push(KirInstruction {
                    id,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: KirValueType::Mask { lanes: 2 },
                    }],
                    kind: KirInstructionKind::VectorCompare {
                        op: *op,
                        left,
                        right,
                        region: self.region,
                    },
                    memory: None,
                    effect: None,
                });
                self.emitted_operations.insert(
                    (instruction.id, self.unroll_index),
                    (
                        id,
                        KirProfileOperation::Compare,
                        KirCostSemantics::NotApplicable,
                    ),
                );
                MappedValue {
                    value: fresh,
                    vector: true,
                }
            }
            _ => {
                return Err(
                    "decision-tree pure DAG contains an unsupported instruction".to_string()
                );
            }
        };
        values.insert(result.value, mapped);
        Ok(())
    }

    fn mapped_scalar_or_vector(
        &self,
        source: crate::ValueId,
        values: &BTreeMap<crate::ValueId, MappedValue>,
    ) -> Result<MappedValue, String> {
        values
            .get(&source)
            .copied()
            .or_else(|| self.map_invariant_value(source, values).ok())
            .ok_or_else(|| "decision-tree scalar operand is unavailable".to_string())
    }

    fn map_invariant_value(
        &self,
        source: crate::ValueId,
        values: &BTreeMap<crate::ValueId, MappedValue>,
    ) -> Result<MappedValue, String> {
        if let Some(mapped) = values.get(&source) {
            return Ok(*mapped);
        }
        if self
            .original
            .params
            .iter()
            .any(|param| param.value == source)
        {
            return Ok(MappedValue {
                value: source,
                vector: false,
            });
        }
        let dominators = crate::compute_kir_dominators(self.original);
        let definition = self
            .original
            .blocks
            .iter()
            .find(|block| {
                block.instructions.iter().any(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == source)
                })
            })
            .ok_or_else(|| "decision-tree value has no source definition".to_string())?;
        if !dominators.dominates(definition.id, self.candidate.preheader)
            || definition
                .instructions
                .iter()
                .find(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == source)
                })
                .is_none_or(|instruction| {
                    instruction.memory.is_some()
                        || instruction.effect.is_some()
                        || !matches!(
                            instruction.kind,
                            KirInstructionKind::ConstInt { .. }
                                | KirInstructionKind::ConstFloat { .. }
                                | KirInstructionKind::ConstBool { .. }
                                | KirInstructionKind::Copy { .. }
                        )
                })
        {
            return Err("decision-tree value is not an invariant scalar".to_string());
        }
        Ok(MappedValue {
            value: source,
            vector: false,
        })
    }

    fn vector_operand(
        &mut self,
        source: crate::ValueId,
        values: &BTreeMap<crate::ValueId, MappedValue>,
    ) -> Result<crate::ValueId, String> {
        let operand = self.mapped_scalar_or_vector(source, values)?;
        if operand.vector {
            return Ok(operand.value);
        }
        if let Some(vector) = self.scalar_splats.get(&operand.value).copied() {
            return Ok(vector);
        }
        let fresh = self.trial.fresh_value()?;
        self.body.push(KirInstruction {
            id: self.trial.fresh_instruction()?,
            results: vec![KirResult {
                value: fresh,
                type_node: KirValueType::FixedVector {
                    lane: KirLaneType::F64,
                    lanes: 2,
                },
            }],
            kind: KirInstructionKind::VectorSplat {
                scalar: operand.value,
                region: self.region,
            },
            memory: None,
            effect: None,
        });
        self.scalar_splats.insert(operand.value, fresh);
        Ok(fresh)
    }

    fn verify_leaf_induction(
        &self,
        update_id: crate::InstructionId,
        update_result: crate::ValueId,
        values: &BTreeMap<crate::ValueId, MappedValue>,
        iv_aliases: &BTreeSet<crate::ValueId>,
        join_edge: &KirEdge,
    ) -> Result<(), String> {
        let update = instruction(self.original, update_id)?;
        let KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } = update.kind
        else {
            return Err(
                "decision-tree leaf does not increment its induction modularly".to_string(),
            );
        };
        if update.memory.is_some()
            || update.effect.is_some()
            || update.results.as_slice().first().map(|result| result.value) != Some(update_result)
        {
            return Err("decision-tree induction update has effects or a false result".to_string());
        }
        let left_iv = iv_aliases.contains(&left);
        let right_iv = iv_aliases.contains(&right);
        if left_iv == right_iv || !(is_one(self.original, left) || is_one(self.original, right)) {
            return Err(
                "decision-tree induction update is not exactly source IV plus one".to_string(),
            );
        }
        let _ = values;
        if !join_edge.args.contains(&update_result) {
            return Err(
                "decision-tree leaf does not forward its increment to the latch".to_string(),
            );
        }
        Ok(())
    }
}

fn preflight(
    state: &KirVerifiedProgramState,
    candidate: &WasmDecisionTreeCandidate,
) -> Result<(), String> {
    let module = state.module();
    if module.config.consumer != crate::KirConsumer::WebAssembly
        || module.profile.wasm_features() != Some(crate::KirWasmFeatures::Simd128)
        || module.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || module.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || module.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
        || candidate.vf != 2
        || !matches!(candidate.uf, 1 | 4)
        || candidate.minimum_trip < 2 * u32::from(candidate.uf)
        || candidate.range_requirements.len() != 2
    {
        return Err(
            "decision-tree candidate has an unsupported target, mode, or VF/UF".to_string(),
        );
    }
    let original = module
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .ok_or_else(|| "decision-tree candidate function is missing".to_string())?;
    let header = block(original, candidate.header)?;
    let preheader = block(original, candidate.preheader)?;
    let root = block(original, candidate.root)?;
    let input_origin = descriptor_origin_slice(
        original,
        candidate.input_region,
        candidate.input_partition,
        candidate.input_slice,
    )?;
    let output_origin = descriptor_origin_slice(
        original,
        candidate.output_region,
        candidate.output_partition,
        candidate.output_slice,
    )?;
    let ranges = normalized_range_requirements(original, candidate, input_origin, output_origin)?;
    if ranges.iter().any(|requirement| {
        requirement.count != WasmRangeCount::TripBound(candidate.bound)
            || requirement.element_bytes != 8
            || requirement.start.is_some()
    }) {
        return Err(
            "decision-tree range proof is not an exact zero-based full trip range".to_string(),
        );
    }
    if !matches!(preheader.terminator, crate::KirTerminator::Jump { ref edge } if edge.target == candidate.header)
        || !matches!(header.terminator, crate::KirTerminator::Branch { ref then_edge, ref else_edge, .. } if then_edge.target == candidate.root && else_edge.target == candidate.exit)
    {
        return Err("decision-tree loop header/preheader CFG is not canonical".to_string());
    }
    let root_load = instruction(original, candidate.root_load)?;
    if !matches!(root_load.kind, KirInstructionKind::Load { .. })
        || root_load
            .results
            .as_slice()
            .first()
            .map(|result| result.value)
            != Some(candidate.root_load_value)
        || root_load.memory.as_ref().is_none_or(|memory| {
            memory.region != candidate.input_partition || memory.output.is_some()
        })
    {
        return Err("decision-tree root load identity or partition differs".to_string());
    }
    if !root
        .params
        .iter()
        .any(|param| param.value == candidate.root_induction)
    {
        return Err("decision-tree range or induction evidence is incomplete".to_string());
    }
    let mut tree_blocks = Vec::new();
    let mut stores = Vec::new();
    let mut branches = Vec::new();
    visit_nodes(
        &candidate.tree,
        &mut tree_blocks,
        &mut stores,
        &mut branches,
    );
    if tree_blocks != candidate.blocks
        || tree_blocks.iter().copied().collect::<BTreeSet<_>>().len() != tree_blocks.len()
        || !(2..=4).contains(&stores.len())
        || branches.is_empty()
        || branches.len() > 3
    {
        return Err(
            "decision-tree node identity or size differs from its closed shape".to_string(),
        );
    }
    let members = tree_blocks.iter().copied().collect::<BTreeSet<_>>();
    if members.contains(&candidate.preheader)
        || members.contains(&candidate.header)
        || members.contains(&candidate.join)
        || members.contains(&candidate.exit)
        || candidate
            .blocks
            .iter()
            .any(|id| *id == candidate.root && root.id != *id)
    {
        return Err("decision-tree node blocks overlap loop control".to_string());
    }
    let mut ordered = BTreeSet::new();
    for id in &candidate.ordered_tree_dag {
        if !ordered.insert(*id)
            || !members.iter().any(|block_id| {
                block(original, *block_id)
                    .is_ok_and(|block| block.instructions.iter().any(|item| item.id == *id))
            })
        {
            return Err(
                "decision-tree ordered pure DAG has a duplicate or external instruction"
                    .to_string(),
            );
        }
        let item = instruction(original, *id)?;
        if item.memory.is_some()
            || item.effect.is_some()
            || !matches!(
                item.kind,
                KirInstructionKind::ConstFloat { .. }
                    | KirInstructionKind::ConstInt { .. }
                    | KirInstructionKind::ConstBool { .. }
                    | KirInstructionKind::Copy { .. }
                    | KirInstructionKind::Binary {
                        semantics: KirArithmeticSemantics::StrictFloat,
                        op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
                        ..
                    }
                    | KirInstructionKind::Unary {
                        op: MirUnaryOp::Neg,
                        semantics: KirArithmeticSemantics::StrictFloat,
                        ..
                    }
                    | KirInstructionKind::Compare { .. }
            )
        {
            return Err(
                "decision-tree pure DAG has an unsupported or effectful instruction".to_string(),
            );
        }
    }
    for block_id in &candidate.blocks {
        let source = block(original, *block_id)?;
        if source.instructions.iter().any(|item| {
            item.memory.is_some() && item.id != candidate.root_load && !stores.contains(&item.id)
        }) {
            return Err("decision-tree node contains an unmodeled memory operation".to_string());
        }
    }
    let function_loop_cfg = original
        .blocks
        .iter()
        .filter(|source| {
            terminator_edges(&source.terminator)
                .iter()
                .any(|edge| members.contains(&edge.target))
        })
        .collect::<Vec<_>>();
    if function_loop_cfg.is_empty() {
        return Err("decision-tree node graph has no source predecessor".to_string());
    }
    let leaf_join = block(original, candidate.join)?;
    let crate::KirTerminator::Jump { edge: latch_edge } = &leaf_join.terminator else {
        return Err("decision-tree shared join is not a loop latch".to_string());
    };
    if latch_edge.target != candidate.header {
        return Err("decision-tree latch does not return to the source header".to_string());
    }
    if candidate.predicted_cost.scalar == 0
        || candidate.predicted_cost.total
            != candidate
                .predicted_cost
                .transformed_body
                .saturating_add(candidate.predicted_cost.predicates)
                .saturating_add(candidate.predicted_cost.epilogue)
    {
        return Err("decision-tree predicted cost is malformed".to_string());
    }
    Ok(())
}

fn visit_nodes(
    node: &WasmDecisionTreeNode,
    blocks: &mut Vec<crate::BlockId>,
    stores: &mut Vec<crate::InstructionId>,
    branches: &mut Vec<crate::BlockId>,
) {
    match node {
        WasmDecisionTreeNode::Branch {
            block,
            then_node,
            else_node,
            ..
        } => {
            blocks.push(*block);
            branches.push(*block);
            visit_nodes(then_node, blocks, stores, branches);
            visit_nodes(else_node, blocks, stores, branches);
        }
        WasmDecisionTreeNode::Leaf { block, store, .. } => {
            blocks.push(*block);
            stores.push(*store);
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The mapper must keep source and trial CFG state explicit while remapping one edge."
)]
fn remap_edge_args(
    original: &crate::KirFunction,
    edge: &KirEdge,
    target: &KirBlock,
    header_map: &BTreeMap<crate::ValueId, crate::ValueId>,
    entry_values: &BTreeMap<crate::ValueId, crate::ValueId>,
    header: &KirBlock,
    preheader: &KirBlock,
    state: &KirVerifiedProgramState,
    trial: &mut KirVerifiedProgramState,
    transformed_preheader: &mut KirBlock,
) -> Result<Vec<MappedValue>, String> {
    if edge.args.len() != target.params.len() {
        return Err("decision-tree root edge argument count differs".to_string());
    }
    let mut output = Vec::with_capacity(edge.args.len());
    for value in &edge.args {
        let mapped = if let Some(mapped) = header_map.get(value) {
            MappedValue {
                value: *mapped,
                vector: false,
            }
        } else if let Some(mapped) = entry_values.get(value) {
            MappedValue {
                value: *mapped,
                vector: false,
            }
        } else {
            MappedValue {
                value: materialize_source_edge_value(
                    original,
                    header,
                    preheader,
                    edge,
                    *value,
                    state,
                    trial,
                    transformed_preheader,
                )?,
                vector: false,
            }
        };
        output.push(mapped);
    }
    Ok(output)
}

#[expect(
    clippy::too_many_arguments,
    reason = "SSA forwarding is resolved against the original edge and a mutable trial preheader."
)]
fn materialize_source_edge_value(
    original: &crate::KirFunction,
    header: &KirBlock,
    preheader: &KirBlock,
    entry: &KirEdge,
    value: crate::ValueId,
    state: &KirVerifiedProgramState,
    trial: &mut KirVerifiedProgramState,
    transformed_preheader: &mut KirBlock,
) -> Result<crate::ValueId, String> {
    materialize_entry_value(
        original,
        header,
        preheader,
        entry,
        value,
        state,
        trial,
        transformed_preheader,
        &mut BTreeSet::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_entry_value(
    function: &crate::KirFunction,
    header: &KirBlock,
    preheader: &KirBlock,
    entry: &KirEdge,
    value: crate::ValueId,
    state: &KirVerifiedProgramState,
    trial: &mut KirVerifiedProgramState,
    transformed_preheader: &mut KirBlock,
    seen: &mut BTreeSet<crate::ValueId>,
) -> Result<crate::ValueId, String> {
    if !seen.insert(value) {
        return Err("decision-tree entry value forwarding is cyclic".to_string());
    }
    if let Some(index) = header.params.iter().position(|param| param.value == value) {
        return entry
            .args
            .get(index)
            .copied()
            .ok_or_else(|| "decision-tree header entry edge is incomplete".to_string());
    }
    if function.params.iter().any(|param| param.value == value)
        || preheader.params.iter().any(|param| param.value == value)
        || preheader.instructions.iter().any(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
    {
        return Ok(value);
    }
    if let Some((block_id, param_index)) = function.blocks.iter().find_map(|block| {
        block
            .params
            .iter()
            .position(|param| param.value == value)
            .map(|index| (block.id, index))
    }) {
        let predecessors = function
            .blocks
            .iter()
            .flat_map(|predecessor| {
                terminator_edges(&predecessor.terminator)
                    .into_iter()
                    .filter(move |edge| edge.target == block_id)
                    .map(move |edge| (predecessor.id, edge))
            })
            .collect::<Vec<_>>();
        if predecessors.len() == 1 {
            let (predecessor, edge) = predecessors[0];
            if predecessor == header.id {
                let source = *edge.args.get(param_index).ok_or_else(|| {
                    "decision-tree forwarded value edge is incomplete".to_string()
                })?;
                return materialize_entry_value(
                    function,
                    header,
                    preheader,
                    entry,
                    source,
                    state,
                    trial,
                    transformed_preheader,
                    seen,
                );
            }
        }
    }
    if let Some((definition_block, source_instruction)) = function.blocks.iter().find_map(|block| {
        block
            .instructions
            .iter()
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            })
            .map(|instruction| (block, instruction))
    }) {
        if crate::compute_kir_dominators(function).dominates(definition_block.id, preheader.id)
            && matches!(
                source_instruction.kind,
                KirInstructionKind::ConstInt { .. } | KirInstructionKind::SliceLen { .. }
            )
            && source_instruction.memory.is_none()
            && source_instruction.effect.is_none()
        {
            return Ok(value);
        }
        match &source_instruction.kind {
            KirInstructionKind::Copy { value: source } => {
                return materialize_entry_value(
                    function,
                    header,
                    preheader,
                    entry,
                    *source,
                    state,
                    trial,
                    transformed_preheader,
                    seen,
                );
            }
            KirInstructionKind::ConstInt { value: literal } => {
                let fresh = trial.fresh_value()?;
                transformed_preheader.instructions.push(KirInstruction {
                    id: trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: source_instruction.results[0].type_node.clone(),
                    }],
                    kind: KirInstructionKind::ConstInt {
                        value: literal.clone(),
                    },
                    memory: None,
                    effect: None,
                });
                return Ok(fresh);
            }
            KirInstructionKind::SliceLen { slice } => {
                let root = stable_descriptor_root(function, *slice)?;
                let fresh = trial.fresh_value()?;
                transformed_preheader.instructions.push(KirInstruction {
                    id: trial.fresh_instruction()?,
                    results: vec![KirResult {
                        value: fresh,
                        type_node: source_instruction.results[0].type_node.clone(),
                    }],
                    kind: KirInstructionKind::SliceLen { slice: root },
                    memory: None,
                    effect: None,
                });
                return Ok(fresh);
            }
            _ => {}
        }
    }
    let _ = state;
    let detail = function
        .blocks
        .iter()
        .find_map(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| {
                    instruction
                        .results
                        .iter()
                        .any(|result| result.value == value)
                })
                .map(|instruction| format!(" in b{} as {:?}", block.id.index(), instruction.kind))
        })
        .or_else(|| {
            function
                .blocks
                .iter()
                .find(|block| block.params.iter().any(|param| param.value == value))
                .map(|block| format!(" as parameter of b{}", block.id.index()))
        })
        .unwrap_or_default();
    Err(format!(
        "decision-tree source value v{}{} does not dominate its versioning preheader",
        value.index(),
        detail
    ))
}

fn stable_descriptor_root(
    function: &crate::KirFunction,
    value: crate::ValueId,
) -> Result<crate::ValueId, String> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut roots = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if function.params.iter().any(|param| param.value == current) {
            roots.insert(current);
            continue;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some((block_id, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == current)
                .map(|index| (block.id, index))
        }) {
            let mut incoming_count = 0_usize;
            for predecessor in &function.blocks {
                for edge in terminator_edges(&predecessor.terminator)
                    .into_iter()
                    .filter(|edge| edge.target == block_id)
                {
                    incoming_count = incoming_count.saturating_add(1);
                    pending.push(*edge.args.get(index).ok_or_else(|| {
                        "decision-tree descriptor phi edge is incomplete".to_string()
                    })?);
                }
            }
            if incoming_count == 0 {
                return Err("decision-tree descriptor phi has no incoming edge".to_string());
            }
            continue;
        }
        let definition = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == current)
            })
            .ok_or_else(|| "decision-tree descriptor value has no definition".to_string())?;
        match definition.kind {
            KirInstructionKind::Copy { value: source } => pending.push(source),
            KirInstructionKind::Subslice { .. } | KirInstructionKind::MakeSlice { .. } => {
                roots.insert(current);
            }
            _ => {
                return Err(
                    "decision-tree slice forwarding is not a closed descriptor chain".to_string(),
                );
            }
        }
    }
    if roots.len() != 1 {
        return Err("decision-tree slice has multiple descriptor origins".to_string());
    }
    Ok(*roots.first().expect("single descriptor root"))
}

fn descriptor_origin_slice(
    function: &crate::KirFunction,
    region: MemoryRegionId,
    partition: MemoryRegionId,
    source_slice: crate::ValueId,
) -> Result<crate::ValueId, String> {
    let descriptor = function
        .regions
        .iter()
        .find(|candidate| candidate.id == region)
        .ok_or_else(|| "decision-tree memory descriptor is missing".to_string())?;
    if descriptor.partition != partition {
        return Err("decision-tree place region and MemorySSA partition differ".to_string());
    }
    let origin = match descriptor.origin {
        crate::KirMemoryRegionOrigin::Parameter(value)
        | crate::KirMemoryRegionOrigin::RawSlice(value)
        | crate::KirMemoryRegionOrigin::Subslice(value) => value,
        crate::KirMemoryRegionOrigin::Conservative => {
            return Err("decision-tree vector access has no descriptor-origin slice".to_string());
        }
    };
    let stable = stable_descriptor_root(function, source_slice)
        .map_err(|error| format!("{error} (source slice v{})", source_slice.index()))?;
    if stable != origin {
        return Err(
            "decision-tree source slice does not forward from its exact descriptor origin"
                .to_string(),
        );
    }
    let expected = MirType::Slice(Box::new(MirType::Primitive(MirPrimitiveTypeName::F64)));
    if source_value_type(function, origin).and_then(|type_node| type_node.as_scalar().cloned())
        != Some(expected)
    {
        return Err("decision-tree descriptor-origin slice has the wrong element type".to_string());
    }
    Ok(origin)
}

fn normalized_range_requirements(
    function: &crate::KirFunction,
    candidate: &WasmDecisionTreeCandidate,
    input_slice: crate::ValueId,
    output_slice: crate::ValueId,
) -> Result<Vec<crate::WasmSliceRangeRequirement>, String> {
    let mut input_seen = false;
    let mut output_seen = false;
    let mut normalized = Vec::with_capacity(candidate.range_requirements.len());
    for requirement in &candidate.range_requirements {
        let source_root = stable_descriptor_root(function, requirement.slice)?;
        let (expected, normalized_slice) = if source_root == input_slice {
            input_seen = true;
            (candidate.input_slice, input_slice)
        } else if source_root == output_slice {
            output_seen = true;
            (candidate.output_slice, output_slice)
        } else {
            return Err("decision-tree range guard refers to an unrelated slice".to_string());
        };
        if stable_descriptor_root(function, expected)? != source_root {
            return Err(
                "decision-tree range slice identity is not all-path equivalent".to_string(),
            );
        }
        let mut requirement = *requirement;
        requirement.slice = normalized_slice;
        normalized.push(requirement);
    }
    if !input_seen || !output_seen || normalized.len() != 2 {
        return Err("decision-tree range guard does not cover both exact descriptors".to_string());
    }
    Ok(normalized)
}

fn source_value_is_header_iv(
    header: &KirBlock,
    edge: &KirEdge,
    target_index: usize,
    induction: crate::ValueId,
) -> bool {
    edge.args
        .get(target_index)
        .is_some_and(|value| *value == induction)
        && header.params.iter().any(|param| param.value == induction)
}

fn source_value_header_origin(
    header: &KirBlock,
    edge: &KirEdge,
    target_index: usize,
) -> Option<crate::ValueId> {
    let value = *edge.args.get(target_index)?;
    header
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| param.value)
}

fn make_vector_loop_condition(
    trial: &mut KirVerifiedProgramState,
    induction: crate::ValueId,
    limit: crate::ValueId,
) -> Result<(crate::ValueId, KirInstruction), String> {
    let value = trial.fresh_value()?;
    Ok((
        value,
        KirInstruction {
            id: trial.fresh_instruction()?,
            results: vec![KirResult {
                value,
                type_node: MirType::Primitive(MirPrimitiveTypeName::Bool).into(),
            }],
            kind: KirInstructionKind::Compare {
                op: MirCompareOp::Le,
                left: induction,
                right: limit,
            },
            memory: None,
            effect: None,
        },
    ))
}

fn vector_access(
    slice: crate::ValueId,
    start: crate::ValueId,
    end: crate::ValueId,
) -> KirVectorMemoryAccess {
    KirVectorMemoryAccess {
        slice,
        start,
        end,
        lane: KirLaneType::F64,
        lanes: 2,
        byte_footprint: 16,
        known_alignment: 8,
        required_alignment: 8,
    }
}

fn vector_binary(op: MirBinaryOp) -> Result<KirVectorBinaryOp, String> {
    match op {
        MirBinaryOp::Add => Ok(KirVectorBinaryOp::Add),
        MirBinaryOp::Sub => Ok(KirVectorBinaryOp::Subtract),
        MirBinaryOp::Mul => Ok(KirVectorBinaryOp::Multiply),
        MirBinaryOp::Div | MirBinaryOp::Mod => {
            Err("decision-tree division/remainder is not supported".to_string())
        }
    }
}

fn plan_operations(
    original: &crate::KirFunction,
    candidate: &WasmDecisionTreeCandidate,
    emitted: &BTreeMap<
        (crate::InstructionId, u8),
        (crate::InstructionId, KirProfileOperation, KirCostSemantics),
    >,
) -> Result<Vec<VectorOperationMapping>, String> {
    let source_operations = candidate
        .ordered_tree_dag
        .iter()
        .filter(|id| {
            instruction(original, **id).is_ok_and(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::Binary {
                        semantics: KirArithmeticSemantics::StrictFloat,
                        ..
                    } | KirInstructionKind::Unary {
                        semantics: KirArithmeticSemantics::StrictFloat,
                        ..
                    } | KirInstructionKind::Compare { .. }
                )
            })
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let expected = (0..candidate.uf)
        .flat_map(|unroll_index| {
            source_operations
                .iter()
                .copied()
                .map(move |scalar| (scalar, unroll_index))
        })
        .collect::<BTreeSet<_>>();
    if expected.len() != emitted.len() || expected.iter().any(|id| !emitted.contains_key(id)) {
        return Err(
            "decision-tree vector operation map does not close over source DAG".to_string(),
        );
    }
    expected
        .into_iter()
        .map(|(scalar, unroll_index)| {
            let (vector, operation, semantics) = emitted[&(scalar, unroll_index)];
            Ok(VectorOperationMapping {
                scalar,
                vector,
                unroll_index,
                operation,
                lane_type: KirLaneType::F64,
                semantics,
                alignment: KirAlignmentClass::NotApplicable,
                lanes: vec![
                    VectorLaneMapping {
                        lane: 0,
                        scalar_iteration: u32::from(unroll_index) * 2,
                    },
                    VectorLaneMapping {
                        lane: 1,
                        scalar_iteration: u32::from(unroll_index) * 2 + 1,
                    },
                ],
            })
        })
        .collect()
}

fn decision_tree_predicates(
    candidate: &WasmDecisionTreeCandidate,
    range_requirements: &[crate::WasmSliceRangeRequirement],
    roots: &VectorProofRoots,
) -> Vec<VectorPredicate> {
    let mut predicates = vec![VectorPredicate::TripThreshold {
        trip_count: candidate.bound,
        minimum: candidate.minimum_trip,
        proof: roots.trip_partition,
    }];
    predicates.extend(range_requirements.iter().copied().map(|requirement| {
        VectorPredicate::WasmSliceRange {
            requirement,
            proof: roots.target_legality,
        }
    }));
    predicates
}

fn insert_proofs(
    trial: &mut KirVerifiedProgramState,
    candidate: &WasmDecisionTreeCandidate,
) -> Result<VectorProofRoots, String> {
    let use_site = FactUseSite {
        function: candidate.function,
        block: candidate.header,
        instruction: None,
        contract_instance: None,
    };
    let mut insert = || {
        trial
            .proofs_mut()
            .try_insert(
                use_site,
                vec![ProofStep::TypeBounds {
                    claim: ScalarClaim::new(
                        candidate.bound,
                        ScalarInterval::new(0.into(), u32::MAX.into())
                            .map_err(|_| "decision-tree u32 interval is malformed".to_string())?,
                        ScalarFailure::None,
                    ),
                }],
                ProofStepId::from_index(0),
            )
            .map_err(|error| error.to_string())
    };
    Ok(VectorProofRoots {
        canonical_loop: insert()?,
        trip_partition: insert()?,
        lane_mapping: insert()?,
        operation_equivalence: insert()?,
        fallback_identity: insert()?,
        target_legality: insert()?,
        cost_and_budget: insert()?,
    })
}

fn decision_tree_charge(plan: &DecisionTreeVectorPlan) -> CandidateBudgetCharge {
    let vector = &plan.vector;
    let lanes = vector.operations.iter().fold(0_u32, |total, operation| {
        total.saturating_add(u32::try_from(operation.lanes.len()).unwrap_or(u32::MAX))
    });
    let memory = vector.memory_groups.iter().fold(0_u32, |total, group| {
        total.saturating_add(u32::try_from(group.scalar_instructions.len()).unwrap_or(u32::MAX))
    });
    let operations = u32::try_from(vector.operations.len()).unwrap_or(u32::MAX);
    let groups = u32::try_from(vector.memory_groups.len()).unwrap_or(u32::MAX);
    let predicates = u32::try_from(vector.predicates.len()).unwrap_or(u32::MAX);
    let selects = u32::try_from(plan.selects.len()).unwrap_or(u32::MAX);
    CandidateBudgetCharge::single(
        vector.pre_state.function,
        8_u32
            .saturating_add(operations.saturating_mul(4))
            .saturating_add(lanes)
            .saturating_add(groups.saturating_mul(4))
            .saturating_add(memory)
            .saturating_add(predicates.saturating_mul(3))
            .saturating_add(2)
            .saturating_add(selects.saturating_mul(6)),
        16_u32
            .saturating_add(operations.saturating_mul(6))
            .saturating_add(lanes.saturating_mul(2))
            .saturating_add(groups.saturating_mul(6))
            .saturating_add(memory.saturating_mul(2))
            .saturating_add(predicates.saturating_mul(4))
            .saturating_add(7)
            .saturating_add(3)
            .saturating_add(selects.saturating_mul(10)),
    )
}

fn check_leaf_backedges(
    function: &crate::KirFunction,
    candidate: &WasmDecisionTreeCandidate,
    header: &KirBlock,
    join: &KirBlock,
    latch_edge: &KirEdge,
    leaves: &[LeafState],
    header_memories: &BTreeMap<MemoryVersionId, MemoryVersionId>,
) -> Result<(), String> {
    let expected_leaf_count = tree_store_ids(&candidate.tree)
        .len()
        .saturating_mul(usize::from(candidate.uf));
    let expected_pairs = (0..candidate.uf)
        .flat_map(|unroll_index| {
            candidate
                .blocks
                .iter()
                .filter(|block_id| find_leaf(&candidate.tree, **block_id).is_some())
                .copied()
                .map(move |block_id| (unroll_index, block_id))
        })
        .collect::<BTreeSet<_>>();
    if leaves.len() != expected_leaf_count
        || leaves
            .iter()
            .map(|leaf| (leaf.unroll_index, leaf.block))
            .collect::<BTreeSet<_>>()
            != expected_pairs
    {
        return Err("decision-tree leaf recurrence coverage is incomplete".to_string());
    }
    for header_param in &header.params {
        let header_index = header
            .params
            .iter()
            .position(|param| param.value == header_param.value)
            .ok_or_else(|| "decision-tree header value index is missing".to_string())?;
        let join_value = *latch_edge
            .args
            .get(header_index)
            .ok_or_else(|| "decision-tree latch value edge is incomplete".to_string())?;
        let join_index = join
            .params
            .iter()
            .position(|param| param.value == join_value)
            .ok_or_else(|| "decision-tree join phi is missing a header value".to_string())?;
        for leaf in leaves {
            let Some(WasmDecisionTreeNode::Leaf {
                join_edge,
                induction_result,
                ..
            }) = find_leaf(&candidate.tree, leaf.block)
            else {
                return Err("decision-tree leaf candidate disappeared".to_string());
            };
            let source = *join_edge
                .args
                .get(join_index)
                .ok_or_else(|| "decision-tree leaf join edge is incomplete".to_string())?;
            if header_param.value == candidate.induction {
                if source != *induction_result {
                    return Err(
                        "decision-tree leaf does not return its exact induction update".to_string(),
                    );
                }
            } else if resolve_invariant_origin(function, leaf, source)? != Some(header_param.value)
            {
                return Err("decision-tree changes non-induction scalar loop state".to_string());
            }
        }
    }
    for header_memory in &header.memory_params {
        let index = header
            .memory_params
            .iter()
            .position(|param| param.version == header_memory.version)
            .ok_or_else(|| "decision-tree header memory index is missing".to_string())?;
        let join_version = *latch_edge
            .memory_args
            .get(index)
            .ok_or_else(|| "decision-tree latch memory edge is incomplete".to_string())?;
        let join_index = join
            .memory_params
            .iter()
            .position(|param| param.version == join_version)
            .ok_or_else(|| "decision-tree join memory phi is missing".to_string())?;
        for leaf in leaves {
            let Some(WasmDecisionTreeNode::Leaf {
                join_edge,
                store: store_id,
                ..
            }) = find_leaf(&candidate.tree, leaf.block)
            else {
                return Err("decision-tree leaf candidate disappeared".to_string());
            };
            let source = *join_edge
                .memory_args
                .get(join_index)
                .ok_or_else(|| "decision-tree leaf memory edge is incomplete".to_string())?;
            if header_memory.region == candidate.output_partition {
                let store = instruction(function, *store_id)?;
                if store.memory.as_ref().and_then(|memory| memory.output) != Some(source) {
                    return Err(
                        "decision-tree output memory does not follow its leaf store".to_string()
                    );
                }
            } else if leaf.memory_origins.get(&source).copied()
                != header_memories.get(&header_memory.version).copied()
            {
                return Err("decision-tree changes non-output MemorySSA state".to_string());
            }
        }
    }
    Ok(())
}

fn resolve_invariant_origin(
    function: &crate::KirFunction,
    leaf: &LeafState,
    value: crate::ValueId,
) -> Result<Option<crate::ValueId>, String> {
    if let Some(origin) = leaf.invariant_origins.get(&value) {
        return Ok(Some(*origin));
    }
    let _ = function;
    Ok(None)
}

fn find_leaf(
    node: &WasmDecisionTreeNode,
    block_id: crate::BlockId,
) -> Option<&WasmDecisionTreeNode> {
    match node {
        WasmDecisionTreeNode::Branch {
            then_node,
            else_node,
            ..
        } => find_leaf(then_node, block_id).or_else(|| find_leaf(else_node, block_id)),
        WasmDecisionTreeNode::Leaf { block, .. } if *block == block_id => Some(node),
        WasmDecisionTreeNode::Leaf { .. } => None,
    }
}

fn tree_store_ids(node: &WasmDecisionTreeNode) -> Vec<crate::InstructionId> {
    let mut stores = Vec::new();
    collect_stores(node, &mut stores);
    stores.sort_unstable();
    stores
}

fn collect_stores(node: &WasmDecisionTreeNode, stores: &mut Vec<crate::InstructionId>) {
    match node {
        WasmDecisionTreeNode::Branch {
            then_node,
            else_node,
            ..
        } => {
            collect_stores(then_node, stores);
            collect_stores(else_node, stores);
        }
        WasmDecisionTreeNode::Leaf { store, .. } => stores.push(*store),
    }
}

fn join_shape<'a>(
    function: &'a crate::KirFunction,
    candidate: &WasmDecisionTreeCandidate,
) -> Result<(&'a KirEdge, &'a KirBlock), String> {
    let join = block(function, candidate.join)?;
    let crate::KirTerminator::Jump { edge } = &join.terminator else {
        return Err("decision-tree join is not a jump".to_string());
    };
    if edge.target != candidate.header {
        return Err("decision-tree join does not target the loop header".to_string());
    }
    Ok((edge, join))
}

fn block(function: &crate::KirFunction, id: crate::BlockId) -> Result<&KirBlock, String> {
    function
        .blocks
        .iter()
        .find(|block| block.id == id)
        .ok_or_else(|| format!("decision-tree block b{} is missing", id.index()))
}

fn instruction(
    function: &crate::KirFunction,
    id: crate::InstructionId,
) -> Result<&KirInstruction, String> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == id)
        .ok_or_else(|| format!("decision-tree instruction i{} is missing", id.index()))
}

fn source_value_type(function: &crate::KirFunction, value: crate::ValueId) -> Option<KirValueType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map(|param| param.type_node.clone().into())
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.params)
                .find(|param| param.value == value)
                .map(|param| param.type_node.clone())
        })
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .flat_map(|instruction| &instruction.results)
                .find(|result| result.value == value)
                .map(|result| result.type_node.clone())
        })
}

fn terminator_edges(terminator: &crate::KirTerminator) -> Vec<&KirEdge> {
    match terminator {
        crate::KirTerminator::Jump { edge } => vec![edge],
        crate::KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        crate::KirTerminator::Return { .. } => Vec::new(),
    }
}

fn is_one(function: &crate::KirFunction, value: crate::ValueId) -> bool {
    function.blocks.iter().flat_map(|block| &block.instructions).any(|instruction| {
        instruction.results.iter().any(|result| result.value == value)
            && matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "1")
    })
}

fn preheader_u32_constant(
    trial: &mut KirVerifiedProgramState,
    preheader: &mut KirBlock,
    value: u32,
) -> Result<crate::ValueId, String> {
    if let Some(existing) = preheader.instructions.iter().find_map(|instruction| {
        (matches!(&instruction.kind, KirInstructionKind::ConstInt { value: text } if text == &value.to_string())
            && instruction.results.first().is_some_and(|result| {
                result.type_node.as_scalar() == Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
            }))
            .then(|| instruction.results.first().map(|result| result.value))
            .flatten()
    }) {
        return Ok(existing);
    }
    let fresh = trial.fresh_value()?;
    preheader.instructions.push(KirInstruction {
        id: trial.fresh_instruction()?,
        results: vec![KirResult {
            value: fresh,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind: KirInstructionKind::ConstInt {
            value: value.to_string(),
        },
        memory: None,
        effect: None,
    });
    Ok(fresh)
}

fn module_units(module: &crate::KirModule) -> u32 {
    module.functions.iter().fold(0_u32, |total, function| {
        total.saturating_add(kir_function_units(function))
    })
}
