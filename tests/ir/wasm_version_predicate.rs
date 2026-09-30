use calckernel::{
    BlockId, FunctionId, InstructionId, KirBlock, KirBoundsMode, KirBuildConfig, KirConsumer,
    KirFunction, KirInstruction, KirInstructionKind, KirModule, KirOptimizationLevel,
    KirOverflowMode, KirResult, KirSanitizerMode, KirTargetProfile, KirTerminator,
    KirVersionPredicate, KirVersionPredicateConjunct, KirWasmFeatures, MirPrimitiveTypeName,
    MirType, ValueId, print_kir_module, run_kir_pass_pipeline, validate_kir_module,
};

fn primitive(name: MirPrimitiveTypeName) -> MirType {
    MirType::Primitive(name)
}

fn range() -> KirVersionPredicateConjunct {
    KirVersionPredicateConjunct::WasmSliceRange {
        slice: ValueId::from_index(0),
        start: ValueId::from_index(1),
        count: ValueId::from_index(2),
        element_bytes: 4,
    }
}

fn module(
    element: MirType,
    start: MirType,
    count: MirType,
    conjuncts: Vec<KirVersionPredicateConjunct>,
    features: KirWasmFeatures,
) -> KirModule {
    let result = ValueId::from_index(3);
    KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: KirTargetProfile::webassembly_with_features(features),
        entry: None,
        structs: Vec::new(),
        functions: vec![KirFunction {
            id: FunctionId::from_index(0),
            name: "range_guard".to_string(),
            exported: true,
            params: vec![
                calckernel::KirParam {
                    value: ValueId::from_index(0),
                    name: "items".to_string(),
                    type_node: MirType::Slice(Box::new(element)),
                },
                calckernel::KirParam {
                    value: ValueId::from_index(1),
                    name: "start".to_string(),
                    type_node: start,
                },
                calckernel::KirParam {
                    value: ValueId::from_index(2),
                    name: "count".to_string(),
                    type_node: count,
                },
            ],
            return_type: primitive(MirPrimitiveTypeName::Bool),
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![KirBlock {
                id: BlockId::from_index(0),
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: vec![KirInstruction {
                    id: InstructionId::from_index(0),
                    results: vec![KirResult {
                        value: result,
                        type_node: primitive(MirPrimitiveTypeName::Bool).into(),
                    }],
                    kind: KirInstructionKind::VersionPredicate {
                        predicate: KirVersionPredicate {
                            address_bits: 32,
                            conjuncts,
                        },
                    },
                    memory: None,
                    effect: None,
                }],
                terminator: KirTerminator::Return {
                    value: Some(result),
                    memory: Vec::new(),
                    effect_order: 0,
                },
            }],
        }],
    }
}

fn has_error(module: &KirModule) -> bool {
    !validate_kir_module(module).errors.is_empty()
}

#[test]
fn wasm_slice_range_accepts_supported_element_widths_and_exact_u32_bounds() {
    for (element, bytes) in [
        (MirPrimitiveTypeName::I32, 4),
        (MirPrimitiveTypeName::U32, 4),
        (MirPrimitiveTypeName::I64, 8),
        (MirPrimitiveTypeName::U64, 8),
        (MirPrimitiveTypeName::F64, 8),
    ] {
        let mut conjunct = range();
        let KirVersionPredicateConjunct::WasmSliceRange { element_bytes, .. } = &mut conjunct
        else {
            unreachable!();
        };
        *element_bytes = bytes;
        let valid = module(
            primitive(element),
            primitive(MirPrimitiveTypeName::U32),
            primitive(MirPrimitiveTypeName::U32),
            vec![conjunct],
            KirWasmFeatures::Simd128,
        );
        assert_eq!(validate_kir_module(&valid).errors, [], "{element:?}");
    }
}

#[test]
fn wasm_slice_range_rejects_unsupported_types_and_widths_in_both_wasm_profiles() {
    let valid_start = primitive(MirPrimitiveTypeName::U32);
    let valid_count = primitive(MirPrimitiveTypeName::U32);

    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::Bool),
        valid_start.clone(),
        valid_count.clone(),
        vec![range()],
        KirWasmFeatures::Simd128,
    )));

    let mut wrong_width = range();
    let KirVersionPredicateConjunct::WasmSliceRange { element_bytes, .. } = &mut wrong_width else {
        unreachable!();
    };
    *element_bytes = 8;
    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        valid_start.clone(),
        valid_count.clone(),
        vec![wrong_width],
        KirWasmFeatures::Simd128,
    )));

    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::I32),
        valid_count.clone(),
        vec![range()],
        KirWasmFeatures::Simd128,
    )));
    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        valid_start,
        primitive(MirPrimitiveTypeName::I32),
        vec![range()],
        KirWasmFeatures::Simd128,
    )));
    assert_eq!(
        validate_kir_module(&module(
            primitive(MirPrimitiveTypeName::I32),
            primitive(MirPrimitiveTypeName::U32),
            primitive(MirPrimitiveTypeName::U32),
            vec![range()],
            KirWasmFeatures::Baseline,
        ))
        .errors,
        []
    );
    let mut wrong_baseline_width = range();
    let KirVersionPredicateConjunct::WasmSliceRange { element_bytes, .. } =
        &mut wrong_baseline_width
    else {
        unreachable!();
    };
    *element_bytes = 8;
    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        vec![wrong_baseline_width],
        KirWasmFeatures::Baseline,
    )));
}

#[test]
fn version_predicate_budget_allows_one_trip_and_three_nontrip_conjuncts() {
    let trip = KirVersionPredicateConjunct::TripThreshold {
        value: ValueId::from_index(2),
        minimum: 4,
    };
    let mut maximum = vec![trip.clone(), range(), range(), range()];
    let valid = module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        maximum.clone(),
        KirWasmFeatures::Simd128,
    );
    assert_eq!(validate_kir_module(&valid).errors, []);

    maximum[1] = trip.clone();
    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        maximum,
        KirWasmFeatures::Simd128,
    )));

    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        vec![range(), range(), range(), range()],
        KirWasmFeatures::Simd128,
    )));

    assert!(has_error(&module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        vec![trip, range(), range(), range(), range()],
        KirWasmFeatures::Simd128,
    )));
}

#[test]
fn version_predicate_budget_counts_disjoint_and_range_conjuncts_together() {
    let mut candidate = module(
        primitive(MirPrimitiveTypeName::I32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        vec![range()],
        KirWasmFeatures::Simd128,
    );
    candidate.functions[0].params.push(calckernel::KirParam {
        value: ValueId::from_index(4),
        name: "other".to_string(),
        type_node: MirType::Slice(Box::new(primitive(MirPrimitiveTypeName::I32))),
    });
    let disjoint = KirVersionPredicateConjunct::AddressIntervalsDisjoint {
        left: ValueId::from_index(0),
        left_count: ValueId::from_index(2),
        left_element_bytes: 4,
        right: ValueId::from_index(4),
        right_count: ValueId::from_index(2),
        right_element_bytes: 4,
    };
    let set_conjuncts = |module: &mut KirModule, conjuncts| {
        let KirInstructionKind::VersionPredicate { predicate } =
            &mut module.functions[0].blocks[0].instructions[0].kind
        else {
            unreachable!();
        };
        predicate.conjuncts = conjuncts;
    };
    set_conjuncts(
        &mut candidate,
        vec![
            disjoint.clone(),
            disjoint.clone(),
            disjoint.clone(),
            range(),
        ],
    );
    assert!(
        validate_kir_module(&candidate)
            .errors
            .iter()
            .any(|error| error.message.contains("three non-trip"))
    );
    set_conjuncts(&mut candidate, vec![disjoint.clone(), disjoint, range()]);
    assert!(validate_kir_module(&candidate).errors.is_empty());
}

#[test]
fn wasm_slice_range_prints_operand_ssa_identities() {
    let valid = module(
        primitive(MirPrimitiveTypeName::F64),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        vec![KirVersionPredicateConjunct::WasmSliceRange {
            slice: ValueId::from_index(0),
            start: ValueId::from_index(1),
            count: ValueId::from_index(2),
            element_bytes: 8,
        }],
        KirWasmFeatures::Simd128,
    );
    let text = print_kir_module(&valid);
    assert!(text.contains("slice_range(v0,v1,v2,8)"), "{text}");
}

#[test]
fn wasm_slice_range_uses_keep_definitions_live_through_kir_passes() {
    let mut module = module(
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        primitive(MirPrimitiveTypeName::U32),
        vec![range()],
        KirWasmFeatures::Simd128,
    );
    let block = &mut module.functions[0].blocks[0];
    let mut predicate = block.instructions.remove(0);
    let start_copy = ValueId::from_index(4);
    let count_copy = ValueId::from_index(5);
    let KirInstructionKind::VersionPredicate { predicate: guard } = &mut predicate.kind else {
        unreachable!();
    };
    let KirVersionPredicateConjunct::WasmSliceRange { start, count, .. } = &mut guard.conjuncts[0]
    else {
        unreachable!();
    };
    *start = start_copy;
    *count = count_copy;
    predicate.id = InstructionId::from_index(2);
    block.instructions = vec![
        KirInstruction {
            id: InstructionId::from_index(0),
            results: vec![KirResult {
                value: start_copy,
                type_node: primitive(MirPrimitiveTypeName::U32).into(),
            }],
            kind: KirInstructionKind::Copy {
                value: ValueId::from_index(1),
            },
            memory: None,
            effect: None,
        },
        KirInstruction {
            id: InstructionId::from_index(1),
            results: vec![KirResult {
                value: count_copy,
                type_node: primitive(MirPrimitiveTypeName::U32).into(),
            }],
            kind: KirInstructionKind::Copy {
                value: ValueId::from_index(2),
            },
            memory: None,
            effect: None,
        },
        predicate,
    ];

    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O1, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let optimized = result.artifact.expect("validated optimized KIR");
    assert!(validate_kir_module(&optimized).errors.is_empty());
}
