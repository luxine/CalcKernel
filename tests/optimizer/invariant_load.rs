use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile,
    build_kir_module_with_profile, check, check_wasm_invariant_load_trial_independently,
    discover_wasm_invariant_load_candidates, emit_wasm_kir_module, emit_wat_kir_module,
    import_contract_facts, lower_to_mir, prepare_wasm_invariant_load_trial, run_kir_pass_pipeline,
};
use std::{fs, process::Command};

const MATMUL: &str = r#"
export unsafe fn matmul(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void contract {
  requires n <= a.len && n <= b.len && n <= out.len;
  requires a.len == b.len && b.len == out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
} {
  let i: u32 = 0;
  while i < out.len {
    out[i] = 0.0;
    i = i + 1;
  }
  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        out[out_index] = out[out_index] + a[a_index] * b[b_index];
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

fn baseline_state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("invariant-load.ck", source));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
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
    .expect("KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).expect("contracts");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("O2 KIR"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified O2 KIR")
}

fn baseline_o3_result(source: &str) -> calckernel::KirPassManagerResult {
    let checked = check(&SourceFile::new("invariant-load-o3.ck", source));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
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
    .expect("KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).expect("contracts");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    assert!(
        optimized.artifact.is_some(),
        "O3 must produce a verified artifact"
    );
    optimized
}

#[test]
fn discovery_should_find_the_a_load_in_the_innermost_matmul_loop() {
    let state = baseline_state(MATMUL);
    let candidates = discover_wasm_invariant_load_candidates(&state);
    assert_eq!(
        candidates.candidates.len(),
        1,
        "{:#?}",
        candidates.fallbacks
    );
    assert!(matches!(
        &candidates.candidates[0].key,
        calckernel::CandidateKey::LoopFrontier {
            kind: calckernel::LoopCandidateKind::WasmInvariantLoad,
            variant: calckernel::LoopCandidateVariant::Scalar,
            vf: 1,
            uf: 8,
            ..
        }
    ));
}

#[test]
fn discovery_should_require_the_source_noalias_contract() {
    let source = MATMUL.replace(" && noalias(a, out)", "");
    let state = baseline_state(&source);
    let candidates = discover_wasm_invariant_load_candidates(&state);
    assert!(candidates.candidates.is_empty());
}

#[test]
fn proposer_should_reject_nonzero_start_and_nonunit_induction_step() {
    for source in [
        MATMUL.replace("let col: u32 = 0;", "let col: u32 = 1;"),
        MATMUL.replace("col = col + 1;", "col = col + 2;"),
    ] {
        let state = baseline_state(&source);
        let discovery = discover_wasm_invariant_load_candidates(&state);
        assert!(
            discovery.candidates.is_empty(),
            "candidate must require zero-based unit-stride induction: {:#?}",
            discovery.candidates
        );
    }
}

#[test]
fn frozen_style_matmul_o3_pipeline_should_accept_and_emit_invariant_load_candidate() {
    let o2_state = baseline_state(MATMUL);
    let o2_candidates = discover_wasm_invariant_load_candidates(&o2_state);
    assert_eq!(o2_candidates.candidates.len(), 1, "O2 source candidate");
    let result = baseline_o3_result(MATMUL);
    let accepted = result.audit.attempts().iter().any(|attempt| {
        matches!(
            attempt.key,
            calckernel::CandidateKey::LoopFrontier {
                kind: calckernel::LoopCandidateKind::WasmInvariantLoad,
                ..
            }
        ) && attempt.disposition == calckernel::CandidateDisposition::Accepted
    });
    assert!(
        accepted,
        "invariant-load audit: {}\npasses={:?}\nfallbacks={:?}",
        calckernel::print_optimization_audit(&result.audit),
        result
            .records
            .iter()
            .map(|record| record.name.as_str())
            .collect::<Vec<_>>(),
        result.analysis_fallbacks,
    );
    let artifact = result.artifact.as_ref().expect("verified O3 artifact");
    let emitted = emit_wasm_kir_module(artifact, calckernel::EmitWasmOptions { opt_level: 3 })
        .expect("pipeline artifact emits as baseline Wasm");
    assert_eq!(&emitted[..4], b"\0asm");
}

#[test]
fn materialized_trial_should_pass_independent_reconstruction() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");
    assert!(
        calckernel::validate_kir_module(prepared.trial.module())
            .errors
            .is_empty(),
        "trial KIR must be structurally valid: {:?}",
        calckernel::validate_kir_module(prepared.trial.module()).errors
    );
    check_wasm_invariant_load_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect("independent check");
}

#[test]
fn materialized_fast_path_should_stripmine_eight_scalar_columns() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");
    let fast_body = prepared
        .trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .and_then(|function| {
            function
                .blocks
                .iter()
                .find(|block| block.id == prepared.plan.fast_body)
        })
        .expect("guarded fast body");
    let stores = fast_body
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Store { .. }
            )
        })
        .count();
    assert_eq!(
        stores, 8,
        "fast loop should contain eight ordered scalar updates"
    );
}

#[test]
fn baseline_wasm_should_emit_the_guard_in_wat_and_direct_binary_at_o0_and_o3() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");
    for opt_level in [0, 3] {
        let wat = emit_wat_kir_module(
            prepared.trial.module(),
            calckernel::EmitWasmOptions { opt_level },
        )
        .expect("baseline WAT with total range guard");
        assert!(wat.contains("i64.extend_i32_u"), "{opt_level}");
        let binary = emit_wasm_kir_module(
            prepared.trial.module(),
            calckernel::EmitWasmOptions { opt_level },
        )
        .expect("baseline direct binary with total range guard");
        assert_eq!(&binary[..4], b"\0asm");
    }
}

#[test]
fn independent_checker_should_reject_mutated_threshold_index_noalias_and_memory_map() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");

    let mut bad_threshold = prepared.trial.clone();
    mutate_guard(&mut bad_threshold, candidate.preheader, |predicate| {
        let threshold = predicate
            .conjuncts
            .iter_mut()
            .find_map(|conjunct| match conjunct {
                calckernel::KirVersionPredicateConjunct::TripThreshold { minimum, .. } => {
                    Some(minimum)
                }
                _ => None,
            })
            .expect("trip guard");
        *threshold += 1;
    });
    assert!(
        calckernel::validate_kir_module(bad_threshold.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_threshold,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_index = prepared.trial.clone();
    mutate_guard(&mut bad_index, candidate.preheader, |predicate| {
        let start = predicate
            .conjuncts
            .iter_mut()
            .find_map(|conjunct| match conjunct {
                calckernel::KirVersionPredicateConjunct::WasmSliceRange { start, .. } => {
                    Some(start)
                }
                _ => None,
            })
            .expect("range guard");
        *start = prepared.plan.count_one;
    });
    assert!(
        calckernel::validate_kir_module(bad_index.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_index,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_output_range = prepared.trial.clone();
    mutate_guard(&mut bad_output_range, candidate.preheader, |predicate| {
        let output_count = predicate
            .conjuncts
            .iter_mut()
            .find_map(|conjunct| match conjunct {
                calckernel::KirVersionPredicateConjunct::WasmSliceRange {
                    slice, count, ..
                } if *slice == candidate.output_slice => Some(count),
                _ => None,
            })
            .expect("output range guard");
        *output_count = prepared.plan.count_one;
    });
    assert!(
        calckernel::validate_kir_module(bad_output_range.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_output_range,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let replacement_noalias = state
        .contract_facts()
        .expect("contract facts")
        .facts()
        .facts()
        .iter()
        .find(|fact| {
            fact.id != candidate.noalias_fact
                && matches!(
                    fact.predicate,
                    calckernel::FactPredicate::Contract(
                        calckernel::ContractFactPredicate::NoAlias { .. }
                    )
                )
        })
        .expect("another trusted noalias fact")
        .id;
    let mut bad_noalias_plan = prepared.plan.clone();
    bad_noalias_plan.candidate.noalias_fact = replacement_noalias;
    bad_noalias_plan.noalias_fact = replacement_noalias;
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &prepared.trial,
            &bad_noalias_plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_memory_plan = prepared.plan.clone();
    bad_memory_plan.memory_mapping.pop();
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &prepared.trial,
            &bad_memory_plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_memory_ssa = prepared.trial.clone();
    let function = bad_memory_ssa
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let b_load = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("source function")
        .blocks
        .iter()
        .find(|block| block.id == candidate.body)
        .expect("source body")
        .instructions
        .iter()
        .filter_map(|instruction| {
            let calckernel::KirInstructionKind::Load { place } = &instruction.kind else {
                return None;
            };
            let calckernel::KirPlace::SliceIndex { slice, .. } = place.as_ref() else {
                return None;
            };
            if *slice != candidate.input_slice && *slice != candidate.output_slice {
                instruction
                    .memory
                    .as_ref()
                    .map(|memory| (instruction.id, memory.region))
            } else {
                None
            }
        })
        .next()
        .expect("B source load");
    let wrong_version = function
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.fast_header)
        .expect("fast header")
        .memory_params
        .iter()
        .find(|parameter| parameter.region == b_load.1)
        .expect("fast header B partition")
        .version;
    let cloned_b_load = prepared
        .plan
        .body_instruction_map
        .iter()
        .filter_map(|(source, cloned)| (*source == b_load.0).then_some(*cloned))
        .nth(4)
        .expect("fifth cloned B load");
    let cloned_b_load = function
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_body)
        .expect("fast body")
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == cloned_b_load)
        .expect("cloned B instruction");
    cloned_b_load
        .memory
        .as_mut()
        .expect("MemorySSA access")
        .input = wrong_version;
    assert!(
        calckernel::validate_kir_module(bad_memory_ssa.module())
            .errors
            .is_empty(),
        "same-partition dominating MemorySSA mutation should be valid KIR"
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_memory_ssa,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );
}

#[test]
fn independent_checker_should_reject_changed_lane_step_and_scalar_tail_edge() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");

    let mut bad_lane = prepared.trial.clone();
    let source = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("source function")
        .blocks
        .iter()
        .find(|block| block.id == candidate.body)
        .expect("source loop body");
    let b_load = source
        .instructions
        .iter()
        .find_map(|instruction| {
            let calckernel::KirInstructionKind::Load { place } = &instruction.kind else {
                return None;
            };
            let calckernel::KirPlace::SliceIndex { slice, .. } = place.as_ref() else {
                return None;
            };
            (*slice != candidate.input_slice && *slice != candidate.output_slice)
                .then_some(instruction.id)
        })
        .expect("B source load");
    let lane_four_b = prepared
        .plan
        .body_instruction_map
        .iter()
        .filter_map(|(source, cloned)| (*source == b_load).then_some(*cloned))
        .nth(4)
        .expect("fifth B load clone");
    let lane_block = bad_lane
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function")
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_body)
        .expect("fast body");
    let lane_b = lane_block
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == lane_four_b)
        .expect("lane-four B load");
    let calckernel::KirInstructionKind::Load { place } = &mut lane_b.kind else {
        panic!("lane-four B clone remains a load");
    };
    let calckernel::KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("lane-four B clone remains slice indexed");
    };
    *index = prepared.plan.lane_induction_values[0];
    assert!(
        calckernel::validate_kir_module(bad_lane.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_lane,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_step = prepared.trial.clone();
    let step_constant = bad_step
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function")
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_body)
        .expect("fast body")
        .instructions
        .iter_mut()
        .find(|instruction| {
            instruction
                .results
                .first()
                .is_some_and(|result| result.value == prepared.plan.fast_induction_step_constant)
        })
        .expect("fast step constant");
    step_constant.kind = calckernel::KirInstructionKind::ConstInt { value: "7".into() };
    assert!(
        calckernel::validate_kir_module(bad_step.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_step,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_tail = prepared.trial.clone();
    let fast_header = bad_tail
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function")
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_header)
        .expect("fast header");
    let induction_index = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("source function")
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .expect("source header")
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.induction)
        .expect("source induction parameter");
    let calckernel::KirTerminator::Branch { else_edge, .. } = &mut fast_header.terminator else {
        panic!("fast header remains a branch");
    };
    else_edge.args[induction_index] = prepared.plan.bulk_limit;
    assert!(
        calckernel::validate_kir_module(bad_tail.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_tail,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_offset = prepared.trial.clone();
    let offset = bad_offset
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function")
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_entry)
        .expect("fast entry")
        .instructions
        .iter_mut()
        .find(|instruction| {
            instruction
                .results
                .first()
                .is_some_and(|result| result.value == prepared.plan.bulk_limit_offset)
        })
        .expect("bulk-limit offset");
    offset.kind = calckernel::KirInstructionKind::ConstInt { value: "6".into() };
    assert!(
        calckernel::validate_kir_module(bad_offset.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_offset,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_header = prepared.trial.clone();
    let source_header = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("source function")
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .expect("source header");
    let bound_index = source_header
        .params
        .iter()
        .position(|parameter| parameter.value == candidate.bound);
    let fast_bound = bound_index.map_or(candidate.bound, |index| {
        prepared
            .trial
            .module()
            .functions
            .iter()
            .find(|function| function.id == candidate.function)
            .expect("matmul function")
            .blocks
            .iter()
            .find(|block| block.id == prepared.plan.fast_header)
            .expect("fast header")
            .params[index]
            .value
    });
    let compare_clone = prepared
        .plan
        .header_instruction_map
        .iter()
        .find_map(|(source, clone)| (*source == source_header.instructions[0].id).then_some(*clone))
        .expect("fast header compare clone");
    let compare = bad_header
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function")
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_header)
        .expect("fast header")
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == compare_clone)
        .expect("fast header compare");
    let calckernel::KirInstructionKind::Compare { right, .. } = &mut compare.kind else {
        panic!("fast header compare remains a compare");
    };
    *right = fast_bound;
    assert!(
        calckernel::validate_kir_module(bad_header.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_header,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_minimum = prepared.trial.clone();
    mutate_guard(&mut bad_minimum, candidate.preheader, |predicate| {
        let minimum = predicate
            .conjuncts
            .iter_mut()
            .find_map(|conjunct| match conjunct {
                calckernel::KirVersionPredicateConjunct::TripThreshold { minimum, .. } => {
                    Some(minimum)
                }
                _ => None,
            })
            .expect("trip threshold");
        *minimum = 7;
    });
    assert!(
        calckernel::validate_kir_module(bad_minimum.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_minimum,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );

    let mut bad_hoisted_order = prepared.trial.clone();
    let hoisted = bad_hoisted_order
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function")
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_entry)
        .expect("fast entry")
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == prepared.plan.hoisted_load)
        .expect("hoisted A load");
    hoisted.effect.as_mut().expect("load effect").order = u32::MAX;
    assert!(
        calckernel::validate_kir_module(bad_hoisted_order.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_hoisted_order,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );
}

#[test]
fn independent_checker_should_reject_changed_scalar_fallback() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");
    let mut bad_fallback = prepared.trial.clone();
    let function = bad_fallback
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let source_load = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == candidate.load)
        .expect("fallback A load");
    let calckernel::KirInstructionKind::Load { place } = &mut source_load.kind else {
        panic!("source A load is still a load");
    };
    let calckernel::KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("source A load is still slice indexed");
    };
    *index = candidate.induction;
    assert!(
        calckernel::validate_kir_module(bad_fallback.module())
            .errors
            .is_empty()
    );
    assert!(
        check_wasm_invariant_load_trial_independently(
            &state,
            &bad_fallback,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );
}

#[test]
fn baseline_backend_should_reject_non_whitelisted_version_predicates() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");

    let mut address_guard = prepared.trial.clone();
    mutate_guard(&mut address_guard, candidate.preheader, |predicate| {
        predicate.conjuncts = vec![
            calckernel::KirVersionPredicateConjunct::TripThreshold {
                value: predicate
                    .conjuncts
                    .iter()
                    .find_map(|conjunct| match conjunct {
                        calckernel::KirVersionPredicateConjunct::TripThreshold {
                            value, ..
                        } => Some(*value),
                        _ => None,
                    })
                    .expect("trip guard"),
                minimum: candidate.minimum_trip,
            },
            calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                left: candidate.input_slice,
                left_count: prepared.plan.count_one,
                left_element_bytes: 8,
                right: candidate.output_slice,
                right_count: prepared.plan.count_one,
                right_element_bytes: 8,
            },
        ];
    });
    assert!(
        emit_wat_kir_module(
            address_guard.module(),
            calckernel::EmitWasmOptions { opt_level: 0 },
        )
        .is_err()
    );

    let mut wrong_count = prepared.trial.clone();
    let function = wrong_count
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let count = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("preheader")
        .instructions
        .iter_mut()
        .find(|instruction| {
            instruction
                .results
                .first()
                .is_some_and(|result| result.value == prepared.plan.count_one)
        })
        .expect("one-element count constant");
    count.kind = calckernel::KirInstructionKind::ConstInt { value: "2".into() };
    assert!(
        calckernel::validate_kir_module(wrong_count.module())
            .errors
            .is_empty()
    );
    assert!(
        emit_wat_kir_module(
            wrong_count.module(),
            calckernel::EmitWasmOptions { opt_level: 0 },
        )
        .is_err()
    );

    let mut wrong_width = prepared.trial.clone();
    mutate_guard(&mut wrong_width, candidate.preheader, |predicate| {
        let width = predicate
            .conjuncts
            .iter_mut()
            .find_map(|conjunct| match conjunct {
                calckernel::KirVersionPredicateConjunct::WasmSliceRange {
                    element_bytes, ..
                } => Some(element_bytes),
                _ => None,
            })
            .expect("range guard");
        *width = 4;
    });
    assert!(
        emit_wat_kir_module(
            wrong_width.module(),
            calckernel::EmitWasmOptions { opt_level: 0 },
        )
        .is_err()
    );
}

fn mutate_guard(
    state: &mut KirVerifiedProgramState,
    preheader: calckernel::BlockId,
    mutate: impl FnOnce(&mut calckernel::KirVersionPredicate),
) {
    let function = state
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.blocks.iter().any(|block| block.id == preheader))
        .expect("guard function");
    let guard = function
        .blocks
        .iter_mut()
        .find(|block| block.id == preheader)
        .expect("preheader")
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VersionPredicate { predicate } => Some(predicate),
            _ => None,
        })
        .expect("version predicate");
    mutate(guard);
}

#[test]
fn node_should_match_frozen_matmul_bytes_for_fast_small_and_physical_end_cases() {
    let state = baseline_state(MATMUL);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("matmul invariant load");
    assert!(
        candidate.minimum_trip >= 8 && candidate.minimum_trip <= 64,
        "unexpectedly large threshold for focused runtime fixture: {}",
        candidate.minimum_trip
    );
    let aligned_base = candidate.minimum_trip.div_ceil(8) * 8;
    let fast_cases = (0..8).map(|tail| aligned_base + tail).collect::<Vec<_>>();
    let reference =
        emit_wasm_kir_module(state.module(), calckernel::EmitWasmOptions { opt_level: 3 })
            .expect("reference direct binary");
    let production = baseline_o3_result(MATMUL);
    assert!(production.audit.attempts().iter().any(|attempt| {
        matches!(
            attempt.key,
            calckernel::CandidateKey::LoopFrontier {
                kind: calckernel::LoopCandidateKind::WasmInvariantLoad,
                ..
            }
        ) && attempt.disposition == calckernel::CandidateDisposition::Accepted
    }));
    let guarded = emit_wasm_kir_module(
        production.artifact.as_ref().expect("verified O3 candidate"),
        calckernel::EmitWasmOptions { opt_level: 3 },
    )
    .expect("production O3 direct binary");
    let runner = r#"
const fs = require("node:fs");
async function execute(path, n, physicalEnd) {
  const {instance} = await WebAssembly.instantiate(fs.readFileSync(path));
  const memory = instance.exports.memory;
  memory.grow(2);
  const view = new DataView(memory.buffer);
  const a = physicalEnd ? memory.buffer.byteLength - 4 : 1024;
  const b = 65536;
  const out = 131072;
  const length = n * n;
  if (!physicalEnd) {
    for (let i = 0; i < length; i++) {
      view.setFloat64(a + i * 8, ((i * 7) % 13 - 6) / 4, true);
      view.setFloat64(b + i * 8, ((i * 5) % 11 - 4) / 8, true);
    }
  }
  for (let i = 0; i < length; i++) view.setFloat64(out + i * 8, 123.5, true);
  let status = "ok";
  try { instance.exports.matmul(a, length, b, length, out, length, n); }
  catch (_) { status = "trap"; }
  const bytes = Buffer.from(new Uint8Array(memory.buffer, out, length * 8)).toString("hex");
  return `${status}:${bytes}`;
}
async function compare(n, physicalEnd) {
  const before = await execute(process.argv[2], n, physicalEnd);
  const after = await execute(process.argv[3], n, physicalEnd);
  if (before !== after) throw new Error(`n=${n} physical=${physicalEnd}: ${before} != ${after}`);
}
(async () => {
  const values = process.argv[4].split(",").map(Number);
  const smallCases = values.slice(0, 8);
  const smallN = values[8];
  const fastCases = values.slice(9);
  for (const n of smallCases) await compare(n, false);
  for (const n of fastCases) await compare(n, false);
  await compare(fastCases[0], true);
  await compare(smallN, false);
  process.stdout.write("matmul-byte-parity");
})().catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let small_n = candidate.minimum_trip.saturating_sub(1);
    let argument = (0..8)
        .chain(std::iter::once(small_n))
        .chain(fast_cases)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let Some(actual) = run_node_pair(&reference, &guarded, &argument, runner) else {
        return;
    };
    assert_eq!(actual, "matmul-byte-parity");
}

const WRAPPING_PROBE: &str = r#"
export unsafe fn wrapped_row(a: slice<f64>, b: slice<f64>, out: slice<f64>, row: u32, n: u32, inner: u32, width: u32, shift: u32) -> void contract {
  requires 1 <= a.len;
  requires width <= b.len && width <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
} {
  let col: u32 = 0;
  while col < width {
    out[col + shift] = out[col + shift] + a[row * n + inner] * b[inner * width + col];
    col = col + 1;
  }
}
"#;

#[test]
fn node_should_preserve_u32_wrapping_index_in_the_guarded_fast_path() {
    let state = baseline_state(WRAPPING_PROBE);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("wrapped row invariant load");
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");
    let width = candidate.minimum_trip.max(4);
    assert!(
        width <= 128,
        "unexpected wrapping probe trip threshold {width}"
    );
    let reference =
        emit_wasm_kir_module(state.module(), calckernel::EmitWasmOptions { opt_level: 3 })
            .expect("reference direct binary");
    let guarded = emit_wasm_kir_module(
        prepared.trial.module(),
        calckernel::EmitWasmOptions { opt_level: 3 },
    )
    .expect("guarded direct binary");
    let runner = r#"
const fs = require("node:fs");
async function execute(path, width) {
  const {instance} = await WebAssembly.instantiate(fs.readFileSync(path));
  const view = new DataView(instance.exports.memory.buffer);
  const a = 1024, b = 4096, out = 8192;
  view.setFloat64(a, 1.25, true);
  for (let i = 0; i < width; i++) {
    view.setFloat64(b + i * 8, (i - 3) / 4, true);
    view.setFloat64(out + i * 8, (i + 2) / 8, true);
  }
  instance.exports.wrapped_row(a, 1, b, width, out, width, 0x80000000, 2, 0, width, 0);
  return Buffer.from(new Uint8Array(instance.exports.memory.buffer, out, width * 8)).toString("hex");
}
(async () => {
  const width = Number(process.argv[4]);
  const before = await execute(process.argv[2], width);
  const after = await execute(process.argv[3], width);
  if (before !== after) throw new Error(`wrapped index bytes differ: ${before} != ${after}`);
  process.stdout.write("wrapped-u32-byte-parity");
})().catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(actual) = run_node_pair(&reference, &guarded, &width.to_string(), runner) else {
        return;
    };
    assert_eq!(actual, "wrapped-u32-byte-parity");
}

#[test]
fn node_should_fallback_when_out_of_slice_writes_alias_the_hoisted_input() {
    let state = baseline_state(WRAPPING_PROBE);
    let candidate = discover_wasm_invariant_load_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("wrapped row invariant load");
    assert!(
        candidate.minimum_trip <= 32,
        "the shifted-output alias case must pass the profitability threshold"
    );
    let prepared = prepare_wasm_invariant_load_trial(&state, &candidate).expect("trial");
    let reference =
        emit_wasm_kir_module(state.module(), calckernel::EmitWasmOptions { opt_level: 3 })
            .expect("reference direct binary");
    let guarded = emit_wasm_kir_module(
        prepared.trial.module(),
        calckernel::EmitWasmOptions { opt_level: 3 },
    )
    .expect("guarded direct binary");
    let runner = r#"
const fs = require("node:fs");
async function execute(path) {
  const {instance} = await WebAssembly.instantiate(fs.readFileSync(path));
  const view = new DataView(instance.exports.memory.buffer);
  const a = 1024, b = 4096, out = 768, width = 32;
  for (let i = 0; i < width; i++) {
    view.setFloat64(a + i * 8, i === 0 ? 1.25 : 0.25, true);
    view.setFloat64(b + i * 8, 0.5, true);
    view.setFloat64(out + i * 8, 0.125, true);
  }
  instance.exports.wrapped_row(a, width, b, width, out, width, 0, 2, 0, width, width);
  return Buffer.from(new Uint8Array(instance.exports.memory.buffer, a, width * 8)).toString("hex");
}
(async () => {
  const before = await execute(process.argv[2]);
  const after = await execute(process.argv[3]);
  if (before !== after) throw new Error(`out-of-slice alias bytes differ: ${before} != ${after}`);
  process.stdout.write("fallback-write-alias-byte-parity");
})().catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(actual) = run_node_pair(&reference, &guarded, "", runner) else {
        return;
    };
    assert_eq!(actual, "fallback-write-alias-byte-parity");
}

fn run_node_pair(reference: &[u8], guarded: &[u8], argument: &str, runner: &str) -> Option<String> {
    if !super::support::command::node_available() {
        return None;
    }
    let directory = super::support::temp::temp_dir("ck-wasm-invariant-load");
    fs::create_dir_all(&directory).expect("create Node test directory");
    let reference_path = directory.join("reference.wasm");
    let guarded_path = directory.join("guarded.wasm");
    let runner_path = directory.join("runner.cjs");
    fs::write(&reference_path, reference).expect("write reference Wasm");
    fs::write(&guarded_path, guarded).expect("write guarded Wasm");
    fs::write(&runner_path, runner).expect("write Node runner");
    let output = Command::new("node")
        .arg(&runner_path)
        .arg(&reference_path)
        .arg(&guarded_path)
        .arg(argument)
        .output()
        .expect("run Node Wasm comparison");
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "Node Wasm comparison failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8(output.stdout).expect("Node output is UTF-8"))
}
