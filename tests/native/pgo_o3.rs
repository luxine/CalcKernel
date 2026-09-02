use std::collections::{BTreeMap, BTreeSet};

use calckernel::{
    CkImmutableProfileAnalysis, CkProfileAnalysis, CkProfileAnalyzedSite, CkProfileContract,
    CkProfileEvent, CkProfileFunctionWork, CkProfileKirMode, CkProfileObservation,
    CkProfileSiteKind, EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer,
    KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, NativeContext,
    NativeOptimizationLevel, NativeTarget, SourceFile, build_kir_module, check,
    import_contract_facts, lower_native_kir_module, lower_to_mir, prepare_ck_profile_kir,
    run_kir_pass_pipeline, run_profile_guided_kir_pass_pipeline,
};

const SOURCE: &str =
    "export fn choose(value: i32) -> i32 { if value == 7 { return 10; } return 20; }";

const LOOP_SOURCE: &str = "export fn accumulate(n: u32, selector: u32) -> u32 { let i: u32 = 0; let total: u32 = 0; while i < n { if selector == 3 { total = total + i; } else { total = total + 1; } i = i + 1; } return total; }";

const HOT_COLD_SOURCE: &str = r#"
fn hot(x: i32) -> i32 {
  let a01: i32 = x + 1; let a02: i32 = a01 + 1;
  let a03: i32 = a02 + 1; let a04: i32 = a03 + 1;
  let a05: i32 = a04 + 1; let a06: i32 = a05 + 1;
  let a07: i32 = a06 + 1; let a08: i32 = a07 + 1;
  let a09: i32 = a08 + 1; let a10: i32 = a09 + 1;
  let a11: i32 = a10 + 1; let a12: i32 = a11 + 1;
  let a13: i32 = a12 + 1; let a14: i32 = a13 + 1;
  let a15: i32 = a14 + 1; let a16: i32 = a15 + 1;
  let a17: i32 = a16 + 1;
  return a17;
}
fn cold(x: i32) -> i32 { return x - 1; }
export fn kernel(x: i32, flag: i32) -> i32 {
  if flag == 7 { return hot(x); }
  return cold(x);
}
"#;

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

fn profiled_loop() -> calckernel::KirPassManagerResult {
    let checked = check(&SourceFile::new("pgo-o3-loop.ck", LOOP_SOURCE));
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
        identity_digest: [0x77; 32],
        sites: plan
            .sites
            .iter()
            .cloned()
            .map(|descriptor| {
                let observation = match &descriptor.kind {
                    CkProfileSiteKind::FunctionEntry | CkProfileSiteKind::Edge { .. } => {
                        CkProfileObservation::Scalar(256)
                    }
                    CkProfileSiteKind::LoopTripHistogram { .. } => {
                        let mut buckets = [0; 16];
                        buckets[10] = 256;
                        CkProfileObservation::Histogram(buckets)
                    }
                    CkProfileSiteKind::SliceLengthHistogram { .. } => {
                        CkProfileObservation::Histogram([0; 16])
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
            dynamic_work: Some(100_000),
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
    assert_eq!(result.pgo.as_ref().expect("PGO plan").loop_hints.len(), 1);
    result
}

fn profiled_hot_cold() -> calckernel::KirPassManagerResult {
    let checked = check(&SourceFile::new("pgo-o3-hot-cold.ck", HOT_COLD_SOURCE));
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
    let names = plan
        .annotations
        .iter()
        .filter_map(|annotation| {
            let CkProfileEvent::FunctionEntry { function, .. } = annotation.event else {
                return None;
            };
            let name = plan
                .module
                .functions
                .iter()
                .find(|candidate| candidate.id == function)?
                .name
                .clone();
            Some((annotation.descriptor.function_digest, name))
        })
        .collect::<BTreeMap<_, _>>();
    let entry_count = |digest: &[u8; 32]| match names.get(digest).map(String::as_str) {
        Some("cold") => 0,
        Some("hot" | "kernel") => 256,
        other => panic!("unexpected function name: {other:?}"),
    };
    let sites = plan
        .sites
        .iter()
        .cloned()
        .map(|descriptor| {
            let observation = match &descriptor.kind {
                CkProfileSiteKind::FunctionEntry => {
                    CkProfileObservation::Scalar(entry_count(&descriptor.function_digest))
                }
                CkProfileSiteKind::Edge { .. } => CkProfileObservation::Scalar(256),
                CkProfileSiteKind::LoopTripHistogram { .. }
                | CkProfileSiteKind::SliceLengthHistogram { .. } => {
                    CkProfileObservation::Histogram([0; 16])
                }
                CkProfileSiteKind::CandidateConstant { candidates, .. } => {
                    CkProfileObservation::CandidateConstant {
                        candidates: vec![256; candidates.len()],
                        other: 0,
                    }
                }
            };
            CkProfileAnalyzedSite {
                descriptor,
                observation,
            }
        })
        .collect();
    let functions = plan
        .sites
        .iter()
        .filter(|site| matches!(site.kind, CkProfileSiteKind::FunctionEntry))
        .map(|site| site.function_digest)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(index, function_digest)| {
            let entries = entry_count(&function_digest);
            CkProfileFunctionWork {
                function_digest,
                dynamic_work: Some(u128::from(entries).saturating_mul(100)),
                rank: Some(u32::try_from(index + 1).expect("rank")),
                hot_root: names[&function_digest] == "kernel",
            }
        })
        .collect();
    let analysis = CkImmutableProfileAnalysis::new(CkProfileAnalysis {
        identity_digest: [0x88; 32],
        sites,
        functions,
    });
    let result = run_profile_guided_kir_pass_pipeline(
        &plan,
        &analysis,
        &CkProfileContract::schema1(),
        prepared.contract_facts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
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

#[test]
fn pgo_o3_licm_should_preserve_exact_profile_branch_mapping() {
    let result = profiled_loop();
    assert_eq!(result.pgo.as_ref().expect("PGO plan").branches.len(), 1);
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let ir = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower checked loop PGO module")
        .verify()
        .expect("verify loop PGO module")
        .to_ir_string()
        .expect("PGO loop LLVM IR");
    assert!(ir.contains("branch_weights"), "{ir}");
}

#[test]
fn pgo_o3_should_mark_a_callee_reached_only_from_a_profile_cold_block() {
    let result = profiled_hot_cold();
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let verified = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower checked hot/cold PGO module")
        .verify()
        .expect("verify hot/cold PGO module");
    let ir = verified.to_ir_string().expect("hot/cold PGO LLVM IR");
    let cold_definition = ir
        .lines()
        .find(|line| line.starts_with("define") && line.contains("@cold("))
        .expect("cold fallback definition");
    let attribute_group = cold_definition
        .split_ascii_whitespace()
        .find(|token| token.starts_with('#'))
        .expect("cold fallback attribute group");
    let attributes = ir
        .lines()
        .find(|line| line.starts_with(&format!("attributes {attribute_group} =")))
        .expect("cold fallback attributes");
    assert!(
        attributes
            .split_ascii_whitespace()
            .any(|token| token == "cold"),
        "{cold_definition}\n{attributes}\n{ir}"
    );
    let optimized_ir = verified
        .audit()
        .expect("hot/cold fact audit")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("hot/cold O3 optimization")
        .to_ir_string()
        .expect("optimized hot/cold LLVM IR");
    assert!(
        optimized_ir
            .lines()
            .any(|line| line.contains(" call ") && line.contains("@cold(")),
        "{optimized_ir}"
    );
}
