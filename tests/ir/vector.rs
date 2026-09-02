use calckernel::{
    BlockId, EmitWasmOptions, FunctionId, InstructionId, KirAlignmentClass, KirArithmeticSemantics,
    KirBlock, KirBoundsMode, KirBuildConfig, KirConsumer, KirCostKey, KirCostSemantics, KirEdge,
    KirEffectKind, KirFunction, KirInitialMemory, KirInstruction, KirInstructionKind, KirLaneType,
    KirLegalCost, KirMemoryAccess, KirMemoryBlockParam, KirMemoryRegion, KirMemoryRegionOrigin,
    KirModule, KirNativeCpuPolicy, KirOrderedEffect, KirOverflowMode, KirProfileOperation,
    KirResult, KirSanitizerMode, KirTargetProfile, KirTargetProfileBuilder, KirTerminator,
    KirValueType, KirVectorBinaryOp, KirVectorCastOp, KirVectorMemoryAccess, KirVectorReductionOp,
    KirVectorRegion, KirVectorUnaryOp, MemoryRegionId, MemoryVersionId, MirCompareOp,
    MirPrimitiveTypeName, MirType, ProofId, ValueId, VectorRegionId, emit_c_kir_module,
    emit_wat_kir_module, print_kir_module, validate_kir_module,
};

fn scalar(primitive: MirPrimitiveTypeName) -> KirValueType {
    KirValueType::Scalar(MirType::Primitive(primitive))
}

fn vector(lane: KirLaneType) -> KirValueType {
    KirValueType::FixedVector { lane, lanes: 4 }
}

fn vector_profile() -> KirTargetProfile {
    vector_profile_without(None)
}

fn vector_profile_without(excluded: Option<KirProfileOperation>) -> KirTargetProfile {
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        "aarch64-unknown-linux-gnu",
        64,
        true,
        KirNativeCpuPolicy::Baseline,
        "generic",
        vec!["+neon".to_string()],
    )
    .expect("native profile builder");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lanes == 4
                && matches!(key.lane, KirLaneType::I32 | KirLaneType::F64)
                && (key.operation != KirProfileOperation::MaskNot
                    || key.lane == calckernel::KIR_MASK_COST_LANE)
                && excluded != Some(key.operation)
        })
    {
        let legalized_type = format!("v4{:?}", key.lane).to_ascii_lowercase();
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type,
                },
            )
            .expect("legal vector query");
    }
    builder.build().expect("complete vector profile")
}

fn access() -> KirVectorMemoryAccess {
    KirVectorMemoryAccess {
        slice: ValueId::from_index(1),
        start: ValueId::from_index(2),
        end: ValueId::from_index(3),
        lane: KirLaneType::I32,
        lanes: 4,
        byte_footprint: 16,
        known_alignment: 16,
        required_alignment: 16,
    }
}

fn instruction(
    index: u32,
    result: Option<(u32, KirValueType)>,
    kind: KirInstructionKind,
    memory: Option<KirMemoryAccess>,
    effect: Option<KirOrderedEffect>,
) -> KirInstruction {
    KirInstruction {
        id: InstructionId::from_index(index),
        results: result.map_or_else(Vec::new, |(value, type_node)| {
            vec![KirResult {
                value: ValueId::from_index(value),
                type_node,
            }]
        }),
        kind,
        memory,
        effect,
    }
}

fn vector_module() -> KirModule {
    let region = VectorRegionId::from_index(0);
    let memory_region = MemoryRegionId::from_index(0);
    let memory0 = MemoryVersionId::from_index(0);
    let memory1 = MemoryVersionId::from_index(1);
    let block = BlockId::from_index(0);
    let instructions = vec![
        instruction(
            0,
            Some((10, vector(KirLaneType::I32))),
            KirInstructionKind::VectorSplat {
                scalar: ValueId::from_index(0),
                region,
            },
            None,
            None,
        ),
        instruction(
            1,
            Some((11, vector(KirLaneType::I32))),
            KirInstructionKind::VectorLoad {
                access: access(),
                region,
            },
            Some(KirMemoryAccess {
                region: memory_region,
                input: memory0,
                output: None,
            }),
            Some(KirOrderedEffect {
                order: 0,
                kind: KirEffectKind::ReadMemory,
            }),
        ),
        instruction(
            2,
            Some((12, vector(KirLaneType::I32))),
            KirInstructionKind::VectorBinary {
                op: KirVectorBinaryOp::Add,
                left: ValueId::from_index(10),
                right: ValueId::from_index(11),
                semantics: KirArithmeticSemantics::Modular,
                no_failure_proof: None,
                region,
            },
            None,
            None,
        ),
        instruction(
            3,
            Some((13, KirValueType::Mask { lanes: 4 })),
            KirInstructionKind::VectorCompare {
                op: MirCompareOp::Lt,
                left: ValueId::from_index(10),
                right: ValueId::from_index(11),
                region,
            },
            None,
            None,
        ),
        instruction(
            4,
            Some((14, vector(KirLaneType::I32))),
            KirInstructionKind::VectorSelect {
                mask: ValueId::from_index(13),
                when_true: ValueId::from_index(12),
                when_false: ValueId::from_index(11),
                region,
            },
            None,
            None,
        ),
        instruction(
            5,
            Some((15, vector(KirLaneType::I32))),
            KirInstructionKind::VectorUnary {
                op: KirVectorUnaryOp::Negate,
                operand: ValueId::from_index(14),
                semantics: KirArithmeticSemantics::Modular,
                no_failure_proof: None,
                region,
            },
            None,
            None,
        ),
        instruction(
            6,
            Some((16, vector(KirLaneType::F64))),
            KirInstructionKind::VectorCast {
                op: KirVectorCastOp::I32ToF64,
                value: ValueId::from_index(15),
                region,
            },
            None,
            None,
        ),
        instruction(
            7,
            Some((17, vector(KirLaneType::F64))),
            KirInstructionKind::VectorInsert {
                vector: ValueId::from_index(16),
                scalar: ValueId::from_index(4),
                lane_index: 2,
                region,
            },
            None,
            None,
        ),
        instruction(
            8,
            Some((18, scalar(MirPrimitiveTypeName::F64))),
            KirInstructionKind::VectorExtract {
                vector: ValueId::from_index(17),
                lane_index: 2,
                region,
            },
            None,
            None,
        ),
        instruction(
            9,
            Some((19, scalar(MirPrimitiveTypeName::I32))),
            KirInstructionKind::VectorReduce {
                op: KirVectorReductionOp::ModularAdd,
                vector: ValueId::from_index(12),
                semantics: KirArithmeticSemantics::Modular,
                region,
            },
            None,
            None,
        ),
        instruction(
            10,
            None,
            KirInstructionKind::VectorStore {
                access: access(),
                value: ValueId::from_index(12),
                region,
            },
            Some(KirMemoryAccess {
                region: memory_region,
                input: memory0,
                output: Some(memory1),
            }),
            Some(KirOrderedEffect {
                order: 1,
                kind: KirEffectKind::WriteMemory,
            }),
        ),
    ];
    KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: vector_profile(),
        entry: None,
        structs: Vec::new(),
        tune_layout: None,
        functions: vec![KirFunction {
            id: FunctionId::from_index(0),
            name: "vector_kernel".to_string(),
            exported: false,
            params: vec![
                calckernel::KirParam {
                    value: ValueId::from_index(0),
                    name: "scalar".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::I32),
                },
                calckernel::KirParam {
                    value: ValueId::from_index(1),
                    name: "items".to_string(),
                    type_node: MirType::Slice(Box::new(MirType::Primitive(
                        MirPrimitiveTypeName::I32,
                    ))),
                },
                calckernel::KirParam {
                    value: ValueId::from_index(2),
                    name: "start".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32),
                },
                calckernel::KirParam {
                    value: ValueId::from_index(3),
                    name: "end".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32),
                },
                calckernel::KirParam {
                    value: ValueId::from_index(4),
                    name: "replacement".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
                },
            ],
            return_type: MirType::Primitive(MirPrimitiveTypeName::I32),
            regions: vec![KirMemoryRegion {
                id: memory_region,
                origin: KirMemoryRegionOrigin::Parameter(ValueId::from_index(1)),
                parent: None,
                partition: memory_region,
                byte_interval: None,
            }],
            initial_memory: vec![KirInitialMemory {
                region: memory_region,
                version: memory0,
            }],
            vector_regions: vec![KirVectorRegion {
                id: region,
                blocks: vec![block],
            }],
            blocks: vec![KirBlock {
                id: block,
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions,
                terminator: KirTerminator::Return {
                    value: Some(ValueId::from_index(19)),
                    memory: vec![(memory_region, memory1)],
                    effect_order: 2,
                },
            }],
        }],
    }
}

fn messages(module: &KirModule) -> Vec<String> {
    validate_kir_module(module)
        .errors
        .into_iter()
        .map(|error| error.message)
        .collect()
}

#[test]
fn vector_instruction_all_closed_families_should_print_and_validate() {
    let module = vector_module();
    assert_eq!(messages(&module), Vec::<String>::new());
    let text = print_kir_module(&module);
    for spelling in [
        "vector_splat",
        "vector_load",
        "vector_add.modular",
        "vector_compare_lt",
        "vector_select",
        "vector_negate.modular",
        "vector_cast_i32tof64",
        "vector_insert",
        "vector_extract",
        "vector_reduce_modularadd.modular",
        "vector_store",
    ] {
        assert!(text.contains(spelling), "missing {spelling}:\n{text}");
    }
}

#[test]
fn vector_instruction_mutations_should_reject_lane_mask_semantics_and_profile() {
    let mut lane = vector_module();
    lane.functions[0].blocks[0].instructions[2].results[0].type_node = KirValueType::FixedVector {
        lane: KirLaneType::I64,
        lanes: 4,
    };
    assert!(
        messages(&lane)
            .iter()
            .any(|message| message.contains("vector binary"))
    );

    let mut mask = vector_module();
    mask.functions[0].blocks[0].instructions[4].kind = KirInstructionKind::VectorSelect {
        mask: ValueId::from_index(12),
        when_true: ValueId::from_index(12),
        when_false: ValueId::from_index(11),
        region: VectorRegionId::from_index(0),
    };
    assert!(
        messages(&mask)
            .iter()
            .any(|message| message.contains("mask"))
    );

    let mut strict_integer = vector_module();
    let KirInstructionKind::VectorBinary { semantics, .. } =
        &mut strict_integer.functions[0].blocks[0].instructions[2].kind
    else {
        unreachable!()
    };
    *semantics = KirArithmeticSemantics::StrictFloat;
    assert!(
        messages(&strict_integer)
            .iter()
            .any(|message| message.contains("semantics"))
    );

    let mut disabled = vector_module();
    disabled.config.consumer = KirConsumer::Inspection;
    disabled.profile = KirTargetProfile::inspection();
    assert!(
        messages(&disabled)
            .iter()
            .any(|message| message.contains("vector operations are disabled"))
    );
}

#[test]
fn vector_mask_not_should_require_its_exact_profile_operation() {
    let mut module = vector_module();
    module.profile = vector_profile_without(Some(KirProfileOperation::MaskNot));
    let instruction = &mut module.functions[0].blocks[0].instructions[4];
    instruction.results[0].type_node = KirValueType::Mask { lanes: 4 };
    instruction.kind = KirInstructionKind::VectorUnary {
        op: KirVectorUnaryOp::MaskNot,
        operand: ValueId::from_index(13),
        semantics: KirArithmeticSemantics::Modular,
        no_failure_proof: None,
        region: VectorRegionId::from_index(0),
    };

    assert!(
        messages(&module)
            .iter()
            .any(|message| message.contains("unavailable in the exact target profile"))
    );
}

#[test]
fn vector_memory_should_validate_exact_footprint_alignment_memory_ssa_and_region() {
    let mut footprint = vector_module();
    let KirInstructionKind::VectorLoad { access, .. } =
        &mut footprint.functions[0].blocks[0].instructions[1].kind
    else {
        unreachable!()
    };
    access.byte_footprint = 32;
    assert!(
        messages(&footprint)
            .iter()
            .any(|message| message.contains("byte footprint"))
    );

    let mut alignment = vector_module();
    let KirInstructionKind::VectorStore { access, .. } =
        &mut alignment.functions[0].blocks[0].instructions[10].kind
    else {
        unreachable!()
    };
    access.required_alignment = 32;
    assert!(
        messages(&alignment)
            .iter()
            .any(|message| message.contains("alignment"))
    );

    let mut memory = vector_module();
    memory.functions[0].blocks[0].instructions[10].memory = None;
    assert!(
        messages(&memory)
            .iter()
            .any(|message| message.contains("Memory SSA"))
    );

    let mut bad_region = vector_module();
    let KirInstructionKind::VectorBinary { region, .. } =
        &mut bad_region.functions[0].blocks[0].instructions[2].kind
    else {
        unreachable!()
    };
    *region = VectorRegionId::from_index(99);
    assert!(
        messages(&bad_region)
            .iter()
            .any(|message| message.contains("vector region"))
    );
}

#[test]
fn vector_proof_should_be_required_for_integer_division_and_remainder() {
    for op in [KirVectorBinaryOp::Divide, KirVectorBinaryOp::Remainder] {
        let mut module = vector_module();
        let KirInstructionKind::VectorBinary {
            op: actual,
            no_failure_proof,
            ..
        } = &mut module.functions[0].blocks[0].instructions[2].kind
        else {
            unreachable!()
        };
        *actual = op;
        *no_failure_proof = Some(ProofId::from_index(7));
        assert_eq!(messages(&module), Vec::<String>::new(), "{op:?}");
        let KirInstructionKind::VectorBinary {
            no_failure_proof, ..
        } = &mut module.functions[0].blocks[0].instructions[2].kind
        else {
            unreachable!()
        };
        *no_failure_proof = None;
        assert!(
            messages(&module)
                .iter()
                .any(|message| message.contains("no-failure proof"))
        );
    }

    let mut checked_add = vector_module();
    let KirInstructionKind::VectorBinary {
        semantics,
        no_failure_proof,
        ..
    } = &mut checked_add.functions[0].blocks[0].instructions[2].kind
    else {
        unreachable!()
    };
    *semantics = KirArithmeticSemantics::Checked;
    *no_failure_proof = None;
    assert!(
        messages(&checked_add)
            .iter()
            .any(|message| message.contains("no-failure proof"))
    );

    let mut checked_negate = vector_module();
    let KirInstructionKind::VectorUnary {
        semantics,
        no_failure_proof,
        ..
    } = &mut checked_negate.functions[0].blocks[0].instructions[5].kind
    else {
        unreachable!()
    };
    *semantics = KirArithmeticSemantics::Checked;
    *no_failure_proof = None;
    assert!(
        messages(&checked_negate)
            .iter()
            .any(|message| message.contains("no-failure proof"))
    );
}

#[test]
fn vector_instruction_portable_backends_should_reject_without_scalarizing() {
    let module = vector_module();
    assert!(
        emit_c_kir_module(&module)
            .expect_err("C must reject vectors")
            .contains("cannot lower vector value")
    );
    assert!(
        emit_wat_kir_module(&module, EmitWasmOptions::default())
            .expect_err("Wasm must reject vectors")
            .contains("cannot lower vector values")
    );
}

#[test]
fn vector_type_query_universe_should_keep_closed_width_lane_and_alignment_keys() {
    let keys = KirTargetProfile::fixed_query_universe();
    assert!(keys.iter().any(|key| {
        key == &KirCostKey {
            operation: KirProfileOperation::Load,
            lane: KirLaneType::I32,
            lanes: 4,
            semantics: KirCostSemantics::NotApplicable,
            alignment: KirAlignmentClass::Bytes(16),
        }
    }));
    assert!(keys.iter().all(|key| {
        matches!(
            key.lane,
            KirLaneType::I32
                | KirLaneType::I64
                | KirLaneType::U32
                | KirLaneType::U64
                | KirLaneType::F64
        ) && matches!(key.lanes, 1 | 2 | 4 | 8 | 16)
            && u16::from(key.lanes) * key.lane.bit_width() <= 512
    }));
}

#[test]
fn vector_type_region_escape_should_be_rejected_on_a_block_edge() {
    let mut module = vector_module();
    let function = &mut module.functions[0];
    function.blocks[0].terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: BlockId::from_index(1),
            args: vec![ValueId::from_index(12)],
            memory_args: vec![MemoryVersionId::from_index(1)],
        },
    };
    function.blocks.push(KirBlock {
        id: BlockId::from_index(1),
        label: "outside".to_string(),
        params: vec![calckernel::KirBlockParam {
            value: ValueId::from_index(20),
            slot: "escaped".to_string(),
            type_node: vector(KirLaneType::I32),
        }],
        memory_params: vec![KirMemoryBlockParam {
            version: MemoryVersionId::from_index(2),
            region: MemoryRegionId::from_index(0),
        }],
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: Some(ValueId::from_index(19)),
            memory: vec![(
                MemoryRegionId::from_index(0),
                MemoryVersionId::from_index(2),
            )],
            effect_order: 2,
        },
    });
    assert!(
        messages(&module)
            .iter()
            .any(|message| message.contains("escapes its vector region"))
    );
}
