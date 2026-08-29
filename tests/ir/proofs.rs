use calckernel::{
    BlockId, ContractInstanceId, FactArena, FactDerivation, FactOrigin, FactPredicate, FactScope,
    FactUseSite, FunctionId, InstructionId, KirBoundsMode, KirBuildConfig, KirConsumer,
    KirInstructionKind, KirOverflowMode, KirSanitizerMode, ProofArena, ProofStep, ProofStepId,
    ScalarAnalysisConfig, ScalarClaim, ScalarFailure, ScalarInterval, SourceFile, ValueId,
    analyze_scalar_function, build_kir_module, check, import_contract_facts, lower_to_mir,
    materialize_scalar_facts, print_fact_arena, print_proof_arena, verify_fact_arena,
    verify_proof_arena, verify_scalar_analysis_result,
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
                calckernel::MirType::Primitive(calckernel::MirPrimitiveTypeName::U32)
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
