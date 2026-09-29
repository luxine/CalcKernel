use std::collections::BTreeMap;

use calckernel::{
    CandidateDisposition, ContractFactSet, KirAlignmentClass, KirBoundsMode, KirBuildConfig,
    KirConsumer, KirCostKey, KirLegalCost, KirNativeCpuPolicy, KirOperationAvailability,
    KirOptimizationLevel, KirOverflowMode, KirProfileOperation, KirSanitizerMode, KirTargetProfile,
    KirTargetProfileBuilder, KirVerifiedProgramState, KirWasmFeatures, SourceFile, VectorEpilogue,
    build_kir_module_with_profile, check, check_vectorization_trial_independently,
    discover_vectorization_candidates, import_contract_facts, lower_to_mir,
    prepare_vectorization_trial, print_kir_module, run_kir_multiversion_pass_pipeline,
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
    native_profile_with_triple(consumer, maximum_interleave_factor, "aarch64-apple-darwin")
}

fn native_profile_with_triple(
    consumer: KirConsumer,
    maximum_interleave_factor: u8,
    triple: &str,
) -> KirTargetProfile {
    native_profile_with_cpu_features(
        consumer,
        maximum_interleave_factor,
        triple,
        KirNativeCpuPolicy::Baseline,
        vec!["+neon".to_string()],
    )
}

fn native_profile_with_cpu_features(
    consumer: KirConsumer,
    maximum_interleave_factor: u8,
    triple: &str,
    policy: KirNativeCpuPolicy,
    features: Vec<String>,
) -> KirTargetProfile {
    native_profile_with_cpu_features_missing_u32x4_splat(
        consumer,
        maximum_interleave_factor,
        triple,
        policy,
        features,
        false,
    )
}

fn native_profile_without_u32x4_splat() -> KirTargetProfile {
    native_profile_with_cpu_features_missing_u32x4_splat(
        KirConsumer::NativeLibrary,
        1,
        "aarch64-apple-darwin",
        KirNativeCpuPolicy::Baseline,
        vec!["+neon".to_string()],
        true,
    )
}

fn native_profile_with_cpu_features_missing_u32x4_splat(
    consumer: KirConsumer,
    maximum_interleave_factor: u8,
    triple: &str,
    policy: KirNativeCpuPolicy,
    features: Vec<String>,
    missing_u32x4_splat: bool,
) -> KirTargetProfile {
    let mut builder =
        KirTargetProfileBuilder::native(consumer, triple, 64, true, policy, "generic", features)
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
        if missing_u32x4_splat
            && key.operation == KirProfileOperation::Splat
            && key.lane == calckernel::KirLaneType::U32
            && key.lanes == 4
        {
            builder
                .set_unavailable(key)
                .expect("unavailable vector splat");
            continue;
        }
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
    let consumer = profile.consumer();
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer,
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

fn wasm_map_state(
    source: &str,
    features: KirWasmFeatures,
) -> (KirVerifiedProgramState, Option<ContractFactSet>) {
    map_state_with_profile(
        source,
        KirTargetProfile::webassembly_with_features(features),
    )
}

fn wasm_pure_diamond_source(lane: &str, comparison: &str) -> String {
    format!(
        r#"
export unsafe fn map(a: slice<{lane}>, b: slice<{lane}>, n: u32, pivot: {lane}, delta: {lane}) -> void
contract {{ requires n <= a.len && n <= b.len; requires noalias(a, b); effects read(a), write(b); }}
{{
  let i: u32 = 0;
  while i < n {{
    let x: {lane} = a[i];
    let selected: {lane} = delta;
    if x {comparison} pivot {{ selected = x + delta; }} else {{ selected = x - delta; }}
    b[i] = selected;
    i = i + 1;
  }}
}}
"#
    )
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

const INTERLEAVE_ZIP: &str = r#"
export unsafe fn zip_u32(
  a: slice<u32>, b: slice<u32>, out: slice<u32>, n: u32
) -> void
contract {
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), write(out);
}
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] + b[i]; i = i + 1; }
}
"#;

const SPECIALIZED_LENGTH_MAP: &str = r#"
unsafe fn fixed_map(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 13; i = i + 1; }
}

export unsafe fn map(a: slice<u32>, b: slice<u32>) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  unsafe { fixed_map(a, b, 4000); }
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

const WASM_I32_CAST_MAP: &str = r#"
export unsafe fn map(a: slice<i32>, b: slice<f64>, n: u32) -> void
contract { requires n <= a.len && n <= b.len; requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = i32_to_f64(a[i]); i = i + 1; }
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
    assert!(
        discovery.candidates.iter().all(|candidate| {
            candidate.minimum_trip >= u32::from(candidate.vf) * u32::from(candidate.uf) * 2
        }),
        "AArch64 runtime-trip Loop SIMD must retain the proven two-chunk floor: {discovery:#?}"
    );
    assert!(
        discovery.candidates.iter().any(|candidate| {
            candidate.minimum_trip == u32::from(candidate.vf) * u32::from(candidate.uf) * 2
        }),
        "AArch64 must not inherit the x86-specific four-chunk penalty: {discovery:#?}"
    );

    let (x86_pre, _) = map_state_with_profile(
        INTERLEAVE_MAP,
        native_profile_with_triple(KirConsumer::NativeLibrary, 4, "x86_64-unknown-linux-gnu"),
    );
    let x86_discovery = discover_vectorization_candidates(&x86_pre);
    assert!(
        x86_discovery.candidates.iter().all(|candidate| {
            let uf = u32::from(candidate.uf);
            candidate.minimum_trip >= 4_u32.div_ceil(uf) * u32::from(candidate.vf) * uf
        }),
        "x86 runtime-trip Loop SIMD must amortize control over four actual vector operations: {x86_discovery:#?}"
    );
    assert!(
        x86_discovery.candidates.iter().any(|candidate| {
            candidate.vf == 4 && candidate.uf == 4 && candidate.minimum_trip == 16
        }),
        "x86 VF4/UF4 must admit the exact 16-element noalias loop: {x86_discovery:#?}"
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
                    calckernel::CandidateKey::LoopFrontier { vf: 4, uf: 4, .. }
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
fn x86_independent_three_stream_loop_should_select_four_vector_chains() {
    let (pre, contracts) = map_state_with_profile(
        INTERLEAVE_ZIP,
        native_profile_with_triple(KirConsumer::NativeLibrary, 4, "x86_64-unknown-linux-gnu"),
    );
    let candidate = discover_vectorization_candidates(&pre)
        .candidates
        .into_iter()
        .find(|candidate| candidate.vf == 4 && candidate.uf == 4)
        .expect("x86 VF4/UF4 three-stream candidate");
    let prepared =
        prepare_vectorization_trial(&pre, &candidate).expect("compact x86 VF4/UF4 trial");
    assert!(
        prepared.plan.growth.module_after_units
            <= prepared.plan.growth.module_before_units.saturating_mul(2),
        "four-chain plan exceeded the unchanged aggregate growth ceiling: {:#?}",
        prepared.plan.growth
    );
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    let mut forged_stride = prepared.trial.clone();
    let stride = forged_stride.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_body")
        .expect("vector body")
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::ConstInt { value } if value == "4" => Some(value),
            _ => None,
        })
        .expect("shared vector-width stride");
    *stride = "5".to_string();
    assert!(
        check_vectorization_trial_independently(
            &pre,
            &forged_stride,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "independent checker accepted a forged UF stride"
    );
    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let accepted = result
        .vector_explanations
        .iter()
        .find(|explanation| explanation.disposition == CandidateDisposition::Accepted)
        .expect("accepted x86 three-stream vector plan");
    assert_eq!(
        (accepted.vf, accepted.uf),
        (4, 4),
        "x86 independent three-stream loop must amortize control across four chains; explanations={:#?}; audit={:#?}",
        result.vector_explanations,
        result.audit.attempts()
    );
}

#[test]
fn x86_single_map_loop_should_select_four_vector_chains_without_padding_the_module() {
    let (pre, contracts) = map_state_with_profile(
        MAP,
        native_profile_with_triple(KirConsumer::NativeLibrary, 4, "x86_64-unknown-linux-gnu"),
    );
    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let accepted = result
        .vector_explanations
        .iter()
        .find(|explanation| explanation.disposition == CandidateDisposition::Accepted)
        .expect("accepted x86 map vector plan");
    assert_eq!(
        (accepted.vf, accepted.uf),
        (4, 4),
        "a standalone x86 streaming map must retain four independent chains under the unchanged module-growth ceiling; explanations={:#?}; audit={:#?}",
        result.vector_explanations,
        result.audit.attempts()
    );
    assert!(
        accepted.growth.module_after_units <= accepted.growth.module_before_units.saturating_mul(2),
        "standalone four-chain map exceeded the unchanged aggregate growth ceiling: {:#?}",
        accepted.growth
    );
    let vector_body = result
        .artifact
        .as_ref()
        .expect("vectorized artifact")
        .functions
        .iter()
        .find(|function| function.name == "map")
        .expect("vectorized function")
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_body")
        .expect("vector body");
    assert!(
        vector_body.params.is_empty(),
        "single-predecessor interleaved vector body retained redundant parameters"
    );
}

#[test]
fn x86_widening_integer_cast_should_respect_the_two_chain_frontend_budget() {
    let source = r#"
export unsafe fn map_cast(a: slice<u32>, out: slice<f64>, n: u32) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = u32_to_f64(a[i]); i = i + 1; }
}
"#;
    let (pre, contracts) = map_state_with_profile(
        source,
        native_profile_with_triple(KirConsumer::NativeLibrary, 4, "x86_64-unknown-linux-gnu"),
    );
    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let accepted = result
        .vector_explanations
        .iter()
        .find(|explanation| explanation.disposition == CandidateDisposition::Accepted)
        .expect("accepted x86 widening-cast vector plan");
    assert_eq!(
        (accepted.vf, accepted.uf),
        (2, 2),
        "the x86 u32-to-f64 expansion must stay within the measured two-chain frontend budget; explanations={:#?}",
        result.vector_explanations
    );
}

#[test]
fn loop_simd_should_schedule_independent_unrolled_loads_before_stores() {
    let (pre, _) = map_state_with_profile(
        INTERLEAVE_MAP,
        native_profile_with_triple(KirConsumer::NativeLibrary, 2, "x86_64-unknown-linux-gnu"),
    );
    let candidate = discover_vectorization_candidates(&pre)
        .candidates
        .into_iter()
        .find(|candidate| candidate.vf == 4 && candidate.uf == 2)
        .expect("x86 VF4/UF2 candidate");
    let prepared = prepare_vectorization_trial(&pre, &candidate).expect("vector trial");
    let vector_body = prepared
        .trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("vectorized function")
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_body")
        .expect("vector body");
    let load_positions = vector_body
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorLoad { .. }
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let store_positions = vector_body
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorStore { .. }
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();

    assert_eq!(load_positions.len(), 2);
    assert_eq!(store_positions.len(), 2);
    assert!(
        load_positions.iter().max() < store_positions.iter().min(),
        "independent UF chunks must expose both loads before either store:\n{}",
        print_kir_module(prepared.trial.module())
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
fn loop_simd_x86_strict_f64_should_admit_four_independent_vector_chains() {
    let (pre, _) = map_state_with_profile(
        STRICT_F64_MAP,
        native_profile_with_triple(KirConsumer::NativeLibrary, 4, "x86_64-unknown-linux-gnu"),
    );
    let discovery = discover_vectorization_candidates(&pre);
    assert!(
        discovery
            .candidates
            .iter()
            .any(|candidate| candidate.vf == 2 && candidate.uf == 4),
        "x86 strict-f64 map omitted its four-chain schedule: {discovery:#?}"
    );
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
fn x86_loop_simd_should_defer_horizontal_reductions_to_the_native_loop_vectorizer() {
    let profile =
        native_profile_with_triple(KirConsumer::NativeLibrary, 1, "x86_64-unknown-linux-gnu");
    let (pre, _) = map_state_with_profile(MODULAR_REDUCTIONS, profile);
    let discovery = discover_vectorization_candidates(&pre);
    assert!(
        discovery
            .candidates
            .iter()
            .all(|candidate| candidate.reduction.is_none()),
        "a per-chunk horizontal reduction blocks LLVM's loop-carried vector accumulator: {discovery:#?}"
    );
    assert!(discovery.fallbacks.iter().any(|fallback| {
        fallback.reason == "x86-horizontal-reduction-deferred-to-native-loop-vectorizer"
    }));
}

#[test]
fn aarch64_sve_tiers_should_defer_whole_loops_to_the_scalable_native_vectorizer() {
    for feature in ["+sve", "+sve2"] {
        let profile = native_profile_with_cpu_features(
            KirConsumer::NativeLibrary,
            1,
            "aarch64-unknown-linux-gnu",
            KirNativeCpuPolicy::Multiversion,
            vec!["+neon".to_string(), feature.to_string()],
        );
        let (pre, _) = map_state_with_profile(INTERLEAVE_MAP, profile);
        let discovery = discover_vectorization_candidates(&pre);
        assert!(
            discovery.candidates.is_empty(),
            "fixed-width KIR vectors preempt the SVE whole-loop vectorizer: {discovery:#?}"
        );
        assert!(discovery.fallbacks.iter().any(|fallback| {
            fallback.reason == "aarch64-sve-loop-deferred-to-native-loop-vectorizer"
        }));
    }

    let baseline =
        native_profile_with_triple(KirConsumer::NativeLibrary, 1, "aarch64-unknown-linux-gnu");
    let (pre, _) = map_state_with_profile(INTERLEAVE_MAP, baseline);
    assert!(
        !discover_vectorization_candidates(&pre)
            .candidates
            .is_empty(),
        "the AArch64 baseline tier must retain fixed-width KIR vectorization"
    );
}

#[test]
fn multiversion_baseline_should_defer_whole_loops_to_each_native_target_vectorizer() {
    for triple in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
        let profile = native_profile_with_cpu_features(
            KirConsumer::NativeLibrary,
            4,
            triple,
            KirNativeCpuPolicy::Multiversion,
            vec!["+neon".to_string()],
        );
        let (pre, contracts) = map_state_with_profile(INTERLEAVE_MAP, profile);
        let ordinary = run_kir_pass_pipeline(
            pre.module().clone(),
            KirOptimizationLevel::O3,
            contracts.as_ref(),
        );
        let deferred = run_kir_multiversion_pass_pipeline(
            pre.module().clone(),
            KirOptimizationLevel::O3,
            contracts.as_ref(),
        );
        assert!(
            ordinary.stats.vectorized_loops > 0,
            "ordinary Native O3 should still use verified KIR vectors: {ordinary:#?}"
        );
        assert_eq!(deferred.stats.vectorized_loops, 0);
        assert_eq!(
            deferred.stats.full_unrolled_loops
                + deferred.stats.partial_unrolled_loops_factor_2
                + deferred.stats.partial_unrolled_loops_factor_4,
            0,
            "multiversion loop shape must remain available to each LLVM target: {deferred:#?}"
        );
        assert!(deferred.analysis_fallbacks.iter().any(|fallback| {
            fallback.reason == "multiversion-loop-deferred-to-native-loop-vectorizer"
        }));
    }
}

#[test]
fn constant_call_loop_should_defer_unroll_and_vector_width_to_native_llvm() {
    let profile =
        native_profile_with_triple(KirConsumer::NativeLibrary, 4, "x86_64-unknown-linux-gnu");
    let (pre, _) = map_state_with_profile(SPECIALIZED_LENGTH_MAP, profile);
    let discovery = discover_vectorization_candidates(&pre);
    assert!(
        discovery.candidates.is_empty(),
        "the constant-call loop must remain scalar for LLVM: {discovery:#?}"
    );
    assert!(
        discovery.fallbacks.iter().any(|fallback| {
            fallback.reason == "constant-call-loop-deferred-to-native-loop-vectorizer"
        }),
        "missing constant-call deferral: {discovery:#?}"
    );
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
    assert_eq!(
        predicate.address_bits, 64,
        "native target retains native width"
    );
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
fn wasm32_alias_predicate_should_fail_closed_at_address_space_edges() {
    let count = calckernel::ValueId::from_index(0);
    let left = calckernel::ValueId::from_index(1);
    let right = calckernel::ValueId::from_index(2);
    let predicate = calckernel::TotalVersionPredicate {
        address_bits: 32,
        conjuncts: vec![
            calckernel::VersionPredicateConjunct::AddressIntervalsDisjoint {
                left,
                left_count: count,
                left_element_bytes: 4,
                right,
                right_count: count,
                right_element_bytes: 4,
            },
        ],
    };

    // A valid pair close to the top of the 32-bit address space remains disjoint.
    assert!(predicate.evaluate(&BTreeMap::from([
        (count, 3),
        (left, 0xffff_ff00),
        (right, 0x1000),
    ])));
    // End-address addition must not wrap and turn an invalid interval into a
    // seemingly disjoint low-address range.
    assert!(!predicate.evaluate(&BTreeMap::from([
        (count, 4),
        (left, 0xffff_fff8),
        (right, 0x1000),
    ])));
    // Inputs outside the Wasm32 pointer/count domain also fail closed.
    assert!(!predicate.evaluate(&BTreeMap::from([
        (count, 1_u64 << 32),
        (left, 0x1000),
        (right, 0x2000),
    ])));
    assert!(!predicate.evaluate(&BTreeMap::from([
        (count, 1),
        (left, 1_u64 << 32),
        (right, 0x2000),
    ])));
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

const WASM_F64_MAP: &str = r#"
export unsafe fn map_f64(a: slice<f64>, b: slice<f64>, n: u32, factor: f64, bias: f64) -> void
contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
}
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] * factor + bias; i = i + 1; }
}
"#;

const WASM_CONST_F64_MAP: &str = r#"
export unsafe fn map_f64(a: slice<f64>, b: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
}
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] * 1.25 + 0.5; i = i + 1; }
}
"#;

const WASM_SCALAR_SELECT_ARMS: &str = r#"
export unsafe fn pick(a: slice<u32>, c: slice<u32>, b: slice<u32>, n: u32) -> void
contract {
  requires n <= a.len && n <= c.len && n <= b.len;
  requires noalias(a, c) && noalias(a, b) && noalias(c, b);
  effects read(a), read(c), write(b);
}
{
  let i: u32 = 0;
  while i < n {
    let left: u32 = a[i];
    let right: u32 = c[i];
    let picked: u32 = 0;
    if left < right { picked = 5; } else { picked = 10; }
    b[i] = picked;
    i = i + 1;
  }
}
"#;

const WASM_SLICE_LEN_SUM: &str = r#"
export fn sum(a: slice<u32>, other: slice<u32>) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < a.len { total = total + a[i]; i = i + 1; }
  return total;
}
"#;

const WASM_CHANGING_SLICE_LEN_SUM: &str = r#"
export fn sum(a: slice<u32>, other: slice<u32>) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;
  let current: slice<u32> = a;
  while i < current.len {
    total = total + a[i];
    current = other;
    i = i + 1;
  }
  return total;
}
"#;

const WASM_I32_MAP: &str = r#"
export unsafe fn map_i32(a: slice<i32>, b: slice<i32>, n: u32, factor: i32, bias: i32) -> void
contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
}
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] * factor + bias; i = i + 1; }
}
"#;

const WASM_RUNTIME_ALIAS_MAP: &str = include_str!("../../examples/wasm/alias_map.ck");
const WASM_MODULAR_REDUCTIONS: &str = include_str!("../../examples/wasm/reduction.ck");

#[test]
fn wasm_simd128_slice_len_bound_should_materialize_from_a_stable_descriptor_root() {
    let (pre, _) = wasm_map_state(WASM_SLICE_LEN_SUM, KirWasmFeatures::Simd128);
    let discovery = discover_vectorization_candidates(&pre);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4)
        .unwrap_or_else(|| panic!("expected a u32 reduction candidate: {discovery:#?}"));
    let prepared = prepare_vectorization_trial(&pre, candidate)
        .expect("stable slice descriptor length should materialize in the preheader");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
    let mut forged = prepared.trial.clone();
    let other = forged.module().functions[0].params[1].value;
    let preheader = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("sum function")
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("versioning preheader");
    let slice_len = preheader
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::SliceLen { slice } => Some(slice),
            _ => None,
        })
        .expect("materialized slice length");
    *slice_len = other;
    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge,)
            .is_err(),
        "the checker must reject a preheader SliceLen derived from a different parameter"
    );
    assert!(print_kir_module(prepared.trial.module()).contains("vector_reduce"));
}

#[test]
fn wasm_simd128_const_float_operands_should_remain_scalar_splat_inputs() {
    let (pre, _) = wasm_map_state(WASM_CONST_F64_MAP, KirWasmFeatures::Simd128);
    let discovery = discover_vectorization_candidates(&pre);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 2)
        .unwrap_or_else(|| panic!("expected an f64 candidate: {discovery:#?}"));
    let prepared = prepare_vectorization_trial(&pre, candidate)
        .expect("constant f64 values should remain scalar inputs to vector splats");
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
    assert!(text.contains("vector_splat"), "{text}");
    assert!(text.contains("vector_multiply.strict"), "{text}");
    assert!(text.contains("vector_add.strict"), "{text}");
}

#[test]
fn wasm_simd128_select_arm_splats_should_be_counted_in_vector_cost() {
    let (pre, _) = wasm_map_state(WASM_SCALAR_SELECT_ARMS, KirWasmFeatures::Simd128);
    let discovery = discover_vectorization_candidates(&pre);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4 && candidate.uf == 1)
        .unwrap_or_else(|| panic!("expected SIMD select candidate: {discovery:#?}"));
    let prepared = prepare_vectorization_trial(&pre, candidate)
        .expect("scalar select arms should materialize splats");
    let vector_splats = prepared.trial.module().functions[0]
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_body")
        .expect("vector body")
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorSplat { .. }
            )
        })
        .count();
    assert_eq!(vector_splats, 2, "both scalar select arms need one splat");

    let profile = &pre.module().profile;
    let cost = |key: KirCostKey| match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(legal)) if legal.legalization_parts == 1 => legal.cost,
        Some(KirOperationAvailability::Unavailable)
            if key.operation == KirProfileOperation::Branch =>
        {
            1
        }
        availability => panic!("missing exact profile cost for {key:?}: {availability:?}"),
    };
    let lanes = u8::try_from(candidate.vf).expect("vector width fits profile key");
    let operation_chunk_cost = candidate
        .operations
        .iter()
        .map(|operation| {
            cost(KirCostKey {
                operation: operation.operation,
                lane: operation.lane_type,
                lanes,
                semantics: operation.semantics,
                alignment: operation.alignment,
            })
        })
        .sum::<u32>()
        .saturating_mul(u32::from(candidate.uf));
    let memory_chunk_cost = candidate
        .accesses
        .iter()
        .map(|access| {
            let lane = match access.element_type {
                calckernel::MirType::Primitive(calckernel::MirPrimitiveTypeName::I32) => {
                    calckernel::KirLaneType::I32
                }
                calckernel::MirType::Primitive(calckernel::MirPrimitiveTypeName::U32) => {
                    calckernel::KirLaneType::U32
                }
                _ => panic!("unexpected select memory type: {:?}", access.element_type),
            };
            cost(KirCostKey {
                operation: if access.kind == calckernel::LoopMemoryAccessKind::Read {
                    KirProfileOperation::Load
                } else {
                    KirProfileOperation::Store
                },
                lane,
                lanes,
                semantics: calckernel::KirCostSemantics::NotApplicable,
                alignment: KirAlignmentClass::Bytes(
                    u16::try_from(access.element_bytes).expect("alignment fits profile key"),
                ),
            })
        })
        .sum::<u32>()
        .saturating_mul(u32::from(candidate.uf));
    let control_cost = cost(KirCostKey {
        operation: KirProfileOperation::Add,
        lane: calckernel::KirLaneType::U32,
        lanes: 1,
        semantics: calckernel::KirCostSemantics::Modular,
        alignment: KirAlignmentClass::NotApplicable,
    }) + cost(KirCostKey {
        operation: KirProfileOperation::Compare,
        lane: calckernel::KirLaneType::U32,
        lanes: 1,
        semantics: calckernel::KirCostSemantics::NotApplicable,
        alignment: KirAlignmentClass::NotApplicable,
    }) + cost(KirCostKey {
        operation: KirProfileOperation::Branch,
        lane: calckernel::KirLaneType::U32,
        lanes: 1,
        semantics: calckernel::KirCostSemantics::NotApplicable,
        alignment: KirAlignmentClass::NotApplicable,
    });
    let splat_cost = 2 * cost(KirCostKey {
        operation: KirProfileOperation::Splat,
        lane: calckernel::KirLaneType::U32,
        lanes,
        semantics: calckernel::KirCostSemantics::NotApplicable,
        alignment: KirAlignmentClass::NotApplicable,
    });
    let chunk_cost = operation_chunk_cost
        .saturating_add(memory_chunk_cost)
        .saturating_add(control_cost)
        .saturating_add(splat_cost);
    let chunks = candidate.minimum_trip / (u32::from(candidate.vf) * u32::from(candidate.uf));
    assert_eq!(
        candidate.predicted_cost.transformed_body,
        chunk_cost.saturating_mul(chunks),
        "vector cost must include both Select-arm splats"
    );
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let mut forged = prepared.trial.clone();
    let function = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("pick function");
    let vector_body = function
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_body")
        .expect("vector body");
    let non_splat_vector = vector_body
        .instructions
        .iter()
        .find_map(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorLoad { .. }
            )
            .then(|| instruction.results.first().map(|result| result.value))
            .flatten()
        })
        .expect("vector load result for forged arm");
    let select = vector_body
        .instructions
        .iter_mut()
        .find(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorSelect { .. }
            )
        })
        .expect("vector select");
    let calckernel::KirInstructionKind::VectorSelect { when_true, .. } = &mut select.kind else {
        unreachable!();
    };
    *when_true = non_splat_vector;
    assert!(
        check_vectorization_trial_independently(&pre, &forged, &prepared.plan, &prepared.charge)
            .is_err(),
        "an arbitrary same-typed vector must not masquerade as a scalar-arm splat"
    );
}

#[test]
fn vector_candidate_requires_target_splat_support_for_scalar_select_arms() {
    let profile = native_profile_without_u32x4_splat();
    let splat_key = KirCostKey {
        operation: KirProfileOperation::Splat,
        lane: calckernel::KirLaneType::U32,
        lanes: 4,
        semantics: calckernel::KirCostSemantics::NotApplicable,
        alignment: KirAlignmentClass::NotApplicable,
    };
    assert!(matches!(
        profile.operation_availability(&splat_key),
        Some(KirOperationAvailability::Unavailable)
    ));
    let (pre, _) = map_state_with_profile(WASM_SCALAR_SELECT_ARMS, profile);
    let discovery = discover_vectorization_candidates(&pre);
    assert!(
        discovery
            .candidates
            .iter()
            .all(|candidate| candidate.vf != 4),
        "a target without u32x4 Splat must not receive a VF4 scalar-select candidate: {discovery:#?}"
    );
}

#[test]
fn wasm_simd128_slice_len_bound_with_changing_backedge_descriptor_should_be_rejected() {
    let (pre, _) = wasm_map_state(WASM_CHANGING_SLICE_LEN_SUM, KirWasmFeatures::Simd128);
    let discovery = discover_vectorization_candidates(&pre);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4)
        .unwrap_or_else(|| {
            panic!("expected the reducer shape before safety checking: {discovery:#?}")
        });
    let error = prepare_vectorization_trial(&pre, candidate)
        .expect_err("changing slice descriptor must not become a fixed trip bound");
    assert_eq!(
        error,
        "vector slice-length descriptor is not invariant across the loop"
    );
}

#[test]
fn wasm_simd128_should_discover_independently_check_and_accept_f64x2_and_i32x4_maps() {
    for (source, expected_vf, expected_lane, vector_op) in [
        (WASM_F64_MAP, 2, "vector<f64, 2>", "vector_add.strict"),
        (WASM_I32_MAP, 4, "vector<i32, 4>", "vector_add.modular"),
    ] {
        let (pre, contracts) = wasm_map_state(source, KirWasmFeatures::Simd128);
        let discovery = discover_vectorization_candidates(&pre);
        let candidate = discovery
            .candidates
            .iter()
            .find(|candidate| candidate.vf == expected_vf)
            .unwrap_or_else(|| {
                panic!(
                    "SIMD128 should discover the {expected_lane} map: {discovery:#?}\n{}",
                    print_kir_module(pre.module())
                )
            });

        let prepared = prepare_vectorization_trial(&pre, candidate)
            .expect("SIMD128 loop candidate should prepare a trial");
        assert_eq!(
            check_vectorization_trial_independently(
                &pre,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Ok(()),
            "the independent checker must accept the {expected_lane} map"
        );
        assert!(matches!(
            prepared.plan.epilogue,
            VectorEpilogue::Scalar { .. }
        ));
        let trial_text = print_kir_module(prepared.trial.module());
        assert!(trial_text.contains(expected_lane), "{trial_text}");
        assert!(trial_text.contains("vector_load"), "{trial_text}");
        assert!(trial_text.contains("vector_store"), "{trial_text}");
        assert!(trial_text.contains(vector_op), "{trial_text}");
        assert!(
            prepared.trial.module().functions[0]
                .blocks
                .iter()
                .any(|block| block.id == candidate.header),
            "the original scalar loop must remain as the tail path"
        );

        let result = run_kir_pass_pipeline(
            pre.module().clone(),
            KirOptimizationLevel::O3,
            contracts.as_ref(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result.stats.vectorized_loops >= 1, "{:#?}", result.stats);
        assert!(
            result.vector_explanations.iter().any(|explanation| {
                explanation.disposition == CandidateDisposition::Accepted
                    && explanation.vf == expected_vf
            }),
            "{:#?}",
            result.vector_explanations
        );
        let optimized_text =
            print_kir_module(result.artifact.as_ref().expect("SIMD128 O3 artifact"));
        assert!(optimized_text.contains("vector_load"), "{optimized_text}");
        assert!(optimized_text.contains("vector_store"), "{optimized_text}");
    }
}

#[test]
fn wasm_baseline_should_keep_contiguous_maps_scalar() {
    for source in [WASM_F64_MAP, WASM_I32_MAP] {
        let (pre, contracts) = wasm_map_state(source, KirWasmFeatures::Baseline);
        assert!(
            discover_vectorization_candidates(&pre)
                .candidates
                .is_empty()
        );
        let result = run_kir_pass_pipeline(
            pre.module().clone(),
            KirOptimizationLevel::O3,
            contracts.as_ref(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.stats.vectorized_loops, 0);
        assert_eq!(result.stats.slp_packs, 0);
        let text = print_kir_module(result.artifact.as_ref().expect("baseline O3 artifact"));
        assert!(!text.contains("vector_"), "{text}");
    }
}

#[test]
fn wasm_simd128_should_leave_unsupported_loop_shapes_scalar() {
    const STRICT_F64_REDUCTION: &str = r#"
export fn strict_sum(a: slice<f64>, n: u32, initial: f64) -> f64 {
  let i: u32 = 0;
  let total: f64 = initial;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}
"#;
    const MINIMUM_REDUCTION: &str = r#"
export fn minimum(a: slice<u32>, n: u32, initial: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = initial;
  while i < n {
    let value: u32 = a[i];
    if value < total { total = value; }
    i = i + 1;
  }
  return total;
}
"#;
    for source in [STRICT_F64_REDUCTION, MINIMUM_REDUCTION] {
        let (pre, contracts) = wasm_map_state(source, KirWasmFeatures::Simd128);
        let discovery = discover_vectorization_candidates(&pre);
        assert!(
            discovery.candidates.is_empty(),
            "unsupported strict-f64/min reductions must stay scalar: {discovery:#?}\n{}",
            print_kir_module(pre.module())
        );
        let result = run_kir_pass_pipeline(
            pre.module().clone(),
            KirOptimizationLevel::O3,
            contracts.as_ref(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.stats.vectorized_loops, 0);
        let text = print_kir_module(result.artifact.as_ref().expect("scalar O3 artifact"));
        assert!(!text.contains("vector_"), "{text}");
    }
}

#[test]
fn wasm_simd128_should_not_create_slp_only_vector_packs() {
    let source = r#"
export fn lanes(a0: i32, a1: i32, a2: i32, a3: i32,
                b0: i32, b1: i32, b2: i32, b3: i32) -> i32 {
  let p0: i32 = a0 * b0;
  let p1: i32 = a1 * b1;
  let p2: i32 = a2 * b2;
  let p3: i32 = a3 * b3;
  return p0 + p1 + p2 + p3;
}
"#;
    let (pre, contracts) = wasm_map_state(source, KirWasmFeatures::Simd128);
    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.slp_packs, 0);
    let text = print_kir_module(result.artifact.as_ref().expect("scalar SLP-only artifact"));
    assert!(!text.contains("vector_"), "{text}");
}

#[test]
fn wasm_simd128_should_discover_and_independently_check_all_full_width_diamond_comparisons() {
    const COMPARISONS: [(&str, calckernel::MirCompareOp); 6] = [
        ("==", calckernel::MirCompareOp::Eq),
        ("!=", calckernel::MirCompareOp::Ne),
        ("<", calckernel::MirCompareOp::Lt),
        ("<=", calckernel::MirCompareOp::Le),
        (">", calckernel::MirCompareOp::Gt),
        (">=", calckernel::MirCompareOp::Ge),
    ];

    for (lane, lanes) in [("f64", 2), ("i32", 4), ("u32", 4)] {
        for (comparison, compare_op) in COMPARISONS {
            let source = wasm_pure_diamond_source(lane, comparison);
            let (pre, _) = wasm_map_state(&source, KirWasmFeatures::Simd128);
            let discovery = discover_vectorization_candidates(&pre);
            let candidate = discovery
                .candidates
                .iter()
                .find(|candidate| candidate.vf == lanes)
                .unwrap_or_else(|| {
                    panic!(
                        "SIMD128 should discover {lane}x{lanes} comparison `{comparison}`: {discovery:#?}\n{}",
                        print_kir_module(pre.module())
                    )
                });
            let prepared = prepare_vectorization_trial(&pre, candidate)
                .expect("pure compare/select diamond should prepare a vector trial");
            assert_eq!(
                check_vectorization_trial_independently(
                    &pre,
                    &prepared.trial,
                    &prepared.plan,
                    &prepared.charge,
                ),
                Ok(()),
                "independent checker rejected {lane} `{comparison}`"
            );
            assert!(matches!(
                prepared.plan.epilogue,
                VectorEpilogue::Scalar { .. }
            ));

            let instructions = prepared.trial.module().functions[0]
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .collect::<Vec<_>>();
            assert!(
                instructions.iter().any(|instruction| matches!(
                    instruction.kind,
                    calckernel::KirInstructionKind::VectorCompare { op, .. }
                        if op == compare_op
                )),
                "trial must preserve {lane} comparison `{comparison}`:\n{}",
                print_kir_module(prepared.trial.module())
            );
            assert!(instructions.iter().any(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorSelect { .. }
            )));
            assert!(
                instructions.iter().any(|instruction| {
                    instruction.results.iter().any(|result| {
                        matches!(
                            result.type_node,
                            calckernel::KirValueType::Mask { lanes: mask_lanes }
                                if mask_lanes == lanes
                        )
                    })
                }),
                "comparison must produce a full-width {lanes}-lane mask"
            );
        }
    }
}

#[test]
fn wasm_simd128_should_discover_and_independently_check_exact_i32x2_cast_maps() {
    for (source, source_lane, cast_name) in [
        (
            WASM_I32_CAST_MAP,
            calckernel::KirLaneType::I32,
            "i32_to_f64",
        ),
        (CAST_MAP, calckernel::KirLaneType::U32, "u32_to_f64"),
    ] {
        let (pre, _) = wasm_map_state(source, KirWasmFeatures::Simd128);
        let discovery = discover_vectorization_candidates(&pre);
        let candidate = discovery
            .candidates
            .iter()
            .find(|candidate| candidate.vf == 2)
            .unwrap_or_else(|| {
                panic!(
                    "SIMD128 should discover exact {cast_name} I32x2 cast: {discovery:#?}\n{}",
                    print_kir_module(pre.module())
                )
            });
        let prepared = prepare_vectorization_trial(&pre, candidate)
            .expect("two-lane integer-to-f64 cast should prepare");
        assert_eq!(
            check_vectorization_trial_independently(
                &pre,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Ok(())
        );
        assert!(candidate.accesses.iter().any(|access| {
            access.kind == calckernel::LoopMemoryAccessKind::Read
                && access.element_bytes == 4
                && access.base_alignment == 4
                && access.known_alignment >= 4
        }));
        assert!(candidate.operations.iter().any(|operation| {
            operation.operation == KirProfileOperation::Cast
                && operation.lane_type == source_lane
                && operation.result_lane_type == calckernel::KirLaneType::F64
        }));
        let trial_instructions = prepared.trial.module().functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .collect::<Vec<_>>();
        let source_load = trial_instructions
            .iter()
            .find_map(|instruction| match &instruction.kind {
                calckernel::KirInstructionKind::VectorLoad { access, .. }
                    if access.lane == source_lane =>
                {
                    Some(access)
                }
                _ => None,
            })
            .expect("exact two-lane integer source load");
        assert_eq!(source_load.lanes, 2);
        assert_eq!(source_load.byte_footprint, 8);
        let destination_store = trial_instructions
            .iter()
            .find_map(|instruction| match &instruction.kind {
                calckernel::KirInstructionKind::VectorStore { access, .. }
                    if access.lane == calckernel::KirLaneType::F64 =>
                {
                    Some(access)
                }
                _ => None,
            })
            .expect("two-lane f64 destination store");
        assert_eq!(destination_store.lanes, 2);
        assert_eq!(destination_store.byte_footprint, 16);
        assert!(matches!(
            prepared.plan.epilogue,
            VectorEpilogue::Scalar { .. }
        ));
        let text = print_kir_module(prepared.trial.module());
        assert!(text.contains("vector_load"), "{text}");
        let vector_cast_name = if source_lane == calckernel::KirLaneType::I32 {
            "vector_cast_i32tof64"
        } else {
            "vector_cast_u32tof64"
        };
        assert!(text.contains(vector_cast_name), "{text}");
        assert!(text.contains("vector_store"), "{text}");
        assert!(text.contains("vector<f64, 2>"), "{text}");
    }
}

#[test]
fn wasm_simd128_unknown_alias_map_should_get_one_checked_total_predicate_and_scalar_fallback() {
    let (pre, contracts) = wasm_map_state(WASM_RUNTIME_ALIAS_MAP, KirWasmFeatures::Simd128);
    let discovery = discover_vectorization_candidates(&pre);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.vf == 4 && candidate.version_predicate.is_some())
        .unwrap_or_else(|| {
            panic!(
                "Wasm SIMD128 should version one unknown-alias map: {discovery:#?}\n{}",
                print_kir_module(pre.module())
            )
        });
    let predicate = candidate
        .version_predicate
        .as_ref()
        .expect("unknown Wasm alias requires a runtime predicate");
    assert_eq!(predicate.address_bits, 32);
    assert_eq!(predicate.conjuncts.len(), 1);
    assert!(predicate.conjuncts.iter().any(|conjunct| matches!(
        conjunct,
        calckernel::VersionPredicateConjunct::AddressIntervalsDisjoint {
            left_element_bytes: 4,
            right_element_bytes: 4,
            ..
        }
    )));

    let prepared = prepare_vectorization_trial(&pre, candidate)
        .expect("unknown-alias Wasm map should prepare a versioned trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "{}",
        print_kir_module(prepared.trial.module())
    );
    assert!(matches!(
        prepared.plan.epilogue,
        VectorEpilogue::Scalar { .. }
    ));
    assert!(prepared.plan.predicates.iter().any(|predicate| matches!(
        predicate,
        calckernel::VectorPredicate::AddressNonOverlap { .. }
    )));
    let trial = print_kir_module(prepared.trial.module());
    assert!(trial.contains("version_predicate"), "{trial}");

    let result = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        contracts.as_ref(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops, 1,
        "{:#?}",
        result.vector_explanations
    );
    assert!(result.vector_explanations.iter().any(|explanation| {
        explanation.disposition == CandidateDisposition::Accepted && explanation.vf == 4
    }));
    let optimized = print_kir_module(result.artifact.as_ref().expect("versioned O3 artifact"));
    assert!(optimized.contains("version_predicate"), "{optimized}");
}

#[test]
fn wasm_simd128_should_discover_and_independently_check_modular_reductions() {
    let (pre, _) = wasm_map_state(WASM_MODULAR_REDUCTIONS, KirWasmFeatures::Simd128);
    let discovery = discover_vectorization_candidates(&pre);
    assert!(
        !discovery.candidates.is_empty(),
        "Wasm SIMD128 should discover modular reduction candidates: {discovery:#?}\n{}",
        print_kir_module(pre.module())
    );

    let mut covered = BTreeMap::new();
    for candidate in &discovery.candidates {
        let function = &pre.module().functions[candidate.function.index() as usize];
        let (expected_operation, expected_lane) = match function.name.as_str() {
            "sum_u32" | "sum_i32" => (
                KirProfileOperation::ReduceAdd,
                if function.name.ends_with("u32") {
                    calckernel::KirLaneType::U32
                } else {
                    calckernel::KirLaneType::I32
                },
            ),
            "product_u32" | "product_i32" => (
                KirProfileOperation::ReduceMultiply,
                if function.name.ends_with("u32") {
                    calckernel::KirLaneType::U32
                } else {
                    calckernel::KirLaneType::I32
                },
            ),
            other => panic!("unexpected reduction function {other}"),
        };
        let reduction = candidate
            .reduction
            .as_ref()
            .expect("reduction candidate metadata");
        assert_eq!(reduction.operation, expected_operation, "{function:?}");
        assert_eq!(reduction.lane_type, expected_lane, "{function:?}");

        let prepared = prepare_vectorization_trial(&pre, candidate)
            .expect("Wasm modular reduction trial should prepare");
        assert_eq!(
            check_vectorization_trial_independently(
                &pre,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Ok(()),
            "{}",
            print_kir_module(prepared.trial.module())
        );
        let text = print_kir_module(prepared.trial.module());
        assert!(
            text.contains("vector_reduce_modularadd")
                || text.contains("vector_reduce_modularmultiply"),
            "{text}"
        );
        covered.insert(function.name.as_str(), true);
    }
    assert_eq!(
        covered.len(),
        4,
        "discovery should include add/multiply for i32/u32"
    );
}

#[test]
fn wasm_unknown_alias_maps_should_stay_scalar_in_baseline_and_o0() {
    for features in [KirWasmFeatures::Baseline, KirWasmFeatures::Simd128] {
        let (pre, contracts) = wasm_map_state(WASM_RUNTIME_ALIAS_MAP, features);
        if features == KirWasmFeatures::Baseline {
            assert!(
                discover_vectorization_candidates(&pre)
                    .candidates
                    .is_empty()
            );
        }
        let result = run_kir_pass_pipeline(
            pre.module().clone(),
            KirOptimizationLevel::O0,
            contracts.as_ref(),
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.stats.vectorized_loops, 0);
        let text = print_kir_module(result.artifact.as_ref().expect("scalar O0 artifact"));
        assert!(!text.contains("version_predicate"), "{text}");
        assert!(!text.contains("vector_load"), "{text}");
        assert!(!text.contains("vector_store"), "{text}");
    }
}
