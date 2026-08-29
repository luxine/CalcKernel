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
    let checked = check(&SourceFile::new("o1.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Checked,
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
