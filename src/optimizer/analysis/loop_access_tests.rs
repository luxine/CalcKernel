use super::*;
use crate::{
    BlockId, ContractFactPointer, ContractFactPredicate, ContractInstanceId, FactDerivation,
    FactOrigin, FactPredicate, FactScope, FunctionId, InductionVariable, KirArithmeticSemantics,
    KirBlock, KirBlockParam, KirBoundsMode, KirBuildConfig, KirConsumer, KirEdge, KirFunction,
    KirInstruction, KirMemoryRegion, KirMemoryRegionOrigin, KirOverflowMode, KirParam, KirPlace,
    KirResult, KirSanitizerMode, KirTerminator, KirValueType, LoopId, LoopTripCount,
    MemoryRegionId, MirCompareOp, validate_kir_module,
};

fn value(index: u32) -> ValueId {
    ValueId::from_index(index)
}

fn block(index: u32) -> BlockId {
    BlockId::from_index(index)
}

fn result(value: ValueId, ty: MirType) -> KirResult {
    KirResult {
        value,
        type_node: KirValueType::scalar(ty),
    }
}

fn edge(target: u32, args: Vec<ValueId>) -> KirEdge {
    KirEdge {
        target: block(target),
        args,
        memory_args: Vec::new(),
    }
}

fn affine_fixture(index: ValueId) -> (KirFunction, CanonicalLoopDescriptor) {
    let u32_type = MirType::Primitive(MirPrimitiveTypeName::U32);
    let i32_type = MirType::Primitive(MirPrimitiveTypeName::I32);
    let base = value(0);
    let row = value(1);
    let column = value(2);
    let bound = value(3);
    let induction = value(4);
    let initial = value(5);
    let two = value(6);
    let scaled_row = value(7);
    let row_and_column = value(8);
    let loaded = value(10);
    let next_induction = value(11);
    let condition = value(12);
    let region = MemoryRegionId::from_index(0);
    let mut body_instructions = vec![
        KirInstruction {
            id: InstructionId::from_index(1),
            results: vec![result(two, u32_type.clone())],
            kind: KirInstructionKind::ConstInt {
                value: "2".to_string(),
            },
            memory: None,
            effect: None,
        },
        KirInstruction {
            id: InstructionId::from_index(2),
            results: vec![result(scaled_row, u32_type.clone())],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Mul,
                left: two,
                right: row,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        },
        KirInstruction {
            id: InstructionId::from_index(3),
            results: vec![result(row_and_column, u32_type.clone())],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: scaled_row,
                right: column,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        },
    ];
    if index == value(9) {
        body_instructions.push(KirInstruction {
            id: InstructionId::from_index(4),
            results: vec![result(index, u32_type.clone())],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: row_and_column,
                right: induction,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        });
    }
    body_instructions.push(KirInstruction {
        id: InstructionId::from_index(5),
        results: vec![result(loaded, i32_type.clone())],
        kind: KirInstructionKind::Load {
            place: Box::new(KirPlace::SliceIndex {
                slice: base,
                index,
                type_node: i32_type.clone(),
                region,
            }),
        },
        memory: None,
        effect: None,
    });

    let function = KirFunction {
        id: FunctionId::from_index(0),
        name: "affine_fixture".to_string(),
        exported: false,
        params: vec![
            KirParam {
                value: base,
                name: "base".to_string(),
                type_node: MirType::Slice(Box::new(i32_type.clone())),
            },
            KirParam {
                value: row,
                name: "row".to_string(),
                type_node: u32_type.clone(),
            },
            KirParam {
                value: column,
                name: "column".to_string(),
                type_node: u32_type.clone(),
            },
            KirParam {
                value: bound,
                name: "bound".to_string(),
                type_node: u32_type.clone(),
            },
        ],
        return_type: MirType::Void,
        regions: vec![KirMemoryRegion {
            id: region,
            origin: KirMemoryRegionOrigin::RawSlice(base),
            parent: None,
            partition: region,
            byte_interval: None,
        }],
        initial_memory: Vec::new(),
        vector_regions: Vec::new(),
        blocks: vec![
            KirBlock {
                id: block(0),
                label: "preheader".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: vec![KirInstruction {
                    id: InstructionId::from_index(0),
                    results: vec![result(initial, u32_type.clone())],
                    kind: KirInstructionKind::ConstInt {
                        value: "0".to_string(),
                    },
                    memory: None,
                    effect: None,
                }],
                terminator: KirTerminator::Jump {
                    edge: edge(1, vec![initial]),
                },
            },
            KirBlock {
                id: block(1),
                label: "header".to_string(),
                params: vec![KirBlockParam {
                    value: induction,
                    slot: "i".to_string(),
                    type_node: KirValueType::scalar(u32_type.clone()),
                }],
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Branch {
                    condition,
                    then_edge: edge(2, Vec::new()),
                    else_edge: edge(4, Vec::new()),
                },
            },
            KirBlock {
                id: block(2),
                label: "body".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: body_instructions,
                terminator: KirTerminator::Jump {
                    edge: edge(3, Vec::new()),
                },
            },
            KirBlock {
                id: block(3),
                label: "latch".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Jump {
                    edge: edge(1, vec![next_induction]),
                },
            },
            KirBlock {
                id: block(4),
                label: "exit".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Return {
                    value: None,
                    memory: Vec::new(),
                    effect_order: 0,
                },
            },
        ],
    };
    let descriptor = CanonicalLoopDescriptor {
        id: LoopId::from_index(0),
        function: function.id,
        header: block(1),
        preheader: Some(block(0)),
        latch: Some(block(3)),
        blocks: vec![block(1), block(2), block(3)],
        exits: vec![block(4)],
        parent: None,
        depth: 0,
        innermost: true,
        dedicated_exits: true,
        lcssa: true,
        induction: Some(InductionVariable {
            header: block(1),
            value: induction,
            type_node: super::super::IntegerType::U32,
            start: 0.into(),
            step: 1.into(),
            bound,
            comparison: MirCompareOp::Lt,
            wrap_safe_for_strict_bound: true,
        }),
        trip_count: LoopTripCount::Runtime {
            start: 0.into(),
            bound,
            step: 1.into(),
            comparison: MirCompareOp::Lt,
        },
    };
    (function, descriptor)
}

fn reachable_forwarding_diamonds(count: u32) -> (KirFunction, CanonicalLoopDescriptor) {
    let (mut function, mut descriptor) = affine_fixture(value(15));
    let u32_type = MirType::Primitive(MirPrimitiveTypeName::U32);
    let bool_type = MirType::Primitive(MirPrimitiveTypeName::Bool);
    function.params.push(KirParam {
        value: value(16),
        name: "branch_flag".to_string(),
        type_node: bool_type.clone(),
    });
    let header = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(1))
        .unwrap();
    header.params.push(KirBlockParam {
        value: value(15),
        slot: "invariant_index".to_string(),
        type_node: KirValueType::scalar(u32_type.clone()),
    });
    header.instructions.push(KirInstruction {
        id: InstructionId::from_index(10),
        results: vec![result(value(12), bool_type)],
        kind: KirInstructionKind::Compare {
            op: MirCompareOp::Lt,
            left: value(4),
            right: value(3),
        },
        memory: None,
        effect: None,
    });
    let KirTerminator::Branch { then_edge, .. } = &mut header.terminator else {
        panic!("fixture header should branch");
    };
    then_edge.target = block(100);

    let preheader = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(0))
        .unwrap();
    let KirTerminator::Jump {
        edge: incoming_edge,
    } = &mut preheader.terminator
    else {
        panic!("fixture preheader should jump");
    };
    incoming_edge.args.push(value(1));

    let mut previous = value(15);
    for index in 0..count {
        let condition_block = 100 + index * 4;
        let merged_value = value(1000 + index);
        function.blocks.extend([
            KirBlock {
                id: block(condition_block),
                label: "forward_condition".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Branch {
                    condition: value(16),
                    then_edge: edge(condition_block + 1, Vec::new()),
                    else_edge: edge(condition_block + 2, Vec::new()),
                },
            },
            KirBlock {
                id: block(condition_block + 1),
                label: "forward_left".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Jump {
                    edge: edge(condition_block + 3, vec![previous]),
                },
            },
            KirBlock {
                id: block(condition_block + 2),
                label: "forward_right".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Jump {
                    edge: edge(condition_block + 3, vec![previous]),
                },
            },
            KirBlock {
                id: block(condition_block + 3),
                label: "forward_merge".to_string(),
                params: vec![KirBlockParam {
                    value: merged_value,
                    slot: "forwarded_index".to_string(),
                    type_node: KirValueType::scalar(u32_type.clone()),
                }],
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Jump {
                    edge: edge(
                        if index + 1 == count {
                            2
                        } else {
                            condition_block + 4
                        },
                        Vec::new(),
                    ),
                },
            },
        ]);
        descriptor
            .blocks
            .extend((condition_block..condition_block + 4).map(block));
        previous = merged_value;
    }

    let latch = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(3))
        .unwrap();
    latch.instructions.extend([
        KirInstruction {
            id: InstructionId::from_index(11),
            results: vec![result(value(17), u32_type.clone())],
            kind: KirInstructionKind::ConstInt {
                value: "1".to_string(),
            },
            memory: None,
            effect: None,
        },
        KirInstruction {
            id: InstructionId::from_index(12),
            results: vec![result(value(11), u32_type)],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: value(4),
                right: value(17),
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        },
    ]);
    let KirTerminator::Jump { edge } = &mut latch.terminator else {
        panic!("fixture latch should jump");
    };
    edge.args.push(previous);
    descriptor.blocks.sort_unstable();
    (function, descriptor)
}

#[test]
fn reachable_phi_diamonds_should_use_a_linear_forwarding_budget() {
    let diamonds = 14;
    let (function, descriptor) = reachable_forwarding_diamonds(diamonds);
    let module = crate::KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: crate::KirTargetProfile::inspection(),
        entry: None,
        structs: Vec::new(),
        functions: vec![function.clone()],
    };
    assert!(validate_kir_module(&module).errors.is_empty());

    let mut budget = AffineAnalysisBudget::for_function(&function);
    let index = ForwardingIndex::new(&function, &mut budget).expect("index fits its budget");
    let before_walk = budget.remaining_steps;
    let leaves = forwarding_leaves(&index, value(1000 + diamonds - 1), &mut budget)
        .expect("walk fits its budget")
        .expect("all phi nodes have incoming values");
    let walk_steps = before_walk - budget.remaining_steps;
    assert_eq!(leaves, BTreeSet::from([value(1)]));
    assert!(
        walk_steps <= 4 * diamonds + 8,
        "forwarding used {walk_steps} steps for {diamonds} diamonds"
    );

    let mut exhausted_budget = AffineAnalysisBudget {
        remaining_steps: 1,
        exhausted: false,
    };
    assert!(forwarding_leaves(&index, value(1000 + diamonds - 1), &mut exhausted_budget).is_err());
    assert!(exhausted_budget.exhausted);

    let analysis = analyze_affine_loop_accesses(&function, &descriptor, None).unwrap();
    assert!(!analysis.budget_exhausted);
    assert_eq!(analysis.accesses.len(), 1);
    assert!(analysis.rejected_instructions.is_empty());
}

#[test]
fn reachable_phi_forwarding_should_reject_an_alternate_root_even_across_a_cycle() {
    let (mut function, descriptor) = reachable_forwarding_diamonds(3);
    let alternate_arm = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(102))
        .unwrap();
    let KirTerminator::Jump { edge } = &mut alternate_arm.terminator else {
        panic!("fixture arm should jump");
    };
    edge.args[0] = value(2);

    let module = crate::KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: crate::KirTargetProfile::inspection(),
        entry: None,
        structs: Vec::new(),
        functions: vec![function.clone()],
    };
    assert!(validate_kir_module(&module).errors.is_empty());

    let analysis = analyze_affine_loop_accesses(&function, &descriptor, None).unwrap();
    assert!(!analysis.budget_exhausted);
    assert!(analysis.accesses.is_empty());
    assert_eq!(
        analysis.rejected_instructions,
        [InstructionId::from_index(5)]
    );
}

#[test]
fn alignment_fact_budget_exhaustion_should_reject_the_access() {
    let (function, descriptor) = affine_fixture(value(4));
    let mut facts = FactArena::new(0);
    for _ in 0..1000 {
        facts
            .try_insert(
                FactOrigin::TrustedContract {
                    instance: ContractInstanceId::from_index(0),
                },
                FactScope::FunctionEntry(function.id),
                FactPredicate::Contract(ContractFactPredicate::Aligned {
                    pointer: ContractFactPointer::Value(value(0)),
                    alignment: 16,
                }),
                FactDerivation::TrustedContractLeaf,
            )
            .unwrap();
    }

    let analysis = analyze_affine_loop_accesses(&function, &descriptor, Some(&facts)).unwrap();

    assert!(analysis.budget_exhausted);
    assert!(analysis.accesses.is_empty());
    assert_eq!(
        analysis.rejected_instructions,
        [InstructionId::from_index(5)]
    );

    let dependence = crate::analyze_loop_dependences(&function, &descriptor, Some(&facts)).unwrap();
    assert!(dependence.budget_exhausted);
    assert!(!dependence.all_writes_classified);
    assert!(dependence.reductions.is_empty());
}

fn direct_access(kind: LoopMemoryAccessKind) -> AffineMemoryAccess {
    let (function, descriptor) = affine_fixture(value(4));
    let mut access = analyze_affine_loop_accesses(&function, &descriptor, None)
        .unwrap()
        .accesses
        .remove(0);
    access.kind = kind;
    access
}

fn with_bias(mut access: AffineMemoryAccess, bias: i32) -> AffineMemoryAccess {
    access.affine_index = access.affine_index.add_constant(bias.into());
    access.bias = bias.into();
    access
}

#[test]
fn affine_access_should_preserve_multiple_scaled_invariant_leaves() {
    let index = value(9);
    let (function, descriptor) = affine_fixture(index);
    let analysis = analyze_affine_loop_accesses(&function, &descriptor, None).unwrap();

    assert!(analysis.rejected_instructions.is_empty());
    assert_eq!(analysis.accesses.len(), 1);
    let form = &analysis.accesses[0].affine_index;
    assert_eq!(form.coefficient(value(4)), 1.into());
    assert_eq!(form.coefficient(value(1)), 2.into());
    assert_eq!(form.coefficient(value(2)), 1.into());
    assert_eq!(form.terms().len(), 3);
    assert!(!analysis.accesses[0].legacy_projection_exact);
    assert_eq!(analysis.accesses[0].arithmetic, AffineArithmetic::MayWrap);
}

#[test]
fn lone_broadcast_store_should_fail_loop_legality_without_injectivity() {
    let broadcast_index = value(1);
    let (mut function, descriptor) = affine_fixture(broadcast_index);
    let body_block = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(2))
        .unwrap();
    let store = body_block
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == InstructionId::from_index(5))
        .unwrap();
    let KirInstructionKind::Load { place } = &store.kind else {
        panic!("fixture instruction should be a load");
    };
    store.kind = KirInstructionKind::Store {
        place: place.clone(),
        value: value(1),
    };

    let legality = crate::analyze_loop_legality(&function, &descriptor, None).unwrap();

    assert!(!legality.eligible);
    assert!(
        legality
            .fallback_reasons
            .contains(&crate::LoopFallbackReason::LoopCarriedDependence)
    );
}

#[test]
fn rejected_load_must_keep_loop_with_writes_illegal() {
    let (mut function, descriptor) = affine_fixture(value(4));
    let body = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(2))
        .unwrap();
    let store = body
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == InstructionId::from_index(5))
        .unwrap();
    let KirInstructionKind::Load { place } = &store.kind else {
        panic!("fixture instruction should be a load");
    };
    let store_place = place.clone();
    store.kind = KirInstructionKind::Store {
        place: store_place,
        value: value(1),
    };
    body.instructions.push(KirInstruction {
        id: InstructionId::from_index(6),
        results: vec![result(
            value(13),
            MirType::Primitive(MirPrimitiveTypeName::U32),
        )],
        kind: KirInstructionKind::Binary {
            op: MirBinaryOp::Div,
            left: value(1),
            right: value(2),
            semantics: KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });
    body.instructions.push(KirInstruction {
        id: InstructionId::from_index(7),
        results: vec![result(
            value(14),
            MirType::Primitive(MirPrimitiveTypeName::I32),
        )],
        kind: KirInstructionKind::Load {
            place: Box::new(KirPlace::SliceIndex {
                slice: value(0),
                index: value(13),
                type_node: MirType::Primitive(MirPrimitiveTypeName::I32),
                region: MemoryRegionId::from_index(0),
            }),
        },
        memory: None,
        effect: None,
    });

    let legality = crate::analyze_loop_legality(&function, &descriptor, None).unwrap();

    assert!(!legality.eligible);
    assert!(
        legality
            .fallback_reasons
            .contains(&crate::LoopFallbackReason::UnknownMemoryDependence)
    );
}

#[test]
fn shifted_memory_accesses_should_report_raw_war_and_waw_with_signed_distances() {
    let write_next = with_bias(direct_access(LoopMemoryAccessKind::Write), 1);
    let read_here = direct_access(LoopMemoryAccessKind::Read);
    let raw = crate::query_memory_dependence(
        &write_next,
        &read_here,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&write_next, &read_here),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        raw.dependences[0].direction,
        crate::DependenceDirection::Raw
    );
    assert_eq!(
        raw.dependences[0].relation,
        crate::DependenceRelation::LoopCarried
    );
    assert_eq!(raw.dependences[0].distance, Some(1));

    let read_here = direct_access(LoopMemoryAccessKind::Read);
    let write_previous = with_bias(direct_access(LoopMemoryAccessKind::Write), -1);
    let war = crate::query_memory_dependence(
        &read_here,
        &write_previous,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&read_here, &write_previous),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        war.dependences[0].direction,
        crate::DependenceDirection::War
    );
    assert_eq!(
        war.dependences[0].relation,
        crate::DependenceRelation::LoopCarried
    );
    assert_eq!(war.dependences[0].distance, Some(1));

    let write_here = direct_access(LoopMemoryAccessKind::Write);
    let write_next = with_bias(direct_access(LoopMemoryAccessKind::Write), 1);
    let waw = crate::query_memory_dependence(
        &write_here,
        &write_next,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&write_here, &write_next),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        waw.dependences[0].direction,
        crate::DependenceDirection::Waw
    );
    assert_eq!(
        waw.dependences[0].relation,
        crate::DependenceRelation::LoopCarried
    );
    assert_eq!(waw.dependences[0].distance, Some(-1));
}

#[test]
fn dependence_query_should_fail_closed_for_unknown_alias_and_exhausted_budget() {
    let write = direct_access(LoopMemoryAccessKind::Write);
    let read = direct_access(LoopMemoryAccessKind::Read);
    let unknown_alias = crate::query_memory_dependence(
        &write,
        &read,
        crate::AliasKind::MayAlias,
        crate::DependenceWrapFacts::from_accesses(&write, &read),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        unknown_alias.dependences[0].relation,
        crate::DependenceRelation::Unknown
    );

    let exhausted = crate::query_memory_dependence(
        &write,
        &read,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&write, &read),
        crate::DependenceQueryBudget::with_max_steps(0),
    );
    assert!(exhausted.budget_exhausted);
    assert_eq!(
        exhausted.dependences[0].relation,
        crate::DependenceRelation::Unknown
    );
}

#[test]
fn modular_index_difference_should_not_prove_disjointness_without_nonwrap_facts() {
    let mut left = direct_access(LoopMemoryAccessKind::Write);
    let mut right = with_bias(direct_access(LoopMemoryAccessKind::Write), 1);
    left.arithmetic = AffineArithmetic::MayWrap;
    right.arithmetic = AffineArithmetic::MayWrap;
    left.index_no_wrap = false;
    right.index_no_wrap = false;
    let unknown = crate::query_memory_dependence(
        &left,
        &right,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&left, &right),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        unknown.dependences[0].relation,
        crate::DependenceRelation::Unknown
    );

    let proven = crate::query_memory_dependence(
        &left,
        &right,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts {
            left: crate::IndexWrapProof::ProvenNoWrap,
            right: crate::IndexWrapProof::ProvenNoWrap,
        },
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        proven.dependences[0].relation,
        crate::DependenceRelation::LoopCarried
    );
}

#[test]
fn equal_index_writes_should_be_same_iteration_but_broadcast_writes_are_carried() {
    let write = direct_access(LoopMemoryAccessKind::Write);
    let same = crate::query_memory_dependence(
        &write,
        &write,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&write, &write),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        same.dependences[0].relation,
        crate::DependenceRelation::SameIteration
    );
    assert_eq!(
        same.dependences[0].direction,
        crate::DependenceDirection::Waw
    );

    let (function, descriptor) = affine_fixture(value(1));
    let broadcast = analyze_affine_loop_accesses(&function, &descriptor, None)
        .unwrap()
        .accesses
        .into_iter()
        .next()
        .unwrap();
    let mut broadcast = broadcast;
    broadcast.kind = LoopMemoryAccessKind::Write;
    let carried = crate::query_memory_dependence(
        &broadcast,
        &broadcast,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&broadcast, &broadcast),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        carried.dependences[0].relation,
        crate::DependenceRelation::LoopCarried
    );
    assert_eq!(carried.dependences[0].distance, Some(1));
}

#[test]
fn constant_stride_accesses_should_be_disjoint_when_progressions_do_not_intersect() {
    let mut left = direct_access(LoopMemoryAccessKind::Write);
    let mut right = direct_access(LoopMemoryAccessKind::Write);
    left.affine_index = crate::AffineForm::variable(value(4)).scale(&2.into());
    right.affine_index = left.affine_index.add_constant(1.into());
    left.arithmetic = AffineArithmetic::Mathematical;
    right.arithmetic = AffineArithmetic::Mathematical;
    let result = crate::query_memory_dependence(
        &left,
        &right,
        crate::AliasKind::MustAlias,
        crate::DependenceWrapFacts::from_accesses(&left, &right),
        crate::DependenceQueryBudget::default(),
    );
    assert_eq!(
        result.dependences[0].relation,
        crate::DependenceRelation::Independent
    );
}

#[test]
fn affine_access_analysis_should_fail_closed_when_recursion_depth_is_exhausted() {
    let (mut function, descriptor) = affine_fixture(value(4));
    let body = function
        .blocks
        .iter_mut()
        .find(|candidate| candidate.id == block(2))
        .unwrap();
    let load_index = body
        .instructions
        .iter()
        .position(|instruction| instruction.id == InstructionId::from_index(5))
        .unwrap();
    let mut load = body.instructions.remove(load_index);
    let mut previous = value(4);
    for index in 0..130_u32 {
        let next = value(1000 + index);
        body.instructions.push(KirInstruction {
            id: InstructionId::from_index(20 + index),
            results: vec![result(next, MirType::Primitive(MirPrimitiveTypeName::U32))],
            kind: KirInstructionKind::Copy { value: previous },
            memory: None,
            effect: None,
        });
        previous = next;
    }
    let KirInstructionKind::Load { place } = &mut load.kind else {
        panic!("fixture instruction should be a load");
    };
    let KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("fixture place should be a slice index");
    };
    *index = previous;
    body.instructions.push(load);

    let analysis = analyze_affine_loop_accesses(&function, &descriptor, None).unwrap();

    assert!(analysis.budget_exhausted);
    assert_eq!(
        analysis.rejected_instructions,
        [InstructionId::from_index(5)]
    );
}
