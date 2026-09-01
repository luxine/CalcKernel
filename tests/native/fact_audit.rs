use calckernel::{
    EmitLlvmOptions, KirArithmeticSemantics, KirBoundsMode, KirBuildConfig, KirConsumer,
    KirInstructionKind, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, KirTargetProfile,
    LLVM_BRIDGE_ABI_VERSION, NativeContext, NativeCpu, NativeFactAuditReport, NativeModule,
    NativeOptimizationLevel, NativeStage, NativeStrengtheningKind, NativeTarget, ProofId,
    SourceFile, build_kir_module, check, import_contract_facts, lower_native_kir_module,
    lower_to_mir, native_fact_audit_test_inject_untracked,
    native_fact_audit_test_inject_untracked_flag, run_kir_pass_pipeline,
};

use super::vector_llvm::vector_module;

fn audited_kir(source: &str) -> (String, NativeFactAuditReport) {
    let checked = check(&SourceFile::new("fact-audit.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host().expect("target");
    let audited = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower KIR")
        .verify()
        .expect("verify")
        .audit()
        .expect("fact audit");
    (
        audited.to_ir_string().expect("LLVM IR"),
        audited.audit_report().clone(),
    )
}

#[test]
fn fact_audit_typestate_should_run_between_verify_and_optimize() {
    assert_eq!(LLVM_BRIDGE_ABI_VERSION, 3);
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host().expect("target");
    let verified = NativeModule::empty(&context)
        .expect("empty module")
        .verify()
        .expect("verify");
    let audited = verified.audit().expect("empty fact audit");
    assert_eq!(audited.audit_report().property_count, 0);
    audited
        .optimize(&target, NativeOptimizationLevel::O0)
        .expect("optimize only after audit");
}

#[test]
fn fact_audit_should_reject_injected_untracked_strengthening_before_optimization() {
    let context = NativeContext::new().expect("context");
    let verified = NativeModule::empty(&context)
        .expect("empty module")
        .verify()
        .expect("verify");
    native_fact_audit_test_inject_untracked(&verified).expect("inject mutation");
    let error = verified.audit().expect_err("untracked property must fail");
    assert_eq!(error.stage, NativeStage::Module);
    assert!(error.message.contains("untracked CK-owned strengthening"));
}

#[test]
fn fact_audit_should_reject_an_untracked_llvm_flag_without_a_test_side_channel() {
    let context = NativeContext::new().expect("context");
    let verified = NativeModule::empty(&context)
        .expect("empty module")
        .verify()
        .expect("verify");
    native_fact_audit_test_inject_untracked_flag(&verified).expect("inject flag mutation");
    let error = verified.audit().expect_err("untracked flag must fail");
    assert_eq!(error.stage, NativeStage::Module);
    assert!(error.message.contains("untracked CK-owned strengthening"));
}

#[test]
fn fact_audit_should_map_complete_contract_facts_to_parameter_attributes() {
    let (text, report) = audited_kir(
        r#"
        export unsafe fn attrs(a: slice<i32>, b: slice<i32>) -> void
        contract {
          requires noalias(a, b);
          requires aligned(a.data, 32);
          requires a.len > 0;
          requires b.len > 0;
          effects read(a), write(b);
        }
        { let value: i32 = a[0]; b[0] = value; }
        "#,
    );

    assert!(
        text.contains("ptr noalias readonly align 32 %a.data"),
        "{text}"
    );
    assert!(text.contains("ptr noalias writeonly %b.data"), "{text}");
    assert!(text.contains("!alias.scope"), "{text}");
    assert!(text.contains("!noalias"), "{text}");
    for kind in [
        NativeStrengtheningKind::ParameterNoAlias,
        NativeStrengtheningKind::Alignment,
        NativeStrengtheningKind::ReadOnly,
        NativeStrengtheningKind::WriteOnly,
        NativeStrengtheningKind::AliasScope,
    ] {
        assert!(
            report
                .properties
                .iter()
                .any(|property| property.kind == kind),
            "missing {kind:?}: {report:?}"
        );
    }
    assert_eq!(report.property_count, report.fact_sources);
}

#[test]
fn fact_audit_should_not_promote_partial_pairwise_noalias_to_parameter_noalias() {
    let (text, report) = audited_kir(
        r#"
        export unsafe fn attrs(a: slice<i32>, b: slice<i32>, c: slice<i32>) -> void
        contract { requires noalias(a, b); requires a.len > 0; effects read(a); }
        { let value: i32 = a[0]; }
        "#,
    );

    assert!(!text.contains(" noalias "), "{text}");
    assert!(text.contains("!alias.scope"), "{text}");
    assert!(text.contains("!noalias"), "{text}");
    assert!(
        !report
            .properties
            .iter()
            .any(|property| { property.kind == NativeStrengtheningKind::ParameterNoAlias })
    );
}

#[test]
fn fact_audit_should_not_promote_noalias_across_pointer_return_or_call_capture() {
    let (status_return_text, status_return_report) = audited_kir(
        r#"
        export unsafe fn read(a: slice<i32>, b: slice<i32>) -> i32
        contract { requires noalias(a, b); requires a.len > 0; effects read(a); }
        { return a[0]; }
        "#,
    );
    let read_signature = status_return_text
        .lines()
        .find(|line| line.contains("@__ck_impl_read("))
        .expect("read signature");
    assert!(!read_signature.contains("noalias"), "{read_signature}");
    assert!(!status_return_report.properties.iter().any(|property| {
        property.function == "read" && property.kind == NativeStrengtheningKind::ParameterNoAlias
    }));

    let (return_text, return_report) = audited_kir(
        r#"
        unsafe fn choose(a: slice<i32>, b: slice<i32>) -> slice<i32>
        contract { requires noalias(a, b); effects none; }
        { return a; }
        export unsafe fn wrapper(a: slice<i32>, b: slice<i32>) -> void
        contract { requires noalias(a, b); effects none; }
        { unsafe { let chosen: slice<i32> = choose(a, b); } }
        "#,
    );
    let choose_signature = return_text
        .lines()
        .find(|line| line.contains("@choose("))
        .expect("choose signature");
    assert!(!choose_signature.contains("noalias"), "{choose_signature}");
    assert!(!return_report.properties.iter().any(|property| {
        property.function == "choose" && property.kind == NativeStrengtheningKind::ParameterNoAlias
    }));

    let (capture_text, capture_report) = audited_kir(
        r#"
        export fn sink(value: slice<i32>) -> void { value[0] = 1; }
        export unsafe fn capture(a: slice<i32>, b: slice<i32>) -> void
        contract { requires noalias(a, b); requires a.len > 0; effects write(a); }
        { sink(a); }
        "#,
    );
    let capture_signature = capture_text
        .lines()
        .find(|line| line.contains("@__ck_impl_capture("))
        .expect("capture signature");
    assert!(
        !capture_signature.contains("noalias"),
        "{capture_signature}"
    );
    assert!(!capture_report.properties.iter().any(|property| {
        property.function == "capture" && property.kind == NativeStrengtheningKind::ParameterNoAlias
    }));
}

#[test]
fn fact_audit_should_map_removed_overflow_guard_proof_to_nsw() {
    let (text, report) = audited_kir(
        r#"
        export unsafe fn add(n: i32) -> i32
        contract { requires n >= 0; requires n < 100; effects none; }
        { return n + 1; }
        export unsafe fn uadd(n: u32) -> u32
        contract { requires n < 100; effects none; }
        { return n + 1; }
        export unsafe fn pure_marker(n: i32) -> void
        contract { requires n >= 0; effects none; }
        {}
        "#,
    );

    assert!(text.contains("add nsw i32"), "{text}");
    assert!(text.contains("add nuw i32"), "{text}");
    assert!(text.contains("call void @llvm.assume"), "{text}");
    assert!(text.contains("memory(none)"), "{text}");
    assert!(
        report
            .properties
            .iter()
            .any(|property| { property.kind == NativeStrengtheningKind::NoSignedWrap }),
        "{report:?}"
    );
    assert!(
        report
            .properties
            .iter()
            .any(|property| { property.kind == NativeStrengtheningKind::NoUnsignedWrap })
    );
    assert!(
        report
            .properties
            .iter()
            .any(|property| { property.kind == NativeStrengtheningKind::MemoryEffects })
    );
    assert!(
        report
            .properties
            .iter()
            .any(|property| { property.kind == NativeStrengtheningKind::Range })
    );
    assert!(
        report
            .properties
            .iter()
            .any(|property| { property.kind == NativeStrengtheningKind::Assume })
    );
    assert_eq!(report.proof_sources, 2);
}

#[test]
fn fact_audit_vector_alignment_mutation_should_fail_before_llvm_optimization() {
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let mut result = run_kir_pass_pipeline(vector_module(&target), KirOptimizationLevel::O0, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let artifact = result.artifact.as_mut().expect("verified vector artifact");
    let load = artifact.functions[0].blocks[0]
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            KirInstructionKind::VectorLoad { access, .. } => Some(access),
            _ => None,
        })
        .expect("vector load");
    load.required_alignment = 8;

    let error = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect_err("post-verification alignment strengthening must fail");
    assert!(
        error.message.contains("changed after verification"),
        "{error:?}"
    );
}

#[test]
fn fact_audit_vector_missing_no_failure_proof_should_withhold_the_artifact() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let mut module = vector_module(&target);
    let KirInstructionKind::VectorBinary {
        semantics,
        no_failure_proof,
        ..
    } = &mut module.functions[0].blocks[0].instructions[2].kind
    else {
        unreachable!()
    };
    *semantics = KirArithmeticSemantics::Checked;
    *no_failure_proof = Some(ProofId::from_index(999));

    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O0, None);
    assert!(result.artifact.is_none());
    assert!(
        result
            .errors
            .iter()
            .any(|message| message.contains("missing vector no-failure proof")),
        "{:?}",
        result.errors
    );
}

#[test]
fn fact_audit_stale_vector_profile_should_fail_before_llvm_optimization() {
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let mut result = run_kir_pass_pipeline(vector_module(&target), KirOptimizationLevel::O0, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let native_target = NativeTarget::host_with_cpu(NativeCpu::Native).expect("native target");
    result
        .artifact
        .as_mut()
        .expect("verified vector artifact")
        .profile = native_target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("native target profile");

    let error = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect_err("stale exact target profile must fail");
    assert!(error.message.contains("does not match"), "{error:?}");
}

#[test]
fn fact_audit_unprofiled_vector_operation_should_fail_before_llvm_optimization() {
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let mut result = run_kir_pass_pipeline(vector_module(&target), KirOptimizationLevel::O0, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result
        .artifact
        .as_mut()
        .expect("verified vector artifact")
        .profile = KirTargetProfile::for_consumer(KirConsumer::NativeLibrary);

    let error = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect_err("vector operation without an exact profile must fail");
    assert!(
        error.message.contains("changed after verification"),
        "{error:?}"
    );
}
