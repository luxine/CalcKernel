use calckernel::{
    CkImmutableProfileAnalysis, CkProfileAnalysis, CkProfileAnalyzedSite, CkProfileContract,
    CkProfileFunctionWork, CkProfileKirMode, CkProfileObservation, CkProfileSiteKind,
    EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, NativeContext, NativeOptimizationLevel, NativeTarget,
    SourceFile, build_kir_module, check, import_contract_facts, lower_native_kir_module,
    lower_to_mir, prepare_ck_profile_kir, run_kir_pass_pipeline,
    run_profile_guided_kir_pass_pipeline,
};

const SOURCE: &str =
    "export fn choose(value: i32) -> i32 { if value == 7 { return 10; } return 20; }";

fn profiled() -> calckernel::KirPassManagerResult {
    let checked = check(&SourceFile::new("pgo-o3.ck", SOURCE));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0).expect("contracts");
    let prepared = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, Some(&contracts));
    assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
    let plan = prepare_ck_profile_kir(
        prepared.artifact.as_ref().expect("O1 artifact"),
        CkProfileKirMode::Use,
    )
    .expect("use plan");
    let function_digest = plan
        .sites
        .iter()
        .find(|site| matches!(site.kind, CkProfileSiteKind::FunctionEntry))
        .expect("entry site")
        .function_digest;
    let analysis = CkImmutableProfileAnalysis::new(CkProfileAnalysis {
        identity_digest: [0x66; 32],
        sites: plan
            .sites
            .iter()
            .cloned()
            .map(|descriptor| {
                let observation = match &descriptor.kind {
                    CkProfileSiteKind::FunctionEntry | CkProfileSiteKind::Edge { .. } => {
                        CkProfileObservation::Scalar(256)
                    }
                    CkProfileSiteKind::LoopTripHistogram { .. }
                    | CkProfileSiteKind::SliceLengthHistogram { .. } => {
                        let mut buckets = [0; 16];
                        buckets[7] = 256;
                        CkProfileObservation::Histogram(buckets)
                    }
                    CkProfileSiteKind::CandidateConstant { candidates, .. } => {
                        CkProfileObservation::CandidateConstant {
                            candidates: vec![240; candidates.len()],
                            other: 16,
                        }
                    }
                };
                CkProfileAnalyzedSite {
                    descriptor,
                    observation,
                }
            })
            .collect(),
        functions: vec![CkProfileFunctionWork {
            function_digest,
            dynamic_work: Some(10_000),
            rank: Some(1),
            hot_root: true,
        }],
    });
    let result = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        prepared.contract_facts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.pgo.as_ref().expect("PGO plan").branches.len(), 1);
    result
}

#[test]
fn pgo_o3_profiled_optimizer_should_attach_checked_entry_and_branch_metadata() {
    let result = profiled();
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let verified = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower checked PGO module")
        .verify()
        .expect("verify PGO module");
    let ir = verified.to_ir_string().expect("PGO LLVM IR");
    assert!(ir.contains("function_entry_count"), "{ir}");
    assert!(ir.contains("branch_weights"), "{ir}");
    assert!(ir.contains("hot"), "{ir}");
    let optimized = verified
        .audit()
        .expect("fact audit")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("O3 optimize");
    assert!(
        !target
            .emit_object(optimized)
            .expect("PGO object")
            .is_empty()
    );
}

#[test]
fn pgo_metadata_should_withhold_module_after_profile_plan_mutation() {
    let mut result = profiled();
    result.pgo.as_mut().expect("PGO plan").branches[0].then_count += 1;
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let error = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect_err("mutated PGO plan must withhold LLVM module");
    assert!(error.message.contains("audit digest mismatch"), "{error:?}");
}
