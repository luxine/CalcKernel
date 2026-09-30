use calckernel::{
    KirBoundsMode, KirBuildConfig, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode,
    KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile,
    build_kir_module_with_profile, check_vectorization_trial_independently,
    discover_vectorization_candidates, import_contract_facts, lower_to_mir,
    prepare_vectorization_trial, run_kir_pass_pipeline, validate_kir_module,
};

const NESTED_MATMUL_COLUMN: &str = r#"
export unsafe fn matmul_column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        let previous: f64 = out[out_index];
        let scalar: f64 = a[a_index];
        let varying: f64 = b[b_index];
        out[out_index] = previous + scalar * varying;
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

const MATMUL_WITH_CARRIED_OUTPUT_STATE: &str = r#"
export unsafe fn matmul_column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let carried: f64 = 0.0;
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        let previous: f64 = out[out_index];
        let scalar: f64 = a[a_index];
        let varying: f64 = b[b_index];
        out[out_index] = carried + scalar * varying;
        carried = previous;
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

fn matmul_state(source: &str) -> KirVerifiedProgramState {
    let checked = calckernel::check(&SourceFile::new("matmul-uf.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid fixture: {source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: profile.consumer(),
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("WASM KIR");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let scalar = optimized.artifact.expect("optimized pre-state");
    let contracts =
        import_contract_facts(&scalar, &checked.checked_program, 0).expect("late source contracts");
    KirVerifiedProgramState::from_parts(
        scalar,
        Some(contracts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified pre-state")
}

fn matmul_uf4_candidate(state: &KirVerifiedProgramState) -> calckernel::VectorizationCandidate {
    discover_vectorization_candidates(state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("nested matmul should expose strict SIMD128 VF2/UF4")
}

#[test]
fn matmul_uf4_shared_broadcast_trial_passes_independent_checker() {
    let state = matmul_state(NESTED_MATMUL_COLUMN);
    let candidate = matmul_uf4_candidate(&state);
    assert!(
        candidate
            .operations
            .iter()
            .all(|operation| { operation.semantics == calckernel::KirCostSemantics::StrictFloat })
    );

    let prepared = prepare_vectorization_trial(&state, &candidate).expect("matmul UF4 trial");
    assert_eq!((prepared.plan.vf, prepared.plan.uf), (2, 4));
    assert_eq!(prepared.plan.broadcast_groups.len(), 1);
    assert_eq!(prepared.plan.broadcast_groups[0].unroll_index, 0);
    assert_eq!(prepared.plan.memory_groups.len(), 12);
    assert_eq!(prepared.plan.operations.len(), 8);
    assert_eq!(
        prepared
            .plan
            .predicates
            .iter()
            .filter(|predicate| matches!(
                predicate,
                calckernel::VectorPredicate::WasmSliceRange { .. }
            ))
            .count(),
        3,
        "A single element and both complete varying ranges remain guarded"
    );
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "source-only checker should prove shared A broadcast and four strict chunks"
    );
}

#[test]
fn matmul_uf4_checker_rejects_forged_shared_broadcast_mapping() {
    let state = matmul_state(NESTED_MATMUL_COLUMN);
    let candidate = matmul_uf4_candidate(&state);
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("matmul UF4 trial");
    let mut plan = prepared.plan.clone();
    plan.broadcast_groups[0].unroll_index = 1;
    assert!(
        check_vectorization_trial_independently(&state, &prepared.trial, &plan, &prepared.charge,)
            .is_err(),
        "checker accepted a shared A broadcast assigned to a later chunk"
    );
}

#[test]
fn matmul_uf4_checker_rejects_valid_kir_wrong_broadcast_index() {
    let state = matmul_state(NESTED_MATMUL_COLUMN);
    let candidate = matmul_uf4_candidate(&state);
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("matmul UF4 trial");
    let mut wrong_index = prepared.trial.clone();
    let function = wrong_index
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let induction_index = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .and_then(|function| {
            function
                .blocks
                .iter()
                .find(|block| block.id == candidate.header)
        })
        .and_then(|header| {
            header
                .params
                .iter()
                .position(|param| param.value == candidate.induction)
        })
        .expect("source induction parameter index");
    let vector_induction = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_header")
        .and_then(|block| block.params.get(induction_index))
        .expect("vector induction parameter")
        .value;
    let scalar_load = prepared.plan.broadcast_groups[0].emitted_scalar_load;
    let instruction = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == scalar_load)
        .expect("emitted scalar A load");
    let calckernel::KirInstructionKind::Load { place } = &mut instruction.kind else {
        panic!("A broadcast must start with a scalar load");
    };
    let calckernel::KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("A broadcast must use a slice index");
    };
    *index = vector_induction;

    assert!(
        validate_kir_module(wrong_index.module()).errors.is_empty(),
        "the mutation should remain structurally valid KIR"
    );
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_index,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a broadcast index that is not the invariant source A index"
    );
}

#[test]
fn matmul_uf4_checker_rejects_valid_kir_wrong_fourth_b_access_offset() {
    let state = matmul_state(NESTED_MATMUL_COLUMN);
    let candidate = matmul_uf4_candidate(&state);
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("matmul UF4 trial");
    let affine = candidate.wasm_affine.as_ref().expect("affine source proof");
    let output_slice = affine
        .accesses
        .iter()
        .find(|access| access.kind == calckernel::LoopMemoryAccessKind::Write)
        .expect("output store")
        .slice;
    let b_instruction = affine
        .accesses
        .iter()
        .find(|access| {
            access.kind == calckernel::LoopMemoryAccessKind::Read
                && access.slice != output_slice
                && matches!(access.shape, calckernel::WasmAffineShape::Contiguous { .. })
        })
        .expect("contiguous B read")
        .instruction;
    let first_group = prepared
        .plan
        .memory_groups
        .iter()
        .find(|group| {
            group.scalar_instructions == [b_instruction]
                && group.unroll_index == 0
                && group.access == calckernel::VectorMemoryAccessKind::Read
        })
        .expect("first B vector load");
    let fourth_group = prepared
        .plan
        .memory_groups
        .iter()
        .find(|group| {
            group.scalar_instructions == [b_instruction]
                && group.unroll_index == 3
                && group.access == calckernel::VectorMemoryAccessKind::Read
        })
        .expect("fourth B vector load");
    let function = prepared
        .trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let first_start = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == first_group.vector_instruction)
        .and_then(|instruction| match &instruction.kind {
            calckernel::KirInstructionKind::VectorLoad { access, .. } => Some(access.start),
            _ => None,
        })
        .expect("first B start");

    let mut wrong_offset = prepared.trial.clone();
    let function = wrong_offset
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let fourth = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == fourth_group.vector_instruction)
        .expect("fourth B vector load");
    let calckernel::KirInstructionKind::VectorLoad { access, .. } = &mut fourth.kind else {
        panic!("fourth B group must be a vector load");
    };
    access.start = first_start;

    assert!(
        validate_kir_module(wrong_offset.module()).errors.is_empty(),
        "the wrong but well-typed address should remain valid KIR"
    );
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_offset,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted the first chunk's B address in the fourth chunk"
    );
}

#[test]
fn matmul_uf4_checker_rejects_valid_kir_wrong_strict_multiply_input() {
    let state = matmul_state(NESTED_MATMUL_COLUMN);
    let candidate = matmul_uf4_candidate(&state);
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("matmul UF4 trial");
    let mut wrong_operand = prepared.trial.clone();
    let function = wrong_operand
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("matmul function");
    let earlier_vector_load = prepared
        .plan
        .memory_groups
        .iter()
        .find(|group| {
            group.unroll_index == 0 && group.access == calckernel::VectorMemoryAccessKind::Read
        })
        .and_then(|group| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find(|instruction| instruction.id == group.vector_instruction)
        })
        .and_then(|instruction| instruction.results.first())
        .expect("first chunk output vector read")
        .value;
    let second_multiply = prepared
        .plan
        .operations
        .iter()
        .find(|mapping| {
            mapping.operation == calckernel::KirProfileOperation::Multiply
                && mapping.unroll_index == 1
        })
        .expect("second strict multiply mapping")
        .vector;
    let instruction = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == second_multiply)
        .expect("second vector multiply");
    let calckernel::KirInstructionKind::VectorBinary { right, .. } = &mut instruction.kind else {
        panic!("strict multiply must be a vector binary operation");
    };
    *right = earlier_vector_load;

    assert!(
        validate_kir_module(wrong_operand.module())
            .errors
            .is_empty(),
        "the mutation should remain structurally valid KIR"
    );
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_operand,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a strict multiply whose varying input no longer maps to B"
    );
}

#[test]
fn matmul_uf4_without_static_noalias_must_keep_scalar_fallback() {
    let source = NESTED_MATMUL_COLUMN.replace(
        "  requires noalias(a, b) && noalias(a, out) && noalias(b, out);\n",
        "",
    );
    let state = matmul_state(&source);
    let candidates = discover_vectorization_candidates(&state).candidates;
    assert!(
        candidates.iter().all(|candidate| {
            candidate.wasm_affine.is_none() || candidate.vf != 2 || candidate.uf != 4
        }),
        "the matmul UF4 fast loop requires source-proven alias separation"
    );
}

#[test]
fn affine_matmul_with_a_carried_output_value_must_keep_the_scalar_loop() {
    let state = matmul_state(MATMUL_WITH_CARRIED_OUTPUT_STATE);
    let discovery = discover_vectorization_candidates(&state);
    let affine_vector_candidates = discovery
        .candidates
        .iter()
        .filter(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2)
        .collect::<Vec<_>>();
    for candidate in &affine_vector_candidates {
        let Ok(prepared) = prepare_vectorization_trial(&state, candidate) else {
            continue;
        };
        assert!(
            check_vectorization_trial_independently(
                &state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            )
            .is_err(),
            "independent checker accepted a UF{} trial that reuses loop-carried output state",
            candidate.uf
        );
    }
    assert!(
        affine_vector_candidates.is_empty(),
        "carried output state must not be broadcast across vector lanes or UF chunks"
    );
}
