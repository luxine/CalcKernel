use calckernel::{
    BlockId, ContractInstanceId, FactArena, FactDerivation, FactOrigin, FactPredicate, FactScope,
    FactUseSite, FunctionId, InstructionId, KirBoundsMode, KirBuildConfig, KirConsumer,
    KirInstructionKind, KirOverflowMode, KirSanitizerMode, ProofArena, ProofStep, ProofStepId,
    ScalarAnalysisConfig, ScalarClaim, ScalarFailure, ScalarInterval, SourceFile, ValueId,
    analyze_canonical_loops, analyze_scalar_function, build_kir_module, canonicalize_kir_loops,
    check, import_contract_facts, lower_to_mir, materialize_scalar_facts, print_fact_arena,
    print_proof_arena, verify_fact_arena, verify_proof_arena, verify_scalar_analysis_result,
};
use num_bigint::BigInt;

fn interval(lower: i64, upper: i64) -> ScalarInterval {
    ScalarInterval::new(BigInt::from(lower), BigInt::from(upper)).expect("valid interval")
}

fn build(source_text: &str) -> (calckernel::CheckedProgram, calckernel::KirModule) {
    let checked = check(&SourceFile::new("proof.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR construction");
    (checked.checked_program, kir)
}

#[test]
fn proof_fact_ids_origins_and_scopes_should_be_stable() {
    let mut facts = FactArena::new(3);
    let entry = facts
        .try_insert(
            FactOrigin::TrustedContract {
                instance: ContractInstanceId::from_index(4),
            },
            FactScope::FunctionEntry(FunctionId::from_index(1)),
            FactPredicate::ValueInterval {
                value: ValueId::from_index(7),
                interval: interval(0, 63),
            },
            FactDerivation::TrustedContractLeaf,
        )
        .expect("valid entry fact");
    let derived = facts
        .try_insert(
            FactOrigin::Proven,
            FactScope::Block {
                function: FunctionId::from_index(1),
                block: BlockId::from_index(2),
            },
            FactPredicate::ValueInterval {
                value: ValueId::from_index(9),
                interval: interval(1, 64),
            },
            FactDerivation::BinaryTransfer {
                instruction: InstructionId::from_index(5),
                inputs: vec![entry],
            },
        )
        .expect("valid derived fact");

    assert_eq!(entry.index(), 0);
    assert_eq!(derived.index(), 1);
    assert_eq!(facts.generation(), 3);
    assert_eq!(
        facts.get(entry).expect("entry fact").origin,
        FactOrigin::TrustedContract {
            instance: ContractInstanceId::from_index(4),
        }
    );

    let expected = concat!(
        "facts generation=3\n",
        "fact0 trusted-contract(instance=ci4) scope=function-entry(f1) ",
        "range(v7, 0..=63) <- trusted-contract\n",
        "fact1 proven scope=block(f1,b2) range(v9, 1..=64) ",
        "<- binary(i5; fact0)\n",
    );
    assert_eq!(print_fact_arena(&facts), expected);
    for _ in 0..50 {
        assert_eq!(print_fact_arena(&facts), expected);
    }
}

#[test]
fn proof_fact_arena_should_reject_forward_dependencies() {
    let mut facts = FactArena::new(0);
    let error = facts
        .try_insert(
            FactOrigin::Proven,
            FactScope::FunctionEntry(FunctionId::from_index(0)),
            FactPredicate::ValueInterval {
                value: ValueId::from_index(0),
                interval: interval(0, 0),
            },
            FactDerivation::BinaryTransfer {
                instruction: InstructionId::from_index(0),
                inputs: vec![calckernel::FactId::from_index(1)],
            },
        )
        .expect_err("forward dependency must fail");

    assert_eq!(
        error.to_string(),
        "fact dependency fact1 is not already defined"
    );
}

#[test]
fn proof_checker_should_accept_closed_constant_and_binary_certificate_deterministically() {
    let (_, kir) = build("export fn answer() -> i32 { return 20 + 22; }");
    let function = &kir.functions[0];
    let block = &function.blocks[0];
    let constants = block
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::ConstInt { .. }))
        .collect::<Vec<_>>();
    let binary = block
        .instructions
        .iter()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .expect("binary");
    let mut proofs = ProofArena::new(7);
    let proof = proofs
        .try_insert(
            FactUseSite {
                function: function.id,
                block: block.id,
                instruction: Some(binary.id),
                contract_instance: None,
            },
            vec![
                ProofStep::Constant {
                    instruction: constants[0].id,
                    claim: ScalarClaim::new(
                        constants[0].results[0].value,
                        interval(20, 20),
                        ScalarFailure::None,
                    ),
                },
                ProofStep::Constant {
                    instruction: constants[1].id,
                    claim: ScalarClaim::new(
                        constants[1].results[0].value,
                        interval(22, 22),
                        ScalarFailure::None,
                    ),
                },
                ProofStep::BinaryTransfer {
                    instruction: binary.id,
                    left: ProofStepId::from_index(0),
                    right: ProofStepId::from_index(1),
                    claim: ScalarClaim::new(
                        binary.results[0].value,
                        interval(42, 42),
                        ScalarFailure::None,
                    ),
                },
            ],
            ProofStepId::from_index(2),
        )
        .expect("closed proof");

    assert_eq!(proof.index(), 0);
    assert_eq!(
        verify_proof_arena(&kir, &FactArena::new(7), None, &proofs, 7).errors,
        []
    );
    let printed = print_proof_arena(&proofs);
    for _ in 0..50 {
        assert_eq!(print_proof_arena(&proofs), printed);
    }
}

#[test]
fn mutation_fake_producer_and_proof_id_should_be_rejected() {
    let (_, kir) = build("export fn answer() -> i32 { return 7; }");
    let function = &kir.functions[0];
    let block = &function.blocks[0];
    let constant = block
        .instructions
        .iter()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::ConstInt { .. }))
        .expect("constant");
    let mut proofs = ProofArena::new(0);
    proofs
        .try_insert(
            FactUseSite {
                function: function.id,
                block: block.id,
                instruction: Some(constant.id),
                contract_instance: None,
            },
            vec![ProofStep::Constant {
                instruction: constant.id,
                claim: ScalarClaim::new(
                    constant.results[0].value,
                    interval(8, 8),
                    ScalarFailure::None,
                ),
            }],
            ProofStepId::from_index(0),
        )
        .expect("structurally closed fake proof");

    assert_eq!(
        verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0).errors[0].message,
        "proof0 step0 constant claim does not match KIR instruction"
    );
}

#[test]
fn mutation_stale_fact_origin_and_wrong_contract_instance_should_be_rejected() {
    let (checked, kir) = build(
        r#"
        export unsafe fn bounded(n: u32) -> u32
        contract { requires n < 8; }
        { return n; }
        "#,
    );
    let imported = import_contract_facts(&kir, &checked, 5).expect("contract facts");
    assert_eq!(
        verify_fact_arena(&kir, Some(&imported), imported.facts(), 5).errors,
        []
    );

    let mut stale = imported.facts().clone();
    stale
        .get_mut(calckernel::FactId::from_index(0))
        .expect("fact")
        .generation = 4;
    assert_eq!(
        verify_fact_arena(&kir, Some(&imported), &stale, 5).errors[0].message,
        "fact0 belongs to stale generation 4, expected 5"
    );

    let mut wrong_origin = imported.facts().clone();
    wrong_origin
        .get_mut(calckernel::FactId::from_index(0))
        .expect("fact")
        .origin = FactOrigin::Proven;
    assert_eq!(
        verify_fact_arena(&kir, Some(&imported), &wrong_origin, 5).errors[0].message,
        "fact0 proven origin cannot use a trusted-contract derivation"
    );

    let mut wrong_instance = imported.facts().clone();
    wrong_instance
        .get_mut(calckernel::FactId::from_index(0))
        .expect("fact")
        .origin = FactOrigin::TrustedContract {
        instance: ContractInstanceId::from_index(99),
    };
    assert_eq!(
        verify_fact_arena(&kir, Some(&imported), &wrong_instance, 5).errors[0].message,
        "fact0 names missing contract instance ci99"
    );
}

#[test]
fn proof_loop_invariant_should_require_the_claimed_transfer_on_every_backedge() {
    for binds_backedge in [false, true] {
        let (_, mut kir) = build(
            "export fn count(n: u32) -> u32 { let i: u32 = 0; while i < n { let unused: u32 = i + 0; i = i + 1; } return i; }",
        );
        let function = &mut kir.functions[0];
        let header = calckernel::analyze_natural_loops(function).loops[0].header;
        let header_block = function
            .blocks
            .iter()
            .find(|block| block.id == header)
            .expect("header");
        let (phi_index, param) = header_block
            .params
            .iter()
            .enumerate()
            .find(|(_, param)| param.slot == "i")
            .expect("induction phi");
        let phi = param.value;
        let instruction = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
            .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
            .expect("unused i+0");
        let KirInstructionKind::Binary { left, .. } = &mut instruction.kind else {
            unreachable!()
        };
        *left = phi;
        let transfer = instruction.id;
        let transferred_value = instruction.results[0].value;
        if binds_backedge {
            let entry = function.blocks[0].id;
            for block in &mut function.blocks {
                if block.id != entry
                    && let calckernel::KirTerminator::Jump { edge } = &mut block.terminator
                    && edge.target == header
                {
                    edge.args[phi_index] = transferred_value;
                }
            }
        }
        let mut proofs = ProofArena::new(0);
        proofs
            .try_insert(
                FactUseSite {
                    function: function.id,
                    block: header,
                    instruction: None,
                    contract_instance: None,
                },
                vec![ProofStep::LoopInvariant {
                    header,
                    phi,
                    transfer,
                    claim: ScalarClaim::new(phi, interval(0, 0), ScalarFailure::None),
                }],
                ProofStepId::from_index(0),
            )
            .expect("closed proof draft");
        assert!(calckernel::validate_kir_module(&kir).errors.is_empty());
        let result = verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0);
        assert_eq!(
            result.errors.is_empty(),
            binds_backedge,
            "an unused i+0 must not certify a loop whose actual backedge is i+1: {result:?}"
        );
    }
}

#[test]
fn proof_loop_guard_should_use_ssa_identity_not_source_slot_names() {
    let (checked, mut kir) = build(
        "export unsafe fn sum(items: slice<i32>, len: u32) -> i32 contract { requires len <= items.len; effects read(items); } { let i: u32 = 0; let total: i32 = 0; while i < len { total = total + items[i]; i = i + 1; } return total; }",
    );
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contract import");
    let function = &kir.functions[0];
    let (block, condition) = function
        .blocks
        .iter()
        .find_map(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::CheckCondition {
                            kind: calckernel::KirCheckConditionKind::SliceOutOfBounds,
                            ..
                        }
                    )
                })
                .map(|condition| (block.id, condition.id))
        })
        .expect("bounds condition");
    let mut steps = contracts
        .facts()
        .facts()
        .iter()
        .map(|fact| ProofStep::FactLeaf { fact: fact.id })
        .collect::<Vec<_>>();
    let root = ProofStepId::from_index(steps.len() as u32);
    steps.push(ProofStep::GuardSafety {
        condition_instruction: condition,
        premises: (0..root.index()).map(ProofStepId::from_index).collect(),
        allow_loop_reasoning: true,
    });
    let mut proofs = ProofArena::new(0);
    proofs
        .try_insert(
            FactUseSite {
                function: function.id,
                block,
                instruction: Some(condition),
                contract_instance: None,
            },
            steps,
            root,
        )
        .expect("closed certificate");
    assert_eq!(
        verify_proof_arena(&kir, contracts.facts(), Some(&contracts), &proofs, 0).errors,
        []
    );
    for param in kir.functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.params)
    {
        param.slot = format!("anonymous_{}", param.value.index());
    }
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);
    assert_eq!(
        verify_proof_arena(&kir, contracts.facts(), Some(&contracts), &proofs, 0).errors,
        [],
        "renaming source slots must not invalidate actual SSA identity"
    );
}

#[test]
fn proof_loop_guard_should_require_the_taken_edge_not_only_its_target() {
    let (_, mut kir) = build(
        "export fn count(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; } return i; }",
    );
    let function = &kir.functions[0];
    let (block, condition) = function
        .blocks
        .iter()
        .find_map(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
                .map(|instruction| (block.id, instruction.id))
        })
        .expect("checked increment");
    let mut proofs = ProofArena::new(0);
    proofs
        .try_insert(
            FactUseSite {
                function: function.id,
                block,
                instruction: Some(condition),
                contract_instance: None,
            },
            vec![ProofStep::GuardSafety {
                condition_instruction: condition,
                premises: vec![],
                allow_loop_reasoning: true,
            }],
            ProofStepId::from_index(0),
        )
        .expect("closed certificate");
    assert_eq!(
        verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0).errors,
        []
    );
    let header = kir.functions[0]
        .blocks
        .iter_mut()
        .find(|block| matches!(block.terminator, calckernel::KirTerminator::Branch { .. }))
        .expect("header");
    let calckernel::KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &mut header.terminator
    else {
        unreachable!()
    };
    *else_edge = then_edge.clone();
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);
    assert!(
        !verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0)
            .errors
            .is_empty(),
        "the false edge also enters the body, so i < n is not established"
    );
}

#[test]
fn proof_checker_should_not_call_the_optimizing_loop_analysis() {
    let checker = include_str!("../../src/optimizer/verify.rs");
    assert!(
        !checker.contains("analyze_natural_loops"),
        "proof checking must be independent of optimizing loop analysis"
    );
}

#[test]
fn proof_checker_should_reject_invalid_loop_invariant_and_budget_identity() {
    let (_, kir) = build(
        r#"
        export fn count(n: u32) -> u32 {
          let i: u32 = 0;
          while i < n { i = i + 1; }
          return i;
        }
        "#,
    );
    let function = kir
        .functions
        .iter()
        .find(|f| f.name == "count")
        .expect("count");
    let analysis = analyze_scalar_function(function, ScalarAnalysisConfig::default())
        .expect("scalar analysis");
    assert_eq!(
        verify_scalar_analysis_result(function, &analysis).errors,
        []
    );

    let mut changed_function = function.clone();
    let duplicate = changed_function.blocks[0].instructions[0].clone();
    changed_function.blocks[0].instructions.push(duplicate);
    assert_eq!(
        verify_scalar_analysis_result(&changed_function, &analysis).errors[0].message,
        "scalar analysis budget identity does not match current KIR"
    );

    let header = function.blocks[1].id;
    let phi = function.blocks[1]
        .params
        .iter()
        .find(|param| {
            matches!(
                param.type_node,
                calckernel::KirValueType::Scalar(calckernel::MirType::Primitive(
                    calckernel::MirPrimitiveTypeName::U32
                ))
            )
        })
        .expect("u32 loop phi")
        .value;
    let transfer = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .expect("loop transfer");
    let mut proofs = ProofArena::new(0);
    proofs
        .try_insert(
            FactUseSite {
                function: function.id,
                block: header,
                instruction: None,
                contract_instance: None,
            },
            vec![ProofStep::LoopInvariant {
                header,
                phi,
                transfer: transfer.id,
                claim: ScalarClaim::new(phi, interval(0, 1), ScalarFailure::None),
            }],
            ProofStepId::from_index(0),
        )
        .expect("structural invariant draft");
    assert_eq!(
        verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0).errors[0].message,
        "proof0 step0 loop invariant is not closed under its transfer"
    );
}

#[test]
fn mutation_non_dominating_fact_should_be_rejected() {
    let (_, kir) = build(
        r#"
        export fn choose(flag: bool) -> i32 {
          let value: i32 = 0;
          if flag { value = 7; }
          return value;
        }
        "#,
    );
    let function = &kir.functions[0];
    let analysis = analyze_scalar_function(function, ScalarAnalysisConfig::default())
        .expect("scalar analysis");
    let facts = materialize_scalar_facts(function, &analysis, 0).expect("facts");
    let seven_instruction = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "7")
        })
        .expect("seven");
    let seven_fact = facts
        .facts()
        .iter()
        .find(|fact| {
            matches!(
                fact.derivation,
                FactDerivation::Constant { instruction } if instruction == seven_instruction.id
            )
        })
        .expect("seven fact")
        .id;
    let return_block = function
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, calckernel::KirTerminator::Return { .. }))
        .expect("return block");
    let mut proofs = ProofArena::new(0);
    proofs
        .try_insert(
            FactUseSite {
                function: function.id,
                block: return_block.id,
                instruction: None,
                contract_instance: None,
            },
            vec![ProofStep::FactLeaf { fact: seven_fact }],
            ProofStepId::from_index(0),
        )
        .expect("fact leaf draft");

    assert_eq!(
        verify_proof_arena(&kir, &facts, None, &proofs, 0).errors[0].message,
        "proof0 step0 fact does not dominate the proof use"
    );
}

#[test]
fn loop_proof_should_recompute_canonical_shape_and_exact_trip_count() {
    let (_, mut kir) =
        build("export fn count() -> u32 { let i: u32 = 0; while i < 8 { i = i + 1; } return i; }");
    canonicalize_kir_loops(&mut kir).expect("loop simplify");
    let analysis = analyze_canonical_loops(&kir.functions[0]);
    let descriptor = &analysis.loops[0];
    let calckernel::LoopTripCount::Exact { iterations } = descriptor.trip_count else {
        panic!("exact trip")
    };
    let induction = descriptor.induction.as_ref().expect("induction");
    let mut proofs = ProofArena::new(0);
    let proof_id = proofs
        .try_insert(
            FactUseSite {
                function: kir.functions[0].id,
                block: descriptor.header,
                instruction: None,
                contract_instance: None,
            },
            vec![
                ProofStep::CanonicalLoop {
                    loop_id: descriptor.id,
                    cfg_digest: analysis.cfg_digest.clone(),
                    header: descriptor.header,
                    preheader: descriptor.preheader.expect("preheader"),
                    latch: descriptor.latch.expect("latch"),
                    blocks: descriptor.blocks.clone(),
                    exits: descriptor.exits.clone(),
                },
                ProofStep::LoopTripCount {
                    canonical_loop: ProofStepId::from_index(0),
                    induction: induction.value,
                    start: induction.start.to_string(),
                    step: induction.step.to_string(),
                    bound: induction.bound,
                    comparison: induction.comparison,
                    iterations,
                },
            ],
            ProofStepId::from_index(1),
        )
        .expect("loop certificate");
    assert!(
        verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0)
            .errors
            .is_empty()
    );

    let ProofStep::LoopTripCount { iterations, .. } =
        &mut proofs.get_mut(proof_id).expect("proof").steps[1]
    else {
        unreachable!()
    };
    *iterations += 1;
    let errors = verify_proof_arena(&kir, &FactArena::new(0), None, &proofs, 0).errors;
    assert!(errors[0].message.contains("trip count"), "{errors:?}");
}
