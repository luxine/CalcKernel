use calckernel::{
    BlockId, CandidateBudgetCharge, FactUseSite, FunctionId, InstructionId, KirBoundsMode,
    KirBuildConfig, KirConsumer, KirCostEstimate, KirCostKey, KirLegalCost, KirNativeCpuPolicy,
    KirOverflowMode, KirPreStateIdentity, KirProfileOperation, KirSanitizerMode, KirTargetProfile,
    KirTargetProfileBuilder, KirVerifiedProgramState, LoopId, MemoryRegionId, ProofArena, ProofId,
    ProofStep, ProofStepId, ScalarClaim, ScalarFailure, ScalarInterval, SlpPlan, SlpProofRecord,
    SourceFile, SpecializationPlan, UnrollInstructionMapping, UnrollPlan, UnrollProofRecord,
    ValueId, VectorEpilogue, VectorLaneMapping, VectorMemoryAccessKind, VectorMemoryGroup,
    VectorOperationMapping, VectorPlanGrowth, VectorPredicate, VectorProofRoots, VectorizationPlan,
    build_kir_module, check, check_vectorization_plan_independently, kir_function_units,
    lower_to_mir, print_vectorization_plan, validate_vectorization_plan,
};

fn profile() -> KirTargetProfile {
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        "x86_64-unknown-linux-gnu",
        64,
        true,
        KirNativeCpuPolicy::Baseline,
        "x86-64",
        vec!["+sse2".to_string()],
    )
    .expect("native profile");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lane == calckernel::KirLaneType::I32
                && key.lanes == 4
                && key.operation != KirProfileOperation::MaskNot
        })
    {
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type: "v4i32".to_string(),
                },
            )
            .expect("legal query");
    }
    builder.build().expect("profile")
}

fn plan(profile: &KirTargetProfile) -> VectorizationPlan {
    VectorizationPlan {
        pre_state: KirPreStateIdentity {
            function: FunctionId::from_index(1),
            kir_digest: "11".repeat(32),
            profile_digest: profile.digest_hex(),
            evidence_generation: 7,
            frozen_kir_units: 20,
        },
        loop_id: LoopId::from_index(2),
        vf: 4,
        uf: 1,
        operations: vec![VectorOperationMapping {
            scalar: InstructionId::from_index(10),
            vector: InstructionId::from_index(30),
            unroll_index: 0,
            operation: KirProfileOperation::Add,
            lane_type: calckernel::KirLaneType::I32,
            semantics: calckernel::KirCostSemantics::Modular,
            alignment: calckernel::KirAlignmentClass::NotApplicable,
            lanes: vec![
                VectorLaneMapping {
                    lane: 0,
                    scalar_iteration: 0,
                },
                VectorLaneMapping {
                    lane: 1,
                    scalar_iteration: 1,
                },
                VectorLaneMapping {
                    lane: 2,
                    scalar_iteration: 2,
                },
                VectorLaneMapping {
                    lane: 3,
                    scalar_iteration: 3,
                },
            ],
        }],
        memory_groups: vec![VectorMemoryGroup {
            region: MemoryRegionId::from_index(0),
            access: VectorMemoryAccessKind::Read,
            scalar_instructions: vec![InstructionId::from_index(11)],
            vector_instruction: InstructionId::from_index(31),
            unroll_index: 0,
            footprint_proof: ProofId::from_index(4),
        }],
        predicates: vec![VectorPredicate::TripThreshold {
            trip_count: ValueId::from_index(5),
            minimum: 8,
            proof: ProofId::from_index(5),
        }],
        epilogue: VectorEpilogue::Scalar {
            start: ValueId::from_index(6),
            end: ValueId::from_index(5),
            coverage_proof: ProofId::from_index(6),
        },
        cost: KirCostEstimate {
            scalar: 100,
            transformed_body: 60,
            predicates: 4,
            epilogue: 6,
            total: 70,
        },
        growth: VectorPlanGrowth {
            original_units: 20,
            transformed_units: 30,
            module_before_units: 80,
            module_after_units: 90,
        },
        proofs: VectorProofRoots {
            canonical_loop: ProofId::from_index(0),
            trip_partition: ProofId::from_index(1),
            lane_mapping: ProofId::from_index(2),
            operation_equivalence: ProofId::from_index(3),
            fallback_identity: ProofId::from_index(7),
            target_legality: ProofId::from_index(8),
            cost_and_budget: ProofId::from_index(9),
        },
    }
}

fn independently_checkable_plan() -> (
    KirVerifiedProgramState,
    VectorizationPlan,
    CandidateBudgetCharge,
) {
    let profile = profile();
    let checked = check(&SourceFile::new(
        "independent-check.ck",
        "export fn answer(n: i32) -> i32 { return n + 1; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let mut module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    module.config.consumer = KirConsumer::NativeLibrary;
    module.profile = profile.clone();
    let function = &module.functions[0];
    let function_id = function.id;
    let block = function.blocks[0].id;
    let parameter = function.params[0].value;
    let scalar = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Binary {
                    op: calckernel::MirBinaryOp::Add,
                    ..
                }
            )
        })
        .expect("scalar add")
        .id;
    let generation = 7;
    let mut proofs = ProofArena::new(generation);
    for _ in 0..7 {
        proofs
            .try_insert(
                FactUseSite {
                    function: function_id,
                    block,
                    instruction: None,
                    contract_instance: None,
                },
                vec![ProofStep::TypeBounds {
                    claim: ScalarClaim::new(
                        parameter,
                        ScalarInterval::new((-2147483648_i64).into(), 2147483647_i64.into())
                            .unwrap(),
                        ScalarFailure::None,
                    ),
                }],
                ProofStepId::from_index(0),
            )
            .expect("proof");
    }
    let state = KirVerifiedProgramState::from_parts(module, None, proofs, vec![], generation)
        .expect("verified state");
    let original_units = kir_function_units(&state.module().functions[0]);
    let module_units = original_units;
    let plan = VectorizationPlan {
        pre_state: KirPreStateIdentity {
            function: function_id,
            kir_digest: state.kir_digest(),
            profile_digest: profile.digest_hex(),
            evidence_generation: generation,
            frozen_kir_units: original_units,
        },
        loop_id: LoopId::from_index(0),
        vf: 4,
        uf: 1,
        operations: vec![VectorOperationMapping {
            scalar,
            vector: InstructionId::from_index(100),
            unroll_index: 0,
            operation: KirProfileOperation::Add,
            lane_type: calckernel::KirLaneType::I32,
            semantics: calckernel::KirCostSemantics::Modular,
            alignment: calckernel::KirAlignmentClass::NotApplicable,
            lanes: (0..4)
                .map(|lane| VectorLaneMapping {
                    lane,
                    scalar_iteration: u32::from(lane),
                })
                .collect(),
        }],
        memory_groups: vec![],
        predicates: vec![],
        epilogue: VectorEpilogue::None,
        cost: KirCostEstimate::new(10, 8, 0, 0),
        growth: VectorPlanGrowth::new(
            original_units,
            original_units + 1,
            module_units,
            module_units + 1,
        ),
        proofs: VectorProofRoots {
            canonical_loop: ProofId::from_index(0),
            trip_partition: ProofId::from_index(1),
            lane_mapping: ProofId::from_index(2),
            operation_equivalence: ProofId::from_index(3),
            fallback_identity: ProofId::from_index(4),
            target_legality: ProofId::from_index(5),
            cost_and_budget: ProofId::from_index(6),
        },
    };
    // Fixed stage-05 structural accounting: 8 + 4 op + 4 lane steps,
    // and 16 + 6 op + 8 lane steps + 7 proof-root steps.
    let charge = CandidateBudgetCharge::single(function_id, 16, 37);
    (state, plan, charge)
}

#[test]
fn vector_plan_closed_schema_should_validate_and_print_deterministically() {
    let profile = profile();
    let plan = plan(&profile);
    assert_eq!(validate_vectorization_plan(&plan, &profile), Ok(()));
    let first = print_vectorization_plan(&plan);
    assert_eq!(first, print_vectorization_plan(&plan));
    for field in [
        "loop=2",
        "vf=4",
        "uf=1",
        "op=add",
        "lanes=0:0,1:1,2:2,3:3",
        "profile=",
        "cost=100/60/4/6/70",
    ] {
        assert!(first.contains(field), "missing {field}:\n{first}");
    }
}

#[test]
fn vector_plan_mutations_should_reject_profile_lane_cost_and_growth_claims() {
    let profile = profile();
    let mut stale = plan(&profile);
    stale.pre_state.profile_digest = "00".repeat(32);
    assert!(
        validate_vectorization_plan(&stale, &profile)
            .expect_err("stale profile")
            .contains("profile")
    );

    let mut lanes = plan(&profile);
    lanes.operations[0].lanes[3].lane = 2;
    assert!(
        validate_vectorization_plan(&lanes, &profile)
            .expect_err("bad lanes")
            .contains("lane mapping")
    );

    let mut cost = plan(&profile);
    cost.cost.total = 69;
    assert!(
        validate_vectorization_plan(&cost, &profile)
            .expect_err("bad total")
            .contains("cost")
    );

    let mut growth = plan(&profile);
    growth.growth.transformed_units = 100;
    assert!(
        validate_vectorization_plan(&growth, &profile)
            .expect_err("bad growth")
            .contains("growth")
    );
}

#[test]
fn vector_plan_related_closed_records_should_have_no_open_operation_strings() {
    let profile = profile();
    let pre_state = plan(&profile).pre_state;
    let slp = SlpPlan {
        pre_state: pre_state.clone(),
        block: BlockId::from_index(3),
        root: InstructionId::from_index(4),
        lanes: 4,
        lane_type: calckernel::KirLaneType::I32,
        semantics: calckernel::KirCostSemantics::Modular,
        scalar_instructions: vec![
            InstructionId::from_index(4),
            InstructionId::from_index(5),
            InstructionId::from_index(6),
            InstructionId::from_index(7),
        ],
        setup_instructions: vec![],
        vector_instructions: vec![InstructionId::from_index(8)],
        extracts: vec![],
        operations: vec![KirProfileOperation::Multiply],
        memory: None,
        cost: KirCostEstimate::new(20, 12, 0, 0),
        growth: VectorPlanGrowth::new(8, 10, 80, 82),
        proof: SlpProofRecord {
            block: BlockId::from_index(3),
            source_order: vec![
                InstructionId::from_index(4),
                InstructionId::from_index(5),
                InstructionId::from_index(6),
                InstructionId::from_index(7),
            ],
            identity_lanes: vec![0, 1, 2, 3],
            barrier_free: true,
            exact_memory_footprint: None,
        },
    };
    let unroll = UnrollPlan {
        pre_state: pre_state.clone(),
        function: pre_state.function,
        loop_id: LoopId::from_index(2),
        header: BlockId::from_index(2),
        factor: 4,
        full: false,
        trip_count: 12,
        remainder: 0,
        body_units: 4,
        o3_entry_module_units: 80,
        instruction_mapping: vec![UnrollInstructionMapping {
            scalar_iteration: 0,
            source: InstructionId::from_index(4),
            transformed: InstructionId::from_index(4),
        }],
        cost: KirCostEstimate::new(40, 30, 0, 0),
        growth: VectorPlanGrowth::new(20, 28, 80, 88),
        proof: UnrollProofRecord {
            cfg_digest: "33".repeat(32),
            source_order: vec![InstructionId::from_index(4)],
            iterations: 12,
            factor: 4,
            remainder: 0,
            dedicated_exits: true,
            lcssa: true,
        },
    };
    let specialization = SpecializationPlan {
        pre_state,
        caller: FunctionId::from_index(1),
        call: InstructionId::from_index(9),
        callee: FunctionId::from_index(2),
        fact_set_digest: "22".repeat(32),
        clone_ordinal: 1,
        clone: FunctionId::from_index(3),
        clone_name: "__ck_spec_f2_f2_2222222222222222".to_string(),
        reused: false,
        o3_entry_module_units: 80,
        facts: vec![],
        mapping: calckernel::SpecializationIdMapping {
            parameters: vec![],
            blocks: vec![],
            instructions: vec![],
            values: vec![],
            memory_regions: vec![],
            memory_versions: vec![],
            vector_regions: vec![],
        },
        cost: KirCostEstimate::new(30, 20, 0, 0),
        growth: VectorPlanGrowth::new(20, 24, 80, 84),
        argument_mapping_proof: ProofId::from_index(3),
        fact_scope_proof: ProofId::from_index(4),
    };
    let text = format!("{slp:?}\n{unroll:?}\n{specialization:?}");
    assert!(text.contains("Multiply"));
    assert!(!text.contains("operation: \""));
}

#[test]
fn vector_plan_profile_query_should_not_accept_an_unavailable_operation() {
    let profile = profile();
    let mut plan = plan(&profile);
    plan.operations[0].operation = KirProfileOperation::MaskNot;
    assert!(
        validate_vectorization_plan(&plan, &profile)
            .expect_err("unavailable operation")
            .contains("target operation")
    );

    let key = KirCostKey {
        operation: KirProfileOperation::MaskNot,
        lane: calckernel::KirLaneType::I32,
        lanes: 4,
        semantics: calckernel::KirCostSemantics::NotApplicable,
        alignment: calckernel::KirAlignmentClass::NotApplicable,
    };
    assert!(matches!(
        profile.operation_availability(&key),
        Some(calckernel::KirOperationAvailability::Unavailable)
    ));
}

#[test]
fn independent_checker_should_accept_a_closed_plan_and_reject_each_forged_dimension() {
    let (state, plan, charge) = independently_checkable_plan();
    assert_eq!(
        check_vectorization_plan_independently(&state, &plan, &charge),
        Ok(())
    );

    let mut mutations: Vec<(&str, VectorizationPlan, CandidateBudgetCharge)> = Vec::new();
    let mut cost = plan.clone();
    cost.cost.total -= 1;
    mutations.push(("cost", cost, charge.clone()));
    let mut growth = plan.clone();
    growth.growth.module_after_units += 1;
    mutations.push(("growth", growth, charge.clone()));
    let mut profile = plan.clone();
    profile.pre_state.profile_digest = "00".repeat(32);
    mutations.push(("profile", profile, charge.clone()));
    let mut proof = plan.clone();
    proof.proofs.target_legality = ProofId::from_index(99);
    mutations.push(("proof", proof, charge.clone()));
    let mut lanes = plan.clone();
    lanes.operations[0].lanes.swap(1, 2);
    mutations.push(("lane-map", lanes, charge.clone()));
    let mut fallback = plan.clone();
    fallback.proofs.fallback_identity = fallback.proofs.canonical_loop;
    mutations.push(("fallback", fallback, charge.clone()));
    let mut budget = charge.clone();
    budget.checker_units += 1;
    mutations.push(("budget", plan.clone(), budget));

    for (dimension, mutation, charge) in mutations {
        assert!(
            check_vectorization_plan_independently(&state, &mutation, &charge).is_err(),
            "forged {dimension} was accepted"
        );
    }
}

#[test]
fn independent_checker_should_reject_scalar_operation_and_stale_kir_forgery() {
    let (state, plan, charge) = independently_checkable_plan();
    let mut operation = plan.clone();
    operation.operations[0].operation = KirProfileOperation::Multiply;
    assert!(check_vectorization_plan_independently(&state, &operation, &charge).is_err());
    let mut stale = plan;
    stale.pre_state.kir_digest = "11".repeat(32);
    assert!(check_vectorization_plan_independently(&state, &stale, &charge).is_err());
}
