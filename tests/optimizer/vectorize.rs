use std::collections::BTreeMap;

use calckernel::{
    CandidateDisposition, ContractFactSet, KirAlignmentClass, KirBoundsMode, KirBuildConfig,
    KirConsumer, KirCostKey, KirLegalCost, KirNativeCpuPolicy, KirOperationAvailability,
    KirOptimizationLevel, KirOverflowMode, KirProfileOperation, KirSanitizerMode, KirTargetProfile,
    KirTargetProfileBuilder, KirVerifiedProgramState, SourceFile, VectorEpilogue,
    build_kir_module_with_profile, check, check_vectorization_trial_independently,
    discover_tuning_vectorization_candidates, discover_vectorization_candidates,
    import_contract_facts, lower_to_mir, prepare_vectorization_trial, print_kir_module,
    run_kir_pass_pipeline,
};

#[test]
fn independent_checker_should_not_call_vector_proposer_or_dependence_analysis() {
    let checker = include_str!("../../src/optimizer/vectorize_check.rs");
    for forbidden in [
        "discover_vectorization_candidates",
        "analyze_loop_dependences",
        "analyze_loop_legality",
        "candidate_cost_and_threshold",
        "vectorization_charge(plan)",
    ] {
        assert!(
            !checker.contains(forbidden),
            "independent vector checker must not call `{forbidden}`"
        );
    }
}

fn native_profile() -> KirTargetProfile {
    native_profile_for(KirConsumer::NativeLibrary)
}

fn native_profile_for(consumer: KirConsumer) -> KirTargetProfile {
    native_profile_with_interleave(consumer, 1)
}

fn native_profile_with_interleave(
    consumer: KirConsumer,
    maximum_interleave_factor: u8,
) -> KirTargetProfile {
    let mut builder = KirTargetProfileBuilder::native(
        consumer,
        "aarch64-apple-darwin",
        64,
        true,
        KirNativeCpuPolicy::Baseline,
        "generic",
        vec!["+neon".to_string()],
    )
    .expect("native profile builder");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            (((key.lanes == 2 || key.lanes == 4) && key.lane == calckernel::KirLaneType::U32)
                || (key.lanes == 2 && key.lane == calckernel::KirLaneType::F64))
                && matches!(
                    key.operation,
                    KirProfileOperation::Splat
                        | KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
                        | KirProfileOperation::Divide
                        | KirProfileOperation::Negate
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                        | KirProfileOperation::Compare
                        | KirProfileOperation::Select
                        | KirProfileOperation::Cast
                        | KirProfileOperation::Insert
                        | KirProfileOperation::Extract
                        | KirProfileOperation::RuntimePredicate
                        | KirProfileOperation::ReduceAdd
                        | KirProfileOperation::ReduceMultiply
                )
                && (!matches!(
                    key.operation,
                    KirProfileOperation::Load | KirProfileOperation::Store
                ) || key.alignment
                    == KirAlignmentClass::Bytes(if key.lane == calckernel::KirLaneType::F64 {
                        8
                    } else {
                        4
                    }))
        })
    {
        let legalized_type = match (key.lane, key.lanes) {
            (calckernel::KirLaneType::F64, 2) => "v2f64",
            (calckernel::KirLaneType::U32, 2) => "v2i32",
            _ => "v4i32",
        };
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type: legalized_type.to_string(),
                },
            )
            .expect("legal vector operation");
    }
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lanes == 1
                && matches!(
                    (key.lane, key.operation),
                    (calckernel::KirLaneType::U32, KirProfileOperation::Add)
                        | (calckernel::KirLaneType::U32, KirProfileOperation::Multiply)
                        | (calckernel::KirLaneType::F64, KirProfileOperation::Multiply)
                        | (calckernel::KirLaneType::F64, KirProfileOperation::Divide)
                        | (calckernel::KirLaneType::F64, KirProfileOperation::Negate)
                )
        })
    {
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 10,
                    legalization_parts: 1,
                    legalized_type: "i32".to_string(),
                },
            )
            .expect("scalar comparison cost");
    }
    builder.set_maximum_interleave_factor(maximum_interleave_factor);
    builder.build().expect("native vector profile")
}

fn map_state(source: &str) -> (KirVerifiedProgramState, Option<ContractFactSet>) {
    map_state_with_profile(source, native_profile())
}

fn map_state_with_profile(
    source: &str,
    profile: KirTargetProfile,
) -> (KirVerifiedProgramState, Option<ContractFactSet>) {
    let checked = check(&SourceFile::new("vectorize.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let state = KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("O2 artifact"),
        optimized.contract_facts.clone(),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified O2 state");
    (state, optimized.contract_facts)
}

const MAP: &str = r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 7; i = i + 1; }
}
"#;

const INTERLEAVE_MAP: &str = r#"
export fn preserved_anchor(x: u32) -> u32 { return x + 1; }

export unsafe fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 7; i = i + 1; }
}
"#;

const STRICT_F64_MAP: &str = r#"
export unsafe fn map(a: slice<f64>, b: slice<f64>, n: u32, factor: f64) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] * factor; i = i + 1; }
}
"#;

const STRICT_F64_UNARY_DIVIDE_MAP: &str = r#"
export unsafe fn map(a: slice<f64>, b: slice<f64>, n: u32, divisor: f64) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = -a[i] / divisor; i = i + 1; }
}
"#;

const CAST_MAP: &str = r#"
export unsafe fn map(a: slice<u32>, b: slice<f64>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = u32_to_f64(a[i]); i = i + 1; }
}
"#;

const PURE_DIAMOND_MAP: &str = r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>, n: u32, pivot: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n {
    let x: u32 = a[i];
    let selected: u32 = 0;
    if x < pivot { selected = x + 1; } else { selected = x - 1; }
    b[i] = selected;
    i = i + 1;
  }
}
"#;

const PREDICATED_UPDATE_MAP: &str = r#"
export unsafe fn relax(distance: slice<f64>, candidate_values: slice<f64>, n: u32) -> void
contract { requires noalias(distance, candidate_values); effects read(candidate_values), readwrite(distance); }
{
  let i: u32 = 0;
  while i < n {
    let candidate: f64 = candidate_values[i];
    let old: f64 = distance[i];
    if candidate < old { distance[i] = candidate; }
    i = i + 1;
  }
}
"#;

#[test]
fn predicated_update_discovery_should_accept_same_place_update() {
    let (pre, _) = map_state(PREDICATED_UPDATE_MAP);
    let discovery = discover_tuning_vectorization_candidates(&pre);
    assert!(
        !discovery.candidates.is_empty(),
        "{discovery:#?}\n{}",
        print_kir_module(pre.module())
    );
    let candidate = &discovery.candidates[0];
    let update = candidate
        .predicated_update
        .as_ref()
        .expect("predicated update metadata");
    assert!(candidate.diamond.is_none());
    assert!(candidate.reduction.is_none());
    assert!(update.store_when_true);
    assert_ne!(update.memory_input, update.memory_output);
    assert!(candidate.accesses.iter().any(|access| {
        access.instruction == update.old_load_instruction
            && access.kind == calckernel::LoopMemoryAccessKind::Read
    }));
    assert!(candidate.accesses.iter().any(|access| {
        access.instruction == update.store_instruction
            && access.kind == calckernel::LoopMemoryAccessKind::Write
    }));
    let keys = discovery
        .candidates
        .iter()
        .map(|candidate| candidate.key.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys.len(), discovery.candidates.len());
}

#[test]
fn predicated_update_discovery_should_fail_closed_on_false_shapes() {
    let cases = [
        (
            "non-strict-compare",
            "if candidate <= old { distance[i] = candidate; }",
        ),
        ("stores-old", "if candidate < old { distance[i] = old; }"),
        (
            "different-index",
            "if candidate < old { distance[i + 1] = candidate; }",
        ),
        (
            "both-arms-store",
            "if candidate < old { distance[i] = candidate; } else { distance[i] = old; }",
        ),
        (
            "second-store",
            "if candidate < old { distance[i + 1] = candidate; distance[i] = candidate; }",
        ),
    ];
    for (name, conditional) in cases {
        let source = format!(
            r#"
export unsafe fn relax(distance: slice<f64>, candidate_values: slice<f64>, n: u32) -> void
contract {{ requires noalias(distance, candidate_values); effects read(candidate_values), readwrite(distance); }}
{{
  let i: u32 = 0;
  while i < n {{
    let candidate: f64 = candidate_values[i];
    let old: f64 = distance[i];
    {conditional}
    i = i + 1;
  }}
}}
"#
        );
        let (pre, _) = map_state(&source);
        let discovery = discover_tuning_vectorization_candidates(&pre);
        assert!(
            discovery
                .candidates
                .iter()
                .all(|candidate| candidate.predicated_update.is_none()),
            "false shape `{name}` produced a predicated candidate: {discovery:#?}"
        );
        assert!(
            !discovery.fallbacks.is_empty(),
            "false shape `{name}` did not retain a stable fallback"
        );
    }
}

const MODULAR_REDUCTIONS: &str = r#"
export fn sum(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}

export fn product(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 1;
  while i < n { total = total * a[i]; i = i + 1; }
  return total;
}
"#;

#[test]
fn loop_simd_should_enumerate_and_materialize_target_bounded_interleave_factors() {
    let (pre, contracts) = map_state_with_profile(
        INTERLEAVE_MAP,
        native_profile_with_interleave(KirConsumer::NativeLibrary, 4),
    );
    let discovery = discover_vectorization_candidates(&pre);
    let identities = discovery
        .candidates
        .iter()
        .map(|candidate| (candidate.vf, candidate.uf))
        .collect::<Vec<_>>();
    assert_eq!(
        identities,
        vec![(2, 1), (2, 2), (2, 4), (4, 1), (4, 2), (4, 4)],
        "{discovery:#?}"
    );

    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4 && candidate.uf == 2)
        .expect("VF4/UF2 candidate");
    let prepared = prepare_vectorization_trial(&pre, candidate).expect("interleaved vector trial");
    assert_eq!(prepared.plan.uf, 2);
    assert_eq!(
        prepared.plan.operations.len(),
        candidate.operations.len() * usize::from(candidate.uf)
    );
    assert_eq!(
        prepared.plan.memory_groups.len(),
        candidate.accesses.len() * usize::from(candidate.uf)
    );
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "growth={:#?}",
        prepared.plan.growth
    );

    let mut forged_plan = prepared.plan.clone();
    forged_plan
        .operations
        .iter_mut()
        .find(|mapping| mapping.unroll_index == 1)
        .expect("second UF operation")
        .unroll_index = 0;
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &forged_plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut forged_trial = prepared.trial.clone();
    let offset = forged_trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("interleaved function")
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_body")
        .expect("interleaved body")
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::ConstInt { value }
                if value == &candidate.vf.to_string() =>
            {
                Some(value)
            }
            _ => None,
        })
        .expect("second UF offset");
    *offset = (u32::from(candidate.vf) + 1).to_string();
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &forged_trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a forged UF offset"
    );

    let mut forged_trial = prepared.trial.clone();
    let preheader = forged_trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("interleaved function")
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("interleaved preheader");
    let vector_limit_stride = preheader
        .instructions
        .iter()
        .find_map(|instruction| match instruction.kind {
            calckernel::KirInstructionKind::Binary {
                op: calckernel::MirBinaryOp::Sub,
                right,
                semantics: calckernel::KirArithmeticSemantics::Modular,
                ..
            } => Some(right),
            _ => None,
        })
        .expect("vector limit stride");
    let stride = preheader
        .instructions
        .iter_mut()
        .find_map(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == vector_limit_stride)
                .then_some(&mut instruction.kind)
        })
        .and_then(|kind| match kind {
            calckernel::KirInstructionKind::ConstInt { value } => Some(value),
            _ => None,
        })
        .expect("vector limit stride constant");
    *stride = candidate.vf.to_string();
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &forged_trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a vector limit smaller than VF*UF"
    );

    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let vector_attempts = result
        .audit
        .attempts()
        .iter()
        .filter(|attempt| {
            matches!(
                attempt.key,
                calckernel::CandidateKey::LoopFrontier {
                    kind: calckernel::LoopCandidateKind::LoopSimd,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(vector_attempts.len(), 6, "{vector_attempts:#?}");
    assert_eq!(
        vector_attempts
            .iter()
            .filter(|attempt| attempt.disposition == CandidateDisposition::Accepted)
            .count(),
        1,
        "{vector_attempts:#?}"
    );
    assert!(
        vector_attempts.iter().any(|attempt| {
            attempt.disposition == CandidateDisposition::Accepted
                && matches!(
                    attempt.key,
                    calckernel::CandidateKey::LoopFrontier { vf: 4, uf: 2, .. }
                )
        }),
        "frontier did not compare runtime-trip candidates at one common scope: {vector_attempts:#?}"
    );
    assert!(
        vector_attempts
            .iter()
            .any(|attempt| attempt.disposition == CandidateDisposition::NonWinner),
        "{vector_attempts:#?}"
    );
}

#[test]
fn loop_simd_runtime_map_should_materialize_vector_body_scalar_fallback_and_epilogue() {
    let (pre, _) = map_state(MAP);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(discovery.candidates.len(), 2, "{discovery:?}");
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4)
        .expect("VF4 candidate");
    assert_eq!(candidate.vf, 4);
    let prepared = prepare_vectorization_trial(&pre, candidate).expect("vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    assert!(matches!(
        prepared.plan.epilogue,
        VectorEpilogue::Scalar { .. }
    ));
    let text = print_kir_module(prepared.trial.module());
    assert!(text.contains("vector_load"), "{text}");
    assert!(text.contains("vector_store"), "{text}");
    assert!(text.contains("vector_add.modular"), "{text}");
    assert!(text.contains("loop_simd_body"), "{text}");
    assert!(text.contains("branch"), "{text}");
}

#[test]
fn loop_simd_short_exact_trip_should_be_rejected_before_materialization() {
    let source = r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < 3 { b[i] = a[i] + 7; i = i + 1; }
}
"#;
    let (pre, _) = map_state(source);
    let discovery = discover_vectorization_candidates(&pre);
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(
        discovery
            .fallbacks
            .iter()
            .any(|fallback| { fallback.reason == "vector-profitability-threshold-not-met" })
    );
}

#[test]
fn loop_simd_strict_f64_elementwise_should_preserve_lane_rounding_without_fast_math() {
    let (pre, _) = map_state(STRICT_F64_MAP);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(discovery.candidates.len(), 1, "{discovery:#?}");
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.vf, 2);
    let prepared = prepare_vectorization_trial(&pre, candidate).expect("strict f64 vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    let text = print_kir_module(prepared.trial.module());
    assert!(text.contains("vector_multiply.strict"), "{text}");
    assert!(!text.contains("fast"), "{text}");
}

#[test]
fn loop_simd_strict_f64_unary_and_divide_should_remain_ordered_lane_operations() {
    let (pre, _) = map_state(STRICT_F64_UNARY_DIVIDE_MAP);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(discovery.candidates.len(), 1, "{discovery:?}");
    let candidate = &discovery.candidates[0];
    assert!(candidate.operations.iter().any(|operation| {
        operation.operation == KirProfileOperation::Negate
            && operation.semantics == calckernel::KirCostSemantics::StrictFloat
    }));
    assert!(candidate.operations.iter().any(|operation| {
        operation.operation == KirProfileOperation::Divide
            && operation.semantics == calckernel::KirCostSemantics::StrictFloat
    }));
    let prepared = prepare_vectorization_trial(&pre, candidate).expect("strict f64 unary trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    let text = print_kir_module(prepared.trial.module());
    assert!(text.contains("vector_negate.strict"), "{text}");
    assert!(text.contains("vector_divide.strict"), "{text}");
}

#[test]
fn loop_simd_supported_cast_should_map_input_lanes_to_f64_result_lanes() {
    let (pre, _) = map_state(CAST_MAP);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(discovery.candidates.len(), 1, "{discovery:#?}");
    let candidate = &discovery.candidates[0];
    assert_eq!(candidate.vf, 2);
    let prepared = prepare_vectorization_trial(&pre, candidate).expect("cast vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    let text = print_kir_module(prepared.trial.module());
    assert!(text.contains("vector_cast_u32tof64"), "{text}");
    assert!(text.contains("vector<u32, 2>"), "{text}");
    assert!(text.contains("vector<f64, 2>"), "{text}");
}

#[test]
fn loop_simd_pure_diamond_should_if_convert_to_compare_mask_and_select() {
    let (pre, _) = map_state(PURE_DIAMOND_MAP);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(
        discovery.candidates.len(),
        2,
        "{discovery:#?}\n{}",
        print_kir_module(pre.module())
    );
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4)
        .expect("VF4 diamond candidate");
    let prepared = prepare_vectorization_trial(&pre, candidate).expect("diamond vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    let text = print_kir_module(prepared.trial.module());
    assert!(text.contains("vector_compare"), "{text}");
    assert!(text.contains("vector_select"), "{text}");
}

#[test]
fn loop_simd_modular_add_and_multiply_reductions_should_fold_exact_lane_partitions() {
    let (pre, _) = map_state(MODULAR_REDUCTIONS);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(
        discovery.candidates.len(),
        4,
        "{discovery:#?}\n{}",
        print_kir_module(pre.module())
    );
    for candidate in discovery.candidates {
        let prepared = prepare_vectorization_trial(&pre, &candidate).expect("reduction trial");
        assert_eq!(
            check_vectorization_trial_independently(
                &pre,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Ok(())
        );
        let text = print_kir_module(prepared.trial.module());
        if candidate.function.index() == 0 {
            assert!(text.contains("vector_reduce_modularadd"), "{text}");
        } else {
            assert!(text.contains("vector_reduce_modularmultiply"), "{text}");
        }
    }
}

#[test]
fn loop_simd_unsupported_strict_f64_reduction_and_scan_should_remain_scalar() {
    let source = r#"
export fn strict_sum(a: slice<f64>, n: u32) -> f64 {
  let i: u32 = 0; let total: f64 = 0.0;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}
export unsafe fn prefix_sum(a: slice<u32>, b: slice<u32>, n: u32) -> u32
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0; let total: u32 = 0;
  while i < n { total = total + a[i]; b[i] = total; i = i + 1; }
  return total;
}
"#;
    let checked = check(&SourceFile::new("unsupported-reductions.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("unsupported reduction MIR");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        native_profile(),
    )
    .expect("unsupported reduction KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0)
        .expect("unsupported reduction facts");
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.vectorized_loops, 0);
    assert!(
        result
            .analysis_fallbacks
            .iter()
            .filter(|fallback| fallback.pass == "loop-simd")
            .count()
            >= 2,
        "{:?}",
        result.analysis_fallbacks
    );
    let text = print_kir_module(result.artifact.as_ref().expect("scalar reduction artifact"));
    assert!(!text.contains("vector_"), "{text}");
}

#[test]
fn vector_checker_should_reject_trial_plan_lane_partition_and_fallback_mutations() {
    let (pre, _) = map_state(MAP);
    let candidate = discover_vectorization_candidates(&pre).candidates.remove(0);
    let prepared = prepare_vectorization_trial(&pre, &candidate).expect("vector trial");

    let mut lane = prepared.plan.clone();
    lane.operations[0].lanes.swap(0, 1);
    assert!(
        check_vectorization_trial_independently(&pre, &prepared.trial, &lane, &prepared.charge)
            .is_err()
    );

    let mut partition = prepared.plan.clone();
    partition.epilogue = VectorEpilogue::None;
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &partition,
            &prepared.charge
        )
        .is_err()
    );

    let mut fallback = prepared.trial.clone();
    let function = &mut fallback.module_mut().functions[0];
    function.blocks.retain(|block| block.id != candidate.header);
    assert!(
        check_vectorization_trial_independently(&pre, &fallback, &prepared.plan, &prepared.charge)
            .is_err()
    );

    let mut wrong_binary = prepared.trial.clone();
    let binary_op = wrong_binary.module_mut().functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VectorBinary { op, .. } => Some(op),
            _ => None,
        })
        .expect("vector binary");
    *binary_op = calckernel::KirVectorBinaryOp::Multiply;
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &wrong_binary,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let (cast_pre, _) = map_state(CAST_MAP);
    let cast_candidate = discover_vectorization_candidates(&cast_pre)
        .candidates
        .remove(0);
    let cast = prepare_vectorization_trial(&cast_pre, &cast_candidate).expect("cast vector trial");
    let mut wrong_cast = cast.trial.clone();
    let cast_op = wrong_cast.module_mut().functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VectorCast { op, .. } => Some(op),
            _ => None,
        })
        .expect("vector cast");
    *cast_op = calckernel::KirVectorCastOp::I32ToF64;
    assert!(
        check_vectorization_trial_independently(&cast_pre, &wrong_cast, &cast.plan, &cast.charge,)
            .is_err()
    );

    let (diamond_pre, _) = map_state(PURE_DIAMOND_MAP);
    let diamond_candidate = discover_vectorization_candidates(&diamond_pre)
        .candidates
        .remove(0);
    let diamond = prepare_vectorization_trial(&diamond_pre, &diamond_candidate)
        .expect("diamond vector trial");
    let mut swapped = diamond.trial.clone();
    let select = swapped.module_mut().functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VectorSelect {
                when_true,
                when_false,
                ..
            } => Some((when_true, when_false)),
            _ => None,
        })
        .expect("diamond vector select");
    std::mem::swap(select.0, select.1);
    assert!(
        check_vectorization_trial_independently(
            &diamond_pre,
            &swapped,
            &diamond.plan,
            &diamond.charge,
        )
        .is_err()
    );
    let mut wrong_compare = diamond.trial.clone();
    let compare_op = wrong_compare.module_mut().functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VectorCompare { op, .. } => Some(op),
            _ => None,
        })
        .expect("vector compare");
    *compare_op = calckernel::MirCompareOp::Ge;
    assert!(
        check_vectorization_trial_independently(
            &diamond_pre,
            &wrong_compare,
            &diamond.plan,
            &diamond.charge,
        )
        .is_err()
    );

    let (reduction_pre, _) = map_state(MODULAR_REDUCTIONS);
    let reduction_candidate = discover_vectorization_candidates(&reduction_pre)
        .candidates
        .remove(0);
    let reduction = prepare_vectorization_trial(&reduction_pre, &reduction_candidate)
        .expect("reduction vector trial");
    let mut wrong_reduction = reduction.trial.clone();
    let reduction_op = wrong_reduction.module_mut().functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VectorReduce { op, .. } => Some(op),
            _ => None,
        })
        .expect("vector reduction");
    *reduction_op = calckernel::KirVectorReductionOp::ModularMultiply;
    assert!(
        check_vectorization_trial_independently(
            &reduction_pre,
            &wrong_reduction,
            &reduction.plan,
            &reduction.charge,
        )
        .is_err()
    );
}

#[test]
fn loop_simd_unknown_alias_should_emit_one_total_runtime_predicate_and_scalar_fallback() {
    let source = r#"
export fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 1; i = i + 1; }
}
"#;
    let (pre, _) = map_state(source);
    let discovery = discover_vectorization_candidates(&pre);
    assert_eq!(discovery.candidates.len(), 2, "{discovery:#?}");
    let candidate = discovery
        .candidates
        .into_iter()
        .find(|candidate| candidate.vf == 4)
        .expect("VF4 versioned candidate");
    let predicate = candidate
        .version_predicate
        .as_ref()
        .expect("unknown alias needs versioning");
    assert_eq!(predicate.conjuncts.len(), 1);
    let prepared = prepare_vectorization_trial(&pre, &candidate).expect("versioned vector trial");
    let text = print_kir_module(prepared.trial.module());
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "{text}"
    );
    assert!(prepared.plan.predicates.iter().any(|predicate| matches!(
        predicate,
        calckernel::VectorPredicate::AddressNonOverlap { .. }
    )));
    assert!(text.contains("version_predicate"), "{text}");

    let mut incomplete = prepared.plan.clone();
    incomplete.predicates.retain(|predicate| {
        !matches!(
            predicate,
            calckernel::VectorPredicate::AddressNonOverlap { .. }
        )
    });
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &incomplete,
            &prepared.charge,
        )
        .is_err()
    );
}

#[test]
fn vector_differential_total_predicate_and_lane_partition_cover_edges() {
    let trip = calckernel::ValueId::from_index(1);
    let left = calckernel::ValueId::from_index(2);
    let right = calckernel::ValueId::from_index(3);
    let predicate = calckernel::TotalVersionPredicate {
        address_bits: 64,
        conjuncts: vec![
            calckernel::VersionPredicateConjunct::TripThreshold {
                trip_count: trip,
                minimum: 8,
            },
            calckernel::VersionPredicateConjunct::AddressIntervalsDisjoint {
                left,
                left_count: trip,
                left_element_bytes: 4,
                right,
                right_count: trip,
                right_element_bytes: 4,
            },
        ],
    };
    for length in 0_u32..=257 {
        let vector_end = if length >= 8 {
            length - (length % 4)
        } else {
            0
        };
        let visited = (0..vector_end)
            .chain(vector_end..length)
            .collect::<Vec<_>>();
        assert_eq!(visited, (0..length).collect::<Vec<_>>(), "length={length}");

        let mut values =
            BTreeMap::from([(trip, u64::from(length)), (left, 0x1000), (right, 0x8000)]);
        assert_eq!(predicate.evaluate(&values), length >= 8, "length={length}");
        values.insert(right, 0x1004);
        assert!(!predicate.evaluate(&values), "overlap length={length}");
    }

    let overflowing = BTreeMap::from([(trip, 8), (left, u64::MAX - 8), (right, 0x8000)]);
    assert!(!predicate.evaluate(&overflowing));
}

#[test]
fn vector_frontier_pipeline_should_commit_one_native_winner_and_audit_alternatives() {
    let (pre, contracts) = map_state(MAP);
    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops,
        1,
        "fallbacks={:?} audit={:?}",
        result.analysis_fallbacks,
        result.audit.attempts()
    );
    let accepted = result
        .audit
        .attempts()
        .iter()
        .filter(|attempt| attempt.disposition == CandidateDisposition::Accepted)
        .collect::<Vec<_>>();
    assert_eq!(accepted.len(), 1, "{:?}", result.audit.attempts());
    let artifact = result.artifact.expect("verified artifact");
    assert!(artifact.functions[0].vector_regions.len() == 1);
}

#[test]
fn vector_frontier_exact_loop_should_price_slp_from_the_same_immutable_pre_state() {
    let source = r#"
export unsafe fn map(a: slice<u32>, b: slice<u32>) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < 16 {
    let x: u32 = a[i];
    let p0: u32 = x + 1;
    let p1: u32 = x + 2;
    let p2: u32 = x + 3;
    let p3: u32 = x + 4;
    b[i] = p0 + p1 + p2 + p3;
    i = i + 1;
  }
}
"#;
    let (pre, contracts) = map_state(source);
    let direct = discover_vectorization_candidates(&pre);
    assert_eq!(direct.candidates.len(), 2, "{direct:?}");
    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops,
        1,
        "fallbacks={:?} audit={:?}",
        result.analysis_fallbacks,
        result.audit.attempts()
    );
    let attempts = result.audit.attempts();
    assert!(
        attempts.iter().any(|attempt| {
            matches!(attempt.key, calckernel::CandidateKey::ResidualSlp { .. })
                && attempt.disposition == CandidateDisposition::NonWinner
                && attempt.reason == "higher-cost-loop-alternative"
        }),
        "{attempts:?}"
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt.disposition == CandidateDisposition::Accepted)
            .count(),
        1,
        "{attempts:?}"
    );
}

#[test]
fn loop_simd_checked_ordered_or_unknown_alias_neighbors_should_remain_scalar() {
    for source in [
        "export fn copy(a: slice<u32>, b: slice<u32>, n: u32) -> void { let i: u32 = 0; while i < n { b[i] = a[i]; i = i + 1; } }",
        "fn observe(x: u32) -> u32 { return x; } export fn noisy(a: slice<u32>, b: slice<u32>, n: u32) -> void { let i: u32 = 0; while i < n { b[i] = observe(a[i]); i = i + 1; } }",
    ] {
        let (pre, _) = map_state(source);
        assert!(
            discover_vectorization_candidates(&pre)
                .candidates
                .is_empty()
        );
    }
}

#[test]
fn loop_simd_checked_modes_should_preserve_guards_and_scalar_first_error_order() {
    let checked = check(&SourceFile::new("checked-vectorize.ck", MAP));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("checked MIR");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        native_profile(),
    )
    .expect("checked KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0)
        .expect("checked contract facts");
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.vectorized_loops, 0);
    assert!(result.analysis_fallbacks.iter().any(|fallback| {
        fallback.pass == "loop-simd" && fallback.reason == "checked-mode-requires-lane-proof"
    }));
    let text = print_kir_module(result.artifact.as_ref().expect("checked scalar artifact"));
    assert!(!text.contains("vector_"), "{text}");
    assert!(text.contains("guard"), "{text}");
}

#[test]
fn loop_simd_contract_sanitizer_should_disable_every_code_duplicating_frontier() {
    let checked = check(&SourceFile::new("sanitized-vectorize.ck", MAP));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("sanitized MIR");
    let mut module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        native_profile(),
    )
    .expect("sanitized KIR");
    // Public construction reserves contract sanitization for executables. This
    // optimizer-only fixture keeps the vector-capable library profile and
    // toggles the mode solely to verify every code-duplicating frontier gate.
    module.config.sanitizer_mode = KirSanitizerMode::Contracts;
    let contracts = import_contract_facts(&module, &checked.checked_program, 0)
        .expect("sanitized contract facts");
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.vectorized_loops, 0);
    assert_eq!(result.stats.slp_packs, 0);
    assert_eq!(result.stats.full_unrolled_loops, 0);
    assert_eq!(result.stats.partial_unrolled_loops_factor_2, 0);
    assert_eq!(result.stats.partial_unrolled_loops_factor_4, 0);
    let text = print_kir_module(result.artifact.as_ref().expect("sanitized scalar artifact"));
    assert!(!text.contains("vector_"), "{text}");
}

#[test]
fn loop_simd_profile_must_make_every_emitted_operation_legal() {
    let (pre, _) = map_state(MAP);
    let candidate = discover_vectorization_candidates(&pre).candidates.remove(0);
    for operation in &candidate.operations {
        assert!(matches!(
            pre.module().profile.operation_availability(&KirCostKey {
                operation: operation.operation,
                lane: operation.lane_type,
                lanes: u8::try_from(candidate.vf).unwrap(),
                semantics: operation.semantics,
                alignment: operation.alignment,
            }),
            Some(KirOperationAvailability::Legal(_))
        ));
    }
}
