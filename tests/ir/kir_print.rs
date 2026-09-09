use calckernel::{
    BlockId, FunctionId, InstructionId, KirArithmeticSemantics, KirBlock, KirBlockParam,
    KirBoundsMode, KirBuildConfig, KirCheckConditionKind, KirConsumer, KirEdge, KirEffectKind,
    KirFailureKind, KirFunction, KirInitialMemory, KirInstruction, KirInstructionKind,
    KirMemoryAccess, KirMemoryBlockParam, KirMemoryRegion, KirMemoryRegionOrigin, KirModule,
    KirOrderedEffect, KirOverflowMode, KirParam, KirPlace, KirResult, KirSanitizerMode,
    KirSymbolicByteInterval, KirTargetProfile, KirTerminator, KirTuneFunctionLayout,
    KirTuneLayoutPlan, KirValueType, KirVectorBinaryOp, KirVectorUnaryOp, KirVersionPredicate,
    KirVersionPredicateConjunct, MemoryRegionId, MemoryVersionId, MirBinaryOp, MirCastOp,
    MirCompareOp, MirEntryPoint, MirEntryResult, MirPrimitiveTypeName, MirRuntimeIntrinsic,
    MirStruct, MirStructField, MirType, MirUnaryOp, ProofId, SourceFile, ValueId, build_kir_module,
    check, lower_to_mir, print_kir_module, print_mir_type,
};
use sha2::{Digest, Sha256};

use super::allocation_counter;

fn scalar_module(instructions: u32) -> KirModule {
    let scalar = MirType::Primitive(MirPrimitiveTypeName::I32);
    KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: KirTargetProfile::inspection(),
        entry: None,
        structs: Vec::new(),
        tune_layout: None,
        functions: vec![KirFunction {
            id: FunctionId::from_index(0),
            name: "MixedCase_计算".to_string(),
            exported: true,
            tune_noinline: false,
            params: Vec::new(),
            return_type: scalar.clone(),
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![KirBlock {
                id: BlockId::from_index(0),
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: (0..instructions)
                    .map(|index| KirInstruction {
                        id: InstructionId::from_index(index),
                        results: vec![KirResult {
                            value: ValueId::from_index(index),
                            type_node: scalar.clone().into(),
                        }],
                        kind: KirInstructionKind::ConstInt {
                            value: "42".to_string(),
                        },
                        memory: None,
                        effect: None,
                    })
                    .collect(),
                terminator: KirTerminator::Return {
                    value: instructions.checked_sub(1).map(ValueId::from_index),
                    memory: Vec::new(),
                    effect_order: 0,
                },
            }],
        }],
    }
}

fn mixed_module() -> KirModule {
    let value = ValueId::from_index;
    let region = MemoryRegionId::from_index;
    let memory = MemoryVersionId::from_index;
    let scalar = MirType::Primitive(MirPrimitiveTypeName::U32);
    let nested = MirType::Pointer(Box::new(MirType::Slice(Box::new(MirType::Struct(
        "MixedCase_记录".to_string(),
    )))));
    let place = |value_index| KirPlace::Value {
        value: value(value_index),
        type_node: scalar.clone(),
        region: region(0),
    };
    let mut module = scalar_module(0);
    module.config.overflow_mode = KirOverflowMode::Checked;
    module.config.bounds_mode = KirBoundsMode::Checked;
    module.config.sanitizer_mode = KirSanitizerMode::Contracts;
    module.entry = Some(MirEntryPoint {
        function_name: "MixedCase_计算".to_string(),
        result: MirEntryResult::Void,
    });
    module.structs = vec![
        MirStruct {
            name: "MixedCase_记录".to_string(),
            fields: vec![
                MirStructField {
                    name: "Items".to_string(),
                    type_node: nested.clone(),
                },
                MirStructField {
                    name: "Flag".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::Bool),
                },
            ],
        },
        MirStruct {
            name: "Empty".to_string(),
            fields: Vec::new(),
        },
    ];
    module.tune_layout = Some(KirTuneLayoutPlan {
        functions: vec![
            KirTuneFunctionLayout {
                function: FunctionId::from_index(0),
                blocks: vec![BlockId::from_index(1), BlockId::from_index(0)],
            },
            KirTuneFunctionLayout {
                function: FunctionId::from_index(1),
                blocks: Vec::new(),
            },
        ],
    });
    let function = &mut module.functions[0];
    function.tune_noinline = true;
    function.return_type = MirType::Void;
    function.params = vec![
        KirParam {
            value: value(0),
            name: "Ptr".to_string(),
            type_node: nested,
        },
        KirParam {
            value: value(1),
            name: "Count".to_string(),
            type_node: scalar.clone(),
        },
    ];
    function.regions = [
        KirMemoryRegionOrigin::Conservative,
        KirMemoryRegionOrigin::Parameter(value(0)),
        KirMemoryRegionOrigin::RawSlice(value(1)),
        KirMemoryRegionOrigin::Subslice(value(2)),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, origin)| KirMemoryRegion {
        id: region(index as u32),
        origin,
        parent: (index > 0).then_some(region(0)),
        partition: region(0),
        byte_interval: (index > 1).then_some(KirSymbolicByteInterval {
            start: value(1),
            end: value(2),
            element_type: scalar.clone(),
        }),
    })
    .collect();
    function.initial_memory = vec![KirInitialMemory {
        region: region(0),
        version: memory(0),
    }];
    let kinds = vec![
        KirInstructionKind::Undef {
            slot: "MixedCase_值".to_string(),
        },
        KirInstructionKind::ConstInt {
            value: "18446744073709551615".to_string(),
        },
        KirInstructionKind::ConstFloat {
            value: "1.25E+30".to_string(),
        },
        KirInstructionKind::ConstBool { value: true },
        KirInstructionKind::Copy { value: value(1) },
        KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left: value(1),
            right: value(2),
            semantics: KirArithmeticSemantics::Modular,
        },
        KirInstructionKind::Binary {
            op: MirBinaryOp::Mul,
            left: value(1),
            right: value(2),
            semantics: KirArithmeticSemantics::Checked,
        },
        KirInstructionKind::Unary {
            op: MirUnaryOp::Neg,
            operand: value(3),
            semantics: KirArithmeticSemantics::StrictFloat,
        },
        KirInstructionKind::Compare {
            op: MirCompareOp::Ne,
            left: value(1),
            right: value(2),
        },
        KirInstructionKind::Cast {
            op: MirCastOp::U32ToF64,
            value: value(1),
        },
        KirInstructionKind::CheckCondition {
            kind: KirCheckConditionKind::InvalidSubslice,
            args: vec![value(1), value(2)],
        },
        KirInstructionKind::CheckCondition {
            kind: KirCheckConditionKind::DivisionByZero,
            args: Vec::new(),
        },
        KirInstructionKind::Guard {
            condition: value(3),
            failure: KirFailureKind::ContractViolation,
        },
        KirInstructionKind::Address {
            place: Box::new(KirPlace::Field {
                base: Box::new(KirPlace::Index {
                    base: Box::new(place(0)),
                    index: value(1),
                    type_node: scalar.clone(),
                    region: region(0),
                }),
                field_name: "MixedCase_字段".to_string(),
                type_node: scalar.clone(),
                region: region(0),
            }),
        },
        KirInstructionKind::Load {
            place: Box::new(KirPlace::Deref {
                pointer: value(0),
                type_node: scalar.clone(),
                region: region(0),
            }),
        },
        KirInstructionKind::Store {
            place: Box::new(KirPlace::SliceIndex {
                slice: value(0),
                index: value(1),
                type_node: scalar.clone(),
                region: region(0),
            }),
            value: value(2),
        },
        KirInstructionKind::MakeSlice {
            data: value(0),
            len: value(1),
        },
        KirInstructionKind::SliceData { slice: value(0) },
        KirInstructionKind::SliceLen { slice: value(0) },
        KirInstructionKind::Subslice {
            slice: value(0),
            start: value(1),
            end: value(2),
        },
        KirInstructionKind::Call {
            function_name: "MixedCase_辅助".to_string(),
            args: vec![value(0), value(1)],
        },
        KirInstructionKind::Call {
            function_name: "Empty".to_string(),
            args: Vec::new(),
        },
        KirInstructionKind::RuntimeCall {
            intrinsic: MirRuntimeIntrinsic::PrintU32,
            args: vec![value(1)],
        },
        KirInstructionKind::RuntimeCall {
            intrinsic: MirRuntimeIntrinsic::PrintNewline,
            args: Vec::new(),
        },
        KirInstructionKind::VersionPredicate {
            predicate: KirVersionPredicate {
                address_bits: 64,
                conjuncts: vec![
                    KirVersionPredicateConjunct::TripThreshold {
                        value: value(1),
                        minimum: 32,
                    },
                    KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left: value(0),
                        left_count: value(1),
                        left_element_bytes: 4,
                        right: value(2),
                        right_count: value(3),
                        right_element_bytes: 8,
                    },
                ],
            },
        },
        KirInstructionKind::VersionPredicate {
            predicate: KirVersionPredicate {
                address_bits: 32,
                conjuncts: Vec::new(),
            },
        },
    ];
    let block = &mut function.blocks[0];
    block.params = vec![
        KirBlockParam {
            value: value(7),
            slot: "Merged".to_string(),
            type_node: scalar.clone().into(),
        },
        KirBlockParam {
            value: value(8),
            slot: "Mask".to_string(),
            type_node: KirValueType::Mask { lanes: 4 },
        },
    ];
    block.memory_params = vec![KirMemoryBlockParam {
        version: memory(1),
        region: region(0),
    }];
    block.instructions = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| KirInstruction {
            id: InstructionId::from_index(index as u32),
            results: if index % 3 == 0 {
                Vec::new()
            } else {
                vec![KirResult {
                    value: value(index as u32 + 10),
                    type_node: scalar.clone().into(),
                }]
            },
            kind,
            memory: (index % 4 == 0).then_some(KirMemoryAccess {
                region: region(0),
                input: memory(1),
                output: (index % 8 == 0).then_some(memory(2)),
            }),
            effect: (index % 5 == 0).then_some(KirOrderedEffect {
                order: index as u32,
                kind: KirEffectKind::MayFail,
            }),
        })
        .collect();
    block.terminator = KirTerminator::Branch {
        condition: value(3),
        then_edge: KirEdge {
            target: BlockId::from_index(1),
            args: vec![value(1), value(2)],
            memory_args: vec![memory(1), memory(2)],
        },
        else_edge: KirEdge {
            target: BlockId::from_index(2),
            args: Vec::new(),
            memory_args: Vec::new(),
        },
    };
    function.blocks.push(KirBlock {
        id: BlockId::from_index(1),
        label: "memory_only".to_string(),
        params: Vec::new(),
        memory_params: vec![KirMemoryBlockParam {
            version: memory(3),
            region: region(0),
        }],
        instructions: Vec::new(),
        terminator: KirTerminator::Jump {
            edge: KirEdge {
                target: BlockId::from_index(2),
                args: Vec::new(),
                memory_args: vec![memory(3)],
            },
        },
    });
    function.blocks.push(KirBlock {
        id: BlockId::from_index(2),
        label: "done".to_string(),
        params: Vec::new(),
        memory_params: Vec::new(),
        instructions: Vec::new(),
        terminator: KirTerminator::Return {
            value: None,
            memory: vec![(region(0), memory(2)), (region(1), memory(3))],
            effect_order: 9,
        },
    });
    module
}

fn proof_vector_module() -> KirModule {
    let mut module = super::vector::vector_module();
    for instruction in &mut module.functions[0].blocks[0].instructions {
        match &mut instruction.kind {
            KirInstructionKind::VectorBinary {
                op,
                semantics,
                no_failure_proof,
                ..
            } => {
                *op = KirVectorBinaryOp::Divide;
                *semantics = KirArithmeticSemantics::Checked;
                *no_failure_proof = Some(ProofId::from_index(37));
            }
            KirInstructionKind::VectorUnary {
                op,
                no_failure_proof,
                ..
            } => {
                *op = KirVectorUnaryOp::MaskNot;
                *no_failure_proof = Some(ProofId::from_index(38));
            }
            _ => {}
        }
    }
    module
}

#[test]
fn kir_print_should_not_allocate_a_temporary_for_each_scalar_instruction() {
    let mut counts = Vec::new();
    for instructions in [1, 64, 512] {
        let module = scalar_module(instructions);
        let (printed, allocations) = allocation_counter::measure(|| print_kir_module(&module));
        assert!(printed.contains(&format!("i{} ", instructions - 1)));
        counts.push((instructions, allocations));
    }
    eprintln!("KIR scalar instruction/allocation counts: {counts:?}");
    assert!(
        counts.iter().all(|(_, allocations)| *allocations <= 64),
        "KIR serialization must avoid per-instruction temporaries: {counts:?}"
    );
}

#[test]
fn kir_print_should_not_allocate_a_temporary_for_each_vector_operation() {
    let module = super::vector::vector_module();
    let (printed, allocations) = allocation_counter::measure(|| print_kir_module(&module));
    assert!(printed.contains("vector_store"));
    eprintln!("KIR complete vector allocation count: {allocations}");
    assert!(
        allocations <= 64,
        "vector KIR serialization made {allocations} allocations"
    );
}

#[test]
fn kir_print_should_preserve_frozen_scalar_and_vector_bytes() {
    let records = [
        ("scalar-empty", scalar_module(0)),
        ("scalar-full", scalar_module(512)),
        ("vector-full", super::vector::vector_module()),
        ("mixed-metadata-and-operations", mixed_module()),
        ("vector-with-proofs", proof_vector_module()),
    ]
    .into_iter()
    .map(|(name, module)| {
        let text = print_kir_module(&module);
        (
            name,
            text.len(),
            format!("{:x}", Sha256::digest(text.as_bytes())),
        )
    })
    .collect::<Vec<_>>();
    // Captured from the unchanged KIR v3 printer before the buffer rewrite.
    let expected = [
        (
            "scalar-empty",
            255,
            "b8608a06242ed214529ff31d2756deff6d3381deea1634a950a4813f871fc83b",
        ),
        (
            "scalar-full",
            16424,
            "4453906a868c53b11379441d204b392c503b7829b2390d18eeaf35d94fc54c63",
        ),
        (
            "vector-full",
            1229,
            "7206a655842693ef34bca3af7d285c47a2ca5cc40743bde811e1084c63a6c03d",
        ),
        (
            "mixed-metadata-and-operations",
            2155,
            "b71c0ef29368b2f78efdb75d1bcd3b1652a6a1a885ab1c783eff1903b4e3da4f",
        ),
        (
            "vector-with-proofs",
            1231,
            "967d3c0421ec5c4ee4870839ecffb429c0abfcbef7792ba2718c4086bdc1776c",
        ),
    ]
    .map(|(name, bytes, digest)| (name, bytes, digest.to_string()));
    assert_eq!(records, expected);
}

#[test]
fn kir_print_should_preserve_real_source_bytes() {
    let sources = [
        (
            "branch-layout",
            include_str!("../../benches/fixtures/pgo/branch_layout.ck"),
        ),
        (
            "call-constant-length",
            include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
        ),
    ];
    let records = sources
        .into_iter()
        .map(|(name, source)| {
            let checked = check(&SourceFile::new("printer.ck", source));
            assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
            let mir = lower_to_mir(&checked.checked_program).expect("MIR");
            let module = build_kir_module(&mir, scalar_module(0).config).expect("KIR");
            let text = print_kir_module(&module);
            (
                name,
                text.len(),
                format!("{:x}", Sha256::digest(text.as_bytes())),
            )
        })
        .collect::<Vec<_>>();
    let expected = [
        (
            "branch-layout",
            2821,
            "363d7fc63aaa47dad53e185ab53061f8feaa42dafdee44e3b805dbed8861cbc5",
        ),
        (
            "call-constant-length",
            3314,
            "63d971cb1f57502c52fa0e670765ccf398fcdc9ed0b18f2fb8b21e194ce067a1",
        ),
    ]
    .map(|(name, bytes, digest)| (name, bytes, digest.to_string()));
    assert_eq!(records, expected);
}

#[test]
fn kir_print_shared_type_formatter_should_preserve_all_mir_type_spellings() {
    let types = [
        (MirType::Primitive(MirPrimitiveTypeName::I32), "i32"),
        (MirType::Primitive(MirPrimitiveTypeName::I64), "i64"),
        (MirType::Primitive(MirPrimitiveTypeName::U32), "u32"),
        (MirType::Primitive(MirPrimitiveTypeName::U64), "u64"),
        (MirType::Primitive(MirPrimitiveTypeName::F64), "f64"),
        (MirType::Primitive(MirPrimitiveTypeName::Bool), "bool"),
        (MirType::Void, "void"),
        (
            MirType::Struct("MixedCase_记录".to_string()),
            "MixedCase_记录",
        ),
        (
            MirType::Pointer(Box::new(MirType::Slice(Box::new(MirType::Primitive(
                MirPrimitiveTypeName::U64,
            ))))),
            "ptr<slice<u64>>",
        ),
    ];
    for (type_node, expected) in types {
        assert_eq!(print_mir_type(&type_node), expected);
    }
}
