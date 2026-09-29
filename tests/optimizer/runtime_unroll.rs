use calckernel::{
    BoundsMode, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, OverflowMode,
    RuntimeScalarUnrollKind, SourceFile, analyze_canonical_loops, build_kir_module_with_profile,
    check, check_wasm_runtime_scalar_unroll_independently,
    discover_wasm_runtime_scalar_unroll_candidates, import_contract_facts, lower_to_mir,
    prepare_wasm_runtime_scalar_unroll_trial, run_kir_pass_pipeline, validate_kir_module,
};

const SUM: &str = r#"
export unsafe fn sum(input: slice<u32>, out: slice<u32>) -> void contract {
  requires out.len == 1;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < input.len {
    total = total + input[i];
    i = i + 1;
  }
  out[0] = total;
}
"#;

const MEMORY_TRANSFORM: &str = r#"
export unsafe fn transform(input: slice<f64>, out: slice<f64>) -> void contract {
  requires input.len == out.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  while i < input.len {
    out[i] = input[i] * 0.5 + 0.25;
    i = i + 1;
  }
}
"#;

fn wasm_state(source: &str, import_contracts: bool) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("runtime-unroll.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid KIR fixture:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Baseline);
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("WASM KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("source contract facts");
    let optimized = run_kir_pass_pipeline(
        module,
        KirOptimizationLevel::O2,
        import_contracts.then_some(&contracts),
    );
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("optimized pre-state"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified pre-state")
}

fn scalar_u32_constant(
    function: &calckernel::KirFunction,
    value: calckernel::ValueId,
) -> Option<u32> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
        .and_then(|instruction| match &instruction.kind {
            calckernel::KirInstructionKind::ConstInt { value } => value.parse().ok(),
            _ => None,
        })
}

fn discover(state: &KirVerifiedProgramState) -> Vec<calckernel::RuntimeScalarUnrollCandidate> {
    let mut candidates = Vec::new();
    for function in &state.module().functions {
        let loops = analyze_canonical_loops(function);
        assert!(
            loops.loops.iter().any(|loop_info| matches!(
                loop_info.trip_count,
                calckernel::LoopTripCount::Runtime { .. }
            )),
            "fixture function {} must retain a source-derived runtime trip count",
            function.name
        );
        candidates
            .extend(discover_wasm_runtime_scalar_unroll_candidates(state, &loops.loops).candidates);
    }
    candidates
}

#[test]
fn baseline_runtime_unroll_should_recognize_only_source_proven_sum_and_strict_map_shapes() {
    let sum = wasm_state(SUM, true);
    let sum_candidates = discover(&sum);
    assert_eq!(sum_candidates.len(), 1, "{sum_candidates:#?}");
    assert_eq!(
        sum_candidates[0].kind,
        RuntimeScalarUnrollKind::U32ModularSum
    );
    assert_eq!(sum_candidates[0].factor, 4);

    let map = wasm_state(MEMORY_TRANSFORM, true);
    let map_candidates = discover(&map);
    assert_eq!(map_candidates.len(), 1, "{map_candidates:#?}");
    assert_eq!(
        map_candidates[0].kind,
        RuntimeScalarUnrollKind::StrictF64DirectMap
    );
    assert_eq!(map_candidates[0].factor, 4);
}

#[test]
fn baseline_runtime_unroll_should_materialize_a_checked_main_loop_and_original_scalar_tail() {
    for (source, expected_kind) in [
        (SUM, RuntimeScalarUnrollKind::U32ModularSum),
        (
            MEMORY_TRANSFORM,
            RuntimeScalarUnrollKind::StrictF64DirectMap,
        ),
    ] {
        let state = wasm_state(source, true);
        let candidate = discover(&state)
            .into_iter()
            .find(|candidate| candidate.kind == expected_kind)
            .expect("source-proven scalar UF4 candidate");
        let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate)
            .expect("runtime scalar UF4 materialization");
        assert_eq!(prepared.plan.factor, 4);
        assert_eq!(prepared.plan.kind, expected_kind);
        assert!(
            validate_kir_module(prepared.trial.module())
                .errors
                .is_empty(),
            "materialized KIR must validate independently"
        );
        check_wasm_runtime_scalar_unroll_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .expect("independent scalar UF4 reconstruction");
        let transformed = prepared
            .trial
            .module()
            .functions
            .iter()
            .find(|function| function.id == candidate.function)
            .expect("transformed function");
        let loops = analyze_canonical_loops(transformed).loops;
        assert!(
            loops
                .iter()
                .any(|loop_info| loop_info.header == candidate.header),
            "the untouched source loop remains as the scalar remainder"
        );
        assert!(
            loops
                .iter()
                .any(|loop_info| loop_info.header == prepared.plan.main_header),
            "the independently checked four-item main loop is present"
        );
    }
}

#[test]
fn runtime_unroll_should_address_each_lane_from_the_group_base_and_advance_by_four() {
    let state = wasm_state(SUM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::U32ModularSum)
        .expect("source-proven sum");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let source_function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("source function");
    let source_header = source_function
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .expect("source header");
    let source_then = match &source_header.terminator {
        calckernel::KirTerminator::Branch { then_edge, .. } => then_edge,
        _ => panic!("source header branches to its body"),
    };
    let induction_index = source_header
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.induction)
        .expect("source induction parameter");
    let body_induction_index = source_then
        .args
        .iter()
        .position(|value| *value == candidate.induction)
        .expect("body induction parameter");

    let transformed = prepared
        .trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("transformed function");
    let grouped_header = transformed
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.main_header)
        .expect("group header");
    let grouped_body = transformed
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.main_body)
        .expect("group body");
    let group_base = grouped_body.params[body_induction_index].value;
    let lane_loads = grouped_body
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            calckernel::KirInstructionKind::Load { place } => match place.as_ref() {
                calckernel::KirPlace::SliceIndex { index, .. } => Some(*index),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lane_loads.len(), 4, "sum fast path has four ordered loads");
    assert_eq!(lane_loads[0], group_base);
    for (lane, index) in lane_loads.iter().enumerate().skip(1) {
        let definition = grouped_body
            .instructions
            .iter()
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == *index)
            })
            .expect("independent lane index definition");
        assert!(
            matches!(
                &definition.kind,
                calckernel::KirInstructionKind::Binary {
                    op: calckernel::MirBinaryOp::Add,
                    left,
                    right,
                    semantics: calckernel::KirArithmeticSemantics::Modular,
                } if *left == group_base && scalar_u32_constant(transformed, *right) == Some(lane as u32)
            ),
            "lane {lane} address must be group-base + {lane}, not chained from the prior lane"
        );
    }
    let calckernel::KirTerminator::Jump { edge } = &grouped_body.terminator else {
        panic!("group body has a backedge")
    };
    let group_next_induction = edge.args[induction_index];
    let group_step = grouped_body
        .instructions
        .iter()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == group_next_induction)
        })
        .expect("group induction step");
    assert!(
        matches!(
            &group_step.kind,
            calckernel::KirInstructionKind::Binary {
                op: calckernel::MirBinaryOp::Add,
                left,
                right,
                semantics: calckernel::KirArithmeticSemantics::Modular,
            } if *left == group_base && scalar_u32_constant(transformed, *right) == Some(4)
        ),
        "group backedge advances once by four from the original induction value"
    );
    assert_eq!(
        grouped_header.params[induction_index].type_node,
        source_header.params[induction_index].type_node,
        "group induction retains its exact source type"
    );
}

#[test]
fn independent_checker_should_reject_valid_kir_with_a_chained_lane_offset() {
    let state = wasm_state(SUM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::U32ModularSum)
        .expect("source-proven sum");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let mut forged = prepared.trial.clone();
    let transformed = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("function");
    let body = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.main_body)
        .expect("grouped body");
    let first_offset = prepared.plan.lane_induction_values[1];
    let one = body
        .instructions
        .iter()
        .find(|instruction| instruction.id == prepared.plan.induction_aux_instruction_ids[0])
        .expect("offset one constant")
        .results[0]
        .value;
    let second_lane = body
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == prepared.plan.lane_index_instruction_ids[1])
        .expect("offset two addition");
    let calckernel::KirInstructionKind::Binary { left, right, .. } = &mut second_lane.kind else {
        panic!("expected lane index addition")
    };
    *left = first_offset;
    *right = one;

    assert!(
        validate_kir_module(forged.module()).errors.is_empty(),
        "chained modular address remains valid KIR"
    );
    assert!(
        matches!(
            check_wasm_runtime_scalar_unroll_independently(
                &state,
                &forged,
                &prepared.plan,
                &prepared.charge,
            ),
            Err(calckernel::TransactionCheckError::Reject(_))
        ),
        "checker must require each lane address to be derived directly from the group base"
    );
}

#[test]
fn independent_checker_should_reject_valid_kir_with_a_forged_group_stride() {
    let state = wasm_state(SUM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::U32ModularSum)
        .expect("source-proven sum");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let mut forged = prepared.trial.clone();
    let transformed = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("function");
    let body = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.main_body)
        .expect("grouped body");
    let stride = body
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == prepared.plan.induction_aux_instruction_ids[6])
        .expect("group stride constant");
    let calckernel::KirInstructionKind::ConstInt { value } = &mut stride.kind else {
        panic!("expected group stride constant")
    };
    *value = "3".to_string();

    assert!(
        validate_kir_module(forged.module()).errors.is_empty(),
        "wrong but nontrapping group stride remains valid KIR"
    );
    assert!(
        matches!(
            check_wasm_runtime_scalar_unroll_independently(
                &state,
                &forged,
                &prepared.plan,
                &prepared.charge,
            ),
            Err(calckernel::TransactionCheckError::Reject(_))
        ),
        "checker must prove that the single grouped recurrence advances by four"
    );
}

#[test]
fn baseline_runtime_map_should_require_an_explicit_source_noalias_contract() {
    let without_noalias = MEMORY_TRANSFORM.replace("  requires noalias(input, out);\n", "");
    let state = wasm_state(&without_noalias, true);
    assert!(discover(&state).is_empty());
}

#[test]
fn unsupported_valid_wasm_map_should_keep_the_o3_scalar_artifact() {
    let without_noalias = MEMORY_TRANSFORM.replace("  requires noalias(input, out);\n", "");
    let result = super::support::compiler::optimized_module(
        &without_noalias,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    assert!(
        result.errors.is_empty(),
        "optional UF4 must not fail a valid source program"
    );
    assert!(
        result.artifact.is_some(),
        "the original scalar implementation remains compilable"
    );
    assert!(
        !result.audit.attempts().iter().any(|attempt| {
            matches!(
                attempt.key,
                calckernel::CandidateKey::LoopFrontier {
                    kind: calckernel::LoopCandidateKind::RuntimeScalarUnroll,
                    ..
                }
            ) && attempt.disposition == calckernel::CandidateDisposition::Accepted
        }),
        "without source noalias evidence the compiler must keep the scalar loop"
    );
}

#[test]
fn independent_checker_should_reject_valid_kir_with_a_lane_recurrence_mutation() {
    let state = wasm_state(MEMORY_TRANSFORM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::StrictF64DirectMap)
        .expect("source-proven direct map");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let mut forged = prepared.trial.clone();
    let body_id = prepared.plan.main_body;
    let lane_len = prepared.plan.lane_instruction_ids[0].len();
    let first_load_id = prepared.plan.lane_instruction_ids[0][0];
    let second_lane_load_id = prepared.plan.lane_instruction_ids[1][0];
    let transformed = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("function");
    let body = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == body_id)
        .expect("UF4 body");
    let first_index = match &body
        .instructions
        .iter()
        .find(|instruction| instruction.id == first_load_id)
        .unwrap()
        .kind
    {
        calckernel::KirInstructionKind::Load { place } => match place.as_ref() {
            calckernel::KirPlace::SliceIndex { index, .. } => *index,
            _ => panic!("expected slice load"),
        },
        _ => panic!("expected load"),
    };
    let second_lane_load = body
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == second_lane_load_id)
        .expect("lane one load");
    let calckernel::KirInstructionKind::Load { place } = &mut second_lane_load.kind else {
        panic!("expected load")
    };
    let calckernel::KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("expected slice load")
    };
    *index = first_index;

    assert_eq!(
        lane_len, 6,
        "each transformed map lane contains load, multiply, add and store chain instructions"
    );
    assert!(
        validate_kir_module(forged.module()).errors.is_empty(),
        "mutation remains structurally valid KIR"
    );
    assert!(
        matches!(
            check_wasm_runtime_scalar_unroll_independently(
                &state,
                &forged,
                &prepared.plan,
                &prepared.charge,
            ),
            Err(calckernel::TransactionCheckError::Reject(_))
        ),
        "checker must reconstruct each lane's exact source induction value"
    );
}

#[test]
fn independent_checker_should_reject_valid_kir_with_a_forged_nontrapping_limit() {
    let state = wasm_state(SUM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::U32ModularSum)
        .expect("source-proven sum");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let mut forged = prepared.trial.clone();
    let transformed = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("function");
    let fast = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_entry)
        .expect("fast entry");
    let constant = fast.instructions.first_mut().expect("remainder constant");
    let calckernel::KirInstructionKind::ConstInt { value } = &mut constant.kind else {
        panic!("expected integer constant")
    };
    *value = "2".to_string();

    assert!(
        validate_kir_module(forged.module()).errors.is_empty(),
        "nontrapping mutation remains valid KIR"
    );
    assert!(
        check_wasm_runtime_scalar_unroll_independently(
            &state,
            &forged,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker must prove the limit uses the actual UF4 remainder"
    );
}

#[test]
fn independent_checker_should_reject_valid_kir_with_a_stale_memory_backedge() {
    let state = wasm_state(MEMORY_TRANSFORM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::StrictF64DirectMap)
        .expect("source-proven direct map");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let mut forged = prepared.trial.clone();
    let transformed = forged
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("function");
    let grouped_header = transformed
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.main_header)
        .expect("grouped header");
    let output_memory_index = grouped_header
        .memory_params
        .iter()
        .position(|parameter| Some(parameter.region) == candidate.output_region)
        .expect("output memory partition");
    let grouped_body = transformed
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.main_body)
        .expect("grouped body");
    let calckernel::KirTerminator::Jump { edge } = &mut grouped_body.terminator else {
        panic!("expected grouped backedge");
    };
    edge.memory_args[output_memory_index] = grouped_body.memory_params[output_memory_index].version;

    assert!(
        validate_kir_module(forged.module()).errors.is_empty(),
        "stale but same-region backedge remains structurally valid KIR"
    );
    assert!(
        check_wasm_runtime_scalar_unroll_independently(
            &state,
            &forged,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker must require the grouped backedge to carry lane-four store state"
    );
}

#[test]
fn independent_checker_should_reject_noalias_without_trusted_contract_provenance() {
    let state = wasm_state(MEMORY_TRANSFORM, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::StrictF64DirectMap)
        .expect("source-proven direct map");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let fact_id = candidate
        .noalias_fact
        .expect("trusted noalias contract fact");
    let mut forged_pre = state.clone();
    let mut forged_trial = prepared.trial.clone();
    for forged in [&mut forged_pre, &mut forged_trial] {
        let fact = forged
            .contract_facts_mut()
            .and_then(|contracts| contracts.facts_mut().get_mut(fact_id))
            .expect("noalias contract fact");
        fact.origin = calckernel::FactOrigin::Proven;
    }

    assert!(validate_kir_module(forged_pre.module()).errors.is_empty());
    assert!(validate_kir_module(forged_trial.module()).errors.is_empty());
    assert!(
        matches!(
            check_wasm_runtime_scalar_unroll_independently(
                &forged_pre,
                &forged_trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Err(calckernel::TransactionCheckError::Reject(_))
        ),
        "checker must require noalias to originate in a trusted source contract"
    );
}

#[test]
fn independent_checker_should_reject_noalias_borrowed_from_another_function_contract() {
    let source = format!("{SUM}\n{MEMORY_TRANSFORM}");
    let state = wasm_state(&source, true);
    let candidate = discover(&state)
        .into_iter()
        .find(|candidate| candidate.kind == RuntimeScalarUnrollKind::StrictF64DirectMap)
        .expect("source-proven direct map");
    let prepared = prepare_wasm_runtime_scalar_unroll_trial(&state, &candidate).expect("UF4 trial");
    let fact_id = candidate
        .noalias_fact
        .expect("trusted noalias contract fact");
    let mut forged_pre = state.clone();
    let mut forged_trial = prepared.trial.clone();
    for forged in [&mut forged_pre, &mut forged_trial] {
        let fact = forged
            .contract_facts_mut()
            .and_then(|contracts| contracts.facts_mut().get_mut(fact_id))
            .expect("noalias contract fact");
        fact.origin = calckernel::FactOrigin::TrustedContract {
            instance: calckernel::ContractInstanceId::from_index(0),
        };
    }

    assert!(validate_kir_module(forged_pre.module()).errors.is_empty());
    assert!(validate_kir_module(forged_trial.module()).errors.is_empty());
    assert!(
        matches!(
            check_wasm_runtime_scalar_unroll_independently(
                &forged_pre,
                &forged_trial,
                &prepared.plan,
                &prepared.charge,
            ),
            Err(calckernel::TransactionCheckError::Reject(_))
        ),
        "checker must bind NoAlias to this transform function's own contract instance"
    );
}
