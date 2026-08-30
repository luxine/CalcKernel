use calckernel::{
    ContractFactSet, InstructionId, KirBoundsMode, KirBuildConfig, KirConsumer, KirInstructionKind,
    KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, ProofStep, SourceFile,
    build_kir_module, check, import_contract_facts, lower_to_mir, print_kir_module,
    run_kir_pass_pipeline, validate_kir_optimization_evidence,
};

fn build(
    source_text: &str,
) -> (
    calckernel::CheckedProgram,
    calckernel::KirModule,
    Option<ContractFactSet>,
) {
    build_with_overflow(source_text, KirOverflowMode::Checked)
}

fn build_with_overflow(
    source_text: &str,
    overflow_mode: KirOverflowMode,
) -> (
    calckernel::CheckedProgram,
    calckernel::KirModule,
    Option<ContractFactSet>,
) {
    let checked = check(&SourceFile::new("o1.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = checked
        .checked_program
        .functions
        .iter()
        .any(|function| function.is_unsafe)
        .then(|| import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts"));
    (checked.checked_program, kir, contracts)
}

#[test]
fn kir_o0_pipeline_should_validate_without_optional_rewrite() {
    let (_, kir, contracts) = build("export fn add(a: i32, b: i32) -> i32 { return a + b; }");
    let before = print_kir_module(&kir);
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O0, contracts.as_ref());

    assert!(result.errors.is_empty());
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].name, "verify-o0");
    assert!(result.records[0].verified);
    assert!(!result.records[0].changed);
    assert_eq!(
        print_kir_module(result.artifact.as_ref().expect("artifact")),
        before
    );
    assert!(result.eliminated_guards.is_empty());
}

#[test]
fn kir_o0_pipeline_should_reject_invalid_input_without_artifact() {
    let (_, mut kir, contracts) = build("export fn add(a: i32, b: i32) -> i32 { return a + b; }");
    let binary = kir.functions[0].blocks[0]
        .instructions
        .iter_mut()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .expect("binary");
    let KirInstructionKind::Binary { left, .. } = &mut binary.kind else {
        unreachable!();
    };
    *left = calckernel::ValueId::from_index(u32::MAX);
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O0, contracts.as_ref());

    assert!(result.artifact.is_none());
    assert!(!result.errors.is_empty());
}

#[test]
fn kir_o0_pipeline_should_reject_invalid_contract_facts_without_artifact() {
    let (_, kir, mut contracts) = build(
        r#"
        export unsafe fn bounded(n: u32) -> u32
        contract { requires n < 8; }
        { return n; }
        "#,
    );
    contracts
        .as_mut()
        .expect("contract facts")
        .facts_mut()
        .get_mut(calckernel::FactId::from_index(0))
        .expect("fact")
        .generation = 1;

    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O0, contracts.as_ref());

    assert!(result.artifact.is_none());
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("stale generation 1, expected 0")),
        "{:?}",
        result.errors
    );
}

#[test]
fn kir_o1_pipeline_should_use_the_exact_verified_pass_order() {
    let (_, kir, contracts) = build("export fn answer() -> i32 { return 20 + 22; }");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result
            .records
            .iter()
            .map(|record| record.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "cfg-canonicalize",
            "sccp-range",
            "check-elimination",
            "dead-code-elimination",
            "cleanup",
        ]
    );
    assert!(result.records.iter().all(|record| record.verified));
}

#[test]
fn kir_o1_sccp_should_fold_modular_integer_chains() {
    let (_, kir, contracts) = build_with_overflow(
        "export fn answer() -> i32 { return (20 + 22) * 2; }",
        KirOverflowMode::Unchecked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let module = result.artifact.as_ref().expect("verified artifact");
    let instructions = &module.functions[0].blocks[0].instructions;

    assert!(
        !instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. })),
        "SCCP must actually rewrite constant arithmetic:\n{}",
        print_kir_module(module)
    );
    assert!(
        instructions
            .iter()
            .any(|instruction| matches!(&instruction.kind,
        KirInstructionKind::ConstInt { value } if value == "84"))
    );
    assert!(
        result
            .records
            .iter()
            .any(|record| record.name == "sccp-range" && record.changed && record.verified)
    );
}

#[test]
fn kir_o1_sccp_should_respect_wrapping_at_all_integer_widths() {
    for (ty, maximum, expected) in [
        ("i32", "2147483647", "-2147483648"),
        ("u32", "4294967295", "0"),
        ("i64", "9223372036854775807", "-9223372036854775808"),
        ("u64", "18446744073709551615", "0"),
    ] {
        let source = format!("export fn wrapped() -> {ty} {{ return {maximum} + 1; }}");
        let (_, kir, contracts) = build_with_overflow(&source, KirOverflowMode::Unchecked);
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
        let module = result.artifact.as_ref().expect("verified artifact");
        assert!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .any(|instruction| matches!(
                    &instruction.kind, KirInstructionKind::ConstInt { value } if value == expected
                )),
            "{ty} must fold with CK modular semantics:\n{}",
            print_kir_module(module)
        );
    }
}

#[test]
fn kir_o1_sccp_should_fold_integer_comparisons() {
    for (expression, expected) in [
        ("(20 + 22) == 42", true),
        ("(20 + 22) != 42", false),
        ("(20 + 22) < 43", true),
        ("(20 + 22) <= 41", false),
        ("(20 + 22) > 42", false),
        ("(20 + 22) >= 42", true),
    ] {
        let (_, kir, contracts) = build_with_overflow(
            &format!("export fn compare() -> bool {{ return {expression}; }}"),
            KirOverflowMode::Unchecked,
        );
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
        let module = result.artifact.as_ref().expect("verified artifact");
        let instructions = &module.functions[0].blocks[0].instructions;
        assert!(
            !instructions
                .iter()
                .any(|instruction| matches!(instruction.kind, KirInstructionKind::Compare { .. })),
            "comparison must propagate in KIR:\n{}",
            print_kir_module(module)
        );
        assert!(instructions.iter().any(|instruction|
            matches!(instruction.kind, KirInstructionKind::ConstBool { value } if value == expected)));
    }
}

#[test]
fn kir_o1_sccp_should_propagate_through_integer_copy() {
    let (_, mut kir, contracts) = build_with_overflow(
        "export fn answer() -> i32 { return (20 + 22) * 2; }",
        KirOverflowMode::Unchecked,
    );
    let instructions = &mut kir.functions[0].blocks[0].instructions;
    let twenty = instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            KirInstructionKind::ConstInt { value } if value == "20" => {
                Some(instruction.results[0].value)
            }
            _ => None,
        })
        .expect("20");
    instructions
        .iter_mut()
        .find(|instruction| {
            matches!(&instruction.kind,
        KirInstructionKind::ConstInt { value } if value == "2")
        })
        .expect("2")
        .kind = KirInstructionKind::Copy { value: twenty };

    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let module = result.artifact.as_ref().expect("verified artifact");
    assert!(module.functions[0].blocks[0].instructions.iter().any(|instruction|
        matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "840")),
        "copy must propagate into multiply:\n{}", print_kir_module(module));
}

#[test]
fn kir_o1_sccp_should_propagate_a_constant_phi() {
    let (_, kir, contracts) = build_with_overflow(
        "export fn phi(flag: bool) -> i32 { let x: i32 = 0; if flag { x = 42; } else { x = 42; } return x + 1; }",
        KirOverflowMode::Unchecked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let module = result.artifact.as_ref().expect("verified artifact");
    assert!(
        module.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(&instruction.kind,
            KirInstructionKind::ConstInt { value } if value == "43")),
        "all incoming phi values are 42:\n{}",
        print_kir_module(module)
    );
}

#[test]
fn kir_o1_sccp_should_not_choose_only_one_phi_input() {
    for source in [
        "export fn phi(flag: bool) -> i32 { let x: i32 = 0; if flag { x = 42; } else { x = 41; } return x + 1; }",
        "export fn phi(n: u32) -> i32 { let x: i32 = 42; let i: u32 = 0; while i < n { x = x + 1; i = i + 1; } return x + 1; }",
    ] {
        let (_, kir, contracts) = build_with_overflow(source, KirOverflowMode::Unchecked);
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
        let module = result.artifact.as_ref().expect("verified artifact");
        let returned = module.functions[0]
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                calckernel::KirTerminator::Return { value, .. } => value,
                _ => None,
            })
            .expect("returned value");
        assert!(
            module.functions[0]
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| instruction
                    .results
                    .first()
                    .is_some_and(|result| result.value == returned)
                    && matches!(instruction.kind, KirInstructionKind::Binary { .. })),
            "different incoming values must remain dynamic:\n{}",
            print_kir_module(module)
        );
    }
}

#[test]
fn kir_o1_sccp_should_use_dominating_contract_ranges() {
    let (_, kir, contracts) = build_with_overflow(
        "export unsafe fn bounded(n: u32) -> bool contract { requires n < 8; } { return n < 8; }",
        KirOverflowMode::Unchecked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let module = result.artifact.as_ref().expect("verified artifact");
    assert!(
        module.functions[0].blocks[0]
            .instructions
            .iter()
            .any(|instruction| matches!(
                instruction.kind,
                KirInstructionKind::ConstBool { value: true }
            )),
        "entry contract must drive a real comparison rewrite:\n{}",
        print_kir_module(module)
    );
}

#[test]
fn kir_o1_sccp_should_refine_each_branch_without_exporting_its_range() {
    let (_, kir, contracts) = build_with_overflow(
        "export fn bounded(n: u32) -> bool { if n < 8 { return n < 16; } return n < 8; }",
        KirOverflowMode::Unchecked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let module = result.artifact.as_ref().expect("verified artifact");
    let instructions = module.functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .collect::<Vec<_>>();
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Compare { .. }))
            .count(),
        1,
        "only the entry comparison is unknown:\n{}",
        print_kir_module(module)
    );
    for expected in [true, false] {
        assert!(
            instructions
                .iter()
                .any(|instruction| matches!(instruction.kind,
            KirInstructionKind::ConstBool { value } if value == expected))
        );
    }
}

#[test]
fn kir_o1_sccp_should_keep_out_of_domain_literal_analysis_conservative() {
    for (ty, magnitude) in [("i32", "2147483648"), ("i64", "9223372036854775808")] {
        let (_, kir, contracts) = build(&format!(
            "export fn literal() -> {ty} {{ return -{magnitude}; }}"
        ));
        let before =
            run_kir_pass_pipeline(kir.clone(), KirOptimizationLevel::O0, contracts.as_ref());
        assert!(before.artifact.is_some(), "O0 accepts this semantic KIR");
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
        assert!(result.errors.is_empty(), "{ty}: {:?}", result.errors);
        assert_eq!(
            result.artifact, before.artifact,
            "range analysis must not reinterpret the literal or remove its checked negation"
        );
    }
}

#[test]
fn guard_path_range_should_remove_only_the_dominated_overflow_check() {
    let (_, kir, contracts) =
        build("export fn bounded(n: u32) -> u32 { if n < 8 { return n + 1; } return n + 1; }");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let module = result.artifact.as_ref().expect("verified artifact");
    assert_eq!(
        result.eliminated_guards.len(),
        1,
        "only the taken edge proves overflow safety"
    );
    assert_eq!(
        module.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
            .count(),
        1
    );
    assert!(
        validate_kir_optimization_evidence(
            module,
            result.contract_facts.as_ref(),
            &result.proofs,
            &result.eliminated_guards,
            0
        )
        .errors
        .is_empty()
    );
}

#[test]
fn guard_path_range_should_preserve_certificates_through_o2_and_o3() {
    for level in [KirOptimizationLevel::O2, KirOptimizationLevel::O3] {
        let (_, kir, contracts) = build(
            "export fn bounded(n: u32) -> u32 { if n < 8 { let dead: u32 = 99; return n + 8; } return n + 8; }",
        );
        let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
        assert!(result.errors.is_empty(), "{level:?}: {:?}", result.errors);
        assert_eq!(result.eliminated_guards.len(), 1);
        let artifact = result.artifact.as_ref().expect("verified artifact");
        assert!(!artifact.functions[0].blocks.iter().flat_map(|block| &block.instructions)
            .any(|instruction| matches!(&instruction.kind, KirInstructionKind::ConstInt { value } if value == "99")),
            "irrelevant constants must not be retained by a range proof");
    }
}

#[test]
fn guard_path_range_should_prove_nonzero_divisors_but_retain_zero_neighbor() {
    let (_, kir, contracts) =
        build("export fn divide(n: u32) -> u32 { if n > 0 { return 40 / n; } return 40 / n; }");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.eliminated_guards.len(),
        1,
        "the nonzero range must discharge exactly one division guard"
    );
}

#[test]
fn guard_contract_range_should_prove_division_and_keep_signed_overflow_neighbor() {
    for (range, removed) in [("n > 0", 2), ("n < 0", 1)] {
        let source = format!(
            "export unsafe fn divide(a: i32, n: i32) -> i32 contract {{ requires {range}; }} {{ return a / n; }}"
        );
        let (_, kir, contracts) = build(&source);
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
        assert!(result.errors.is_empty(), "{range}: {:?}", result.errors);
        assert_eq!(
            result.eliminated_guards.len(),
            removed,
            "{range}: -1 is the signed-overflow neighbor"
        );
    }
}

#[test]
fn guard_path_range_should_prove_a_slice_index_but_keep_the_boundary_neighbor() {
    let (_, kir, contracts) = build(
        "export fn get(data: ptr<i32>, n: u32) -> i32 { if n < 8 { let items: slice<i32> = slice(data, 8); return items[n]; } let items: slice<i32> = slice(data, 8); return items[n]; }",
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.eliminated_guards.len(),
        1,
        "only indices in [0, 7] fit the eight-element slice"
    );
}

#[test]
fn kir_o1_sccp_should_not_fold_away_checked_overflow_or_strict_float() {
    for source in [
        "export fn fails() -> i32 { print_i32(7); return 2147483647 + 1; }",
        "export fn strict(x: f64) -> f64 { return x + 0.0; }",
    ] {
        let (_, kir, contracts) = build(source);
        let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
        let module = result.artifact.as_ref().expect("verified artifact");
        assert!(
            module.functions[0]
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        );
        if source.contains("print_i32") {
            let ordered = module.functions[0]
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| instruction.effect.is_some())
                .collect::<Vec<_>>();
            assert!(matches!(
                ordered[0].kind,
                KirInstructionKind::RuntimeCall { .. }
            ));
            assert!(
                ordered.iter().any(|instruction| matches!(
                    instruction.kind,
                    KirInstructionKind::Guard { .. }
                ))
            );
        }
    }
}

#[test]
fn guard_elimination_should_remove_constant_safe_overflow_with_valid_proof() {
    let (_, kir, contracts) = build("export fn answer() -> i32 { return 20 + 22; }");
    let before = kir.functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
        .count();
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");
    let after = artifact.functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
        .count();

    assert_eq!((before, after), (1, 0));
    assert_eq!(result.eliminated_guards.len(), 1);
    assert!(result.eliminated_guards[0].proof.is_some());
    assert!(result.explanations[0].reason.contains("locally verified"));
}

#[test]
fn guard_elimination_should_retain_unknown_neighbor_with_deterministic_reason() {
    let (_, kir, contracts) = build("export fn add(a: i32) -> i32 { return a + 1; }");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");

    assert!(
        artifact.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
    );
    assert!(result.explanations.iter().any(|explanation| {
        !explanation.removed && explanation.reason == "retained: scalar safety is unknown"
    }));
}

#[test]
fn guard_elimination_should_use_dominating_contract_range_for_slice_bounds() {
    let (_, kir, contracts) = build(
        r#"
        export unsafe fn get(items: slice<i32>, n: u32) -> i32
        contract { requires n < items.len; effects read(items); }
        { return items[n]; }
        "#,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(
        !artifact.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
    );
    assert!(result.eliminated_guards[0].used_trusted_contract);
}

#[test]
fn guard_invalid_certificate_mutation_should_fail_verification_and_commit_no_artifact() {
    let (_, kir, contracts) = build("export fn answer() -> i32 { return 20 + 22; }");
    let mut result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let proof = result.eliminated_guards[0].proof.expect("proof");
    let certificate = result.proofs.get_mut(proof).expect("certificate");
    let ProofStep::GuardSafety {
        condition_instruction,
        ..
    } = certificate.steps.last_mut().expect("root step")
    else {
        panic!("guard safety root");
    };
    *condition_instruction = InstructionId::from_index(u32::MAX);
    let evidence = validate_kir_optimization_evidence(
        result.artifact.as_ref().expect("pre-mutation artifact"),
        contracts.as_ref(),
        &result.proofs,
        &result.eliminated_guards,
        0,
    );

    assert!(!evidence.errors.is_empty());
    result.artifact = None;
    assert!(result.artifact.is_none());
}

#[test]
fn runtime_print_o1_pipeline_should_never_delete_or_reorder_observable_effect() {
    let (_, kir, contracts) = build("fn main() -> void { print_i32(20 + 22); }");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");
    let instructions = artifact.functions[0].blocks[0]
        .instructions
        .iter()
        .map(|instruction| &instruction.kind)
        .collect::<Vec<_>>();

    assert!(
        instructions
            .iter()
            .any(|kind| matches!(kind, KirInstructionKind::RuntimeCall { .. }))
    );
    assert_eq!(
        artifact.functions[0].blocks[0]
            .instructions
            .iter()
            .filter_map(|instruction| instruction.effect.as_ref())
            .map(|effect| effect.order)
            .collect::<Vec<_>>(),
        vec![0]
    );
}
